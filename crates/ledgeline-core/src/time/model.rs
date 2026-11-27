//! What a time book is once read: entries, accounts with their tags resolved,
//! running timers, and the files it came from.
//!
//! [`TimeBook::build`] turns a roll-up's sources ([`TimeSource`]) and account
//! declarations into one book. Nothing here reads storage; [`super::book`]
//! does, then calls this.

use super::book::TimeConfig;
use super::timeclock::{self, Moment, TimeclockFile};
use super::timedot::{self, TimedotFile};
use super::{Finding, TagNames, TagRole, TimeFormat, TimeQty, is_yes};
use crate::model::AccountDeclaration;
use crate::vault::{Revision, VaultPath};
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

/// Where an entry (or a marker) is written: a file and its lines.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SourceRef {
    /// The file, in the books' vault.
    pub file: VaultPath,
    /// 1-based line numbers, end exclusive: the entry line, or a session's
    /// `i` line through its `o` line.
    pub lines: Range<u32>,
}

/// How an entry is written, for the writer to find it again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Origin {
    /// A timedot entry line (0-based index into its file's lines), and the
    /// letter, for one letter of a letters entry.
    Timedot {
        /// The line index.
        line: usize,
        /// The letter.
        letter: Option<char>,
    },
    /// One day of a timeclock session (index into its file's sessions).
    Timeclock {
        /// The session index.
        session: usize,
    },
}

/// One posting of time, as hledger would read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimeEntry {
    /// ISO date.
    pub date: String,
    /// The time account.
    pub account: String,
    /// How long, exactly.
    pub quantity: TimeQty,
    /// Who did it (P-person), when known.
    pub person: Option<String>,
    /// The entry's note: its comment's free text.
    pub note: String,
    /// Every tag that applies: the entry's own first, then its day's (or its
    /// session's), as `hledger reg tag:…` matches either.
    pub tags: Vec<(String, String)>,
    /// Where it's written.
    pub source: SourceRef,
    /// How it's written.
    pub origin: Origin,
}

impl TimeEntry {
    /// The value of `role` among the entry's tags.
    #[must_use]
    pub fn tag<'t>(&'t self, names: &TagNames, role: TagRole) -> Option<&'t str> {
        names.value(&self.tags, role)
    }

    /// Whether it went on an invoice, a bill, or was written off: any of
    /// `invoice`, `bill`, `nobill`.
    #[must_use]
    pub fn is_settled(&self, names: &TagNames) -> bool {
        [TagRole::Invoice, TagRole::Bill, TagRole::Nobill]
            .into_iter()
            .any(|role| names.has(&self.tags, role))
    }
}

/// A tag value resolved up the account tree, with where it was declared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    /// The value.
    pub value: String,
    /// The account that declares it (the account itself, or an ancestor).
    pub from: String,
}

/// A time account: declared, used, or the ancestor of one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimeAccount {
    /// The full name.
    pub name: String,
    /// Whether an `account` directive declares it.
    pub declared: bool,
    /// Whether any entry uses it directly.
    pub used: bool,
    /// Its own declared tags.
    pub tags: Vec<(String, String)>,
    /// `customer:`, nearest ancestor wins.
    pub customer: Option<Resolved>,
    /// `billable:`, nearest ancestor wins; `None` when nothing in the chain
    /// says (which means no).
    pub billable: Option<Resolved>,
    /// `item:`, nearest ancestor wins.
    pub item: Option<Resolved>,
    /// `archived:`, nearest ancestor wins.
    pub archived: Option<Resolved>,
}

impl TimeAccount {
    /// Billable, defaulting to no.
    #[must_use]
    pub fn is_billable(&self) -> bool {
        self.billable.as_ref().is_some_and(|r| is_yes(&r.value))
    }

    /// Archived, defaulting to no.
    #[must_use]
    pub fn is_archived(&self) -> bool {
        self.archived.as_ref().is_some_and(|r| is_yes(&r.value))
    }

    /// The customer, if linked.
    #[must_use]
    pub fn customer_name(&self) -> Option<&str> {
        self.customer
            .as_ref()
            .map(|r| r.value.as_str())
            .filter(|v| !v.is_empty())
    }
}

/// A running timer: a timedot marker (T7) or an open timeclock session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Running {
    /// The account being timed.
    pub account: String,
    /// When it started.
    pub since: Moment,
    /// Who, when known.
    pub person: Option<String>,
    /// The marker or `i` line.
    pub source: SourceRef,
    /// Which format records it.
    pub format: TimeFormat,
}

/// One time file as parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedTime {
    /// A timedot file.
    Timedot(TimedotFile),
    /// A timeclock file.
    Timeclock(TimeclockFile),
}

