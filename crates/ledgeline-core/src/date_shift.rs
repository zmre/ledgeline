//! Moving a whole journal through time by whole calendar months.
//!
//! The sample ledger is hand-authored with fixed dates, so a month after it was
//! written its newest transactions have already slid out of the default
//! "last 90 days" view. [`shift_into_month_of`] moves every date in it forward
//! (or back) by the number of months that puts the newest transaction in the
//! current month. Whole months, not days: a rent that lands on the 1st still
//! lands on the 1st, a monthly budget still lines up with its months, and a
//! salary on the 27th stays on the 27th. A day past the end of the target month
//! is clamped (`2026-01-31` + 1 month is `2026-02-28`), as hledger's own month
//! arithmetic does.
//!
//! It is a text rewrite, line by line, so everything else in the file (layout,
//! comments, amount styles) comes out byte for byte. What moves:
//!
//! - transaction dates and secondary dates (`2026-01-05=2026-01-07`);
//! - `P` price directives;
//! - the dates inside a `~` periodic rule's period expression, including
//!   year-month bounds (`from 2026-01 to 2026-06`);
//! - `date:` / `date2:` terms in an `=` auto-posting rule's query;
//! - posting dates written as `[2026-01-05]`, `[=2026-01-07]` or both, in any
//!   transaction or posting comment;
//! - full dates in the value of any `name:value` tag (`reviewed:2026-01-05`,
//!   `date:2026-01-05`) in a transaction or posting comment;
//! - the `Y` / `year` / `apply year` default year, by whole years once the
//!   months cross a January. A yearless date (`01-05` under `Y 2026`) is
//!   resolved against the ORIGINAL default year and written back with its year,
//!   because its shifted year need not be the shifted default's.
//!
//! What does not: free comment text (a column-0 `;` line, a `comment` block),
//! `include` paths, account and payee names, amounts, and a year on its own in
//! a period expression (`~ yearly from 2026`), which a month shift cannot
//! express without changing what the rule means. A balance assertion has no
//! date of its own; it holds as of its transaction, which moves with it.
//!
//! Pure and clock-free like the rest of the crate: the caller supplies "today".

use std::collections::BTreeMap;

use crate::parse::is_iso_date;
use crate::reports::periods::{add_months, months_between};

/// The months that put `text`'s newest transaction (by primary date) in the
/// month of `today` (`YYYY-MM-DD`): positive moves forward. `None` when
/// `today` is not a valid ISO date or the journal has no dated transaction.
#[must_use]
pub fn months_until(text: &str, today: &str) -> Option<i64> {
    if !is_iso_date(today) {
        return None;
    }
    let mut scanner = Scanner::default();
    let latest = text
        .lines()
        .filter_map(|line| match scanner.classify(line) {
            Kind::Header => leading_token(line)
                .split('=')
                .next()
                .and_then(|token| DateText::parse(token, scanner.default_year, false))
                .map(|date| date.iso()),
            _ => None,
        })
        .max()?;
    Some(months_between(&latest, today))
}

/// `text` with every date shifted so its newest transaction falls in the month
/// of `today` (`YYYY-MM-DD`). Unchanged when [`months_until`] has no answer.
#[must_use]
pub fn shift_into_month_of(text: &str, today: &str) -> String {
    months_until(text, today).map_or_else(|| text.to_string(), |months| shift_months(text, months))
}

/// `text` with every date it carries (see the module docs for which) moved by
/// `months` calendar months, clamping the day to the target month's length.
#[must_use]
pub fn shift_months(text: &str, months: i64) -> String {
    if months == 0 {
        return text.to_string();
    }
    let mut scanner = Scanner::default();
    text.split_inclusive('\n')
        .map(|raw| {
            let body = raw.trim_end_matches(['\n', '\r']);
            let ending = &raw[body.len()..];
            // The default year in force for THIS line: a `Y` directive is
            // classified (and so read) before it is rewritten.
            let kind = scanner.classify(body);
            let shifter = Shifter {
                months,
                default_year: scanner.default_year,
            };
            let shifted = match kind {
                Kind::Header => shifter.header(body),
                Kind::Price => shifter.price(body),
                Kind::Periodic => shifter.periodic(body),
                Kind::AutoRule => shifter.auto_rule(body),
                Kind::Year { at } => shifter.year_directive(body, at),
                Kind::Indented => shifter.after_comment_mark(body, 0),
                Kind::Other => body.to_string(),
            };
            shifted + ending
        })
        .collect()
}

