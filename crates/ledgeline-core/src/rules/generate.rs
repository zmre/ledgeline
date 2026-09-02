//! Drafting a **new** `*.rules` file from a CSV that has none (WP-16 Phase 2).
//!
//! Every other module in `rules/` starts from a file the user already has.
//! This one starts from their *data*: a [`Tabular`] the `convert` lane already
//! produced, and one account name only the user can supply. What comes back is
//! a [`RulesDoc`] the editor can open, correct and save through the routes that
//! already exist — a **starting point**, never an answer.
//!
//! # It drafts; it does not decide
//!
//! Column mapping is a guess, and a wrong one is a real failure mode: hledger
//! accepts an unrecognised `fields` name in silence (verified against 1.52 — a
//! misspelled `descriptoin` produces blank descriptions, exit 0, empty stderr),
//! so nothing downstream would catch it. Two consequences shape this module:
//!
//! - **A column this module cannot map is not guessed at.** It comes back with
//!   `field: None` and a confidence of 0, keeps its own header as a plain
//!   `fields` name so a later rule can still interpolate it (`%fitid`), and the
//!   caller shows both. [`ColumnGuess::confidence`] exists so a UI can mark the
//!   uncertain ones rather than presenting every guess as a fact.
//! - **Nothing here writes to disk**, opens a file, or knows a path. Same rule
//!   `convert` holds to, and for the same reason: the write is a separate,
//!   separately-testable operation.
//!
//! # What the draft describes is the CONVERTED CSV, not the download
//!
//! This is the fact most likely to be "fixed" into a bug by a later reader, so
//! it is stated first. The file this rules file will be used with is
//! `convert::to_csv`'s output — comma-separated, UTF-8, header on line 1, with
//! the preamble and trailer already stripped (`Stage::data`, and the copy a
//! commit writes to the user's destination). Therefore:
//!
//! | Directive | Why it is NOT emitted |
//! | --- | --- |
//! | `separator` | [`crate::convert::to_csv`] always writes commas. A `separator ;` copied off the original download would describe a file that no longer exists. |
//! | `encoding` | The converted CSV is UTF-8 whatever the download was. `encoding windows-1252` would make hledger mis-decode a UTF-8 file — the note the conversion left is about bytes this rules file will never see. |
//! | `skip N > 1` | `ConvertNote::PreambleSkipped` describes lines already gone. The converted file's only non-data record is its header, so the answer is always `skip 1`. See `docs/imports.md` § *The `skip` a rules file already says* for the other half of this: `align_to_skip` returns a `skip 1` file byte for byte, so nothing about the common path changes. |
//!
//! # hledger facts this module is built on
//!
//! All verified against the 1.52 binary (`hledger --no-conf -f DATA.csv
//! --rules-file R.rules print`), not read from the manual:
//!
//! - **`%-m`/`%-d` parse a superset of `%m`/`%d`.** A padded specifier *rejects*
//!   an unpadded value (`%m/%d/%Y` on `1/2/2026` is exit 1), while the relaxed
//!   form reads both. hledger's own error message recommends the relaxed form.
//!   So [`guess_date_format`] emits the relaxed spelling the moment any sample
//!   is unpadded, and the clean padded one when none is — which keeps the common
//!   case readable and the messy case working.
//! - **A format must consume the whole value.** `%Y-%m-%d` does not truncate
//!   `2026-01-02T13:45:00`; the time specifiers have to be in the pattern.
//! - **With no `date-format` at all hledger reads year-first dates only**
//!   (`YYYY/M/D`, `YYYY-M-D`, `YYYY.M.D`). Anything else needs the directive, so
//!   a date column we cannot recognise is a **warning**, never a silent omission.
//! - **`currency` is a blind string prefix, not "set the commodity".**
//!   `currency $` over a cell already reading `$-4.50` produces `$$-4.50` — a
//!   real, distinct commodity, exit 0, no warning. So the directive is emitted
//!   **only** when no sample carries a symbol of its own *and* the statement
//!   format volunteered one.
//! - **A lone `,` with no `decimal-mark` is read as a DECIMAL POINT.** `1,234`
//!   becomes `1.234`, and `print` re-renders it as `1,234`, so the 1000× error
//!   is invisible in hledger's own output. This is why [`guess_decimal_mark`]
//!   exists at all and why it emits a directive that is a no-op in the easy
//!   cases: the case it is for is the one nothing else can catch.
//! - **`amount-out` is negated; `amount-in` is not.** Both set non-zero on one
//!   row is a hard error, so at most one amount scheme is ever mapped.
//! - **A `status` column is deliberately never mapped.** hledger's `status`
//!   field wants `*`/`!`/empty; a bank's `Posted`/`Pending` column mapped to it
//!   is a broken journal, and "Status" is a very common header.
//!
//! # The draft contains no comments, and that is load-bearing
//!
//! A generated file would read better with a `# written by Ledgeline` header.
//! It cannot have one. [`crate::rules::ItemBody`] has no comment variant — by
//! design, since it is what stops a client smuggling raw text into a rules file
//! — so a comment line would come back as an `ItemKind::Trivia` item that the
//! save wire can only express as `{kind:"keep", id}`, and there is no file yet
//! to keep bytes *from*. Every item in a draft is therefore one the create
//! `PUT` can name. `every_drafted_item_can_be_written_back` pins it.

use crate::convert::Tabular;
use crate::rules::matching;
use crate::rules::{
    ControlField, DirectiveName, DirectiveValue, EditPlan, HledgerField, ItemBody, MatchScope,
    MatcherGroupSpec, MatcherSpec, RulesDoc, RulesError, Slot, field_name_text, hledger_field,
};

// ---------------------------------------------------------------------------
// Budgets
// ---------------------------------------------------------------------------

/// How many rows are read when sampling a column.
///
/// Enough that a header row of blanks or a single odd first transaction cannot
/// decide a whole file's mapping, small enough that this stays a scan of the top
/// of the table rather than of a year of statements.
const MAX_SAMPLES: usize = 24;

/// How many columns are examined. Past this a CSV is not something a mapping UI
/// can lay out anyway, and `Discovery`'s own preview stops at the same width.
const MAX_COLUMNS: usize = 64;

/// The `account2` every draft carries.
///
/// Deliberately dumb. Guessing a category from the payee is a **separate**,
/// already-tracked piece of work (`TODO.md`, "intelligent category suggestions")
/// and is explicitly out of scope here — see `plans/16-import-rules-enhancements-ii.md`.
/// An obvious fallback is also what makes the unrecognised rows easy to find
/// afterwards, which is the whole reason this value is boring.
pub const DEFAULT_ACCOUNT2: &str = "expenses:unknown";

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// What one CSV column was read as.
///
/// `field: None` is not a failure — it is the honest answer, and `name` still
/// carries the column's own header so a later rule can interpolate it.
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnGuess {
    /// The column's 0-based position.
    pub index: usize,
    /// The hledger field this column assigns, or `None` when nothing was
    /// confident enough to claim it.
    pub field: Option<HledgerField>,
    /// `0.0..=1.0`. A confidence, not a probability: it orders guesses and marks
    /// the shaky ones, and nothing computes with it.
    pub confidence: f32,
    /// What goes in the `fields` list for this column: the hledger field name
    /// when one was claimed, else a plain name derived from the header, else
    /// empty (hledger's own "ignore this column").
    pub name: String,
}

/// A `date-format` value, and how sure we are of it.
#[derive(Debug, Clone, PartialEq)]
pub struct DateFormatGuess {
    /// The directive's value, e.g. `%-m/%-d/%Y`. Never carries trailing
    /// whitespace: hledger does not trim the format string, so a trailing space
    /// makes the pattern **unsatisfiable** (verified — the value is trimmed and
    /// the format is not, so nothing can ever match).
    pub format: String,
    pub confidence: f32,
    /// Another format in the catalogue reads every sample too — `01/02/2026` is
    /// the classic. The caller must say so; picking one silently is how March
    /// transactions end up filed in December.
    pub ambiguous: bool,
}

/// A drafted rules file plus everything the caller has to show alongside it.
#[derive(Debug, Clone)]
pub struct Draft {
    /// The document itself, already parsed — so it can be rendered through the
    /// same wire projection `GET /api/rules/{*id}` uses.
    pub doc: RulesDoc,
    /// One entry per column of the table, in column order.
    pub columns: Vec<ColumnGuess>,
    /// The `date-format` this draft carries, or `None` when no format in the
    /// catalogue read every sample.
    pub date_format: Option<DateFormatGuess>,
    /// What the user has to be told, in sentences. Never a refusal — a draft is
    /// produced whatever these say.
    pub warnings: Vec<String>,
}

// ---------------------------------------------------------------------------
// Column mapping
// ---------------------------------------------------------------------------

/// A header synonym and what it means, matched on letters and digits only.
///
/// Ordered longest-first *within* a field so `posteddate` cannot be reached by
/// the `date` entry first; the lookup below is an exact match on the normalised
/// name, then a containment pass, so the order across fields is what breaks a
/// tie between two fields that both contain the same word.
struct Synonym {
    /// The normalised header text ([`normalize`]).
    name: &'static str,
    field: HledgerField,
    /// The confidence an EXACT match earns. A containment match earns less.
    confidence: f32,
}

