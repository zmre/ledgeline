//! The time book's files: the `[time]` settings, where the roll-up, accounts
//! file and tracker files are, reading the book, and setting it up (plan 46
//! T2, T3, T15, F-files, T-config).
//!
//! The pattern is plan 45's recognition file ([`crate::accrual::book`]):
//!
//! - **Discovery.** [`TimeConfig`] reads `[time]` from `ledgeline.toml`
//!   (only when someone wrote it; this module never creates that file), and
//!   [`paths`] places every file inside the journal's own folder.
//! - **Reading never assumes our layout.** [`load`] reads whatever the
//!   roll-up includes; its `account` declarations and its time files are the
//!   book ([`TimeBook::build`]). With no roll-up, [`find_time_files`] lists
//!   the `.timedot`/`.timeclock` files under the books' folder for setup to
//!   offer.
//! - **Explicit creation.** [`setup_plan`] + [`create`] write our default
//!   roll-up, accounts file and the first tracker file, and nothing else;
//!   [`adopt_plan`] + [`set_time_file`] point `[time] file` at an existing
//!   journal or a single time file instead.
//! - **Coverage.** [`coverage`] says whether the roll-up's `include`s reach a
//!   tracker file, and which `include` line would.
//!
//! One deviation from F-files, found while verifying it against hledger 1.52:
//! **an `include` glob that matches nothing is an error** ("No files were
//! matched by: …"), in hledger and in Ledgeline. So setup writes the tracker
//! format's glob only, and creates the first tracker file (header only) so the
//! glob matches from the start; a `*.timeclock` glob is added only once a
//! timeclock file exists ([`coverage`] supplies the line).

use super::model::{TimeBook, TimeSource};
use super::timeclock::Moment;
use super::{TagNames, TimeError, TimeFormat};
use crate::ledger_file::{self, LedgerFile};
use crate::parse::{glob_path_match, parse_time_rollup};
use crate::vault::{Vault, VaultError, VaultPath};
use std::collections::BTreeSet;
use toml_edit::{DocumentMut, Item, Table, value};

/// The `[time]` table of `ledgeline.toml`.
pub const TIME_TABLE: &str = "time";
/// `file`: the roll-up.
pub const FILE_KEY: &str = "file";
/// `accounts-file`.
pub const ACCOUNTS_FILE_KEY: &str = "accounts-file";
/// `track-file`: the tracker's file pattern.
pub const TRACK_FILE_KEY: &str = "track-file";
/// `record = "timedot" | "timeclock"`.
pub const RECORD_KEY: &str = "record";
/// `weekday = "short" | "none"`.
pub const WEEKDAY_KEY: &str = "weekday";
/// `[time.tags]`.
pub const TAGS_TABLE: &str = "tags";
/// `[time.words]`.
pub const WORDS_TABLE: &str = "words";

/// The roll-up's default name, beside the main journal.
pub const DEFAULT_FILE: &str = "time.journal";
/// The accounts file's default name.
pub const DEFAULT_ACCOUNTS_FILE: &str = "time-accounts.journal";
/// The tracker's default file pattern in timedot mode (T3).
pub const DEFAULT_TRACK_FILE: &str = "time/{person}/{yyyy}-{mm}.timedot";
/// The tracker's default file pattern in timeclock mode.
pub const DEFAULT_TRACK_FILE_TIMECLOCK: &str = "time/{person}/{yyyy}-{mm}.timeclock";

/// What the tracker writes (T13).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RecordMode {
    /// Rounded minutes per day, with a running marker.
    #[default]
    Timedot,
    /// `i`/`o` pairs with seconds.
    Timeclock,
}

/// The weekday text written on a date line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WeekdayStyle {
    /// `Thu` (from [`Words::weekdays`]).
    #[default]
    Short,
    /// Nothing: a bare date.
    None,
}

/// The words Ledgeline writes into files (`[time.words]`, T15).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Words {
    /// Short weekday names, Monday first.
    pub weekdays: [String; 7],
    /// Month names, January first.
    pub months: [String; 12],
    /// The seeded billable branch.
    pub clients: String,
    /// The seeded non-billable branch.
    pub internal: String,
    /// The seeded admin account under [`Self::internal`].
    pub admin: String,
    /// The tracker file header's first word.
    pub title: String,
}

impl Default for Words {
    fn default() -> Self {
        let strings = |names: &[&str]| names.iter().map(|n| (*n).to_string()).collect::<Vec<_>>();
        Self {
            weekdays: strings(&["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"])
                .try_into()
                .unwrap_or_default(),
            months: strings(&[
                "January",
                "February",
                "March",
                "April",
                "May",
                "June",
                "July",
                "August",
                "September",
                "October",
                "November",
                "December",
            ])
            .try_into()
            .unwrap_or_default(),
            clients: "clients".to_string(),
            internal: "internal".to_string(),
            admin: "admin".to_string(),
            title: "Time".to_string(),
        }
    }
}

impl Words {
    /// The weekday name of ISO `date`.
    #[must_use]
    pub fn weekday(&self, date: &str) -> &str {
        let index = usize::try_from(crate::reports::periods::weekday_of(date) - 1).unwrap_or(0);
        self.weekdays.get(index).map_or("", String::as_str)
    }

    /// The month name of ISO `date`.
    #[must_use]
    pub fn month(&self, date: &str) -> &str {
        let (_, month, _) = crate::reports::periods::parts(date);
        usize::try_from(month - 1)
            .ok()
            .and_then(|at| self.months.get(at))
            .map_or("", String::as_str)
    }
}

/// What `[time]` says, with defaults for what it doesn't.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TimeConfig {
    /// The roll-up, relative to the journal's folder.
    pub file: Option<String>,
    /// The accounts file.
    pub accounts_file: Option<String>,
    /// The tracker's file pattern.
    pub track_file: Option<String>,
    /// What the tracker writes.
    pub record: RecordMode,
    /// The weekday on date lines.
    pub weekday: WeekdayStyle,
    /// Tag names.
    pub tags: TagNames,
    /// Words written.
    pub words: Words,
    /// What was ignored and why.
    pub warnings: Vec<String>,
}

impl TimeConfig {
    /// The roll-up's configured (or default) name.
    #[must_use]
    pub fn file(&self) -> &str {
        self.file.as_deref().unwrap_or(DEFAULT_FILE)
    }

    /// The accounts file's configured (or default) name.
    #[must_use]
    pub fn accounts_file(&self) -> &str {
        self.accounts_file
            .as_deref()
            .unwrap_or(DEFAULT_ACCOUNTS_FILE)
    }