/// What a journal line is, as far as its dates go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// A transaction's first line: it starts with a date.
    Header,
    /// `P DATE COMMODITY PRICE`.
    Price,
    /// `~ PERIOD  description`.
    Periodic,
    /// `= QUERY`.
    AutoRule,
    /// `Y 2026`, `year 2026`, `apply year 2026`; the year starts at byte `at`.
    Year { at: usize },
    /// A posting, or a comment line belonging to a transaction or rule.
    Indented,
    /// Anything else: column-0 comments, `comment` blocks, other directives.
    Other,
}

/// The line-to-line state the classification needs.
#[derive(Debug, Default)]
struct Scanner {
    /// Inside a `comment` … `end comment` block.
    in_block_comment: bool,
    /// The year a `Y` directive set, as written in the original text.
    default_year: Option<i64>,
}

impl Scanner {
    fn classify(&mut self, line: &str) -> Kind {
        if self.in_block_comment {
            if line.trim_end() == "end comment" {
                self.in_block_comment = false;
            }
            return Kind::Other;
        }
        if line.starts_with([' ', '\t']) {
            return if line.trim().is_empty() {
                Kind::Other
            } else {
                Kind::Indented
            };
        }
        if line.trim_end() == "comment" {
            self.in_block_comment = true;
            return Kind::Other;
        }
        if line.starts_with(|c: char| c.is_ascii_digit()) {
            return Kind::Header;
        }
        if let Some(at) = year_directive_at(line) {
            self.default_year = line[at..].trim().parse().ok();
            return Kind::Year { at };
        }
        match line.split_whitespace().next() {
            Some("P") => Kind::Price,
            Some(word) if word.starts_with('~') => Kind::Periodic,
            Some(word) if word.starts_with('=') => Kind::AutoRule,
            _ => Kind::Other,
        }
    }
}

/// Where the year starts in a `Y 2026` / `year 2026` / `apply year 2026` line.
fn year_directive_at(line: &str) -> Option<usize> {
    ["apply year ", "year ", "Y "]
        .iter()
        .find_map(|keyword| line.strip_prefix(keyword))
        .or_else(|| {
            // hledger also accepts `Y2026`.
            line.strip_prefix('Y')
                .filter(|rest| rest.starts_with(|c: char| c.is_ascii_digit()))
        })
        .filter(|rest| {
            let year = rest.trim();
            !year.is_empty() && year.bytes().all(|byte| byte.is_ascii_digit())
        })
        .map(|rest| line.len() - rest.len())
}

/// The first whitespace- or `;`-delimited token of `line`.
fn leading_token(line: &str) -> &str {
    let end = line
        .find(|c: char| c.is_whitespace() || c == ';')
        .unwrap_or(line.len());
    &line[..end]
}

/// A date as written: its value plus how to write a shifted one back the same
/// way (separator, zero padding).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DateText {
    year: i64,
    month: i64,
    /// `None` for a year-month (`2026-01`), which only a period expression or
    /// a query term may use.
    day: Option<i64>,
    separator: char,
    month_width: usize,
    day_width: usize,
}

impl DateText {
    /// Read `token` as `YYYY-MM-DD`, `MM-DD` (under a `default_year`), or —
    /// with `allow_year_month` — `YYYY-MM`. `-`, `/` and `.` separate, one
    /// kind per date. `None` for anything else, including an impossible date.
    fn parse(token: &str, default_year: Option<i64>, allow_year_month: bool) -> Option<Self> {
        let separator = token.chars().find(|c| matches!(c, '-' | '/' | '.'))?;
        let parts: Vec<&str> = token.split(separator).collect();
        let digits = |part: &str, widths: std::ops::RangeInclusive<usize>| {
            (widths.contains(&part.len()) && part.bytes().all(|byte| byte.is_ascii_digit()))
                .then(|| part.parse::<i64>().ok())
                .flatten()
        };
        let (year, month, day, month_width, day_width) = match parts.as_slice() {
            [year, month, day] => (
                digits(year, 4..=4)?,
                digits(month, 1..=2)?,
                Some(digits(day, 1..=2)?),
                month.len(),
                day.len(),
            ),
            [year, month] if allow_year_month && year.len() == 4 => (
                digits(year, 4..=4)?,
                digits(month, 1..=2)?,
                None,
                month.len(),
                0,
            ),
            [month, day] => (
                default_year?,
                digits(month, 1..=2)?,
                Some(digits(day, 1..=2)?),
                month.len(),
                day.len(),
            ),
            _ => return None,
        };
        let date = Self {
            year,
            month,
            day,
            separator,
            month_width,
            day_width,
        };
        is_iso_date(&date.iso()).then_some(date)
    }

