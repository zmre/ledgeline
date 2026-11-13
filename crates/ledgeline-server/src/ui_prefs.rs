//! UI preferences, kept in files (plan 38, WP-5; save routing reworked
//! 2026-11-12).
//!
//! The SPA's view state — which columns show, which panels are open, the
//! Review and Find Matches numbers, the benchmark lines — used to live in the
//! browser's `localStorage`. The desktop app's WebView origin includes a port
//! that changes on every launch, so every relaunch forgot all of it. It now
//! lives in two TOML files the engine reads, merged per key:
//!
//! 1. `ledgeline.toml` in the MAIN JOURNAL'S DIRECTORY, keys under `[ui]`. A
//!    person writes this file by hand to give one ledger its own values. The
//!    app creates it in one case only: the user asks to keep a [`PINNABLE`]
//!    key for this ledger ([`pin`], plan 39).
//! 2. `ui-prefs.toml` in the config directory ([`AppDirs::config_dir`]), the
//!    same keys at top level. The app's own file: it creates it on the first
//!    save and edits it after.
//! 3. The built-in defaults ([`SPECS`]).
//!
//! For each key the first layer that holds a USABLE value wins. A value that is
//! present but unusable is reported as an [`Issue`] and the key falls through
//! to the next layer, so one bad line never costs the user the rest.
//!
//! # Where a save goes
//!
//! Per key, and per leaf for a group (`columns.status`): a changed key is
//! written to `ledgeline.toml` only when that file already exists and its
//! `[ui]` table already sets that key to a value; everything else goes to
//! `ui-prefs.toml`. So a ledger overrides exactly the keys someone put in its
//! file by hand (or pinned, see [`pin`]), those keys keep being saved there,
//! and opening or using the app never makes a file appear in somebody's
//! repository unless they asked for it. Reading never writes anything.
//!
//! # `ledgeline.toml` is untrusted
//!
//! It sits beside the journal, so it travels with a cloned or shared repository
//! and may have been written by someone else. It may therefore hold UI
//! preferences ONLY, and nothing read from it is ever executed or
//! security-relevant:
//!
//! - It is never consulted for `hledgerPath` (that is `prefs.toml`'s, in the
//!   user's own config directory) or for the server URL or token (the
//!   browser's). Those keys are reported as ignored when present.
//! - Every value is validated on load: numbers are clamped to their range,
//!   choices are checked against their closed set, strings are length-capped
//!   and refused with control characters, ticker lists are filtered.
//! - The file is capped at [`MAX_FILE_BYTES`] and read without following a
//!   symlink (`Vault::read_pinned`, `O_NOFOLLOW` on the desktop). A symlinked,
//!   oversized or unparseable file is ignored on read and never written: it
//!   sets no key, so every save goes to the global file.
//! - Unknown keys are ignored, and kept on write.
//!
//! # Writes
//!
//! Both files are written through the pinned primitives
//! (`ledgeline_core::pinned`, through the journal's vault for the ledger
//! file): `replace` against the revision just read, and for the global file
//! `create_new_pinned` (exclusive, `O_NOFOLLOW`, owner-only) the first time. A
//! write that loses a race re-reads and re-applies — a patch is a set of key
//! assignments, so applying it again to newer bytes is exactly right — up to
//! [`WRITE_ATTEMPTS`] times. The edit goes through `toml_edit`'s document
//! model, so comments, key order and unknown keys survive. A file that exists
//! but cannot be parsed is never replaced.
//!
//! `ui-prefs.toml` is READ through a symlink, because dotfile managers
//! (home-manager, stow) deliver it as one. It is never WRITTEN through one:
//! home-manager links into the read-only Nix store, and replacing the link
//! would quietly take the file out of the manager's hands. A save meant for a
//! symlinked global file — or for a host with no config directory — is kept
//! in memory for the life of the process, over the global file, and reported
//! as an [`Issue`].
//!
//! # Where the preferences live, per session ([`Storage`])
//!
//! - `files`: the two files above — the normal case, with or without a
//!   journal file (with none, there is no `ledgeline.toml` to read).
//! - `browser`: the headless server was started with
//!   `$LEDGELINE_UI_PREFS_IN_BROWSER` set. Built-in defaults only, nothing is
//!   read or written, and the SPA keeps its view state in `localStorage` as it
//!   used to. For the e2e and live-contract harnesses, which run against
//!   committed fixtures and rely on each browser context starting from its own
//!   clean state.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;
use std::sync::{Mutex, PoisonError};

use ledgeline_core::vault::{Revision, VaultError, VaultPath};
use serde::Serialize;
use serde_json::{Map, Value as Json};
use toml_edit::{DocumentMut, InlineTable, Item, Table, TableLike};

use crate::app_dirs::AppDirs;
use crate::error::AppError;
use crate::{AppState, JournalLocation};

/// Env switch for [`Storage::Browser`]. Any non-empty value turns it on.
pub const UI_PREFS_IN_BROWSER_ENV: &str = "LEDGELINE_UI_PREFS_IN_BROWSER";

/// The per-ledger override file, beside the main journal. Hand-made, or
/// created by an explicit [`pin`].
pub(crate) const LEDGER_FILE: &str = "ledgeline.toml";

/// The global file, in the config directory: where saves normally go.
pub(crate) const GLOBAL_FILE: &str = "ui-prefs.toml";

/// The top of a [`GLOBAL_FILE`] the app creates.
const GLOBAL_HEADER: &str = concat!(
    "# Ledgeline's view settings for every ledger. Ledgeline saves the keys you\n",
    "# change here. A ledger's own ledgeline.toml overrides any key its [ui] table\n",
    "# sets. Comments and keys Ledgeline does not know are kept. Docs: \"Settings files\".\n",
    "\n",
);

/// Why a save was kept in memory rather than written to [`GLOBAL_FILE`].
const KEPT_SYMLINK: &str = "is a symbolic link, so it is managed outside Ledgeline; view settings you change are kept until Ledgeline quits";
const KEPT_NO_CONFIG_DIR: &str =
    "has no config folder to live in; view settings you change are kept until Ledgeline quits";

/// The table in [`LEDGER_FILE`] the UI keys live under, leaving the rest of
/// the file for later per-ledger settings.
const UI_TABLE: &str = "ui";

/// The largest settings file read. Real ones are a few hundred bytes.
pub(crate) const MAX_FILE_BYTES: u64 = 64 * 1024;

/// The most benchmark lines kept. The catalog is a handful of symbols.
const MAX_BENCHMARKS: usize = 32;

/// The longest scenario file id kept (a relative path).
const MAX_FILE_ID_BYTES: usize = 1024;

/// Tries at a write that keeps losing a race with another writer.
const WRITE_ATTEMPTS: usize = 3;

/// Keys that must never be honoured from [`LEDGER_FILE`], named so a file that
/// tries is told so rather than silently ignored.
const FORBIDDEN_LEDGER_KEYS: &[&str] = &["hledgerPath", "serverUrl", "serverToken"];

/// The insights box's two views (`InsightsTab` in `settings.svelte.ts`).
const INSIGHTS_TABS: &[&str] = &["activity", "balances"];

/// What the Holdings pie can be divided by (`PIE_DIMENSIONS` in
/// `web/src/lib/holdings/profileTypes.ts`).
const PIE_DIMENSIONS: &[&str] = &[
    "holding",
    "assetClass",
    "sector",
    "industry",
    "securityType",
    "category",
    "risk",
];

/// Theme ids (plan 39). A family id (`github`) follows the OS light/dark
/// setting; a variant id (`github-dark`) pins one; dark-only families have one
/// id. The SPA's `THEME_IDS` (`web/src/lib/settings/uiPrefs.ts`) is the same
/// list.
const THEMES: &[&str] = &[
    "default",
    "default-light",
    "default-dark",
    "github",
    "github-light",
    "github-dark",
    "catppuccin",
    "catppuccin-latte",
    "catppuccin-mocha",
    "gruvbox",
    "gruvbox-light",
    "gruvbox-dark",
    "tokyo-night",
    "tokyo-night-day",
    "tokyo-night-night",
    "solarized",
    "solarized-light",
    "solarized-dark",
    "one-dark-pro",
    "dracula",
    "monokai",
    "nord",
];

