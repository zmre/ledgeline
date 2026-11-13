//! Ledgeline ▸ "Make Ledgeline the Default for Journal Files" (macOS).
//!
//! `assets/Info.plist.in` already claims journals at `Owner` rank, which makes
//! Ledgeline the default on a Mac where nothing else claims them. Rank is only
//! a tie-breaker between apps, though: another app claiming the same type at
//! `Owner` (an editor that imports `org.ledger-cli.journal`, say) leaves the
//! choice to Launch Services, and it can flip each time either app is
//! re-registered. The user's own choice outranks every claim, and that is what
//! this command records, the same entry Finder's Get Info ▸ "Change All…"
//! writes.
//!
//! Two Launch Services calls, both through `unsafe` FFI (they are C functions):
//! `LSRegisterURL` on the running bundle first, so a freshly installed or
//! rebuilt copy is known before we name it, then
//! `LSSetDefaultRoleHandlerForContentType` per content type. The second is
//! deprecated in favour of an async `NSWorkspace` method that takes an app URL
//! and a completion block; the C call is synchronous, keyed by bundle id rather
//! than by one copy's path (so it survives a reinstall to a new path), and still
//! works on current macOS.
//!
//! The work runs on its own thread, like File ▸ Export, because the result
//! dialog blocks.

use std::path::Path;

use muda::{MenuId, MenuItem};
use objc2_core_foundation::{CFString, CFURL};
#[allow(deprecated)] // see the module docs for why the C call is kept
use objc2_core_services::{
    LSCopyDefaultRoleHandlerForContentType, LSRegisterURL, LSRolesMask,
    LSSetDefaultRoleHandlerForContentType,
};

const MENU_ID: &str = "make-default-for-journals";

/// `CFBundleIdentifier` in `assets/Info.plist.in`.
const BUNDLE_ID: &str = "com.zmre.ledgeline";

/// The content types the command makes Ledgeline the default for: every type
/// in the plist's `Owner`-ranked document type (a unit test holds the two
/// lists together). The `Alternate` ones (`.ledger`, `.j`) are left alone on
/// purpose; see the plist's comment.
const DEFAULT_CONTENT_TYPES: [&str; 2] = ["com.zmre.ledgeline.journal", "org.ledger-cli.journal"];

#[derive(Debug, thiserror::Error)]
enum FileAssocError {
    #[error(
        "Ledgeline is not running from an app bundle ({exe}), so there is no app to make the default."
    )]
    NotInBundle { exe: String },
    #[error("Launch Services could not register {bundle} (OSStatus {status}).")]
    Register { bundle: String, status: i32 },
    #[error(
        "Launch Services refused to set the default app for {content_type} (OSStatus {status})."
    )]
    SetDefault { content_type: String, status: i32 },
    #[error("The default app for {content_type} is still {actual} after setting it to Ledgeline.")]
    NotApplied {
        content_type: String,
        actual: String,
    },
}

/// The app-menu item.
pub(super) fn menu_item() -> MenuItem {
    MenuItem::with_id(
        MENU_ID,
        "Make Ledgeline the Default for Journal Files",
        true,
        None,
    )
}

/// Run the command if `id` is ours. Returns whether it was.
pub(super) fn handle(id: &MenuId) -> bool {
    if *id != MENU_ID {
        return false;
    }
    std::thread::spawn(|| match make_default() {
        Ok(()) => show(
            rfd::MessageLevel::Info,
            "Ledgeline is now the default",
            "Double-clicking a .journal or .hledger file in Finder now opens it in Ledgeline.",
        ),
        Err(error) => {
            eprintln!("ledgeline: make default failed: {error}");
            show(
                rfd::MessageLevel::Error,
                "Could not make Ledgeline the default",
                &error.to_string(),
            );
        }
    });
    true
}

fn make_default() -> Result<(), FileAssocError> {
    // Canonical, because `bin/ledgeline` in a Nix profile is a symlink INTO the
    // bundle and the bundle is what we need.
    let exe = std::env::current_exe()
        .and_then(|exe| exe.canonicalize())
        .map_err(|error| FileAssocError::NotInBundle {
            exe: error.to_string(),
        })?;
    let bundle = enclosing_app_bundle(&exe).ok_or_else(|| FileAssocError::NotInBundle {
        exe: exe.display().to_string(),
    })?;
    register(bundle)?;
    DEFAULT_CONTENT_TYPES
        .iter()
        .try_for_each(|content_type| set_default(content_type))
}

