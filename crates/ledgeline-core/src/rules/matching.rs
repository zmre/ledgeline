//! Scoring a `*.rules` file against a dropped statement file (Imports, WP-11
//! lane D) — deciding which of the user's rules files, if any, actually fits the
//! data they just handed us.
//!
//! # The obvious approach does not work
//!
//! "Run hledger and see if it parses" is wrong, and not marginally. Verified
//! against hledger 1.52, the version pinned in `flake.nix`:
//!
//! - A **checking-account** rules file run against a **credit-card** CSV produced
//!   transactions with `income:unknown` postings and a posting carrying **no
//!   amount at all** — and `hledger check` exited **0**, perfectly happy.
//! - A rules file merely *lacking* a `currency` rule produces **bare** amounts,
//!   which form a commodity of their own. The import "succeeds", the accounts are
//!   all correctly categorised, `print` looks right — and the user's `$` balance
//!   never moves, because none of that money is in `$`.
//!
//! Both are silent. Neither is an error, an exit code, or a line of stderr. So:
//!
//! > **Parse success is not a matching signal.** (`plans/11-enhanced-import.md`,
//! > fact 4 — the whole reason this module exists.)
//!
//! What *is* a signal is the **shape of what hledger produced**, which is why
//! stage 2 reads `hledger print -O json` rather than hledger's exit status.
//! `fixtures/import/match/garbage-success.rules` and `no-currency.rules` are the
//! two findings above, committed, and the tests assert they score near the bottom
//! *because* hledger accepts them.
//!
//! # Two stages, because 200 subprocesses is not acceptable
//!
//! [`super::discover`] already permits `MAX_RULES_FILES = 200`. Spawning one
//! hledger per rules file per dropped statement would be two hundred processes
//! for one drag-and-drop, so the work is split:
//!
//! | Stage | What it is | What it costs | What it decides |
//! | --- | --- | --- | --- |
//! | 1 — [`prefilter`] | pure Rust over data already parsed | nothing | rejects the obviously-wrong |
//! | 2 — [`signals_from_hledger_json`] + [`score`] | pure, over JSON the caller obtained | one subprocess per survivor | ranks the plausible |
//!
//! Stage 1 rejects on facts that need no execution: the `fields` list is wider
//! than the data, `skip` swallows the whole file, or the `date-format` cannot
//! read the date column. At most [`MAX_SCORED_CANDIDATES`] survivors reach
//! stage 2.
//!
//! # This module never runs anything
//!
//! There is no [`std::process::Command`] here and there must not be. Every
//! function is pure: [`prefilter`] takes an already-parsed [`RulesDoc`] and
//! [`Tabular`], and [`signals_from_hledger_json`] takes JSON the **caller**
//! obtained. That keeps the interesting logic testable without a binary on
//! `PATH` — the test suite is hermetic and drives committed goldens under
//! `fixtures/import/match/golden/` — and it keeps process spawning confined to
//! the one server module allowed to do it (`docs/imports.md`).
//!
//! `-O json` is used precisely so that **no human-readable hledger output is
//! ever regex-scraped**. A rendered ledger is a display format; scraping one is
//! how a tool comes to believe a `$-3,000.00` is a `$3.00`.
//!
//! # No path ever appears here
//!
//! Same rule the rules API already holds to (`docs/imports.md` § Security). No
//! [`MatchError`] carries a path, and [`Candidate::id`]/[`Candidate::label`] are
//! the discovery handle and display name, which are relative by construction.
//! hledger's JSON *does* carry an absolute `sourceName` in `tsourcepos`; this
//! module never reads that field, and the golden generator strips it before the
//! bytes are committed.

use crate::convert::Tabular;
use crate::rules::RulesDoc;
use serde_json::Value;
use std::time::SystemTime;
use thiserror::Error;

// ---------------------------------------------------------------------------
// Budgets and weights
// ---------------------------------------------------------------------------

/// How many stage-1 survivors are handed to hledger.
///
/// One subprocess each, on the interaction path of a drag-and-drop, so this is a
/// latency budget rather than a correctness one. Eight is comfortably more than
/// the number of rules files that can plausibly pass stage 1 for one statement —
/// they have to agree on column count *and* date format — and it keeps the worst
/// case to well under a second.
pub const MAX_SCORED_CANDIDATES: usize = 8;

/// How many date cells stage 1 tries against a rules file's `date-format`.
///
/// The check is a *rejection*, so it only has to see enough to be sure. Eight
/// rows past `skip` is plenty to tell `%d.%m.%Y` from `%m/%d/%Y`, and reading
/// more of a large statement to reach the same answer would be work spent on
/// candidates that are about to be discarded anyway.
const DATE_SAMPLES: usize = 8;

/// A posting with **no amount** is the fact-4 signature: silently broken output
/// that hledger reports as a success. Weight 1.0, so a candidate that breaks
/// every transaction this way scores exactly zero.
const W_AMOUNTLESS: f32 = 1.0;

/// An amount in the wrong commodity — no commodity at all, or not the one the
/// caller is importing into. The commodity trap: the money is imported, and the
/// balance the user is watching never moves. Exactly as fatal as an amountless
/// posting, and weighted the same.
const W_BARE_COMMODITY: f32 = 1.0;

/// A posting left in `expenses:unknown` / `income:unknown`. Moderate, not fatal:
/// the amounts and dates are right and the money lands in the right account on
/// one side, so the file is *usable* and the user fixes it by adding a rule. A
/// candidate every one of whose postings is unknown still scores 0.4, which is
/// above anything structurally broken and below anything categorised.
const W_UNKNOWN_ACCOUNT: f32 = 0.6;