/// The synonym table.
///
/// Deliberately not exhaustive and deliberately conservative. Every entry here
/// was chosen because a real export writes it and because getting it wrong is
/// visible in the mapping table the user is looking at. Three families are
/// **absent on purpose**:
///
/// - **`status`** — see the module docs. A `Posted`/`Pending` column mapped to
///   hledger's `status` field is a journal that will not parse.
/// - **`reference` / `id` / `fitid`** — plausible `code` columns, but `code`
///   puts the value in parentheses on every description line, and a bank
///   reference number is noise there. They stay unmapped-but-named, so
///   `%fitid` is available to a rule the user writes later.
/// - **`payment`** — means money *out* on a bank statement and money *in* on a
///   credit-card one. A synonym that is backwards half the time is worse than
///   no synonym.
const SYNONYMS: &[Synonym] = &[
    // --- date ------------------------------------------------------------
    Synonym {
        name: "date",
        field: HledgerField::Date,
        confidence: 1.0,
    },
    Synonym {
        name: "transactiondate",
        field: HledgerField::Date,
        confidence: 0.95,
    },
    Synonym {
        name: "posteddate",
        field: HledgerField::Date,
        confidence: 0.95,
    },
    Synonym {
        name: "dateposted",
        field: HledgerField::Date,
        confidence: 0.95,
    },
    Synonym {
        name: "postingdate",
        field: HledgerField::Date,
        confidence: 0.95,
    },
    Synonym {
        name: "bookingdate",
        field: HledgerField::Date,
        confidence: 0.9,
    },
    Synonym {
        name: "trandate",
        field: HledgerField::Date,
        confidence: 0.9,
    },
    Synonym {
        name: "posted",
        field: HledgerField::Date,
        confidence: 0.85,
    },
    Synonym {
        name: "activitydate",
        field: HledgerField::Date,
        confidence: 0.85,
    },
    Synonym {
        name: "tradedate",
        field: HledgerField::Date,
        confidence: 0.85,
    },
    // --- date2 -----------------------------------------------------------
    Synonym {
        name: "date2",
        field: HledgerField::Date2,
        confidence: 1.0,
    },
    Synonym {
        name: "valuedate",
        field: HledgerField::Date2,
        confidence: 0.8,
    },
    Synonym {
        name: "effectivedate",
        field: HledgerField::Date2,
        confidence: 0.8,
    },
    Synonym {
        name: "settlementdate",
        field: HledgerField::Date2,
        confidence: 0.8,
    },
    // --- description -----------------------------------------------------
    Synonym {
        name: "description",
        field: HledgerField::Description,
        confidence: 1.0,
    },
    Synonym {
        name: "payee",
        field: HledgerField::Description,
        confidence: 0.95,
    },
    Synonym {
        name: "merchant",
        field: HledgerField::Description,
        confidence: 0.9,
    },
    Synonym {
        name: "narration",
        field: HledgerField::Description,
        confidence: 0.9,
    },
    Synonym {
        name: "memo",
        field: HledgerField::Description,
        confidence: 0.85,
    },
    Synonym {
        name: "originaldescription",
        field: HledgerField::Description,
        confidence: 0.85,
    },
    Synonym {
        name: "transactiondescription",
        field: HledgerField::Description,
        confidence: 0.85,
    },
    Synonym {
        name: "particulars",
        field: HledgerField::Description,
        confidence: 0.8,
    },
    Synonym {
        name: "details",
        field: HledgerField::Description,
        confidence: 0.75,
    },
    Synonym {
        name: "name",
        field: HledgerField::Description,
        confidence: 0.7,
    },
    // --- amount ----------------------------------------------------------
    Synonym {
        name: "amount",
        field: HledgerField::Amount,
        confidence: 1.0,
    },
    Synonym {
        name: "transactionamount",
        field: HledgerField::Amount,
        confidence: 0.95,
    },
    Synonym {
        name: "netamount",
        field: HledgerField::Amount,
        confidence: 0.9,
    },
    Synonym {
        name: "amt",
        field: HledgerField::Amount,
        confidence: 0.85,
    },
    Synonym {
        name: "value",
        field: HledgerField::Amount,
        confidence: 0.75,
    },
    // --- amount-in / amount-out ------------------------------------------
    Synonym {
        name: "amountin",
        field: HledgerField::AmountIn,
        confidence: 1.0,
    },
    Synonym {
        name: "credit",
        field: HledgerField::AmountIn,
        confidence: 0.9,
    },
    Synonym {
        name: "creditamount",
        field: HledgerField::AmountIn,
        confidence: 0.9,
    },
    Synonym {
        name: "deposit",
        field: HledgerField::AmountIn,
        confidence: 0.9,
    },
    Synonym {
        name: "deposits",
        field: HledgerField::AmountIn,
        confidence: 0.9,
    },
    Synonym {
        name: "moneyin",
        field: HledgerField::AmountIn,
        confidence: 0.9,
    },
    Synonym {
        name: "paidin",
        field: HledgerField::AmountIn,
        confidence: 0.9,
    },
    Synonym {
        name: "inflow",
        field: HledgerField::AmountIn,
        confidence: 0.85,
    },
    Synonym {
        name: "amountout",
        field: HledgerField::AmountOut,
        confidence: 1.0,
    },
    Synonym {
        name: "debit",
        field: HledgerField::AmountOut,
        confidence: 0.9,
    },
    Synonym {
        name: "debitamount",
        field: HledgerField::AmountOut,
        confidence: 0.9,
    },
    Synonym {
        name: "withdrawal",
        field: HledgerField::AmountOut,
        confidence: 0.9,
    },
    Synonym {
        name: "withdrawals",
        field: HledgerField::AmountOut,
        confidence: 0.9,
    },
    Synonym {
        name: "moneyout",
        field: HledgerField::AmountOut,
        confidence: 0.9,
    },
    Synonym {
        name: "paidout",
        field: HledgerField::AmountOut,
        confidence: 0.9,
    },
    Synonym {
        name: "outflow",
        field: HledgerField::AmountOut,
        confidence: 0.85,
    },
    // --- balance ---------------------------------------------------------
    Synonym {
        name: "balance",
        field: HledgerField::Balance,
        confidence: 1.0,
    },
    Synonym {
        name: "runningbalance",
        field: HledgerField::Balance,
        confidence: 0.95,
    },
    Synonym {
        name: "accountbalance",
        field: HledgerField::Balance,
        confidence: 0.9,
    },
    Synonym {
        name: "endingbalance",
        field: HledgerField::Balance,
        confidence: 0.9,
    },
    Synonym {
        name: "balanceamount",
        field: HledgerField::Balance,
        confidence: 0.9,
    },
    // --- the rest --------------------------------------------------------
    Synonym {
        name: "code",
        field: HledgerField::Code,
        confidence: 1.0,
    },
    Synonym {
        name: "checknumber",
        field: HledgerField::Code,
        confidence: 0.85,
    },
    Synonym {
        name: "chequenumber",
        field: HledgerField::Code,
        confidence: 0.85,
    },
    Synonym {
        name: "checkno",
        field: HledgerField::Code,
        confidence: 0.85,
    },
    Synonym {
        name: "comment",
        field: HledgerField::Comment,
        confidence: 1.0,
    },
    Synonym {
        name: "notes",
        field: HledgerField::Comment,
        confidence: 0.8,
    },
    Synonym {
        name: "note",
        field: HledgerField::Comment,
        confidence: 0.8,
    },
    Synonym {
        name: "currency",
        field: HledgerField::Currency,
        confidence: 1.0,
    },
    Synonym {
        name: "currencycode",
        field: HledgerField::Currency,
        confidence: 0.95,
    },
    Synonym {
        name: "commodity",
        field: HledgerField::Currency,
        confidence: 0.85,
    },
    Synonym {
        name: "ccy",
        field: HledgerField::Currency,
        confidence: 0.85,
    },
];