impl ParsedTime {
    /// Parse `text` as `format`.
    #[must_use]
    pub fn parse(format: TimeFormat, text: &str) -> Self {
        match format {
            TimeFormat::Timedot => Self::Timedot(timedot::parse(text)),
            TimeFormat::Timeclock => Self::Timeclock(timeclock::parse(text)),
        }
    }

    /// Its findings.
    #[must_use]
    pub fn findings(&self) -> &[Finding] {
        match self {
            Self::Timedot(file) => &file.findings,
            Self::Timeclock(file) => &file.findings,
        }
    }
}

/// One time file the roll-up includes: where, what it said, and what was
/// read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimeSource {
    /// Where.
    pub path: VaultPath,
    /// Its path relative to the books' folder (what the track-file pattern
    /// matches against).
    pub relative: String,
    /// The exact text read.
    pub text: String,
    /// The revision of those bytes, when they came from storage.
    pub revision: Option<Revision>,
    /// The parse.
    pub parsed: ParsedTime,
}

impl TimeSource {
    /// Parse `text` read from `path`.
    #[must_use]
    pub fn new(
        path: VaultPath,
        relative: String,
        format: TimeFormat,
        text: String,
        revision: Option<Revision>,
    ) -> Self {
        let parsed = ParsedTime::parse(format, &text);
        Self {
            path,
            relative,
            text,
            revision,
            parsed,
        }
    }

    /// Its format.
    #[must_use]
    pub fn format(&self) -> TimeFormat {
        match self.parsed {
            ParsedTime::Timedot(_) => TimeFormat::Timedot,
            ParsedTime::Timeclock(_) => TimeFormat::Timeclock,
        }
    }
}

/// A finding located in a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileFinding {
    /// The file.
    pub file: VaultPath,
    /// The finding.
    pub finding: Finding,
}

/// A loaded time book.
#[derive(Debug, Clone)]
pub struct TimeBook {
    /// The tag names in force.
    pub tags: TagNames,
    /// Every time account (declared, used, or an ancestor of one), by name.
    pub accounts: BTreeMap<String, TimeAccount>,
    /// Every entry, in file order (files in include order).
    pub entries: Vec<TimeEntry>,
    /// Running timers.
    pub running: Vec<Running>,
    /// The time files.
    pub sources: Vec<TimeSource>,
    /// What the parsers found.
    pub findings: Vec<FileFinding>,
}

impl TimeBook {
    /// Build a book from its time files and the account declarations its
    /// roll-up read. `now` ends a timeclock session still open (hledger
    /// clocks it to now); with `None` an open session counts nothing yet.
    #[must_use]
    pub fn build(
        sources: Vec<TimeSource>,
        declarations: &[AccountDeclaration],
        config: &TimeConfig,
        now: Option<&Moment>,
    ) -> Self {
        let tags = config.tags.clone();
        let pattern = config.track_pattern();
        let mut entries = Vec::new();
        let mut running = Vec::new();
        let mut findings = Vec::new();
        for source in &sources {
            let folder_person = pattern
                .as_ref()
                .and_then(|pattern| pattern.person_of(&source.relative));
            findings.extend(source.parsed.findings().iter().map(|finding| FileFinding {
                file: source.path.clone(),
                finding: finding.clone(),
            }));
            match &source.parsed {
                ParsedTime::Timedot(file) => timedot_entries(
                    source,
                    file,
                    &tags,
                    folder_person.as_deref(),
                    &mut entries,
                    &mut running,
                ),
                ParsedTime::Timeclock(file) => timeclock_entries(
                    source,
                    file,
                    &tags,
                    folder_person.as_deref(),
                    now,
                    &mut entries,
                    &mut running,
                ),
            }
        }
        let accounts = resolve_accounts(declarations, &entries, &tags);
        Self {
            tags,
            accounts,
            entries,
            running,
            sources,
            findings,
        }
    }

    /// The account `name`, if the book knows it.
    #[must_use]
    pub fn account(&self, name: &str) -> Option<&TimeAccount> {
        self.accounts.get(name)
    }

    /// The source at `path`.
    #[must_use]
    pub fn source(&self, path: &VaultPath) -> Option<&TimeSource> {
        self.sources.iter().find(|source| &source.path == path)
    }

    /// Accounts used by entries but not declared (Accounts → *undeclared*).
    #[must_use]
    pub fn undeclared(&self) -> Vec<&str> {
        self.accounts
            .values()
            .filter(|account| account.used && !account.declared)
            .map(|account| account.name.as_str())
            .collect()
    }

    /// Every person named anywhere in the book, sorted.
    #[must_use]
    pub fn people(&self) -> Vec<&str> {
        self.entries
            .iter()
            .filter_map(|entry| entry.person.as_deref())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }
}

