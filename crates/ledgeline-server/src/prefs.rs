//! General application preferences — the app's FIRST such store (WP-11).
//!
//! `recents.rs` is the model for this file: same config directory, same
//! `$LEDGELINE_CONFIG_DIR` override, same
//! [`ledgeline_core::edit::atomic_write`], same move-a-corrupt-file-aside rule.
//! Two stores that disagree about where the config lives, or about what to do
//! with a file they cannot parse, is exactly the drift worth spending a doc
//! comment to prevent.
//!
//! The store is a TOML file, `prefs.toml`, in the injected
//! [`AppDirs::config_dir`] (on the desktop, `$LEDGELINE_CONFIG_DIR` or
//! `dirs::config_dir()/ledgeline` — see [`AppDirs::from_env`]), holding one
//! setting:
//!
//! * `hledgerPath` — an explicit `hledger` binary, for the (common) case where
//!   the desktop app is launched from Finder and inherits a `$PATH` that has
//!   never seen the user's shell profile.
//!
//! TOML rather than JSON (plan 38, WP-5) so a person can edit it by hand. It is
//! read and rewritten through `toml_edit`'s document model, so a comment, the
//! key order and any key this build does not know all survive a save from the
//! settings form.
//!
//! # Migration from `prefs.json`
//!
//! Builds before WP-5 wrote `prefs.json`. The first [`load`] or [`store`] that
//! finds `prefs.json` and no `prefs.toml` converts it once: it creates
//! `prefs.toml` exclusively (`O_EXCL`, so two processes migrating at once
//! cannot clobber each other) and only then renames the old file to
//! `prefs.json.migrated`. Nothing is deleted. The conversion carries
//! `hledgerPath` and nothing else: the only other key an old file can hold is
//! the retired `gitAutocommit`, and the renamed original keeps every byte.
//!
//! The migrated path is NOT re-validated with [`check_storable`]: that probe
//! executes the binary, which a startup read must never do, and a value that
//! was storable once is exactly as trustworthy as it was. [`load`] treats it
//! like any other stored path (see below).
//!
//! # Why `store` validates and `load` does not
//!
//! `hledger_path` is checked at STORE time ([`check_storable`]) and rejected
//! rather than persisted. A bad value written now would not fail now — it would
//! fail at import time, several screens later, as "could not run hledger",
//! which is the mysterious-failure shape this module exists to avoid. The
//! `PUT /api/prefs` route turns that rejection into a `400` the settings form
//! can render next to the field the user just typed in.
//!
//! [`load`] stays forgiving for the mirror-image reason: the binary may have
//! been valid when it was stored and removed since (a Nix garbage-collect will
//! do it), and refusing to start because a *preference* went stale is worse than
//! the alternative. So a stale path survives `load`, and
//! [`Hledger::resolve`](crate::hledger::Hledger::resolve) simply falls through
//! to the next candidate. Every consumer therefore re-validates before use with
//! the CHEAP half of the check ([`check_executable`]), which lives here, once,
//! for both of them.
//!
//! # `hledgerPath` is a code-execution setting, and is validated like one
//!
//! SEC-14. This preference is not a display option: whatever it names becomes
//! the first candidate in [`Hledger::candidates`](crate::hledger) and is handed
//! straight to `Command::new` on every import, and it SURVIVES RESTARTS. So the
//! store-time gate is stricter than "is this a file with an `+x` bit on it":
//! [`check_storable`] additionally refuses a binary that any user but its owner
//! can write, and actually RUNS the candidate with `--version` before persisting
//! it, so a value that could never have worked is refused at the API rather than
//! discovered three screens later.
//!
//! Read the limits of that honestly. It removes the *persistent* half for
//! anything that is not plausibly hledger — such a value is refused and never
//! written down. It does not remove execution: probing IS an `exec`, so this
//! route still runs whatever it is handed, once, before refusing it. And an
//! attacker who points it at a binary they own outright which does print an
//! hledger version banner walks through all of it. It is defence in depth for
//! the token exposure documented in [`crate::security`], not a fix for it.
//!
//! This is also why `hledgerPath` lives ONLY here, in the user's own config
//! directory. The per-ledger `ledgeline.toml` travels with a cloned repository
//! and is never consulted for it (see `ui_prefs.rs`).
//!
//! # Why a corrupt file is moved aside rather than overwritten
//!
//! Same reasoning as `recents.rs`, and it applies on BOTH paths here. Reading an
//! unparseable `prefs.toml` as "no preferences" and then writing a fresh file
//! over it would destroy settings we had merely failed to *parse* — so [`load`]
//! renames it to `prefs.toml.corrupt` and returns defaults, and [`store`] checks
//! again before writing in case the file was replaced underneath us in between.
//! A file that parses but holds a `hledgerPath` that is not a string is corrupt
//! in the same sense: it is not something this build wrote or can interpret.
//!
//! # Forward compatibility
//!
//! A key this build does not know is ignored on load and KEPT on store, because
//! the store edits the parsed document in place rather than serializing a
//! struct over it. So an older build saving a file a newer one wrote no longer
//! drops the newer settings — the trade-off the JSON store had to document.