/// Lowercase, letters and digits only — the same normalisation
/// [`matching`](crate::rules::matching) compares header names with, so
/// `Amount In`, `amount-in` and `AMOUNT_IN` are one name here too.
fn normalize(name: &str) -> String {
    name.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// The best synonym for one header cell: an exact normalised match, else the
/// longest contained synonym.
///
/// Containment is what reads `Transaction Date (local)` as a date without a
/// table entry for every parenthetical a bank invents. It costs confidence
/// because it is a weaker claim, and the **longest** match wins so
/// `Posted Date` is a date rather than being read through a shorter entry.
fn match_header(header: &str) -> Option<(HledgerField, f32)> {
    let normalized = normalize(header);
    if normalized.is_empty() {
        return None;
    }
    if let Some(exact) = SYNONYMS.iter().find(|entry| entry.name == normalized) {
        return Some((exact.field, exact.confidence));
    }
    SYNONYMS
        .iter()
        .filter(|entry| normalized.contains(entry.name))
        .max_by_key(|entry| entry.name.len())
        // A weaker claim than an exact match, and scaled by the entry's own
        // confidence so a shaky synonym found inside a longer header is shakier
        // still.
        .map(|entry| (entry.field, entry.confidence * 0.7))
}

/// A column's first few non-empty values, trimmed.
fn samples(rows: &[Vec<String>], index: usize) -> Vec<String> {
    rows.iter()
        .filter_map(|row| row.get(index))
        .map(|cell| cell.trim().to_string())
        .filter(|cell| !cell.is_empty())
        .take(MAX_SAMPLES)
        .collect()
}

/// Does every sample read as a date under some catalogue format?
fn looks_like_dates(values: &[String]) -> bool {
    !values.is_empty() && best_format(values).is_some()
}

/// Does every sample read as an amount — digits, with at most the punctuation a
/// money column carries?
///
/// Deliberately strict about what may surround the digits: a description column
/// holding `INV 12345` must not read as an amount, and a date column must not
/// either (`/` and `-` between digit runs are excluded by requiring a single
/// numeric body).
fn looks_like_amounts(values: &[String]) -> bool {
    !values.is_empty() && values.iter().all(|value| amount_body(value).is_some())
}

/// The digits-and-separators core of an amount cell, with sign, parentheses,
/// spaces and any currency symbol stripped — or `None` when what is left is not
/// one number.
fn amount_body(value: &str) -> Option<String> {
    let trimmed = value.trim();
    let unwrapped = trimmed
        .strip_prefix('(')
        .and_then(|rest| rest.strip_suffix(')'))
        .unwrap_or(trimmed);
    let body: String = unwrapped
        .chars()
        .filter(|c| !c.is_whitespace() && !matches!(c, '+' | '-'))
        .collect();
    // Whatever is left must be digits and separators only, with at least one
    // digit, once a leading or trailing symbol run has been taken off.
    let core = body
        .trim_start_matches(|c: char| !c.is_ascii_digit() && c != '.' && c != ',')
        .trim_end_matches(|c: char| !c.is_ascii_digit() && c != '.' && c != ',');
    if core.is_empty() || !core.chars().any(|c| c.is_ascii_digit()) {
        return None;
    }
    core.chars()
        .all(|c| c.is_ascii_digit() || c == '.' || c == ',')
        .then(|| core.to_string())
}

/// Score every column of `header` + `sample_rows` against hledger's field
/// vocabulary.
///
/// Pure. The header is consulted first (it is what the user themselves reads),
/// and a column whose header says nothing falls back to what its **values** look
/// like — which is the only thing available for the headerless exports some
/// brokerages still write.
///
/// One field is claimed by at most one column: where two columns want the same
/// field the more confident one wins and the other becomes unmapped-but-named,
/// because two columns named `amount` would be resolved by hledger as
/// "first one wins", silently (verified).
#[must_use]
pub fn guess_columns(header: &[String], sample_rows: &[Vec<String>]) -> Vec<ColumnGuess> {
    let width = header
        .len()
        .max(sample_rows.iter().map(Vec::len).max().unwrap_or(0))
        .min(MAX_COLUMNS);

    // Pass 1 — every column's best claim, independent of every other column's.
    let claims: Vec<(Option<HledgerField>, f32)> = (0..width)
        .map(|index| {
            let cells = samples(sample_rows, index);
            match header.get(index).and_then(|name| match_header(name)) {
                Some((field, confidence)) => (Some(field), confidence),
                // No header, or a header nothing recognised: ask the data. A
                // date column is unmistakable and an amount column nearly so,
                // and both are worth strictly less than a header that said it.
                None if looks_like_dates(&cells) => (Some(HledgerField::Date), 0.5),
                None if looks_like_amounts(&cells) => (Some(HledgerField::Amount), 0.35),
                None => (None, 0.0),
            }
        })
        .collect();

    // Pass 2 — resolve the conflicts, highest confidence first, ties by
    // position so the answer does not depend on iteration order.
    let mut order: Vec<usize> = (0..width).collect();
    order.sort_by(|&a, &b| {
        claims[b]
            .1
            .partial_cmp(&claims[a].1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.cmp(&b))
    });
    let mut taken: Vec<HledgerField> = Vec::new();
    let mut resolved: Vec<(Option<HledgerField>, f32)> = vec![(None, 0.0); width];
    for index in order {
        let (field, confidence) = claims[index];
        let Some(field) = field else {
            continue;
        };
        // A second date column is `date2` rather than nothing. Card exports
        // routinely carry both a transaction date and a posted date, and
        // hledger has a field for exactly that — so the runner-up is demoted
        // rather than discarded. The demotion is one step and applies to dates
        // only: there is no `amount2` that means "the other amount", and
        // inventing one would be this module deciding rather than drafting.
        let placed = [field, HledgerField::Date2]
            .into_iter()
            .take(if field == HledgerField::Date { 2 } else { 1 })
            .find(|candidate| !taken.contains(candidate));
        match placed {
            Some(placed) => {
                taken.push(placed);
                resolved[index] = (
                    Some(placed),
                    if placed == field {
                        confidence
                    } else {
                        confidence * 0.8
                    },
                );
            }
            // A more confident column already holds this field, and there is no
            // second home for it. Unmapped-but-named.
            None => resolved[index] = (None, 0.0),
        }
    }

    // Pass 2b — ONE amount scheme, never both. A record carrying a value in
    // `amount` and in `amount-in`/`amount-out` at once is a hard hledger error
    // ("Multiple non-zero amounts were assigned for an amount field", verified
    // against 1.52), and a file offering all three columns is ordinary: a bank
    // that writes a signed total *and* separate debit/credit columns. Pass 2
    // cannot see this, because the three are different fields and so never
    // collide with each other.
    let single = resolved
        .iter()
        .position(|(field, _)| *field == Some(HledgerField::Amount));
    let split: Vec<usize> = resolved
        .iter()
        .enumerate()
        .filter(|(_, (field, _))| {
            matches!(
                field,
                Some(HledgerField::AmountIn | HledgerField::AmountOut)
            )
        })
        .map(|(index, _)| index)
        .collect();
    if let Some(single) = single
        && !split.is_empty()
    {
        let split_confidence = split
            .iter()
            .map(|&index| resolved[index].1)
            .fold(0.0_f32, f32::max);
        // The signed total wins a tie: it is one column to check rather than
        // two, and `amount` is the spelling the rest of this feature assumes.
        if resolved[single].1 >= split_confidence {
            for index in split {
                resolved[index] = (None, 0.0);
            }
        } else {
            resolved[single] = (None, 0.0);
        }
    }

    // Pass 3 — names. A mapped column is named for its field; an unmapped one
    // keeps a plain form of its own header, so `%fitid` stays reachable. A name
    // that would collide with one already used is dropped rather than
    // duplicated: hledger resolves duplicate `fields` names as "first one
    // wins", in silence.
    let mut used: Vec<String> = Vec::new();
    (0..width)
        .map(|index| {
            let (field, confidence) = resolved[index];
            let name = match field {
                Some(field) => field_name_text(field),
                None => header
                    .get(index)
                    .map(|text| normalize(text))
                    .filter(|name| !name.is_empty())
                    // A bare header that IS an hledger field name but was not
                    // claimed above (a losing duplicate) must not be written as
                    // one — it would assign the field after all.
                    .filter(|name| hledger_field(name).is_none())
                    .unwrap_or_default(),
            };
            let name = if name.is_empty() || used.contains(&name) {
                String::new()
            } else {
                used.push(name.clone());
                name
            };
            ColumnGuess {
                index,
                field,
                confidence,
                name,
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Isolated cells: a report's litter, left in the data area
// ---------------------------------------------------------------------------

/// The most isolated rows a draft will write exclusions for.
///
/// Past this the answer stops being "an anomaly" and starts being "the file's
/// shape", and drafting exclusions across a large part of the data is a
/// different and much riskier act than leaving out one or two clear outliers.
/// Eight is also about as many row numbers as a warning can name and still be
/// checkable at a glance, which is the same limit from the other side.
const MAX_ISOLATED_ROWS: usize = 8;

/// How many rows the table must carry for each isolated row found.
///
/// Four, i.e. at most a quarter of the table. Together with
/// [`MAX_ISOLATED_ROWS`] this only bites on tables under thirty-odd rows —
/// exactly where a wrong guess is proportionally worst, and where the user can
/// see the whole file anyway.
const ROWS_PER_ISOLATED_ROW: usize = 4;

/// The narrowest table in which one populated cell says anything.
///
/// `single-column.xlsx`'s lesson from `fixtures/import/README.md`, in another
/// lane: in a one-wide table *every* row holds exactly one populated cell, and
/// in a two-wide one a populated cell is half the record. A statement carries a
/// date, a payee and an amount, so three is where the signal starts.
const MIN_ISOLATION_WIDTH: usize = 3;

/// The longest a cell may be when a warning quotes it back.
const MAX_WARNING_CELL_CHARS: usize = 40;

/// One column holding cells that are not data, and the rows they sit in.
///
/// Grouped by **column** because that is the unit of the fix: a single
/// `if %name .` block excludes every one of `rows` at once, so a report that
/// left three section labels in one column is one finding and one rule rather
/// than three of each.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IsolatedColumn {
    /// The column's 0-based position.
    pub column: usize,
    /// The 0-based indices into `Tabular::rows` of the rows whose only
    /// populated cell is in this column. Ascending, and never empty.
    pub rows: Vec<usize>,
}

/// The index of a row's only populated cell, when it has exactly one.
///
/// Whitespace-only is empty, which is not a detail: hledger trims a field before
/// matching it (verified against 1.52 — a cell holding one space does **not**
/// match `.`), so reading a blank-but-not-empty cell as populated here would
/// draft a rule that then failed to exclude the row it was written for.
fn lone_cell(row: &[String]) -> Option<usize> {
    let mut populated = row
        .iter()
        .enumerate()
        .filter(|(_, cell)| !cell.trim().is_empty());
    let (index, _) = populated.next()?;
    populated.next().is_none().then_some(index)
}

/// The columns whose only content is a cell that cannot be part of a record.
///
/// Pure. This exists because of a real report: a spreadsheet exported from
/// accounting software whose header row was followed by one row holding the
/// words `General Ledger` in a column with no header and nothing else in it
/// anywhere. Every heuristic below sampled that row along with the data, and
/// hledger — handed a record with no date — **abandoned the entire import**
/// rather than skipping the row (verified against 1.52).
///
/// # Two conditions, and neither is safe on its own
///
/// A cell is isolated only when **both** hold:
///
/// 1. **Its row holds nothing else.** Exactly one populated cell in the whole
///    row — not "few", not "mostly blank". This is the condition that saves a
///    legitimately sometimes-blank field: a check-number column populated only
///    on check payments is sparse, but those rows still carry a date, a payee
///    and an amount, so they are not isolated and nothing excludes them. Keying
///    on column sparsity alone would silently drop every check the user wrote.
/// 2. **Its column holds nothing else.** Every populated cell in that column
///    sits in a row that is itself condition-1 isolated. "Nothing else" is
///    literal — one real value anywhere in the column disqualifies the whole
///    column, and with it every row in it. There is no fudge factor and
///    deliberately none: the claim being made is "this column is never used for
///    data", and a single counter-example is a refutation, not noise. What
///    would have wanted slack — a report that left several labels in one column
///    — needs none, because those rows are themselves isolated and so are not
///    "else".
///
/// Together they are narrow. Apart, either is dangerous.
///
/// # Why a false positive cannot cost a transaction
///
/// An isolated row's one populated cell sits in a column that holds nothing in
/// any other row — so that column is neither the date column nor the amount
/// column, both of which are populated on every real record. An isolated row
/// therefore has no date and no amount, and is a row hledger would refuse (or
/// import as an amountless posting) rather than one carrying money. The caps
/// below guard against a degenerate *table shape*, not against losing data.
///
/// # The caps
///
/// [`MIN_ISOLATION_WIDTH`], [`MAX_ISOLATED_ROWS`] and
/// [`ROWS_PER_ISOLATED_ROW`], all three of which must pass or the answer is
/// the empty list. The refusal is silent on purpose: there is no confident
/// sentence to write about a file whose shape this is, and the module's
/// warnings are for things it *knows*.
#[must_use]
pub fn isolated_columns(tabular: &Tabular) -> Vec<IsolatedColumn> {
    let width = tabular
        .header
        .as_ref()
        .map_or(0, Vec::len)
        .max(tabular.rows.iter().map(Vec::len).max().unwrap_or(0));
    if width < MIN_ISOLATION_WIDTH {
        return Vec::new();
    }
    // Condition 1, for every row at once: where its only populated cell is.
    let lone: Vec<Option<usize>> = tabular.rows.iter().map(|row| lone_cell(row)).collect();

    let found: Vec<IsolatedColumn> = (0..width)
        .filter_map(|column| {
            let rows: Vec<usize> = lone
                .iter()
                .enumerate()
                .filter(|(_, at)| **at == Some(column))
                .map(|(row, _)| row)
                .collect();
            if rows.is_empty() {
                return None;
            }
            // Condition 2. "Every other row" means every row that is not one of
            // these, which is what lets several labels in one column count as
            // one finding instead of disqualifying each other.
            let unused_elsewhere = tabular.rows.iter().enumerate().all(|(row, cells)| {
                lone[row] == Some(column)
                    || cells.get(column).is_none_or(|cell| cell.trim().is_empty())
            });
            unused_elsewhere.then_some(IsolatedColumn { column, rows })
        })
        .collect();

    let total: usize = found.iter().map(|found| found.rows.len()).sum();
    if total > MAX_ISOLATED_ROWS || total * ROWS_PER_ISOLATED_ROW > tabular.rows.len() {
        return Vec::new();
    }
    found
}

/// A `fields` name for an isolated column, so a rule can say `%name`.
///
/// The reported file's isolated column had **no header at all**, and an
/// unmapped column with no header is named `""` — hledger's "ignore this
/// column" — which no matcher can refer to. So one is made up, through
/// [`ColumnGuess::name`] rather than through any second naming channel: this
/// only chooses the *text*, and `guess_columns`' own pass 3 still decides
/// whether it survives.
///
/// Disambiguated against every other header because a file may genuinely have a
/// column headed `Column 1`, and a duplicate `fields` name is resolved by
/// hledger as "first one wins", in silence. At most `taken.len()` spellings can
/// be spoken for, so one of the `taken.len() + 1` candidates tried is always
/// free — the search is bounded, not hopeful.
fn placeholder_name(column: usize, header: &[String]) -> String {
    let taken: Vec<String> = header.iter().map(|text| normalize(text)).collect();
    let base = format!("column{}", column + 1);
    (0..=taken.len())
        .map(|suffix| {
            if suffix == 0 {
                base.clone()
            } else {
                format!("{base}x{suffix}")
            }
        })
        .find(|candidate| !taken.contains(candidate))
        .unwrap_or(base)
}

/// The header [`guess_columns`] is shown, with every isolated column renamed.
///
/// The substitution does two jobs at once, and both are needed:
///
/// - **It supplies the name** an exclusion rule interpolates.
/// - **It withdraws the column's claim on a field.** A column that is empty on
///   every importable row is not that field, whatever its header says — and
///   mapping it would be worse than useless, because an exact header match
///   outscores the value-shaped guess a *real* money or date column has to fall
///   back on, so the label's column would take the field and the real one would
///   go unmapped. Isolation therefore wins over a confident field guess, always:
///   the header said something about a column the data contradicts.
///
/// Uniform rather than conditional — an isolated column's own header, where it
/// has one, is precisely the thing that misdescribed it, and the warning names
/// the generated rule verbatim so the identifier does not have to be pretty.
fn mapping_header(header: &[String], width: usize, isolated: &[IsolatedColumn]) -> Vec<String> {
    (0..width)
        .map(|column| {
            if isolated.iter().any(|found| found.column == column) {
                placeholder_name(column, header)
            } else {
                header.get(column).cloned().unwrap_or_default()
            }
        })
        .collect()
}

/// The table's rows with the isolated ones taken out.
///
/// The rows left out here are **exactly** the rows the drafted rules leave out
/// of the import, and that equality is the point: a draft whose `date-format`
/// was read off rows hledger will never see describes a different file from the
/// one it will be handed.
fn retained_rows(rows: &[Vec<String>], isolated: &[IsolatedColumn]) -> Vec<Vec<String>> {
    rows.iter()
        .enumerate()
        .filter(|(row, _)| !isolated.iter().any(|found| found.rows.contains(row)))
        .map(|(_, cells)| cells.clone())
        .collect()
}

// ---------------------------------------------------------------------------
// Date formats
// ---------------------------------------------------------------------------

/// The formats tried, commonest first.
///
/// Written **padded**; [`relax`] rewrites the month and day specifiers when the
/// data needs it. Ordering is the tie-break: two formats that both read every
/// sample (`01/02/2026`) are a genuine ambiguity, and the earlier one is the
/// guess this module offers while [`DateFormatGuess::ambiguous`] says it is one.
///
/// A port of `web/src/lib/imports/dateFormats.ts`'s catalogue plus the two
/// datetime shapes an export writes, because a format must consume the **whole**
/// value — `%Y-%m-%d` does not truncate `2026-01-02T13:45:00`, it fails it.
const FORMATS: &[&str] = &[
    "%Y-%m-%d",
    "%Y/%m/%d",
    "%Y.%m.%d",
    "%Y%m%d",
    "%m/%d/%Y",
    "%d/%m/%Y",
    "%m-%d-%Y",
    "%d-%m-%Y",
    "%d.%m.%Y",
    "%m/%d/%y",
    "%d/%m/%y",
    "%y-%m-%d",
    "%d-%b-%Y",
    "%b %d, %Y",
    "%d %b %Y",
    "%B %d, %Y",
    "%d %B %Y",
    "%Y-%m-%dT%H:%M:%S",
    "%Y-%m-%d %H:%M:%S",
    "%m/%d/%Y %H:%M:%S",
    "%m/%d/%Y %H:%M",
];

/// Every catalogue format that reads **all** of `values`, in catalogue order.
///
/// Reuses `matching`'s own format reader rather than growing a second one: it
/// already models the specifier set hledger's rules files use, it is already
/// exercised against the binary by `LEDGELINE_HLEDGER_MATCH_CHECK`, and a second
/// implementation would be free to disagree with the one that scores candidates.
fn readable_formats(values: &[String]) -> Vec<&'static str> {
    // No samples is not "every format works", though that is exactly what the
    // count below says: `Some(0) == Some(0)` for all of them. A column with
    // nothing in it supports no conclusion at all, and the honest answer is the
    // empty list -- which becomes `None` at every caller.
    if values.is_empty() {
        return Vec::new();
    }
    FORMATS
        .iter()
        .copied()
        .filter(|format| matching::count_parsable(format, values) == Some(values.len()))
        .collect()
}

/// The first catalogue format reading every value, or `None`.
fn best_format(values: &[String]) -> Option<&'static str> {
    readable_formats(values).into_iter().next()
}

/// Does `value` carry a numeric run of a single digit — an unpadded month or
/// day, which a `%m`/`%d` specifier would reject outright?
///
/// A four-digit year is not the question, and neither is a two-digit one: this
/// asks only whether some run is shorter than the two characters a padded
/// specifier demands.
fn has_unpadded_component(value: &str) -> bool {
    value
        .split(|c: char| !c.is_ascii_digit())
        .filter(|run| !run.is_empty())
        .any(|run| run.len() == 1)
}

/// `%m` → `%-m` and `%d` → `%-d`, for data that is not zero-padded.
///
/// Only the two specifiers that can be short in real data. `%Y`/`%y`/`%H` and
/// friends are left alone: a one-digit year or hour is not a shape any exporter
/// writes, and rewriting them would make every generated format unreadable for
/// no gain.
fn relax(format: &str) -> String {
    format.replace("%m", "%-m").replace("%d", "%-d")
}

/// Read a `date-format` off sample values from the mapped date column.
///
/// Pure. `None` means **no catalogue format read every sample**, which is a
/// thing the caller has to say out loud: with no `date-format` directive hledger
/// accepts year-first dates only, so an unrecognised format is an import that
/// fails on its first record rather than one that quietly does something else.
#[must_use]
pub fn guess_date_format(samples: &[String]) -> Option<DateFormatGuess> {
    let readable = readable_formats(samples);
    let best = *readable.first()?;
    // Only a *shape* ambiguity counts. `%Y-%m-%d` and `%Y.%m.%d` never both read
    // the same value, so anything that survives here differs in which component
    // is the month — which is the ambiguity that files March in December.
    let ambiguous = readable.len() > 1;
    let unpadded = samples.iter().any(|value| has_unpadded_component(value));
    Some(DateFormatGuess {
        format: if unpadded {
            relax(best)
        } else {
            best.to_string()
        },
        confidence: if ambiguous { 0.5 } else { 0.9 },
        ambiguous,
    })
}

// ---------------------------------------------------------------------------
// Amount shape: the decimal mark, and whether the cells carry a commodity
// ---------------------------------------------------------------------------

/// What the amount column's own text says about how to read it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct AmountShape {
    /// The `decimal-mark` to declare, when declaring one changes anything.
    decimal_mark: Option<char>,
    /// The symbol every sample carries, when they all carry the same one.
    symbol: Option<String>,
    /// Samples disagreed about the decimal mark.
    mark_conflict: bool,
    /// Samples disagreed about the currency symbol.
    symbol_conflict: bool,
}

/// Which separator a value's punctuation says is its decimal mark.
///
/// Two kinds of evidence, and the strong one wins:
///
/// 1. **A separator followed by one or two digits at the end** is a decimal
///    mark. `4,50` is four euros fifty; `1,234` is not.
/// 2. **A separator followed by exactly three digits, with nothing after it**,
///    is a *group* separator, so the decimal mark is the other character. This
///    is the case that matters: `1,234` with no `decimal-mark` is read by
///    hledger as `1.234`, and `print` renders it back as `1,234`, so the 1000×
///    error never appears in any output.
fn decimal_evidence(body: &str) -> Option<char> {
    let (at, separator) = body.char_indices().rfind(|(_, c)| *c == '.' || *c == ',')?;
    let tail = &body[at + separator.len_utf8()..];
    if !tail.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    match tail.len() {
        1 | 2 => Some(separator),
        3 => Some(if separator == ',' { '.' } else { ',' }),
        _ => None,
    }
}

/// The currency symbol a cell carries, if any: whatever sits outside the digits
/// and separators once sign, parentheses and whitespace are gone.
fn symbol_of(value: &str) -> Option<String> {
    let trimmed = value.trim();
    let unwrapped = trimmed
        .strip_prefix('(')
        .and_then(|rest| rest.strip_suffix(')'))
        .unwrap_or(trimmed);
    let symbol: String = unwrapped
        .chars()
        .filter(|c| {
            !c.is_ascii_digit()
                && !c.is_whitespace()
                && !matches!(c, '.' | ',' | '+' | '-' | '(' | ')')
        })
        .collect();
    (!symbol.is_empty()).then_some(symbol)
}

/// Read the amount column's punctuation and commodity.
fn amount_shape(values: &[String]) -> AmountShape {
    let bodies: Vec<String> = values
        .iter()
        .filter_map(|value| amount_body(value))
        .collect();
    let marks: Vec<char> = bodies
        .iter()
        .filter_map(|body| decimal_evidence(body))
        .collect();
    let mark_conflict = marks.windows(2).any(|pair| pair[0] != pair[1]);
    // Only worth declaring when the text is actually ambiguous — a column of
    // plain `-4.50` needs no directive, and one that says nothing is one fewer
    // line for the user to wonder about.
    let ambiguous_text = bodies
        .iter()
        .any(|body| body.contains(',') || body.matches('.').count() > 1);
    let decimal_mark = (!mark_conflict && ambiguous_text)
        .then(|| marks.first().copied())
        .flatten();

    let symbols: Vec<String> = values.iter().filter_map(|value| symbol_of(value)).collect();
    let symbol_conflict = symbols.windows(2).any(|pair| pair[0] != pair[1]);
    // Every sample must carry one, not merely some of them: a column where only
    // half the cells have a `$` is a column this module will not describe.
    let symbol = (!symbol_conflict && symbols.len() == values.len())
        .then(|| symbols.first().cloned())
        .flatten();

    AmountShape {
        decimal_mark,
        symbol,
        mark_conflict,
        symbol_conflict,
    }
}

/// The `decimal-mark` a column of amount text calls for, or `None`.
///
/// Public because it is the one piece of this module a caller might reasonably
/// want on its own, and because it is the piece with the sharpest edge — see
/// [`decimal_evidence`].
#[must_use]
pub fn guess_decimal_mark(samples: &[String]) -> Option<char> {
    amount_shape(samples).decimal_mark
}

// ---------------------------------------------------------------------------
// The draft
// ---------------------------------------------------------------------------

/// Which column each interesting field ended up on.
fn column_for(columns: &[ColumnGuess], field: HledgerField) -> Option<usize> {
    columns
        .iter()
        .find(|column| column.field == Some(field))
        .map(|column| column.index)
}

/// Every value of **every** mapped amount column, pooled.
///
/// All of them, not the first one found, and that distinction is a bug this
/// function had: a split scheme maps `amount-in` AND `amount-out`, the two
/// carry different rows, and sampling only one of them misses whatever is in
/// the other. On a real card export — `Debit` holding `1,200`, `Credit` holding
/// `2400.00` — that meant no `decimal-mark` was written and hledger read twelve
/// hundred dollars as one dollar twenty, silently. Caught by driving the live
/// route rather than by a test, which is why the test now exists.
///
/// Pooling is sound because the decimal mark and the currency symbol are
/// properties of the FILE, not of a column: an exporter that writes `1.200,00`
/// in one money column does not write `1,200.00` in the next one. Where the two
/// genuinely disagree, [`amount_shape`] reports a conflict and declares
/// nothing, which is the same answer it gives for one self-contradictory column.
fn amount_samples(columns: &[ColumnGuess], rows: &[Vec<String>]) -> Vec<String> {
    [
        HledgerField::Amount,
        HledgerField::AmountIn,
        HledgerField::AmountOut,
    ]
    .into_iter()
    .filter_map(|field| column_for(columns, field))
    .flat_map(|index| samples(rows, index))
    .collect()
}

/// A `skip` / `date-format` / `decimal-mark` directive as an insertable item.
fn directive(name: DirectiveName, value: DirectiveValue) -> Slot {
    Slot::Insert(ItemBody::Directive { name, value })
}

/// A top-level field assignment as an insertable item.
fn assignment(field: HledgerField, value: &str) -> Slot {
    Slot::Insert(ItemBody::Assignment {
        field,
        value: value.to_string(),
    })
}

/// The pattern an exclusion rule matches with: hledger's "anything at all".
///
/// Verified against 1.52 over the reported file's exact shape. `if %col .` with
/// a bare `skip` drops every record whose `col` holds something and leaves every
/// record whose `col` is empty **alone** — the three data rows below the label
/// imported unchanged. A cell holding only whitespace does not match either,
/// because hledger trims a field before matching it, which is the same reading
/// [`lone_cell`] takes; the two agree by construction rather than by luck.
const ANY_CONTENT: &str = ".";

/// `if %NAME .` over a bare `skip` — one column's exclusion rule.
///
/// Built through [`ItemBody::IfBlock`]'s existing `control` field rather than as
/// an assignment, because `skip` does not assign anything: it changes which CSV
/// records are read at all. Nothing in `rules.rs` needed changing for this — the
/// shape has been writable since same-line `&&` and `skip`/`end` blocks became
/// editable.
fn exclusion(name: &str) -> Slot {
    Slot::Insert(ItemBody::IfBlock {
        groups: vec![MatcherGroupSpec {
            matchers: vec![MatcherSpec {
                scope: MatchScope::Field(name.to_string()),
                pattern: ANY_CONTENT.to_string(),
            }],
        }],
        assignments: Vec::new(),
        control: Some(ControlField::Skip),
    })
}

/// Render a starting-point rules file from a mapping.
///
/// The text is produced by [`RulesDoc::apply`] over an **empty** document rather
/// than by formatting strings here, which is not a stylistic choice: it means a
/// draft goes through the same renderer, the same edit policy and the same
/// [`check_body`](crate::rules) validation as every save the editor performs, so
/// there is no second spelling of a directive to drift.
///
/// `account1` is the one thing no CSV can supply. It may be empty — the draft
/// then carries a bare `account1` line for the caller's form to fill in — but a
/// file saved that way is not one hledger can use, which is the caller's to
/// enforce.
///
/// `isolated` names the columns [`isolated_columns`] found, each of which earns
/// one trailing `if %name .` / `skip` block. `tabular` must be the table with
/// those rows **already removed** — see [`retained_rows`] for why the two have
/// to agree.
///
/// # Errors
/// [`RulesError`] if the renderer refuses a value, which for a caller-supplied
/// `account1` means a control character or an over-long line. Everything else in
/// the draft is this module's own, so in practice this is the account name's
/// error and nobody else's.
pub fn draft(
    tabular: &Tabular,
    columns: &[ColumnGuess],
    date_format: Option<&str>,
    account1: &str,
    isolated: &[IsolatedColumn],
) -> Result<RulesDoc, RulesError> {
    let empty = RulesDoc::parse("");
    let mut order = Vec::new();

    // `skip 1`, and only ever 1: the converted CSV's header is its only
    // non-data record. See the module docs.
    if tabular.header.is_some() {
        order.push(directive(DirectiveName::Skip, DirectiveValue::Skip(1)));
    }
    if let Some(format) = date_format {
        order.push(directive(
            DirectiveName::DateFormat,
            // Trimmed at the end because hledger does **not** trim a format
            // string, so a trailing space makes the pattern unsatisfiable.
            DirectiveValue::Text(format.trim_end().to_string()),
        ));
    }
    let amounts = amount_samples(columns, &tabular.rows);
    let shape = amount_shape(&amounts);
    if let Some(mark) = shape.decimal_mark {
        order.push(directive(
            DirectiveName::DecimalMark,
            DirectiveValue::DecimalMark(mark),
        ));
    }
    order.push(Slot::Insert(ItemBody::Fields {
        names: columns.iter().map(|column| column.name.clone()).collect(),
    }));
    // `currency` ONLY over bare amounts. It is a blind string prefix, so
    // declaring it over cells that already carry a `$` produces `$$`, which is a
    // distinct commodity and never reported.
    if shape.symbol.is_none()
        && let Some(currency) = statement_currency(tabular)
    {
        order.push(assignment(HledgerField::Currency, &currency));
    }
    order.push(assignment(
        HledgerField::Numbered {
            base: crate::rules::NumberedField::Account,
            n: 1,
        },
        account1,
    ));
    order.push(assignment(
        HledgerField::Numbered {
            base: crate::rules::NumberedField::Account,
            n: 2,
        },
        DEFAULT_ACCOUNT2,
    ));
    // The exclusions go last, where a rules file's rules go. Position among
    // them is free — a `skip` block moved relative to other rules changes not
    // one imported row (verified against 1.52) — and a draft has no other rules
    // for these to sit among anyway.
    //
    // `get` rather than an index because nothing here may panic; it cannot miss,
    // since `columns` is as wide as the table `isolated` was read from.
    order.extend(
        isolated
            .iter()
            .filter_map(|found| columns.get(found.column))
            .filter(|column| !column.name.is_empty())
            .map(|column| exclusion(&column.name)),
    );

    let plan = EditPlan {
        order,
        delete: Vec::new(),
    };
    let text = empty.apply(&plan)?;
    Ok(RulesDoc::parse(&text))
}

/// The commodity the source format volunteered, when it did.
fn statement_currency(tabular: &Tabular) -> Option<String> {
    tabular
        .statement
        .as_ref()
        .and_then(|meta| meta.currency.as_ref())
        .map(|code| code.trim().to_string())
        .filter(|code| !code.is_empty())
}

/// Guess, draft and explain — the whole of this module in one call.
///
/// # Errors
/// [`RulesError`] from [`draft`]; see there.
pub fn generate(tabular: &Tabular, account1: &str) -> Result<Draft, RulesError> {
    let isolated = isolated_columns(tabular);
    let raw_header = tabular.header.clone().unwrap_or_default();
    let width = raw_header
        .len()
        .max(tabular.rows.iter().map(Vec::len).max().unwrap_or(0));
    let header = mapping_header(&raw_header, width, &isolated);
    // Every heuristic below reads `data`, never `tabular`: an isolated row is
    // the row the draft is about to exclude from the import, so it was never
    // evidence about the file's dates, its decimal mark or which column is
    // which. Reading it was the whole of the reported bug.
    let data = Tabular {
        rows: retained_rows(&tabular.rows, &isolated),
        ..tabular.clone()
    };
    let columns = guess_columns(&header, &data.rows);
    let date_samples = column_for(&columns, HledgerField::Date)
        .map(|index| samples(&data.rows, index))
        .unwrap_or_default();
    let date_format = guess_date_format(&date_samples);
    let doc = draft(
        &data,
        &columns,
        date_format.as_ref().map(|guess| guess.format.as_str()),
        account1,
        &isolated,
    )?;
    let warnings = explain(
        &data,
        &columns,
        date_format.as_ref(),
        &header,
        &isolated,
        &tabular.rows,
    );
    Ok(Draft {
        doc,
        columns,
        date_format,
        warnings,
    })
}

/// `1`, `1 and 4`, `1, 4 and 7` — a list a sentence can carry.
fn conjoin(parts: &[String]) -> String {
    match parts.split_last() {
        None => String::new(),
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
    }
}

/// One cell as a warning quotes it: short, and with nothing in it a sentence
/// cannot carry. A statement's cells are the user's own bytes, and this one is
/// about to be read back to them mid-prose.
fn quoted_cell(cell: &str) -> String {
    let text: String = cell
        .trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_WARNING_CELL_CHARS)
        .collect();
    format!("`{text}`")
}