fn timedot_entries(
    source: &TimeSource,
    file: &TimedotFile,
    tags: &TagNames,
    folder_person: Option<&str>,
    entries: &mut Vec<TimeEntry>,
    running: &mut Vec<Running>,
) {
    entries.extend(file.postings().map(|posting| {
        let mut all = posting.entry.tags.clone();
        if posting.letter.is_some() {
            // One posting per letter: only this letter's `t:` applies.
            all.retain(|(name, value)| {
                name != "t" || posting.letter.map(String::from).as_deref() == Some(value)
            });
        }
        all.extend(posting.day.tags.iter().cloned());
        let number = file.lines[posting.line].number;
        TimeEntry {
            date: posting.day.date.clone(),
            account: posting.entry.account.clone(),
            quantity: posting.quantity,
            person: tags
                .value(&all, TagRole::Person)
                .filter(|p| !p.is_empty())
                .map(str::to_string)
                .or_else(|| folder_person.map(str::to_string)),
            note: posting.entry.note.clone(),
            tags: all,
            source: SourceRef {
                file: source.path.clone(),
                lines: number..number + 1,
            },
            origin: Origin::Timedot {
                line: posting.line,
                letter: posting.letter,
            },
        }
    }));
    running.extend(file.markers().filter_map(|(at, marker)| {
        let day = file.date_line(marker.day)?;
        let number = file.lines[at].number;
        Some(Running {
            account: marker.account.clone(),
            since: Moment::new(day.date.clone(), marker.since),
            person: tags
                .value(&day.tags, TagRole::Person)
                .filter(|p| !p.is_empty())
                .map(str::to_string)
                .or_else(|| folder_person.map(str::to_string)),
            source: SourceRef {
                file: source.path.clone(),
                lines: number..number + 1,
            },
            format: TimeFormat::Timedot,
        })
    }));
}

fn timeclock_entries(
    source: &TimeSource,
    file: &TimeclockFile,
    tags: &TagNames,
    folder_person: Option<&str>,
    now: Option<&Moment>,
    entries: &mut Vec<TimeEntry>,
    running: &mut Vec<Running>,
) {
    for (index, session) in file.sessions.iter().enumerate() {
        let first = file.lines[session.in_line].number;
        let last = session.out_line.map_or(first, |at| file.lines[at].number);
        let source_ref = SourceRef {
            file: source.path.clone(),
            lines: first..last + 1,
        };
        let person = tags
            .value(&session.tags, TagRole::Person)
            .filter(|p| !p.is_empty())
            .map(str::to_string)
            .or_else(|| folder_person.map(str::to_string));
        let note = session
            .comments
            .iter()
            .map(|comment| crate::edit::free_text(comment).join(", "))
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("; ");
        if session.end.is_none() {
            running.push(Running {
                account: session.account.clone(),
                since: session.start.clone(),
                person: person.clone(),
                source: source_ref.clone(),
                format: TimeFormat::Timeclock,
            });
        }
        entries.extend(
            session
                .segments(now)
                .into_iter()
                .map(|(date, quantity)| TimeEntry {
                    date,
                    account: session.account.clone(),
                    quantity,
                    person: person.clone(),
                    note: note.clone(),
                    tags: session.tags.clone(),
                    source: source_ref.clone(),
                    origin: Origin::Timeclock { session: index },
                }),
        );
    }
}

/// Every account the book knows — declared, used, and each one's ancestors —
/// with tags resolved nearest-ancestor-wins (fact 7).
fn resolve_accounts(
    declarations: &[AccountDeclaration],
    entries: &[TimeEntry],
    names: &TagNames,
) -> BTreeMap<String, TimeAccount> {
    // The last declaration of a name wins its tags, as a later `account`
    // directive's do in hledger.
    let declared: BTreeMap<&str, &Vec<(String, String)>> = declarations
        .iter()
        .map(|decl| (decl.name.0.as_str(), &decl.tags))
        .collect();
    let used: BTreeSet<&str> = entries.iter().map(|entry| entry.account.as_str()).collect();
    let all: BTreeSet<String> = declared
        .keys()
        .copied()
        .chain(used.iter().copied())
        .flat_map(ancestors_and_self)
        .collect();
    let resolve = |name: &str, role: TagRole| -> Option<Resolved> {
        ancestors_and_self(name).into_iter().rev().find_map(|at| {
            let tags = declared.get(at.as_str())?;
            names.value(tags, role).map(|value| Resolved {
                value: value.to_string(),
                from: at.clone(),
            })
        })
    };
    all.into_iter()
        .map(|name| {
            let account = TimeAccount {
                declared: declared.contains_key(name.as_str()),
                used: used.contains(name.as_str()),
                tags: declared
                    .get(name.as_str())
                    .map(|tags| (*tags).clone())
                    .unwrap_or_default(),
                customer: resolve(&name, TagRole::Customer),
                billable: resolve(&name, TagRole::Billable),
                item: resolve(&name, TagRole::Item),
                archived: resolve(&name, TagRole::Archived),
                name: name.clone(),
            };
            (name, account)
        })
        .collect()
}