/// The keys [`pin`] may write into [`LEDGER_FILE`]: what a person would want
/// to tell one ledger from another by.
const PINNABLE: &[&str] = &["theme", "uiScale"];

/// What a key holds, how a value for it is checked, and its built-in default.
#[derive(Debug, Clone, Copy)]
enum Kind {
    /// `true`/`false`.
    Flag(bool),
    /// One string out of a closed set.
    Choice {
        allowed: &'static [&'static str],
        default: &'static str,
    },
    /// A whole number in `min..=max`.
    Whole { min: i64, max: i64, default: i64 },
    /// A scenario file's relative id, or none (`""` in a file, `null` on the
    /// wire). The routes that open it re-validate it as a path; this only
    /// bounds it.
    FileId,
    /// Benchmark ticker symbols, deduplicated, in order.
    Tickers,
}

/// One preference. `key` is the wire name; a dotted key (`columns.date`) is a
/// member of a group, nested on the wire and as a sub-table in the files.
#[derive(Debug)]
struct Spec {
    key: &'static str,
    kind: Kind,
}

/// Every preference, with the defaults `settings.svelte.ts` has always used.
/// The bounds are the SPA's (`REVIEW_LIMIT_RANGE` and friends).
const SPECS: &[Spec] = &[
    flag("columns.date", true),
    flag("columns.status", true),
    flag("columns.description", true),
    flag("columns.accounts", true),
    flag("columns.amount", true),
    flag("insightsOpen", true),
    Spec {
        key: "insightsTab",
        kind: Kind::Choice {
            allowed: INSIGHTS_TABS,
            default: "activity",
        },
    },
    flag("hideZeroBalances", true),
    flag("flowsInOpen", true),
    flag("flowsOutOpen", true),
    flag("netWorthChartOpen", true),
    flag("cashFlowChartOpen", true),
    flag("budgetGapsOpen", false),
    flag("startupMoreOpen", false),
    Spec {
        key: "holdingsPieDimension",
        kind: Kind::Choice {
            allowed: PIE_DIMENSIONS,
            default: "holding",
        },
    },
    Spec {
        key: "lastProjectionId",
        kind: Kind::FileId,
    },
    Spec {
        key: "benchmarks",
        kind: Kind::Tickers,
    },
    whole("reviewRecentLimit", 1, 1000, 50),
    whole("reviewMaxDays", 0, 36500, 365),
    flag("reviewMarkers", true),
    whole("matchWindow", 1, 60, 10),
    whole("matchLookback", 0, 36500, 365),
    whole("matchTransitDays", 1, 365, 10),
    Spec {
        key: "theme",
        kind: Kind::Choice {
            allowed: THEMES,
            default: "default",
        },
    },
    // Whole percent. The SPA steps 70–200; the wider range is for hand edits.
    whole("uiScale", 50, 300, 100),
];

const fn flag(key: &'static str, default: bool) -> Spec {
    Spec {
        key,
        kind: Kind::Flag(default),
    }
}

const fn whole(key: &'static str, min: i64, max: i64, default: i64) -> Spec {
    Spec {
        key,
        kind: Kind::Whole { min, max, default },
    }
}

/// The spec named `key`, if any.
fn spec(key: &str) -> Option<&'static Spec> {
    SPECS.iter().find(|spec| spec.key == key)
}

/// A checked preference value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PrefValue {
    Flag(bool),
    Text(String),
    Whole(i64),
    FileId(Option<String>),
    Tickers(Vec<String>),
}

/// A complete or partial set of values, by [`Spec::key`].
type Values = BTreeMap<&'static str, PrefValue>;

impl Kind {
    fn default_value(self) -> PrefValue {
        match self {
            Self::Flag(default) => PrefValue::Flag(default),
            Self::Choice { default, .. } => PrefValue::Text(default.to_string()),
            Self::Whole { default, .. } => PrefValue::Whole(default),
            Self::FileId => PrefValue::FileId(None),
            Self::Tickers => PrefValue::Tickers(Vec::new()),
        }
    }

    /// A value read from a FILE: lenient where leniency loses nothing (a
    /// number is clamped, a bad ticker dropped), with a note saying so. `Err`
    /// is why the whole key is unusable.
    fn check_toml(self, item: &Item) -> Result<(PrefValue, Option<&'static str>), &'static str> {
        let value = item
            .as_value()
            .ok_or("must be a single value, not a table")?;
        match self {
            Self::Flag(_) => value
                .as_bool()
                .map(|flag| (PrefValue::Flag(flag), None))
                .ok_or("must be true or false"),
            Self::Choice { allowed, .. } => value
                .as_str()
                .filter(|choice| allowed.contains(choice))
                .map(|choice| (PrefValue::Text(choice.to_string()), None))
                .ok_or("is not one of the accepted values"),
            Self::Whole { min, max, .. } => {
                let number = value
                    .as_integer()
                    .or_else(|| value.as_float().and_then(round_finite))
                    .ok_or("must be a whole number")?;
                let clamped = number.clamp(min, max);
                let note = (clamped != number).then_some("was out of range and has been clamped");
                Ok((PrefValue::Whole(clamped), note))
            }
            Self::FileId => {
                let text = value.as_str().ok_or("must be a string")?;
                check_file_id(text).map(|id| (PrefValue::FileId(id), None))
            }
            Self::Tickers => {
                let entries = value.as_array().ok_or("must be a list of ticker symbols")?;
                let tickers = entries
                    .iter()
                    .filter_map(toml_edit::Value::as_str)
                    .filter(|symbol| is_ticker(symbol));
                let kept = dedupe(tickers);
                let note = if kept.len() > MAX_BENCHMARKS {
                    Some("has too many entries; only the first 32 are used")
                } else if kept.len() < entries.len() {
                    Some("has entries that are not ticker symbols; they are ignored")
                } else {
                    None
                };
                let kept = kept.into_iter().take(MAX_BENCHMARKS).collect();
                Ok((PrefValue::Tickers(kept), note))
            }
        }
    }

    /// A value from a PATCH body: strict, because it comes from our own client
    /// and a value it should never send is a bug worth a `400`.
    fn check_json(self, value: &Json) -> Result<PrefValue, &'static str> {
        match self {
            Self::Flag(_) => value
                .as_bool()
                .map(PrefValue::Flag)
                .ok_or("must be true or false"),
            Self::Choice { allowed, .. } => value
                .as_str()
                .filter(|choice| allowed.contains(choice))
                .map(|choice| PrefValue::Text(choice.to_string()))
                .ok_or("is not one of the accepted values"),
            Self::Whole { min, max, .. } => value
                .as_i64()
                .filter(|number| (min..=max).contains(number))
                .map(PrefValue::Whole)
                .ok_or("must be a whole number within its range"),
            Self::FileId => match value {
                Json::Null => Ok(PrefValue::FileId(None)),
                Json::String(text) => check_file_id(text).map(PrefValue::FileId),
                _ => Err("must be a string or null"),
            },
            Self::Tickers => {
                let entries = value.as_array().ok_or("must be a list of ticker symbols")?;
                let symbols = entries
                    .iter()
                    .map(|entry| entry.as_str().filter(|symbol| is_ticker(symbol)))
                    .collect::<Option<Vec<&str>>>()
                    .ok_or("must be a list of ticker symbols")?;
                let kept = dedupe(symbols.into_iter());
                if kept.len() > MAX_BENCHMARKS {
                    return Err("has too many entries");
                }
                Ok(PrefValue::Tickers(kept))
            }
        }
    }
}

/// `value` rounded to a whole number, when it is a finite float in range.
fn round_finite(value: f64) -> Option<i64> {
    // `as` saturates, and the clamp that follows bounds it anyway; the
    // `is_finite` check is what keeps NaN from saturating to 0.
    #[allow(clippy::cast_possible_truncation)]
    value.is_finite().then(|| value.round() as i64)
}

/// A scenario file id as kept: `""` is none; anything long or carrying a
/// control character is refused.
fn check_file_id(text: &str) -> Result<Option<String>, &'static str> {
    if text.len() > MAX_FILE_ID_BYTES {
        return Err("is too long");
    }
    if text.chars().any(char::is_control) {
        return Err("contains control characters");
    }
    Ok((!text.is_empty()).then(|| text.to_string()))
}

