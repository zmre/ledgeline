//! Rules-file **families** (plan 32 D27): one rules file per year, all for the
//! same account, read as one family, and which of them a given date belongs to.
//!
//! A common layout keeps one rules file per year — `citi-2025.csv.rules`,
//! `citi-2026.csv.rules` — or one per year directory —
//! `2025/citi.csv.rules`, `2026/citi.csv.rules`. They are near-identical, so
//! they score alike against a statement and all name the same `account1`.
//! This module groups them and picks one for a date, with no metadata beyond
//! the names:
//!
//! - **The family** ([`family_of`]) is the file's id with its year or date
//!   token removed, from the name and from any directory component that is
//!   wholly a year or date. Two files are one family exactly when those keys
//!   are equal, so a family is always "the same name in the same directory,
//!   or in sibling year directories".
//! - **Within a family** ([`order_family`]): the file for the date's year,
//!   else the newest year at or before it. With no date (or no stamps), the
//!   newest `.latest` date, then mtime. When the year's own file is chosen but
//!   a later year exists, that newest file comes second, for a user importing
//!   ahead.
//!
//! Both the import's candidate ranking ([`super::matching::rank_in_families`])
//! and the journal's rules-file inference ([`super::infer`]) go through
//! [`order_family`], so the two can never disagree about which file of a
//! family comes first. Pure: no I/O.

use std::cmp::Ordering;
use std::ops::Range;
use std::time::SystemTime;

/// What replaces a year or date token in a [`FamilyKey`]. A NUL cannot occur
/// in a file name, so a key with one can never equal a real id by accident.
const PLACEHOLDER: char = '\0';

/// The earliest and latest years a token is read as. Outside this, four
/// digits are an account number or a branch code, not a year.
const YEARS: std::ops::RangeInclusive<u16> = 1900..=2099;

/// The year (and, when the token carries them, month and day) a rules file is
/// named for: `2026`, `2026-03`, `2026-03-15`, `202603`, `20260315`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Stamp {
    /// The year.
    pub year: u16,
    /// The month, `1..=12`, when the token has one.
    pub month: Option<u8>,
    /// The day, `1..=31`, when the token has one.
    pub day: Option<u8>,
}

impl Stamp {
    /// The first day the stamp covers, as `(year, month, day)`: `2026` is
    /// 2026-01-01 and `2026-03` is 2026-03-01.
    fn start(self) -> (u16, u8, u8) {
        (self.year, self.month.unwrap_or(1), self.day.unwrap_or(1))
    }
}

/// Which family a rules file belongs to: its id with every year or date token
/// replaced by a placeholder. Equal keys, one family.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FamilyKey(String);

impl FamilyKey {
    /// The key as a string. Opaque: it holds a NUL where each token was.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A rules file's family, read from its id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Family {
    /// The family it is in.
    pub key: FamilyKey,
    /// The family's display name: the file's label with the token removed and
    /// the separators around it trimmed (`citi-2026.csv.rules` → `citi`).
    pub label: String,
    /// The year it is named for — the file name's token first, else the
    /// nearest year directory's — or `None`.
    pub stamp: Option<Stamp>,
}

/// The family of the rules file `id` (a forward-slash relative path).
///
/// `import/citi-2026.csv.rules` and `import/citi-2025.csv.rules` are one
/// family, and so are `2025/citi.csv.rules` and `2026/citi.csv.rules`;
/// `citi.csv.rules` (no year) is a family of its own.
#[must_use]
pub fn family_of(id: &str) -> Family {
    let mut parts: Vec<&str> = id.split('/').collect();
    let name = parts.pop().unwrap_or(id);
    let mut dir_stamp = None;
    let dirs: Vec<String> = parts
        .iter()
        .map(|component| match whole_stamp(component) {
            Some(stamp) => {
                dir_stamp = Some(stamp);
                PLACEHOLDER.to_string()
            }
            None => (*component).to_string(),
        })
        .collect();
    let (stem, name_stamp) = match find_stamp(name) {
        Some((range, stamp)) => (
            format!(
                "{}{PLACEHOLDER}{}",
                &name[..range.start],
                &name[range.end..]
            ),
            Some(stamp),
        ),
        None => (name.to_string(), None),
    };
    let label = family_label(&stem, &parts);
    let key = dirs
        .into_iter()
        .chain(std::iter::once(stem))
        .collect::<Vec<_>>()
        .join("/");
    Family {
        key: FamilyKey(key),
        label,
        stamp: name_stamp.or(dir_stamp),
    }
}