/// An empty payee. Light: unpleasant to read, but the transaction is otherwise
/// sound and nothing about it is wrong.
const W_EMPTY_DESCRIPTION: f32 = 0.2;

/// The `fields` list is not exactly as wide as the data. Corroborating only —
/// naming fewer columns than a file has is legitimate and common (you name the
/// ones you use), so this can nudge a ranking and must never decide one.
const W_COLUMN_COUNT: f32 = 0.15;

/// The data's header row does not name anything the `fields` list names. The
/// weakest signal here, and deliberately so — see [`Signals::header_matches_source`].
const W_HEADER: f32 = 0.05;

/// The most any candidate producing an amountless posting or an off-commodity
/// amount may score.
///
/// This is the cap that makes fact 4 a *policy* rather than an arithmetic
/// accident. Both failures are **silent**, so rarity is not mitigation: one
/// amountless posting in five hundred still means the rules file is wrong about
/// at least one row shape, and the user will not notice. Without the cap that
/// candidate would rate 0.996 and be offered as the answer. With it, it can be
/// listed and chosen, but never suggested.
const FATAL_CAP: f32 = 0.25;

// ---------------------------------------------------------------------------
// Score
// ---------------------------------------------------------------------------

/// A match score in `0.0..=1.0`, 1.0 being "nothing was wrong with the output".
///
/// A newtype with a **private** field so it cannot be confused with a count —
/// every other number in [`Signals`] is a tally, and a bare `f32` next to eight
/// `usize`s is an invitation. The private field also buys an invariant:
/// [`Score::new`] is the only constructor, it clamps to the range and maps `NaN`
/// to zero, so **a `Score` is never `NaN`**. That is what lets this type be
/// `Ord`, and therefore lets [`rank`] be a total sort rather than a partial one
/// that silently misplaces elements.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Score(f32);

impl Score {
    /// The score of something that is not a match at all.
    pub const ZERO: Self = Self(0.0);

    /// Clamp `value` into `0.0..=1.0`, mapping `NaN` to [`Score::ZERO`].
    ///
    /// `NaN` maps to zero rather than panicking because the alternative is a
    /// crash on a candidate list, and "we could not score this" and "this scores
    /// nothing" are the same answer to the only question being asked.
    #[must_use]
    pub fn new(value: f32) -> Self {
        if value.is_nan() {
            return Self::ZERO;
        }
        Self(value.clamp(0.0, 1.0))
    }

    /// The score as a number, for display and for the wire.
    #[must_use]
    pub fn value(self) -> f32 {
        self.0
    }
}

/// Sound because [`Score::new`] is the only way to build one and it excludes
/// `NaN`, which is the sole reason `f32` is not `Eq` in the first place.
impl Eq for Score {}

impl Ord for Score {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // `total_cmp` rather than `partial_cmp(..).unwrap()`: it needs no
        // unwrap, and it is total even for the `NaN` this type cannot hold.
        self.0.total_cmp(&other.0)
    }
}

impl PartialOrd for Score {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

// ---------------------------------------------------------------------------
// Signals
// ---------------------------------------------------------------------------

/// What one rules file did to one statement file, counted.
///
/// Everything here is an **observation**, never a judgement — turning these into
/// a single number is [`score`]'s job alone, so the weighting lives in exactly
/// one place and a caller cannot quietly invent its own.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Signals {
    /// Transactions hledger produced. Zero means the rules file read the file
    /// and found nothing in it, which is not a match at any score.
    pub txns: usize,
    /// Postings across all of `txns`.
    pub postings: usize,
    /// **Fact 4: silently broken.** Postings whose `pamount` is an EMPTY ARRAY —
    /// hledger wrote a posting with no amount because the column the rules
    /// pointed `amount` at was empty on that row. `hledger check` accepts this.
    pub amountless_postings: usize,
    /// **Fact 4: the commodity trap.** Amounts that will not land in the
    /// commodity being imported into: `acommodity` is `""` (no commodity at all,
    /// which is what a missing `currency` rule produces), or — when the caller
    /// named an expected commodity — it is a different one. Both mean the import
    /// succeeds and the watched balance does not move.
    pub bare_commodity_amounts: usize,
    /// Postings whose account ends in `:unknown` — hledger's
    /// `expenses:unknown` / `income:unknown` fallback for a record no if-rule
    /// matched.
    ///
    /// A rules file that *declares* `account2 expenses:unknown` as its own
    /// default is counted identically, and that is correct rather than a
    /// limitation: an uncategorised posting is uncategorised whether hledger
    /// chose the name or the user did.
    pub unknown_accounts: usize,
    /// Transactions with an empty (or whitespace-only) `tdescription`.
    pub empty_descriptions: usize,
    /// The `fields` list is exactly as wide as the data. From stage 1 — hledger's
    /// output cannot say. See [`Signals::with_prefilter`].
    pub column_count_matches: bool,
    /// The data's header row names at least one thing the `fields` list names.
    ///
    /// Deliberately "at least one", not "all". Rules files routinely name columns
    /// for hledger's semantics (`amount-out`) rather than for the bank's spelling
    /// (`Withdrawal`), so demanding a full match would punish the correct file;
    /// but `date`, `description` and `amount` are near-universal, so *some*
    /// overlap is real corroboration. It is weighted as the weak hint it is.
    pub header_matches_source: bool,
}

impl Signals {
    /// Fold stage 1's two shape observations into signals read from hledger.
    ///
    /// The split exists because neither half can see the other's evidence:
    /// hledger's JSON says nothing about how wide the CSV was, and stage 1 has
    /// not run hledger. [`signals_from_hledger_json`] therefore leaves both flags
    /// `false`, which is the **fail-safe** direction — a caller that forgets this
    /// call gets a slightly pessimistic score, never an optimistic one.
    #[must_use]
    pub fn with_prefilter(self, pass: &PrefilterPass) -> Self {
        Self {
            column_count_matches: pass.column_count_matches,
            header_matches_source: pass.header_matches_source,
            ..self
        }
    }
}