/// The `.app` bundle an executable at `…/X.app/Contents/MacOS/<exe>` lives in.
fn enclosing_app_bundle(exe: &Path) -> Option<&Path> {
    let macos = exe
        .parent()
        .filter(|dir| dir.file_name() == Some("MacOS".as_ref()))?;
    let contents = macos
        .parent()
        .filter(|dir| dir.file_name() == Some("Contents".as_ref()))?;
    contents
        .parent()
        .filter(|dir| dir.extension() == Some("app".as_ref()))
}

fn register(bundle: &Path) -> Result<(), FileAssocError> {
    let failed = |status| FileAssocError::Register {
        bundle: bundle.display().to_string(),
        status,
    };
    let url = CFURL::from_directory_path(bundle).ok_or_else(|| failed(-1))?;
    // SAFETY: `url` is a valid, retained CFURL that outlives the call, which
    // only reads it.
    let status = unsafe { LSRegisterURL(&url, true) };
    if status == 0 {
        Ok(())
    } else {
        Err(failed(status))
    }
}

#[allow(deprecated)] // see the module docs
fn set_default(content_type: &str) -> Result<(), FileAssocError> {
    let uti = CFString::from_str(content_type);
    let handler = CFString::from_str(BUNDLE_ID);
    // SAFETY: both arguments are valid, retained CFStrings that outlive the
    // call, which only reads them.
    let status = unsafe { LSSetDefaultRoleHandlerForContentType(&uti, LSRolesMask::All, &handler) };
    if status != 0 {
        return Err(FileAssocError::SetDefault {
            content_type: content_type.to_string(),
            status,
        });
    }
    // Read it back rather than trust the status: a refusal that still returns
    // noErr would otherwise report success.
    // SAFETY: as above; the result is an owned (+1) CFString or none.
    let actual = unsafe { LSCopyDefaultRoleHandlerForContentType(&uti, LSRolesMask::All) }
        .map(|id| id.to_string());
    match actual {
        Some(id) if id.eq_ignore_ascii_case(BUNDLE_ID) => Ok(()),
        other => Err(FileAssocError::NotApplied {
            content_type: content_type.to_string(),
            actual: other.unwrap_or_else(|| "unset".to_string()),
        }),
    }
}

fn show(level: rfd::MessageLevel, title: &str, text: &str) {
    rfd::MessageDialog::new()
        .set_level(level)
        .set_title(title)
        .set_description(text)
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{BUNDLE_ID, DEFAULT_CONTENT_TYPES, enclosing_app_bundle};

    #[test]
    fn finds_the_bundle_around_its_executable() {
        assert_eq!(
            enclosing_app_bundle(Path::new(
                "/Applications/Ledgeline.app/Contents/MacOS/ledgeline"
            )),
            Some(Path::new("/Applications/Ledgeline.app"))
        );
    }

    #[test]
    fn a_bare_binary_has_no_bundle() {
        assert_eq!(
            enclosing_app_bundle(Path::new("/repo/target/debug/ledgeline")),
            None
        );
    }

    #[test]
    fn a_lookalike_path_outside_an_app_has_no_bundle() {
        assert_eq!(
            enclosing_app_bundle(Path::new("/tmp/Ledgeline/Contents/MacOS/ledgeline")),
            None
        );
    }

    /// The plist template, read at run time: `include_str!` would reach outside
    /// the published crate. The Nix source filter keeps this one file.
    fn info_plist() -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/Info.plist.in");
        std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("reading {}: {error}", path.display()))
    }

    /// Text between `<key>{key}</key>` and the next top-level key.
    fn section<'a>(plist: &'a str, key: &str, next_key: &str) -> &'a str {
        let start = plist
            .find(&format!("<key>{key}</key>"))
            .unwrap_or_else(|| panic!("{key} missing from Info.plist.in"));
        let end = plist[start..]
            .find(&format!("<key>{next_key}</key>"))
            .map_or(plist.len(), |offset| start + offset);
        &plist[start..end]
    }

    #[test]
    fn the_plist_claims_every_type_the_command_sets_at_owner_rank() {
        let plist = info_plist();
        let document_types = section(
            &plist,
            "CFBundleDocumentTypes",
            "UTExportedTypeDeclarations",
        );
        let owner = document_types
            .split("</dict>")
            .find(|dict| dict.contains("<string>Owner</string>"))
            .expect("an Owner-ranked document type");
        let claimed: Vec<&str> = owner
            .split("<string>")
            .skip(1)
            .filter_map(|rest| rest.split("</string>").next())
            .filter(|value| value.contains('.'))
            .collect();
        assert_eq!(claimed, DEFAULT_CONTENT_TYPES);
    }

    #[test]
    fn the_plist_bundle_id_is_the_one_the_command_names() {
        assert!(info_plist().contains(&format!(
            "<key>CFBundleIdentifier</key>\n  <string>{BUNDLE_ID}</string>"
        )));
    }
}