/// A benchmark symbol as `settings.svelte.ts`'s `TICKER` accepts it:
/// `^[A-Z0-9.^=-]{1,12}$`.
fn is_ticker(symbol: &str) -> bool {
    (1..=12).contains(&symbol.len())
        && symbol
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || ".^=-".contains(c))
}

/// `symbols` once each, in first-seen order.
fn dedupe<'a>(symbols: impl Iterator<Item = &'a str>) -> Vec<String> {
    symbols.fold(Vec::new(), |mut kept, symbol| {
        if !kept.iter().any(|seen| seen == symbol) {
            kept.push(symbol.to_string());
        }
        kept
    })
}

impl PrefValue {
    fn to_json(&self) -> Json {
        match self {
            Self::Flag(flag) => Json::Bool(*flag),
            Self::Text(text) => Json::String(text.clone()),
            Self::Whole(number) => Json::from(*number),
            Self::FileId(id) => id.clone().map_or(Json::Null, Json::String),
            Self::Tickers(symbols) => Json::from(symbols.clone()),
        }
    }

    fn to_toml(&self) -> toml_edit::Value {
        match self {
            Self::Flag(flag) => toml_edit::Value::from(*flag),
            Self::Text(text) => toml_edit::Value::from(text.as_str()),
            Self::Whole(number) => toml_edit::Value::from(*number),
            // TOML has no null: none is the empty string, which reads back as
            // none ([`check_file_id`]) and still overrides a global value.
            Self::FileId(id) => toml_edit::Value::from(id.as_deref().unwrap_or_default()),
            Self::Tickers(symbols) => {
                toml_edit::Value::Array(symbols.iter().map(String::as_str).collect())
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Layers
// ---------------------------------------------------------------------------

/// Something in a settings file that was not used as written. Fixed phrases
/// only: no value from the file and no path is echoed back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct Issue {
    /// [`LEDGER_FILE`] or [`GLOBAL_FILE`].
    pub(crate) file: &'static str,
    /// The key as written in that file (`ui.matchWindow`), or `None` for the
    /// whole file.
    pub(crate) key: Option<String>,
    pub(crate) message: &'static str,
}

impl Issue {
    fn file(file: &'static str, message: &'static str) -> Self {
        Self {
            file,
            key: None,
            message,
        }
    }

    fn key(file: &'static str, key: String, message: &'static str) -> Self {
        Self {
            file,
            key: Some(key),
            message,
        }
    }
}

/// The usable values one file holds, and what it held that was not usable.
#[derive(Debug, Default)]
struct Layer {
    values: Values,
    issues: Vec<Issue>,
}

/// The item at a (possibly dotted) spec key in `table`.
fn lookup<'a>(table: &'a dyn TableLike, key: &str) -> Option<&'a Item> {
    match key.split_once('.') {
        Some((group, leaf)) => table.get(group)?.as_table_like()?.get(leaf),
        None => table.get(key),
    }
}

/// Every spec'd key in `table`, checked. `prefix` is how the file spells the
/// table's path (`"ui."` in [`LEDGER_FILE`]), for the issues.
fn read_layer(table: &dyn TableLike, file: &'static str, prefix: &str) -> Layer {
    let mut layer = Layer::default();
    // A group spelled as a scalar (`columns = true`) has no members to read;
    // say so once, rather than once per member.
    let mut groups: Vec<&str> = SPECS
        .iter()
        .filter_map(|spec| spec.key.split_once('.').map(|(group, _)| group))
        .collect();
    groups.dedup();
    for group in groups {
        if table
            .get(group)
            .is_some_and(|item| item.as_table_like().is_none())
        {
            layer.issues.push(Issue::key(
                file,
                format!("{prefix}{group}"),
                "must be a table",
            ));
        }
    }
    for spec in SPECS {
        let Some(item) = lookup(table, spec.key) else {
            continue;
        };
        match spec.kind.check_toml(item) {
            Ok((value, note)) => {
                layer.values.insert(spec.key, value);
                if let Some(note) = note {
                    layer
                        .issues
                        .push(Issue::key(file, format!("{prefix}{}", spec.key), note));
                }
            }
            Err(reason) => {
                layer
                    .issues
                    .push(Issue::key(file, format!("{prefix}{}", spec.key), reason))
            }
        }
    }
    layer
}

/// `ui-prefs.toml` from the config directory, or an empty layer.
///
/// Read with the ordinary, symlink-following open: it is the user's own
/// config, and dotfile managers (home-manager, stow) deliver it as a symlink.
fn global_layer(dirs: &AppDirs) -> Layer {
    let Some(path) = dirs.config_file(GLOBAL_FILE) else {
        return Layer::default();
    };
    match read_global(&path) {
        Ok(None) => Layer::default(),
        Ok(Some(text)) => match text.parse::<DocumentMut>() {
            Ok(document) => read_layer(document.as_table(), GLOBAL_FILE, ""),
            Err(_) => Layer {
                values: Values::new(),
                issues: vec![Issue::file(
                    GLOBAL_FILE,
                    "is not valid TOML and was ignored",
                )],
            },
        },
        Err(reason) => Layer {
            values: Values::new(),
            issues: vec![Issue::file(GLOBAL_FILE, reason)],
        },
    }
}

/// The text of the global file, `None` when there is none.
fn read_global(path: &Path) -> Result<Option<String>, &'static str> {
    let meta = match std::fs::metadata(path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("could not be read and was ignored"),
    };
    // Checked before opening: a FIFO would block the open.
    if !meta.is_file() {
        return Err("is not a regular file and was ignored");
    }
    let mut text = String::new();
    std::fs::File::open(path)
        .and_then(|file| file.take(MAX_FILE_BYTES + 1).read_to_string(&mut text))
        .map_err(|_| "could not be read as UTF-8 text and was ignored")?;
    if text.len() as u64 > MAX_FILE_BYTES {
        return Err("is larger than 64 KiB and was ignored");
    }
    Ok(Some(text))
}

/// The ledger file as read.
enum LedgerFile {
    Missing,
    Present {
        document: Box<DocumentMut>,
        revision: Revision,
    },
    /// Present but not something to read or overwrite; the phrase says why.
    Unusable(&'static str),
}

/// [`LEDGER_FILE`]'s place in the journal's vault.
fn ledger_path(location: &JournalLocation) -> Result<VaultPath, AppError> {
    location
        .root_dir()
        .child(LEDGER_FILE)
        .map_err(|_| AppError::Internal(format!("{LEDGER_FILE} has no place beside the journal")))
}

/// Read [`LEDGER_FILE`] the way an untrusted file is read: capped, and never
/// through a symlink.
fn read_ledger(location: &JournalLocation, path: &VaultPath) -> LedgerFile {
    match location.vault().read_pinned(path, Some(MAX_FILE_BYTES)) {
        Ok((bytes, revision)) => String::from_utf8(bytes)
            .ok()
            .and_then(|text| text.parse::<DocumentMut>().ok())
            .map_or(LedgerFile::Unusable("is not valid TOML"), |document| {
                LedgerFile::Present {
                    document: Box::new(document),
                    revision,
                }
            }),
        Err(error) if error.is_not_found() => LedgerFile::Missing,
        Err(error) if error.kind() == std::io::ErrorKind::FileTooLarge => {
            LedgerFile::Unusable("is larger than 64 KiB")
        }
        Err(error) if error.is_not_regular_file() => {
            LedgerFile::Unusable("is not a regular file (a symbolic link is never followed)")
        }
        Err(VaultError::NotDownloaded { .. }) => LedgerFile::Unusable("is not downloaded yet"),
        Err(_) => LedgerFile::Unusable("could not be read"),
    }
}

/// The `[ui]` values of a parsed ledger file.
fn ledger_layer(document: &DocumentMut) -> Layer {
    let mut layer = match document.get(UI_TABLE) {
        None => Layer::default(),
        Some(item) => match item.as_table_like() {
            Some(table) => read_layer(table, LEDGER_FILE, "ui."),
            None => Layer {
                values: Values::new(),
                issues: vec![Issue::key(
                    LEDGER_FILE,
                    UI_TABLE.to_string(),
                    "must be a table",
                )],
            },
        },
    };
    let ui = document.get(UI_TABLE).and_then(Item::as_table_like);
    for key in FORBIDDEN_LEDGER_KEYS {
        let (at_root, in_ui) = (
            document.contains_key(key),
            ui.is_some_and(|table| table.contains_key(key)),
        );
        let spellings = [(at_root, (*key).to_string()), (in_ui, format!("ui.{key}"))];
        for (present, spelled) in spellings {
            if present {
                layer.issues.push(Issue::key(
                    LEDGER_FILE,
                    spelled,
                    "is never read from ledgeline.toml, which travels with the ledger; it was ignored",
                ));
            }
        }
    }
    layer
}

/// Whether a ledger file's `[ui]` table sets `key` (a spec key, dotted for a
/// group member) to a value — the test that routes a save of `key` to that
/// file. A value counts even when it is unusable: the person put the key
/// there, so a save replaces it with a good one. A table where a value belongs
/// does not count, so the save goes to the global file and nothing a person
/// wrote is overwritten.
fn ledger_sets(document: &DocumentMut, key: &str) -> bool {
    document
        .get(UI_TABLE)
        .and_then(Item::as_table_like)
        .and_then(|ui| lookup(ui, key))
        .is_some_and(Item::is_value)
}

/// The spec keys a ledger file's `[ui]` table sets ([`ledger_sets`]), in spec
/// order.
fn local_keys(document: &DocumentMut) -> Vec<&'static str> {
    SPECS
        .iter()
        .map(|spec| spec.key)
        .filter(|key| ledger_sets(document, key))
        .collect()
}