/// The sentence one isolated column earns.
///
/// It names the row numbers, the values found and the rule generated, because
/// all three are checkable at a glance against the file the user is looking at
/// — and because this is the one guess in the wizard that changes which rows
/// import rather than how they are read.
fn isolated_warning(found: &IsolatedColumn, name: &str, all_rows: &[Vec<String>]) -> String {
    let numbers = conjoin(
        &found
            .rows
            .iter()
            .map(|row| (row + 1).to_string())
            .collect::<Vec<String>>(),
    );
    let values = found
        .rows
        .iter()
        .filter_map(|&row| all_rows.get(row)?.get(found.column))
        .map(|cell| quoted_cell(cell))
        .collect::<Vec<String>>()
        .join(", ");
    let plural = found.rows.len() > 1;
    format!(
        "Only one cell is filled in on data {} {numbers} ({values}), and {} in a column that \
         holds nothing in any other row. That is a label the original report left behind rather \
         than transaction data, and hledger abandons the whole import — not just that row — when \
         a record has no date. The draft therefore ends with `if %{name} .` and a `skip`, leaving \
         {} out. Check {} against your file and delete that rule if the data is real.",
        if plural { "rows" } else { "row" },
        if plural { "they sit" } else { "it sits" },
        if plural { "those rows" } else { "that row" },
        if plural { "those rows" } else { "that row" },
    )
}