use crate::app_dirs::AppDirs;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use thiserror::Error;
use toml_edit::{DocumentMut, Item};

/// File name of the preferences store within the config directory.
const PREFS_FILE: &str = "prefs.toml";

/// The store's name before WP-5, migrated once by [`migrate_legacy_json`].
const LEGACY_JSON_FILE: &str = "prefs.json";

/// The one key this store owns.
const HLEDGER_PATH_KEY: &str = "hledgerPath";

/// What a freshly created `prefs.toml` starts with, so a person opening it by
/// hand knows what it is and that comments are safe to add.
const NEW_FILE_HEADER: &str = "\
# Ledgeline preferences. Setting the hledger path in the app writes `hledgerPath`
# here; you can also edit this file by hand. Comments and unknown keys are kept.
";

/// The persisted preferences. Every field is optional, and `Default` is the
/// "user has never opened the settings screen" state.
///
/// `Deserialize` only for the one-shot read of a legacy `prefs.json`
/// ([`migrate_legacy_json`]); the TOML store is read and written through
/// `toml_edit` so unknown keys and comments survive.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct Prefs {
    /// An explicit `hledger` binary, or `None` to discover one. Validated by
    /// [`store`]; see the module docs for why it is not validated by [`load`].
    pub(crate) hledger_path: Option<PathBuf>,
}