/// Every key's value from the first layer that has one, else its default.
fn merge(layers: &[&Values]) -> Values {
    SPECS
        .iter()
        .map(|spec| {
            let value = layers
                .iter()
                .find_map(|layer| layer.get(spec.key))
                .cloned()
                .unwrap_or_else(|| spec.kind.default_value());
            (spec.key, value)
        })
        .collect()
}

/// `values` as the wire's `prefs` object, groups nested.
fn prefs_json(values: &Values) -> Json {
    let mut root = Map::new();
    for (key, value) in values {
        match key.split_once('.') {
            Some((group, leaf)) => {
                if let Json::Object(members) = root
                    .entry(group)
                    .or_insert_with(|| Json::Object(Map::new()))
                {
                    members.insert(leaf.to_string(), value.to_json());
                }
            }
            None => {
                root.insert((*key).to_string(), value.to_json());
            }
        }
    }
    Json::Object(root)
}

// ---------------------------------------------------------------------------
// Patches
// ---------------------------------------------------------------------------

/// A partial update: the keys a `PATCH /api/ui-prefs` body names, checked.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Patch {
    values: Values,
}

impl Patch {
    /// Check a `PATCH` body: an object of wire keys (`columns` an object of
    /// its members), every key known and every value valid.
    ///
    /// # Errors
    /// [`AppError::BadRequest`] naming the first unknown key or bad value.
    pub(crate) fn from_json(body: &Json) -> Result<Self, AppError> {
        let object = body.as_object().ok_or_else(|| {
            AppError::BadRequest("a UI preferences patch must be a JSON object".to_string())
        })?;
        let mut values = Values::new();
        for (key, value) in object {
            let is_group = SPECS.iter().any(|spec| {
                spec.key
                    .split_once('.')
                    .is_some_and(|(group, _)| group == key)
            });
            if is_group {
                let members = value.as_object().ok_or_else(|| {
                    AppError::BadRequest(format!("UI preference {key} must be an object"))
                })?;
                for (leaf, member) in members {
                    let spec = spec(&format!("{key}.{leaf}")).ok_or_else(|| unknown_key(leaf))?;
                    values.insert(spec.key, checked(spec, member)?);
                }
            } else {
                let spec = spec(key)
                    .filter(|spec| !spec.key.contains('.'))
                    .ok_or_else(|| unknown_key(key))?;
                values.insert(spec.key, checked(spec, value)?);
            }
        }
        Ok(Self { values })
    }
}

fn checked(spec: &Spec, value: &Json) -> Result<PrefValue, AppError> {
    spec.kind
        .check_json(value)
        .map_err(|reason| AppError::BadRequest(format!("UI preference {} {reason}", spec.key)))
}

/// The `400` for a key no spec names. The key is echoed only when it is short
/// and plainly a key, since it is client-supplied text.
fn unknown_key(key: &str) -> AppError {
    let plain = key.len() <= 64 && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    AppError::BadRequest(if plain {
        format!("unknown UI preference: {key}")
    } else {
        "unknown UI preference".to_string()
    })
}

impl Patch {
    fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// The keys `local` holds, and the rest.
    fn split(&self, local: impl Fn(&str) -> bool) -> (Self, Self) {
        let (mine, rest) = self
            .values
            .iter()
            .map(|(key, value)| (*key, value.clone()))
            .partition(|(key, _)| local(key));
        (Self { values: mine }, Self { values: rest })
    }
}

/// Apply `patch` to `table` — the `[ui]` table of [`LEDGER_FILE`], or the root
/// of [`GLOBAL_FILE`] — editing values in place so their comments and
/// positions survive. `file` and `prefix` name the table for the error.
///
/// # Errors
/// [`AppError::Conflict`] when a group (`columns`) exists but is not a table:
/// rewriting it would destroy something a person wrote.
fn patch_table(
    table: &mut dyn TableLike,
    inline: bool,
    file: &str,
    prefix: &str,
    patch: &Patch,
) -> Result<(), AppError> {
    for (key, value) in &patch.values {
        match key.split_once('.') {
            Some((group, leaf)) => {
                let item = table.entry(group).or_insert(if inline {
                    Item::Value(toml_edit::Value::InlineTable(InlineTable::new()))
                } else {
                    Item::Table(Table::new())
                });
                let members = item.as_table_like_mut().ok_or_else(|| {
                    AppError::Conflict(format!(
                        "{prefix}{group} in {file} is not a table, so Ledgeline will not overwrite it"
                    ))
                })?;
                set_value(members, leaf, value);
            }
            None => set_value(table, key, value),
        }
    }
    Ok(())
}

/// Set `key` to `value`, keeping an existing value's surrounding whitespace
/// and trailing comment.
fn set_value(table: &mut dyn TableLike, key: &str, value: &PrefValue) {
    let fresh = value.to_toml();
    match table.get_mut(key).and_then(Item::as_value_mut) {
        Some(existing) => {
            let decor = existing.decor().clone();
            *existing = fresh;
            *existing.decor_mut() = decor;
        }
        None => {
            table.insert(key, Item::Value(fresh));
        }
    }
}

/// One try at a write: done, or lost a race and worth another.
enum Attempt<T> {
    Done(T),
    Retry,
}

/// Run `attempt` until it is done, up to [`WRITE_ATTEMPTS`] times.
fn with_retries<T>(
    file: &str,
    mut attempt: impl FnMut() -> Result<Attempt<T>, AppError>,
) -> Result<T, AppError> {
    for _ in 0..WRITE_ATTEMPTS {
        if let Attempt::Done(done) = attempt()? {
            return Ok(done);
        }
    }
    Err(AppError::Conflict(format!(
        "{file} kept changing while Ledgeline saved to it; nothing was written"
    )))
}

/// A write of `file` that failed outright. Names the error kind, never a path.
fn write_failed(file: &str, kind: std::io::ErrorKind) -> AppError {
    AppError::Internal(format!(
        "could not save {file} ({kind}); nothing was written"
    ))
}