    /// The tracker's configured (or default, by [`Self::record`]) pattern.
    #[must_use]
    pub fn track_file(&self) -> &str {
        self.track_file.as_deref().unwrap_or(match self.record {
            RecordMode::Timedot => DEFAULT_TRACK_FILE,
            RecordMode::Timeclock => DEFAULT_TRACK_FILE_TIMECLOCK,
        })
    }

    /// The tracker's pattern, when it is usable.
    #[must_use]
    pub fn track_pattern(&self) -> Option<TrackPattern> {
        TrackPattern::new(self.track_file()).ok()
    }
}

/// Read `[time]` out of `ledgeline.toml`'s text. Never fails as a whole: an
/// unusable value is dropped with a warning.
#[must_use]
pub fn parse_config(text: &str) -> TimeConfig {
    match text.parse::<DocumentMut>() {
        Ok(document) => config_of(&document),
        Err(_) => TimeConfig {
            warnings: vec![format!("{} is not valid TOML", ledger_file::LEDGER_FILE)],
            ..TimeConfig::default()
        },
    }
}

fn config_of(document: &DocumentMut) -> TimeConfig {
    let mut config = TimeConfig::default();
    let Some(item) = document.get(TIME_TABLE) else {
        return config;
    };
    let Some(table) = item.as_table_like() else {
        config
            .warnings
            .push(format!("[{TIME_TABLE}] must be a table"));
        return config;
    };
    let mut warn = |text: String| config.warnings.push(text);
    let text = |key: &str| table.get(key).and_then(Item::as_str).map(str::trim);
    let mut path = |key: &str, ok: fn(&str) -> bool| -> Option<String> {
        table.get(key)?;
        match text(key).filter(|value| !value.is_empty() && ok(value)) {
            Some(value) => Some(value.to_string()),
            None => {
                warn(format!("time.{key} is not a usable relative path; ignored"));
                None
            }
        }
    };
    let file = path(FILE_KEY, |v| {
        v.ends_with(".journal") || TimeFormat::of_path(v).is_some()
    });
    let accounts_file = path(ACCOUNTS_FILE_KEY, |v| v.ends_with(".journal"));
    let track_file = path(TRACK_FILE_KEY, |v| TrackPattern::new(v).is_ok());
    let mut warnings = Vec::new();
    let record = match text(RECORD_KEY) {
        None if table.get(RECORD_KEY).is_none() => RecordMode::default(),
        Some("timedot") => RecordMode::Timedot,
        Some("timeclock") => RecordMode::Timeclock,
        _ => {
            warnings.push(format!(
                "time.{RECORD_KEY} must be \"timedot\" or \"timeclock\"; ignored"
            ));
            RecordMode::default()
        }
    };
    let weekday = match text(WEEKDAY_KEY) {
        None if table.get(WEEKDAY_KEY).is_none() => WeekdayStyle::default(),
        Some("short") => WeekdayStyle::Short,
        Some("none") => WeekdayStyle::None,
        _ => {
            warnings.push(format!(
                "time.{WEEKDAY_KEY} must be \"short\" or \"none\"; ignored"
            ));
            WeekdayStyle::default()
        }
    };
    let tag_overrides: Vec<(String, String)> = table
        .get(TAGS_TABLE)
        .and_then(Item::as_table_like)
        .map(|tags| {
            tags.iter()
                .filter_map(|(key, item)| Some((key.to_string(), item.as_str()?.to_string())))
                .collect()
        })
        .unwrap_or_default();
    let (tags, tag_warnings) = TagNames::with_overrides(
        tag_overrides
            .iter()
            .map(|(key, name)| (key.as_str(), name.as_str())),
    );
    warnings.extend(tag_warnings);
    let words = table
        .get(WORDS_TABLE)
        .and_then(Item::as_table_like)
        .map(|table| words_of(table, &mut warnings))
        .unwrap_or_default();
    config.file = file;
    config.accounts_file = accounts_file;
    config.track_file = track_file;
    config.record = record;
    config.weekday = weekday;
    config.tags = tags;
    config.words = words;
    config.warnings.extend(warnings);
    config
}