/// The display name of a family whose file name, token removed, is `stem`.
fn family_label(stem: &str, dirs: &[&str]) -> String {
    let plain = stem.replace(PLACEHOLDER, "");
    let label = super::label_for(&plain);
    let trimmed = label.trim_matches(|c: char| matches!(c, '-' | '_' | '.' | ' '));
    if !trimmed.is_empty() {
        return trimmed.to_string();
    }
    // A file named for nothing but its year (`2026.csv.rules`) is known by its
    // directory.
    dirs.iter()
        .rev()
        .find(|dir| whole_stamp(dir).is_none())
        .map_or_else(|| label.clone(), |dir| (*dir).to_string())
}

/// The stamp a whole path component is, or `None` when the component is
/// anything more than a year or date.
fn whole_stamp(component: &str) -> Option<Stamp> {
    find_stamp(component)
        .filter(|(range, _)| range.start == 0 && range.end == component.len())
        .map(|(_, stamp)| stamp)
}

/// The first year or date token in `name`, and where it sits.
///
/// A token is a maximal run of digits that reads as `YYYY`, `YYYYMM` or
/// `YYYYMMDD` with a plausible year, or a `YYYY` followed by `-`, `_` or `.`
/// and a two-digit month (and then a day the same way). Being a MAXIMAL run is
/// what keeps `12025` or an account number's middle from reading as a year.
fn find_stamp(name: &str) -> Option<(Range<usize>, Stamp)> {
    let runs = digit_runs(name);
    let bytes = name.as_bytes();
    let two_digits_after = |end: usize| -> Option<(u8, usize)> {
        let sep = *bytes.get(end)?;
        if !matches!(sep, b'-' | b'_' | b'.') {
            return None;
        }
        let run = runs.iter().find(|run| run.start == end + 1)?;
        (run.len() == 2)
            .then(|| name[run.clone()].parse().ok())
            .flatten()
            .map(|value| (value, run.end))
    };
    runs.iter().find_map(|run| {
        let digits = &name[run.clone()];
        let year: u16 = digits.get(..4)?.parse().ok()?;
        if !YEARS.contains(&year) {
            return None;
        }
        let at = |range: Range<usize>| digits.get(range).and_then(|text| text.parse::<u8>().ok());
        match digits.len() {
            4 => {
                let (month, end) = match two_digits_after(run.end) {
                    Some((month, end)) if valid_month(month) => (Some(month), end),
                    _ => (None, run.end),
                };
                let (day, end) = match month.and_then(|_| two_digits_after(end)) {
                    Some((day, after)) if valid_day(day) => (Some(day), after),
                    _ => (None, end),
                };
                Some((run.start..end, Stamp { year, month, day }))
            }
            6 => {
                let month = at(4..6).filter(|month| valid_month(*month))?;
                Some((
                    run.clone(),
                    Stamp {
                        year,
                        month: Some(month),
                        day: None,
                    },
                ))
            }
            8 => {
                let month = at(4..6).filter(|month| valid_month(*month))?;
                let day = at(6..8).filter(|day| valid_day(*day))?;
                Some((
                    run.clone(),
                    Stamp {
                        year,
                        month: Some(month),
                        day: Some(day),
                    },
                ))
            }
            _ => None,
        }
    })
}

fn valid_month(month: u8) -> bool {
    (1..=12).contains(&month)
}

fn valid_day(day: u8) -> bool {
    (1..=31).contains(&day)
}

