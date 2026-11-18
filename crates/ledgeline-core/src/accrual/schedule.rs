//! Schedules: periods, dates, amounts and the `~` rules that state them
//! (plan 45 D-sched, D-anchor). Pure: no I/O, no clock.
//!
//! A [`ScheduleSpec`] is one or more deferred/prepaid lines from one source
//! transaction that share a start, an end and a proration. [`build`] turns it
//! into a [`Schedule`]: one [`Period`] per month of service, each dated the day
//! its service period begins (Q12), and the fewest rules that generate exactly
//! those periods under hledger's grammar:
//!
//! | Rule  | When | Form |
//! |-------|------|------|
//! | Opening stub | calendar proration, first month partial | `~ START` |
//! | Main | two or more equal periods | `~ monthly from FIRST to LAST+1`, or for an anniversary day 29–31 `~ every Nth day of month from FIRST-OF-MONTH to LAST+1` |
//! | Final | the last period's amount differs | `~ DATE`, with the main rule's `to` set to `DATE` |
//!
//! [`periods_of`] reads rules back through the budget report's own occurrence
//! walk (`reports::budget::occurrences`), which is pinned to hledger, so a
//! schedule's rules are checked against what hledger will generate rather than
//! against this module's idea of it.
//!
//! # Amounts
//!
//! Exact [`Dec`], rounded half away from zero to the commodity's display
//! precision. Each period carries a weight — 1 for a whole month, days served
//! over days in the period for a partial one — and the per-month amount is
//! `m = round(T / Σ weights)`. A partial period gets `round(m · weight)`, a
//! whole one `m`, and the **final** period gets the remainder, so every line
//! sums to its total exactly (D3). With whole months only this is the plan's
//! anniversary rule: `round(T / n)` for periods 1…n−1 and the true-up last.

use super::{RECOGNITION_TAG, RENEWAL_DATE_TAG, day_of, is_iso_date, iso_of};
use crate::decimal::Dec;
use crate::edit::render_amount;
use crate::model::{Amount, AmountStyle, Commodity, PeriodKind, PeriodSpec, PeriodicTransaction};
use crate::parse::JournalStyles;
use crate::periodic::check_account;
use crate::reports::budget::occurrences;
use crate::reports::periods::{
    add_days, add_months, civil_from_days, days_from_civil, days_in_month,
};
use crate::text::has_ascii_control;
use std::collections::BTreeMap;
use thiserror::Error;

/// The longest term a schedule may have, in months (100 years).
pub const MAX_TERM_MONTHS: u32 = 1200;
/// The most lines one schedule may hold.
pub const MAX_LINES: usize = 16;
/// The longest description, in bytes.
const MAX_DESCRIPTION_BYTES: usize = 256;
/// The longest tag value, in bytes.
const MAX_TAG_VALUE_BYTES: usize = 256;

/// Whose line it is: the plan 37 attribution tag a line carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PartyKind {
    /// `customer:` — revenue lines.
    Customer,
    /// `vendor:` — expense lines.
    Vendor,
}

impl PartyKind {
    /// The tag name.
    #[must_use]
    pub fn tag(self) -> &'static str {
        match self {
            Self::Customer => super::CUSTOMER_TAG,
            Self::Vendor => super::VENDOR_TAG,
        }
    }
}

/// A customer or vendor.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Party {
    /// Customer or vendor.
    pub kind: PartyKind,
    /// The name, as the tag value.
    pub name: String,
}

/// How a schedule's months are cut (D-anchor).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Proration {
    /// Periods run from the start date's monthly anniversary.
    Anniversary,
    /// Calendar months, with day-prorated first and last months
    /// (`proration: calendar`).
    Calendar,
}

/// How long service runs.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Term {
    /// Whole months from the start (`term-months`).
    Months(u32),
    /// Through this date, inclusive (`service-end`).
    Through(String),
}

/// One deferred or prepaid line of a schedule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleLine {
    /// The deferred-revenue or prepaid account the source posting hits.
    pub source_account: String,
    /// The revenue or expense account each period recognizes into.
    pub target_account: String,
    /// The source posting's amount, signed as it is written there: negative
    /// for a deferred-revenue credit, positive for a prepaid debit. Every
    /// period's amount carries the same sign.
    pub total: Amount,
    /// The customer or vendor tag each posting carries, if any.
    pub party: Option<Party>,
}

/// Everything a schedule is built from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleSpec {
    /// The `recognition:` id.
    pub id: String,
    /// The lines, in source-posting order.
    pub lines: Vec<ScheduleLine>,
    /// First day of service.
    pub start: String,
    /// Length of service.
    pub term: Term,
    /// Anniversary or calendar periods.
    pub proration: Proration,
    /// Each rule's (and each entry's) description.
    pub description: String,
    /// Extra header tags for every rule, after `recognition` (and
    /// `renewal-date` on the main rule).
    pub header_tags: Vec<(String, String)>,
}

/// What a period is within its schedule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PeriodPart {
    /// A calendar schedule's partial first month, dated the service start.
    Opening,
    /// An ordinary period.
    Regular,
    /// The last period, which takes any rounding remainder.
    Closing,
}

/// One period's recognition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Period {
    /// The day its service period begins: what the entry is dated and what
    /// `recognized:` holds.
    pub date: String,
    /// Per line, in [`ScheduleSpec::lines`] order, signed like the line's
    /// total.
    pub amounts: Vec<Dec>,
    /// Opening stub, regular, or the closing true-up.
    pub part: PeriodPart,
}

