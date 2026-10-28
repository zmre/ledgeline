//! Where the APP keeps its own state — the `AppDirs` seam (plan 26, A2).
//!
//! Everything here is app-local: the preferences and recents stores, the
//! staging area for dropped statements, a future cache, and the `hledger`
//! override. None of it is journal content, so none of it goes through the
//! [`Vault`](ledgeline_core::vault::Vault) — a vault is the user's books, and
//! these are the application's own files beside them.
//!
//! The struct is INJECTED rather than looked up where it is used. Before this
//! seam `prefs.rs`, `recents.rs`, `stage.rs` and `hledger.rs` each asked the
//! process environment and the `dirs` crate on their own, which is the right
//! answer for a desktop process and the wrong one for a sandboxed host: an iOS
//! app's writable directories are whatever its container hands it, and it has
//! no `$LEDGELINE_CONFIG_DIR` or `$LEDGELINE_HLEDGER` to consult. So:
//!
//! * the desktop builds one with [`AppDirs::from_env`] — exactly the lookups
//!   those modules used to make, unchanged — and threads it through;
//! * a sandboxed host builds one from its own container paths
//!   ([`AppDirs::under`], or the fields directly) and hands it to
//!   [`AppState::with_app_dirs`](crate::AppState::with_app_dirs).
//!
//! This file depends on nothing in the crate but the standard library, `dirs`
//! and `ledgeline_core`, because `tests/prefs.rs` compiles it in by `#[path]`
//! alongside `prefs.rs` and `hledger.rs`.

use ledgeline_core::hledger_conf::UserConfDirs;
use std::path::{Path, PathBuf};

/// Env override for the directory holding `prefs.json` and `recent.json`. One
/// variable moves the whole config, on purpose.
pub const CONFIG_DIR_ENV: &str = "LEDGELINE_CONFIG_DIR";
/// Env override naming an `hledger` binary.
pub const HLEDGER_ENV: &str = "LEDGELINE_HLEDGER";
/// Env switch that keeps the desktop from reading `~/.hledger.conf` and the XDG
/// `hledger.conf`, the user-level tiers of hledger's config lookup. Set by the
/// live integration and e2e harnesses, whose server must never pick up the
/// developer's own dotfiles. Any non-empty value turns the tiers off.
pub const IGNORE_USER_CONF_ENV: &str = "LEDGELINE_IGNORE_USER_HLEDGER_CONF";
/// Application subdirectory under the OS config and cache dirs.
const APP_DIR: &str = "ledgeline";

/// The app's own directories, plus the one app-level binary override.
///
/// Plain public fields: a host either takes [`from_env`](Self::from_env)
/// wholesale or states every directory it means, and a builder would add
/// nothing to that.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppDirs {
    /// Where `prefs.json` and `recent.json` live. `None` when the platform
    /// exposes no config directory and nothing overrides it — persistence is
    /// then disabled (recents silently, preferences with an error on save).
    pub config_dir: Option<PathBuf>,
    /// Where regenerable data may be kept. Nothing writes here yet; it is part
    /// of the seam so the first cache does not have to invent its own lookup.
    pub cache_dir: Option<PathBuf>,
    /// The parent of the per-session staging root (`ledgeline-stage-<rand>`,
    /// mode `0700`). Must exist and be writable.
    pub temp_dir: PathBuf,
    /// An explicit `hledger` binary — resolution step 2, after the stored
    /// preference. `None` to skip that step. Meaningless in a build without the
    /// `external-tools` feature, where hledger never runs.
    pub hledger_bin: Option<PathBuf>,
    /// `$HOME` and `$XDG_CONFIG_HOME`, for the last two tiers of hledger's
    /// `hledger.conf` lookup (`~/.hledger.conf`, then the XDG file). Empty —
    /// no user-level config is read — anywhere but the desktop binary, which
    /// is the only place those directories are the user's.
    pub hledger_conf: UserConfDirs,
}