/// Everything about this draft the user has to be told, in sentences.
///
/// Each one names a way the draft can be *wrong in a way hledger will not
/// mention*. There is deliberately nothing here about the parts that are
/// obviously right — a list of reassurances is a list nobody reads, and it is
/// the one warning that mattered that gets lost in it.
fn explain(
    tabular: &Tabular,
    columns: &[ColumnGuess],
    date_format: Option<&DateFormatGuess>,
    header: &[String],
    isolated: &[IsolatedColumn],
    all_rows: &[Vec<String>],
) -> Vec<String> {
    // First, because it is the only warning here about which ROWS import at
    // all, and because the shape it describes is one nothing else in the panel
    // makes visible.
    let mut warnings: Vec<String> = isolated
        .iter()
        .filter_map(|found| {
            let name = &columns.get(found.column)?.name;
            (!name.is_empty()).then(|| isolated_warning(found, name, all_rows))
        })
        .collect();
    let has = |field| column_for(columns, field).is_some();

    if has(HledgerField::Date) {
        match date_format {
            None => warnings.push(
                "Ledgeline could not recognise the dates in that column. With no date format \
                 declared hledger reads year-first dates only, so set one below before importing."
                    .to_string(),
            ),
            Some(guess) if guess.ambiguous => warnings.push(format!(
                "These dates could be read more than one way — `01/02/2026` is either 2 January \
                 or 1 February. Ledgeline guessed `{}`; check it against a transaction whose date \
                 you remember.",
                guess.format
            )),
            Some(_) => {}
        }
    } else {
        warnings.push(
            "No column looks like a date. hledger cannot import a record without one — pick the \
             date column below."
                .to_string(),
        );
    }

    let split = has(HledgerField::AmountIn) || has(HledgerField::AmountOut);
    if !has(HledgerField::Amount) && !split {
        warnings.push(
            "No column looks like an amount. Pick the column holding the money, or the two \
             columns holding money in and money out."
                .to_string(),
        );
    }
    // Asked of the HEADER, not of the resolved mapping: pass 2b has already
    // dropped the losing scheme, so by now only one of them is there and the
    // conflict the user needs to know about is invisible in `columns`.
    let offered = |field| {
        header
            .iter()
            .any(|name| match_header(name).map(|(field, _)| field) == Some(field))
    };
    if offered(HledgerField::Amount)
        && (offered(HledgerField::AmountIn) || offered(HledgerField::AmountOut))
    {
        warnings.push(
            "This file has both a single amount column and separate in/out columns. hledger \
             refuses a record where two of them carry a value, so only one has been mapped — \
             unmap it and map the others if the wrong one was chosen."
                .to_string(),
        );
    }
    if split && !(has(HledgerField::AmountIn) && has(HledgerField::AmountOut)) {
        warnings.push(
            "Only one of the two money columns was recognised. If the statement has both, map \
             the other as well: hledger negates `amount-out` and leaves `amount-in` alone, so a \
             missing half silently gives every row the same sign."
                .to_string(),
        );
    }
    if has(HledgerField::Balance) {
        warnings.push(
            "A running-balance column was mapped to `balance`, so hledger will check your journal \
             against every row. That is a real check worth having once this account's history is \
             complete — and it fails the whole import when it is not. Unmap it if you are \
             importing part of a history."
                .to_string(),
        );
    }

    let amounts = amount_samples(columns, &tabular.rows);
    let shape = amount_shape(&amounts);
    if shape.mark_conflict {
        warnings.push(
            "The amounts do not agree about which character is the decimal point, so no \
             `decimal-mark` was declared. Check the Decimal mark setting — hledger reads a lone \
             `,` as a decimal point, which turns `1,234` into 1.234 without saying so."
                .to_string(),
        );
    }
    if shape.symbol_conflict {
        warnings.push(
            "The amounts carry more than one currency symbol, so no currency was declared. \
             hledger will read each cell's own symbol."
                .to_string(),
        );
    }
    if shape.symbol.is_none() && !shape.symbol_conflict && statement_currency(tabular).is_none() {
        warnings.push(
            "The amounts carry no currency symbol. hledger reads those as a commodity of their \
             own, which never adds up with the `$` amounts already in your journal — set Currency \
             below to the commodity this account is in."
                .to_string(),
        );
    }

    // Named-but-unmapped columns, listed once. This is the sentence that makes
    // an unmapped column a decision the user can see rather than a silent
    // omission — and it is the shape hledger will not warn about.
    let unmapped: Vec<String> = columns
        .iter()
        .filter(|column| column.field.is_none() && !column.name.is_empty())
        // An isolated column is named and unmapped too, but it has its own
        // warning saying so at length. Listing it here as well would offer it
        // to the user as something a later rule might usefully interpolate,
        // which is the opposite of what the other warning just said about it.
        .filter(|column| !isolated.iter().any(|found| found.column == column.index))
        .filter_map(|column| header.get(column.index).cloned())
        .collect();
    if !unmapped.is_empty() {
        warnings.push(format!(
            "Not imported, only named, so a rule can refer to them later: {}.",
            unmapped.join(", ")
        ));
    }
    warnings
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convert::{StatementMeta, Tabular};

    fn table(header: &[&str], rows: &[&[&str]]) -> Tabular {
        Tabular {
            header: Some(header.iter().map(|c| (*c).to_string()).collect()),
            rows: rows
                .iter()
                .map(|row| row.iter().map(|c| (*c).to_string()).collect())
                .collect(),
            ..Tabular::default()
        }
    }

    fn names(doc: &RulesDoc) -> Vec<String> {
        doc.settings().fields.map(|f| f.value).unwrap_or_default()
    }

    fn text(doc: &RulesDoc) -> String {
        doc.text().to_string()
    }

    #[test]
    fn a_plain_bank_export_maps_itself() {
        let columns = guess_columns(
            &["Date".into(), "Description".into(), "Amount".into()],
            &[vec!["2026-01-02".into(), "COFFEE".into(), "-4.50".into()]],
        );
        assert_eq!(columns[0].field, Some(HledgerField::Date));
        assert_eq!(columns[1].field, Some(HledgerField::Description));
        assert_eq!(columns[2].field, Some(HledgerField::Amount));
        assert!(columns.iter().all(|column| column.confidence > 0.9));
    }

    #[test]
    fn debit_and_credit_become_the_two_amount_halves() {
        // Which way round these go is hledger's, not ours: `amount-out` is
        // NEGATED and `amount-in` is not (verified against 1.52), so a debit —
        // money leaving the account — has to be the `out` half or every
        // withdrawal lands as income.
        let columns = guess_columns(
            &[
                "Posted Date".into(),
                "Payee".into(),
                "Debit".into(),
                "Credit".into(),
            ],
            &[vec![
                "01/02/2026".into(),
                "COFFEE".into(),
                "4.50".into(),
                String::new(),
            ]],
        );
        assert_eq!(columns[0].field, Some(HledgerField::Date));
        assert_eq!(columns[1].field, Some(HledgerField::Description));
        assert_eq!(columns[2].field, Some(HledgerField::AmountOut));
        assert_eq!(columns[3].field, Some(HledgerField::AmountIn));
    }

    #[test]
    fn a_signed_total_beside_debit_and_credit_maps_only_one_scheme() {
        // hledger refuses a record with a value in `amount` AND in
        // `amount-in`/`amount-out` ("Multiple non-zero amounts were assigned",
        // exit 1). A bank that writes all three columns is ordinary, so the
        // draft has to pick a scheme rather than map them all and fail on the
        // first row.
        let columns = guess_columns(
            &[
                "Date".into(),
                "Payee".into(),
                "Amount".into(),
                "Debit".into(),
                "Credit".into(),
            ],
            &[vec![
                "2026-01-02".into(),
                "COFFEE".into(),
                "-4.50".into(),
                "4.50".into(),
                String::new(),
            ]],
        );
        assert_eq!(columns[2].field, Some(HledgerField::Amount));
        assert_eq!(columns[3].field, None, "the split scheme loses to a total");
        assert_eq!(columns[4].field, None);
        // And the loser is still NAMED, so nothing about the file is hidden.
        assert_eq!(columns[3].name, "debit");

        let data = table(
            &["Date", "Payee", "Amount", "Debit", "Credit"],
            &[&["2026-01-02", "COFFEE", "-4.50", "4.50", ""]],
        );
        let drafted = generate(&data, "assets:bank:checking").unwrap();
        assert!(
            drafted
                .warnings
                .iter()
                .any(|w| w.contains("both a single amount column")),
            "{:?}",
            drafted.warnings
        );
    }

    #[test]
    fn a_status_column_is_never_mapped() {
        // hledger's `status` field wants `*`, `!` or nothing. A bank's
        // "Posted"/"Pending" column mapped to it is a journal that will not
        // parse, and "Status" is a very common header — so it is excluded from
        // the table outright rather than scored low.
        let columns = guess_columns(
            &["Date".into(), "Status".into(), "Amount".into()],
            &[vec!["2026-01-02".into(), "Posted".into(), "-4.50".into()]],
        );
        assert_eq!(columns[1].field, None);
        // And it is not NAMED `status` either. An unmapped column normally
        // keeps a plain form of its own header so a later rule can interpolate
        // it -- but `status` IS an hledger field name, so writing it into
        // `fields` would assign the field after all, which is the very thing
        // leaving it unmapped was for.
        assert_eq!(columns[1].name, "");
    }

    #[test]
    fn an_unmapped_column_keeps_its_own_name_for_later_interpolation() {
        let columns = guess_columns(
            &["Date".into(), "FIT ID".into(), "Amount".into()],
            &[vec![
                "2026-01-02".into(),
                "20260102001".into(),
                "-4.50".into(),
            ]],
        );
        assert_eq!(columns[1].field, None);
        assert_eq!(columns[1].name, "fitid", "so `%fitid` is reachable");
    }

    #[test]
    fn a_second_column_claiming_a_taken_field_is_not_named_as_one() {
        // hledger resolves two columns named `amount` as "first one wins", in
        // silence. So the loser must not be written as `amount` at all -- and
        // must not be written as its own header either, since that header IS an
        // hledger field name and would assign the field after all.
        let columns = guess_columns(
            &["Date".into(), "Amount".into(), "Amount".into()],
            &[vec!["2026-01-02".into(), "-4.50".into(), "-4.50".into()]],
        );
        assert_eq!(columns[1].field, Some(HledgerField::Amount));
        assert_eq!(columns[2].field, None);
        assert_eq!(columns[2].name, "", "never a second `amount`");
    }

    #[test]
    fn a_second_date_column_becomes_date2() {
        // The Capital One shape: a transaction date and a posted date. hledger
        // has a field for the second one, so the runner-up is demoted rather
        // than dropped -- and it is worth less than the winner, which is what
        // `confidence` is for.
        let columns = guess_columns(
            &[
                "Transaction Date".into(),
                "Posted Date".into(),
                "Description".into(),
                "Debit".into(),
            ],
            &[vec![
                "2026-01-02".into(),
                "2026-01-03".into(),
                "COFFEE".into(),
                "4.50".into(),
            ]],
        );
        assert_eq!(columns[0].field, Some(HledgerField::Date));
        assert_eq!(columns[1].field, Some(HledgerField::Date2));
        assert!(columns[1].confidence < columns[0].confidence);
    }

    #[test]
    fn a_third_date_column_is_named_not_guessed() {
        let columns = guess_columns(
            &[
                "Date".into(),
                "Posted Date".into(),
                "Value Date".into(),
                "Amount".into(),
            ],
            &[vec![
                "2026-01-02".into(),
                "2026-01-03".into(),
                "2026-01-04".into(),
                "-4.50".into(),
            ]],
        );
        assert_eq!(columns[2].field, None);
        assert_eq!(columns[2].name, "valuedate");
    }

    #[test]
    fn a_headerless_table_is_read_from_its_values() {
        let columns = guess_columns(
            &[],
            &[
                vec!["2026-01-02".into(), "COFFEE".into(), "-4.50".into()],
                vec!["2026-01-03".into(), "BOOKS".into(), "-12.00".into()],
            ],
        );
        assert_eq!(columns[0].field, Some(HledgerField::Date));
        assert_eq!(columns[1].field, None, "prose is not guessed at");
        assert_eq!(columns[2].field, Some(HledgerField::Amount));
    }

    #[test]
    fn an_iso_date_column_gets_the_padded_format() {
        let guess = guess_date_format(&["2026-01-02".into(), "2026-11-30".into()]).unwrap();
        assert_eq!(guess.format, "%Y-%m-%d");
        assert!(!guess.ambiguous);
    }

    #[test]
    fn an_unpadded_column_gets_the_relaxed_specifiers() {
        // `%m/%d/%Y` REJECTS `1/2/2026` outright (exit 1, verified against
        // hledger 1.52, whose own error message recommends `%-m/%-d/%Y`). The
        // relaxed form reads both spellings, so it is what unpadded data gets.
        let guess = guess_date_format(&["1/2/2026".into(), "11/30/2026".into()]).unwrap();
        assert_eq!(guess.format, "%-m/%-d/%Y");
    }

    #[test]
    fn a_month_first_file_is_recognised_once_a_day_exceeds_twelve() {
        let guess = guess_date_format(&["01/02/2026".into(), "01/30/2026".into()]).unwrap();
        assert_eq!(guess.format, "%m/%d/%Y");
        assert!(!guess.ambiguous, "day 30 rules out day-first");
    }

    #[test]
    fn a_day_first_file_is_recognised_the_same_way() {
        let guess = guess_date_format(&["30/01/2026".into(), "02/01/2026".into()]).unwrap();
        assert_eq!(guess.format, "%d/%m/%Y");
        assert!(!guess.ambiguous);
    }

    #[test]
    fn a_genuinely_ambiguous_column_says_so() {
        // Every component is <= 12, so nothing in the data can tell the two
        // readings apart. Guessing silently is how March lands in December.
        let guess = guess_date_format(&["01/02/2026".into(), "03/04/2026".into()]).unwrap();
        assert!(guess.ambiguous);
        assert_eq!(guess.format, "%m/%d/%Y", "month-first is the guess offered");
        assert!(guess.confidence < 0.7);
    }

    #[test]
    fn a_datetime_column_needs_the_time_in_the_format() {
        // A format must consume the WHOLE value: `%Y-%m-%d` does not truncate a
        // datetime, it fails it.
        let guess = guess_date_format(&["2026-01-02T13:45:00".into()]).unwrap();
        assert_eq!(guess.format, "%Y-%m-%dT%H:%M:%S");
    }

    #[test]
    fn an_unrecognisable_date_column_is_no_guess_at_all() {
        assert_eq!(guess_date_format(&["Q1 2026".into()]), None);
        assert_eq!(guess_date_format(&[]), None);
    }

    #[test]
    fn a_split_scheme_samples_both_money_columns() {
        // The bug the live route caught. `Debit` holds `1,200` and `Credit`
        // holds `2400.00`; sampling only the first mapped amount column found
        // — which was `amount-in`, i.e. Credit — saw no separator at all, wrote
        // no `decimal-mark`, and let hledger read twelve hundred dollars as one
        // dollar twenty. Silently: `print` re-renders `1,200` as `1,200`
        // whichever way it read it.
        let data = table(
            &["Date", "Merchant", "Debit", "Credit"],
            &[
                &["01/02/2026", "COFFEE", "4.50", ""],
                &["01/09/2026", "MORTGAGE", "1,200", ""],
                &["01/15/2026", "PAYROLL", "", "2400.00"],
            ],
        );
        let drafted = generate(&data, "assets:bank:checking").unwrap();
        assert_eq!(
            drafted
                .doc
                .settings()
                .decimal_mark
                .map(|setting| setting.value),
            Some('.'),
            "the separator is in the column that is NOT sampled first"
        );
    }

    #[test]
    fn a_thousands_comma_forces_a_decimal_mark() {
        // THE case this exists for: `1,234` with no directive is read by
        // hledger as 1.234 and re-rendered as `1,234`, so the 1000x error is
        // invisible in its own output.
        assert_eq!(
            guess_decimal_mark(&["1,234".into(), "-56".into()]),
            Some('.')
        );
    }

    #[test]
    fn a_decimal_comma_is_declared_too() {
        assert_eq!(
            guess_decimal_mark(&["1.234,56".into(), "-9,00".into()]),
            Some(',')
        );
    }

    #[test]
    fn plain_amounts_need_no_decimal_mark_at_all() {
        assert_eq!(guess_decimal_mark(&["-4.50".into(), "12.00".into()]), None);
        assert_eq!(guess_decimal_mark(&["-4".into(), "12".into()]), None);
    }

    #[test]
    fn a_draft_reads_back_as_the_settings_it_was_given() {
        let data = table(
            &["Date", "Description", "Amount"],
            &[&["2026-01-02", "COFFEE", "-4.50"]],
        );
        let drafted = generate(&data, "assets:bank:checking").unwrap();
        let settings = drafted.doc.settings();
        assert_eq!(settings.skip.map(|s| s.value), Some(1));
        assert_eq!(
            settings.date_format.map(|s| s.value).as_deref(),
            Some("%Y-%m-%d")
        );
        assert_eq!(
            settings.account1.map(|s| s.value).as_deref(),
            Some("assets:bank:checking")
        );
        assert_eq!(
            settings.account2.map(|s| s.value).as_deref(),
            Some(DEFAULT_ACCOUNT2)
        );
        assert_eq!(names(&drafted.doc), ["date", "description", "amount"]);
    }

    #[test]
    fn a_draft_never_declares_a_separator_or_an_encoding() {
        // The file this describes is `convert::to_csv`'s output: always commas,
        // always UTF-8, whatever the download was. A `separator ;` or an
        // `encoding windows-1252` copied off the original would describe a file
        // that no longer exists -- and the encoding one would make hledger
        // MIS-DECODE a UTF-8 file.
        let data = table(&["Date", "Amount"], &[&["2026-01-02", "-4.50"]]);
        let drafted = generate(&data, "assets:bank:checking").unwrap();
        assert!(drafted.doc.settings().separator.is_none());
        assert!(drafted.doc.settings().encoding.is_none());
        assert!(!text(&drafted.doc).contains("separator"));
        assert!(!text(&drafted.doc).contains("encoding"));
    }

    #[test]
    fn currency_is_declared_only_over_bare_amounts() {
        // `currency` is a blind string PREFIX: over a cell already reading
        // `$-4.50` it produces `$$-4.50`, a distinct commodity, exit 0, no
        // warning.
        let with_symbol = Tabular {
            statement: Some(StatementMeta {
                currency: Some("USD".into()),
                ..StatementMeta::default()
            }),
            ..table(&["Date", "Amount"], &[&["2026-01-02", "$-4.50"]])
        };
        let drafted = generate(&with_symbol, "assets:bank:checking").unwrap();
        assert!(
            drafted.doc.settings().currency.is_none(),
            "the cells carry their own symbol"
        );

        let bare = Tabular {
            statement: Some(StatementMeta {
                currency: Some("USD".into()),
                ..StatementMeta::default()
            }),
            ..table(&["Date", "Amount"], &[&["2026-01-02", "-4.50"]])
        };
        let drafted = generate(&bare, "assets:bank:checking").unwrap();
        assert_eq!(
            drafted.doc.settings().currency.map(|s| s.value).as_deref(),
            Some("USD")
        );
    }

    #[test]
    fn a_bare_amount_column_with_no_statement_currency_says_so() {
        let data = table(&["Date", "Amount"], &[&["2026-01-02", "-4.50"]]);
        let drafted = generate(&data, "assets:bank:checking").unwrap();
        assert!(drafted.doc.settings().currency.is_none());
        assert!(
            drafted
                .warnings
                .iter()
                .any(|w| w.contains("no currency symbol")),
            "{:?}",
            drafted.warnings
        );
    }

    #[test]
    fn every_drafted_item_can_be_written_back() {
        // The create PUT can only name TYPED items -- `ItemBody` has no comment
        // variant, and a `trivia` item's only save form is `{kind:"keep", id}`,
        // which needs a file that already exists. So a draft carrying one would
        // be a draft that cannot be saved.
        let data = table(
            &["Date", "Memo", "Debit", "Credit", "Balance"],
            &[&["01/02/2026", "COFFEE", "4.50", "", "1,234.00"]],
        );
        let drafted = generate(&data, "assets:bank:checking").unwrap();
        for item in drafted.doc.items() {
            assert!(
                matches!(
                    item.kind,
                    crate::rules::ItemKind::Directive(_)
                        | crate::rules::ItemKind::Fields(_)
                        | crate::rules::ItemKind::Assignment(_)
                ),
                "a draft may not contain {:?}",
                item.kind
            );
        }
    }

    #[test]
    fn a_draft_round_trips_through_its_own_parse() {
        let data = table(
            &["Date", "Description", "Amount"],
            &[&["2026-01-02", "COFFEE", "-4.50"]],
        );
        let drafted = generate(&data, "assets:bank:checking").unwrap();
        let rendered = text(&drafted.doc);
        let reparsed = RulesDoc::parse(&rendered);
        assert_eq!(reparsed.text(), rendered);
        assert!(reparsed.warnings().is_empty(), "{:?}", reparsed.warnings());
    }

    #[test]
    fn an_empty_account1_still_drafts() {
        // The form fills it in. A draft that refused to exist until the user had
        // typed an account would be a form with nothing in it to look at.
        let data = table(&["Date", "Amount"], &[&["2026-01-02", "-4.50"]]);
        let drafted = generate(&data, "").unwrap();
        assert_eq!(
            drafted.doc.settings().account1.map(|s| s.value).as_deref(),
            Some("")
        );
    }

    #[test]
    fn a_headerless_table_gets_no_skip() {
        let data = Tabular {
            header: None,
            rows: vec![vec!["2026-01-02".into(), "-4.50".into()]],
            ..Tabular::default()
        };
        let drafted = generate(&data, "assets:bank:checking").unwrap();
        assert!(drafted.doc.settings().skip.is_none());
    }

    #[test]
    fn a_missing_date_column_is_a_warning_not_a_silent_omission() {
        let data = table(&["Ref", "Amount"], &[&["X1", "-4.50"]]);
        let drafted = generate(&data, "assets:bank:checking").unwrap();
        assert!(
            drafted
                .warnings
                .iter()
                .any(|w| w.contains("looks like a date")),
            "{:?}",
            drafted.warnings
        );
    }

    // -----------------------------------------------------------------------
    // Isolated cells
    // -----------------------------------------------------------------------

    #[test]
    fn the_report_label_a_user_hit_is_isolated() {
        // The reported shape, exactly: a QuickBooks export converted to CSV
        // whose header row is followed by ONE row holding a stray section
        // label in a column that has no header and no other value anywhere.
        let data = table(
            &["", "Date", "Description", "Amount"],
            &[
                &["General Ledger", "", "", ""],
                &["", "2026-01-02", "COFFEE", "-4.50"],
                &["", "2026-01-03", "BOOKS", "-12.00"],
                &["", "2026-01-04", "PAYROLL", "2400.00"],
            ],
        );
        assert_eq!(
            isolated_columns(&data),
            vec![IsolatedColumn {
                column: 0,
                rows: vec![0]
            }]
        );
    }

    #[test]
    fn a_check_number_column_is_never_isolated() {
        // THE false positive, and the one that would be actively harmful: a
        // real field populated only on some rows. Keyed on column sparsity
        // alone this fires and silently drops every check payment. It must not
        // fire, because those rows have a date, a payee and an amount too --
        // they fail "the row is isolated" however empty the column is.
        let data = table(
            &["Date", "Description", "Check Number", "Amount"],
            &[
                &["2026-01-02", "COFFEE", "", "-4.50"],
                &["2026-01-03", "RENT", "1041", "-1200.00"],
                &["2026-01-04", "BOOKS", "", "-12.00"],
                &["2026-01-05", "PLUMBER", "1042", "-300.00"],
                &["2026-01-06", "PAYROLL", "", "2400.00"],
                &["2026-01-07", "COFFEE", "", "-4.50"],
                &["2026-01-08", "BOOKS", "", "-9.00"],
                &["2026-01-09", "COFFEE", "", "-4.50"],
            ],
        );
        assert_eq!(isolated_columns(&data), Vec::new());
    }

    #[test]
    fn several_labels_in_one_column_are_one_finding() {
        // One rule covers all of them -- `if %column1 .` matches any of the
        // three -- so reporting three near-identical findings would be three
        // ways to say one thing.
        let data = table(
            &["", "Date", "Description", "Amount"],
            &[
                &["General Ledger", "", "", ""],
                &["", "2026-01-02", "COFFEE", "-4.50"],
                &["", "2026-01-03", "BOOKS", "-12.00"],
                &["Checking Account", "", "", ""],
                &["", "2026-01-04", "PAYROLL", "2400.00"],
                &["", "2026-01-05", "RENT", "-1200.00"],
                &["", "2026-01-06", "COFFEE", "-4.50"],
                &["", "2026-01-07", "BOOKS", "-9.00"],
            ],
        );
        assert_eq!(
            isolated_columns(&data),
            vec![IsolatedColumn {
                column: 0,
                rows: vec![0, 3]
            }]
        );
    }

    #[test]
    fn labels_in_two_columns_are_two_findings() {
        // A report can leave litter in more than one place, and the two need
        // different rules -- `%column1` will not match `%column5`.
        let data = table(
            &["", "Date", "Description", "Amount", ""],
            &[
                &["General Ledger", "", "", "", ""],
                &["", "2026-01-02", "COFFEE", "-4.50", ""],
                &["", "2026-01-03", "BOOKS", "-12.00", ""],
                &["", "", "", "", "Report total"],
                &["", "2026-01-04", "PAYROLL", "2400.00", ""],
                &["", "2026-01-05", "RENT", "-1200.00", ""],
                &["", "2026-01-06", "COFFEE", "-4.50", ""],
                &["", "2026-01-07", "BOOKS", "-9.00", ""],
            ],
        );
        assert_eq!(
            isolated_columns(&data),
            vec![
                IsolatedColumn {
                    column: 0,
                    rows: vec![0]
                },
                IsolatedColumn {
                    column: 4,
                    rows: vec![3]
                }
            ]
        );
    }

    #[test]
    fn a_file_that_is_mostly_one_cell_rows_is_a_shape_not_an_anomaly() {
        // The sanity cap. Every row here is a single populated cell in a column
        // that holds nothing else -- so every row satisfies both conditions,
        // and acting on that would draft rules excluding the entire file. At
        // that point it is not an anomaly, it is what the file IS.
        let data = table(
            &["", "", "", ""],
            &[
                &["Alpha", "", "", ""],
                &["", "Bravo", "", ""],
                &["", "", "Charlie", ""],
                &["", "", "", "Delta"],
            ],
        );
        assert_eq!(isolated_columns(&data), Vec::new());
    }

    #[test]
    fn the_absolute_cap_bites_even_when_the_fraction_is_comfortable() {
        // The two caps are independent, and this pins the one the other test
        // cannot reach: nine isolated rows in a forty-row table is well inside
        // the quarter-of-the-file limit (9 * 4 = 36 <= 40) and still refused,
        // because nine is more than a report's worth of section labels and more
        // than a warning can name and stay checkable.
        let mut rows: Vec<Vec<String>> = (0..9)
            .map(|n| vec![format!("Section {n}"), String::new(), String::new()])
            .collect();
        rows.extend((0..31).map(|n| {
            vec![
                String::new(),
                format!("2026-01-{:02}", (n % 28) + 1),
                "-4.50".to_string(),
            ]
        }));
        let nine = Tabular {
            header: Some(vec!["".into(), "Date".into(), "Amount".into()]),
            rows,
            ..Tabular::default()
        };
        assert_eq!(nine.rows.len(), 40);
        assert_eq!(isolated_columns(&nine), Vec::new());

        // One fewer, and the same file is an anomaly again — so the refusal
        // above is the cap talking, not some other condition failing.
        let eight = Tabular {
            rows: nine.rows[1..].to_vec(),
            ..nine
        };
        assert_eq!(
            isolated_columns(&eight)
                .first()
                .map(|found| found.rows.len()),
            Some(8)
        );
    }

    #[test]
    fn a_narrow_table_carries_no_isolation_signal() {
        // `single-column.xlsx`'s lesson from `fixtures/import/README.md`, in
        // another lane: one populated cell out of two says nothing about
        // whether a row is a record.
        let data = table(
            &["", "Amount"],
            &[
                &["General Ledger", ""],
                &["", "-4.50"],
                &["", "-12.00"],
                &["", "2400.00"],
                &["", "-9.00"],
                &["", "-1.00"],
                &["", "-2.00"],
                &["", "-3.00"],
            ],
        );
        assert_eq!(isolated_columns(&data), Vec::new());
    }

    #[test]
    fn an_ordinary_export_has_no_isolated_cells() {
        let data = table(
            &["Date", "Description", "Amount"],
            &[
                &["2026-01-02", "COFFEE", "-4.50"],
                &["2026-01-03", "BOOKS", "-12.00"],
            ],
        );
        assert_eq!(isolated_columns(&data), Vec::new());
    }

    // -----------------------------------------------------------------------
    // What an isolated row must not be allowed to decide
    // -----------------------------------------------------------------------

    /// The user's shape with an amount-shaped label, and real money columns
    /// nothing in the synonym table recognises.
    fn label_beside_unrecognised_columns(label: &str) -> Tabular {
        table(
            &["", "Booked", "Details", "Gross"],
            &[
                &[label, "", "", ""],
                &["", "31/01/2026", "COFFEE", "-4.50"],
                &["", "28/02/2026", "BOOKS", "-12.00"],
                &["", "15/03/2026", "PAYROLL", "2400.00"],
            ],
        )
    }

    #[test]
    fn an_isolated_row_does_not_become_the_amount_column() {
        // The sharpest form of the skew. `1,234` reads as an amount, the real
        // money column is headed `Gross` which nothing recognises, and both
        // therefore claim `amount` from their VALUES at the same confidence --
        // where the tie is broken by position, and the label is column 1. The
        // draft would map the money to a column holding one label, declare a
        // `decimal-mark` from it, and leave the real amounts unmapped. Exit 0.
        let drafted = generate(&label_beside_unrecognised_columns("1,234"), "assets:b").unwrap();
        assert_eq!(
            column_for(&drafted.columns, HledgerField::Amount),
            Some(3),
            "the money is in `Gross`, not in the label"
        );
        assert_eq!(drafted.columns[0].field, None);
        assert_eq!(
            drafted.doc.settings().decimal_mark.map(|s| s.value),
            None,
            "the real amounts are plain, so no directive is owed"
        );
    }

    #[test]
    fn an_isolated_row_does_not_decide_the_date_format() {
        // Same shape, date-shaped label. Read with the label in, the mapped
        // date column is the label's, its single sample reads two ways, and the
        // draft declares an ambiguous month-first format over data that is
        // unambiguously day-first.
        let drafted =
            generate(&label_beside_unrecognised_columns("01/02/2026"), "assets:b").unwrap();
        assert_eq!(column_for(&drafted.columns, HledgerField::Date), Some(1));
        let guess = drafted.date_format.clone().expect("a format");
        assert_eq!(guess.format, "%d/%m/%Y");
        assert!(!guess.ambiguous, "some day is > 12 once the label is gone");
    }

    // -----------------------------------------------------------------------
    // The rule, and the sentence explaining it
    // -----------------------------------------------------------------------

    /// The reported file, as a `Tabular`.
    fn quickbooks_export() -> Tabular {
        table(
            &["", "Date", "Description", "Amount"],
            &[
                &["General Ledger", "", "", ""],
                &["", "2026-01-02", "COFFEE", "-4.50"],
                &["", "2026-01-03", "BOOKS", "-12.00"],
                &["", "2026-01-04", "PAYROLL", "2400.00"],
            ],
        )
    }

    #[test]
    fn the_draft_excludes_the_isolated_row_with_a_skip_block() {
        let drafted = generate(&quickbooks_export(), "assets:bank:checking").unwrap();
        assert_eq!(
            text(&drafted.doc),
            "skip 1\n\
             date-format %Y-%m-%d\n\
             fields column1, date, description, amount\n\
             account1 assets:bank:checking\n\
             account2 expenses:unknown\n\
             if %column1 .\n    \
             skip\n\n"
        );
        // The column is NAMED -- otherwise `%column1` refers to nothing and the
        // rule silently matches every row's whole record instead.
        assert_eq!(
            names(&drafted.doc),
            ["column1", "date", "description", "amount"]
        );
    }

    #[test]
    fn the_warning_names_the_row_the_value_and_the_rule() {
        let drafted = generate(&quickbooks_export(), "assets:bank:checking").unwrap();
        let warning = drafted
            .warnings
            .iter()
            .find(|warning| warning.contains("Only one cell"))
            .unwrap_or_else(|| panic!("{:?}", drafted.warnings));
        assert!(warning.contains("data row 1"), "{warning}");
        assert!(warning.contains("General Ledger"), "{warning}");
        assert!(warning.contains("if %column1 ."), "{warning}");
    }

    #[test]
    fn a_placeholder_name_never_collides_with_a_real_header() {
        // The machine name is `column1`, and a file may genuinely have a column
        // headed `Column 1`. hledger resolves two `fields` entries with the same
        // name as "first one wins", in silence — so a collision would either
        // lose the real column or leave the exclusion rule pointing at a name
        // that is not there, which makes `%column1` a whole-record regex
        // matching every row.
        let data = table(
            &["", "Date", "Column 1", "Amount"],
            &[
                &["General Ledger", "", "", ""],
                &["", "2026-01-02", "A", "-4.50"],
                &["", "2026-01-03", "B", "-12.00"],
                &["", "2026-01-04", "C", "2400.00"],
            ],
        );
        let drafted = generate(&data, "assets:bank:checking").unwrap();
        assert_eq!(
            names(&drafted.doc),
            ["column1x1", "date", "column1", "amount"]
        );
        assert!(
            text(&drafted.doc).contains("if %column1x1 .\n    skip\n"),
            "{}",
            text(&drafted.doc)
        );
    }

    #[test]
    fn an_ordinary_export_drafts_no_exclusion_rule() {
        let data = table(
            &["Date", "Description", "Amount"],
            &[&["2026-01-02", "COFFEE", "-4.50"]],
        );
        let drafted = generate(&data, "assets:bank:checking").unwrap();
        assert!(
            !text(&drafted.doc).contains("if "),
            "{}",
            text(&drafted.doc)
        );
        assert!(
            !drafted
                .warnings
                .iter()
                .any(|warning| warning.contains("Only one cell")),
            "{:?}",
            drafted.warnings
        );
    }

    #[test]
    fn a_refused_exclusion_is_also_a_row_the_guesses_still_read() {
        // The invariant that keeps a draft self-consistent: the rows left out
        // of the guessing are EXACTLY the rows the drafted rules leave out of
        // the import. When the cap refuses there is no rule, so the rows have
        // to be data again -- otherwise the draft describes settings read off a
        // file that is not the one hledger will be handed.
        let data = table(
            &["", "", "", ""],
            &[
                &["2026-01-02", "", "", ""],
                &["", "2026-02-03", "", ""],
                &["", "", "2026-03-04", ""],
                &["", "", "", "2026-04-05"],
            ],
        );
        assert_eq!(isolated_columns(&data), Vec::new(), "the cap refuses");
        let drafted = generate(&data, "assets:bank:checking").unwrap();
        assert!(
            !text(&drafted.doc).contains("if "),
            "{}",
            text(&drafted.doc)
        );
        assert_eq!(
            column_for(&drafted.columns, HledgerField::Date),
            Some(0),
            "column 1 is still data, and still reads as dates"
        );
    }

    #[test]
    fn a_draft_carrying_an_exclusion_is_still_writable_through_the_create_route() {
        // `every_drafted_item_can_be_written_back`'s question for the one item
        // kind this feature adds: an `ifBlock` the create `PUT` must be able to
        // name, since a draft carrying an unnameable item is one nobody can
        // save.
        let drafted = generate(&quickbooks_export(), "assets:bank:checking").unwrap();
        assert!(
            drafted
                .doc
                .items()
                .iter()
                .any(|item| matches!(item.kind, crate::rules::ItemKind::IfBlock(_))),
            "the exclusion is there to be checked"
        );
        for item in drafted.doc.items() {
            assert!(
                matches!(
                    item.kind,
                    crate::rules::ItemKind::Directive(_)
                        | crate::rules::ItemKind::Fields(_)
                        | crate::rules::ItemKind::Assignment(_)
                        | crate::rules::ItemKind::IfBlock(_)
                ),
                "a draft may not contain {:?}",
                item.kind
            );
        }
        let rendered = text(&drafted.doc);
        let reparsed = RulesDoc::parse(&rendered);
        assert_eq!(reparsed.text(), rendered);
        assert!(reparsed.warnings().is_empty(), "{:?}", reparsed.warnings());
    }

    #[test]
    fn a_mapped_balance_column_explains_what_it_will_do() {
        let data = table(
            &["Date", "Amount", "Running Balance"],
            &[&["2026-01-02", "-4.50", "995.50"]],
        );
        let drafted = generate(&data, "assets:bank:checking").unwrap();
        assert_eq!(column_for(&drafted.columns, HledgerField::Balance), Some(2));
        assert!(
            drafted
                .warnings
                .iter()
                .any(|w| w.contains("running-balance")),
            "{:?}",
            drafted.warnings
        );
    }
}