/// Every maximal run of ASCII digits in `text`, as byte ranges.
fn digit_runs(text: &str) -> Vec<Range<usize>> {
    let mut runs = Vec::new();
    let mut start = None;
    for (at, byte) in text.bytes().enumerate() {
        match (byte.is_ascii_digit(), start) {
            (true, None) => start = Some(at),
            (false, Some(from)) => {
                runs.push(from..at);
                start = None;
            }
            _ => {}
        }
    }
    if let Some(from) = start {
        runs.push(from..text.len());
    }
    runs
}

/// A `YYYY-MM-DD` date as `(year, month, day)`, or `None`.
fn ymd(date: &str) -> Option<(u16, u8, u8)> {
    let mut parts = date.get(..10)?.split('-');
    let year = parts.next()?.parse().ok()?;
    let month = parts.next()?.parse().ok()?;
    let day = parts.next()?.parse().ok()?;
    Some((year, month, day))
}

/// One file of a family, as [`order_family`] weighs it.
#[derive(Debug, Clone, Copy)]
pub struct Member<'a> {
    /// The rules id: the last tie-break, so the order is total.
    pub id: &'a str,
    /// [`Family::stamp`].
    pub stamp: Option<Stamp>,
    /// The newest date its `.latest.NAME` marker records, `YYYY-MM-DD`.
    pub latest: Option<&'a str>,
    /// Its mtime. Ranking only, as everywhere (see `DiscoveredRules::modified`).
    pub modified: Option<SystemTime>,
}

/// Why a member is where [`order_family`] put it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    /// Named for the date's own year.
    ForYear(u16),
    /// No file for the date's year; the newest year before it.
    NearestBefore(u16),
    /// Every file is for a later year; the earliest of them.
    NearestAfter(u16),
    /// No date or no stamps: the most recently used (`.latest`, then mtime).
    Newest,
    /// Second, after the year's own file: the family's newest, for a later
    /// year — someone importing ahead.
    Ahead(u16),
    /// Everything else, newest first.
    Rest,
}