/// A preferences write that could not be completed.
///
/// No variant carries a path. The `/api/prefs` response body is rendered from
/// `Display`, and `tests/error_surface.rs` pins that no `/api/*` body discloses
/// an absolute path — a rule this store has to hold to even though the path in
/// question is one the caller just sent us.
#[derive(Debug, Error)]
pub(crate) enum PrefsError {
    /// The platform exposes no config directory and `$LEDGELINE_CONFIG_DIR` is
    /// unset, so there is nowhere to persist anything.
    #[error("no configuration directory is available to save preferences in")]
    NoConfigDir,
    /// `hledger_path` was set to something that is not a runnable binary.
    /// `reason` is a fixed phrase, never the offending path.
    #[error("the hledger path {reason}")]
    InvalidHledgerPath { reason: &'static str },
    /// The config directory could not be created, or the store could not be
    /// written. `std::io::Error`'s own `Display` names the errno, not the file.
    #[error("could not save preferences: {0}")]
    Io(#[from] std::io::Error),
}

/// The stored preferences, or defaults if there are none to read.
///
/// Infallible by design — see the module docs. A store that is present but
/// unparseable is moved aside first, so this never returns defaults *and* leaves
/// a file behind that the next [`store`] would silently clobber. A legacy
/// `prefs.json` is migrated first ([`migrate_legacy_json`]).
pub(crate) fn load(dirs: &AppDirs) -> Prefs {
    dirs.config_file(PREFS_FILE)
        .map_or_else(Prefs::default, |file| load_from(&file))
}

/// Persist `prefs`, keeping whatever else the file holds.
///
/// # Errors
/// [`PrefsError::InvalidHledgerPath`] if `hledger_path` is set to anything but a
/// binary that passes [`check_storable`];
/// [`PrefsError::NoConfigDir`] if there is nowhere to write;
/// [`PrefsError::Io`] if the write itself fails.
/// On any error nothing is written, so the previous settings survive intact.
///
/// The store lives in [`AppDirs::config_dir`], the same directory as the
/// recents store: the two are siblings, so a host that redirects one redirects
/// both.
pub(crate) fn store(dirs: &AppDirs, prefs: &Prefs) -> Result<(), PrefsError> {
    let file = dirs
        .config_file(PREFS_FILE)
        .ok_or(PrefsError::NoConfigDir)?;
    store_in(&file, prefs)
}

/// What the store at a given path currently holds. Mirrors `recents::Stored`,
/// and for the same reason: only a genuine absence may be treated as "no
/// settings", because only a genuine absence has nothing to lose.
enum Stored {
    /// No file there.
    Missing,
    /// A well-formed preferences file, and the document it was read from so a
    /// store can edit it in place.
    Settings(Prefs, Box<DocumentMut>),
    /// Present, but unreadable or not a preferences file. Distinguished from
    /// [`Missing`](Self::Missing) precisely so it is never silently overwritten.
    Corrupt,
}

/// What [`migrate_legacy_json`] did.
enum Migration {
    /// Nothing to migrate, or the migration completed: read `prefs.toml`.
    Done,
    /// A legacy file was read but `prefs.toml` could not be written. Its
    /// settings are still the user's settings for this run; the migration is
    /// retried next time.
    Unwritten(Prefs),
}

/// Read the store at `file` (a `prefs.toml`), migrating a sibling `prefs.json`
/// first and moving the store aside, returning defaults, if it is present but
/// unreadable.
pub(crate) fn load_from(file: &Path) -> Prefs {
    if let Migration::Unwritten(prefs) = migrate_legacy_json(file) {
        return prefs;
    }
    match read_stored(file) {
        Stored::Settings(prefs, _) => prefs,
        Stored::Missing => Prefs::default(),
        Stored::Corrupt => {
            set_corrupt_aside(file);
            Prefs::default()
        }
    }
}

/// Validate `prefs` and write it to `file` (a `prefs.toml`).
///
/// Validation happens BEFORE the migration, the corrupt check and before any
/// directory is created, so a rejected value leaves the filesystem exactly as
/// it found it.
///
/// # Errors
/// See [`store`], which is this with the path resolved from [`AppDirs`].
pub(crate) fn store_in(file: &Path, prefs: &Prefs) -> Result<(), PrefsError> {
    let hledger_path = prefs
        .hledger_path
        .as_deref()
        .map(|path| {
            check_storable(path).map_err(|reason| PrefsError::InvalidHledgerPath { reason })?;
            path.to_str().ok_or(PrefsError::InvalidHledgerPath {
                reason: "is not valid UTF-8",
            })
        })
        .transpose()?;
    // A legacy file is migrated before it could be shadowed by the file this
    // write creates; otherwise it would sit there unmigrated for good.
    if let Migration::Unwritten(_) = migrate_legacy_json(file) {
        eprintln!("ledgeline: prefs.json was not migrated; saving prefs.toml anyway");
    }
    // `load` normally moves a corrupt store aside long before we get here, but
    // not every caller loads first and the file may have been replaced in
    // between. `atomic_write` below is a wholesale replacement, so this is the
    // last chance to preserve bytes we could not parse.
    let document = match read_stored(file) {
        Stored::Settings(_, mut document) => {
            set_hledger_path(&mut document, hledger_path);
            *document
        }
        Stored::Missing => fresh_document(hledger_path),
        Stored::Corrupt => {
            set_corrupt_aside(file);
            fresh_document(hledger_path)
        }
    };
    if let Some(parent) = file.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    ledgeline_core::edit::atomic_write(file, document.to_string().as_bytes())?;
    Ok(())
}

/// A new store carrying [`NEW_FILE_HEADER`] and `path`.
///
/// Rendered as text and parsed, rather than built up through the document API,
/// because `toml_edit` keeps a comment-only document's comment as TRAILING
/// decor — after any key inserted later — and the header belongs on top.
fn fresh_document(path: Option<&str>) -> DocumentMut {
    let entry = path.map_or_else(String::new, |path| {
        format!("{HLEDGER_PATH_KEY} = {}\n", toml_edit::Value::from(path))
    });
    format!("{NEW_FILE_HEADER}{entry}")
        .parse::<DocumentMut>()
        .unwrap_or_else(|_| DocumentMut::new())
}

/// Set or clear `hledgerPath` in `document`, keeping the key's position and
/// any comment written beside it.
fn set_hledger_path(document: &mut DocumentMut, path: Option<&str>) {
    match path {
        None => {
            document.remove(HLEDGER_PATH_KEY);
        }
        Some(path) => match document
            .get_mut(HLEDGER_PATH_KEY)
            .and_then(Item::as_value_mut)
        {
            Some(existing) => {
                let decor = existing.decor().clone();
                *existing = toml_edit::Value::from(path);
                *existing.decor_mut() = decor;
            }
            None => {
                document.insert(HLEDGER_PATH_KEY, toml_edit::value(path));
            }
        },
    }
}

/// Classify the store at `file`.
fn read_stored(file: &Path) -> Stored {
    let Ok(text) = std::fs::read_to_string(file) else {
        // Absent, unreadable, or not UTF-8. Only a genuine absence is safe to
        // treat as "no settings"; anything else is content we failed to read.
        return if file.symlink_metadata().is_ok() {
            Stored::Corrupt
        } else {
            Stored::Missing
        };
    };
    let Ok(document) = text.parse::<DocumentMut>() else {
        return Stored::Corrupt;
    };
    let document = Box::new(document);
    match document.get(HLEDGER_PATH_KEY) {
        None => Stored::Settings(Prefs::default(), document),
        Some(item) => match item.as_str() {
            Some(path) => Stored::Settings(
                Prefs {
                    hledger_path: Some(PathBuf::from(path)),
                },
                document,
            ),
            // A number, a table, an array: not a value this store wrote.
            None => Stored::Corrupt,
        },
    }
}

/// Convert a legacy `prefs.json` beside `toml_file` into `toml_file`, once.
///
/// A no-op unless `toml_file` is absent and the JSON file is present. An
/// unparseable JSON file is moved aside as `prefs.json.corrupt`, exactly as the
/// JSON store itself would have done. See the module docs.
fn migrate_legacy_json(toml_file: &Path) -> Migration {
    let legacy = toml_file.with_file_name(LEGACY_JSON_FILE);
    if toml_file.symlink_metadata().is_ok() || legacy.symlink_metadata().is_err() {
        return Migration::Done;
    }
    let parsed = std::fs::read_to_string(&legacy)
        .ok()
        .and_then(|text| serde_json::from_str::<Prefs>(&text).ok());
    let Some(prefs) = parsed else {
        set_aside(&legacy, "json.corrupt", "unreadable preferences");
        return Migration::Done;
    };
    let document = fresh_document(prefs.hledger_path.as_deref().and_then(Path::to_str));
    match ledgeline_core::pinned::create_new(toml_file, document.to_string().as_bytes()) {
        // Created, or created by a concurrent migration a moment ago: either
        // way `prefs.toml` now holds the settings and the old file can go.
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => {
            eprintln!("ledgeline: could not migrate prefs.json to prefs.toml: {error}");
            return Migration::Unwritten(prefs);
        }
    }
    set_aside(&legacy, "json.migrated", "migrated preferences");
    Migration::Done
}

/// Rename an unreadable store to `prefs.toml.corrupt` so its bytes survive for
/// recovery and the next write starts from a clean file.
fn set_corrupt_aside(file: &Path) {
    set_aside(file, "toml.corrupt", "unreadable preferences");
}

/// Rename `file` to the same name with `extension`, logging either way.
fn set_aside(file: &Path, extension: &str, what: &str) {
    let aside = file.with_extension(extension);
    match std::fs::rename(file, &aside) {
        Ok(()) => eprintln!(
            "ledgeline: {what} {} kept as {}",
            file.display(),
            aside.display()
        ),
        Err(error) => eprintln!(
            "ledgeline: could not set aside {what} {}: {error}",
            file.display()
        ),
    }
}

/// Whether `path` is a binary we could actually execute — the cheap, purely
/// stat-based check both this module (as the first step of [`check_storable`])
/// and [`hledger`](crate::hledger) (at resolve time, once per candidate) use, so
/// "a runnable-looking hledger path" means one thing in this codebase.
pub(crate) fn is_executable_file(path: &Path) -> bool {
    check_executable(path).is_ok()
}

/// The full STORE-time gate for `hledger_path`: [`check_executable`], plus the
/// two checks that only make sense at the moment a user chooses a binary
/// (SEC-14).
///
/// Both extras are here rather than in [`check_executable`] on purpose, because
/// [`Hledger::candidates`](crate::hledger) runs that one over every candidate on
/// every resolve:
///
/// * **Not writable by anyone but its owner.** A group- or world-writable
///   binary is one any of those users can replace between our check and our
///   `exec`, so persisting it as the thing we will run on every import — for
///   good, across restarts — hands them code execution as this user. Refusing it
///   at resolve time instead would silently drop a binary the user explicitly
///   configured, with no screen able to say why; refusing it *here* puts the
///   sentence next to the field they just typed in.
/// * **It answers `--version` like hledger.** One `fork`/`exec` with a short
///   timeout, which is affordable once per settings save and is not affordable
///   per candidate per resolve.
///
/// # What this does NOT check, deliberately
///
/// The DIRECTORIES above the binary. A file that is `0755` inside a
/// world-writable directory can still be swapped out by anyone who can write
/// that directory, and walking every ancestor would reject `/tmp`-rooted paths
/// (mode `1777`, sticky, and legitimately used by the test suites and by `nix
/// shell`). The mode check is the sharp, cheap edge of the problem, not all of
/// it — and none of this is what stops a caller who already has the access
/// token; see the module docs.
///
/// # Errors
/// A fixed phrase naming the failed property, never the path — the module docs
/// on [`PrefsError`] explain why that matters for an `/api/*` body.
pub(crate) fn check_storable(path: &Path) -> Result<(), &'static str> {
    check_executable(path)?;
    check_not_writable_by_others(path)?;
    crate::hledger::probe_answers_like_hledger(path)
}

/// Reject a binary that some user other than its owner may rewrite.
///
/// Unix mode bits only: `0o022` is the group-write and other-write bits
/// together. On non-unix targets this is a no-op, for the same reason
/// [`check_executable`] skips the execute bit there.
#[cfg(unix)]
fn check_not_writable_by_others(path: &Path) -> Result<(), &'static str> {
    use std::os::unix::fs::PermissionsExt;
    let meta = std::fs::metadata(path).map_err(|_| "does not exist")?;
    if meta.permissions().mode() & 0o022 == 0 {
        Ok(())
    } else {
        Err("is writable by other users, so it is not safe to run (chmod go-w it)")
    }
}