fn words_of(table: &dyn toml_edit::TableLike, warnings: &mut Vec<String>) -> Words {
    let mut words = Words::default();
    let list = |key: &str| -> Option<Vec<String>> {
        table.get(key)?.as_array().map(|array| {
            array
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
    };
    if table.get("weekdays").is_some() {
        match list("weekdays").and_then(|names| <[String; 7]>::try_from(names).ok()) {
            Some(names) => words.weekdays = names,
            None => warnings.push("time.words.weekdays must list 7 names; ignored".to_string()),
        }
    }
    if table.get("months").is_some() {
        match list("months").and_then(|names| <[String; 12]>::try_from(names).ok()) {
            Some(names) => words.months = names,
            None => warnings.push("time.words.months must list 12 names; ignored".to_string()),
        }
    }
    for (key, slot) in [
        ("clients", &mut words.clients),
        ("internal", &mut words.internal),
        ("admin", &mut words.admin),
        ("title", &mut words.title),
    ] {
        if let Some(text) = table.get(key).and_then(Item::as_str) {
            if text.trim().is_empty() || text.contains([';', ':', '\n']) {
                warnings.push(format!("time.words.{key} is not usable; ignored"));
            } else {
                *slot = text.trim().to_string();
            }
        }
    }
    words
}

/// The `[time]` of the ledger whose main journal is `main`; the defaults when
/// there is no `ledgeline.toml`.
///
/// # Errors
/// [`TimeError::Settings`] when the file is there but unusable.
pub fn read_config(vault: &dyn Vault, main: &VaultPath) -> Result<TimeConfig, TimeError> {
    let path = ledger_file::ledger_path(main)
        .map_err(|_| TimeError::Settings("has no place".to_string()))?;
    match ledger_file::read(vault, &path) {
        LedgerFile::Missing => Ok(TimeConfig::default()),
        LedgerFile::Present { document, .. } => Ok(config_of(&document)),
        LedgerFile::Unusable(reason) => Err(TimeError::Settings(reason.to_string())),
    }
}

/// `text` with `[time] file` set to `file`, every other byte kept.
///
/// # Errors
/// [`TimeError::Settings`] when `text` isn't TOML or `time` isn't a table.
pub fn with_time_file(text: &str, file: &str) -> Result<String, TimeError> {
    let mut document = text
        .parse::<DocumentMut>()
        .map_err(|_| TimeError::Settings("is not valid TOML".to_string()))?;
    if document.get(TIME_TABLE).is_none() {
        document.insert(TIME_TABLE, Item::Table(Table::new()));
    }
    let table = document
        .get_mut(TIME_TABLE)
        .and_then(Item::as_table_like_mut)
        .ok_or_else(|| TimeError::Settings(format!("[{TIME_TABLE}] is not a table")))?;
    table.insert(FILE_KEY, value(file));
    Ok(document.to_string())
}

/// Point `[time] file` at `file` in the ledger's **existing**
/// `ledgeline.toml` (setup's *Use existing*). The file is never created
/// ([[settings-files-toml]]): with none, [`TimeError::Settings`] says so,
/// and the caller shows the line to add by hand. Pointing at the default
/// name needs no setting and writes nothing.
///
/// # Errors
/// [`TimeError::Settings`] (missing, unusable or malformed) or a storage
/// failure; [`TimeError::ExternalChange`] when it changed during the write.
pub fn set_time_file(vault: &dyn Vault, main: &VaultPath, file: &str) -> Result<bool, TimeError> {
    let path = ledger_file::ledger_path(main)
        .map_err(|_| TimeError::Settings("has no place".to_string()))?;
    match ledger_file::read(vault, &path) {
        LedgerFile::Missing if file == DEFAULT_FILE => Ok(false),
        LedgerFile::Missing => Err(TimeError::Settings(format!(
            "does not exist, and Ledgeline does not create it; add `[{TIME_TABLE}]` with \
             `{FILE_KEY} = \"{file}\"` to it by hand"
        ))),
        LedgerFile::Unusable(reason) => Err(TimeError::Settings(reason.to_string())),
        LedgerFile::Present { document, revision } => {
            let before = document.to_string();
            let after = with_time_file(&before, file)?;
            if after == before {
                return Ok(false);
            }
            vault
                .replace(&path, after.as_bytes(), &revision)
                .map(|_| true)
                .map_err(|error| storage(&path, &error))
        }
    }
}

pub(crate) fn storage(path: &VaultPath, error: &VaultError) -> TimeError {
    match error {
        VaultError::Changed { .. } => TimeError::ExternalChange(path.to_string()),
        other => TimeError::Storage {
            path: path.to_string(),
            kind: other.kind(),
        },
    }
}

/// A tracker file pattern: a relative path whose segments may hold
/// `{person}`, `{yyyy}`, `{mm}` and `{dd}` (T3, T15). With no person, a
/// segment that is just `{person}` is dropped, so
/// `time/{person}/{yyyy}-{mm}.timedot` becomes `time/2026-11.timedot` (T16).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackPattern {
    segments: Vec<String>,
    format: TimeFormat,
}

/// `{person}`.
const PERSON: &str = "{person}";

impl TrackPattern {
    /// Parse a pattern.
    ///
    /// # Errors
    /// Why it's unusable: absolute, climbing out, an empty or hidden segment,
    /// or not naming a `.timedot`/`.timeclock` file.
    pub fn new(pattern: &str) -> Result<Self, String> {
        let pattern = pattern.trim();
        if pattern.starts_with(['/', '\\']) || pattern.contains('\\') {
            return Err("it must be a relative path inside the journal's folder".to_string());
        }
        let segments: Vec<String> = pattern.split('/').map(str::to_string).collect();
        if segments
            .iter()
            .any(|s| s.is_empty() || s == "." || s == ".." || s.starts_with('.'))
        {
            return Err(
                "it must be a plain relative path (no `..`, no hidden folders)".to_string(),
            );
        }
        let format = TimeFormat::of_path(pattern)
            .ok_or_else(|| "it must name a `.timedot` or `.timeclock` file".to_string())?;
        Ok(Self { segments, format })
    }

    /// The format its files are.
    #[must_use]
    pub const fn format(&self) -> TimeFormat {
        self.format
    }

    /// Whether it names a person anywhere.
    #[must_use]
    pub fn has_person(&self) -> bool {
        self.segments.iter().any(|s| s.contains(PERSON))
    }

    /// The file for `person` on ISO `date`, relative to the journal's folder.
    #[must_use]
    pub fn render(&self, person: Option<&str>, date: &str) -> String {
        let (year, month, day) = crate::reports::periods::parts(date);
        self.segments
            .iter()
            .filter(|segment| person.is_some() || segment.as_str() != PERSON)
            .map(|segment| {
                segment
                    .replace(PERSON, person.unwrap_or(""))
                    .replace("{yyyy}", &format!("{year:04}"))
                    .replace("{mm}", &format!("{month:02}"))
                    .replace("{dd}", &format!("{day:02}"))
            })
            .collect::<Vec<_>>()
            .join("/")
    }

    /// The person a file at `relative` (to the journal's folder) belongs to,
    /// when it matches the whole pattern and the pattern names one (P-person
    /// step 3).
    #[must_use]
    pub fn person_of(&self, relative: &str) -> Option<String> {
        if !self.has_person() {
            return None;
        }
        let parts: Vec<&str> = relative.split('/').collect();
        if parts.len() != self.segments.len() {
            return None;
        }
        let mut person = None;
        for (pattern, part) in self.segments.iter().zip(&parts) {
            let captured = match_segment(pattern, part)?;
            person = person.or(captured);
        }
        person
    }
}

/// Match one pattern segment against a path segment: `Some(person)` when it
/// matches (the `{person}` capture, if any).
fn match_segment(pattern: &str, text: &str) -> Option<Option<String>> {
    if let Some(rest) = pattern.strip_prefix('{') {
        let (name, after) = rest.split_once('}')?;
        return match name {
            "yyyy" | "mm" | "dd" => {
                let width = if name == "yyyy" { 4 } else { 2 };
                let digits = text.get(..width)?;
                digits
                    .bytes()
                    .all(|b| b.is_ascii_digit())
                    .then(|| match_segment(after, &text[width..]))?
            }
            "person" => (1..=text.len())
                .rev()
                .filter(|&end| text.is_char_boundary(end))
                .find_map(|end| {
                    let candidate = &text[..end];
                    match match_segment(after, &text[end..]) {
                        Some(_) if !candidate.starts_with('.') => Some(Some(candidate.to_string())),
                        _ => None,
                    }
                }),
            _ => None,
        };
    }
    match pattern.chars().next() {
        None => text.is_empty().then_some(None),
        Some(c) => {
            let rest = text.strip_prefix(c)?;
            match_segment(&pattern[c.len_utf8()..], rest)
        }
    }
}

/// Where the time book's files are, all inside the journal's folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimePaths {
    /// The journal's folder.
    pub dir: VaultPath,
    /// The roll-up.
    pub rollup: VaultPath,
    /// The accounts file.
    pub accounts: VaultPath,
    /// The tracker's pattern.
    pub track: TrackPattern,
}