/// The order `members` (one family) are offered in for a transaction or
/// statement dated `date`, as `(index into members, why)`; the first is the
/// pick.
///
/// 1. With a date: the member for its year (several, as with monthly files:
///    the newest one starting on or before the date); else the newest year
///    before it; else the earliest year after it.
/// 2. With no date, or no member stamped: the newest by stamp, then
///    `.latest` date, then mtime.
/// 3. When step 1 found the year's own file and a later year exists, the
///    newest file is second ([`Placement::Ahead`]).
/// 4. The rest follow newest first. The id breaks every remaining tie.
#[must_use]
pub fn order_family(members: &[Member<'_>], date: Option<&str>) -> Vec<(usize, Placement)> {
    let mut sorted: Vec<usize> = (0..members.len()).collect();
    sorted.sort_by(|&a, &b| newest_first(&members[a], &members[b]));
    let Some(&newest) = sorted.first() else {
        return Vec::new();
    };
    let stamp_of = |at: usize| members[at].stamp;
    let target = date.and_then(ymd);

    let pick = target.and_then(|(year, month, day)| {
        let in_year: Vec<usize> = sorted
            .iter()
            .copied()
            .filter(|&at| stamp_of(at).is_some_and(|stamp| stamp.year == year))
            .collect();
        if let Some(&first) = in_year.first() {
            let started = in_year
                .iter()
                .copied()
                .find(|&at| stamp_of(at).is_some_and(|stamp| stamp.start() <= (year, month, day)));
            return Some((started.unwrap_or(first), Placement::ForYear(year)));
        }
        if let Some(before) = sorted
            .iter()
            .copied()
            .find(|&at| stamp_of(at).is_some_and(|stamp| stamp.year < year))
        {
            return Some((before, Placement::NearestBefore(stamp_of(before)?.year)));
        }
        sorted
            .iter()
            .copied()
            .rev()
            .find(|&at| stamp_of(at).is_some())
            .and_then(|after| Some((after, Placement::NearestAfter(stamp_of(after)?.year))))
    });
    let (first, why) = pick.unwrap_or((newest, Placement::Newest));

    let mut order = vec![(first, why)];
    if let (Placement::ForYear(year), Some(latest)) = (why, stamp_of(newest))
        && newest != first
        && latest.year > year
    {
        order.push((newest, Placement::Ahead(latest.year)));
    }
    order.extend(
        sorted
            .into_iter()
            .filter(|at| order.iter().all(|(placed, _)| placed != at))
            .map(|at| (at, Placement::Rest))
            .collect::<Vec<_>>(),
    );
    order
}

/// Whether [`order_family`]'s pick (the first of `order`) was decided by a
/// D27 rule — stamp, `.latest` date or mtime — rather than by the id alone.
/// `false` when another member is indistinguishable from it, which is a
/// choice the user has to make (Patrick, 2026-10-31: "if after our rules there
/// are still multiple possible rules files, let the user decide").
#[must_use]
pub fn pick_is_decided(members: &[Member<'_>], order: &[(usize, Placement)]) -> bool {
    let Some(&(pick, _)) = order.first() else {
        return false;
    };
    let same = |a: &Member<'_>, b: &Member<'_>| {
        a.stamp == b.stamp && a.latest == b.latest && a.modified == b.modified
    };
    members
        .iter()
        .enumerate()
        .all(|(at, member)| at == pick || !same(member, &members[pick]))
}

/// Newest first: by stamp, then `.latest` date, then mtime (each absent one
/// last), then id ascending.
fn newest_first(a: &Member<'_>, b: &Member<'_>) -> Ordering {
    b.stamp
        .cmp(&a.stamp)
        .then_with(|| b.latest.cmp(&a.latest))
        .then_with(|| b.modified.cmp(&a.modified))
        .then_with(|| a.id.cmp(b.id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn year(year: u16) -> Option<Stamp> {
        Some(Stamp {
            year,
            month: None,
            day: None,
        })
    }

    #[test]
    fn a_year_in_the_name_or_a_year_directory_makes_one_family() {
        let a = family_of("import/citi-2025.csv.rules");
        let b = family_of("import/citi-2026.csv.rules");
        assert_eq!(a.key, b.key);
        assert_eq!(a.label, "citi");
        assert_eq!(a.stamp, year(2025));
        assert_eq!(b.stamp, year(2026));

        let c = family_of("bank/2025/citi.csv.rules");
        let d = family_of("bank/2026/citi.csv.rules");
        assert_eq!(c.key, d.key);
        assert_eq!(c.stamp, year(2025));
        assert_eq!(c.label, "citi");

        // Not sibling year directories, and not the same name: other families.
        assert_ne!(family_of("other/2025/citi.csv.rules").key, c.key);
        assert_ne!(family_of("import/citi.csv.rules").key, a.key);
        assert_ne!(family_of("import/chase-2025.csv.rules").key, a.key);
        assert_eq!(family_of("import/citi.csv.rules").stamp, None);
    }

    #[test]
    fn date_tokens_and_lookalikes() {
        let stamp = |id: &str| family_of(id).stamp;
        assert_eq!(
            stamp("citi-2026-03.csv.rules"),
            Some(Stamp {
                year: 2026,
                month: Some(3),
                day: None
            })
        );
        assert_eq!(
            stamp("citi_20260315.csv.rules"),
            Some(Stamp {
                year: 2026,
                month: Some(3),
                day: Some(15)
            })
        );
        assert_eq!(
            family_of("citi-2026-03.csv.rules").key,
            family_of("citi-2025-11.csv.rules").key
        );
        // An account number, a five-digit run, an impossible month.
        assert_eq!(stamp("acct-4412.csv.rules"), None);
        assert_eq!(stamp("acct-12025.csv.rules"), None);
        assert_eq!(stamp("x-202613.csv.rules"), None);
        // A file named only for its year takes its directory's name.
        assert_eq!(family_of("citi/2026.csv.rules").label, "citi");
    }

    fn members<'a>(stamps: &[(&'a str, Option<u16>)]) -> Vec<Member<'a>> {
        stamps
            .iter()
            .map(|(id, stamp)| Member {
                id,
                stamp: stamp.and_then(year),
                latest: None,
                modified: None,
            })
            .collect()
    }

    fn picked<'a>(members: &[Member<'a>], date: Option<&str>) -> Vec<(&'a str, Placement)> {
        order_family(members, date)
            .into_iter()
            .map(|(at, why)| (members[at].id, why))
            .collect()
    }

    #[test]
    fn the_transactions_year_wins_then_the_newest_before_it() {
        let family = members(&[
            ("c-2024", Some(2024)),
            ("c-2025", Some(2025)),
            ("c-2026", Some(2026)),
        ]);
        assert_eq!(
            picked(&family, Some("2025-06-01")),
            [
                ("c-2025", Placement::ForYear(2025)),
                ("c-2026", Placement::Ahead(2026)),
                ("c-2024", Placement::Rest),
            ]
        );
        assert_eq!(
            picked(&family, Some("2026-01-02"))[0],
            ("c-2026", Placement::ForYear(2026))
        );
        // The year's own file is the newest: nobody is importing ahead.
        assert_eq!(
            picked(&family, Some("2026-01-02"))[1],
            ("c-2025", Placement::Rest)
        );
        // A year with no file: the newest at or before it.
        assert_eq!(
            picked(&family, Some("2028-03-01"))[0],
            ("c-2026", Placement::NearestBefore(2026))
        );
        // Older than every file: the earliest.
        assert_eq!(
            picked(&family, Some("2020-03-01"))[0],
            ("c-2024", Placement::NearestAfter(2024))
        );
        // No date: the newest.
        assert_eq!(picked(&family, None)[0], ("c-2026", Placement::Newest));
    }

    #[test]
    fn monthly_files_pick_the_one_started_by_the_date() {
        let month = |id, month| Member {
            id,
            stamp: Some(Stamp {
                year: 2026,
                month: Some(month),
                day: None,
            }),
            latest: None,
            modified: None,
        };
        let family = [month("jan", 1), month("mar", 3), month("jun", 6)];
        assert_eq!(
            picked(&family, Some("2026-04-10"))[0],
            ("mar", Placement::ForYear(2026))
        );
        assert_eq!(
            picked(&family, Some("2026-07-10"))[0],
            ("jun", Placement::ForYear(2026))
        );
    }

    #[test]
    fn with_no_stamps_the_latest_marker_then_mtime_decides() {
        let at = |seconds| Some(SystemTime::UNIX_EPOCH + Duration::from_secs(seconds));
        let family = [
            Member {
                id: "a",
                stamp: None,
                latest: Some("2026-01-31"),
                modified: at(1),
            },
            Member {
                id: "b",
                stamp: None,
                latest: Some("2026-02-28"),
                modified: at(0),
            },
            Member {
                id: "c",
                stamp: None,
                latest: None,
                modified: at(9),
            },
        ];
        let order: Vec<&str> = picked(&family, Some("2026-03-01"))
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(order, ["b", "a", "c"]);
        assert_eq!(picked(&family, None)[0].1, Placement::Newest);

        let mtime = [
            Member {
                id: "a",
                stamp: None,
                latest: None,
                modified: at(1),
            },
            Member {
                id: "b",
                stamp: None,
                latest: None,
                modified: at(5),
            },
        ];
        assert_eq!(picked(&mtime, None)[0].0, "b");
        assert!(pick_is_decided(&mtime, &order_family(&mtime, None)));

        // Nothing but the id between them: undecided.
        let twins = members(&[("x", Some(2026)), ("y", Some(2026))]);
        assert!(!pick_is_decided(
            &twins,
            &order_family(&twins, Some("2026-05-01"))
        ));
        let years = members(&[("x", Some(2025)), ("y", Some(2026))]);
        assert!(pick_is_decided(
            &years,
            &order_family(&years, Some("2026-05-01"))
        ));
    }
}