/// Write the keys of `patch` that [`LEDGER_FILE`] already sets into it, and
/// answer the rest, for [`GLOBAL_FILE`]. Never creates the file: a missing or
/// unusable one sets no key, so the whole patch is answered back.
///
/// The split is made on the bytes the write is pinned to, so a hand edit that
/// lands mid-save is re-read and re-split rather than overwritten.
fn write_ledger(location: &JournalLocation, patch: &Patch) -> Result<Patch, AppError> {
    let Ok(path) = ledger_path(location) else {
        return Ok(patch.clone());
    };
    let vault = location.vault();
    with_retries(LEDGER_FILE, || {
        let LedgerFile::Present {
            mut document,
            revision,
        } = read_ledger(location, &path)
        else {
            return Ok(Attempt::Done(patch.clone()));
        };
        let (local, rest) = patch.split(|key| ledger_sets(&document, key));
        if local.is_empty() {
            return Ok(Attempt::Done(rest));
        }
        let ui = document
            .get_mut(UI_TABLE)
            .and_then(Item::as_table_like_mut)
            .ok_or_else(|| AppError::Internal(format!("{LEDGER_FILE} lost its [ui] table")))?;
        // Every local key already exists, so no group is created here and
        // `inline` only matters for the type of a group never inserted.
        patch_table(ui, false, LEDGER_FILE, "ui.", &local)?;
        match vault.replace(&path, document.to_string().as_bytes(), &revision) {
            Ok(_) => Ok(Attempt::Done(rest)),
            // Lost a race: re-read. A symlink that appeared is caught by the
            // read as `Unusable`, and its keys go to the global file.
            Err(VaultError::Changed { .. }) => Ok(Attempt::Retry),
            Err(error) => Err(write_failed(LEDGER_FILE, error.kind())),
        }
    })
}

/// What [`write_global`] did with a patch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GlobalSave {
    Written,
    /// Not written; keep it in memory, for the reason given.
    Kept(&'static str),
}

fn is_symlink(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_symlink())
}