// ---------------------------------------------------------------------------
// Candidates and ranking
// ---------------------------------------------------------------------------

/// One rules file, scored, ready for the UI to offer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// [`super::DiscoveredRules::id`] — the existing rules-file handle, a
    /// relative path. The only way to name a rules file anywhere in this feature.
    pub id: String,
    /// [`super::DiscoveredRules::label`] — display only.
    pub label: String,
    /// What [`score`] made of [`Candidate::signals`].
    pub score: Score,
    /// The evidence, carried so the UI can say *why* rather than only *how much*.
    /// "4 postings would have no amount" is actionable; "0.18" is not.
    pub signals: Signals,
}

/// A [`Candidate`] plus the one thing outside it that decides ties.
///
/// A separate type rather than a field on [`Candidate`] because an mtime is not
/// a property of a match — it is a property of a *file*, and it is only ever
/// consulted when two matches are equally good.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ranking {
    /// The scored candidate.
    pub candidate: Candidate,
    /// [`super::DiscoveredRules::modified`]. **Ranking only** — read that field's
    /// own comment before using this for anything else.
    pub modified: Option<SystemTime>,
}

/// Sort candidates best-first: **score descending, then modification time
/// descending**, then `id` ascending.
///
/// The mtime tie-break is the point. A real user has rules files going back
/// years — `checking-2024.csv.rules`, `checking-2025.csv.rules` — which are near
/// enough identical that they score identically, and the one they touched most
/// recently is the one they are still importing into. That naturally prefers the
/// current year's without this module ever parsing a year out of a filename.
///
/// A file with no recorded mtime sorts *after* one that has one: absent is not
/// evidence of recency. `id` breaks the remaining ties so the order is **total**
/// and two runs over an unchanged tree produce the same list — the same
/// reproducibility rule the discovery scan follows.
pub fn rank(rankings: &mut [Ranking]) {
    rankings.sort_by(|a, b| {
        b.candidate
            .score
            .cmp(&a.candidate.score)
            .then_with(|| b.modified.cmp(&a.modified))
            .then_with(|| a.candidate.id.cmp(&b.candidate.id))
    });
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Why hledger's JSON could not be read as transactions.
///
/// Deliberately **strict**: a shape this module does not recognise is an error,
/// not an empty tally. Returning zeroed [`Signals`] for output we failed to
/// understand would score an unreadable candidate as *perfect* — which is fact 4
/// all over again, one level up. No variant carries a path or a value from the
/// user's data.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MatchError {
    /// The top level is not a JSON array. `hledger print -O json` always emits
    /// one, so this is a different command's output, or a different hledger.
    #[error("hledger's output was not a list of transactions")]
    NotTransactions,
    /// A transaction has no `tpostings` array.
    #[error("a transaction in hledger's output has no postings")]
    MalformedTransaction,
    /// A posting has no `paccount` string or no `pamount` array.
    ///
    /// An **absent** `pamount` is an error while an **empty** one is the fact-4
    /// signal, and conflating the two is exactly the mistake that must not be
    /// made: one means "we no longer understand hledger", the other means "this
    /// rules file is wrong".
    #[error("a posting in hledger's output has no account or no amount")]
    MalformedPosting,
}

// ---------------------------------------------------------------------------
// Stage 1 — the pure pre-filter
// ---------------------------------------------------------------------------

/// What stage 1 learned about a rules file it did **not** reject.
///
/// Carried forward rather than recomputed: stage 2 needs the expected commodity
/// and the two shape flags, and deriving them twice is how two answers to one
/// question come to disagree.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PrefilterPass {
    /// The `fields` list is exactly as wide as the data's modal record.
    pub column_count_matches: bool,
    /// The data's header row names at least one thing `fields` names. See
    /// [`Signals::header_matches_source`].
    pub header_matches_source: bool,
    /// How many columns `fields` names, or `None` when the file has no `fields`
    /// list at all.
    pub declared_columns: Option<usize>,
    /// How many columns the data's records actually have (the modal width).
    pub data_columns: usize,
    /// The rules file's top-level `currency`, which is what stage 2 should expect
    /// every amount to be in. `None` when the file does not say — in which case
    /// stage 2 penalises only genuinely commodity-less amounts, never merely
    /// unexpected ones.
    pub expected_commodity: Option<String>,
    /// How many date cells were tried against `date-format`, and how many parsed.
    /// `tried == 0` means the check did not run: no `fields` `date` column, no
    /// `date-format`, or a format using a specifier this module does not model.
    pub dates_tried: usize,
    /// How many of `dates_tried` parsed.
    pub dates_parsed: usize,
}