/// Which rule of a schedule a [`RuleText`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RuleRole {
    /// A calendar schedule's opening stub, `~ START`.
    Opening,
    /// The run of equal periods (or the single period of a one-period
    /// schedule).
    Main,
    /// The final period, when its amount differs.
    Final,
}

/// One `~` rule, rendered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleText {
    /// Which rule.
    pub role: RuleRole,
    /// The period expression, as written after `~`.
    pub period: String,
    /// The whole block: header and postings, each line ending in `\n`.
    pub text: String,
}

/// A built schedule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Schedule {
    /// The `recognition:` id.
    pub id: String,
    /// Every period, in date order.
    pub periods: Vec<Period>,
    /// The rules, in file order: opening, main, final.
    pub rules: Vec<RuleText>,
    /// Service start + term: the day after service ends.
    pub renewal_date: String,
    /// The spec it was built from.
    pub spec: ScheduleSpec,
}

impl Schedule {
    /// The rules as one block group, separated by nothing — what the
    /// recognition file holds for this schedule after its heading comment.
    #[must_use]
    pub fn rules_text(&self) -> String {
        self.rules.iter().map(|rule| rule.text.as_str()).collect()
    }

    /// The period dated `date`, if any.
    #[must_use]
    pub fn period_on(&self, date: &str) -> Option<&Period> {
        self.periods.iter().find(|period| period.date == date)
    }

    /// Each line's total.
    #[must_use]
    pub fn totals(&self) -> Vec<Dec> {
        self.spec
            .lines
            .iter()
            .map(|line| line.total.quantity)
            .collect()
    }
}

/// Why a schedule can't be built.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ScheduleError {
    /// An input that would not read back as written, with the reason.
    #[error("{0}")]
    Invalid(String),
    /// Decimal overflow.
    #[error("the amounts are too large to divide exactly")]
    Overflow,
    /// A rule this module can't enumerate (no `from`, an open-ended
    /// recurrence, or a period hledger accepts but this engine doesn't).
    #[error("the rule `~ {0}` has no bounded set of dates")]
    Unbounded(String),
}

impl From<crate::decimal::DecError> for ScheduleError {
    fn from(_: crate::decimal::DecError) -> Self {
        Self::Overflow
    }
}

fn invalid<T>(why: impl Into<String>) -> Result<T, ScheduleError> {
    Err(ScheduleError::Invalid(why.into()))
}

/// Whether `id` is usable as a `recognition:` value: 1–32 ASCII letters,
/// digits, `-` or `_`, and not `none`.
#[must_use]
pub fn is_valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 32
        && id != super::NONE_VALUE
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Reject a tag value that would not read back as itself: a comma or a
/// newline ends a tag value, and surrounding space is trimmed by hledger.
pub(crate) fn check_tag_value(what: &str, value: &str) -> Result<(), ScheduleError> {
    if value.is_empty()
        || value.len() > MAX_TAG_VALUE_BYTES
        || value.trim() != value
        || value.contains(',')
        || has_ascii_control(value)
    {
        return invalid(format!(
            "{what} `{value}` can't be written as a tag value: it must be 1–{MAX_TAG_VALUE_BYTES} \
             bytes with no comma, control character or surrounding space"
        ));
    }
    Ok(())
}

/// Reject a tag name hledger would not read as one.
pub(crate) fn check_tag_name(name: &str) -> Result<(), ScheduleError> {
    let ok = !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_');
    if ok {
        Ok(())
    } else {
        invalid(format!("`{name}` is not a usable tag name"))
    }
}

/// Reject a description that would not read back as written.
pub(crate) fn check_description(description: &str) -> Result<(), ScheduleError> {
    if description.len() > MAX_DESCRIPTION_BYTES
        || has_ascii_control(description)
        || description.trim() != description
        || description.contains([';', '#'])
        || description.contains("  ")
    {
        return invalid(format!(
            "the description `{description}` can't be written: no `;`, `#`, control \
             characters, double spaces or surrounding space, at most {MAX_DESCRIPTION_BYTES} bytes"
        ));
    }
    Ok(())
}

fn check_spec(spec: &ScheduleSpec) -> Result<(), ScheduleError> {
    if !is_valid_id(&spec.id) {
        return invalid(format!("`{}` is not a usable recognition id", spec.id));
    }
    if spec.lines.is_empty() || spec.lines.len() > MAX_LINES {
        return invalid(format!("a schedule needs 1 to {MAX_LINES} lines"));
    }
    if !is_iso_date(&spec.start) {
        return invalid(format!("`{}` is not a date", spec.start));
    }
    match &spec.term {
        Term::Months(n) if (1..=MAX_TERM_MONTHS).contains(n) => {}
        Term::Months(_) => return invalid(format!("a term is 1 to {MAX_TERM_MONTHS} months")),
        Term::Through(end) if is_iso_date(end) && end.as_str() >= spec.start.as_str() => {}
        Term::Through(end) => {
            return invalid(format!(
                "the service end `{end}` must be a date on or after the start"
            ));
        }
    }
    check_description(&spec.description)?;
    for (name, value) in &spec.header_tags {
        check_tag_name(name)?;
        check_tag_value(name, value)?;
        if [RECOGNITION_TAG, RENEWAL_DATE_TAG, super::RECOGNIZED_TAG].contains(&name.as_str()) {
            return invalid(format!("`{name}` is written by the schedule itself"));
        }
    }
    for line in &spec.lines {
        for account in [&line.source_account, &line.target_account] {
            check_account(account).map_err(|error| ScheduleError::Invalid(error.to_string()))?;
        }
        if line.source_account == line.target_account {
            return invalid("a line must recognize into a different account");
        }
        if line.total.cost.is_some() {
            return invalid(format!(
                "the line on {} has an `@` cost; schedules recognize in one commodity at cost, \
                 so write it in the cost commodity",
                line.source_account
            ));
        }
        if line.total.quantity.is_zero() {
            return invalid(format!("the line on {} is zero", line.source_account));
        }
        if line.total.commodity.0.contains([';', '#']) || has_ascii_control(&line.total.commodity.0)
        {
            return invalid("a commodity may not contain `;`, `#` or control characters");
        }
        if let Some(party) = &line.party {
            check_tag_value(party.kind.tag(), &party.name)?;
        }
    }
    Ok(())
}