impl TimePaths {
    /// `path` relative to the journal's folder.
    #[must_use]
    pub fn relative(&self, path: &VaultPath) -> String {
        relative_to(&self.dir, path)
    }

    /// The tracker file for `person` on `date`.
    ///
    /// # Errors
    /// [`TimeError::Path`] when the rendered path is unusable (a person name
    /// with a `/`).
    pub fn track_file(&self, person: Option<&str>, date: &str) -> Result<VaultPath, TimeError> {
        let relative = self.track.render(person, date);
        if person.is_some_and(|p| p.contains(['/', '\\']) || p.starts_with('.')) {
            return Err(TimeError::Path {
                path: relative,
                reason: "a person's name can't hold `/` or start with `.`".to_string(),
            });
        }
        place(&self.dir, &relative)
    }
}

/// `path` relative to `dir` (`path` itself when it isn't below it).
#[must_use]
pub fn relative_to(dir: &VaultPath, path: &VaultPath) -> String {
    if dir.is_root() {
        return path.as_str().to_string();
    }
    path.as_str()
        .strip_prefix(dir.as_str())
        .and_then(|rest| rest.strip_prefix('/'))
        .unwrap_or(path.as_str())
        .to_string()
}

fn place(dir: &VaultPath, relative: &str) -> Result<VaultPath, TimeError> {
    dir.join(relative)
        .ok()
        .filter(|path| path.starts_with(dir))
        .ok_or_else(|| TimeError::Path {
            path: relative.to_string(),
            reason: "it must be a relative path inside the journal's folder".to_string(),
        })
}

/// Where the files are for the journal whose main file is `main`.
///
/// # Errors
/// [`TimeError::Path`] for a path outside the journal's folder, one that is
/// the main journal, or a track-file pattern that is unusable.
pub fn paths(main: &VaultPath, config: &TimeConfig) -> Result<TimePaths, TimeError> {
    let dir = main.parent().unwrap_or_default();
    let rollup = place(&dir, config.file())?;
    let accounts = place(&dir, config.accounts_file())?;
    for (path, name) in [
        (&rollup, config.file()),
        (&accounts, config.accounts_file()),
    ] {
        if path == main {
            return Err(TimeError::Path {
                path: name.to_string(),
                reason: "it is the main journal".to_string(),
            });
        }
    }
    let track = TrackPattern::new(config.track_file()).map_err(|reason| TimeError::Path {
        path: config.track_file().to_string(),
        reason,
    })?;
    Ok(TimePaths {
        dir,
        rollup,
        accounts,
        track,
    })
}

/// Read the time book: the roll-up and everything it includes. `None` when
/// there is no roll-up yet (setup hasn't run). `now` ends an open timeclock
/// session.
///
/// A bad line in a time file is a finding in the book, not a failure; a
/// roll-up (or included journal) that doesn't parse is.
///
/// # Errors
/// [`TimeError::Parse`], or [`TimeError::Path`] from [`paths`].
pub fn load(
    vault: &dyn Vault,
    main: &VaultPath,
    config: &TimeConfig,
    now: Option<&Moment>,
) -> Result<Option<TimeBook>, TimeError> {
    let paths = paths(main, config)?;
    if vault.stat(&paths.rollup).ok().flatten().is_none() {
        return Ok(None);
    }
    load_rollup(vault, &paths, config, now).map(Some)
}

/// [`load`] of a roll-up known to exist.
///
/// # Errors
/// [`TimeError::Parse`].
pub fn load_rollup(
    vault: &dyn Vault,
    paths: &TimePaths,
    config: &TimeConfig,
    now: Option<&Moment>,
) -> Result<TimeBook, TimeError> {
    let parsed =
        parse_time_rollup(vault, &paths.rollup).map_err(|e| TimeError::Parse(e.to_string()))?;
    let sources: Vec<TimeSource> = parsed
        .sources
        .into_iter()
        .filter_map(|source| {
            let format = source.time_format?;
            let (text, revision) = source.read?;
            let relative = paths.relative(&source.path);
            Some(TimeSource::new(
                source.path,
                relative,
                format,
                text,
                Some(revision),
            ))
        })
        .collect();
    Ok(TimeBook::build(
        sources,
        &parsed.journal.accounts,
        config,
        now,
    ))
}

/// The `.timedot`/`.timeclock` files under the journal's folder (hidden and
/// tool folders skipped, bounded), relative ids sorted: what setup offers to
/// adopt when there's no roll-up.
#[must_use]
pub fn find_time_files(vault: &dyn Vault, main: &VaultPath) -> Vec<String> {
    let spec = crate::dirscan::ScanSpec {
        name_ok: |name| TimeFormat::of_path(name).is_some(),
        skip_dirs: &["node_modules", "target", "build", "dist", "vendor"],
        max_depth: 6,
        max_files: 2_000,
        symlink_note: "time files are only read through the roll-up",
    };
    let root = main.parent().unwrap_or_default();
    let mut found = crate::dirscan::scan(vault, root, &spec, |id, _, _| id).files;
    found.sort();
    found
}

/// Whether a roll-up's `include`s reach a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Coverage {
    /// An `include` line (as written) reaches it.
    Covered(String),
    /// None does; this line, added to the roll-up, would.
    NotCovered(String),
}

/// Whether the roll-up `rollup_text` (at `rollup`) includes `target` — a file
/// that may not exist yet — directly or through a glob, and else the
/// `include` line to add: a `**` glob for a pattern with folders per person
/// or date, `*` for one varying only in its file name, the file itself
/// otherwise. Only the roll-up's own top-level `include` lines are read.
#[must_use]
pub fn coverage(
    rollup_text: &str,
    rollup: &VaultPath,
    target: &VaultPath,
    pattern: Option<&TrackPattern>,
) -> Coverage {
    let dir = rollup.parent().unwrap_or_default();
    let wanted = relative_to(&dir, target);
    let found = rollup_text.lines().find_map(|line| {
        let rest = line.trim_start().strip_prefix("include")?;
        if !rest.starts_with([' ', '\t']) {
            return None;
        }
        let written = rest.split(';').next().unwrap_or("").trim();
        let path = ["timedot:", "timeclock:", "journal:"]
            .iter()
            .find_map(|prefix| written.strip_prefix(prefix))
            .unwrap_or(written);
        glob_path_match(path, &wanted).then(|| line.trim().to_string())
    });
    match found {
        Some(line) => Coverage::Covered(line),
        None => Coverage::NotCovered(format!("include {}", suggested_glob(&wanted, pattern))),
    }
}

