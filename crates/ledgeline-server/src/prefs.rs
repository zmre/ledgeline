//! General application preferences — the app's FIRST such store (WP-11).
//!
//! Until now the only persisted state was the recently-opened journal list, and
//! `recents.rs` is deliberately the model for this file: same config directory,
//! same `$LEDGELINE_CONFIG_DIR` override, same
//! [`ledgeline_core::edit::atomic_write`], same move-a-corrupt-file-aside rule.
//! Two stores that disagree about where the config lives, or about what to do
//! with a file they cannot parse, is exactly the drift worth spending a doc
//! comment to prevent.
//!
//! The store is a single JSON object at
//! `dirs::config_dir()/ledgeline/prefs.json`, holding two settings:
//!
//! * `hledgerPath` — an explicit `hledger` binary, for the (common) case where
//!   the desktop app is launched from Finder and inherits a `$PATH` that has
//!   never seen the user's shell profile.
//! * `gitAutocommit` — whether to commit around imports. `None` is not "off":
//!   it means "commit when a git repo is present", the default posture
//!   `git.rs` implements.
//!
//! # Why `store` validates and `load` does not
//!
//! `hledger_path` is checked at STORE time ([`check_executable`]) and rejected
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
//! to the next candidate. Every consumer therefore re-validates before use, and
//! the check lives here, once, for both of them.
//!
//! # Why a corrupt file is moved aside rather than overwritten
//!
//! Same reasoning as `recents.rs`, and it applies on BOTH paths here. Reading an
//! unparseable `prefs.json` as "no preferences" and then writing a fresh object
//! over it would destroy settings we had merely failed to *parse* — so [`load`]
//! renames it to `prefs.json.corrupt` and returns defaults, and [`store`] checks
//! again before writing in case the file was replaced underneath us in between.
//!
//! # Forward compatibility, and its one sharp edge
//!
//! `#[serde(default)]` on the struct plus serde's default tolerance of unknown
//! fields means neither adding a field in a later version nor reading a file
//! written by one can fail to parse. The sharp edge is that unknown fields are
//! not *round-tripped*: an older build that loads and then stores a file written
//! by a newer one drops the settings it did not understand. That is a deliberate
//! trade — preserving them needs a `#[serde(flatten)]` catch-all map, which the
//! WP-11 contract does not specify — and it is tolerable only because this store
//! is written solely in response to an explicit user action.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Env override for the directory that holds `prefs.json`. Identical to the one
/// `recents.rs` honours, and on purpose: one env var moves the whole config.
const CONFIG_DIR_ENV: &str = "LEDGELINE_CONFIG_DIR";
/// Application subdirectory under the OS config dir.
const APP_DIR: &str = "ledgeline";
/// File name of the preferences store within the config directory.
const PREFS_FILE: &str = "prefs.json";

/// The persisted preferences. Every field is optional, and `Default` is the
/// "user has never opened the settings screen" state.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct Prefs {
    /// An explicit `hledger` binary, or `None` to discover one. Validated by
    /// [`store`]; see the module docs for why it is not validated by [`load`].
    pub(crate) hledger_path: Option<PathBuf>,
    /// `None` = commit around imports when a git repo is present. See `git.rs`.
    ///
    /// Three-valued rather than a `bool` so "I have never been asked" is
    /// distinguishable from "I said no": the import panel offers its opt-out
    /// toggle only in the first case, and writing `Some(false)` is what silences
    /// it for good.
    pub(crate) git_autocommit: Option<bool>,
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
    /// Serializing two `Option`s cannot realistically fail, but it is not
    /// unwrapped.
    #[error("could not serialize preferences: {0}")]
    Serialize(#[from] serde_json::Error),
}

/// The stored preferences, or defaults if there are none to read.
///
/// Infallible by design — see the module docs. A store that is present but
/// unparseable is moved aside first, so this never returns defaults *and* leaves
/// a file behind that the next [`store`] would silently clobber.
pub(crate) fn load() -> Prefs {
    prefs_file().map_or_else(Prefs::default, |file| load_from(&file))
}

/// Persist `prefs`, replacing whatever was there.
///
/// # Errors
/// [`PrefsError::InvalidHledgerPath`] if `hledger_path` is set to anything but
/// an existing, absolute, regular, executable file;
/// [`PrefsError::NoConfigDir`] if there is nowhere to write;
/// [`PrefsError::Io`] / [`PrefsError::Serialize`] if the write itself fails.
/// On any error nothing is written, so the previous settings survive intact.
pub(crate) fn store(prefs: &Prefs) -> Result<(), PrefsError> {
    let file = prefs_file().ok_or(PrefsError::NoConfigDir)?;
    store_in(&file, prefs)
}

