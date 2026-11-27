//! A lossless, line-level timeclock parser with hledger 1.52's pairing
//! semantics (plan 46 fact 9, re-verified 2026-11-26).
//!
//! - `i DATE TIME ACCOUNT␣␣DESCRIPTION  ; comment` starts a session; the
//!   account ends at two or more spaces. `o DATE TIME [ACCOUNT]  ; comment`
//!   ends one: the named account's, or with no account the most recently
//!   started open one. Times are `HH:MM` or `HH:MM:SS`, kept exactly (T13).
//! - Comments on the `i` line, on indented `;` lines after it, and on the
//!   `o` line are all the session's (transaction) tags.
//! - Two open sessions on different accounts are fine; the same account
//!   twice is an error, as is an `o` with nothing to close or an end before
//!   its start.
//! - A session still open at the end of the file runs to the `now` the
//!   caller passes ([`Session::segments`]), as hledger clocks it to now.
//! - Sessions split at midnight into one piece per day.
//! - `b`, `h` and `O` lines are ignored (hledger 1.52 accepts them with a
//!   date and time; with free text it refuses them, which is reported).
//! - With no description, hledger describes a session by its times
//!   (`08:00-09:00`); [`Session::description_or_times`] does the same.

use super::{Finding, FindingKind, Severity, TimeQty, line_spans, parse_clock, split_two_spaces};
use crate::parse::{normalize_date, parse_tags};
use crate::reports::periods::{add_days, days_from_iso};

/// A date and a time of day.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Moment {
    /// ISO `YYYY-MM-DD`.
    pub date: String,
    /// Seconds after midnight.
    pub seconds: u32,
}

impl Moment {
    /// A moment.
    #[must_use]
    pub fn new(date: impl Into<String>, seconds: u32) -> Self {
        Self {
            date: date.into(),
            seconds,
        }
    }

    /// Seconds since 1970-01-01 00:00.
    #[must_use]
    pub fn epoch_seconds(&self) -> i64 {
        days_from_iso(&self.date) * 86_400 + i64::from(self.seconds)
    }
}

/// An `i` line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClockIn {
    /// When.
    pub at: Moment,
    /// The account.
    pub account: String,
    /// The description, trimmed (may be empty).
    pub description: String,
    /// The comment after `;`, trimmed.
    pub comment: Option<String>,
}

/// An `o` line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClockOut {
    /// When.
    pub at: Moment,
    /// The account named, if any.
    pub account: Option<String>,
    /// The comment after `;`, trimmed.
    pub comment: Option<String>,
}

/// What a line is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClockLineKind {
    /// Empty or whitespace.
    Blank,
    /// A top-level comment (`;`, `#`, `*`).
    Comment,
    /// An indented `;` comment: the preceding session's.
    SessionComment(String),
    /// Clock in.
    In(ClockIn),
    /// Clock out.
    Out(ClockOut),
    /// `b`, `h` or `O`: ignored.
    Ignored,
    /// Refused; the finding says why.
    Invalid,
}

/// One line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClockLine {
    /// 1-based line number.
    pub number: u32,
    /// Byte offset of its first byte.
    pub start: usize,
    /// One past its content.
    pub end: usize,
    /// The next line's start.
    pub next: usize,
    /// What it is.
    pub kind: ClockLineKind,
}

/// A paired (or still open) session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    /// Index of its `i` line.
    pub in_line: usize,
    /// Index of its `o` line, when closed.
    pub out_line: Option<usize>,
    /// The account.
    pub account: String,
    /// The `i` line's description.
    pub description: String,
    /// Start.
    pub start: Moment,
    /// End, when closed.
    pub end: Option<Moment>,
    /// Tags from the `i` line, indented comments, and the `o` line, in that
    /// order.
    pub tags: Vec<(String, String)>,
    /// The comments' text, in the same order (hledger's transaction comment).
    pub comments: Vec<String>,
}

impl Session {
    /// The session cut at each midnight, ending at its `o` or else at `now`:
    /// `(date, duration)` per day it touches, in order. Empty when it is open
    /// and there is no `now`, or `now` is before it started.
    #[must_use]
    pub fn segments(&self, now: Option<&Moment>) -> Vec<(String, TimeQty)> {
        let Some(end) = self.end.as_ref().or(now) else {
            return Vec::new();
        };
        split_at_midnight(&self.start, end)
    }

