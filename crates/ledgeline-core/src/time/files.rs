//! Writing time files: [`TimeFiles`] holds each file's text and the revision
//! it was read at, applies [`super::write`] edits to them in memory, and saves
//! only over exactly what was read — the journal editor's discipline, through
//! the same machinery ([`crate::edit::write_pinned`]): refuse to write when a
//! file changed on disk since it was read, atomically replace each one, and
//! create a new file only where nothing is (T-running "Two machines").
//!
//! On top of that, the tracker's operations, which can touch more than one
//! file: [`TimeFiles::start`], [`TimeFiles::stop`] (a session across
//! midnight lands in each day's block, and across a month end in each month's
//! file), [`TimeFiles::add_minutes`], [`TimeFiles::add_entry`], and
//! [`rename_plan`] for renaming an account across every time file and the
//! accounts file.

use super::accounts::AccountsDoc;
use super::book::{RecordMode, TimeConfig, TimePaths, create_file, storage, track_header};
use super::model::{Running, TimeBook};
use super::timeclock::Moment;
use super::write::{self, DayStyle, RenamedLine};
use super::{TimeError, TimeFormat, TimeQty, timedot};
use crate::edit::{EditError, PinnedWrite, file_changed_externally, write_pinned};
use crate::vault::{Revision, Vault, VaultPath};
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Debug, Clone)]
struct FileState {
    text: String,
    /// What was read; `None` when nothing was there (a file to create).
    revision: Option<Revision>,
    dirty: bool,
}

/// Time files being edited.
#[derive(Debug, Clone)]
pub struct TimeFiles {
    vault: Arc<dyn Vault>,
    files: BTreeMap<VaultPath, FileState>,
}

/// Who and where the tracker writes.
#[derive(Debug, Clone, Copy)]
pub struct Tracker<'a> {
    /// The book's paths (the track-file pattern above all).
    pub paths: &'a TimePaths,
    /// The settings (record mode, weekday, tags, words).
    pub config: &'a TimeConfig,
    /// The person tracking, if any (P-person).
    pub person: Option<&'a str>,
}

impl Tracker<'_> {
    /// How new date lines are written.
    #[must_use]
    pub fn style(&self) -> DayStyle<'_> {
        DayStyle {
            tags: &self.config.tags,
            words: &self.config.words,
            weekday: self.config.weekday,
            person: self.person,
        }
    }

    /// The tracker file for `date`.
    ///
    /// # Errors
    /// [`TimeError::Path`].
    pub fn file_for(&self, date: &str) -> Result<VaultPath, TimeError> {
        self.paths.track_file(self.person, date)
    }

    fn header(&self, date: &str) -> String {
        track_header(self.person, date, self.paths.track.format(), self.config)
    }
}

impl TimeFiles {
    /// Nothing loaded yet: files are read as edits reach them.
    #[must_use]
    pub fn new(vault: Arc<dyn Vault>) -> Self {
        Self {
            vault,
            files: BTreeMap::new(),
        }
    }

    /// Seeded with every time file `book` read, at the revisions it read, so
    /// an edit lands only over what the book shows.
    #[must_use]
    pub fn with_book(vault: Arc<dyn Vault>, book: &TimeBook) -> Self {
        let files = book
            .sources
            .iter()
            .filter_map(|source| {
                Some((
                    source.path.clone(),
                    FileState {
                        text: source.text.clone(),
                        revision: Some(source.revision.clone()?),
                        dirty: false,
                    },
                ))
            })
            .collect();
        Self { vault, files }
    }

    fn load(&mut self, path: &VaultPath) -> Result<&mut FileState, TimeError> {
        if !self.files.contains_key(path) {
            let state = match self.vault.read_pinned(path, None) {
                Ok((bytes, revision)) => FileState {
                    text: String::from_utf8(bytes)
                        .map_err(|_| TimeError::Invalid(format!("{path} is not valid UTF-8")))?,
                    revision: Some(revision),
                    dirty: false,
                },
                Err(error) if error.is_not_found() => FileState {
                    text: String::new(),
                    revision: None,
                    dirty: false,
                },
                Err(error) => return Err(storage(path, &error)),
            };
            self.files.insert(path.clone(), state);
        }
        self.files
            .get_mut(path)
            .ok_or_else(|| TimeError::NotFound(path.to_string()))
    }

    /// The file's current text (as edited so far); empty for one not there.
    ///
    /// # Errors
    /// A storage failure, or a file that isn't UTF-8.
    pub fn text(&mut self, path: &VaultPath) -> Result<&str, TimeError> {
        self.load(path).map(|state| state.text.as_str())
    }