/// Directory holding `prefs.json`: `$LEDGELINE_CONFIG_DIR` when set and
/// non-empty, else `dirs::config_dir()/ledgeline`. `None` only if the platform
/// exposes no config dir and no override is set.
///
/// Byte-for-byte the same rule as `recents::config_dir`. The two stores are
/// siblings in one directory, so a user who redirects one and not the other has
/// a bug, not a feature.
fn config_dir() -> Option<PathBuf> {
    match std::env::var_os(CONFIG_DIR_ENV) {
        Some(dir) if !dir.is_empty() => Some(PathBuf::from(dir)),
        _ => dirs::config_dir().map(|base| base.join(APP_DIR)),
    }
}

/// Full path to the preferences store, if a config directory is available.
fn prefs_file() -> Option<PathBuf> {
    config_dir().map(|dir| dir.join(PREFS_FILE))
}

/// What the store at a given path currently holds. Mirrors `recents::Stored`,
/// and for the same reason: only a genuine absence may be treated as "no
/// settings", because only a genuine absence has nothing to lose.
enum Stored {
    /// No file there.
    Missing,
    /// A well-formed preferences object.
    Settings(Prefs),
    /// Present, but unreadable or not a preferences object. Distinguished from
    /// [`Missing`](Self::Missing) precisely so it is never silently overwritten.
    Corrupt,
}

/// Read the store at `file`, moving it aside and returning defaults if it is
/// present but unreadable.
pub(crate) fn load_from(file: &Path) -> Prefs {
    match read_stored(file) {
        Stored::Settings(prefs) => prefs,
        Stored::Missing => Prefs::default(),
        Stored::Corrupt => {
            set_corrupt_aside(file);
            Prefs::default()
        }
    }
}

/// Validate `prefs` and write it to `file`.
///
/// Validation happens BEFORE the corrupt check and before any directory is
/// created, so a rejected value leaves the filesystem exactly as it found it.
///
/// # Errors
/// See [`store`], which is this with the path resolved from the environment.
pub(crate) fn store_in(file: &Path, prefs: &Prefs) -> Result<(), PrefsError> {
    if let Some(path) = prefs.hledger_path.as_deref() {
        check_executable(path).map_err(|reason| PrefsError::InvalidHledgerPath { reason })?;
    }
    // `load` normally moves a corrupt store aside long before we get here, but
    // not every caller loads first and the file may have been replaced in
    // between. `atomic_write` below is a wholesale replacement, so this is the
    // last chance to preserve bytes we could not parse.
    if matches!(read_stored(file), Stored::Corrupt) {
        set_corrupt_aside(file);
    }
    if let Some(parent) = file.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(prefs)?;
    ledgeline_core::edit::atomic_write(file, json.as_bytes())?;
    Ok(())
}

/// Classify the store at `file`.
fn read_stored(file: &Path) -> Stored {
    let Ok(text) = std::fs::read_to_string(file) else {
        // Absent, unreadable, or not UTF-8. Only a genuine absence is safe to
        // treat as "no settings"; anything else is content we failed to read.
        return if file.exists() {
            Stored::Corrupt
        } else {
            Stored::Missing
        };
    };
    serde_json::from_str::<Prefs>(&text).map_or(Stored::Corrupt, Stored::Settings)
}

/// Rename an unreadable store to `prefs.json.corrupt` so its bytes survive for
/// recovery and the next write starts from a clean file.
fn set_corrupt_aside(file: &Path) {
    let aside = file.with_extension("json.corrupt");
    match std::fs::rename(file, &aside) {
        Ok(()) => eprintln!(
            "ledgeline: preferences file {} was unreadable; kept a copy at {} and started fresh",
            file.display(),
            aside.display()
        ),
        Err(error) => eprintln!(
            "ledgeline: could not set aside unreadable preferences {}: {error}",
            file.display()
        ),
    }
}

/// Whether `path` is a binary we could actually execute — the check both this
/// module (at store time) and [`hledger`](crate::hledger) (at resolve time) use,
/// so "a valid hledger path" means one thing in this codebase.
pub(crate) fn is_executable_file(path: &Path) -> bool {
    check_executable(path).is_ok()
}

/// [`is_executable_file`] with the reason attached, for the error a rejected
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