/// Write `patch` into [`GLOBAL_FILE`], creating it (and the config directory)
/// if need be. A symlinked file is left alone ([`GlobalSave::Kept`]); see the
/// module docs.
///
/// # Errors
/// [`AppError::Conflict`] when the file exists but cannot be safely rewritten
/// (too large, not TOML, not a regular file, a group that is not a table);
/// [`AppError::Internal`] when the write itself fails.
fn write_global(dirs: &AppDirs, patch: &Patch) -> Result<GlobalSave, AppError> {
    let Some(path) = dirs.config_file(GLOBAL_FILE) else {
        return Ok(GlobalSave::Kept(KEPT_NO_CONFIG_DIR));
    };
    let refuse = |reason: &str| {
        AppError::Conflict(format!(
            "{GLOBAL_FILE} {reason}, so Ledgeline will not overwrite it"
        ))
    };
    with_retries(GLOBAL_FILE, || {
        if is_symlink(&path) {
            return Ok(Attempt::Done(GlobalSave::Kept(KEPT_SYMLINK)));
        }
        // Without following a link, so the bytes edited are the bytes replaced.
        match ledgeline_core::pinned::read_pinned_capped(&path, MAX_FILE_BYTES) {
            Ok((bytes, pin)) => {
                let mut document = String::from_utf8(bytes)
                    .ok()
                    .and_then(|text| text.parse::<DocumentMut>().ok())
                    .ok_or_else(|| refuse("is not valid TOML"))?;
                patch_table(document.as_table_mut(), false, GLOBAL_FILE, "", patch)?;
                match ledgeline_core::pinned::replace(&path, document.to_string().as_bytes(), &pin)
                {
                    Ok(_) => Ok(Attempt::Done(GlobalSave::Written)),
                    Err(ledgeline_core::pinned::WriteError::Changed) => Ok(Attempt::Retry),
                    Err(ledgeline_core::pinned::WriteError::Io(error)) => {
                        Err(write_failed(GLOBAL_FILE, error.kind()))
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let mut document = DocumentMut::new();
                patch_table(document.as_table_mut(), false, GLOBAL_FILE, "", patch)?;
                if let Some(dir) = path.parent() {
                    std::fs::create_dir_all(dir)
                        .map_err(|error| write_failed(GLOBAL_FILE, error.kind()))?;
                }
                // The header is prepended as text: `toml_edit` keeps a
                // comment-only document's comment as trailing decor.
                let text = format!("{GLOBAL_HEADER}{document}");
                match ledgeline_core::pinned::create_new_pinned(&path, text.as_bytes()) {
                    Ok(_) => Ok(Attempt::Done(GlobalSave::Written)),
                    // Something appeared under the name: re-read it.
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                        Ok(Attempt::Retry)
                    }
                    Err(error) => Err(write_failed(GLOBAL_FILE, error.kind())),
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::FileTooLarge => {
                Err(refuse("is larger than 64 KiB"))
            }
            // A symlink swapped in since the check above lands here too.
            Err(error) if error.kind() == std::io::ErrorKind::InvalidInput => {
                if is_symlink(&path) {
                    Ok(Attempt::Done(GlobalSave::Kept(KEPT_SYMLINK)))
                } else {
                    Err(refuse("is not a regular file"))
                }
            }
            Err(error) => Err(write_failed(GLOBAL_FILE, error.kind())),
        }
    })
}

// ---------------------------------------------------------------------------
// Session state and the two operations
// ---------------------------------------------------------------------------

/// Where this session's UI preferences are kept. See the module docs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Storage {
    Files,
    Browser,
}

/// The `GET`/`PATCH /api/ui-prefs` body.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireUiPrefs {
    /// Every preference's effective value.
    prefs: Json,
    storage: Storage,
    /// The keys the open ledger's `ledgeline.toml` sets, so a change to them
    /// is saved there rather than to `ui-prefs.toml`; wire spelling, a group
    /// member dotted (`columns.status`), in spec order.
    local_keys: Vec<&'static str>,
    issues: Vec<Issue>,
}

/// Saves that could not go to [`GLOBAL_FILE`], and why.
#[derive(Debug, Default)]
struct Kept {
    values: Values,
    reason: Option<&'static str>,
}

/// Per-[`AppState`] UI-preferences state, shared by every clone.
#[derive(Debug, Default)]
pub(crate) struct UiPrefsSession {
    /// [`Storage::Browser`] was asked for.
    browser: bool,
    /// Held across every save, so two never interleave; holds the saves that
    /// could not be written to [`GLOBAL_FILE`] (see the module docs).
    kept: Mutex<Kept>,
}

impl UiPrefsSession {
    /// A session that leaves UI preferences to the browser.
    pub(crate) fn browser() -> Self {
        Self {
            browser: true,
            kept: Mutex::default(),
        }
    }

    fn kept(&self) -> std::sync::MutexGuard<'_, Kept> {
        self.kept.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The effective preferences for `state`'s current journal. Reads only:
/// nothing is created or written.
pub(crate) fn current(state: &AppState) -> WireUiPrefs {
    let session = state.ui_prefs();
    if session.browser {
        return browser_answer(&Values::new());
    }
    let global = global_layer(state.app_dirs());
    let (ledger, local_keys) = state
        .snapshot()
        .storage()
        .and_then(|location| ledger_path(&location).ok().map(|path| (location, path)))
        .map_or_else(
            || (Layer::default(), Vec::new()),
            |(location, path)| match read_ledger(&location, &path) {
                LedgerFile::Missing => (Layer::default(), Vec::new()),
                LedgerFile::Present { document, .. } => {
                    (ledger_layer(&document), local_keys(&document))
                }
                LedgerFile::Unusable(reason) => (
                    Layer {
                        values: Values::new(),
                        issues: vec![Issue::file(LEDGER_FILE, reason)],
                    },
                    Vec::new(),
                ),
            },
        );
    let kept = session.kept();
    let kept_issue = kept.reason.map(|reason| Issue::file(GLOBAL_FILE, reason));
    WireUiPrefs {
        prefs: prefs_json(&merge(&[&ledger.values, &kept.values, &global.values])),
        storage: Storage::Files,
        local_keys,
        issues: ledger
            .issues
            .into_iter()
            .chain(global.issues)
            .chain(kept_issue)
            .collect(),
    }
}

/// Save `patch` — each key to the ledger's `ledgeline.toml` when it already
/// sets that key, else to `ui-prefs.toml` — then answer with the effective
/// values.
///
/// # Errors
/// [`AppError::Conflict`] when a file that should take the save cannot be
/// safely rewritten; [`AppError::Internal`] when the write itself fails. The
/// ledger file is written first, so a failure at the global file can leave
/// the ledger's share saved.
pub(crate) fn apply(state: &AppState, patch: &Patch) -> Result<WireUiPrefs, AppError> {
    let session = state.ui_prefs();
    if session.browser {
        return Ok(browser_answer(&patch.values));
    }
    if !patch.is_empty() {
        // Released before `current`, which takes it again.
        let mut kept = session.kept();
        let global = match state.snapshot().storage() {
            Some(location) => write_ledger(&location, patch)?,
            None => patch.clone(),
        };
        if !global.is_empty() {
            match write_global(state.app_dirs(), &global)? {
                // A key that reached the file no longer needs its session copy.
                GlobalSave::Written => {
                    kept.values
                        .retain(|key, _| !global.values.contains_key(key));
                    if kept.values.is_empty() {
                        kept.reason = None;
                    }
                }
                GlobalSave::Kept(reason) => {
                    kept.values.extend(global.values);
                    kept.reason = Some(reason);
                }
            }
        }
    }
    Ok(current(state))
}

/// A `POST /api/ui-prefs/pin` body, checked: pin one [`PINNABLE`] key to the
/// open ledger, or unpin it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Pin {
    key: &'static str,
    pinned: bool,
}

impl Pin {
    /// Check a pin body: `{"key": "theme", "pinned": true}`, nothing else.
    ///
    /// # Errors
    /// [`AppError::BadRequest`] for any other shape, or a key that is not
    /// [`PINNABLE`].
    pub(crate) fn from_json(body: &Json) -> Result<Self, AppError> {
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Body {
            key: String,
            pinned: bool,
        }
        let body: Body = serde_json::from_value(body.clone()).map_err(|_| {
            AppError::BadRequest(
                "a pin must be {\"key\": <a UI preference>, \"pinned\": true or false}".to_string(),
            )
        })?;
        let key = PINNABLE
            .iter()
            .copied()
            .find(|key| *key == body.key)
            .ok_or_else(|| {
                AppError::BadRequest(format!(
                    "only {} can be kept for one ledger",
                    PINNABLE.join(" and ")
                ))
            })?;
        Ok(Self {
            key,
            pinned: body.pinned,
        })
    }
}

/// The top of a [`LEDGER_FILE`] that [`pin`] creates.
const LEDGER_HEADER: &str = concat!(
    "# Ledgeline view settings for this ledger only. Keys in [ui] override\n",
    "# ui-prefs.toml while this ledger is open. Docs: \"Settings files\".\n",
    "\n",
);

/// Pin `pin.key` to the open ledger — write its current effective value into
/// [`LEDGER_FILE`]'s `[ui]`, CREATING the file if there is none — or unpin it
/// by removing the key from `[ui]` (the file is kept, even empty). Answers
/// with the effective values after, like [`apply`].
///
/// This is the one place the app creates [`LEDGER_FILE`], and only because
/// the user asked to keep a setting for this ledger. Once a key is pinned the
/// ordinary save routing sends its later changes there.
///
/// Pinning a key the file already sets to a usable value, or unpinning one it
/// does not set, writes nothing. An existing file that is symlinked, too large
/// or not TOML is never written; unpinning from one is a no-op, since it sets
/// no key.
///
/// # Errors
/// [`AppError::Conflict`] in browser mode, with no ledger open, or when the
/// ledger file cannot be safely written (see above, or a `[ui]`/key that is
/// not what a pin expects); [`AppError::Internal`] when the write fails.
pub(crate) fn pin(state: &AppState, pin: Pin) -> Result<WireUiPrefs, AppError> {
    let session = state.ui_prefs();
    if session.browser {
        return Err(AppError::Conflict(
            "view settings are kept in this browser, so none can be kept for one ledger"
                .to_string(),
        ));
    }
    let location = state.snapshot().storage().ok_or_else(|| {
        AppError::Conflict(
            "no ledger file is open, so there is no ledgeline.toml to keep a setting in"
                .to_string(),
        )
    })?;
    let path = ledger_path(&location)?;
    {
        // Held across the write, so a save never interleaves with it.
        let kept = session.kept();
        if pin.pinned {
            // What the key is now without the ledger's file. When the file
            // has a usable value, that is already the effective one.
            let global = global_layer(state.app_dirs());
            let value = merge(&[&kept.values, &global.values])
                .remove(pin.key)
                .ok_or_else(|| AppError::Internal(format!("{} has no spec", pin.key)))?;
            pin_ledger(&location, &path, pin.key, &value)?;
        } else {
            unpin_ledger(&location, &path, pin.key)?;
        }
    }
    Ok(current(state))
}

/// The `409` for a [`LEDGER_FILE`] a pin will not write.
fn refuse_ledger(reason: &str) -> AppError {
    AppError::Conflict(format!(
        "{LEDGER_FILE} {reason}, so Ledgeline will not write to it"
    ))
}

/// Set `key` to `value` in [`LEDGER_FILE`]'s `[ui]`, creating the file or the
/// table as needed. See [`pin`].
fn pin_ledger(
    location: &JournalLocation,
    path: &VaultPath,
    key: &'static str,
    value: &PrefValue,
) -> Result<(), AppError> {
    let vault = location.vault();
    let patch = Patch {
        values: Values::from([(key, value.clone())]),
    };
    with_retries(LEDGER_FILE, || match read_ledger(location, path) {
        LedgerFile::Missing => {
            let mut document = DocumentMut::new();
            let mut ui = Table::new();
            patch_table(&mut ui, false, LEDGER_FILE, "ui.", &patch)?;
            document.insert(UI_TABLE, Item::Table(ui));
            let text = format!("{LEDGER_HEADER}{document}");
            // A file the user keeps (and may commit) with their books: the
            // ordinary new-file mode, as for a rules file or a projection.
            match vault.create_new_shared(path, text.as_bytes()) {
                Ok(_) => Ok(Attempt::Done(())),
                // Something appeared under the name: re-read it.
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    Ok(Attempt::Retry)
                }
                Err(error) => Err(write_failed(LEDGER_FILE, error.kind())),
            }
        }
        LedgerFile::Unusable(reason) => Err(refuse_ledger(reason)),
        LedgerFile::Present {
            mut document,
            revision,
        } => {
            if ledger_layer(&document).values.contains_key(key) {
                return Ok(Attempt::Done(()));
            }
            if !document.contains_key(UI_TABLE) {
                // A new table goes last, after what the document holds as
                // trailing text (a comment-only file is all trailing text).
                let trailing = document.trailing().as_str().unwrap_or_default().to_string();
                document.set_trailing("");
                let mut ui = Table::new();
                ui.decor_mut().set_prefix(trailing);
                document.insert(UI_TABLE, Item::Table(ui));
            }
            let ui = document
                .entry(UI_TABLE)
                .or_insert_with(|| Item::Table(Table::new()));
            // A `[ui.columns]` alone makes `[ui]` implicit; it now holds a key.
            if let Some(table) = ui.as_table_mut() {
                table.set_implicit(false);
            }
            let ui = ui
                .as_table_like_mut()
                .ok_or_else(|| refuse_ledger("has a ui that is not a table"))?;
            if ui.get(key).is_some_and(|item| !item.is_value()) {
                return Err(refuse_ledger(&format!(
                    "has a ui.{key} that is not a value"
                )));
            }
            patch_table(ui, false, LEDGER_FILE, "ui.", &patch)?;
            match location
                .vault()
                .replace(path, document.to_string().as_bytes(), &revision)
            {
                Ok(_) => Ok(Attempt::Done(())),
                Err(VaultError::Changed { .. }) => Ok(Attempt::Retry),
                Err(error) => Err(write_failed(LEDGER_FILE, error.kind())),
            }
        }
    })
}

/// Remove `key` from [`LEDGER_FILE`]'s `[ui]`, keeping everything else and the
/// file itself. A key the file does not set, or a file that is missing or
/// unusable, is left alone. The key's own line goes, with any comment lines
/// directly above it.
fn unpin_ledger(
    location: &JournalLocation,
    path: &VaultPath,
    key: &'static str,
) -> Result<(), AppError> {
    with_retries(LEDGER_FILE, || {
        let LedgerFile::Present {
            mut document,
            revision,
        } = read_ledger(location, path)
        else {
            return Ok(Attempt::Done(()));
        };
        if !ledger_sets(&document, key) {
            return Ok(Attempt::Done(()));
        }
        if let Some(ui) = document.get_mut(UI_TABLE).and_then(Item::as_table_like_mut) {
            ui.remove(key);
        }
        match location
            .vault()
            .replace(path, document.to_string().as_bytes(), &revision)
        {
            Ok(_) => Ok(Attempt::Done(())),
            Err(VaultError::Changed { .. }) => Ok(Attempt::Retry),
            Err(error) => Err(write_failed(LEDGER_FILE, error.kind())),
        }
    })
}