/// **Stage 1.** Decide cheaply whether `doc` could possibly describe `data`.
///
/// Pure: no I/O, no subprocess, no `PATH`. `None` means a **clear** mismatch and
/// the caller should not spend an hledger run on it.
///
/// Only three things are checked, all from what the existing parser already
/// produced ([`RulesDoc::settings`]) and what the preprocessor already read
/// ([`Tabular`]):
///
/// 1. **The `fields` list is wider than the data.** The rules file addresses a
///    column that does not exist anywhere in the file. (The converse — naming
///    *fewer* columns than the file has — is legitimate and common, and is not a
///    rejection; it only clears [`PrefilterPass::column_count_matches`].)
/// 2. **`skip` swallows the file.** Nothing would be imported at all.
/// 3. **`date-format` cannot read the date column.** Not one sampled cell parses.
///
/// # Why the bar for rejecting is deliberately high
///
/// A false rejection is invisible: the user's correct rules file simply never
/// appears in the list and they are told nothing fits. A false *acceptance* costs
/// one subprocess and is then caught by stage 2, which looks at what hledger
/// actually produced. The two failures are not symmetric, so every check here
/// declines to fire when it is unsure — an unmodelled `date-format` specifier, a
/// truncated preview, a missing `fields` list and a missing `date-format` each
/// mean "pass, and let stage 2 decide".
///
/// # What it deliberately does not do
///
/// It does not try to disambiguate `%m/%d/%Y` from `%d/%m/%Y` on a file whose
/// dates are all before the 13th — both genuinely parse, and picking one would be
/// guessing. Both survive, stage 2 scores them identically, and the mtime
/// tie-break in [`rank`] decides. Guessing is what a *user* is for.
#[must_use]
pub fn prefilter(doc: &RulesDoc, data: &Tabular) -> Option<PrefilterPass> {
    let settings = doc.settings();
    let fields = settings.fields.as_ref().map(|setting| &setting.value);
    let skip = settings.skip.map_or(0, |setting| setting.value) as usize;

    // (1) The rules file must not address a column the data does not have
    // anywhere. Compared against the WIDEST record, not the modal one: a ragged
    // file whose long rows justify the field list is a match, and the modal width
    // would reject it.
    let widths = record_widths(data);
    let widest = widths.iter().copied().max().unwrap_or(0);
    let declared = fields.map(Vec::len);
    if declared.is_some_and(|declared| declared > widest) {
        return None;
    }

    // (2) `skip` must leave something to import. Only asserted when the extract
    // is complete: `truncated` means `rows` is a lower bound, so a preview of a
    // long file would otherwise reject a rules file with a large legitimate
    // preamble.
    let records = usize::from(data.header.is_some()) + data.rows.len();
    if !data.truncated && skip >= records {
        return None;
    }

    // (3) The date column must be readable by the declared format. `dates_tried`
    // is zero whenever any input to the question is missing, and a question that
    // was not asked never rejects.
    let sampled = sample_dates(fields, data, skip);
    let parsed = settings
        .date_format
        .as_ref()
        .map(|setting| count_parsable(&setting.value, &sampled));
    let (dates_tried, dates_parsed) = match parsed {
        Some(Some(parsed)) => (sampled.len(), parsed),
        // No `date-format`, or one using a specifier this module does not model.
        Some(None) | None => (0, 0),
    };
    if dates_tried > 0 && dates_parsed == 0 {
        return None;
    }

    let modal = modal_width(&widths);
    Some(PrefilterPass {
        column_count_matches: declared.is_some_and(|declared| declared == modal),
        header_matches_source: header_overlaps(fields, data.header.as_deref()),
        declared_columns: declared,
        data_columns: modal,
        expected_commodity: settings
            .currency
            .map(|setting| setting.value.trim().to_string())
            .filter(|currency| !currency.is_empty()),
        dates_tried,
        dates_parsed,
    })
}

/// Every record's field count, header included — the header is a record to
/// hledger, which is the whole reason `skip` exists.
fn record_widths(data: &Tabular) -> Vec<usize> {
    data.header
        .iter()
        .map(Vec::len)
        .chain(data.rows.iter().map(Vec::len))
        .collect()
}

/// The most common record width, largest winning a tie.
///
/// The mode rather than the max, because one ragged trailer row ("5 records
/// exported") must not redefine how wide the file is. Largest-wins on a tie
/// keeps the answer independent of row order.
fn modal_width(widths: &[usize]) -> usize {
    widths
        .iter()
        .copied()
        .map(|width| {
            (
                widths.iter().filter(|other| **other == width).count(),
                width,
            )
        })
        .max()
        .map_or(0, |(_, width)| width)
}

/// The first [`DATE_SAMPLES`] non-empty cells of whichever column `fields` maps
/// to `date`, from the records `skip` leaves behind.
///
/// Empty when there is no `fields` list, no `date` in it, or no such column in
/// the data — each of which means the date question cannot be asked, and an
/// unasked question never rejects a candidate.
fn sample_dates(fields: Option<&Vec<String>>, data: &Tabular, skip: usize) -> Vec<String> {
    // hledger lowercases field names for its own lookups, so `Date` and `date`
    // are the same name to it and must be here too.
    let Some(at) = fields.and_then(|names| {
        names
            .iter()
            .position(|name| name.trim().eq_ignore_ascii_case("date"))
    }) else {
        return Vec::new();
    };
    // `skip` counts from the first record, and the header is record 0. Rows past
    // the header therefore start at `skip - 1` when a header was extracted.
    let skip_rows = skip.saturating_sub(usize::from(data.header.is_some()));
    data.rows
        .iter()
        .skip(skip_rows)
        .filter_map(|row| row.get(at))
        .map(|cell| cell.trim().to_string())
        .filter(|cell| !cell.is_empty())
        .take(DATE_SAMPLES)
        .collect()
}

/// Whether the data's header row names anything the `fields` list names.
///
/// Compared on letters and digits only, case-folded, so `Amount In`, `amount-in`
/// and `AMOUNT_IN` are one name. Empty `fields` entries ("ignore this column")
/// name nothing and are excluded, or every file with one would match everything.
fn header_overlaps(fields: Option<&Vec<String>>, header: Option<&[String]>) -> bool {
    let (Some(fields), Some(header)) = (fields, header) else {
        return false;
    };
    let normalized: Vec<String> = header.iter().map(|name| normalize_name(name)).collect();
    fields
        .iter()
        .map(|name| normalize_name(name))
        .filter(|name| !name.is_empty())
        .any(|name| normalized.contains(&name))
}