    /// `YYYY-MM-DD`; a year-month reads as its first day.
    fn iso(&self) -> String {
        format!(
            "{:04}-{:02}-{:02}",
            self.year,
            self.month,
            self.day.unwrap_or(1)
        )
    }

    /// This date `months` later, written the way it was written (always with
    /// its year: a yearless date's shifted year need not be the default's).
    fn shifted(&self, months: i64) -> String {
        let moved = add_months(&self.iso(), months);
        let (year, month, day) = (&moved[..4], &moved[5..7], &moved[8..10]);
        let pad = |text: &str, width: usize| {
            if width >= 2 {
                text.to_string()
            } else {
                text.trim_start_matches('0').to_string()
            }
        };
        let sep = self.separator;
        let month = pad(month, self.month_width);
        match self.day {
            Some(_) => format!("{year}{sep}{month}{sep}{}", pad(day, self.day_width)),
            None => format!("{year}{sep}{month}"),
        }
    }
}

/// Byte ranges of `text` to replace, keyed by start so overlapping finds
/// (a bracketed date inside a tag value) are applied once.
type Edits = BTreeMap<usize, (usize, String)>;

fn apply(text: &str, edits: &Edits) -> String {
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for (&start, (end, replacement)) in edits {
        if start < at {
            continue;
        }
        out.push_str(&text[at..start]);
        out.push_str(replacement);
        at = *end;
    }
    out.push_str(&text[at..]);
    out
}

/// One shift, applied to each kind of line.
#[derive(Debug, Clone, Copy)]
struct Shifter {
    months: i64,
    default_year: Option<i64>,
}

impl Shifter {
    /// `token` shifted, or `None` when it is not a date.
    fn token(&self, token: &str, allow_year_month: bool) -> Option<String> {
        DateText::parse(token, self.default_year, allow_year_month)
            .map(|date| date.shifted(self.months))
    }

    /// `DATE[=DATE2] …` then the transaction comment.
    fn header(&self, line: &str) -> String {
        let token = leading_token(line);
        let dates = token
            .split('=')
            .map(|part| self.token(part, false).unwrap_or_else(|| part.to_string()))
            .collect::<Vec<_>>()
            .join("=");
        dates + &self.after_comment_mark(&line[token.len()..], 0)
    }

    /// `P DATE …`.
    fn price(&self, line: &str) -> String {
        let start = line.find('P').map_or(0, |at| at + 1);
        let at = start + (line[start..].len() - line[start..].trim_start().len());
        let token = leading_token(&line[at..]);
        match self.token(token, false) {
            Some(shifted) => format!("{}{shifted}{}", &line[..at], &line[at + token.len()..]),
            None => line.to_string(),
        }
    }

    /// `~ PERIOD  description  ; comment`: the period expression runs to the
    /// first double space or tab.
    fn periodic(&self, line: &str) -> String {
        let end = [line.find("  "), line.find('\t'), line.find(';')]
            .into_iter()
            .flatten()
            .min()
            .unwrap_or(line.len());
        let mut edits = Edits::new();
        self.dates_in(line, 0..end, true, &mut edits);
        let head = apply(&line[..end], &edits);
        head + &self.after_comment_mark(&line[end..], 0)
    }

    /// `= QUERY`: only `date:` / `date2:` terms carry dates.
    fn auto_rule(&self, line: &str) -> String {
        let end = line.find(';').unwrap_or(line.len());
        let mut edits = Edits::new();
        let mut at = 0;
        for word in line[..end].split(' ') {
            let term = word.trim_start_matches("not:");
            let offset = word.len() - term.len();
            if let Some(value) = term
                .strip_prefix("date:")
                .or_else(|| term.strip_prefix("date2:"))
            {
                let from = at + offset + (term.len() - value.len());
                self.dates_in(line, from..from + value.len(), true, &mut edits);
            }
            at += word.len() + 1;
        }
        apply(&line[..end], &edits) + &self.after_comment_mark(&line[end..], 0)
    }

    /// `Y 2026` → the year of 2026-01-01 shifted.
    fn year_directive(&self, line: &str, at: usize) -> String {
        let value = line[at..].trim();
        let shifted = value
            .parse::<i64>()
            .ok()
            .map(|year| add_months(&format!("{year:04}-01-01"), self.months)[..4].to_string());
        match shifted {
            Some(year) => format!("{}{year}{}", &line[..at], &line[at + value.len()..]),
            None => line.to_string(),
        }
    }