fn suggested_glob(wanted: &str, pattern: Option<&TrackPattern>) -> String {
    let Some(pattern) = pattern else {
        return wanted.to_string();
    };
    let ext = pattern.format().name();
    let segments = &pattern.segments;
    let varying = segments.iter().position(|s| s.contains('{'));
    match varying {
        Some(at) if at + 1 < segments.len() => {
            format!("{}**/*.{ext}", prefix_of(&segments[..at]))
        }
        Some(at) => format!("{}*.{ext}", prefix_of(&segments[..at])),
        None => wanted.to_string(),
    }
}

fn prefix_of(segments: &[String]) -> String {
    segments.iter().map(|s| format!("{s}/")).collect()
}

/// `rollup_text` with `include_line` added after its last `include` (or at
/// the end), every other byte kept.
#[must_use]
pub fn with_include(rollup_text: &str, include_line: &str) -> String {
    let newline = if rollup_text.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let spans = super::line_spans(rollup_text);
    let after = spans
        .iter()
        .rev()
        .find(|(start, end, _)| {
            rollup_text[*start..*end]
                .trim_start()
                .starts_with("include ")
        })
        .map(|(_, _, next)| *next);
    match after {
        Some(at) => {
            let mut out = rollup_text[..at].to_string();
            if !out.ends_with('\n') {
                out.push_str(newline);
            }
            out.push_str(include_line);
            out.push_str(newline);
            out.push_str(&rollup_text[at..]);
            out
        }
        None => {
            let mut out = rollup_text.to_string();
            if !out.is_empty() && !out.ends_with('\n') {
                out.push_str(newline);
            }
            out.push_str(include_line);
            out.push_str(newline);
            out
        }
    }
}

/// One account setup seeds (T-accounts).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeedAccount {
    /// The account.
    pub name: String,
    /// Its tags.
    pub tags: Vec<(String, String)>,
}

/// The default seed: `clients` (billable), `clients:<slug>` per customer
/// (with `customer:`), `internal` (not billable) and `internal:admin`.
#[must_use]
pub fn default_seed(customers: &[&str], config: &TimeConfig) -> Vec<SeedAccount> {
    use super::TagRole;
    let words = &config.words;
    let tag = |role: TagRole, value: &str| (config.tags.name(role).to_string(), value.to_string());
    let mut seed = vec![SeedAccount {
        name: words.clients.clone(),
        tags: vec![tag(TagRole::Billable, "yes")],
    }];
    seed.extend(customers.iter().filter_map(|customer| {
        let slug = slug(customer)?;
        Some(SeedAccount {
            name: format!("{}:{slug}", words.clients),
            tags: vec![tag(TagRole::Customer, customer)],
        })
    }));
    seed.push(SeedAccount {
        name: words.internal.clone(),
        tags: vec![tag(TagRole::Billable, "no")],
    });
    seed.push(SeedAccount {
        name: format!("{}:{}", words.internal, words.admin),
        tags: Vec::new(),
    });
    seed
}

/// A tag- and account-safe short form: lowercase letters and digits, runs of
/// anything else as one `-` (`Acme Corp.` → `acme-corp`). `None` when nothing
/// is left.
#[must_use]
pub fn slug(text: &str) -> Option<String> {
    let mut out = String::new();
    for c in text.chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            out.push(c);
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-').to_string();
    (!out.is_empty()).then_some(out)
}

/// The person setup suggests (P-person): the first word of the OS account's
/// full name, else the login name, reduced to a tag-safe short form
/// (`Patrick Walsh` → `patrick`). `None` is a valid answer.
#[must_use]
pub fn suggest_person(full_name: Option<&str>, login: Option<&str>) -> Option<String> {
    full_name
        .and_then(|name| name.split_whitespace().next())
        .and_then(slug)
        .or_else(|| login.and_then(slug))
}

/// What setup will create.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetupPlan {
    /// Where everything goes.
    pub paths: TimePaths,
    /// The files to create, in order, with their text. The accounts file is
    /// absent when it already exists (it's included as is).
    pub files: Vec<(VaultPath, String)>,
}

/// What [`create`] would write for the default layout (F-files), checked:
/// the roll-up must not exist yet. `person` and `today` name the first
/// tracker file.
///
/// # Errors
/// [`TimeError::Exists`] when the roll-up exists; [`TimeError::Path`].
pub fn setup_plan(
    vault: &dyn Vault,
    main: &VaultPath,
    config: &TimeConfig,
    person: Option<&str>,
    today: &str,
    seed: &[SeedAccount],
) -> Result<SetupPlan, TimeError> {
    let paths = paths(main, config)?;
    let exists = |path: &VaultPath| vault.stat(path).ok().flatten().is_some();
    if exists(&paths.rollup) {
        return Err(TimeError::Exists(paths.relative(&paths.rollup)));
    }
    let main_name = main.file_name().unwrap_or("books.journal");
    let rollup_name = paths.relative(&paths.rollup);
    let rollup_dir = paths.rollup.parent().unwrap_or_default();
    let accounts_include = relative_to(&rollup_dir, &paths.accounts);
    let track = paths.track_file(person, today)?;
    let include = match coverage("", &paths.rollup, &track, Some(&paths.track)) {
        Coverage::NotCovered(line) | Coverage::Covered(line) => line,
    };
    let mut files = vec![(
        paths.rollup.clone(),
        rollup_text(
            main_name,
            &rollup_name,
            &accounts_include,
            &include,
            paths.track.format(),
            config,
        ),
    )];
    if !exists(&paths.accounts) {
        files.push((
            paths.accounts.clone(),
            super::accounts::new_accounts_text(seed, config)?,
        ));
    }
    if !exists(&track) {
        files.push((
            track.clone(),
            track_header(person, today, paths.track.format(), config),
        ));
    }
    Ok(SetupPlan { paths, files })
}