/// Lowercase, letters and digits only.
fn normalize_name(name: &str) -> String {
    name.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

// ---------------------------------------------------------------------------
// Stage 1's date-format check
// ---------------------------------------------------------------------------

/// How many of `values` `format` reads, or `None` if `format` uses a specifier
/// this module does not model.
///
/// `None` is not a failure — it is the honest "cannot answer", and it makes the
/// caller skip the check rather than reject a candidate on a question it could
/// not evaluate.
fn count_parsable(format: &str, values: &[String]) -> Option<usize> {
    let specs = parse_format(format)?;
    Some(
        values
            .iter()
            .filter(|value| matches_format(&specs, value))
            .count(),
    )
}

/// One element of a `date-format`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Spec {
    /// A character that must appear literally.
    Literal(char),
    /// Whitespace in the format, matching any run of whitespace including none —
    /// which is what strptime does.
    Space,
    /// A run of digits, at most `width` of them, range-checked as `field`.
    Number { field: Field, width: usize },
    /// An English month name, abbreviated or full.
    MonthName,
}

/// Which calendar field a [`Spec::Number`] fills, and therefore what range it
/// must be in. `Other` covers year and time components, where no cheap range
/// check tells a wrong format from a right one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    Month,
    Day,
    Other,
}

impl Field {
    /// Whether `value` is a possible value for this field.
    ///
    /// Month and day are checked against the calendar, never against each other
    /// or against the year: this is what rejects `%d/%m/%Y` on `01/15/2024`
    /// (month 15) without needing to know that 2024 is a leap year. Day is
    /// `1..=31` for every month, because refusing 31 April would be rejecting a
    /// *format* over one bad *row*.
    fn admits(self, value: u32) -> bool {
        match self {
            Self::Month => (1..=12).contains(&value),
            Self::Day => (1..=31).contains(&value),
            Self::Other => true,
        }
    }
}

/// Compile a `date-format`, or `None` for one using a specifier not modelled
/// here.
///
/// The supported set is what hledger's own documentation shows in rules files,
/// plus the padding modifiers. Everything else — `%Z`, `%z`, `%j`, `%s`, `%a`,
/// `%p` and friends — returns `None`, so an exotic format costs a subprocess
/// instead of costing the user their correct rules file.
fn parse_format(format: &str) -> Option<Vec<Spec>> {
    let chars: Vec<char> = format.chars().collect();
    let mut specs = Vec::with_capacity(chars.len());
    let mut at = 0usize;

    while at < chars.len() {
        let ch = chars[at];
        at += 1;
        if ch != '%' {
            specs.push(if ch.is_whitespace() {
                Spec::Space
            } else {
                Spec::Literal(ch)
            });
            continue;
        }
        // `%-d` / `%_d` / `%0d` differ from `%d` only in padding, and the number
        // reader below is already indifferent to padding.
        if matches!(chars.get(at), Some('-' | '_' | '0')) {
            at += 1;
        }
        let directive = *chars.get(at)?;
        at += 1;
        match directive {
            '%' => specs.push(Spec::Literal('%')),
            'Y' => specs.push(Spec::Number {
                field: Field::Other,
                width: 4,
            }),
            'y' | 'C' => specs.push(Spec::Number {
                field: Field::Other,
                width: 2,
            }),
            'm' => specs.push(Spec::Number {
                field: Field::Month,
                width: 2,
            }),
            'd' | 'e' => specs.push(Spec::Number {
                field: Field::Day,
                width: 2,
            }),
            'H' | 'I' | 'M' | 'S' => specs.push(Spec::Number {
                field: Field::Other,
                width: 2,
            }),
            'b' | 'h' | 'B' => specs.push(Spec::MonthName),
            // `%F` is `%Y-%m-%d`, spelled out rather than recursed into.
            'F' => specs.extend([
                Spec::Number {
                    field: Field::Other,
                    width: 4,
                },
                Spec::Literal('-'),
                Spec::Number {
                    field: Field::Month,
                    width: 2,
                },
                Spec::Literal('-'),
                Spec::Number {
                    field: Field::Day,
                    width: 2,
                },
            ]),
            _ => return None,
        }
    }
    Some(specs)
}

/// The twelve month names, longest form first so `January` is never read as
/// `Jan` with a stray `uary` left over.
const MONTHS: [(&str, &str); 12] = [
    ("january", "jan"),
    ("february", "feb"),
    ("march", "mar"),
    ("april", "apr"),
    ("may", "may"),
    ("june", "jun"),
    ("july", "jul"),
    ("august", "aug"),
    ("september", "sep"),
    ("october", "oct"),
    ("november", "nov"),
    ("december", "dec"),
];

/// Whether `value` reads completely as `specs`.
///
/// Leading and trailing whitespace on the cell is tolerated. hledger itself is
/// stricter, so this can accept a value hledger would refuse — which is the
/// correct direction for a filter whose false rejections are invisible and whose
/// false acceptances cost one subprocess.
fn matches_format(specs: &[Spec], value: &str) -> bool {
    let rest = specs
        .iter()
        .try_fold(value.trim(), |rest, spec| consume(*spec, rest));
    rest.is_some_and(|rest| rest.trim().is_empty())
}