/// The exclusive end of service.
#[must_use]
pub fn end_of(start: &str, term: &Term) -> String {
    match term {
        Term::Months(n) => add_months(start, i64::from(*n)),
        Term::Through(last) => add_days(last, 1),
    }
}

/// One period before amounts: its date and its weight `num / den`.
#[derive(Debug, Clone, Copy)]
struct Slot {
    day: i32,
    num: i64,
    den: i64,
    opening: bool,
}

fn ymd(day: i32) -> (i64, i64, i64) {
    civil_from_days(i64::from(day))
}

/// The day number of the 1st of `month` (normalized into its year) in `year`.
fn first_of_month(year: i64, month: i64) -> i32 {
    let index = year * 12 + (month - 1);
    i32::try_from(days_from_civil(
        index.div_euclid(12),
        index.rem_euclid(12) + 1,
        1,
    ))
    .unwrap_or(i32::MAX)
}

/// The periods of `[start, end)` under `proration`.
fn slots(start: &str, end: &str, proration: Proration) -> Vec<Slot> {
    let (s, e) = (day_of(start), day_of(end));
    match proration {
        Proration::Anniversary => (0_i64..)
            .map(|k| {
                (
                    day_of(&add_months(start, k)),
                    day_of(&add_months(start, k + 1)),
                )
            })
            .take_while(|(from, _)| *from < e)
            .map(|(from, to)| Slot {
                day: from,
                num: i64::from(to.min(e) - from),
                den: i64::from(to - from),
                opening: false,
            })
            .collect(),
        Proration::Calendar => {
            let (sy, sm, _) = ymd(s);
            (0_i64..)
                .map(|k| (first_of_month(sy, sm + k), first_of_month(sy, sm + k + 1)))
                .take_while(|(month, _)| *month < e)
                .map(|(month, next)| {
                    let from = month.max(s);
                    Slot {
                        day: from,
                        num: i64::from(next.min(e) - from),
                        den: i64::from(next - month),
                        opening: from != month,
                    }
                })
                .collect()
        }
    }
}

fn gcd(a: i64, b: i64) -> i64 {
    if b == 0 { a.abs() } else { gcd(b, a % b) }
}

/// `num / den`, rounded half away from zero.
fn div_round(num: i128, den: i128) -> Option<i128> {
    if den == 0 {
        return None;
    }
    let (q, r) = (num / den, num % den);
    let away = r.unsigned_abs() * 2 >= den.unsigned_abs();
    let sign = if (num < 0) == (den < 0) { 1 } else { -1 };
    Some(if away && r != 0 { q + sign } else { q })
}

/// Per-period amounts of one line: weights to amounts, the remainder last.
fn line_amounts(total: Dec, precision: u32, slots: &[Slot]) -> Result<Vec<Dec>, ScheduleError> {
    let count = slots.len();
    if count == 1 {
        return Ok(vec![total]);
    }
    // Σ weights as one fraction n/d.
    let d = slots
        .iter()
        .filter(|slot| slot.num != slot.den)
        .fold(1_i64, |d, slot| d / gcd(d, slot.den) * slot.den);
    let n: i64 = slots
        .iter()
        .map(|slot| {
            if slot.num == slot.den {
                d
            } else {
                slot.num * (d / slot.den)
            }
        })
        .sum();
    // m = round(T · d / n) at `precision`.
    let (t, q) = (total.mantissa, total.places);
    let up = |places: u32| 10_i128.checked_pow(places).ok_or(ScheduleError::Overflow);
    let numerator = t
        .checked_mul(i128::from(d))
        .and_then(|x| x.checked_mul(up(precision.saturating_sub(q)).ok()?))
        .ok_or(ScheduleError::Overflow)?;
    let denominator = i128::from(n)
        .checked_mul(up(q.saturating_sub(precision))?)
        .ok_or(ScheduleError::Overflow)?;
    let m = div_round(numerator, denominator).ok_or(ScheduleError::Overflow)?;
    let mut amounts: Vec<Dec> = slots[..count - 1]
        .iter()
        .map(|slot| {
            let value = if slot.num == slot.den {
                Some(m)
            } else {
                m.checked_mul(i128::from(slot.num))
                    .and_then(|x| div_round(x, i128::from(slot.den)))
            };
            value
                .map(|mantissa| Dec::new(mantissa, precision))
                .ok_or(ScheduleError::Overflow)
        })
        .collect::<Result<_, _>>()?;
    let spent = amounts
        .iter()
        .try_fold(Dec::zero(), |sum, amount| sum.add(*amount))?;
    amounts.push(total.sub(spent)?);
    Ok(amounts)
}