    /// Whether the file is on disk (or will be created by [`Self::save`]).
    ///
    /// # Errors
    /// As [`Self::text`].
    pub fn exists(&mut self, path: &VaultPath) -> Result<bool, TimeError> {
        self.load(path)
            .map(|state| state.revision.is_some() || state.dirty)
    }

    /// Apply `edit` (old text → new text) to the file, in memory. A file not
    /// there starts as `new_header`; with `None` it must exist.
    ///
    /// # Errors
    /// [`TimeError::Missing`], or whatever `edit` refuses.
    pub fn edit(
        &mut self,
        path: &VaultPath,
        new_header: Option<&str>,
        edit: impl FnOnce(&str) -> Result<String, TimeError>,
    ) -> Result<(), TimeError> {
        let state = self.load(path)?;
        let base = if state.revision.is_none() && !state.dirty {
            match new_header {
                Some(header) => header.to_string(),
                None => return Err(TimeError::Missing(path.to_string())),
            }
        } else {
            state.text.clone()
        };
        let text = edit(&base)?;
        if text != state.text || state.revision.is_none() {
            state.text = text;
            state.dirty = true;
        }
        Ok(())
    }

    /// The files an edit changed, with their new text: the preview.
    #[must_use]
    pub fn pending(&self) -> Vec<(&VaultPath, &str, bool)> {
        self.files
            .iter()
            .filter(|(_, state)| state.dirty)
            .map(|(path, state)| (path, state.text.as_str(), state.revision.is_none()))
            .collect()
    }

    /// Write every changed file: none if any existing one changed on disk
    /// since it was read; new files first (each only where nothing is), then
    /// every changed existing file through [`write_pinned`]. Returns what was
    /// written.
    ///
    /// # Errors
    /// [`TimeError::ExternalChange`] (nothing written, or — if a file appeared
    /// or changed in the microseconds between the check and its own write —
    /// the files before it written) or a storage failure.
    pub fn save(&mut self) -> Result<Vec<VaultPath>, TimeError> {
        let changed_on_disk = |path: &VaultPath, error: EditError| match error {
            EditError::ExternalChange => TimeError::ExternalChange(path.to_string()),
            other => TimeError::Storage {
                path: path.to_string(),
                kind: match other {
                    EditError::Io(io) => io.kind(),
                    _ => std::io::ErrorKind::Other,
                },
            },
        };
        for (path, state) in self.files.iter().filter(|(_, s)| s.dirty) {
            if let Some(revision) = &state.revision
                && file_changed_externally(&*self.vault, path, revision)
                    .map_err(|e| changed_on_disk(path, e))?
            {
                return Err(TimeError::ExternalChange(path.to_string()));
            }
        }
        let mut saved = Vec::new();
        let created: Vec<VaultPath> = self
            .files
            .iter()
            .filter(|(_, s)| s.dirty && s.revision.is_none())
            .map(|(path, _)| path.clone())
            .collect();
        for path in created {
            let text = self.files[&path].text.clone();
            create_file(&*self.vault, &path, &text).map_err(|error| match error {
                TimeError::Exists(_) => TimeError::ExternalChange(path.to_string()),
                other => other,
            })?;
            let revision = Revision::of_bytes(text.as_bytes());
            if let Some(state) = self.files.get_mut(&path) {
                state.revision = Some(
                    self.vault
                        .read_pinned(&path, None)
                        .map(|(_, r)| r)
                        .unwrap_or(revision),
                );
                state.dirty = false;
            }
            saved.push(path);
        }
        let writes: Vec<PinnedWrite> = self
            .files
            .iter()
            .filter(|(_, s)| s.dirty)
            .filter_map(|(path, s)| {
                Some(PinnedWrite {
                    path: path.clone(),
                    text: s.text.clone(),
                    expected: s.revision.clone()?,
                })
            })
            .collect();
        let vault = Arc::clone(&self.vault);
        let files = &mut self.files;
        let result = write_pinned(&*vault, &writes, |at, revision| {
            if let Some(state) = files.get_mut(&writes[at].path) {
                state.revision = Some(revision);
                state.dirty = false;
            }
            saved.push(writes[at].path.clone());
        });
        result.map_err(|error| {
            let path = writes
                .iter()
                .find(|w| files.get(&w.path).is_some_and(|s| s.dirty))
                .map_or_else(VaultPath::root, |w| w.path.clone());
            changed_on_disk(&path, error)
        })?;
        Ok(saved)
    }

    // -----------------------------------------------------------------------
    // The tracker
    // -----------------------------------------------------------------------