/// Consume one `spec` from the front of `rest`, or `None` if it does not fit.
fn consume(spec: Spec, rest: &str) -> Option<&str> {
    match spec {
        Spec::Literal(expected) => rest.strip_prefix(expected),
        Spec::Space => Some(rest.trim_start()),
        Spec::Number { field, width } => {
            // `%e` is space-padded, so a leading space belongs to the number.
            let rest = rest.trim_start();
            let digits = rest
                .chars()
                .take(width)
                .take_while(char::is_ascii_digit)
                .count();
            if digits == 0 {
                return None;
            }
            // `digits` is a COUNT OF CHARS being used as a BYTE offset, which is
            // only sound because every char counted is an ASCII digit and is
            // therefore exactly one byte. `edit.rs` DL-1 is what happens when
            // that reasoning is left implicit, so it is written down: any
            // widening of the predicate past ASCII must revisit this line.
            let (number, tail) = rest.split_at(digits);
            // A run this short cannot overflow `u32`; the `ok()?` is there so a
            // future widening of `width` degrades to "no match" and never panics.
            field.admits(number.parse::<u32>().ok()?).then_some(tail)
        }
        Spec::MonthName => {
            // `to_ascii_lowercase` maps only ASCII, so it preserves every char's
            // byte length. A month name matched in `lower` therefore sits at the
            // same byte offsets in `rest`, and slicing `rest` by the name's
            // length can never split a code point — see the `Spec::Number` note.
            let lower = rest.to_ascii_lowercase();
            MONTHS
                .iter()
                .flat_map(|(full, abbreviated)| [*full, *abbreviated])
                .find(|name| lower.starts_with(name))
                .and_then(|name| rest.get(name.len()..))
        }
    }
}

// ---------------------------------------------------------------------------
// Stage 2 — reading hledger's structured output
// ---------------------------------------------------------------------------

/// hledger's JSON keys, named once so a shape change is one edit and one grep.
const TPOSTINGS: &str = "tpostings";
const TDESCRIPTION: &str = "tdescription";
const PACCOUNT: &str = "paccount";
const PAMOUNT: &str = "pamount";
const ACOMMODITY: &str = "acommodity";

/// The account suffix hledger gives a record no if-rule matched.
const UNKNOWN_SUFFIX: &str = ":unknown";

/// **Stage 2, first half.** Count the signals in `hledger print -f DATA --rules R -O json`.
///
/// Pure — the caller runs hledger. `expected_commodity` is what amounts *should*
/// be denominated in (from [`PrefilterPass::expected_commodity`], or from the
/// statement's own currency when a format volunteered one); `None` means "not
/// known", and then only genuinely commodity-less amounts are counted.
///
/// The four penalties and the exact JSON they come from — all verified against
/// hledger 1.52, not read from documentation:
///
/// | JSON | Means |
/// | --- | --- |
/// | `pamount: []` | the posting has **no amount**; fact 4 |
/// | `acommodity: ""` | a **bare** amount in a commodity of its own; fact 4 |
/// | `paccount` ends `:unknown` | no if-rule matched the record |
/// | `tdescription` empty | no payee |
///
/// [`Signals::column_count_matches`] and [`Signals::header_matches_source`] come
/// back `false`: hledger's output cannot know them. Merge stage 1's answer with
/// [`Signals::with_prefilter`].
///
/// # Errors
/// [`MatchError`] if the JSON is not the shape `hledger print -O json` produces.
/// Strictly — see the type's own documentation for why a tolerant reader here
/// would recreate the exact bug this module exists to prevent.
pub fn signals_from_hledger_json(
    json: &Value,
    expected_commodity: Option<&str>,
) -> Result<Signals, MatchError> {
    // An expected commodity that is blank is not an expectation. Normalised here
    // so no caller has to remember, and so `Some("")` cannot mean "every amount
    // is in the wrong commodity".
    let expected = expected_commodity.map(str::trim).filter(|c| !c.is_empty());

    json.as_array()
        .ok_or(MatchError::NotTransactions)?
        .iter()
        .try_fold(Signals::default(), |signals, transaction| {
            let postings = transaction
                .get(TPOSTINGS)
                .and_then(Value::as_array)
                .ok_or(MatchError::MalformedTransaction)?;
            // A description that is absent, not a string, or whitespace-only is
            // an empty description. This is the ONE field with a safe default:
            // "no payee" is a real reading of a missing payee, and it lowers the
            // score rather than raising it.
            let empty_description = transaction
                .get(TDESCRIPTION)
                .and_then(Value::as_str)
                .is_none_or(|text| text.trim().is_empty());

            postings.iter().try_fold(
                Signals {
                    txns: signals.txns + 1,
                    empty_descriptions: signals.empty_descriptions + usize::from(empty_description),
                    ..signals
                },
                |signals, posting| add_posting(signals, posting, expected),
            )
        })
}

/// Fold one posting's evidence into `signals`.
fn add_posting(
    signals: Signals,
    posting: &Value,
    expected: Option<&str>,
) -> Result<Signals, MatchError> {
    let account = posting
        .get(PACCOUNT)
        .and_then(Value::as_str)
        .ok_or(MatchError::MalformedPosting)?;
    let amounts = posting
        .get(PAMOUNT)
        .and_then(Value::as_array)
        .ok_or(MatchError::MalformedPosting)?;

    Ok(Signals {
        postings: signals.postings + 1,
        // The empty array IS the signal. See `MatchError::MalformedPosting` for
        // why an absent `pamount` is an error and an empty one is not.
        amountless_postings: signals.amountless_postings + usize::from(amounts.is_empty()),
        bare_commodity_amounts: signals.bare_commodity_amounts
            + amounts
                .iter()
                .filter(|amount| is_off_commodity(amount, expected))
                .count(),
        unknown_accounts: signals.unknown_accounts + usize::from(is_unknown(account)),
        ..signals
    })
}

/// Whether an amount will fail to land in the commodity being imported into.
///
/// An **absent** `acommodity` reads as bare. That is the fail-safe direction: it
/// can only lower a score, and an amount whose commodity we cannot see is
/// precisely an amount we cannot promise will move the user's balance.
fn is_off_commodity(amount: &Value, expected: Option<&str>) -> bool {
    let commodity = amount
        .get(ACOMMODITY)
        .and_then(Value::as_str)
        .unwrap_or_default();
    expected.map_or_else(|| commodity.is_empty(), |expected| commodity != expected)
}

