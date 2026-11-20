//! <App> ▸ "Make <App> the Default for <Documents>" (macOS): an app-menu
//! command, described by a [`DefaultHandler`] (Ledgeline's: "Make Ledgeline
//! the Default for Journal Files").
//!
//! An app's `Info.plist` can claim its document types at `Owner` rank, which
//! makes it the default on a Mac where nothing else claims them. Rank is only
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

use crate::AppIdentity;

/// One app's "make me the default" command: its menu item, the content types
/// it claims, and what the dialogs say. The bundle id is the app's
/// [`AppIdentity::bundle_id`].
#[derive(Debug)]
pub struct DefaultHandler {
    pub app: &'static AppIdentity,
    /// The menu item's id.
    pub menu_id: &'static str,
    /// The menu item's label.
    pub menu_label: &'static str,
    /// The content types the command makes the app the default for: every
    /// type in its plist's `Owner`-ranked document type (an app keeps a unit
    /// test holding the two lists together).
    pub content_types: &'static [&'static str],
    /// The success dialog's title and text.
    pub done_title: &'static str,
    pub done_text: &'static str,
    /// The failure dialog's title.
    pub failed_title: &'static str,
}

#[derive(Debug, thiserror::Error)]
enum FileAssocError {
    #[error(
        "{app} is not running from an app bundle ({exe}), so there is no app to make the default."
    )]
    NotInBundle { app: &'static str, exe: String },
    #[error("Launch Services could not register {bundle} (OSStatus {status}).")]
    Register { bundle: String, status: i32 },
    #[error(
        "Launch Services refused to set the default app for {content_type} (OSStatus {status})."
    )]
    SetDefault { content_type: String, status: i32 },
    #[error("The default app for {content_type} is still {actual} after setting it to {app}.")]
    NotApplied {
        app: &'static str,
        content_type: String,
        actual: String,
    },
}

impl DefaultHandler {
    /// The app-menu item.
    #[must_use]
    pub fn menu_item(&self) -> MenuItem {
        MenuItem::with_id(self.menu_id, self.menu_label, true, None)
    }

    /// Run the command if `id` is ours. Returns whether it was.
    pub fn handle(&'static self, id: &MenuId) -> bool {
        if *id != self.menu_id {
            return false;
        }
        std::thread::spawn(move || match self.make_default() {
            Ok(()) => show(rfd::MessageLevel::Info, self.done_title, self.done_text),
            Err(error) => {
                eprintln!("{}: make default failed: {error}", self.app.log_prefix);
                show(
                    rfd::MessageLevel::Error,
                    self.failed_title,
                    &error.to_string(),
                );
            }
        });
        true
    }

    fn make_default(&self) -> Result<(), FileAssocError> {
        let app = self.app.name;
        // Canonical, because `bin/ledgeline` in a Nix profile is a symlink INTO the
        // bundle and the bundle is what we need.
        let exe = std::env::current_exe()
            .and_then(|exe| exe.canonicalize())
            .map_err(|error| FileAssocError::NotInBundle {
                app,
                exe: error.to_string(),
            })?;
        let bundle = enclosing_app_bundle(&exe).ok_or_else(|| FileAssocError::NotInBundle {
            app,
            exe: exe.display().to_string(),
        })?;
        register(bundle)?;
        self.content_types
            .iter()
            .try_for_each(|content_type| set_default(self.app, content_type))
    }
}

/// The `.app` bundle an executable at `…/X.app/Contents/MacOS/<exe>` lives in.
#[must_use]
pub fn enclosing_app_bundle(exe: &Path) -> Option<&Path> {
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
fn set_default(app: &AppIdentity, content_type: &str) -> Result<(), FileAssocError> {
    let uti = CFString::from_str(content_type);
    let handler = CFString::from_str(app.bundle_id);
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
        Some(id) if id.eq_ignore_ascii_case(app.bundle_id) => Ok(()),
        other => Err(FileAssocError::NotApplied {
            app: app.name,
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

    use super::enclosing_app_bundle;

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
}
