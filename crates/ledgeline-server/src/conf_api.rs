//! `GET`/`PUT /api/hledger-conf` — Settings › hledger config (plan 30,
//! Decision 10 and Phase 4b).
//!
//! # What this reads, and where
//!
//! The config file in force is found the way hledger finds it
//! ([`hledger_conf::locate_conf_for`]): the journal's directory and each parent,
//! then `~/.hledger.conf`, then the XDG file — the last two only on the desktop,
//! where [`AppDirs::hledger_conf`](crate::AppDirs) names a real home. The import
//! path's `--alias` forwarding reads through the same locator, so both features
//! always agree about which file that is.
//!
//! Ledgeline READS the file and never delegates to it: every hledger subprocess
//! still passes `--no-conf` (see `hledger.rs`). What it honours is
//! [`ConfSettings`] — general-section `--auto` and per-report `--depth`.
//!
//! # Writing
//!
//! A write updates the file that was found, wherever it is (Patrick,
//! 2026-10-27), or — when there is none — creates `hledger.conf` in the
//! journal's directory with [`hledger_conf::new_file_header`]. It is the same
//! discipline as every other write target here:
//!
//! 1. **Editing must be enabled.**
//! 2. **No path comes from the client.** The target is whatever the locator
//!    found, or one fixed name in the journal's directory.
//! 3. **Regular files only.** The read before the write is pinned
//!    ([`Vault::read_pinned`], which does not follow links): a symlinked config
//!    (a dotfile manager's) is shown but reported read-only, never written
//!    through.
//! 4. **Revision / 409**, re-checked by the pinned replace itself.
//! 5. **Surgical.** [`hledger_conf::with_option`] / [`without_option`] change one
//!    option line each and leave every other byte alone; the result is re-read
//!    and must say exactly what was asked, or nothing is written. A hijacked
//!    file is refused with [`hledger_conf::hijack_sentence`].
//! 6. **The same lock as the alias write** (`import_writes`): a config file is
//!    read by every hledger invocation an import makes.
//!
//! A write republishes the snapshot, so the generation changes and every
//! client re-reads its reports — and Phase 5's `--auto` application re-runs.

use crate::AppState;
use crate::edit_api::json_body;
use crate::error::AppError;
use crate::location::JournalLocation;
use crate::reports_api::compute;
use crate::security::no_store_json;
use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::response::Response;
use ledgeline_core::edit::Fingerprint;
use ledgeline_core::hledger_conf::{
    self, ConfSettings, ConfSlot, ConfSource, DepthCommand, DepthSpec, LocatedConf, ManagedOption,
    Section, UserConfDirs, without_option,
};
use ledgeline_core::vault::{LocalVault, Revision, Vault, VaultError, VaultPath};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;

/// The deepest `--depth` this screen writes. The reports' own depth controls
/// stop at 99 too.
pub(crate) const MAX_DEPTH: u32 = 99;

/// How many `REGEX=N` overrides one section may be given in one write. A
/// bound on a request, not on a user: nobody hand-maintains more.
const MAX_OVERRIDES: usize = 50;

/// The wire key for the general section.
const GENERAL: &str = "general";

/// The config file in force for one journal, read once.
#[derive(Debug, Clone, Default)]
pub(crate) struct ConfState {
    /// Where it is, or `None` when no tier of the lookup found one.
    pub(crate) located: Option<LocatedConf>,
    /// Its text (lossy UTF-8), or empty.
    pub(crate) text: String,
    /// A fingerprint of `text`, or `""` when there is no file — the revision a
    /// write must echo.
    pub(crate) revision: String,
    /// What Ledgeline honours from it.
    pub(crate) settings: ConfSettings,
    /// The command word it forces on every hledger invocation, if any.
    pub(crate) hijacked_by: Option<String>,
    /// It was found but could not be read (past the size cap, vanished).
    pub(crate) unreadable: bool,
}