    /// `line` with the comment that starts at its first `;` at or after
    /// `from` shifted: bracketed posting dates and dates in tag values.
    fn after_comment_mark(&self, line: &str, from: usize) -> String {
        match line[from..].find(';') {
            Some(semi) => {
                let start = from + semi + 1;
                format!("{}{}", &line[..start], self.comment(&line[start..]))
            }
            None => line.to_string(),
        }
    }

    /// A comment body: `[DATE]`, `[=DATE2]`, `[DATE=DATE2]`, and full dates in
    /// `name:value` tag values (hledger's tag rule: the name is the last word
    /// before a `:`, the value runs to the next comma).
    fn comment(&self, text: &str) -> String {
        let mut edits = Edits::new();
        // Bracketed posting dates.
        let mut from = 0;
        while let Some(open) = text[from..].find('[').map(|at| from + at) {
            let Some(close) = text[open..].find(']').map(|at| open + at) else {
                break;
            };
            let mut at = open + 1;
            for part in text[open + 1..close].split('=') {
                if let Some(shifted) = self.token(part, false) {
                    edits.insert(at, (at + part.len(), shifted));
                }
                at += part.len() + 1;
            }
            from = close + 1;
        }
        // Tag values.
        let mut at = 0;
        for segment in text.split(',') {
            if let Some(colon) = segment.find(':')
                && segment[..colon].split_whitespace().next_back().is_some()
            {
                let value = at + colon + 1;
                self.dates_in(text, value..at + segment.len(), false, &mut edits);
            }
            at += segment.len() + 1;
        }
        apply(text, &edits)
    }

    /// Record an edit for every date in `text[range]` that stands on its own
    /// (no digit or letter touching it on either side).
    fn dates_in(
        &self,
        text: &str,
        range: std::ops::Range<usize>,
        allow_year_month: bool,
        edits: &mut Edits,
    ) {
        let bytes = text.as_bytes();
        let boundary = |at: usize| at >= range.end || !bytes[at].is_ascii_alphanumeric();
        let mut at = range.start;
        while at < range.end {
            let starts_here = bytes[at].is_ascii_digit()
                && (at == range.start || !bytes[at - 1].is_ascii_alphanumeric());
            if !starts_here {
                at += 1;
                continue;
            }
            // The longest run of digits and separators from here.
            let end = (at..range.end)
                .find(|&i| !(bytes[i].is_ascii_digit() || matches!(bytes[i], b'-' | b'/' | b'.')))
                .unwrap_or(range.end);
            let candidate = longest_date(&text[at..end], allow_year_month)
                .filter(|&len| boundary(at + len))
                .and_then(|len| {
                    self.token(&text[at..at + len], allow_year_month)
                        .map(|shifted| (len, shifted))
                });
            match candidate {
                Some((len, shifted)) => {
                    edits.insert(at, (at + len, shifted));
                    at += len;
                }
                None => at = end.max(at + 1),
            }
        }
    }
}