    /// Add `minutes` to `account` on `date` in the tracker's file (T6),
    /// creating the file with its header when it's new. Returns the minutes
    /// really added. Timedot only: in timeclock mode time is sessions.
    ///
    /// # Errors
    /// As [`write::add_minutes`]; [`TimeError::Invalid`] in timeclock mode.
    pub fn add_minutes(
        &mut self,
        tracker: &Tracker<'_>,
        date: &str,
        account: &str,
        minutes: i64,
    ) -> Result<i64, TimeError> {
        let path = tracker.file_for(date)?;
        if TimeFormat::of_path(path.as_str()) != Some(TimeFormat::Timedot) {
            return Err(TimeError::Invalid(
                "minutes can be added to a timedot file only; edit the session instead".to_string(),
            ));
        }
        let style = tracker.style();
        let mut applied = 0;
        self.edit(&path, Some(&tracker.header(date)), |text| {
            let added = write::add_minutes(text, date, account, minutes, &style)?;
            applied = added.applied;
            Ok(added.text)
        })?;
        Ok(applied)
    }

    /// Add an entry (Entries → Add) in the tracker's file for `date`.
    ///
    /// # Errors
    /// As [`write::insert_entry`]; [`TimeError::Invalid`] in timeclock mode.
    pub fn add_entry(
        &mut self,
        tracker: &Tracker<'_>,
        date: &str,
        account: &str,
        quantity: TimeQty,
        note: &str,
        tags: &[(String, String)],
    ) -> Result<VaultPath, TimeError> {
        let path = tracker.file_for(date)?;
        if TimeFormat::of_path(path.as_str()) != Some(TimeFormat::Timedot) {
            return Err(TimeError::Invalid(
                "entries are added to a timedot file; in timeclock mode clock in and out"
                    .to_string(),
            ));
        }
        let style = tracker.style();
        self.edit(&path, Some(&tracker.header(date)), |text| {
            write::insert_entry(text, date, account, quantity, note, tags, &style)
        })?;
        Ok(path)
    }

    /// Start timing `account` at `at` (T7, T13): a marker under that day in
    /// timedot mode, an `i` line in timeclock mode. Stopping whatever runs is
    /// the caller's ([`Self::stop`] first, then this: one press switches).
    ///
    /// # Errors
    /// As [`write::start_marker`] / [`write::clock_in`].
    pub fn start(
        &mut self,
        tracker: &Tracker<'_>,
        account: &str,
        at: &Moment,
    ) -> Result<VaultPath, TimeError> {
        let path = tracker.file_for(&at.date)?;
        let style = tracker.style();
        let header = tracker.header(&at.date);
        match tracker.config.record {
            RecordMode::Timedot => self.edit(&path, Some(&header), |text| {
                write::start_marker(text, &at.date, account, at.seconds, &style)
            })?,
            RecordMode::Timeclock => {
                let person_tag = tracker.person.map(|person| {
                    (
                        tracker.config.tags.name(super::TagRole::Person).to_string(),
                        person.to_string(),
                    )
                });
                self.edit(&path, Some(&header), |text| {
                    write::clock_in(
                        text,
                        at,
                        account,
                        "",
                        &person_tag.into_iter().collect::<Vec<_>>(),
                    )
                })?;
            }
        }
        Ok(path)
    }

    /// Stop `running` at `now` (T-running). A timedot marker becomes minutes
    /// rounded half up ([`write::stop_plan`]), added to each day's open line
    /// in each day's file — across midnight, and across a month end into the
    /// next month's file — and the marker is removed; under 30 s only the
    /// marker goes. A timeclock session gets its `o` line, with seconds, in
    /// the file it was clocked in. Returns the minutes per day (timeclock:
    /// the exact seconds as one entry per day, rounded only for display).
    ///
    /// # Errors
    /// [`TimeError::NotFound`] when the marker or session is gone (re-read);
    /// as the edits.
    pub fn stop(
        &mut self,
        tracker: &Tracker<'_>,
        running: &Running,
        now: &Moment,
    ) -> Result<Vec<(String, i64)>, TimeError> {
        match running.format {
            TimeFormat::Timeclock => {
                self.edit(&running.source.file, None, |text| {
                    write::clock_out(text, now, &running.account)
                })?;
                Ok(super::timeclock::split_at_midnight(&running.since, now)
                    .into_iter()
                    .map(|(date, qty)| (date, qty.round_minutes()))
                    .collect())
            }
            TimeFormat::Timedot => {
                let plan = write::stop_plan(&running.since, now);
                for (date, minutes) in &plan {
                    let added = self.add_minutes(tracker, date, &running.account, *minutes)?;
                    if added != *minutes {
                        return Err(TimeError::Verify(format!(
                            "{minutes} minutes were to be added on {date}, but {added} were"
                        )));
                    }
                }
                self.edit(&running.source.file, None, |text| {
                    let file = timedot::parse(text);
                    let (line, _) = file
                        .markers()
                        .find(|(_, marker)| marker.account == running.account)
                        .ok_or_else(|| {
                            TimeError::NotFound(format!(
                                "{} is no longer running in {}",
                                running.account, running.source.file
                            ))
                        })?;
                    write::remove_marker(text, line)
                })?;
                Ok(plan)
            }
        }
    }
}