    /// The whole duration up to its end (or `now`), zero when unknown.
    #[must_use]
    pub fn duration(&self, now: Option<&Moment>) -> TimeQty {
        TimeQty::sum(self.segments(now).into_iter().map(|(_, qty)| qty))
    }

    /// hledger's description: the `i` line's, or `HH:MM-HH:MM`.
    #[must_use]
    pub fn description_or_times(&self, now: Option<&Moment>) -> String {
        if !self.description.is_empty() {
            return self.description.clone();
        }
        let hm = |seconds: u32| format!("{:02}:{:02}", seconds / 3600, seconds / 60 % 60);
        let end = self.end.as_ref().or(now).map_or(0, |end| end.seconds);
        format!("{}-{}", hm(self.start.seconds), hm(end))
    }
}

/// The time from `start` to `end` cut at each midnight: `(date, duration)`
/// per day, in order; empty when `end` is not after `start`.
#[must_use]
pub fn split_at_midnight(start: &Moment, end: &Moment) -> Vec<(String, TimeQty)> {
    let (from, to) = (start.epoch_seconds(), end.epoch_seconds());
    if to <= from {
        return Vec::new();
    }
    let mut pieces = Vec::new();
    let mut date = start.date.clone();
    let mut cursor = from;
    while cursor < to {
        let midnight = (days_from_iso(&date) + 1) * 86_400;
        let stop = midnight.min(to);
        pieces.push((date.clone(), TimeQty::from_seconds(stop - cursor)));
        cursor = stop;
        date = add_days(&date, 1);
    }
    pieces
}

/// A parsed timeclock file.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TimeclockFile {
    /// Every line, in order.
    pub lines: Vec<ClockLine>,
    /// Sessions in the order they started.
    pub sessions: Vec<Session>,
    /// What hledger refuses or reads differently.
    pub findings: Vec<Finding>,
}

impl TimeclockFile {
    /// Whether any finding is an error.
    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.findings
            .iter()
            .any(|finding| finding.severity == Severity::Error)
    }

    /// Sessions still open at the end of the file.
    pub fn open_sessions(&self) -> impl Iterator<Item = &Session> {
        self.sessions.iter().filter(|session| session.end.is_none())
    }
}