/// The roll-up's text (F-files), declaring the commodity strict mode needs
/// for `format`'s files (fact 10).
fn rollup_text(
    main_name: &str,
    rollup_name: &str,
    accounts_include: &str,
    include: &str,
    format: TimeFormat,
    config: &TimeConfig,
) -> String {
    use super::TagRole;
    let billable = config.tags.name(TagRole::Billable);
    let invoice = config.tags.name(TagRole::Invoice);
    let customer = config.tags.name(TagRole::Customer);
    format!(
        "; Time tracking roll-up. Not included by {main_name} on purpose:\n\
         ; hours are not money. To report on time:\n\
         ;   hledger -f {rollup_name} bal --tree -p thismonth\n\
         ;   hledger -f {rollup_name} bal --pivot {billable} -p lastmonth\n\
         ;   hledger -f {rollup_name} reg not:tag:{invoice} tag:{customer}=Acme\n\
         ; To see time beside the books, include this file from {main_name}.\n\
         \n\
         {TIMEDOT_COMMODITY}\n\
         {clock}\
         \n\
         include {accounts_include}\n\
         {include}\n",
        clock = match format {
            TimeFormat::Timedot => String::new(),
            TimeFormat::Timeclock => format!("{TIMECLOCK_COMMODITY}\n"),
        },
    )
}

/// A new tracker file's header: `# Time — pat — November 2026`, then how to
/// edit it. `#` comments for timedot, `;` for timeclock.
#[must_use]
pub fn track_header(
    person: Option<&str>,
    date: &str,
    format: TimeFormat,
    config: &TimeConfig,
) -> String {
    let words = &config.words;
    let (year, _, _) = crate::reports::periods::parts(date);
    let mark = match format {
        TimeFormat::Timedot => '#',
        TimeFormat::Timeclock => ';',
    };
    let who = person.map(|p| format!(" — {p}")).unwrap_or_default();
    let advice = match format {
        TimeFormat::Timedot => "Keep two spaces before amounts.",
        TimeFormat::Timeclock => "Keep two spaces after the account.",
    };
    format!(
        "{mark} {} {who} — {} {year:04}\n{mark} Written by Ledgeline; hand edits welcome. {advice}\n\n",
        words.title,
        words.month(date),
    )
    .replace("  —", " —")
}

/// Create every file in `plan` (folders as needed), each only if nothing is
/// there: setup's explicit **Create**. Stops at the first file that can't be
/// created; files already created stay (a retry skips them, as the plan is
/// re-made).
///
/// # Errors
/// [`TimeError::Exists`] or a storage failure.
pub fn create(vault: &dyn Vault, plan: &SetupPlan) -> Result<(), TimeError> {
    for (path, text) in &plan.files {
        create_file(vault, path, text)?;
    }
    Ok(())
}

/// Create `path` with `text`, its folders first; never over anything.
pub(crate) fn create_file(
    vault: &dyn Vault,
    path: &VaultPath,
    text: &str,
) -> Result<(), TimeError> {
    if let Some(parent) = path.parent() {
        vault
            .create_dir_all(&parent)
            .map_err(|error| storage(&parent, &error))?;
    }
    vault
        .create_new(path, text.as_bytes())
        .map(|_| ())
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                TimeError::Exists(path.to_string())
            } else {
                storage(path, &error)
            }
        })
}

/// The roll-up's declaration of timedot's commodity (fact 10).
pub const TIMEDOT_COMMODITY: &str =
    "commodity \"\"                      ; time has no commodity (needed for -s)";
/// The roll-up's declaration of timeclock's `h` (fact 10: `hledger -s`
/// refuses a timeclock file's hours without it).
pub const TIMECLOCK_COMMODITY: &str =
    "commodity h                       ; timeclock hours (needed for -s)";

/// The commodity symbol a `commodity` or `D` directive's spec names: `""`
/// for `""`, `h` for `h`, `0.00h` or `1.000,00 h`.
fn spec_symbol(spec: &str) -> String {
    let spec = spec.split(';').next().unwrap_or("").trim();
    if let Some(quoted) = spec.strip_prefix('"') {
        return quoted.split('"').next().unwrap_or("").to_string();
    }
    spec.chars()
        .filter(|c| !(c.is_ascii_digit() || matches!(c, '.' | ',' | ' ' | '-' | '+')))
        .collect()
}

/// The commodities `text` declares with `commodity`, and the `D` default's
/// commodity, if any (top-level directives only).
fn declarations(text: &str) -> (BTreeSet<String>, Option<String>) {
    let mut declared = BTreeSet::new();
    let mut default = None;
    for line in text.lines() {
        if let Some(spec) = line.strip_prefix("commodity")
            && spec.starts_with([' ', '\t'])
        {
            declared.insert(spec_symbol(spec));
        } else if let Some(spec) = line.strip_prefix("D")
            && spec.starts_with([' ', '\t'])
        {
            default = Some(spec_symbol(spec));
        }
    }
    (declared, default)
}

/// The `commodity` lines strict mode (`hledger -s`, fact 10) needs that the
/// roll-up `rollup_text` doesn't declare, for time files of `formats`: the
/// timedot commodity (`""`, or the `D` default's) and timeclock's `h`.
/// `also` is the text of journals it includes, whose declarations count.
#[must_use]
pub fn missing_commodities(
    rollup_text: &str,
    also: &[&str],
    formats: &[TimeFormat],
) -> Vec<String> {
    let mut declared = BTreeSet::new();
    let mut default = None;
    for text in std::iter::once(rollup_text).chain(also.iter().copied()) {
        let (more, d) = declarations(text);
        declared.extend(more);
        default = default.or(d);
    }
    let mut missing = Vec::new();
    if formats.contains(&TimeFormat::Timedot) {
        match default {
            Some(symbol) if !declared.contains(&symbol) => {
                missing.push(format!("commodity {symbol}"));
            }
            None if !declared.contains("") => missing.push(TIMEDOT_COMMODITY.to_string()),
            _ => {}
        }
    }
    if formats.contains(&TimeFormat::Timeclock) && !declared.contains("h") {
        missing.push(TIMECLOCK_COMMODITY.to_string());
    }
    missing
}

/// `rollup_text` with `line` (a `commodity` declaration) added after its
/// last `commodity` line, else before its first `include`, else at the end;
/// every other byte kept.
#[must_use]
pub fn with_declaration(rollup_text: &str, line: &str) -> String {
    let newline = if rollup_text.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let spans = super::line_spans(rollup_text);
    let starts = |(start, end, _): &(usize, usize, usize), prefix: &str| {
        rollup_text[*start..*end].starts_with(prefix)
    };
    let at = spans
        .iter()
        .rev()
        .find(|span| starts(span, "commodity "))
        .map(|(_, _, next)| *next)
        .or_else(|| {
            spans
                .iter()
                .find(|span| starts(span, "include "))
                .map(|(start, _, _)| *start)
        });
    match at {
        Some(at) => {
            let mut out = rollup_text[..at].to_string();
            if !out.is_empty() && !out.ends_with('\n') {
                out.push_str(newline);
            }
            out.push_str(line);
            out.push_str(newline);
            out.push_str(&rollup_text[at..]);
            out
        }
        None => with_include(rollup_text, line),
    }
}