/// `a`, `a:b`, `a:b:c` for `a:b:c`: root first.
#[must_use]
pub fn ancestors_and_self(name: &str) -> Vec<String> {
    name.match_indices(':')
        .map(|(at, _)| name[..at].to_string())
        .chain(std::iter::once(name.to_string()))
        .collect()
}

/// Whether `account` is `branch` or below it.
#[must_use]
pub fn is_under(account: &str, branch: &str) -> bool {
    account == branch
        || account
            .strip_prefix(branch)
            .is_some_and(|rest| rest.starts_with(':'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AccountName, SourcePos};

    fn decl(name: &str, tags: &[(&str, &str)]) -> AccountDeclaration {
        AccountDeclaration {
            name: AccountName(name.to_string()),
            tags: tags
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect(),
            comment: String::new(),
            position: SourcePos { line: 1, column: 1 },
            source_file: "/x".into(),
        }
    }

    fn source(path: &str, text: &str) -> TimeSource {
        let format = TimeFormat::of_path(path).expect("format");
        TimeSource::new(
            VaultPath::new(path).expect("path"),
            path.to_string(),
            format,
            text.to_string(),
            None,
        )
    }

    #[test]
    fn resolves_nearest_ancestor_and_people() {
        let decls = vec![
            decl("clients", &[("billable", "yes")]),
            decl(
                "clients:acme",
                &[("customer", "Acme Corp"), ("item", "ARCH")],
            ),
            decl("clients:acme:travel", &[("billable", "no")]),
            decl("clients:acme:tax", &[("item", "TAX")]),
        ];
        let sources = vec![
            source(
                "time/pat/2026-11.timedot",
                "2026-11-25 Wed  ; person: pat\n    clients:acme:tax  130m\n    clients:acme:travel  1h  ; person: sam\n",
            ),
            source(
                "time/sam/2026-11.timedot",
                "2026-11-25\n    clients:acme:advisory  30m\n    ; ● running clients:acme:tax  since 09:00:00\n",
            ),
            source(
                "time/imported/2026-q3.timeclock",
                "i 2026-09-01 09:00 clients:globex  x\n",
            ),
        ];
        let book = TimeBook::build(
            sources,
            &decls,
            &TimeConfig::default(),
            Some(&Moment::new("2026-09-01", 10 * 3600)),
        );
        let tax = book.account("clients:acme:tax").expect("tax");
        assert!(tax.is_billable());
        assert_eq!(tax.customer_name(), Some("Acme Corp"));
        assert_eq!(
            tax.item.as_ref().map(|r| r.from.as_str()),
            Some("clients:acme:tax")
        );
        let travel = book.account("clients:acme:travel").expect("travel");
        assert!(!travel.is_billable());
        assert_eq!(
            travel.billable.as_ref().map(|r| r.from.as_str()),
            Some("clients:acme:travel")
        );
        let advisory = book.account("clients:acme:advisory").expect("advisory");
        assert!(!advisory.declared && advisory.used);
        assert_eq!(
            advisory.item.as_ref().map(|r| r.value.as_str()),
            Some("ARCH")
        );
        // Under `clients`, so billable by inheritance.
        assert!(
            book.account("clients:globex")
                .expect("globex")
                .is_billable()
        );
        let people: Vec<Option<&str>> = book.entries.iter().map(|e| e.person.as_deref()).collect();
        // Entry tag beats the day's; the folder fills in; `imported` doesn't
        // match the pattern's file name, so it stays unattributed.
        assert_eq!(people, vec![Some("pat"), Some("sam"), Some("sam"), None]);
        assert_eq!(book.entries[3].quantity, TimeQty::from_minutes(60));
        assert_eq!(book.running.len(), 2);
        assert_eq!(book.running[0].person.as_deref(), Some("sam"));
        assert_eq!(
            book.undeclared(),
            vec!["clients:acme:advisory", "clients:globex"]
        );
    }

    #[test]
    fn ancestry() {
        assert_eq!(ancestors_and_self("a:b:c"), vec!["a", "a:b", "a:b:c"]);
        assert!(is_under("a:b:c", "a:b"));
        assert!(!is_under("a:bc", "a:b"));
    }
}