/// Whether an account is hledger's uncategorised fallback.
///
/// Matched on the `:unknown` leaf rather than on the full `expenses:unknown` /
/// `income:unknown`, so a user's own `assets:unknown` default counts too — it
/// means the same thing to the person who has to categorise it later.
fn is_unknown(account: &str) -> bool {
    account.ends_with(UNKNOWN_SUFFIX)
}

// ---------------------------------------------------------------------------
// Stage 2 — the score
// ---------------------------------------------------------------------------

/// **Stage 2, second half.** Reduce [`Signals`] to one number in `0.0..=1.0`.
///
/// # The formula
///
/// ```text
/// quality = (1 - 1.00 * amountless_postings    / txns    )   fact 4: broken
///         * (1 - 1.00 * bare_commodity_amounts / txns    )   fact 4: invisible
///         * (1 - 0.60 * unknown_accounts       / postings)   uncategorised
///         * (1 - 0.20 * empty_descriptions     / txns    )   no payee
///
/// shape   = (1 - 0.15 if the fields list is not as wide as the data)
///         * (1 - 0.05 if the header names nothing the fields list names)
///
/// score   = quality * shape,  capped at 0.25 if either fact-4 count is non-zero
/// ```
///
/// Every ratio is clamped to `1.0`, and `txns == 0` scores zero outright: a rules
/// file that read the statement and produced nothing is not a match at any score.
///
/// # Why these weights
///
/// **Multiplicative, not additive.** Independent failures compound — a file that
/// is both uncategorised *and* half-amountless is worse than either — and a
/// product of factors in `0..=1` cannot leave the range, so the newtype's
/// invariant costs no clamping arithmetic.
///
/// **The two fact-4 signals are measured per transaction, not per posting**,
/// because that is the unit they destroy. One amountless posting does not spoil a
/// posting, it spoils the whole transaction it is in; one bare amount does not
/// hide a posting, it hides the transaction from the balance the user is
/// watching. Measuring them per posting would halve both penalties for the
/// arithmetic reason that transactions usually have two postings, which is not a
/// reason.
///
/// **`unknown_accounts` is 0.6 and per posting**, because it is the one failure
/// that is *recoverable and visible*: the dates and amounts are right, the money
/// is in the right account on one side, and the user fixes it by adding an
/// if-rule. It must rank below a fully categorised file and above a broken one.
///
/// **`empty_descriptions` is 0.2** — a blank payee is a real defect and a real
/// annoyance, but nothing about the transaction is *wrong*.
///
/// **The shape terms are 0.15 and 0.05** — small on purpose. They are corroboration
/// from stage 1, and neither is evidence of an actual defect in the output: naming
/// fewer columns than a file has is normal practice, and naming them for hledger's
/// semantics rather than the bank's is normal practice too. They break ties between
/// candidates that hledger agrees about; they never overturn one.
///
/// **The cap is the policy.** Both fact-4 failures are silent, so rarity is not
/// mitigation. One amountless posting in five hundred still means the rules file
/// is wrong about a row shape, and the user will not notice; uncapped it would
/// rate 0.996 and be offered as the answer. Capped, it can be listed and chosen,
/// but never suggested.
///
/// # Monotonicity
///
/// **Increasing any penalty count, or clearing either shape flag, never raises
/// the score.** Each factor is non-increasing in its own count, the factors are
/// non-negative, so their product is non-increasing in every count; the cap is a
/// `min` against a constant and is itself monotone. Asserted as a property test
/// in `tests/matching.rs` rather than left as an argument.
#[must_use]
pub fn score(signals: &Signals) -> Score {
    if signals.txns == 0 {
        return Score::ZERO;
    }

    let per_txn = |count: usize| ratio(count, signals.txns);
    let per_posting = |count: usize| ratio(count, signals.postings);

    let quality = (1.0 - W_AMOUNTLESS * per_txn(signals.amountless_postings))
        * (1.0 - W_BARE_COMMODITY * per_txn(signals.bare_commodity_amounts))
        * (1.0 - W_UNKNOWN_ACCOUNT * per_posting(signals.unknown_accounts))
        * (1.0 - W_EMPTY_DESCRIPTION * per_txn(signals.empty_descriptions));

    let shape = penalty(signals.column_count_matches, W_COLUMN_COUNT)
        * penalty(signals.header_matches_source, W_HEADER);

    let scored = quality * shape;
    let fatal = signals.amountless_postings > 0 || signals.bare_commodity_amounts > 0;
    Score::new(if fatal { scored.min(FATAL_CAP) } else { scored })
}

/// `count / total`, clamped to `0.0..=1.0`.
///
/// A zero `total` reads as `count / 1`, so a count with nothing to divide by
/// saturates rather than producing an infinity — which would make the whole
/// product `NaN` and, without [`Score::new`]'s guard, an unorderable score.
fn ratio(count: usize, total: usize) -> f32 {
    (count as f32 / total.max(1) as f32).clamp(0.0, 1.0)
}

/// `1.0` when the flag holds, `1 - weight` when it does not.
fn penalty(holds: bool, weight: f32) -> f32 {
    if holds { 1.0 } else { 1.0 - weight }
}