/// What adopting an existing file means (setup's *Use existing*).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdoptPlan {
    /// The file, relative to the journal's folder.
    pub file: String,
    /// A roll-up journal, or a single time file used as a roll-up of one.
    pub format: Option<TimeFormat>,
    /// Whether `[time] file` must be written (it isn't the default name).
    pub needs_setting: bool,
    /// The `commodity` lines `hledger -s` needs that the roll-up lacks
    /// ([`missing_commodities`]), to add by hand; empty for a single time
    /// file (it can hold no directives) or a roll-up that has them.
    pub missing: Vec<String>,
}

/// Check that `file` (relative to the journal's folder) can be the roll-up:
/// it exists, is a `.journal`, `.timedot` or `.timeclock`, isn't the main
/// journal, and reads.
///
/// # Errors
/// [`TimeError::Path`], [`TimeError::Missing`] or [`TimeError::Parse`].
pub fn adopt_plan(vault: &dyn Vault, main: &VaultPath, file: &str) -> Result<AdoptPlan, TimeError> {
    let config = TimeConfig {
        file: Some(file.trim().to_string()),
        ..TimeConfig::default()
    };
    let format = TimeFormat::of_path(file);
    if format.is_none() && !file.ends_with(".journal") {
        return Err(TimeError::Path {
            path: file.to_string(),
            reason: "it must be a `.journal`, `.timedot` or `.timeclock` file".to_string(),
        });
    }
    let paths = paths(main, &config)?;
    if vault.stat(&paths.rollup).ok().flatten().is_none() {
        return Err(TimeError::Missing(file.to_string()));
    }
    let parsed =
        parse_time_rollup(vault, &paths.rollup).map_err(|e| TimeError::Parse(e.to_string()))?;
    let formats: Vec<TimeFormat> = parsed
        .sources
        .iter()
        .filter_map(|source| source.time_format)
        .collect();
    let journals: Vec<&str> = parsed
        .sources
        .iter()
        .filter(|source| source.time_format.is_none())
        .filter_map(|source| source.read.as_ref().map(|(text, _)| text.as_str()))
        .collect();
    let missing = match (format, journals.split_first()) {
        (None, Some((rollup, also))) => missing_commodities(rollup, also, &formats),
        _ => Vec::new(),
    };
    Ok(AdoptPlan {
        file: file.trim().to_string(),
        format,
        needs_setting: file.trim() != DEFAULT_FILE,
        missing,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::MemVault;

    fn main_path() -> VaultPath {
        VaultPath::new("books.journal").expect("path")
    }

    #[test]
    fn config_reads_every_key_and_warns() {
        let config = parse_config(
            "[time]\nfile = \"t/time.journal\"\naccounts-file = \"t/acc.journal\"\n\
             track-file = \"t/{yyyy}.timeclock\"\nrecord = \"timeclock\"\nweekday = \"none\"\n\
             [time.tags]\ninvoice = \"rechnung\"\n[time.words]\nclients = \"kunden\"\n\
             weekdays = [\"Mo\",\"Di\",\"Mi\",\"Do\",\"Fr\",\"Sa\",\"So\"]\n",
        );
        assert_eq!(config.file(), "t/time.journal");
        assert_eq!(config.accounts_file(), "t/acc.journal");
        assert_eq!(config.track_file(), "t/{yyyy}.timeclock");
        assert_eq!(config.record, RecordMode::Timeclock);
        assert_eq!(config.weekday, WeekdayStyle::None);
        assert_eq!(config.tags.name(super::super::TagRole::Invoice), "rechnung");
        assert_eq!(config.words.clients, "kunden");
        assert_eq!(config.words.weekday("2026-11-26"), "Do");
        assert!(config.warnings.is_empty(), "{:?}", config.warnings);
        let bad = parse_config("[time]\nfile = \"x.txt\"\nrecord = \"paper\"\n");
        assert_eq!(bad.file(), DEFAULT_FILE);
        assert_eq!(bad.warnings.len(), 2);
        assert_eq!(TimeConfig::default().track_file(), DEFAULT_TRACK_FILE);
    }

    #[test]
    fn track_patterns_render_and_match() {
        let pattern = TrackPattern::new(DEFAULT_TRACK_FILE).expect("pattern");
        assert_eq!(
            pattern.render(Some("pat"), "2026-11-26"),
            "time/pat/2026-11.timedot"
        );
        assert_eq!(pattern.render(None, "2026-11-26"), "time/2026-11.timedot");
        assert_eq!(
            pattern.person_of("time/sam/2026-11.timedot").as_deref(),
            Some("sam")
        );
        assert_eq!(pattern.person_of("time/imported/2026-q3.timeclock"), None);
        assert_eq!(pattern.person_of("time/2026-11.timedot"), None);
        let inline = TrackPattern::new("t/{person}-{yyyy}.timedot").expect("inline");
        assert_eq!(
            inline.person_of("t/ann-lee-2026.timedot").as_deref(),
            Some("ann-lee")
        );
        assert!(TrackPattern::new("../x.timedot").is_err());
        assert!(TrackPattern::new("x.journal").is_err());
    }

    #[test]
    fn coverage_and_include_lines() {
        let rollup = VaultPath::new("time.journal").expect("path");
        let text = "commodity \"\"\ninclude time-accounts.journal\ninclude time/**/*.timedot\n";
        let pattern = TrackPattern::new(DEFAULT_TRACK_FILE).expect("pattern");
        let target = VaultPath::new("time/pat/2026-12.timedot").expect("path");
        assert_eq!(
            coverage(text, &rollup, &target, Some(&pattern)),
            Coverage::Covered("include time/**/*.timedot".to_string())
        );
        let clock = VaultPath::new("time/pat/2026-12.timeclock").expect("path");
        let clock_pattern = TrackPattern::new(DEFAULT_TRACK_FILE_TIMECLOCK).expect("pattern");
        let Coverage::NotCovered(line) = coverage(text, &rollup, &clock, Some(&clock_pattern))
        else {
            panic!("not covered");
        };
        assert_eq!(line, "include time/**/*.timeclock");
        let added = with_include(text, &line);
        assert!(added.ends_with("include time/**/*.timedot\ninclude time/**/*.timeclock\n"));
        let flat = TrackPattern::new("time/{yyyy}-{mm}.timedot").expect("flat");
        let flat_target = VaultPath::new("time/2026-12.timedot").expect("path");
        assert_eq!(
            coverage("", &rollup, &flat_target, Some(&flat)),
            Coverage::NotCovered("include time/*.timedot".to_string())
        );
    }

    /// Fact 10: `hledger -s` refuses timeclock hours unless `commodity h` is
    /// declared (verified with `hledger -s -f time.journal check` on
    /// `fixtures/time`, 2026-11-26).
    #[test]
    fn strict_mode_declarations_for_timeclock() {
        let vault = MemVault::with_root("/books");
        vault.insert_file("books.journal", "");
        let config = parse_config(
            "[time]\ntrack-file = \"time/{person}/{yyyy}-{mm}.timeclock\"\nrecord = \"timeclock\"\n",
        );
        let plan = setup_plan(
            &vault,
            &main_path(),
            &config,
            Some("pat"),
            "2026-11-26",
            &[],
        )
        .expect("plan");
        let rollup = &plan.files[0].1;
        assert!(
            rollup.contains(&format!(
                "{TIMEDOT_COMMODITY}\n{TIMECLOCK_COMMODITY}\n\ninclude "
            )),
            "{rollup}"
        );
        let both = [TimeFormat::Timedot, TimeFormat::Timeclock];
        assert!(missing_commodities(rollup, &[], &both).is_empty());
        // A timedot setup declares only "".
        let plan = setup_plan(
            &vault,
            &main_path(),
            &TimeConfig::default(),
            None,
            "2026-11-26",
            &[],
        )
        .expect("plan");
        assert!(!plan.files[0].1.contains("commodity h"));
        assert_eq!(
            missing_commodities(&plan.files[0].1, &[], &both),
            vec![TIMECLOCK_COMMODITY.to_string()]
        );
        // Declarations in an included journal count; a `D` needs its own.
        assert!(missing_commodities("", &["commodity \"\"\ncommodity 0.00h\n"], &both).is_empty());
        assert_eq!(
            missing_commodities("D 1.00 h\n", &[], &[TimeFormat::Timedot]),
            vec!["commodity h".to_string()]
        );
        // Added after the last commodity line, else before the includes.
        assert_eq!(
            with_declaration("; x\ncommodity \"\"\n\ninclude a\n", "commodity h"),
            "; x\ncommodity \"\"\ncommodity h\n\ninclude a\n"
        );
        assert_eq!(
            with_declaration("; x\ninclude a\n", "commodity h"),
            "; x\ncommodity h\ninclude a\n"
        );
        // Adopt reports what's missing; a single time file can't hold it.
        vault.insert_file(
            "hours.journal",
            "commodity \"\"\ninclude t.timeclock\ninclude d.timedot\n",
        );
        vault.insert_file("t.timeclock", "i 2026-11-01 08:00 a\no 2026-11-01 09:00\n");
        vault.insert_file("d.timedot", "2026-11-01\n  a  1h\n");
        let adopted = adopt_plan(&vault, &main_path(), "hours.journal").expect("adopt");
        assert_eq!(adopted.missing, vec![TIMECLOCK_COMMODITY.to_string()]);
        let single = adopt_plan(&vault, &main_path(), "t.timeclock").expect("adopt");
        assert!(single.missing.is_empty());
    }

    #[test]
    fn setup_creates_the_default_files_and_hledger_layout() {
        let vault = MemVault::with_root("/books");
        vault.insert_file("books.journal", "2026-01-01 x\n  a  $1\n  b\n");
        let config = TimeConfig::default();
        let seed = default_seed(&["Acme Corp"], &config);
        let plan = setup_plan(
            &vault,
            &main_path(),
            &config,
            Some("pat"),
            "2026-11-26",
            &seed,
        )
        .expect("plan");
        let names: Vec<&str> = plan.files.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "time.journal",
                "time-accounts.journal",
                "time/pat/2026-11.timedot"
            ]
        );
        assert!(
            plan.files[0]
                .1
                .contains("include time-accounts.journal\ninclude time/**/*.timedot\n")
        );
        assert!(plan.files[0].1.contains("commodity \"\""));
        assert_eq!(
            plan.files[2].1,
            "# Time — pat — November 2026\n# Written by Ledgeline; hand edits welcome. Keep two spaces before amounts.\n\n"
        );
        create(&vault, &plan).expect("create");
        let book = load(&vault, &main_path(), &config, None)
            .expect("load")
            .expect("book");
        assert!(book.entries.is_empty());
        let acme = book.account("clients:acme-corp").expect("acme");
        assert!(acme.is_billable());
        assert_eq!(acme.customer_name(), Some("Acme Corp"));
        assert!(matches!(
            setup_plan(&vault, &main_path(), &config, None, "2026-11-26", &seed),
            Err(TimeError::Exists(_))
        ));
        assert_eq!(
            find_time_files(&vault, &main_path()),
            vec!["time/pat/2026-11.timedot"]
        );
    }

    #[test]
    fn adopt_and_settings() {
        let vault = MemVault::with_root("/books");
        vault.insert_file("books.journal", "");
        vault.insert_file("hours.timedot", "2026-11-01\n  a  1h\n");
        let plan = adopt_plan(&vault, &main_path(), "hours.timedot").expect("adopt");
        assert!(plan.needs_setting);
        assert_eq!(plan.format, Some(TimeFormat::Timedot));
        assert!(matches!(
            set_time_file(&vault, &main_path(), "hours.timedot"),
            Err(TimeError::Settings(_))
        ));
        vault.insert_file("ledgeline.toml", "# mine\n[ui]\ntheme = \"dark\"\n");
        assert!(set_time_file(&vault, &main_path(), "hours.timedot").expect("set"));
        let text = vault
            .read_to_string(&VaultPath::new("ledgeline.toml").expect("p"))
            .expect("read");
        assert_eq!(
            text,
            "# mine\n[ui]\ntheme = \"dark\"\n\n[time]\nfile = \"hours.timedot\"\n"
        );
        let config = read_config(&vault, &main_path()).expect("config");
        let book = load(&vault, &main_path(), &config, None)
            .expect("load")
            .expect("book");
        assert_eq!(book.entries.len(), 1);
        assert!(adopt_plan(&vault, &main_path(), "nope.journal").is_err());
        assert_eq!(
            suggest_person(Some("Patrick Walsh"), Some("pwalsh")).as_deref(),
            Some("patrick")
        );
        assert_eq!(suggest_person(None, Some("pw")).as_deref(), Some("pw"));
        assert_eq!(slug("Acme Corp."), Some("acme-corp".to_string()));
    }
}