/// The display precision a line's amounts are rounded to: the commodity's
/// declared (or first-seen) style, else the total's own.
fn precision_of(line: &ScheduleLine, styles: &JournalStyles<'_>) -> u32 {
    styles
        .declared_style(&line.total.commodity)
        .map_or(line.total.style.precision, |style| style.precision)
}

/// The style an amount of `commodity` is written in.
fn style_of(line: &ScheduleLine, styles: &JournalStyles<'_>) -> AmountStyle {
    styles
        .declared_style(&line.total.commodity)
        .cloned()
        .unwrap_or_else(|| line.total.style.clone())
}

/// `quantity` of `line`'s commodity, ready to render.
pub(crate) fn amount_in(commodity: &Commodity, style: &AmountStyle, quantity: Dec) -> Amount {
    Amount {
        commodity: commodity.clone(),
        quantity,
        style: AmountStyle {
            precision: quantity.places,
            ..style.clone()
        },
        cost: None,
    }
}

/// Build the schedule `spec` describes, with its rules rendered in `styles`
/// (the main journal's: [`JournalStyles::of`]).
///
/// # Errors
/// [`ScheduleError::Invalid`] for an input that would not read back as
/// written (an unusable id, account, description or tag value; a cost; a zero
/// line; a term out of range); [`ScheduleError::Overflow`] when the arithmetic
/// overflows.
pub fn build(spec: &ScheduleSpec, styles: &JournalStyles<'_>) -> Result<Schedule, ScheduleError> {
    check_spec(spec)?;
    let end = end_of(&spec.start, &spec.term);
    let slots = slots(&spec.start, &end, spec.proration);
    let per_line: Vec<Vec<Dec>> = spec
        .lines
        .iter()
        .map(|line| line_amounts(line.total.quantity, precision_of(line, styles), &slots))
        .collect::<Result<_, _>>()?;
    let last = slots.len() - 1;
    let periods: Vec<Period> = slots
        .iter()
        .enumerate()
        .map(|(k, slot)| Period {
            date: iso_of(slot.day),
            amounts: per_line.iter().map(|amounts| amounts[k]).collect(),
            part: match (slot.opening, k == last) {
                (true, false) => PeriodPart::Opening,
                (_, true) if last > 0 => PeriodPart::Closing,
                _ => PeriodPart::Regular,
            },
        })
        .collect();
    let rules = rules_for(spec, styles, &periods, &end)?;
    Ok(Schedule {
        id: spec.id.clone(),
        periods,
        rules,
        renewal_date: end,
        spec: spec.clone(),
    })
}

/// The rules generating `periods`: opening stub, main run, final.
fn rules_for(
    spec: &ScheduleSpec,
    styles: &JournalStyles<'_>,
    periods: &[Period],
    renewal: &str,
) -> Result<Vec<RuleText>, ScheduleError> {
    let opening = usize::from(
        periods
            .first()
            .is_some_and(|p| p.part == PeriodPart::Opening),
    );
    let rest = &periods[opening..];
    let split_final = rest.len() >= 2 && rest[rest.len() - 1].amounts != rest[0].amounts;
    let (main, last) = if split_final {
        (&rest[..rest.len() - 1], rest.last())
    } else {
        (rest, None)
    };
    let mut out = Vec::new();
    if opening == 1 && !main.is_empty() {
        out.push((
            RuleRole::Opening,
            periods[0].date.clone(),
            &periods[0],
            false,
        ));
    }
    let main_rule = match main {
        [] => None,
        [only] => Some(only.date.clone()),
        [first, .., final_main] => {
            // The final rule's date; else, for calendar months, the end of
            // service (D-anchor's `to 2027-12-01`), and for anniversaries the
            // day after the last occurrence (D-sched's `to 2027-10-18`).
            let to = match (last, spec.proration) {
                (Some(period), _) => period.date.clone(),
                (None, Proration::Calendar) => renewal.to_string(),
                (None, Proration::Anniversary) => add_days(&final_main.date, 1),
            };
            let day = &first.date[8..10];
            Some(if spec.proration == Proration::Anniversary && day > "28" {
                format!(
                    "every {} day of month from {}-01 to {to}",
                    ordinal(day),
                    &first.date[..7]
                )
            } else {
                format!("monthly from {} to {to}", first.date)
            })
        }
    };
    if opening == 1 && main.is_empty() {
        // A single partial month: the stub is the whole schedule.
        out.push((RuleRole::Main, periods[0].date.clone(), &periods[0], true));
    }
    if let (Some(period), Some(first)) = (main_rule, main.first()) {
        out.push((RuleRole::Main, period, first, true));
    }
    if let Some(final_period) = last {
        out.push((
            RuleRole::Final,
            final_period.date.clone(),
            final_period,
            false,
        ));
    }
    out.into_iter()
        .map(|(role, period, sample, main)| {
            let renewal = main.then_some(renewal);
            Ok(RuleText {
                role,
                text: render_rule(spec, styles, &period, sample, renewal)?,
                period,
            })
        })
        .collect()
}