impl AppDirs {
    /// The desktop's directories, from the environment and the OS — byte for
    /// byte the lookups the stores used to make on their own:
    ///
    /// * `config_dir`: `$LEDGELINE_CONFIG_DIR` when set and non-empty, else
    ///   `dirs::config_dir()/ledgeline`;
    /// * `cache_dir`: `dirs::cache_dir()/ledgeline`;
    /// * `temp_dir`: [`std::env::temp_dir`];
    /// * `hledger_bin`: `$LEDGELINE_HLEDGER`, if set and non-empty;
    /// * `hledger_conf`: `dirs::home_dir()` and `$XDG_CONFIG_HOME` (if set and
    ///   non-empty), unless `$LEDGELINE_IGNORE_USER_HLEDGER_CONF` is set.
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            config_dir: non_empty_env(CONFIG_DIR_ENV)
                .or_else(|| dirs::config_dir().map(|base| base.join(APP_DIR))),
            cache_dir: dirs::cache_dir().map(|base| base.join(APP_DIR)),
            temp_dir: std::env::temp_dir(),
            hledger_bin: non_empty_env(HLEDGER_ENV),
            hledger_conf: if non_empty_env(IGNORE_USER_CONF_ENV).is_some() {
                UserConfDirs::default()
            } else {
                UserConfDirs {
                    home: dirs::home_dir(),
                    xdg_config_home: non_empty_env("XDG_CONFIG_HOME"),
                }
            },
        }
    }

    /// Everything under one directory a sandboxed host owns: `config/`,
    /// `cache/` and `tmp/` beneath `root`, and no `hledger`. The directories
    /// are not created here; [`create_all`](Self::create_all) does that.
    #[must_use]
    pub fn under(root: impl AsRef<Path>) -> Self {
        let root = root.as_ref();
        Self {
            config_dir: Some(root.join("config")),
            cache_dir: Some(root.join("cache")),
            temp_dir: root.join("tmp"),
            hledger_bin: None,
            hledger_conf: UserConfDirs::default(),
        }
    }

    /// Create every directory this names that does not exist yet. The desktop
    /// never needs it (the stores create their own directory on first write,
    /// and the OS temp dir exists); a host using [`under`](Self::under) calls it
    /// once at startup.
    ///
    /// # Errors
    /// The first [`std::io::Error`] from creating a directory.
    pub fn create_all(&self) -> std::io::Result<()> {
        self.config_dir
            .iter()
            .chain(self.cache_dir.iter())
            .chain(std::iter::once(&self.temp_dir))
            .try_for_each(std::fs::create_dir_all)
    }

    /// The file `name` inside [`config_dir`](Self::config_dir), if there is one.
    #[must_use]
    pub fn config_file(&self, name: &str) -> Option<PathBuf> {
        self.config_dir.as_ref().map(|dir| dir.join(name))
    }
}

/// `var` as a path when it is set and non-empty. An empty override is treated
/// as unset, which is what every store did before this module existed.
fn non_empty_env(var: &str) -> Option<PathBuf> {
    std::env::var_os(var)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn under_places_everything_beneath_the_root_and_names_no_hledger() {
        let dirs = AppDirs::under("/container/Library");
        assert_eq!(
            dirs.config_dir.as_deref(),
            Some(Path::new("/container/Library/config"))
        );
        assert_eq!(
            dirs.cache_dir.as_deref(),
            Some(Path::new("/container/Library/cache"))
        );
        assert_eq!(dirs.temp_dir, Path::new("/container/Library/tmp"));
        assert_eq!(dirs.hledger_bin, None);
        assert_eq!(dirs.hledger_conf, UserConfDirs::default());
        assert_eq!(
            dirs.config_file("prefs.json").as_deref(),
            Some(Path::new("/container/Library/config/prefs.json"))
        );
    }

    #[test]
    fn create_all_makes_every_named_directory() {
        let root = tempfile::TempDir::new().expect("temp dir");
        let dirs = AppDirs::under(root.path().join("nested"));
        dirs.create_all().expect("create");
        assert!(dirs.temp_dir.is_dir());
        assert!(dirs.config_dir.as_deref().is_some_and(Path::is_dir));
        assert!(dirs.cache_dir.as_deref().is_some_and(Path::is_dir));
    }

    #[test]
    fn no_config_dir_means_no_config_file() {
        let dirs = AppDirs {
            config_dir: None,
            ..AppDirs::under("/x")
        };
        assert_eq!(dirs.config_file("recent.json"), None);
    }

    /// `from_env` reads this process's real environment, which the test
    /// harness does not control; what it can pin is that the temp dir is the
    /// OS one, as it always was.
    #[test]
    fn from_env_keeps_the_os_temp_dir() {
        assert_eq!(AppDirs::from_env().temp_dir, std::env::temp_dir());
    }
}