impl ConfState {
    /// Find and read the config file in force for `location`. Never an error:
    /// a file that cannot be read is reported as such and honoured as empty.
    pub(crate) fn read(location: Option<&JournalLocation>, user: &UserConfDirs) -> Self {
        let Some(location) = location else {
            return Self::default();
        };
        let Some(located) =
            hledger_conf::locate_conf_for(&**location.vault(), &location.root_dir(), user)
        else {
            return Self::default();
        };
        match located.read(&**location.vault()) {
            Ok(text) => Self {
                revision: Fingerprint::of_bytes(text.as_bytes()).token(),
                settings: ConfSettings::read(&text),
                hijacked_by: hledger_conf::hijacks_command(&text),
                located: Some(located),
                text,
                unreadable: false,
            },
            Err(_) => Self {
                located: Some(located),
                unreadable: true,
                ..Self::default()
            },
        }
    }

    /// Whether `other` describes a different file, or different bytes.
    pub(crate) fn differs_from(&self, other: &Self) -> bool {
        self.located != other.located || self.revision != other.revision
    }

    /// Every real path whose change should re-read this: the file in force,
    /// and the one a first write would create beside the journal. For the
    /// live-reload watcher.
    pub(crate) fn watch_paths(
        &self,
        location: Option<&JournalLocation>,
    ) -> Vec<std::path::PathBuf> {
        let Some(location) = location else {
            return Vec::new();
        };
        let in_force = self
            .located
            .as_ref()
            .and_then(|located| match &located.source {
                ConfSource::InVault(path) => location.vault().local_path(path),
                ConfSource::Native(path) => Some(path.clone()),
            });
        let beside = location
            .root_dir()
            .child(hledger_conf::CONF_NAME)
            .ok()
            .and_then(|path| location.vault().local_path(&path));
        in_force
            .into_iter()
            .chain(beside)
            .fold(Vec::new(), |mut paths, path| {
                if !paths.contains(&path) {
                    paths.push(path);
                }
                paths
            })
    }
}

// ---------------------------------------------------------------------------
// Wire
// ---------------------------------------------------------------------------

/// One section's `--depth` options.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WireDepth {
    /// The plain limit, or `null`.
    all: Option<u32>,
    /// `REGEX=N` overrides.
    #[serde(default)]
    per_regex: Vec<WireRegexDepth>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WireRegexDepth {
    regex: String,
    depth: u32,
}