// ---------------------------------------------------------------------------
// Unit tests — the pure helpers. The fixtures, the goldens and the properties
// are exercised in `tests/matching.rs`.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn parses(format: &str, value: &str) -> Option<bool> {
        Some(count_parsable(format, &[value.to_string()])? == 1)
    }

    #[test]
    fn the_common_date_formats_read_their_own_dates() {
        assert_eq!(parses("%m/%d/%Y", "01/15/2024"), Some(true));
        assert_eq!(parses("%Y-%m-%d", "2024-02-03"), Some(true));
        assert_eq!(parses("%d.%m.%Y", "15.01.2024"), Some(true));
        assert_eq!(parses("%-m/%-d/%Y", "1/5/2024"), Some(true));
        assert_eq!(parses("%d/%b/%Y", "15/Jan/2024"), Some(true));
        assert_eq!(parses("%B %d, %Y", "January 15, 2024"), Some(true));
        assert_eq!(parses("%F", "2024-01-15"), Some(true));
        assert_eq!(parses("%Y%m%d", "20240115"), Some(true));
        assert_eq!(
            parses("%Y-%m-%d %H:%M:%S", "2024-01-15 09:30:00"),
            Some(true)
        );
    }

    #[test]
    fn a_wrong_format_is_rejected_by_separator_or_by_range() {
        // The literal separators disagree — the `wrong-dateformat.rules` case.
        assert_eq!(parses("%d.%m.%Y", "01/15/2024"), Some(false));
        // The separators agree and the RANGE does not: day/month reversed puts
        // 15 in the month slot.
        assert_eq!(parses("%d/%m/%Y", "01/15/2024"), Some(false));
        assert_eq!(parses("%Y-%m-%d", "01/15/2024"), Some(false));
        // Trailing text is not silently discarded.
        assert_eq!(parses("%Y-%m-%d", "2024-01-15T09:30:00Z"), Some(false));
        assert_eq!(parses("%m/%d/%Y", ""), Some(false));
        assert_eq!(parses("%d/%b/%Y", "15/Smarch/2024"), Some(false));
    }

    #[test]
    fn an_ambiguous_date_parses_under_both_readings() {
        // The case stage 1 must NOT try to resolve: both are genuinely valid and
        // picking one would be guessing. Both survive to stage 2.
        assert_eq!(parses("%m/%d/%Y", "01/02/2024"), Some(true));
        assert_eq!(parses("%d/%m/%Y", "01/02/2024"), Some(true));
    }

    #[test]
    fn an_unmodelled_specifier_declines_to_answer_rather_than_rejecting() {
        // `None` is what makes the caller skip the check. A `Some(false)` here
        // would silently drop the user's correct rules file.
        assert_eq!(
            parses("%Y-%m-%dT%H:%M:%S%Z", "2024-01-15T09:30:00UTC"),
            None
        );
        assert_eq!(parses("%s", "1705276800"), None);
        assert_eq!(parses("%j %Y", "015 2024"), None);
        assert_eq!(parses("%", ""), None, "a trailing bare % is not a format");
    }

    #[test]
    fn whitespace_is_tolerated_around_and_inside_a_date() {
        assert_eq!(parses("%Y-%m-%d", "  2024-01-15  "), Some(true));
        assert_eq!(parses("%b %e %Y", "Jan  5 2024"), Some(true));
        assert_eq!(parses("%%%Y", "%2024"), Some(true));
    }

    #[test]
    fn the_modal_width_ignores_one_ragged_trailer() {
        assert_eq!(modal_width(&[4, 4, 4, 4, 1]), 4);
        assert_eq!(modal_width(&[]), 0);
        assert_eq!(modal_width(&[3]), 3);
        // A tie goes to the wider reading, so row order cannot change the answer.
        assert_eq!(modal_width(&[3, 5]), 5);
        assert_eq!(modal_width(&[5, 3]), 5);
    }

    #[test]
    fn names_compare_on_letters_and_digits_only() {
        assert_eq!(normalize_name("Amount In"), "amountin");
        assert_eq!(normalize_name("amount-in"), "amountin");
        assert_eq!(normalize_name("AMOUNT_IN"), "amountin");
        assert_eq!(normalize_name(""), "");
        assert_eq!(normalize_name("  "), "");
    }

    #[test]
    fn an_ignored_column_names_nothing() {
        let fields = vec![String::new(), "date".to_string()];
        let header = vec!["Date".to_string(), "Other".to_string()];
        assert!(header_overlaps(Some(&fields), Some(&header)));
        // An empty `fields` entry must not match an empty header cell, or a file
        // with one ignored column would corroborate every header on earth.
        let only_ignored = vec![String::new()];
        let blank_header = vec![String::new()];
        assert!(!header_overlaps(Some(&only_ignored), Some(&blank_header)));
        assert!(!header_overlaps(None, Some(&header)));
        assert!(!header_overlaps(Some(&fields), None));
    }

    #[test]
    fn unknown_accounts_are_matched_on_the_leaf() {
        assert!(is_unknown("expenses:unknown"));
        assert!(is_unknown("income:unknown"));
        assert!(is_unknown("assets:bank:unknown"));
        assert!(!is_unknown("expenses:unknowable"));
        assert!(!is_unknown("expenses:food"));
        assert!(!is_unknown(""));
    }

    #[test]
    fn a_score_is_clamped_and_never_nan() {
        assert_eq!(Score::new(f32::NAN), Score::ZERO);
        assert_eq!(Score::new(-1.0).value(), 0.0);
        assert_eq!(Score::new(9.0).value(), 1.0);
        assert!(Score::new(1.0) > Score::new(0.5));
    }

    #[test]
    fn a_ratio_saturates_rather_than_dividing_by_zero() {
        assert_eq!(ratio(0, 0), 0.0);
        assert_eq!(ratio(3, 0), 1.0, "not an infinity, and not a NaN");
        assert_eq!(ratio(1, 2), 0.5);
        assert_eq!(ratio(9, 2), 1.0);
    }
}