/// The byte length of the date `run` starts with (`YYYY-MM-DD`, or with
/// `allow_year_month` `YYYY-MM`), one separator kind throughout; `None` if it
/// starts with neither. `2026-01-05..2026-02` yields the first date only.
fn longest_date(run: &str, allow_year_month: bool) -> Option<usize> {
    let bytes = run.as_bytes();
    if bytes.len() < 6 || !bytes[..4].iter().all(u8::is_ascii_digit) {
        return None;
    }
    let separator = bytes[4];
    if !matches!(separator, b'-' | b'/' | b'.') {
        return None;
    }
    let digits_from = |from: usize| {
        (from..bytes.len().min(from + 2))
            .take_while(|&i| bytes[i].is_ascii_digit())
            .count()
    };
    let month = digits_from(5);
    if month == 0 {
        return None;
    }
    let after_month = 5 + month;
    let day = if bytes.get(after_month) == Some(&separator) {
        digits_from(after_month + 1)
    } else {
        0
    };
    let digit_after = |at: usize| bytes.get(at).is_some_and(u8::is_ascii_digit);
    if day > 0 && !digit_after(after_month + 1 + day) {
        Some(after_month + 1 + day)
    } else if allow_year_month
        && separator != b'.'
        && !digit_after(after_month)
        && bytes.get(after_month) != Some(&separator)
    {
        Some(after_month)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse_journal;

    const SAMPLE: &str = include_str!("../../../fixtures/sample.journal");

    #[test]
    fn transaction_dates_move_by_whole_months_and_clamp_the_day() {
        let text = "2026-01-31 * Rent\n    expenses:rent  $10\n    assets:cash\n";
        assert_eq!(
            shift_months(text, 1),
            "2026-02-28 * Rent\n    expenses:rent  $10\n    assets:cash\n"
        );
        assert_eq!(shift_months("2024-01-31 x\n", 1), "2024-02-29 x\n");
        assert_eq!(shift_months("2026-11-15 x\n", 2), "2027-01-15 x\n");
        assert_eq!(shift_months("2026-03-31 x\n", -1), "2026-02-28 x\n");
    }

    #[test]
    fn secondary_dates_separators_and_padding_are_kept() {
        assert_eq!(
            shift_months("2026-01-15=2026-01-20 x\n", 1),
            "2026-02-15=2026-02-20 x\n"
        );
        assert_eq!(shift_months("2026/1/5 x\n", 1), "2026/2/5 x\n");
        assert_eq!(shift_months("2026.09.05 x\n", 1), "2026.10.05 x\n");
        assert_eq!(shift_months("2026/9/5 x\n", 3), "2026/12/5 x\n");
        assert_eq!(shift_months("2026-01-05;note\n", 1), "2026-02-05;note\n");
    }

    #[test]
    fn yearless_dates_resolve_against_the_original_default_year() {
        let text = "Y 2026\n\n12/15 * gift\n    a  $1\n    b\n";
        assert_eq!(
            shift_months(text, 1),
            "Y 2026\n\n2027/01/15 * gift\n    a  $1\n    b\n"
        );
        assert_eq!(shift_months("year 2026\n", 4), "year 2026\n");
        assert_eq!(shift_months("apply year 2026\n", -1), "apply year 2025\n");
        assert_eq!(shift_months("Y2026\n", 12), "Y2027\n");
    }

    #[test]
    fn price_directives_move() {
        assert_eq!(
            shift_months("P 2026-06-30 AAPL $270.25\n", 4),
            "P 2026-10-30 AAPL $270.25\n"
        );
        assert_eq!(
            shift_months("P 2026-06-30 12:00:00 EUR $1.16\n", 1),
            "P 2026-07-30 12:00:00 EUR $1.16\n"
        );
    }

    #[test]
    fn periodic_rule_bounds_move_but_a_bare_year_does_not() {
        assert_eq!(
            shift_months("~ monthly from 2026-01 to 2026-06-30  budget  ; note\n", 2),
            "~ monthly from 2026-03 to 2026-08-30  budget  ; note\n"
        );
        assert_eq!(
            shift_months("~ 2026-01..2026-07\n", 1),
            "~ 2026-02..2026-08\n"
        );
        assert_eq!(
            shift_months("~ yearly from 2026\n", 4),
            "~ yearly from 2026\n"
        );
        assert_eq!(
            shift_months("~ every 3rd tuesday of month\n", 4),
            "~ every 3rd tuesday of month\n"
        );
    }

    #[test]
    fn auto_rule_date_terms_move() {
        assert_eq!(
            shift_months(
                "= expenses:food date:2026-01..2026-03-15 not:date2:2025/12\n",
                1
            ),
            "= expenses:food date:2026-02..2026-04-15 not:date2:2026/01\n"
        );
        assert_eq!(
            shift_months("= desc:2026-01-01\n", 1),
            "= desc:2026-01-01\n"
        );
    }

    #[test]
    fn dates_in_tag_values_and_brackets_move() {
        let text = "2026-01-05 x  ; reviewed:2026-01-06, trip: munich\n    \
                    ; date: 2026-01-07\n    a  $1  ; [2026-01-08=2026-01-09] receipt: r-2026-01\n    \
                    b  ; [=1/10]\n";
        let expected = "2026-02-05 x  ; reviewed:2026-02-06, trip: munich\n    \
                        ; date: 2026-02-07\n    a  $1  ; [2026-02-08=2026-02-09] receipt: r-2026-01\n    \
                        b  ; [=1/10]\n";
        assert_eq!(shift_months(text, 1), expected);
        // A yearless bracket date needs a default year to be read.
        assert_eq!(
            shift_months("Y 2026\n2026-01-05 x\n    b  ; [=1/10]\n", 1),
            "Y 2026\n2026-02-05 x\n    b  ; [=2026/2/10]\n"
        );
    }

    #[test]
    fn prose_amounts_includes_and_block_comments_stay_put() {
        let text = "; written 2026-01-05\n# also 2026-01-05\ninclude 2026-01.journal\n\
                    comment\n2026-01-05 not a transaction\nend comment\n\
                    2026-01-05 Paid 2026-01-04 invoice\n    a  $2024.07\n    b  1.000,00 EUR\n";
        let expected = "; written 2026-01-05\n# also 2026-01-05\ninclude 2026-01.journal\n\
                        comment\n2026-01-05 not a transaction\nend comment\n\
                        2026-02-05 Paid 2026-01-04 invoice\n    a  $2024.07\n    b  1.000,00 EUR\n";
        assert_eq!(shift_months(text, 1), expected);
    }

    #[test]
    fn line_endings_are_preserved() {
        assert_eq!(
            shift_months("2026-01-05 x\r\n    a  $1\r\n", 1),
            "2026-02-05 x\r\n    a  $1\r\n"
        );
        assert_eq!(shift_months("2026-01-05 x", 1), "2026-02-05 x");
    }

    #[test]
    fn months_until_lands_the_newest_transaction_in_todays_month() {
        let text = "P 2027-01-01 X $1\n2026-03-10 a\n2026-07-03=2026-12-01 b\n2026-05-01 c\n";
        assert_eq!(months_until(text, "2026-11-13"), Some(4));
        assert_eq!(months_until(text, "2026-07-31"), Some(0));
        assert_eq!(months_until(text, "2026-02-01"), Some(-5));
        assert_eq!(months_until(text, "2026-13-01"), None);
        assert_eq!(months_until("; nothing\n", "2026-11-13"), None);
    }

    #[test]
    fn shift_into_month_of_is_unchanged_without_an_answer() {
        assert_eq!(
            shift_into_month_of("2026-07-03 x\n", "nope"),
            "2026-07-03 x\n"
        );
        assert_eq!(
            shift_into_month_of("2026-07-03 x\n", "2026-09-30"),
            "2026-09-03 x\n"
        );
    }

    #[test]
    fn the_sample_still_parses_with_every_date_moved_alike() {
        let today = "2026-11-13";
        let shifted = shift_into_month_of(SAMPLE, today);
        let before = parse_journal(SAMPLE, "sample.journal").expect("the sample parses");
        let after = parse_journal(&shifted, "sample.journal").expect("the shifted sample parses");
        let months = months_until(SAMPLE, today).expect("the sample has transactions");

        assert_eq!(before.transactions.len(), after.transactions.len());
        for (old, new) in before.transactions.iter().zip(&after.transactions) {
            assert_eq!(
                add_months(&old.date, months),
                new.date,
                "{}",
                old.description
            );
            assert_eq!(old.date2.as_ref().map(|d| add_months(d, months)), new.date2);
            assert_eq!(old.postings.len(), new.postings.len());
            for (p_old, p_new) in old.postings.iter().zip(&new.postings) {
                assert_eq!(p_old.account, p_new.account);
                assert_eq!(p_old.amounts, p_new.amounts);
            }
        }
        assert_eq!(before.prices.len(), after.prices.len());
        for (old, new) in before.prices.iter().zip(&after.prices) {
            assert_eq!(add_months(&old.date, months), new.date);
            assert_eq!(old.price, new.price);
        }
        let newest = after.transactions.iter().map(|txn| txn.date.as_str()).max();
        assert!(
            newest.is_some_and(|date| date.starts_with("2026-11")),
            "{newest:?}"
        );
        // Every line that is not a dated directive is untouched.
        let dated =
            |line: &&str| line.starts_with(|c: char| c.is_ascii_digit()) || line.starts_with("P ");
        let undated = |text: &str| {
            text.lines()
                .filter(|line| !dated(line))
                .map(str::to_string)
                .collect::<Vec<_>>()
        };
        assert_eq!(undated(SAMPLE), undated(&shifted));
    }

    #[test]
    fn the_sample_shifted_into_any_month_parses() {
        for today in [
            "2026-07-15",
            "2027-01-31",
            "2027-02-28",
            "2028-02-29",
            "2026-03-31",
            "2030-12-01",
        ] {
            let shifted = shift_into_month_of(SAMPLE, today);
            let journal = parse_journal(&shifted, "sample.journal")
                .unwrap_or_else(|error| panic!("{today}: {error}"));
            let newest = journal
                .transactions
                .iter()
                .map(|txn| txn.date.clone())
                .max()
                .unwrap_or_default();
            assert_eq!(&newest[..7], &today[..7]);
        }
    }
}