impl From<&DepthSpec> for WireDepth {
    fn from(spec: &DepthSpec) -> Self {
        Self {
            all: spec.all,
            per_regex: spec
                .per_regex
                .iter()
                .map(|(regex, depth)| WireRegexDepth {
                    regex: regex.clone(),
                    depth: *depth,
                })
                .collect(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct WireWarning {
    code: &'static str,
    message: String,
}

/// `GET /api/hledger-conf`, and the answer to a `PUT`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct WireConf {
    /// The file in force: `hledger.conf`, `../hledger.conf`, `~/.hledger.conf`
    /// or `~/.config/hledger/hledger.conf` — never an absolute path. `null`
    /// when there is none.
    handle: Option<String>,
    /// Which tier found it: `ledger`, `home` or `xdg`; `null` with `handle`.
    origin: Option<&'static str>,
    /// Where a first write would create one, relative to the journal's
    /// directory (`hledger.conf`). `null` when a file is in force, or when this
    /// server has nowhere to create one.
    create_handle: Option<String>,
    /// A write would land. False on a read-only server, a symlinked or
    /// oversized file, or a hijacked one.
    editable: bool,
    /// Why not, when `editable` is false. `null` otherwise.
    read_only_reason: Option<String>,
    /// The hijack sentence ([`hledger_conf::hijack_sentence`]), or `null`.
    hijacked: Option<String>,
    /// The revision a `PUT` must echo: `""` for no file.
    revision: String,
    /// General-section `--auto`.
    auto: bool,
    /// Each section's options as written: `general` and the four report
    /// sections, every key always present.
    depth: BTreeMap<&'static str, WireDepth>,
    /// What hledger applies to each report command: general, then its section.
    effective_depth: BTreeMap<&'static str, WireDepth>,
    warnings: Vec<WireWarning>,
}

/// `PUT /api/hledger-conf`. Absent fields are left as they are.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WireConfUpdate {
    revision: String,
    /// `true` writes `--auto` into the general section, `false` removes it.
    #[serde(default)]
    auto: Option<bool>,
    /// For each section named (`general`, `balancesheet`, `incomestatement`,
    /// `cashflow`, `balance`), what its `--depth` options should become: the
    /// plain limit set or cleared, and exactly these overrides.
    #[serde(default)]
    depth: BTreeMap<String, WireDepth>,
}

// ---------------------------------------------------------------------------
// Where a write lands
// ---------------------------------------------------------------------------

/// The file a write would replace or create, read pinned.
struct Target {
    vault: Arc<dyn Vault>,
    path: VaultPath,
    /// Present. A write replaces it (pinned to `read`); absent, it is created.
    exists: bool,
    /// What a pinned read saw — identity and bytes — for the replace.
    read: Option<Revision>,
    /// Why no write can land here, or `None`.
    refusal: Option<String>,
}

fn target(location: &JournalLocation, conf: &ConfState) -> Target {
    let (vault, path): (Arc<dyn Vault>, Option<VaultPath>) = match &conf.located {
        None => (
            Arc::clone(location.vault()),
            location.root_dir().child(hledger_conf::CONF_NAME).ok(),
        ),
        Some(located) => match &located.source {
            ConfSource::InVault(path) => (Arc::clone(location.vault()), Some(path.clone())),
            ConfSource::Native(path) => {
                // A vault rooted at the file's own directory, so the same
                // pinned read and replace guard it as guard every in-vault
                // write: no symlink followed, no FIFO opened, no file swapped
                // in between the read and the rename.
                let directory = path.parent().map(LocalVault::new);
                let name = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .and_then(|name| VaultPath::new(name).ok());
                match (directory, name) {
                    (Some(directory), Some(name)) => (Arc::new(directory), Some(name)),
                    _ => (Arc::clone(location.vault()), None),
                }
            }
        },
    };
    let Some(path) = path else {
        return Target {
            vault,
            path: VaultPath::root(),
            exists: false,
            read: None,
            refusal: Some("this journal has no directory a config file can be written in".into()),
        };
    };
    let not_regular = || {
        Some(
            "it is not a regular file (a symlink — as a dotfile manager makes — a directory, or \
             something else), so Ledgeline will not write through it. Edit it where it points."
                .to_string(),
        )
    };
    let (exists, read, refusal) = match vault.read_pinned(&path, Some(hledger_conf::MAX_CONF_BYTES))
    {
        Ok((_, read)) => (true, Some(read), None),
        Err(error) if error.is_not_found() && conf.located.is_none() => (false, None, None),
        Err(error) if error.kind() == std::io::ErrorKind::FileTooLarge => (
            true,
            None,
            Some(format!(
                "it is larger than the {} KiB Ledgeline reads",
                hledger_conf::MAX_CONF_BYTES / 1024
            )),
        ),
        Err(error) if error.is_not_regular_file() => (true, None, not_regular()),
        Err(error) => (
            true,
            None,
            Some(format!("it could not be read ({})", error.kind())),
        ),
    };
    let refusal = refusal.or_else(|| {
        conf.hijacked_by
            .as_deref()
            .map(hledger_conf::hijack_sentence)
    });
    Target {
        vault,
        path,
        exists,
        read,
        refusal,
    }
}

fn body(state: &AppState, location: Option<&JournalLocation>, conf: &ConfState) -> WireConf {
    let target = location.map(|location| target(location, conf));
    let refusal = if !state.editing_enabled() {
        Some("editing is not enabled on this server".to_string())
    } else if conf.unreadable {
        Some("the config file in force could not be read".to_string())
    } else {
        match &target {
            None => {
                Some("this journal has no directory a config file can be written in".to_string())
            }
            Some(target) => target.refusal.clone(),
        }
    };
    let settings = &conf.settings;
    let depth = std::iter::once((GENERAL, WireDepth::from(&settings.general)))
        .chain(DepthCommand::ALL.into_iter().map(|command| {
            (
                command.name(),
                WireDepth::from(&settings.written(Some(command))),
            )
        }))
        .collect();
    let effective_depth = DepthCommand::ALL
        .into_iter()
        .map(|command| (command.name(), WireDepth::from(&settings.depth(command))))
        .collect();
    WireConf {
        handle: conf.located.as_ref().map(LocatedConf::handle),
        origin: conf.located.as_ref().map(|located| located.origin.code()),
        create_handle: (conf.located.is_none()
            && target.as_ref().is_some_and(|t| t.refusal.is_none()))
        .then(|| hledger_conf::CONF_NAME.to_string()),
        editable: refusal.is_none(),
        read_only_reason: refusal,
        hijacked: conf
            .hijacked_by
            .as_deref()
            .map(hledger_conf::hijack_sentence),
        revision: conf.revision.clone(),
        auto: settings.auto,
        depth,
        effective_depth,
        warnings: settings
            .warnings
            .iter()
            .map(|warning| WireWarning {
                code: warning.code(),
                message: warning.message(),
            })
            .collect(),
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `GET /api/hledger-conf` — the config file in force and what Ledgeline reads
/// from it.
///
/// Read FRESH from disk, not from the snapshot: a file outside the watched
/// directories (`~/.hledger.conf`) has no watcher behind it, and a stale answer
/// here would hand the editor a revision every write then refuses. When the
/// fresh read differs from the one the snapshot holds, the snapshot is
/// republished, so reports catch up too — the poll half of "an external change
/// refreshes".
pub(crate) async fn get(State(state): State<AppState>) -> Result<Response, AppError> {
    let Json(body) = compute(move || {
        let conf = state.refresh_conf();
        let location = state.snapshot().storage();
        Ok(body(&state, location.as_deref(), &conf))
    })
    .await?;
    Ok(no_store_json(body))
}

/// `PUT /api/hledger-conf` — set or clear `--auto` and per-section `--depth`.
pub(crate) async fn put(
    State(state): State<AppState>,
    payload: Result<Json<WireConfUpdate>, JsonRejection>,
) -> Result<Response, AppError> {
    let request = json_body(payload)?;
    if !state.editing_enabled() {
        return Err(crate::error::editing_disabled());
    }
    // The alias write's lock: a config file is read by every hledger command
    // an import runs, so it must not change under one.
    let guard = state.clone();
    let _write = guard.import_writes().lock().await;
    let Json(body) = compute(move || run_put(&state, &request)).await?;
    Ok(no_store_json(body))
}

fn run_put(state: &AppState, request: &WireConfUpdate) -> Result<WireConf, AppError> {
    let location = state
        .snapshot()
        .storage()
        .ok_or_else(|| AppError::NotFound("no journal is open".to_string()))?;
    let conf = ConfState::read(Some(&location), &state.app_dirs().hledger_conf);
    let handle = conf
        .located
        .as_ref()
        .map_or_else(|| hledger_conf::CONF_NAME.to_string(), LocatedConf::handle);
    if conf.revision != request.revision {
        return Err(stale(&handle));
    }
    if conf.unreadable {
        return Err(AppError::BadRequest(format!(
            "{handle} could not be read, so nothing was written"
        )));
    }
    let target = target(&location, &conf);
    if let Some(reason) = &target.refusal {
        return Err(AppError::BadRequest(format!(
            "{handle} cannot be edited: {reason}"
        )));
    }

    let base = if target.exists {
        conf.text.clone()
    } else {
        hledger_conf::new_file_header()
    };
    let next = edited(&base, request)?;
    if next == base {
        // Nothing to change writes NOTHING, not even identical bytes: a
        // rewrite still bumps mtime and wakes somebody's watcher.
        return Ok(body(state, Some(&location), &conf));
    }

    let written = if target.exists {
        match &target.read {
            Some(read) => target
                .vault
                .replace(&target.path, next.as_bytes(), read)
                .map(|_| ()),
            None => Err(VaultError::Changed {
                path: target.path.to_string(),
            }),
        }
    } else {
        target
            .vault
            .create_new(&target.path, next.as_bytes())
            .map(|_| ())
            .map_err(|error| {
                if error.kind() == std::io::ErrorKind::AlreadyExists {
                    VaultError::Changed {
                        path: target.path.to_string(),
                    }
                } else {
                    error
                }
            })
    };
    written.map_err(|error| match error {
        VaultError::Changed { .. } => stale(&handle),
        // Only the kind: a write error can carry a temp path.
        other => AppError::Internal(format!(
            "{handle} could not be written: {}. Nothing else was changed.",
            other.kind()
        )),
    })?;

    // Republish so reports and the `--auto` application see the new file, then
    // answer from the bytes WRITTEN — a re-read could pick up somebody else's
    // write and hand this client a revision for bytes it never saw.
    let written = ConfState::read(Some(&location), &state.app_dirs().hledger_conf);
    let mine = Fingerprint::of_bytes(next.as_bytes()).token();
    let answer = if written.revision == mine {
        written
    } else {
        ConfState {
            located: written.located,
            revision: mine,
            settings: ConfSettings::read(&next),
            hijacked_by: hledger_conf::hijacks_command(&next),
            text: next,
            unreadable: false,
        }
    };
    state.republish_with_conf(answer.clone());
    Ok(body(state, Some(&location), &answer))
}

fn stale(handle: &str) -> AppError {
    AppError::Conflict(format!(
        "{handle} changed on disk since this page read it, so nothing was written. Reload and try \
         again."
    ))
}

/// `text` with the request's edits applied, re-read and checked: the result
/// must say exactly what was asked, or nothing is written.
fn edited(text: &str, request: &WireConfUpdate) -> Result<String, AppError> {
    let refused = |error: hledger_conf::ConfWriteError| AppError::BadRequest(error.to_string());
    let mut next = text.to_string();
    if let Some(auto) = request.auto {
        next = if auto {
            hledger_conf::with_option(&next, &Section::General, &ManagedOption::Auto)
        } else {
            without_option(&next, &Section::General, &ConfSlot::Auto)
        }
        .map_err(refused)?;
    }
    let sections = request
        .depth
        .iter()
        .map(|(key, wanted)| Ok((section_of(key)?, wanted)))
        .collect::<Result<Vec<_>, AppError>>()?;
    for ((section, command), wanted) in &sections {
        validate(wanted)?;
        next = match wanted.all {
            Some(depth) => hledger_conf::with_option(&next, section, &ManagedOption::Depth(depth)),
            None => without_option(&next, section, &ConfSlot::Depth),
        }
        .map_err(refused)?;
        let dropped: Vec<String> = ConfSettings::read(&next)
            .written(*command)
            .per_regex
            .into_iter()
            .map(|(regex, _)| regex)
            .filter(|regex| !wanted.per_regex.iter().any(|keep| &keep.regex == regex))
            .collect();
        for regex in dropped {
            next = without_option(&next, section, &ConfSlot::DepthFor(regex)).map_err(refused)?;
        }
        for keep in &wanted.per_regex {
            next = hledger_conf::with_option(
                &next,
                section,
                &ManagedOption::DepthFor {
                    regex: keep.regex.clone(),
                    depth: keep.depth,
                },
            )
            .map_err(refused)?;
        }
    }

    // Read back. Anything other than exactly what was asked — an option hledger
    // would read differently than we wrote it — writes nothing.
    let read = ConfSettings::read(&next);
    let auto_ok = request.auto.is_none_or(|auto| read.auto == auto);
    let depth_ok = sections.iter().all(|((_, command), wanted)| {
        let got = read.written(*command);
        got.all == wanted.all
            && got.per_regex.len() == wanted.per_regex.len()
            && wanted
                .per_regex
                .iter()
                .all(|keep| got.per_regex.contains(&(keep.regex.clone(), keep.depth)))
    });
    if !(auto_ok && depth_ok) {
        return Err(AppError::BadRequest(
            "that change could not be written so that hledger reads it back the same way, so \
             nothing was written"
                .to_string(),
        ));
    }
    Ok(next)
}

fn section_of(key: &str) -> Result<(Section, Option<DepthCommand>), AppError> {
    if key == GENERAL {
        return Ok((Section::General, None));
    }
    DepthCommand::from_name(key)
        .map(|command| (Section::Command(command.name().to_string()), Some(command)))
        .ok_or_else(|| {
            AppError::BadRequest(format!(
                "{key:?} is not a section this screen writes: use general, balancesheet, \
                 incomestatement, cashflow or balance"
            ))
        })
}

fn validate(wanted: &WireDepth) -> Result<(), AppError> {
    let too_deep = |depth: u32| depth > MAX_DEPTH;
    if wanted.all.is_some_and(too_deep) || wanted.per_regex.iter().any(|keep| too_deep(keep.depth))
    {
        return Err(AppError::BadRequest(format!(
            "a depth must be between 0 and {MAX_DEPTH}"
        )));
    }
    if wanted.per_regex.len() > MAX_OVERRIDES {
        return Err(AppError::BadRequest(format!(
            "at most {MAX_OVERRIDES} per-account depths per section"
        )));
    }
    for (at, keep) in wanted.per_regex.iter().enumerate() {
        if wanted.per_regex[..at]
            .iter()
            .any(|other| other.regex == keep.regex)
        {
            return Err(AppError::BadRequest(format!(
                "the account pattern {:?} is listed twice",
                keep.regex
            )));
        }
        if let Some(reason) = hledger_conf::regex_refusal(&keep.regex) {
            return Err(AppError::BadRequest(format!(
                "the account pattern {:?} cannot be written: {reason}",
                keep.regex
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn update(json: &str) -> WireConfUpdate {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn an_edit_is_applied_and_read_back() {
        let next = edited(
            "# mine\n--forecast\n",
            &update(
                r#"{"revision":"","auto":true,"depth":{"general":{"all":3},
                    "balancesheet":{"all":null,"perRegex":[{"regex":"assets:inv","depth":2}]}}}"#,
            ),
        )
        .unwrap();
        assert_eq!(
            next,
            "# mine\n--forecast\n--auto\n--depth 3\n\n[balancesheet]\n--depth assets:inv=2\n"
        );
    }

    #[test]
    fn overrides_not_asked_for_are_removed_and_the_rest_kept_in_place() {
        let next = edited(
            "[cashflow]\n--depth a=1 # keep me\n--depth b=2\n",
            &update(r#"{"revision":"","depth":{"cashflow":{"all":null,"perRegex":[{"regex":"a","depth":4}]}}}"#),
        )
        .unwrap();
        assert_eq!(next, "[cashflow]\n--depth a=4 # keep me\n");
    }

    #[test]
    fn a_bad_request_writes_nothing() {
        for json in [
            r#"{"revision":"","depth":{"bs":{"all":1}}}"#,
            r#"{"revision":"","depth":{"general":{"all":100}}}"#,
            r#"{"revision":"","depth":{"general":{"all":null,"perRegex":[{"regex":"a b","depth":1}]}}}"#,
            r#"{"revision":"","depth":{"general":{"all":null,"perRegex":[{"regex":"a","depth":1},{"regex":"a","depth":2}]}}}"#,
        ] {
            assert!(
                matches!(edited("", &update(json)), Err(AppError::BadRequest(_))),
                "{json}"
            );
        }
        assert!(matches!(
            edited("print\n", &update(r#"{"revision":"","auto":true}"#)),
            Err(AppError::BadRequest(message)) if message.contains("`print`")
        ));
    }

    #[test]
    fn an_empty_request_changes_nothing() {
        let text = "--auto\n";
        assert_eq!(edited(text, &update(r#"{"revision":""}"#)).unwrap(), text);
    }
}