/// `29` → `29th`, `31` → `31st`.
fn ordinal(day: &str) -> String {
    let suffix = match day {
        "31" | "21" | "01" => "st",
        "22" | "02" => "nd",
        "23" | "03" => "rd",
        _ => "th",
    };
    format!("{}{suffix}", day.trim_start_matches('0'))
}

/// The header comment's tags.
fn header_tags<'a>(
    id: &'a str,
    renewal: Option<&'a str>,
    extra: &'a [(String, String)],
) -> Vec<(&'a str, &'a str)> {
    std::iter::once((RECOGNITION_TAG, id))
        .chain(renewal.map(|date| (RENEWAL_DATE_TAG, date)))
        .chain(extra.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .collect()
}

pub(crate) fn tags_comment(tags: &[(&str, &str)]) -> String {
    tags.iter()
        .map(|(name, value)| format!("{name}: {value}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The posting lines of one period: per line, the source account takes
/// `-amount` and the target `+amount`, each with the party tag.
pub(crate) fn posting_lines(
    lines: &[ScheduleLine],
    styles: &JournalStyles<'_>,
    amounts: &[Dec],
) -> Result<Vec<(String, String, Option<String>)>, ScheduleError> {
    lines
        .iter()
        .zip(amounts)
        .flat_map(|(line, amount)| {
            let style = style_of(line, styles);
            let party = line
                .party
                .as_ref()
                .map(|party| format!("{}: {}", party.kind.tag(), party.name));
            let render =
                move |quantity: Result<Dec, ScheduleError>| -> Result<String, ScheduleError> {
                    let text = render_amount(&amount_in(&line.total.commodity, &style, quantity?));
                    if text.contains("  ") || text.contains('\t') {
                        return invalid(format!("the amount `{text}` would not read back"));
                    }
                    Ok(text)
                };
            let negated = amount.neg().map_err(ScheduleError::from);
            [
                (line.source_account.clone(), render(negated), party.clone()),
                (line.target_account.clone(), render(Ok(*amount)), party),
            ]
        })
        .map(|(account, amount, party)| amount.map(|amount| (account, amount, party)))
        .collect()
}

/// Lay out posting lines with the amounts right-aligned in one column.
pub(crate) fn layout_postings(postings: &[(String, String, Option<String>)]) -> String {
    let account_width = postings
        .iter()
        .map(|(a, _, _)| a.chars().count())
        .max()
        .unwrap_or(0);
    let amount_width = postings
        .iter()
        .map(|(_, m, _)| m.chars().count())
        .max()
        .unwrap_or(0);
    postings
        .iter()
        .map(|(account, amount, party)| {
            let comment = party
                .as_ref()
                .map(|party| format!("  ; {party}"))
                .unwrap_or_default();
            format!("    {account:<account_width$}  {amount:>amount_width$}{comment}\n")
        })
        .collect()
}

fn render_rule(
    spec: &ScheduleSpec,
    styles: &JournalStyles<'_>,
    period: &str,
    sample: &Period,
    renewal: Option<&str>,
) -> Result<String, ScheduleError> {
    let tags = tags_comment(&header_tags(&spec.id, renewal, &spec.header_tags));
    let description = if spec.description.is_empty() {
        String::new()
    } else {
        format!("  {}", spec.description)
    };
    let postings = posting_lines(&spec.lines, styles, &sample.amounts)?;
    Ok(format!(
        "~ {period}{description}  ; {tags}\n{}",
        layout_postings(&postings)
    ))
}

// ---------------------------------------------------------------------------
// Reading rules back
// ---------------------------------------------------------------------------

/// One generated occurrence: its date and, per source account, the amount
/// recognized out of it (signed like the source line).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Occurrence {
    /// The date the rule fires.
    pub date: String,
    /// Per source account asked for.
    pub amounts: Vec<Dec>,
}

/// The dates `spec` fires on, as day numbers, through `through` (inclusive)
/// and before its own exclusive `to`. `None` when the rule has no bounded set
/// of dates: no `from`, an unbounded recurrence with no `through`, or an
/// [`PeriodKind::Unsupported`] period.
///
/// The forms a schedule writes (`monthly from`, `every Nth day of month
/// from`, a single date) are walked here on day numbers, which is what keeps
/// the due set inside its budget; every other form goes through the budget
/// report's walk. A test pins the two together.
#[must_use]
pub(crate) fn occurrence_days(spec: &PeriodSpec, through: Option<i32>) -> Option<Vec<i32>> {
    let from = spec.start.as_deref()?;
    let to = spec.end.as_deref().map(day_of);
    let limit = match (to, through) {
        (Some(to), Some(through)) => (to - 1).min(through),
        (Some(to), None) => to - 1,
        (None, Some(through)) => through,
        (None, None) if spec.kind == PeriodKind::Once => day_of(from),
        (None, None) => return None,
    };
    let monthly = |anchor_day: i64, lower: i32| -> Vec<i32> {
        let (y, m, _) = ymd(day_of(from));
        (0_i64..)
            .map(|k| {
                let month = first_of_month(y, m + k);
                let (yy, mm, _) = ymd(month);
                month + i32::try_from(anchor_day.min(days_in_month(yy, mm)) - 1).unwrap_or(0)
            })
            .take_while(|day| *day <= limit)
            .filter(|day| *day >= lower)
            .collect()
    };
    match spec.kind {
        PeriodKind::Once => Some(
            [day_of(from)]
                .into_iter()
                .filter(|day| *day <= limit)
                .collect(),
        ),
        PeriodKind::Every {
            unit: crate::model::PeriodExpr::Monthly,
            multiplier: 1,
        } => Some(monthly(ymd(day_of(from)).2, i32::MIN)),
        PeriodKind::Anchored {
            unit: crate::model::PeriodExpr::Monthly,
            anchor: crate::model::Anchor::DayOfMonth(n),
        } => Some(monthly(i64::from(n), day_of(from))),
        PeriodKind::Unsupported => None,
        _ => occurrences(from, &iso_of(limit), spec)
            .ok()
            .map(|dates| dates.iter().map(|date| day_of(date)).collect()),
    }
}

/// Read `rules` back into their occurrences, through the budget report's own
/// occurrence walk: per date, the amount each of `source_accounts` is
/// recognized by (the negation of what the rules post to it). Several rules
/// firing on one date add up.
///
/// # Errors
/// [`ScheduleError::Unbounded`] for a rule with no bounded set of dates;
/// [`ScheduleError::Overflow`] on decimal overflow.
pub fn periods_of(
    rules: &[&PeriodicTransaction],
    source_accounts: &[&str],
) -> Result<Vec<Occurrence>, ScheduleError> {
    let mut by_date: BTreeMap<String, Vec<Dec>> = BTreeMap::new();
    for rule in rules {
        let spec = &rule.period;
        let unbounded = || ScheduleError::Unbounded(spec.raw.clone());
        let from = spec.start.as_deref().ok_or_else(unbounded)?;
        let last = match (&spec.kind, spec.end.as_deref()) {
            (PeriodKind::Once, _) => from.to_string(),
            (PeriodKind::Unsupported, _) | (_, None) => return Err(unbounded()),
            (_, Some(to)) => add_days(to, -1),
        };
        let dates = occurrences(from, &last, spec).map_err(|_| unbounded())?;
        for date in dates {
            let amounts = by_date
                .entry(date)
                .or_insert_with(|| vec![Dec::zero(); source_accounts.len()]);
            for posting in &rule.postings {
                let Some(at) = source_accounts
                    .iter()
                    .position(|account| *account == posting.account.0)
                else {
                    continue;
                };
                for amount in &posting.amounts {
                    amounts[at] = amounts[at].sub(amount.quantity)?;
                }
            }
        }
    }
    Ok(by_date
        .into_iter()
        .map(|(date, amounts)| Occurrence { date, amounts })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CommoditySide, Journal};
    use crate::parse::{parse_journal, parse_journal_with_styles};
    use proptest::prelude::*;

    fn usd(mantissa: i128, places: u32) -> Amount {
        Amount {
            commodity: Commodity("$".into()),
            quantity: Dec::new(mantissa, places),
            style: AmountStyle {
                side: CommoditySide::Left,
                spaced: false,
                decimal_mark: Some('.'),
                digit_groups: None,
                precision: places,
            },
            cost: None,
        }
    }

    fn main_journal(precision: u32) -> Journal {
        let zeros = "0".repeat(precision as usize);
        let directive = if precision == 0 {
            "commodity $1000\n".to_string()
        } else {
            format!("commodity $1000.{zeros}\n")
        };
        parse_journal(&directive, "books.journal").expect("parses")
    }

    fn spec(
        total: Amount,
        start: &str,
        term: Term,
        proration: Proration,
        lines: usize,
    ) -> ScheduleSpec {
        ScheduleSpec {
            id: "k7f3a2".into(),
            lines: (0..lines)
                .map(|i| ScheduleLine {
                    source_account: format!("liabilities:deferred-revenue:l{i}"),
                    target_account: format!("revenue:sales:l{i}"),
                    total: total.clone(),
                    party: Some(Party {
                        kind: PartyKind::Customer,
                        name: "Customer X".into(),
                    }),
                })
                .collect(),
            start: start.into(),
            term,
            proration,
            description: "Customer X Recognized".into(),
            header_tags: vec![],
        }
    }

    fn read_back(schedule: &Schedule, journal: &Journal) -> Vec<(String, Vec<Dec>)> {
        let text = schedule.rules_text();
        let parsed =
            parse_journal_with_styles(&text, "recognition.journal", &journal.commodity_styles)
                .unwrap_or_else(|e| panic!("rules parse: {e}\n{text}"));
        let rules: Vec<&PeriodicTransaction> = parsed.periodic_transactions.iter().collect();
        let accounts: Vec<&str> = schedule
            .spec
            .lines
            .iter()
            .map(|line| line.source_account.as_str())
            .collect();
        periods_of(&rules, &accounts)
            .expect("bounded")
            .into_iter()
            .map(|o| (o.date, o.amounts.into_iter().map(Dec::normalized).collect()))
            .collect()
    }

    fn as_pairs(schedule: &Schedule) -> Vec<(String, Vec<Dec>)> {
        schedule
            .periods
            .iter()
            .map(|p| {
                (
                    p.date.clone(),
                    p.amounts.iter().copied().map(Dec::normalized).collect(),
                )
            })
            .collect()
    }

    #[test]
    fn patricks_scenario_two_is_twelve_in_advance() {
        let j = main_journal(2);
        let s = build(
            &spec(
                usd(-1_200_000, 2),
                "2026-11-17",
                Term::Months(12),
                Proration::Anniversary,
                1,
            ),
            &JournalStyles::of(&j),
        )
        .expect("builds");
        assert_eq!(s.periods.len(), 12);
        assert_eq!(s.periods[0].date, "2026-11-17");
        assert_eq!(s.periods[11].date, "2027-10-17");
        assert_eq!(s.renewal_date, "2027-11-17");
        assert_eq!(s.rules.len(), 1);
        assert_eq!(
            s.rules[0].text,
            "~ monthly from 2026-11-17 to 2027-10-18  Customer X Recognized  ; recognition: k7f3a2, renewal-date: 2027-11-17\n    \
             liabilities:deferred-revenue:l0   $1000.00  ; customer: Customer X\n    \
             revenue:sales:l0                 $-1000.00  ; customer: Customer X\n"
        );
        assert_eq!(read_back(&s, &j), as_pairs(&s));
    }

    #[test]
    fn ten_thousand_over_twelve_trues_up_the_last() {
        let j = main_journal(2);
        let s = build(
            &spec(
                usd(-1_000_000, 2),
                "2026-11-17",
                Term::Months(12),
                Proration::Anniversary,
                1,
            ),
            &JournalStyles::of(&j),
        )
        .expect("builds");
        assert!(
            s.periods[..11]
                .iter()
                .all(|p| p.amounts[0] == Dec::new(-83333, 2))
        );
        assert_eq!(s.periods[11].amounts[0], Dec::new(-83337, 2));
        assert_eq!(s.rules.len(), 2);
        assert!(
            s.rules[0]
                .text
                .starts_with("~ monthly from 2026-11-17 to 2027-10-17 ")
        );
        assert!(
            s.rules[1]
                .text
                .starts_with("~ 2027-10-17  Customer X Recognized  ; recognition: k7f3a2\n")
        );
        assert_eq!(read_back(&s, &j), as_pairs(&s));
    }

    #[test]
    fn anchored_shape_one_has_stub_year_and_remainder() {
        let j = main_journal(2);
        let s = build(
            &spec(
                usd(-1_200_000, 2),
                "2026-11-17",
                Term::Months(12),
                Proration::Calendar,
                1,
            ),
            &JournalStyles::of(&j),
        )
        .expect("builds");
        let got: Vec<(&str, Dec)> = s
            .periods
            .iter()
            .map(|p| (p.date.as_str(), p.amounts[0]))
            .collect();
        assert_eq!(got[0], ("2026-11-17", Dec::new(-46667, 2)));
        assert!(
            got[1..12]
                .iter()
                .all(|(d, a)| d.ends_with("-01") && *a == Dec::new(-100_000, 2))
        );
        assert_eq!(got[12], ("2027-11-01", Dec::new(-53333, 2)));
        let periods: Vec<&str> = s.rules.iter().map(|r| r.period.as_str()).collect();
        assert_eq!(
            periods,
            vec![
                "2026-11-17",
                "monthly from 2026-12-01 to 2027-11-01",
                "2027-11-01"
            ]
        );
        assert_eq!(read_back(&s, &j), as_pairs(&s));
    }

    #[test]
    fn anchored_shape_two_is_a_stub_schedule_and_a_whole_year() {
        let j = main_journal(2);
        let stub = build(
            &spec(
                usd(-46667, 2),
                "2026-11-17",
                Term::Through("2026-11-30".into()),
                Proration::Calendar,
                1,
            ),
            &JournalStyles::of(&j),
        )
        .expect("builds");
        assert_eq!(stub.rules.len(), 1);
        assert_eq!(stub.rules[0].period, "2026-11-17");
        let year = build(
            &spec(
                usd(-1_200_000, 2),
                "2026-12-01",
                Term::Months(12),
                Proration::Calendar,
                1,
            ),
            &JournalStyles::of(&j),
        )
        .expect("builds");
        assert_eq!(year.rules.len(), 1);
        assert_eq!(
            year.rules[0].period,
            "monthly from 2026-12-01 to 2027-12-01"
        );
    }

    #[test]
    fn days_29_to_31_use_the_anchored_form() {
        let j = main_journal(2);
        let s = build(
            &spec(
                usd(-1_200_000, 2),
                "2027-01-31",
                Term::Months(12),
                Proration::Anniversary,
                1,
            ),
            &JournalStyles::of(&j),
        )
        .expect("builds");
        assert_eq!(
            s.rules[0].period,
            "every 31st day of month from 2027-01-01 to 2028-01-01"
        );
        let dates: Vec<&str> = s.periods.iter().take(3).map(|p| p.date.as_str()).collect();
        assert_eq!(dates, vec!["2027-01-31", "2027-02-28", "2027-03-31"]);
        assert_eq!(read_back(&s, &j), as_pairs(&s));
    }

    #[test]
    fn bad_inputs_are_refused_with_a_reason() {
        let j = main_journal(2);
        let styles = JournalStyles::of(&j);
        let base = spec(
            usd(-100, 2),
            "2026-11-17",
            Term::Months(12),
            Proration::Anniversary,
            1,
        );
        let cases: Vec<ScheduleSpec> = vec![
            ScheduleSpec {
                id: "none".into(),
                ..base.clone()
            },
            ScheduleSpec {
                term: Term::Months(0),
                ..base.clone()
            },
            ScheduleSpec {
                term: Term::Through("2026-01-01".into()),
                ..base.clone()
            },
            ScheduleSpec {
                description: "a ; b".into(),
                ..base.clone()
            },
            ScheduleSpec {
                start: "2026-02-30".into(),
                ..base.clone()
            },
            ScheduleSpec {
                lines: vec![],
                ..base.clone()
            },
            ScheduleSpec {
                header_tags: vec![("recognized".into(), "x".into())],
                ..base.clone()
            },
        ];
        for case in cases {
            assert!(
                matches!(build(&case, &styles), Err(ScheduleError::Invalid(_))),
                "{case:?}"
            );
        }
        let mut comma = base.clone();
        comma.lines[0].party = Some(Party {
            kind: PartyKind::Customer,
            name: "A, Inc".into(),
        });
        assert!(build(&comma, &styles).is_err());
    }

    fn arb_date() -> impl Strategy<Value = String> {
        (2020_i64..2032, 1_i64..=12, 1_i64..=31).prop_map(|(y, m, d)| {
            let d = d.min(days_in_month(y, m));
            format!("{y:04}-{m:02}-{d:02}")
        })
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(400))]

        #[test]
        fn schedules_sum_exactly_and_read_back_as_their_periods(
            precision in 0_u32..=4,
            units in 1_i128..=100_000_000_000,
            negative in any::<bool>(),
            term in 1_u32..=120,
            start in arb_date(),
            calendar in any::<bool>(),
            through_offset in proptest::option::of(0_i64..800),
            lines in 1_usize..=4,
        ) {
            let j = main_journal(precision);
            let styles = JournalStyles::of(&j);
            let scale = 10_i128.pow(precision);
            // Amounts from 0.01 to 10^9, at the commodity's precision.
            let mantissa = (units % (1_000_000_000 * scale)).max(1) * if negative { -1 } else { 1 };
            let term = match through_offset {
                Some(days) => Term::Through(add_days(&start, days)),
                None => Term::Months(term),
            };
            let proration = if calendar { Proration::Calendar } else { Proration::Anniversary };
            let s = build(&spec(usd(mantissa, precision), &start, term, proration, lines), &styles)
                .expect("builds");

            // Σ amounts = T exactly, per line.
            for line in 0..lines {
                let sum = s.periods.iter().try_fold(Dec::zero(), |acc, p| acc.add(p.amounts[line])).expect("sums");
                prop_assert_eq!(sum.normalized(), Dec::new(mantissa, precision).normalized());
            }
            // Every regular amount is equal; only opening and closing differ.
            let regular: Vec<&Period> = s.periods.iter().filter(|p| p.part == PeriodPart::Regular).collect();
            if let Some(first) = regular.first() {
                prop_assert!(regular.iter().all(|p| p.amounts == first.amounts));
            }
            // Dates strictly increase, at most one per calendar month.
            for pair in s.periods.windows(2) {
                prop_assert!(pair[0].date < pair[1].date);
                prop_assert!(pair[0].date[..7] < pair[1].date[..7]);
            }
            // The rules read back as exactly the periods.
            prop_assert_eq!(read_back(&s, &j), as_pairs(&s));
            // The main rule's `to` is the day after its last occurrence, or the final rule's date.
            if let Some(main) = s.rules.iter().find(|r| r.role == RuleRole::Main && r.period.contains(" to ")) {
                let to = main.period.rsplit(" to ").next().expect("to");
                match s.rules.iter().find(|r| r.role == RuleRole::Final) {
                    Some(last) => prop_assert_eq!(to, last.period.as_str()),
                    None if calendar => prop_assert_eq!(to, s.renewal_date.as_str()),
                    None => prop_assert_eq!(to, add_days(&s.periods.last().expect("one").date, 1)),
                }
                // Day 29–31 anniversaries use `every Nth day of month`.
                let day = &s.periods[0].date[8..10];
                prop_assert_eq!(
                    main.period.starts_with("every "),
                    proration == Proration::Anniversary && day > "28"
                );
            }
        }

        #[test]
        fn the_fast_walk_matches_the_budget_walk(
            start in arb_date(),
            months in 1_i64..60,
            form in 0_u8..3,
            through in proptest::option::of(0_i64..2000),
        ) {
            let to = add_months(&start, months);
            let raw = match form {
                0 => format!("monthly from {start} to {to}"),
                1 => format!("every {} day of month from {}-01 to {to}", ordinal(&start[8..10]), &start[..7]),
                _ => start.clone(),
            };
            let spec = crate::parse::parse_period_spec(&raw);
            let through_day = through.map(|days| day_of(&add_days(&start, days)));
            let fast = occurrence_days(&spec, through_day).expect("bounded");
            let last = match (spec.kind, through_day) {
                (PeriodKind::Once, Some(t)) => iso_of(t.min(day_of(&start))),
                (PeriodKind::Once, None) => start.clone(),
                (_, Some(t)) => iso_of(t.min(day_of(&to) - 1)),
                (_, None) => add_days(&to, -1),
            };
            let from = spec.start.clone().expect("a from");
            let slow: Vec<i32> = if last < from {
                vec![]
            } else {
                occurrences(&from, &last, &spec)
                    .expect("walks")
                    .iter()
                    .map(|d| day_of(d))
                    .collect()
            };
            prop_assert_eq!(fast, slow, "{}", raw);
        }
    }
}