#[cfg(not(unix))]
fn check_not_writable_by_others(_path: &Path) -> Result<(), &'static str> {
    Ok(())
}

/// [`is_executable_file`] with the reason attached, and the first step of
/// [`check_storable`] — so the reason reaches the error a rejected
/// `PUT /api/prefs` returns.
///
/// Four properties, each earning its place:
///
/// * **Absolute.** A relative path resolves against the process's working
///   directory, which for a double-clicked desktop app is `/` on macOS and the
///   user's home on Linux. A preference that means a different binary depending
///   on how the app was launched is not a preference.
/// * **Exists.**
/// * **A regular file.** `exists()` is equally true of a directory, a device
///   node and a FIFO — the same distinction `recents::is_regular_file` draws,
///   and here it also keeps us from handing `Command::new` a path that could
///   take an unbounded time to fail on.
/// * **Executable.** Checked via the mode bits on unix. This is an
///   INFORMATIONAL check, not an access-control one: it can race, and the real
///   authority is the kernel at `exec` time. It exists to produce a good error
///   at the moment the user typed the path rather than a bad one much later.
///
/// On non-unix targets the mode check is skipped (Windows has no execute bit;
/// executability there is a function of the file extension). Ledgeline builds
/// for macOS and Linux, so that branch is a courtesy, not a supported path.
pub(crate) fn check_executable(path: &Path) -> Result<(), &'static str> {
    if !path.is_absolute() {
        return Err("must be an absolute path");
    }
    let meta = std::fs::metadata(path).map_err(|_| "does not exist")?;
    if !meta.is_file() {
        return Err("is not a regular file");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if meta.permissions().mode() & 0o111 == 0 {
            return Err("is not executable");
        }
    }
    Ok(())
}