/// Parse `text` as timeclock.
#[must_use]
pub fn parse(text: &str) -> TimeclockFile {
    let body = text.strip_prefix('\u{feff}').unwrap_or(text);
    let bom = text.len() - body.len();
    let mut file = TimeclockFile::default();
    // Indices into `file.sessions` of the open ones, oldest first.
    let mut open: Vec<usize> = Vec::new();
    // The session the last clock line belonged to, for indented comments.
    let mut last: Option<usize> = None;
    for (n, (start, end, next)) in line_spans(text).into_iter().enumerate() {
        let number = u32::try_from(n + 1).unwrap_or(u32::MAX);
        let content = &text[(if n == 0 { start + bom } else { start }).min(end)..end];
        let mut problem: Option<(FindingKind, String)> = None;
        let kind = classify(content, &mut problem);
        let index = file.lines.len();
        let kind = match kind {
            ClockLineKind::In(clock_in) => {
                if let Some(&clash) = open
                    .iter()
                    .find(|&&at| file.sessions[at].account == clock_in.account)
                {
                    problem = Some((
                        FindingKind::Pairing,
                        format!(
                            "{} is already clocked in (since {} {}); hledger refuses overlapping \
                             sessions on one account",
                            clock_in.account,
                            file.sessions[clash].start.date,
                            super::format_clock(file.sessions[clash].start.seconds)
                        ),
                    ));
                    ClockLineKind::Invalid
                } else {
                    let (tags, comments) = comment_parts(clock_in.comment.as_deref());
                    file.sessions.push(Session {
                        in_line: index,
                        out_line: None,
                        account: clock_in.account.clone(),
                        description: clock_in.description.clone(),
                        start: clock_in.at.clone(),
                        end: None,
                        tags,
                        comments,
                    });
                    open.push(file.sessions.len() - 1);
                    last = Some(file.sessions.len() - 1);
                    ClockLineKind::In(clock_in)
                }
            }
            ClockLineKind::Out(clock_out) => {
                let found = match &clock_out.account {
                    Some(account) => open
                        .iter()
                        .position(|&at| file.sessions[at].account == *account),
                    None => open.len().checked_sub(1),
                };
                match found {
                    None => {
                        problem = Some((
                            FindingKind::Pairing,
                            clock_out.account.as_ref().map_or_else(
                                || "clock-out with no session open".to_string(),
                                |account| format!("clock-out of {account}, which is not clocked in"),
                            ),
                        ));
                        ClockLineKind::Invalid
                    }
                    Some(slot) => {
                        let at = open[slot];
                        let session = &mut file.sessions[at];
                        if clock_out.at.epoch_seconds() < session.start.epoch_seconds() {
                            problem = Some((
                                FindingKind::Pairing,
                                "clock-out is before its clock-in".to_string(),
                            ));
                            ClockLineKind::Invalid
                        } else {
                            open.remove(slot);
                            session.out_line = Some(index);
                            session.end = Some(clock_out.at.clone());
                            let (tags, comments) = comment_parts(clock_out.comment.as_deref());
                            session.tags.extend(tags);
                            session.comments.extend(comments);
                            last = Some(at);
                            ClockLineKind::Out(clock_out)
                        }
                    }
                }
            }
            ClockLineKind::SessionComment(comment) => {
                if let Some(session) = last.and_then(|at| file.sessions.get_mut(at)) {
                    session.tags.extend(parse_tags(&comment));
                    session.comments.push(comment.clone());
                }
                ClockLineKind::SessionComment(comment)
            }
            other => other,
        };
        if let Some((finding_kind, message)) = problem {
            let severity = if finding_kind == FindingKind::Ignored {
                Severity::Info
            } else {
                Severity::Error
            };
            file.findings
                .push(Finding::new(number, severity, finding_kind, message));
        }
        file.lines.push(ClockLine {
            number,
            start,
            end,
            next,
            kind,
        });
    }
    file
}

fn comment_parts(comment: Option<&str>) -> (Vec<(String, String)>, Vec<String>) {
    comment.map_or_else(
        || (Vec::new(), Vec::new()),
        |comment| (parse_tags(comment), vec![comment.to_string()]),
    )
}

fn split_comment(text: &str) -> (&str, Option<String>) {
    match text.find(';') {
        Some(at) => (
            &text[..at],
            Some(text[at + 1..].trim().to_string()).filter(|c| !c.is_empty()),
        ),
        None => (text, None),
    }
}