/// [`Storage::Browser`]'s answer: the defaults under `overrides`, from no file.
fn browser_answer(overrides: &Values) -> WireUiPrefs {
    WireUiPrefs {
        prefs: prefs_json(&merge(&[overrides])),
        storage: Storage::Browser,
        local_keys: Vec::new(),
        issues: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse(text: &str) -> DocumentMut {
        text.parse().expect("test TOML parses")
    }

    fn effective(ledger: &str, global: &str) -> (Json, Vec<Issue>) {
        let ledger = ledger_layer(&parse(ledger));
        let global = read_layer(parse(global).as_table(), GLOBAL_FILE, "");
        let issues = ledger.issues.into_iter().chain(global.issues).collect();
        (
            prefs_json(&merge(&[&ledger.values, &global.values])),
            issues,
        )
    }

    #[test]
    fn the_defaults_are_the_spas() {
        let (prefs, issues) = effective("", "");
        assert!(issues.is_empty());
        assert_eq!(
            prefs,
            json!({
                "columns": {"date": true, "status": true, "description": true, "accounts": true, "amount": true},
                "insightsOpen": true,
                "insightsTab": "activity",
                "hideZeroBalances": true,
                "flowsInOpen": true,
                "flowsOutOpen": true,
                "netWorthChartOpen": true,
                "cashFlowChartOpen": true,
                "budgetGapsOpen": false,
                "startupMoreOpen": false,
                "holdingsPieDimension": "holding",
                "lastProjectionId": null,
                "benchmarks": [],
                "reviewRecentLimit": 50,
                "reviewMaxDays": 365,
                "reviewMarkers": true,
                "matchWindow": 10,
                "matchLookback": 365,
                "matchTransitDays": 10,
                "theme": "default",
                "uiScale": 100,
            })
        );
    }

    #[test]
    fn every_theme_id_is_accepted_from_either_file_and_a_patch() {
        assert_eq!(THEMES.len(), 22);
        for id in THEMES {
            let (prefs, issues) = effective(&format!("[ui]\ntheme = \"{id}\"\n"), "");
            assert!(issues.is_empty(), "{id}: {issues:?}");
            assert_eq!(prefs["theme"], *id);
            let (prefs, _) = effective("", &format!("theme = \"{id}\"\n"));
            assert_eq!(prefs["theme"], *id);
            assert!(Patch::from_json(&json!({"theme": id})).is_ok(), "{id}");
        }
    }

    #[test]
    fn a_hostile_theme_or_scale_falls_through_with_an_issue() {
        let (prefs, issues) = effective(
            "[ui]\ntheme = \"../../etc/passwd\"\nuiScale = \"huge\"\n",
            "theme = \"nord\"\nuiScale = 125\n",
        );
        assert_eq!(
            prefs["theme"], "nord",
            "a bad theme falls through to global"
        );
        assert_eq!(prefs["uiScale"], 125, "a bad type falls through");
        let keys: Vec<&str> = issues.iter().filter_map(|i| i.key.as_deref()).collect();
        assert_eq!(keys, ["ui.theme", "ui.uiScale"]);
        assert!(issues.iter().all(|issue| !issue.message.contains("passwd")));

        // Wrong case, a table, a number: no layer usable, so the default.
        for bad in ["theme = \"Nord\"", "theme = 3", "[ui.theme]\nx = 1"] {
            let (prefs, issues) = effective(&format!("[ui]\n{bad}\n"), "");
            assert_eq!(prefs["theme"], "default", "{bad}");
            assert_eq!(issues.len(), 1, "{bad}: {issues:?}");
        }
    }

    #[test]
    fn a_hand_edited_scale_is_clamped_and_a_patch_is_range_checked() {
        for (written, read) in [("10", 50), ("9999", 300), ("-1", 50), ("112.6", 113)] {
            let (prefs, issues) = effective(&format!("[ui]\nuiScale = {written}\n"), "");
            assert_eq!(prefs["uiScale"], read, "{written}");
            assert_eq!(
                issues.len(),
                usize::from(written != "112.6"),
                "{written}: {issues:?}"
            );
        }
        for ok in [50, 70, 100, 300] {
            assert!(Patch::from_json(&json!({"uiScale": ok})).is_ok(), "{ok}");
        }
        for bad in [json!(49), json!(301), json!(1.5), json!("100"), json!(null)] {
            assert!(
                matches!(
                    Patch::from_json(&json!({"uiScale": bad})),
                    Err(AppError::BadRequest(_))
                ),
                "{bad}"
            );
        }
        for bad in [json!("Default"), json!("light"), json!(""), json!(null)] {
            assert!(
                matches!(
                    Patch::from_json(&json!({"theme": bad})),
                    Err(AppError::BadRequest(_))
                ),
                "{bad}"
            );
        }
    }

    #[test]
    fn only_theme_and_scale_can_be_pinned() {
        assert_eq!(
            Pin::from_json(&json!({"key": "theme", "pinned": true})).expect("pin"),
            Pin {
                key: "theme",
                pinned: true
            }
        );
        assert_eq!(
            Pin::from_json(&json!({"key": "uiScale", "pinned": false})).expect("unpin"),
            Pin {
                key: "uiScale",
                pinned: false
            }
        );
        for bad in [
            json!({"key": "matchWindow", "pinned": true}),
            json!({"key": "hledgerPath", "pinned": true}),
            json!({"key": "columns.date", "pinned": true}),
            json!({"key": "theme"}),
            json!({"key": "theme", "pinned": "yes"}),
            json!({"key": "theme", "pinned": true, "value": "nord"}),
            json!(["theme"]),
        ] {
            assert!(
                matches!(Pin::from_json(&bad), Err(AppError::BadRequest(_))),
                "{bad} must be refused"
            );
        }
        for key in PINNABLE {
            assert!(
                spec(key).is_some_and(|spec| !spec.key.contains('.')),
                "{key}"
            );
        }
    }

    #[test]
    fn the_ledger_beats_the_global_file_which_beats_the_default_per_key() {
        let (prefs, issues) = effective(
            "[ui]\nmatchWindow = 20\n[ui.columns]\nstatus = false\n",
            "matchWindow = 30\nmatchLookback = 90\n[columns]\nstatus = true\namount = false\n",
        );
        assert!(issues.is_empty(), "{issues:?}");
        assert_eq!(prefs["matchWindow"], 20, "ledger wins");
        assert_eq!(
            prefs["matchLookback"], 90,
            "global fills what the ledger lacks"
        );
        assert_eq!(prefs["matchTransitDays"], 10, "default fills the rest");
        assert_eq!(
            prefs["columns"]["status"], false,
            "per member, not per group"
        );
        assert_eq!(prefs["columns"]["amount"], false);
        assert_eq!(prefs["columns"]["date"], true);
    }

    #[test]
    fn a_hostile_ledger_file_is_clamped_checked_and_falls_through() {
        let (prefs, issues) = effective(
            r#"
hledgerPath = "/tmp/evil"
serverToken = "x"
[ui]
hledgerPath = "/tmp/evil"
serverUrl = "http://evil.example"
reviewRecentLimit = 999999999
reviewMaxDays = -5
matchWindow = "ten"
insightsTab = "../../etc"
holdingsPieDimension = 3
insightsOpen = "yes"
lastProjectionId = "a\u0007b"
benchmarks = ["SPY", "rm -rf", 7, "SPY", "QQQ"]
columns = true
unknownKey = { deep = [1, 2] }
"#,
            "matchWindow = 25\ninsightsTab = \"balances\"\n",
        );
        assert_eq!(prefs["reviewRecentLimit"], 1000, "clamped high");
        assert_eq!(prefs["reviewMaxDays"], 0, "clamped low");
        assert_eq!(
            prefs["matchWindow"], 25,
            "a bad type falls through to global"
        );
        assert_eq!(
            prefs["insightsTab"], "balances",
            "a bad choice falls through"
        );
        assert_eq!(prefs["holdingsPieDimension"], "holding");
        assert_eq!(prefs["insightsOpen"], true);
        assert_eq!(prefs["lastProjectionId"], Json::Null);
        assert_eq!(prefs["benchmarks"], json!(["SPY", "QQQ"]));
        assert_eq!(prefs["columns"]["date"], true);
        let keys: Vec<&str> = issues.iter().filter_map(|i| i.key.as_deref()).collect();
        for expected in [
            "hledgerPath",
            "serverToken",
            "ui.hledgerPath",
            "ui.serverUrl",
            "ui.reviewRecentLimit",
            "ui.reviewMaxDays",
            "ui.matchWindow",
            "ui.insightsTab",
            "ui.holdingsPieDimension",
            "ui.insightsOpen",
            "ui.lastProjectionId",
            "ui.benchmarks",
            "ui.columns",
        ] {
            assert!(
                keys.contains(&expected),
                "no issue for {expected}: {keys:?}"
            );
        }
        assert!(
            !keys.contains(&"ui.unknownKey"),
            "unknown keys are ignored quietly"
        );
        assert!(
            issues.iter().all(|issue| !issue.message.contains("evil")),
            "no value is echoed"
        );
    }

    #[test]
    fn a_float_rounds_and_too_many_benchmarks_are_capped() {
        let many: Vec<String> = (0..40).map(|n| format!("\"S{n}\"")).collect();
        let (prefs, issues) = effective(
            &format!(
                "[ui]\nmatchWindow = 12.6\nbenchmarks = [{}]\n",
                many.join(",")
            ),
            "",
        );
        assert_eq!(prefs["matchWindow"], 13);
        assert_eq!(prefs["benchmarks"].as_array().map(Vec::len), Some(32));
        assert_eq!(issues.len(), 1, "{issues:?}");
    }

    #[test]
    fn an_empty_projection_id_overrides_the_global_one_as_none() {
        let (prefs, _) = effective(
            "[ui]\nlastProjectionId = \"\"\n",
            "lastProjectionId = \"a.journal\"\n",
        );
        assert_eq!(prefs["lastProjectionId"], Json::Null);
    }

    #[test]
    fn a_non_table_ui_is_reported_and_sets_no_key() {
        let document = parse("ui = 5\n");
        assert_eq!(ledger_layer(&document).issues.len(), 1);
        assert!(local_keys(&document).is_empty());
        assert!(local_keys(&parse("[ui]\n")).is_empty());
    }

    #[test]
    fn a_ledger_file_sets_the_value_keys_in_its_ui_table_per_leaf() {
        let document = parse(
            "matchLookback = 5\n[ui]\nmatchWindow = \"bad\"\ninsightsTab = \"balances\"\n\
             [ui.reviewMaxDays]\nx = 1\n[ui.columns]\nstatus = false\n",
        );
        assert_eq!(
            local_keys(&document),
            ["columns.status", "insightsTab", "matchWindow"],
            "a bad value still counts; a table, a root key and an absent leaf do not"
        );
        let inline = parse("ui = { matchWindow = 5, columns = { date = true } }\n");
        assert_eq!(local_keys(&inline), ["columns.date", "matchWindow"]);
    }

    #[test]
    fn a_patch_splits_by_key_and_by_leaf() {
        let patch = Patch::from_json(&json!({
            "matchWindow": 7,
            "matchLookback": 9,
            "columns": {"status": false, "date": false},
        }))
        .expect("patch");
        let (local, rest) = patch.split(|key| ["matchWindow", "columns.status"].contains(&key));
        assert_eq!(
            local.values.keys().copied().collect::<Vec<_>>(),
            ["columns.status", "matchWindow"]
        );
        assert_eq!(
            rest.values.keys().copied().collect::<Vec<_>>(),
            ["columns.date", "matchLookback"]
        );
    }

    #[test]
    fn a_patch_is_strict() {
        let ok = Patch::from_json(&json!({
            "columns": {"date": false},
            "lastProjectionId": null,
            "benchmarks": ["SPY", "SPY"],
            "matchWindow": 60,
        }))
        .expect("valid patch");
        assert_eq!(ok.values.len(), 4);
        assert_eq!(
            ok.values.get("benchmarks"),
            Some(&PrefValue::Tickers(vec!["SPY".to_string()]))
        );
        for bad in [
            json!([]),
            json!({"nope": true}),
            json!({"columns": true}),
            json!({"columns": {"nope": true}}),
            json!({"matchWindow": 61}),
            json!({"matchWindow": 1.5}),
            json!({"insightsTab": "nope"}),
            json!({"benchmarks": ["lower"]}),
            json!({"lastProjectionId": 3}),
            json!({"hledgerPath": "/bin/sh"}),
            json!({"columns.date": false}),
        ] {
            assert!(
                matches!(Patch::from_json(&bad), Err(AppError::BadRequest(_))),
                "{bad} must be refused"
            );
        }
    }

    #[test]
    fn patching_keeps_comments_order_and_unknown_keys() {
        let original = "\
# my notes about this ledger
title = \"kept\"   # unknown root key

[ui]
# why I like it narrow
matchWindow = 5   # trailing comment
someFutureKey = \"kept too\"

[ui.columns]
date = true # keep me

[other]
x = 1
";
        let mut document = parse(original);
        let patch = Patch::from_json(&json!({
            "matchWindow": 7,
            "columns": {"date": false, "amount": false},
            "insightsOpen": false,
        }))
        .expect("patch");
        let ui = document["ui"].as_table_like_mut().expect("[ui]");
        patch_table(ui, false, LEDGER_FILE, "ui.", &patch).expect("patched");
        let written = document.to_string();
        let expected = "\
# my notes about this ledger
title = \"kept\"   # unknown root key

[ui]
# why I like it narrow
matchWindow = 7   # trailing comment
someFutureKey = \"kept too\"
insightsOpen = false

[ui.columns]
date = false # keep me
amount = false

[other]
x = 1
";
        assert_eq!(written, expected);
    }

    #[test]
    fn a_global_document_takes_keys_at_its_root_and_groups_as_tables() {
        let mut document = parse("# mine\nmatchWindow = 3 # narrow\n");
        let patch =
            Patch::from_json(&json!({"reviewMarkers": false, "columns": {"status": false}}))
                .expect("patch");
        patch_table(document.as_table_mut(), false, GLOBAL_FILE, "", &patch).expect("patched");
        assert_eq!(
            document.to_string(),
            "# mine\nmatchWindow = 3 # narrow\nreviewMarkers = false\n\n[columns]\nstatus = false\n"
        );
        let layer = read_layer(document.as_table(), GLOBAL_FILE, "");
        assert!(layer.issues.is_empty());
        assert_eq!(layer.values.len(), 3);
    }

    #[test]
    fn an_inline_table_is_patched_inline() {
        let mut document = parse("ui = { matchWindow = 5, columns = { date = true } }\n");
        let patch = Patch::from_json(&json!({"columns": {"date": false}, "matchWindow": 6}))
            .expect("patch");
        let ui = document["ui"].as_table_like_mut().expect("ui");
        patch_table(ui, true, LEDGER_FILE, "ui.", &patch).expect("patched");
        let layer = ledger_layer(&parse(&document.to_string()));
        assert_eq!(
            layer.values.get("columns.date"),
            Some(&PrefValue::Flag(false))
        );
        assert_eq!(layer.values.get("matchWindow"), Some(&PrefValue::Whole(6)));
    }

    #[test]
    fn a_non_table_group_is_never_overwritten() {
        let patch = Patch::from_json(&json!({"columns": {"date": false}})).expect("patch");
        let mut document = parse("columns = 3\n");
        assert!(matches!(
            patch_table(document.as_table_mut(), false, GLOBAL_FILE, "", &patch),
            Err(AppError::Conflict(_))
        ));
    }

    #[test]
    fn every_spec_default_survives_a_toml_round_trip() {
        for spec in SPECS {
            let value = spec.kind.default_value();
            let item = Item::Value(value.to_toml());
            let (back, note) = spec.kind.check_toml(&item).expect("default reads back");
            assert_eq!(back, value, "{}", spec.key);
            assert_eq!(note, None);
            assert_eq!(
                spec.kind.check_json(&value.to_json()),
                Ok(value),
                "{}",
                spec.key
            );
        }
    }
}