/// A rename across the time book (T-accounts): every changed file's new
/// text, and every changed line for the preview.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenamePlan {
    /// The files and their new text.
    pub files: Vec<(VaultPath, String)>,
    /// Each changed line, by file.
    pub preview: Vec<(VaultPath, RenamedLine)>,
}

impl RenamePlan {
    /// Put every file's new text into `files` (then [`TimeFiles::save`]).
    ///
    /// # Errors
    /// As [`TimeFiles::edit`].
    pub fn apply_to(&self, files: &mut TimeFiles) -> Result<(), TimeError> {
        for (path, text) in &self.files {
            files.edit(path, None, |_| Ok(text.clone()))?;
        }
        Ok(())
    }
}

/// Plan renaming `old` (and every account below it) to `new` in every time
/// file of `book` and in the accounts file (`accounts`: its path and text),
/// each file verified. Billed lines keep their tags.
///
/// # Errors
/// [`TimeError::Invalid`] for a bad name or a clash; [`TimeError::Verify`].
pub fn rename_plan(
    book: &TimeBook,
    accounts: Option<(&VaultPath, &str)>,
    old: &str,
    new: &str,
) -> Result<RenamePlan, TimeError> {
    let mut plan = RenamePlan {
        files: Vec::new(),
        preview: Vec::new(),
    };
    for source in &book.sources {
        let (text, lines) = match source.format() {
            TimeFormat::Timedot => write::rename_in_timedot(&source.text, old, new)?,
            TimeFormat::Timeclock => write::rename_in_timeclock(&source.text, old, new)?,
        };
        if lines.is_empty() {
            continue;
        }
        plan.preview
            .extend(lines.into_iter().map(|line| (source.path.clone(), line)));
        plan.files.push((source.path.clone(), text));
    }
    if let Some((path, text)) = accounts {
        let doc = AccountsDoc::parse(text);
        if doc
            .declarations()
            .iter()
            .any(|d| super::model::is_under(&d.name, old))
        {
            let renamed = doc.rename(old, new)?;
            plan.preview.extend(
                text.lines()
                    .zip(renamed.lines())
                    .enumerate()
                    .filter(|(_, (a, b))| a != b)
                    .map(|(at, (before, after))| {
                        (
                            path.clone(),
                            RenamedLine {
                                line: u32::try_from(at + 1).unwrap_or(u32::MAX),
                                before: before.to_string(),
                                after: after.to_string(),
                            },
                        )
                    }),
            );
            plan.files.push((path.clone(), renamed));
        }
    }
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::book::{self, TimeConfig, default_seed, setup_plan};
    use crate::vault::MemVault;

    fn setup() -> (Arc<MemVault>, VaultPath, TimeConfig) {
        let vault = Arc::new(MemVault::with_root("/books"));
        vault.insert_file("books.journal", "");
        let main = VaultPath::new("books.journal").expect("main");
        let config = TimeConfig::default();
        let plan = setup_plan(
            &*vault,
            &main,
            &config,
            Some("pat"),
            "2026-11-30",
            &default_seed(&[], &config),
        )
        .expect("plan");
        book::create(&*vault, &plan).expect("create");
        (vault, main, config)
    }

    #[test]
    fn stop_across_midnight_and_month_end_writes_both_files() {
        let (vault, main, config) = setup();
        let paths = book::paths(&main, &config).expect("paths");
        let tracker = Tracker {
            paths: &paths,
            config: &config,
            person: Some("pat"),
        };
        let mut files = TimeFiles::new(vault.clone());
        files
            .start(
                &tracker,
                "clients:acme",
                &Moment::new("2026-11-30", 23 * 3600 + 40 * 60 + 40),
            )
            .expect("start");
        files.save().expect("save");
        let book = book::load(&*vault, &main, &config, None)
            .expect("load")
            .expect("book");
        assert_eq!(book.running.len(), 1);
        let mut files = TimeFiles::with_book(vault.clone(), &book);
        let plan = files
            .stop(
                &tracker,
                &book.running[0],
                &Moment::new("2026-12-01", 20 * 60),
            )
            .expect("stop");
        assert_eq!(
            plan,
            vec![
                ("2026-11-30".to_string(), 19),
                ("2026-12-01".to_string(), 20)
            ]
        );
        let saved = files.save().expect("save");
        assert_eq!(saved.len(), 2);
        let dec = vault
            .read_to_string(&VaultPath::new("time/pat/2026-12.timedot").expect("p"))
            .expect("dec");
        assert_eq!(
            dec,
            "# Time — pat — December 2026\n# Written by Ledgeline; hand edits welcome. Keep two spaces before amounts.\n\n2026-12-01 Tue  ; person: pat\n    clients:acme                     20m\n"
        );
        let book = book::load(&*vault, &main, &config, None)
            .expect("load")
            .expect("book");
        assert!(book.running.is_empty());
        let total: i64 = book
            .entries
            .iter()
            .map(|e| e.quantity.round_minutes())
            .sum();
        assert_eq!(total, 39);
    }

    #[test]
    fn refuses_to_write_over_an_outside_change() {
        let (vault, main, config) = setup();
        let paths = book::paths(&main, &config).expect("paths");
        let tracker = Tracker {
            paths: &paths,
            config: &config,
            person: Some("pat"),
        };
        let book = book::load(&*vault, &main, &config, None)
            .expect("load")
            .expect("book");
        let mut files = TimeFiles::with_book(vault.clone(), &book);
        files
            .add_minutes(&tracker, "2026-11-30", "x", 15)
            .expect("add");
        vault.insert_file("time/pat/2026-11.timedot", "# someone else\n");
        assert!(matches!(files.save(), Err(TimeError::ExternalChange(_))));
        assert_eq!(
            vault
                .read_to_string(&VaultPath::new("time/pat/2026-11.timedot").expect("p"))
                .expect("read"),
            "# someone else\n"
        );
    }

    #[test]
    fn timeclock_mode_and_rename() {
        let vault = Arc::new(MemVault::with_root("/books"));
        vault.insert_file("books.journal", "");
        vault.insert_file(
            "time.journal",
            "include time-accounts.journal\ninclude time/**/*.timeclock\n",
        );
        vault.insert_file(
            "time-accounts.journal",
            "account clients:acme  ; customer: Acme\n",
        );
        vault.insert_file("time/2026-11.timeclock", "; clock\n");
        let main = VaultPath::new("books.journal").expect("main");
        let config = TimeConfig {
            record: RecordMode::Timeclock,
            ..TimeConfig::default()
        };
        let paths = book::paths(&main, &config).expect("paths");
        let tracker = Tracker {
            paths: &paths,
            config: &config,
            person: None,
        };
        let mut files = TimeFiles::new(vault.clone());
        files
            .start(
                &tracker,
                "clients:acme",
                &Moment::new("2026-11-26", 9 * 3600 + 1),
            )
            .expect("start");
        files.save().expect("save");
        let book = book::load(&*vault, &main, &config, None)
            .expect("load")
            .expect("book");
        let mut files = TimeFiles::with_book(vault.clone(), &book);
        files
            .stop(
                &tracker,
                &book.running[0],
                &Moment::new("2026-11-26", 10 * 3600 + 2),
            )
            .expect("stop");
        files.save().expect("save");
        let book = book::load(&*vault, &main, &config, None)
            .expect("load")
            .expect("book");
        assert_eq!(book.entries[0].quantity, TimeQty::from_seconds(3601));
        let accounts_path = paths.accounts.clone();
        let accounts_text = vault.read_to_string(&accounts_path).expect("accounts");
        let plan = rename_plan(
            &book,
            Some((&accounts_path, &accounts_text)),
            "clients",
            "kunden",
        )
        .expect("plan");
        assert_eq!(plan.files.len(), 2);
        assert_eq!(plan.preview.len(), 3);
        let mut files = TimeFiles::with_book(vault.clone(), &book);
        files
            .edit(&accounts_path, None, |t| Ok(t.to_string()))
            .expect("load accounts");
        plan.apply_to(&mut files).expect("apply");
        files.save().expect("save");
        let book = book::load(&*vault, &main, &config, None)
            .expect("load")
            .expect("book");
        assert_eq!(book.entries[0].account, "kunden:acme");
        assert_eq!(
            book.account("kunden:acme").and_then(|a| a.customer_name()),
            Some("Acme")
        );
    }
}