fn classify(content: &str, problem: &mut Option<(FindingKind, String)>) -> ClockLineKind {
    let trimmed = content.trim_start();
    if trimmed.is_empty() {
        return ClockLineKind::Blank;
    }
    if content.starts_with([' ', '\t']) {
        if let Some(comment) = trimmed.strip_prefix([';', '#']) {
            return ClockLineKind::SessionComment(comment.trim().to_string());
        }
        *problem = Some((
            FindingKind::Invalid,
            "an indented timeclock line must be a `;` comment".to_string(),
        ));
        return ClockLineKind::Invalid;
    }
    if trimmed.starts_with([';', '#', '*']) {
        return ClockLineKind::Comment;
    }
    let (code, rest) = trimmed.split_at(trimmed.find(' ').unwrap_or(trimmed.len()));
    let refuse = |problem: &mut Option<(FindingKind, String)>, message: String| {
        *problem = Some((FindingKind::Invalid, message));
        ClockLineKind::Invalid
    };
    let (main, comment) = split_comment(rest);
    let mut words = main.trim_start().splitn(3, ' ');
    let date = words.next().and_then(|d| normalize_date(d, None).ok());
    let time = words.next().and_then(parse_clock);
    let tail = words.next().unwrap_or("").trim();
    match (code, date, time) {
        ("i" | "o" | "b" | "h" | "O", Some(date), Some(seconds)) => {
            let at = Moment::new(date, seconds);
            match code {
                "i" => {
                    if tail.is_empty() {
                        return refuse(problem, "a clock-in needs an account".to_string());
                    }
                    let (account, description) = split_two_spaces(tail);
                    ClockLineKind::In(ClockIn {
                        at,
                        account: account.trim().to_string(),
                        description: description.trim().to_string(),
                        comment,
                    })
                }
                "o" => {
                    let (account, _) = split_two_spaces(tail);
                    ClockLineKind::Out(ClockOut {
                        at,
                        account: Some(account.trim().to_string()).filter(|a| !a.is_empty()),
                        comment,
                    })
                }
                _ if tail.is_empty() => {
                    *problem = Some((
                        FindingKind::Ignored,
                        format!("`{code}` lines are ignored"),
                    ));
                    ClockLineKind::Ignored
                }
                _ => {
                    *problem = Some((
                        FindingKind::Ignored,
                        format!(
                            "`{code}` lines are ignored; hledger 1.52 refuses one with text \
                             after the time"
                        ),
                    ));
                    ClockLineKind::Ignored
                }
            }
        }
        ("b" | "h" | "O", _, _) => {
            *problem = Some((
                FindingKind::Ignored,
                format!("`{code}` lines are ignored; hledger 1.52 refuses one without a time"),
            ));
            ClockLineKind::Ignored
        }
        ("i" | "o", None, _) => refuse(problem, "expected a date after `i`/`o`".to_string()),
        ("i" | "o", _, None) => refuse(
            problem,
            "expected a time (HH:MM or HH:MM:SS) after the date".to_string(),
        ),
        _ => refuse(
            problem,
            "a timeclock line starts with i, o, b, h, O or a comment".to_string(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROBE: &str = "; comment\n\
i 2026-09-01 09:00:00 clients:acme:audit  Audit work  ; person: pat\n  ; billable: yes\n\
o 2026-09-01 10:30:15  ; invoice: 101\n\
i 2026-09-02 23:30 clients:globex  late\no 2026-09-03 01:15\n\
i 2026-09-04 08:00 a\ni 2026-09-04 08:30 b\no 2026-09-04 09:00 a\no 2026-09-04 09:30\n";

    #[test]
    fn pairs_like_hledger() {
        let file = parse(PROBE);
        assert!(!file.has_errors(), "{:?}", file.findings);
        assert_eq!(file.sessions.len(), 4);
        let audit = &file.sessions[0];
        assert_eq!(audit.duration(None), TimeQty::from_seconds(5415));
        assert_eq!(audit.description, "Audit work");
        assert_eq!(
            audit.tags,
            vec![
                ("person".into(), "pat".into()),
                ("billable".into(), "yes".into()),
                ("invoice".into(), "101".into())
            ]
        );
        let late = file.sessions[1].segments(None);
        assert_eq!(
            late,
            vec![
                ("2026-09-02".into(), TimeQty::from_minutes(30)),
                ("2026-09-03".into(), TimeQty::from_minutes(75))
            ]
        );
        assert_eq!(file.sessions[2].account, "a");
        assert_eq!(file.sessions[2].end, Some(Moment::new("2026-09-04", 9 * 3600)));
        assert_eq!(file.sessions[3].end, Some(Moment::new("2026-09-04", 34_200)));
        assert_eq!(file.sessions[2].description_or_times(None), "08:00-09:00");
    }

    #[test]
    fn refuses_overlaps_and_runs_open_sessions_to_now() {
        let file = parse("i 2026-09-06 08:00:00 a\ni 2026-09-06 08:30:00 a\n");
        assert!(file.has_errors());
        let open: Vec<_> = file.open_sessions().collect();
        assert_eq!(open.len(), 1);
        let now = Moment::new("2026-09-07", 3600);
        assert_eq!(
            open[0].segments(Some(&now)),
            vec![
                ("2026-09-06".into(), TimeQty::from_minutes(16 * 60)),
                ("2026-09-07".into(), TimeQty::from_minutes(60))
            ]
        );
        assert!(parse("o 2026-09-06 08:00\n").has_errors());
        assert!(parse("i 2026-09-06 08:00 a\no 2026-09-05 08:00\n").has_errors());
    }

    #[test]
    fn ignores_b_h_o_lines() {
        let file = parse("b 2026-09-05 10:00\nh 2026-09-05 10:00:00\nO 2026-09-05 10:00 ; x\nb bla\n");
        assert!(!file.has_errors());
        assert!(file.lines.iter().all(|l| l.kind == ClockLineKind::Ignored));
        assert_eq!(file.findings.len(), 4);
    }
}
