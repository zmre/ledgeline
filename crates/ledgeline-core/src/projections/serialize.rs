//! Scenario ⇄ journal text (plan 22, Phase 3, §"The file format"; the piece
//! model is plan 31, Part C).
//!
//! A scenario file is an ordinary hledger journal. Reading one is
//! [`crate::parse::parse_journal`] over that file alone, a header scan, and a
//! scan of the file into PIECES ([`super::layout`]); writing one is this
//! module, and it is written **piece by piece**, never by re-rendering the
//! file.
//!
//! # Never round-trip through `hledger print`
//!
//! `hledger print` drops `~` rules entirely, so a file that went through it
//! would come back with every scenario line gone and nothing to say it had
//! happened. Ledgeline writes these files itself.
//!
//! # The discipline: a file is its pieces, and an unchanged piece keeps its bytes
//!
//! After the header, a file is a LEAD (the blank lines after the header), then
//! a list of pieces, each followed by its GAP (the blank lines up to the next
//! one). A piece is a `~` rule, an `=` rule, a commented-out ("disabled") one
//! of either, a free-standing comment, an hledger `comment` block, or FIXED
//! journal text this module does not model (`account`, `commodity`,
//! transactions, `P`, `include` …). A comment directly above a rule, with no
//! blank line and no divider, is part of that rule's piece: its ATTACHED
//! comment.
//!
//! The scenario's [`Scenario::layout`] lists the pieces in order, and a save
//! writes:
//!
//! 1. the **header** comment lines this module recognizes (the banner,
//!    `projection:`, `created:`, `updated:`, `cash:`), re-rendered — a user's
//!    own prose above or between them is a piece of its own and copied
//!    through;
//! 2. the original **lead**;
//! 3. each layout item's **body**, then its **separator**.
//!
//! An item CLAIMS the piece it was read from ([`ProjectionDoc::claim`]: by
//! origin and content, then content alone, then origin alone). A claimed piece
//! whose content is unchanged is written with **its own original bytes, even
//! when it has moved**; one whose only change is its disabled flag has those
//! bytes commented or uncommented line by line; anything else is rendered,
//! keeping the attached comment's bytes when that is unchanged. A claimed
//! piece keeps its own gap, plus a blank line when its neighbour changed and
//! it had none, so nothing glues onto what now follows it. Fixed text cannot
//! be added, dropped or reordered among itself.
//!
//! The renderer is careful, and the caller still re-parses the written text
//! and requires it to read back as the layout it asked for, in order
//! ([`write_scenario`]).
//!
//! A scenario with NO layout — a client from before plan 31, a seed, the FFI —
//! is written through [`legacy_layout`], which derives one with the old
//! per-kind slot filling (`~` rules and events fill the claimed `~` slots,
//! `=` rules the claimed `=` slots), every comment, disabled and fixed piece
//! kept where it is, new entries appended. That is the only home of the old
//! algorithm.
//!
//! ## Why not `PeriodicDoc::plan`/`apply`/`verify`
//!
//! Plan 22 Phase 3 says to reuse them. Their **edit vocabulary** cannot be —
//! it is `SetAmount` / `Delete` / `AppendLine` / `AppendBlock`, one written
//! amount at a time, built for a budget editor that changes a number inside a
//! rule somebody else authored. A scenario save rewrites a rule's period, its
//! description, its accounts and its `growth:`/`line:` tags, and `AppendBlock`
//! can only write a *bare* [`crate::model::PeriodExpr`] header — so
//! `~ monthly from 2027-04-01`, which is half the format, is not expressible
//! in it at all. See amendment 28. Where a block begins and ends is still the
//! parser's: [`super::layout`]'s extents are asserted equal to
//! [`crate::periodic::PeriodicDoc`]'s and [`crate::auto_edit::block_spans`]'.
//!
//! # The mapping between a file and a model
//!
//! | file | model |
//! |---|---|
//! | `~ monthly  projection` with N postings | N [`ScenarioLine`]s sharing `group: "rule:K"` |
//! | `~ 2027-03-01  Series A` | one [`ScenarioEvent`] with `id: "rule:K"` |
//! | `= QUERY` block | one [`ScenarioAutoRule`], `key: "auto:B"`, `block: Some(B)` |
//! | a commented-out `~` block (plan 31 C7) | its lines under `group: "off:K"` (or an event `off:K`), its layout item `disabled` |
//! | a commented-out `=` block | a [`ScenarioAutoRule`] `key: "auto:off:K"`, its layout item `disabled` |
//! | a free comment run / `comment` block | a [`ScenarioComment`] `comment:N` |
//! | anything else | a [`FixedPiece`] `fixed:N` |
//! | a posting's `; line: <id>` | that line's logical-row id (a step change's two segments share it) |
//! | a posting's `; growth: 3%/yr` | that line's [`Growth`] |
//! | a `; growth:` on an ASSET-typed account | [`LineRole::Asset`]: the amount is the CONTRIBUTION |
//! | an asset row's `; opening: 200000` | that row's balance override |
//! | a posting's other comment text | [`ScenarioLine::comment`] (plan 31 C6) |
//! | a rule header's comment | [`ScenarioLine::rule_comment`] / [`ScenarioEvent::comment`] |
//!
//! K is the block's index among the file's active `~` blocks (or its disabled
//! ones, for `off:K`), B among its active `=` blocks — the same spelling
//! [`super::seed_scenario`] gives the journal's own rules.
//!
//! # Why reading needs the account types
//!
//! The asset/flow discriminator is "carries `growth:` AND resolves to an asset
//! type" ([`super::lines_from_rule`]), and a projection file usually declares no
//! `account` directives of its own — its accounts are the MAIN journal's. So the
//! journal's declared types are handed in, with any the file declares itself
//! layered on top, and every classification goes through
//! [`crate::reports::account_types`] rather than through a name test. A Spanish
//! chart of accounts round-trips; `activo:banco` is an asset because the journal
//! says it is.
//!
//! A single-date rule becomes an EVENT rather than a one-off line, which is the
//! one place this module normalizes rather than preserves — see amendment 29.
//! The engine treats the two identically, and the tab shows both in the same
//! section, so nothing a user can see moves.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::ops::Range;

use crate::decimal::Dec;
use crate::edit::render_amount;
use crate::model::{
    AccountName, Amount, AutoPostingRule, PeriodKind, PeriodicTransaction, Posting,
};
use crate::parse::{self, ParseError};
use crate::periodic::{MAX_ACCOUNT_BYTES, MAX_DESCRIPTION_BYTES};
use crate::projections::discovery::parse_header;
use crate::projections::layout::{self, Confirmed, RawKind};
use crate::projections::{
    CommentStyle, DEFAULT_GAINS_ACCOUNT, FixedPiece, Growth, GrowthUnit, ItemKind, LayoutItem,
    LineRole, Scenario, ScenarioAutoRule, ScenarioComment, ScenarioEvent, ScenarioLine,
    is_ledgeline_tag, rule_counter, user_comment, virtual_posting,
};
use crate::reports::account_types::{
    AccountType, AccountTypes, account_decls_from, declared_types,
};
use crate::rules::Newline;


/// The indent a posting line gets. Four spaces, which is what hledger's own
/// documentation and every fixture in this repo use.
const INDENT: &str = "    ";

/// The banner Ledgeline writes at the top of a file it created. Display only —
/// nothing keys off it, and a file without it loads identically.
const BANNER: &str = "; Ledgeline projection";

/// The divider a NEW section heading is written under: Patrick's own hand
/// style, 22 semicolons, then `;; Title` (plan 31 C4). Existing headings keep
/// whatever framing they have.
const SECTION_DIVIDER: &str = ";;;;;;;;;;;;;;;;;;;;;;";

/// The longest note (a rule description) this module will write. The same cap
/// `periodic` puts on one, restated by reference rather than by number.
const MAX_NOTE_BYTES: usize = MAX_DESCRIPTION_BYTES;

/// The longest `; projection:` name, `created:` or `updated:` value written.
/// Matches the discovery scan's own header clip, so a name that survives a save
/// is one the listing can show.
const MAX_HEADER_VALUE_BYTES: usize = 200;

/// The longest `line:` tag value written. An id is a machine handle, not prose.
const MAX_ID_BYTES: usize = 200;

/// Why a scenario could not be written.
///
/// Every variant is a value that would make the written line read back as
/// something other than what was asked for — the failure a re-parse would catch
/// afterwards, reported up front with a sentence instead.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SerializeError {
    /// A field this module refuses to write, and why.
    #[error("{0}")]
    Invalid(String),
    /// The written text did not read back as the scenario requested. A bug in
    /// this module, surfaced rather than written to somebody's disk.
    #[error("the scenario did not read back as written: {0}")]
    NotRoundTripped(String),
    /// The written text is not a journal.
    #[error("the result is not a readable journal: {0}")]
    Unreadable(String),
}

fn invalid(what: &str) -> SerializeError {
    SerializeError::Invalid(what.to_string())
}

/// `text` with `\r\n` line ends written `\n`, and a final `\n` supplied when
/// it has text and none: the form every comparison here is made in.
fn canonical(text: &str) -> String {
    let lf = text.replace("\r\n", "\n");
    if lf.is_empty() || lf.ends_with('\n') {
        lf
    } else {
        lf + "\n"
    }
}

// ===========================================================================
// Reading
// ===========================================================================

/// Read a scenario out of a journal file's text.
///
/// `source_name` is the file's own path, and it is what any `include` in the
/// file resolves against — a user may point the tab at a journal that includes
/// another, and refusing to open it because it does would be refusing the ask's
/// "really any journal file with budget-like entries could be used".
///
/// `name` is the display name the caller already read from the header (the
/// discovery scan does it for the listing), falling back to the file's label.
/// The header is re-read here too, because a `GET` of one file must not depend
/// on a listing having happened first.
///
/// `declared` is the MAIN journal's account types, because a projection file
/// usually declares none of its own and the asset/flow discriminator needs them.
/// Anything the file DOES declare wins, so a self-contained scenario classifies
/// correctly with no journal open at all.
///
/// The scenario comes back with its [`Scenario::layout`]: every piece of the
/// file in order, its comments, its fixed text and its disabled entries.
///
/// # Errors
/// [`SerializeError::Unreadable`] when the text is not a journal.
pub fn scenario_from_text(
    text: &str,
    source_name: &str,
    fallback_name: &str,
    declared: &BTreeMap<String, crate::reports::account_types::AccountType>,
) -> Result<Scenario, SerializeError> {
    scenario_from_text_with_styles(text, source_name, fallback_name, declared, &[])
}

/// [`scenario_from_text`], reading amounts under the main journal's declared
/// commodity styles (see [`parse::parse_journal_with_styles`]): `$11,344` in a
/// scenario is eleven thousand dollars when the main journal declares
/// `commodity $1,000.00`, as it would be were the scenario `include`d.
///
/// # Errors
/// As [`scenario_from_text`].
pub fn scenario_from_text_with_styles(
    text: &str,
    source_name: &str,
    fallback_name: &str,
    declared: &BTreeMap<String, crate::reports::account_types::AccountType>,
    inherited: &[(crate::model::Commodity, crate::model::AmountStyle)],
) -> Result<Scenario, SerializeError> {
    read_scenario(text, source_name, fallback_name, declared, inherited).map(|read| read.scenario)
}

/// [`scenario_from_text_with_styles`], plus the account types the file itself
/// declares (`account hmrc:paye ; type: L`). A caller that signs the
/// scenario's numbers — a rule's percentages, an entry — merges them over the
/// main journal's with [`scenario_account_types`], as the read did.
///
/// # Errors
/// As [`scenario_from_text`].
pub fn scenario_and_own_types_from_text_with_styles(
    text: &str,
    source_name: &str,
    fallback_name: &str,
    declared: &BTreeMap<String, AccountType>,
    inherited: &[(crate::model::Commodity, crate::model::AmountStyle)],
) -> Result<(Scenario, BTreeMap<String, AccountType>), SerializeError> {
    read_scenario(text, source_name, fallback_name, declared, inherited)
        .map(|read| (read.scenario, read.own_types))
}

/// The account types a scenario classifies under: the main journal's
/// `declared`, with the scenario file's `own` declarations winning — so a
/// scenario that carries its declarations with it classifies the same way
/// whether or not a main journal is open.
#[must_use]
pub fn scenario_account_types(
    declared: &BTreeMap<String, AccountType>,
    own: &BTreeMap<String, AccountType>,
) -> BTreeMap<String, AccountType> {
    declared
        .iter()
        .chain(own)
        .map(|(account, ty)| (account.clone(), *ty))
        .collect()
}

/// What [`read_scenario`] learns from a file.
struct Read {
    scenario: Scenario,
    /// Every account the file itself declares.
    own_accounts: BTreeSet<String>,
    /// Every commodity the file itself declares a style for.
    own_commodities: BTreeSet<String>,
    /// The account types the file itself declares.
    own_types: BTreeMap<String, AccountType>,
    /// The blank lines between the header and the first piece.
    lead: Range<usize>,
    /// Every piece, in file order; index for index with the layout.
    pieces: Vec<Piece>,
}

/// A commented-out candidate the parser confirmed (plan 31 C7).
enum Disabled {
    Tilde(PeriodicTransaction),
    Auto(AutoPostingRule),
}

/// One piece of a file: where it is, and what it states.
#[derive(Debug, Clone)]
struct Piece {
    kind: ItemKind,
    disabled: bool,
    /// Its key as read.
    key: String,
    /// The whole piece: attached comment and content.
    span: Range<usize>,
    /// Its attached comment: a prefix of `span`, empty for none.
    attached: Range<usize>,
    /// The blank lines after it.
    gap: Range<usize>,
    /// A disabled piece's comment prefix, which every content line carries.
    prefix: Option<String>,
    /// What it states, as [`Entry::print`] prints it. Equal prints mean
    /// writing one over the other would change nothing.
    print: Vec<String>,
}

impl Piece {
    /// The piece without its attached comment.
    fn content(&self) -> Range<usize> {
        self.attached.end..self.span.end
    }
}

/// [`scenario_from_text`], plus every account the text itself declares — what
/// the writer needs to know before it declares a gains account (Decision 4) —
/// and the file's pieces.
fn read_scenario(
    text: &str,
    source_name: &str,
    fallback_name: &str,
    declared: &BTreeMap<String, crate::reports::account_types::AccountType>,
    inherited: &[(crate::model::Commodity, crate::model::AmountStyle)],
) -> Result<Read, SerializeError> {
    let journal = parse::parse_journal_with_styles(text, source_name, inherited)
        .map_err(|error| SerializeError::Unreadable(parse_message(&error)))?;
    // Only the rules this FILE wrote. A journal that includes another would
    // otherwise hand back the includee's rules as rows the save path would then
    // try to write into this file's blocks.
    let own: Vec<&PeriodicTransaction> = journal
        .periodic_transactions
        .iter()
        .filter(|rule| rule.source_file.as_os_str() == std::ffi::OsStr::new(source_name))
        .collect();
    // The fallback, and the condition that makes it safe.
    //
    // A parse whose `source_file` spelling does not match the name handed in (a
    // relative name, a temp path) would produce an EMPTY scenario, which reads
    // as "this file has no lines" and would then be saved back over the ones it
    // does have. So an empty match falls back to every rule — but ONLY when
    // this file is the whole journal. With an `include` in play, "every rule"
    // would include the includee's, and the save path would then try to write
    // another file's rules into this one's blocks.
    let rules: Vec<PeriodicTransaction> = if own.is_empty() && journal.source_files.len() <= 1 {
        journal.periodic_transactions.clone()
    } else {
        own.into_iter().cloned().collect()
    };

    let own_types = declared_types(&account_decls_from(&journal.accounts));
    let types = AccountTypes::from_declared(scenario_account_types(declared, &own_types));

    let header = parse_header(text);
    let own_accounts = journal
        .accounts
        .iter()
        .map(|account| account.name.0.clone())
        .collect();
    // The file's own `=` rules, by the same rule and the same fallback as its
    // `~` rules above: never an includee's.
    let mine =
        |rule: &&AutoPostingRule| rule.source_file.as_os_str() == std::ffi::OsStr::new(source_name);
    let auto_rules: Vec<AutoPostingRule> =
        if !journal.auto_rules.iter().any(|rule| mine(&rule)) && journal.source_files.len() <= 1 {
            journal.auto_rules.clone()
        } else {
            journal.auto_rules.iter().filter(mine).cloned().collect()
        };

    // The pieces. A commented-out candidate is a disabled entry only when its
    // stripped text, parsed ALONE under the same styles, is exactly one `~` or
    // exactly one `=` rule (C7); anything else is prose.
    let mut disabled: VecDeque<Disabled> = VecDeque::new();
    let scan = layout::scan(text, header_span(text).end, &mut |stripped| {
        let parsed = parse::parse_journal_with_styles(stripped, source_name, inherited).ok()?;
        if !parsed.transactions.is_empty() {
            return None;
        }
        let mut periodic = parsed.periodic_transactions;
        let mut autos = parsed.auto_rules;
        match (periodic.len(), autos.len()) {
            (1, 0) => {
                disabled.push_back(Disabled::Tilde(periodic.pop()?));
                Some(Confirmed::Tilde)
            }
            (0, 1) => {
                disabled.push_back(Disabled::Auto(autos.pop()?));
                Some(Confirmed::Auto)
            }
            _ => None,
        }
    });
    let count = |kind: RawKind| {
        scan.pieces
            .iter()
            .filter(|piece| piece.kind == kind)
            .count()
    };
    if count(RawKind::Tilde) != rules.len() || count(RawKind::Auto) != auto_rules.len() {
        return Err(SerializeError::Unreadable(format!(
            "the file's {} `~` and {} `=` blocks could not be lined up with the {} and {} rules it \
             reads as",
            count(RawKind::Tilde),
            count(RawKind::Auto),
            rules.len(),
            auto_rules.len()
        )));
    }

    let mut scenario = Scenario {
        name: header
            .name
            .clone()
            .unwrap_or_else(|| fallback_name.to_string()),
        created: header.created.clone(),
        updated: header.updated.clone(),
        cash: header.cash.clone().map(AccountName),
        ..Scenario::default()
    };
    let mut rules = rules.into_iter();
    let mut autos = auto_rules.into_iter();
    let mut counters = PieceCounters::default();
    let mut pieces = Vec::with_capacity(scan.pieces.len());
    for raw in &scan.pieces {
        let missing = || SerializeError::Unreadable("a block was read twice".to_string());
        let (kind, key, print) = match raw.kind {
            RawKind::Tilde => {
                let rule = rules.next().ok_or_else(missing)?;
                let key = group_key(counters.next(Counter::Tilde));
                let (kind, print) = scenario.push_tilde(&key, &rule, &types);
                (kind, key, print)
            }
            RawKind::DisabledTilde => {
                let Some(Disabled::Tilde(rule)) = disabled.pop_front() else {
                    return Err(missing());
                };
                let key = format!("off:{}", counters.next(Counter::OffTilde));
                let (kind, print) = scenario.push_tilde(&key, &rule, &types);
                (kind, key, print)
            }
            RawKind::Auto | RawKind::DisabledAuto => {
                let (rule, block, key) = if raw.kind == RawKind::Auto {
                    let block = counters.next(Counter::Auto);
                    (
                        autos.next().ok_or_else(missing)?,
                        Some(block),
                        format!("auto:{block}"),
                    )
                } else {
                    let Some(Disabled::Auto(rule)) = disabled.pop_front() else {
                        return Err(missing());
                    };
                    (
                        rule,
                        None,
                        format!("auto:off:{}", counters.next(Counter::OffAuto)),
                    )
                };
                let print = vec![canonical(&rule.raw)];
                scenario.auto_rules.push(ScenarioAutoRule {
                    block,
                    key: key.clone(),
                    rule,
                });
                (ItemKind::Auto, key, print)
            }
            RawKind::Note | RawKind::Section | RawKind::Block => {
                let key = format!("comment:{}", counters.next(Counter::Comment));
                let raw_text = canonical(&text[raw.span.clone()]);
                scenario.comments.push(ScenarioComment {
                    key: key.clone(),
                    style: match raw.kind {
                        RawKind::Note => CommentStyle::Note,
                        RawKind::Section => CommentStyle::Section,
                        _ => CommentStyle::Block,
                    },
                    raw: raw_text.clone(),
                });
                (ItemKind::Comment, key, vec![raw_text])
            }
            RawKind::Fixed => {
                let key = format!("fixed:{}", counters.next(Counter::Fixed));
                let raw_text = canonical(&text[raw.span.clone()]);
                scenario.fixed.push(FixedPiece {
                    key: key.clone(),
                    raw: raw_text.clone(),
                });
                (ItemKind::Fixed, key, vec![raw_text])
            }
        };
        let disabled = matches!(raw.kind, RawKind::DisabledTilde | RawKind::DisabledAuto);
        scenario.layout.push(LayoutItem {
            kind,
            key: key.clone(),
            origin: Some(pieces.len()),
            disabled,
            attached: canonical(&text[raw.attached.clone()]),
        });
        pieces.push(Piece {
            kind,
            disabled,
            key,
            span: raw.span.clone(),
            attached: raw.attached.clone(),
            gap: raw.gap.clone(),
            prefix: raw.prefix.clone(),
            print,
        });
    }

    Ok(Read {
        scenario,
        own_accounts,
        own_commodities: journal
            .commodity_styles
            .iter()
            .map(|(commodity, _)| commodity.0.clone())
            .collect(),
        own_types,
        lead: scan.lead,
        pieces,
    })
}

/// The per-kind ordinals keys are made of.
#[derive(Default)]
struct PieceCounters([usize; 6]);

#[derive(Clone, Copy)]
enum Counter {
    Tilde,
    OffTilde,
    Auto,
    OffAuto,
    Comment,
    Fixed,
}

impl PieceCounters {
    /// The next ordinal of `counter`, counting it.
    fn next(&mut self, counter: Counter) -> usize {
        let slot = &mut self.0[counter as usize];
        *slot += 1;
        *slot - 1
    }
}

impl Scenario {
    /// Add one `~` rule under `key`: an event when it is the single-date form,
    /// else a group of lines. What kind it became, and what it states.
    fn push_tilde(
        &mut self,
        key: &str,
        rule: &PeriodicTransaction,
        types: &AccountTypes,
    ) -> (ItemKind, Vec<String>) {
        match event_date(rule) {
            Some(date) => {
                let event = event_from_rule(key, date, rule);
                let print = chunk_print(&Chunk::Event(&event), &Tags::Literal).unwrap_or_default();
                self.events.push(event);
                (ItemKind::Event, print)
            }
            None => {
                let lines = super::lines_from_rule(key, rule, types);
                let print = chunk_print(
                    &Chunk::Rule {
                        key,
                        lines: lines.iter().collect(),
                    },
                    &Tags::Literal,
                )
                .unwrap_or_default();
                self.lines.extend(lines);
                (ItemKind::Rule, print)
            }
        }
    }
}

/// A single-date `~` rule as a [`ScenarioEvent`].
fn event_from_rule(key: &str, date: String, rule: &PeriodicTransaction) -> ScenarioEvent {
    // The elided posting is the counter, not a stated leg.
    let stated: Vec<&Posting> = rule
        .postings
        .iter()
        .enumerate()
        .filter(|(index, _)| Some(*index) != rule.elided)
        .map(|(_, posting)| posting)
        .collect();
    ScenarioEvent {
        id: key.to_string(),
        date,
        description: rule.description.clone(),
        // Flattened to one posting per amount, exactly as the wire flattens an
        // event on the way out — so a scenario that came from a file and one
        // that came back from a browser have the same shape.
        postings: stated
            .iter()
            .flat_map(|posting| {
                posting
                    .amounts
                    .iter()
                    .map(|amount| virtual_posting(posting.account.clone(), amount.clone()))
            })
            .collect(),
        // The block's amountless posting, as for a recurring rule.
        counter: rule_counter(rule),
        comment: user_comment(&rule.comment),
        posting_comments: event_posting_comments(&stated),
    }
}

/// Each flattened posting's comment: a posting's comment goes with its FIRST
/// amount, since a save writes one row per amount and the comment once.
/// Trailing empty ones are dropped, so an event with no comments has none.
fn event_posting_comments(stated: &[&Posting]) -> Vec<String> {
    let mut comments: Vec<String> = stated
        .iter()
        .flat_map(|posting| {
            (0..posting.amounts.len()).map(|at| match at {
                0 => user_comment(&posting.comment),
                _ => String::new(),
            })
        })
        .collect();
    while comments.last().is_some_and(String::is_empty) {
        comments.pop();
    }
    comments
}

/// The date a rule fires on, when it is the single-date form (`~ 2027-03-01`)
/// and therefore a one-off event.
///
/// Deliberately narrow: only a `raw` that IS an ISO date, not every
/// [`PeriodKind::Once`]. `~ from 2027-01-01 to 2027-02-01` also fires once, and
/// rewriting it as `~ 2027-01-01` would be changing a rule the user wrote into
/// a different one that happens to behave the same — so it stays a line and its
/// `raw` is preserved. This is the same test the tab's own
/// `scenarioModel.sectionOfLine` applies.
fn event_date(rule: &PeriodicTransaction) -> Option<String> {
    if rule.period.kind != PeriodKind::Once || !is_iso_date(&rule.period.raw) {
        return None;
    }
    rule.period.start.clone()
}

/// `YYYY-MM-DD`, structurally. Range validation is the parser's; this only
/// decides which of two spellings of "fires once" the file used.
fn is_iso_date(raw: &str) -> bool {
    let bytes = raw.as_bytes();
    bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && [0, 1, 2, 3, 5, 6, 8, 9]
            .iter()
            .all(|&i| bytes[i].is_ascii_digit())
}

/// The group id one `~` block gets: its 0-based index in the file.
fn group_key(index: usize) -> String {
    format!("rule:{index}")
}

/// The block index a group id names, when it names one.
fn block_of(key: &str) -> Option<usize> {
    key.strip_prefix("rule:")?.parse().ok()
}

/// A [`ParseError`]'s message, with nothing path-shaped carried out of it.
///
/// `ParseError::Located` names the file it was reading, and that name is an
/// absolute path this crate's callers must not echo. Only the line is kept,
/// which is the half a user can act on.
fn parse_message(error: &ParseError) -> String {
    match error {
        ParseError::Located { line, message, .. } => format!("line {line}: {message}"),
        other => other.to_string(),
    }
}

// ===========================================================================
// Comments as text (plan 31 C4)
// ===========================================================================

/// A comment piece's DISPLAY text: its prefixes stripped (the leading run of
/// comment characters and one space), and for a section its divider lines
/// dropped. A `comment` block shows verbatim.
#[must_use]
pub fn comment_text(style: CommentStyle, raw: &str) -> String {
    let raw = canonical(raw);
    match style {
        CommentStyle::Block => raw.trim_end_matches('\n').to_string(),
        CommentStyle::Note | CommentStyle::Section => raw
            .lines()
            .filter(|line| style == CommentStyle::Note || !layout::is_divider(line))
            .map(strip_comment_prefix)
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

/// `;; Title` → `Title`.
fn strip_comment_prefix(line: &str) -> &str {
    let rest = line.trim_start_matches([';', '#', '*']);
    rest.strip_prefix(' ').unwrap_or(rest)
}

/// The file text of a comment piece whose display text is `text`.
///
/// - A **note** is written `; ` plus each line (`;` alone for an empty one).
/// - A **section** edited from `previous` keeps that heading's divider lines
///   and framing, and rewrites only its other lines, in the prefix its first
///   one had. A NEW section is Patrick's hand style: 22 semicolons, then
///   `;; Title` (plan 31 C4).
/// - A **block** is `comment`, the text, `end comment`.
///
/// # Errors
/// [`SerializeError::Invalid`] for text that would not read back as that
/// style: a control character, a note line that is a divider, an empty
/// section, a block containing `end comment`.
pub fn comment_raw(
    style: CommentStyle,
    text: &str,
    previous: Option<&str>,
) -> Result<String, SerializeError> {
    let text = text.replace("\r\n", "\n");
    if text
        .chars()
        .any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        return Err(invalid("a comment may not contain a control character"));
    }
    let lines: Vec<&str> = text.trim_end_matches('\n').split('\n').collect();
    let raw = match style {
        CommentStyle::Block => {
            if lines.iter().any(|line| line.trim() == "end comment") {
                return Err(invalid(
                    "a comment block may not contain a line `end comment`, which would end it",
                ));
            }
            let mut out = String::from("comment\n");
            for line in lines.iter().filter(|_| !text.is_empty()) {
                out.push_str(line);
                out.push('\n');
            }
            out.push_str("end comment\n");
            out
        }
        CommentStyle::Note => lines
            .iter()
            .map(|line| prefixed("; ", line))
            .collect::<String>(),
        CommentStyle::Section => {
            let words: Vec<&str> = lines
                .iter()
                .copied()
                .filter(|line| !line.trim().is_empty())
                .collect();
            if words.is_empty() {
                return Err(invalid("a section heading needs a title"));
            }
            section_raw(previous, &words)
        }
    };
    let read_as = comment_style_of(&raw);
    if read_as != Some(style) {
        return Err(invalid(&format!(
            "that text would read back as a {} rather than a {}: {}",
            read_as.map_or("rule", CommentStyle::as_str),
            style.as_str(),
            match style {
                CommentStyle::Note =>
                    "a note may not hold a divider line (three or more of - = _ ~ * # ;)",
                _ => "change the text",
            }
        )));
    }
    Ok(raw)
}

/// `prefix` + `line`, trimmed of trailing space; `;` alone for an empty line.
fn prefixed(prefix: &str, line: &str) -> String {
    let line = line.trim_end();
    if line.is_empty() {
        format!("{}\n", prefix.trim_end())
    } else {
        format!("{prefix}{line}\n")
    }
}

/// A section heading's raw text: `previous`'s dividers and framing with its
/// other lines replaced by `words`, or a new heading in the house style.
fn section_raw(previous: Option<&str>, words: &[&str]) -> String {
    let previous = previous.map(canonical).unwrap_or_default();
    let old: Vec<&str> = previous.lines().collect();
    if !old.iter().any(|line| layout::is_divider(line)) {
        let mut out = format!("{SECTION_DIVIDER}\n");
        out.extend(words.iter().map(|word| prefixed(";; ", word)));
        return out;
    }
    // The prefix the heading's own words were written in, `;; ` by default.
    let prefix =
        old.iter()
            .find(|line| !layout::is_divider(line))
            .map_or(";; ".to_string(), |line| {
                let body = strip_comment_prefix(line);
                line[..line.len() - body.len()].to_string()
            });
    let prefix = if prefix.trim().is_empty() {
        ";; ".to_string()
    } else if prefix.ends_with(' ') {
        prefix
    } else {
        format!("{prefix} ")
    };
    let first_word = old
        .iter()
        .position(|line| !layout::is_divider(line))
        .unwrap_or(1.min(old.len()));
    let mut out = String::new();
    for (at, line) in old.iter().enumerate() {
        if at == first_word {
            out.extend(words.iter().map(|word| prefixed(&prefix, word)));
        }
        if layout::is_divider(line) {
            out.push_str(line);
            out.push('\n');
        }
    }
    if first_word >= old.len() {
        out.extend(words.iter().map(|word| prefixed(&prefix, word)));
    }
    out
}

/// What `raw`, alone, reads as: a note, a section or a block — or `None`
/// when it is not one comment piece at all (blank lines, a rule, or a
/// commented-out rule).
fn comment_style_of(raw: &str) -> Option<CommentStyle> {
    let scan = layout::scan(raw, 0, &mut |stripped| {
        let parsed = parse::parse_journal(stripped, "comment.journal").ok()?;
        match (parsed.periodic_transactions.len(), parsed.auto_rules.len()) {
            (1, 0) => Some(Confirmed::Tilde),
            (0, 1) => Some(Confirmed::Auto),
            _ => None,
        }
    });
    match scan.pieces.as_slice() {
        [piece] if piece.gap.is_empty() && scan.lead.is_empty() => match piece.kind {
            RawKind::Note => Some(CommentStyle::Note),
            RawKind::Section => Some(CommentStyle::Section),
            RawKind::Block => Some(CommentStyle::Block),
            _ => None,
        },
        _ => None,
    }
}

/// The ATTACHED comment written above an entry whose comment reads `text`:
/// `; ` plus each line. Empty text is no comment.
///
/// # Errors
/// [`SerializeError::Invalid`] for text that would not stay attached: a
/// control character, or a divider line (which would make it a section).
pub fn attached_raw(text: &str) -> Result<String, SerializeError> {
    let text = text.replace("\r\n", "\n");
    let text = text.trim_end_matches('\n');
    if text.trim().is_empty() {
        return Ok(String::new());
    }
    let raw = comment_raw(CommentStyle::Note, text, None)?;
    check_attached(&raw)?;
    Ok(raw)
}

/// Refuse an attached comment that would not read back attached: every line a
/// column-1 comment, none blank, none a divider.
fn check_attached(raw: &str) -> Result<(), SerializeError> {
    for line in canonical(raw).lines() {
        if !line.starts_with([';', '#', '*']) {
            return Err(invalid(
                "every line of an attached comment must start with `;`, `#` or `*`",
            ));
        }
        if line.chars().any(|c| c.is_control() && c != '\t') {
            return Err(invalid(
                "an attached comment may not contain a control character",
            ));
        }
        if layout::is_divider(line) {
            return Err(invalid(
                "an attached comment may not hold a divider line (three or more of - = _ ~ * # \
                 ;): that would make it a section heading, which is never attached",
            ));
        }
    }
    Ok(())
}

// ===========================================================================
// Writing
// ===========================================================================

/// One projection file's text, cut into its pieces, with what each states.
///
/// Immutable: [`ProjectionDoc::apply`] returns a new `String` and never mutates
/// `self`, so a refused write cannot leave half a document behind.
#[derive(Debug, Clone)]
pub struct ProjectionDoc {
    text: String,
    newline: Newline,
    /// The blank lines between the header (the leading run of RECOGNIZED
    /// header comment lines, [`header_span`]) and the first piece. A save
    /// re-renders the header and keeps these.
    lead: Range<usize>,
    /// Every piece, in file order (plan 31 C5).
    pieces: Vec<Piece>,
    /// The scenario as this file states it, keys as read: what
    /// [`legacy_layout`] restores the pieces a layout-less client never sent
    /// from (comments, fixed text, disabled entries).
    read: Scenario,
    /// The account types this document was read under, carried so that
    /// [`confirm_round_trip`] re-reads the written text the same way. Reading
    /// the result under a different classification would compare an asset row
    /// against the flow line it was not.
    declared: BTreeMap<String, crate::reports::account_types::AccountType>,
    /// Every account the FILE itself declares, so a gains account it already
    /// declares is not declared twice.
    own_accounts: BTreeSet<String>,
    /// The main journal's declared commodity styles, which this file's amounts
    /// are read under (see [`scenario_from_text_with_styles`]), and which a
    /// save declares in the file for every commodity it uses but does not
    /// declare itself — so `hledger -f main -f this` reads it the same way.
    inherited: Vec<(crate::model::Commodity, crate::model::AmountStyle)>,
    /// Every commodity the FILE itself declares.
    own_commodities: BTreeSet<String>,
}

impl ProjectionDoc {
    /// Cut an existing file into its pieces, and read what each says.
    ///
    /// `source_name` is the file's own path, and `declared` the account types,
    /// for the same reasons [`scenario_from_text`] takes them.
    ///
    /// # Errors
    /// [`SerializeError::Unreadable`] when the text is not a journal.
    pub fn parse(
        text: &str,
        source_name: &str,
        declared: &BTreeMap<String, crate::reports::account_types::AccountType>,
    ) -> Result<Self, SerializeError> {
        Self::parse_with_styles(text, source_name, declared, &[])
    }

    /// [`parse`](Self::parse), under the main journal's declared commodity
    /// styles (see [`scenario_from_text_with_styles`]).
    ///
    /// # Errors
    /// As [`parse`](Self::parse).
    pub fn parse_with_styles(
        text: &str,
        source_name: &str,
        declared: &BTreeMap<String, crate::reports::account_types::AccountType>,
        inherited: &[(crate::model::Commodity, crate::model::AmountStyle)],
    ) -> Result<Self, SerializeError> {
        let Read {
            scenario,
            own_accounts,
            own_commodities,
            lead,
            pieces,
            ..
        } = read_scenario(text, source_name, "", declared, inherited)?;
        Ok(Self {
            newline: Newline::detect(text),
            lead,
            pieces,
            read: scenario,
            text: text.to_string(),
            declared: declared.clone(),
            own_accounts,
            inherited: inherited.to_vec(),
            own_commodities,
        })
    }

    /// An empty document, for a create.
    #[must_use]
    pub fn empty(declared: &BTreeMap<String, crate::reports::account_types::AccountType>) -> Self {
        Self {
            text: String::new(),
            newline: Newline::Lf,
            lead: 0..0,
            pieces: Vec::new(),
            read: Scenario::default(),
            declared: declared.clone(),
            own_accounts: BTreeSet::new(),
            inherited: Vec::new(),
            own_commodities: BTreeSet::new(),
        }
    }

    /// This document, reading and declaring under `inherited` — what a create
    /// needs, since an empty document has nothing of its own to parse.
    #[must_use]
    pub fn inheriting(
        mut self,
        inherited: &[(crate::model::Commodity, crate::model::AmountStyle)],
    ) -> Self {
        self.inherited = inherited.to_vec();
        self
    }

    /// The exact text of piece `origin` (a [`LayoutItem::origin`]), attached
    /// comment included, `\n` line ends — what an unchanged piece is written
    /// as (plan 31 C15's `raw`). `None` for a piece the file does not have.
    #[must_use]
    pub fn piece_text(&self, origin: usize) -> Option<String> {
        self.pieces
            .get(origin)
            .map(|piece| canonical(&self.text[piece.span.clone()]))
    }

    /// How many pieces the file has.
    #[must_use]
    pub fn piece_count(&self) -> usize {
        self.pieces.len()
    }

    /// Render `scenario` into this document (plan 31 C5): the header, the
    /// lead, then each layout item's body and separator. A scenario with no
    /// layout is written through [`legacy_layout`].
    ///
    /// # Errors
    /// [`SerializeError::Invalid`] for a field that cannot be written, or a
    /// layout that does not name the scenario's entries exactly once or moves
    /// the file's fixed text.
    pub fn apply(&self, scenario: &Scenario) -> Result<String, SerializeError> {
        self.render(&self.resolve(scenario)).map(|(text, _)| text)
    }

    /// `scenario` with a layout: its own, or [`legacy_layout`]'s.
    fn resolve<'a>(&self, scenario: &'a Scenario) -> Cow<'a, Scenario> {
        if scenario.layout.is_empty() {
            Cow::Owned(legacy_layout(self, scenario))
        } else {
            Cow::Borrowed(scenario)
        }
    }

    /// The written text, and what it must read back as ([`layout_prints`]'
    /// rows) for [`confirm_round_trip`].
    fn render(&self, scenario: &Scenario) -> Result<(String, Vec<String>), SerializeError> {
        let nl = self.newline.as_str();
        let entries = Entries::of(scenario)?;
        let values = line_tag_values(scenario);
        let tags = Tags::Assigned(&values);
        let mut plan: Vec<Planned<'_>> = scenario
            .layout
            .iter()
            .map(|item| {
                let entry = entries.resolve(item)?;
                let print = entry.print(&tags)?;
                Ok(Planned {
                    item: Cow::Borrowed(item),
                    entry,
                    print,
                    claim: None,
                })
            })
            .collect::<Result<_, SerializeError>>()?;
        let claims = self.claim(
            &scenario.layout,
            &plan
                .iter()
                .map(|planned| planned.print.clone())
                .collect::<Vec<_>>(),
        );
        for (planned, claim) in plan.iter_mut().zip(claims) {
            planned.claim = claim;
        }
        self.keep_fixed(&mut plan);
        self.check_fixed(&plan)?;

        // Every body rendered before a byte is assembled, so an invalid field
        // refuses the whole save rather than half of it.
        let mut body = self.text[self.lead.clone()].to_string();
        let mut want = Vec::new();
        let claims: Vec<Option<usize>> = plan.iter().map(|planned| planned.claim).collect();
        let kinds: Vec<ItemKind> = plan.iter().map(|planned| planned.item.kind).collect();
        for (at, planned) in plan.iter().enumerate() {
            let piece = planned.claim.map(|index| &self.pieces[index]);
            push_terminated(
                &mut body,
                &self.body(&planned.item, &planned.entry, &planned.print, piece, &tags)?,
                nl,
            );
            let separator = self.separator(at, &kinds, &claims);
            if at + 1 == plan.len() {
                body.push_str(separator);
            } else {
                push_terminated(&mut body, separator, nl);
            }
            want.extend(item_print(
                &planned.item,
                &planned.entry,
                &planned.print,
                piece.map(|piece| canonical(&self.text[piece.span.clone()])),
            ));
        }
        // The file's own final-newline convention.
        if !self.text.is_empty() && !self.text.ends_with('\n') {
            body.truncate(body.trim_end_matches(['\r', '\n']).len());
        }
        let (mut out, declarations) = self.render_header(scenario, &body)?;
        // Declarations the header wrote read back as the file's first piece
        // of fixed text.
        if let Some(declarations) = declarations {
            want.insert(0, format!("fixed | {:?}", canonical(&declarations)));
        }
        out.push_str(&body);
        Ok((out, want))
    }

    /// Weave every piece of fixed text the layout does not name back in where
    /// it is in the file: before the first item written from a later piece.
    ///
    /// A client never deletes fixed text (it is not editable), so a layout
    /// without one is a stale layout — typically one read before a save that
    /// declared a gains account or a commodity under the header — and the
    /// text stays rather than the save being refused.
    fn keep_fixed<'a>(&'a self, plan: &mut Vec<Planned<'a>>) {
        let named: BTreeSet<usize> = plan.iter().filter_map(|planned| planned.claim).collect();
        for (index, piece) in self.pieces.iter().enumerate() {
            if piece.kind != ItemKind::Fixed || named.contains(&index) {
                continue;
            }
            let Some(fixed) = self.read.fixed.iter().find(|fixed| fixed.key == piece.key) else {
                continue;
            };
            let at = plan
                .iter()
                .position(|planned| planned.claim.is_some_and(|claim| claim > index))
                .unwrap_or(plan.len());
            plan.insert(
                at,
                Planned {
                    item: Cow::Owned(LayoutItem {
                        kind: ItemKind::Fixed,
                        key: piece.key.clone(),
                        origin: Some(index),
                        disabled: false,
                        attached: String::new(),
                    }),
                    entry: Entry::Fixed(fixed),
                    print: piece.print.clone(),
                    claim: Some(index),
                },
            );
        }
    }

    /// Which piece each layout item CLAIMS (plan 31 C9), most certain first:
    ///
    /// 1. its origin, holding the same content;
    /// 2. any unclaimed piece of its kind holding the same content — an
    ///    unchanged entry whose origin went stale;
    /// 3. its origin alone — an edited entry.
    ///
    /// Kinds must match, with disabled and active of one kind, and recurring
    /// and single-date `~` rules, counted as the same kind. A claim decides
    /// only which bytes may be reused; correctness is
    /// [`confirm_round_trip`]'s.
    fn claim(&self, layout: &[LayoutItem], prints: &[Vec<String>]) -> Vec<Option<usize>> {
        let fits = |item: &LayoutItem, index: usize| {
            self.pieces
                .get(index)
                .is_some_and(|piece| kind_class(piece.kind) == kind_class(item.kind))
        };
        let mut taken = vec![false; self.pieces.len()];
        let mut claims: Vec<Option<usize>> = vec![None; layout.len()];
        for (at, item) in layout.iter().enumerate() {
            if let Some(origin) = item.origin
                && fits(item, origin)
                && !taken[origin]
                && self.pieces[origin].print == prints[at]
            {
                taken[origin] = true;
                claims[at] = Some(origin);
            }
        }
        let mut by_print: BTreeMap<(u8, &[String]), VecDeque<usize>> = BTreeMap::new();
        for (index, piece) in self.pieces.iter().enumerate() {
            if !taken[index] {
                by_print
                    .entry((kind_class(piece.kind), piece.print.as_slice()))
                    .or_default()
                    .push_back(index);
            }
        }
        for (at, item) in layout.iter().enumerate() {
            if claims[at].is_some() {
                continue;
            }
            if let Some(queue) = by_print.get_mut(&(kind_class(item.kind), prints[at].as_slice())) {
                while let Some(index) = queue.pop_front() {
                    if !taken[index] {
                        taken[index] = true;
                        claims[at] = Some(index);
                        break;
                    }
                }
            }
        }
        for (at, item) in layout.iter().enumerate() {
            if claims[at].is_none()
                && let Some(origin) = item.origin
                && fits(item, origin)
                && !taken[origin]
            {
                taken[origin] = true;
                claims[at] = Some(origin);
            }
        }
        claims
    }

    /// Fixed text stays put (plan 31 C5): every fixed item is a piece of
    /// this file, and they keep their order among themselves.
    fn check_fixed(&self, plan: &[Planned<'_>]) -> Result<(), SerializeError> {
        let mut last: Option<usize> = None;
        for planned in plan
            .iter()
            .filter(|planned| planned.item.kind == ItemKind::Fixed)
        {
            let Some(index) = planned.claim else {
                return Err(invalid(&format!(
                    "`{}` is file text this file does not hold; file text can only stay where it \
                     is in the file it was read from",
                    planned.item.key
                )));
            };
            if last.is_some_and(|last| index < last) {
                return Err(invalid(
                    "file text (account declarations, transactions, directives) keeps its order; \
                     other entries can move past it, but it cannot be reordered",
                ));
            }
            last = Some(index);
        }
        Ok(())
    }

    /// One layout item's text, per plan 31 C5's body rule.
    fn body(
        &self,
        item: &LayoutItem,
        entry: &Entry<'_>,
        print: &[String],
        piece: Option<&Piece>,
        tags: &Tags<'_>,
    ) -> Result<String, SerializeError> {
        let nl = self.newline.as_str();
        let unchanged = piece.filter(|piece| piece.print == print);
        match entry {
            Entry::Fixed(_) => piece
                .map(|piece| self.text[piece.span.clone()].to_string())
                .ok_or_else(|| invalid("file text cannot be added by a save")),
            Entry::Comment(comment) => {
                if let Some(piece) = unchanged {
                    return Ok(self.text[piece.span.clone()].to_string());
                }
                if comment_style_of(&canonical(&comment.raw)) != Some(comment.style) {
                    return Err(invalid(&format!(
                        "comment `{}` does not read back as a {}",
                        comment.key,
                        comment.style.as_str()
                    )));
                }
                Ok(with_newline(&canonical(&comment.raw), nl))
            }
            Entry::Chunk(_) | Entry::Auto(_) => {
                let attached = match piece {
                    Some(piece)
                        if canonical(&self.text[piece.attached.clone()])
                            == canonical(&item.attached) =>
                    {
                        self.text[piece.attached.clone()].to_string()
                    }
                    _ => {
                        let raw = canonical(&item.attached);
                        check_attached(&raw)?;
                        with_newline(&raw, nl)
                    }
                };
                let kept = unchanged.and_then(|piece| self.toggled(piece, item.disabled));
                let content = match kept {
                    Some(content) => content,
                    None => {
                        let rendered = match entry {
                            Entry::Chunk(chunk) => self.render_chunk(chunk, tags)?,
                            Entry::Auto(rule) => with_newline(&canonical(&rule.rule.raw), nl),
                            Entry::Comment(_) | Entry::Fixed(_) => unreachable!("matched above"),
                        };
                        if item.disabled {
                            comment_out(&rendered)
                        } else {
                            rendered
                        }
                    }
                };
                let mut out = attached;
                push_terminated(&mut out, &content, nl);
                Ok(out)
            }
        }
    }

    /// An unchanged piece's content bytes, commented out or uncommented line
    /// by line when only its disabled flag changed (plan 31 C7). `None` when
    /// they cannot be (a line without the piece's prefix), and it is rendered.
    fn toggled(&self, piece: &Piece, disabled: bool) -> Option<String> {
        let bytes = &self.text[piece.content()];
        match (piece.disabled, disabled) {
            (was, now) if was == now => Some(bytes.to_string()),
            (false, _) => Some(comment_out(bytes)),
            (true, _) => {
                let prefix = piece.prefix.as_deref()?;
                bytes
                    .split_inclusive('\n')
                    .map(|line| line.strip_prefix(prefix))
                    .collect::<Option<String>>()
            }
        }
    }

    /// What follows layout item `at` (plan 31 C5's separator rule).
    fn separator(&self, at: usize, kinds: &[ItemKind], claims: &[Option<usize>]) -> &str {
        let nl = self.newline.as_str();
        let last = at + 1 == claims.len();
        // What the file ended with after its last piece.
        let trailing = self
            .pieces
            .last()
            .map_or("", |piece| &self.text[piece.gap.clone()]);
        let Some(index) = claims[at] else {
            return if last { trailing } else { nl };
        };
        let piece = &self.pieces[index];
        let gap = &self.text[piece.gap.clone()];
        if last {
            return if index + 1 == self.pieces.len() {
                gap
            } else {
                trailing
            };
        }
        if claims[at + 1] == Some(index + 1) || !gap.is_empty() {
            return gap;
        }
        // Fixed text that had no gap keeps none — unless fixed text now follows
        // it, which would read back as one piece.
        if piece.kind == ItemKind::Fixed && kinds[at + 1] != ItemKind::Fixed {
            return "";
        }
        nl
    }

    /// The header comment block, as this save wants it written.
    ///
    /// Every line is rewritten on every save: `created:` is carried through
    /// from what was loaded, `updated:` is whatever the caller set. A blank
    /// line follows it when there is anything after it (`rest`), so the
    /// header cannot be glued onto a directive. Also the declarations it
    /// wrote, if any, which the next read reads as fixed text.
    fn render_header(
        &self,
        scenario: &Scenario,
        rest: &str,
    ) -> Result<(String, Option<String>), SerializeError> {
        let nl = self.newline.as_str();
        let mut out = String::new();
        out.push_str(BANNER);
        out.push_str(nl);
        if !scenario.name.trim().is_empty() {
            out.push_str(&header_line("projection", &scenario.name)?);
            out.push_str(nl);
        }
        if let Some(created) = &scenario.created {
            out.push_str(&header_line("created", created)?);
            out.push_str(nl);
        }
        if let Some(updated) = &scenario.updated {
            out.push_str(&header_line("updated", updated)?);
            out.push_str(nl);
        }
        if let Some(cash) = &scenario.cash {
            check_account(&cash.0)?;
            out.push_str(&header_line("cash", &cash.0)?);
            out.push_str(nl);
        }
        // Gains accounts the main journal does not declare, declared HERE
        // (Decision 4), so hledger and Ledgeline classify them alike without
        // the main journal being touched. Set off by a blank line so the next
        // read's header span ends before them and a re-save is byte-identical.
        let mut declarations = self.commodity_declarations(scenario);
        declarations.extend(self.gains_declarations(scenario)?);
        let declared = (!declarations.is_empty()).then(|| {
            declarations
                .iter()
                .map(|declaration| format!("{declaration}\n"))
                .collect::<String>()
        });
        if !declarations.is_empty() {
            out.push_str(nl);
            for declaration in declarations {
                out.push_str(&declaration);
                out.push_str(nl);
            }
        }
        // A blank line, unless the header is the whole file. `starts_with` on
        // the remainder rather than a length test: a file that already begins
        // its body with a blank line must not gain a second one on every save.
        if !rest.is_empty() && !rest.starts_with(['\n', '\r']) {
            out.push_str(nl);
        }
        Ok((out, declared))
    }

    /// `commodity $1,000.00` for each commodity the scenario's amounts use that
    /// the main journal declares and this file does not: hledger lends no
    /// directive across sibling `-f` files, so without it `hledger -f main -f
    /// this` would read a hand-typed `$11,344` as 11.344 (verified, 1.52).
    fn commodity_declarations(&self, scenario: &Scenario) -> Vec<String> {
        let used: BTreeSet<&str> = scenario
            .lines
            .iter()
            .flat_map(|line| std::iter::once(&line.amount).chain(line.opening.as_ref()))
            .chain(
                scenario
                    .events
                    .iter()
                    .flat_map(|event| event.postings.iter().flat_map(|posting| &posting.amounts)),
            )
            .map(|amount| amount.commodity.0.as_str())
            .filter(|commodity| !commodity.is_empty())
            .collect();
        self.inherited
            .iter()
            .filter(|(commodity, _)| {
                used.contains(commodity.0.as_str()) && !self.own_commodities.contains(&commodity.0)
            })
            .map(|(commodity, style)| {
                let thousand = Dec::new(1000 * 10_i128.pow(style.precision), style.precision);
                format!(
                    "commodity {}",
                    crate::display::format_amount(&commodity.0, thousand, style, style.precision)
                )
            })
            .collect()
    }

    /// `account NAME  ; type: X` for each gains account an asset row books
    /// appreciation against that neither the main journal nor this file
    /// declares. Only rows that GROW produce gains postings, so only they
    /// count.
    fn gains_declarations(&self, scenario: &Scenario) -> Result<Vec<String>, SerializeError> {
        let types = AccountTypes::from_declared(self.declared.clone());
        let needed: BTreeSet<&str> = scenario
            .lines
            .iter()
            .filter(|line| line.role == LineRole::Asset)
            .filter(|line| {
                line.growth
                    .as_ref()
                    .is_some_and(|growth| !growth.rate.is_zero())
            })
            .map(|line| {
                line.gains
                    .as_ref()
                    .map_or(DEFAULT_GAINS_ACCOUNT, |gains| gains.0.as_str())
            })
            .filter(|gains| {
                !self.declared.contains_key(*gains) && !self.own_accounts.contains(*gains)
            })
            .collect();
        needed
            .into_iter()
            .map(|gains| {
                check_account(gains)?;
                // Equity unless the account already resolves to something else
                // (a user who points gains at income means income).
                let code = type_letter(types.resolve(gains).unwrap_or(AccountType::Equity));
                Ok(format!("account {gains}  ; type: {code}"))
            })
            .collect()
    }

    /// One `~` block, header line and postings, terminated.
    fn render_chunk(&self, chunk: &Chunk<'_>, tags: &Tags<'_>) -> Result<String, SerializeError> {
        let nl = self.newline.as_str();
        let (period, description, comment, rows) = match chunk {
            Chunk::Rule { key, lines } => {
                let period = lines
                    .first()
                    .map_or("monthly", |line| line.period.raw.as_str());
                // A block has ONE header, so a group whose segments disagree
                // about the period cannot be written as one rule. Refused
                // rather than silently taking the first: the second line's
                // recurrence would be quietly changed, which is money.
                if lines.iter().any(|line| line.period.raw != period) {
                    return Err(invalid(
                        "every line of one rule must share its recurrence; split them into \
                         separate rules to give them different ones",
                    ));
                }
                let description = lines.first().map_or("", |line| line.note.as_str());
                let comment = lines.first().map_or("", |line| line.rule_comment.as_str());
                let counter = block_counter(lines)?;
                let mut rows = lines
                    .iter()
                    .enumerate()
                    .map(|(at, line)| {
                        Ok(Row {
                            account: &line.account.0,
                            amount: Some(&line.amount),
                            tags: line_tags(tags.of(key, at, line), line)?,
                            // A line with a counter is a REGULAR posting the
                            // amountless one balances; one without is the
                            // legacy virtual form, outside balancing.
                            virtual_: line.counter.is_none(),
                            comment: comment_lines(&line.comment, "a posting's comment")?,
                        })
                    })
                    .collect::<Result<Vec<_>, SerializeError>>()?;
                if let Some(counter) = counter {
                    rows.push(Row::counter(&counter.0));
                }
                (period, description, comment, rows)
            }
            Chunk::Event(event) => {
                let mut rows = Vec::new();
                for (index, posting) in event.postings.iter().enumerate() {
                    for (at, amount) in posting.amounts.iter().enumerate() {
                        let comment = if at == 0 {
                            event.posting_comments.get(index).map_or("", String::as_str)
                        } else {
                            ""
                        };
                        rows.push(Row {
                            account: &posting.account.0,
                            amount: Some(amount),
                            // An event's postings carry no `line:` — the event
                            // IS the row, and its id is its block's index.
                            tags: String::new(),
                            virtual_: event.counter.is_none(),
                            comment: comment_lines(comment, "a posting's comment")?,
                        });
                    }
                }
                if let Some(counter) = &event.counter {
                    rows.push(Row::counter(&counter.0));
                }
                (
                    event.date.as_str(),
                    event.description.as_str(),
                    event.comment.as_str(),
                    rows,
                )
            }
        };

        let mut out = String::new();
        let comment = comment_lines(comment, "a rule's comment")?;
        let mut header = rule_header(period, description)?;
        if let Some(first) = comment.first() {
            header.push_str("  ; ");
            header.push_str(first);
        }
        out.push_str(&header);
        out.push_str(nl);
        // The rest of the header comment, before the first posting: the
        // parser folds those lines into the rule's own comment.
        for line in comment.iter().skip(1) {
            out.push_str(&format!("{INDENT}; {line}"));
            out.push_str(nl);
        }
        // One column for the whole block, so a rule reads as a table. Computed
        // from the rendered accounts rather than guessed, and deterministic, so
        // a scenario written twice produces identical bytes.
        let width = rows
            .iter()
            .filter(|row| row.amount.is_some())
            .map(|row| row.rendered_account().chars().count())
            .max()
            .unwrap_or(0);
        for row in &rows {
            for line in render_row(row, width)? {
                out.push_str(&line);
                out.push_str(nl);
            }
        }
        Ok(out)
    }
}

/// Recurring and single-date `~` rules are one kind for a claim; disabled and
/// active of one kind too (plan 31 C9).
const fn kind_class(kind: ItemKind) -> u8 {
    match kind {
        ItemKind::Rule | ItemKind::Event => 0,
        ItemKind::Auto => 1,
        ItemKind::Comment => 2,
        ItemKind::Fixed => 3,
    }
}

/// Append `text` to `out`, then `nl` unless it already ends a line.
fn push_terminated(out: &mut String, text: &str, nl: &str) {
    out.push_str(text);
    if !text.is_empty() && !text.ends_with('\n') {
        out.push_str(nl);
    }
}

/// `; ` before every line (plan 31 C7). Each line keeps its own terminator.
fn comment_out(text: &str) -> String {
    text.split_inclusive('\n')
        .map(|line| format!("; {line}"))
        .collect()
}

/// One item a save writes: the layout's own, or fixed text it keeps.
struct Planned<'a> {
    item: Cow<'a, LayoutItem>,
    entry: Entry<'a>,
    /// What the entry states ([`Entry::print`]).
    print: Vec<String>,
    /// The piece it claimed.
    claim: Option<usize>,
}

/// A layout item's content in the scenario.
enum Entry<'a> {
    /// A `~` rule: a group of lines, or an event.
    Chunk(Chunk<'a>),
    /// An `=` rule.
    Auto(&'a ScenarioAutoRule),
    /// A free-standing comment.
    Comment(&'a ScenarioComment),
    /// Fixed journal text.
    Fixed(&'a FixedPiece),
}

impl Entry<'_> {
    /// What the entry states: [`chunk_print`] for a `~` rule, the text for
    /// everything else.
    fn print(&self, tags: &Tags<'_>) -> Result<Vec<String>, SerializeError> {
        Ok(match self {
            Entry::Chunk(chunk) => chunk_print(chunk, tags)?,
            Entry::Auto(rule) => vec![canonical(&rule.rule.raw)],
            Entry::Comment(comment) => vec![canonical(&comment.raw)],
            Entry::Fixed(fixed) => vec![canonical(&fixed.raw)],
        })
    }
}

/// Every keyed entry of a scenario, for resolving a layout against.
struct Entries<'a> {
    groups: BTreeMap<&'a str, Vec<&'a ScenarioLine>>,
    events: BTreeMap<&'a str, &'a ScenarioEvent>,
    autos: BTreeMap<&'a str, &'a ScenarioAutoRule>,
    comments: BTreeMap<&'a str, &'a ScenarioComment>,
    fixed: BTreeMap<&'a str, &'a FixedPiece>,
}

impl<'a> Entries<'a> {
    /// Index `scenario`, and require its layout to name every entry exactly
    /// once.
    fn of(scenario: &'a Scenario) -> Result<Self, SerializeError> {
        let mut groups: BTreeMap<&str, Vec<&ScenarioLine>> = BTreeMap::new();
        for line in &scenario.lines {
            groups.entry(line.group.as_str()).or_default().push(line);
        }
        fn unique<'b, T>(
            items: impl IntoIterator<Item = (&'b str, T)>,
            what: &str,
        ) -> Result<BTreeMap<&'b str, T>, SerializeError> {
            let mut out = BTreeMap::new();
            for (key, item) in items {
                if out.insert(key, item).is_some() {
                    return Err(invalid(&format!("two {what} share the key `{key}`")));
                }
            }
            Ok(out)
        }
        let entries = Self {
            groups,
            events: unique(
                scenario
                    .events
                    .iter()
                    .map(|event| (event.id.as_str(), event)),
                "one-offs",
            )?,
            autos: unique(
                scenario
                    .auto_rules
                    .iter()
                    .map(|rule| (rule.key.as_str(), rule)),
                "auto rules",
            )?,
            comments: unique(
                scenario
                    .comments
                    .iter()
                    .map(|comment| (comment.key.as_str(), comment)),
                "comments",
            )?,
            fixed: unique(
                scenario
                    .fixed
                    .iter()
                    .map(|fixed| (fixed.key.as_str(), fixed)),
                "file text pieces",
            )?,
        };
        let mut named: BTreeSet<(ItemKind, &str)> = BTreeSet::new();
        for item in &scenario.layout {
            if !named.insert((item.kind, item.key.as_str())) {
                return Err(invalid(&format!(
                    "the layout names {} `{}` twice",
                    item.kind.as_str(),
                    item.key
                )));
            }
        }
        let unnamed = |kind: ItemKind, keys: Vec<&str>| -> Result<(), SerializeError> {
            match keys.into_iter().find(|key| !named.contains(&(kind, *key))) {
                Some(key) => Err(invalid(&format!(
                    "the layout leaves out {} `{key}`; every entry of the scenario must have its \
                     place in the file",
                    kind.as_str()
                ))),
                None => Ok(()),
            }
        };
        unnamed(ItemKind::Rule, entries.groups.keys().copied().collect())?;
        unnamed(ItemKind::Event, entries.events.keys().copied().collect())?;
        unnamed(ItemKind::Auto, entries.autos.keys().copied().collect())?;
        unnamed(
            ItemKind::Comment,
            entries.comments.keys().copied().collect(),
        )?;
        unnamed(ItemKind::Fixed, entries.fixed.keys().copied().collect())?;
        Ok(entries)
    }

    /// The entry `item` names.
    fn resolve(&self, item: &'a LayoutItem) -> Result<Entry<'a>, SerializeError> {
        let missing = || {
            invalid(&format!(
                "the layout names {} `{}`, which the scenario does not have",
                item.kind.as_str(),
                item.key
            ))
        };
        if item.disabled && matches!(item.kind, ItemKind::Comment | ItemKind::Fixed) {
            return Err(invalid(&format!(
                "only a rule, a one-off or an auto rule can be turned off, not {} `{}`",
                item.kind.as_str(),
                item.key
            )));
        }
        if !item.attached.is_empty() && matches!(item.kind, ItemKind::Comment | ItemKind::Fixed) {
            return Err(invalid(&format!(
                "{} `{}` cannot carry an attached comment",
                item.kind.as_str(),
                item.key
            )));
        }
        let key = item.key.as_str();
        Ok(match item.kind {
            ItemKind::Rule => {
                let (key, lines) = self.groups.get_key_value(key).ok_or_else(missing)?;
                Entry::Chunk(Chunk::Rule {
                    key,
                    lines: lines.clone(),
                })
            }
            ItemKind::Event => Entry::Chunk(Chunk::Event(
                self.events.get(key).copied().ok_or_else(missing)?,
            )),
            ItemKind::Auto => Entry::Auto(self.autos.get(key).copied().ok_or_else(missing)?),
            ItemKind::Comment => {
                Entry::Comment(self.comments.get(key).copied().ok_or_else(missing)?)
            }
            ItemKind::Fixed => Entry::Fixed(self.fixed.get(key).copied().ok_or_else(missing)?),
        })
    }
}

/// What one layout item must read back as, as rows: its kind, its disabled
/// flag and attached comment, then what it states. `fixed_raw` is the text a
/// fixed item was written as (its piece's), else its own.
fn item_print(
    item: &LayoutItem,
    entry: &Entry<'_>,
    print: &[String],
    fixed_raw: Option<String>,
) -> Vec<String> {
    let mut rows = Vec::with_capacity(print.len() + 1);
    match entry {
        Entry::Chunk(_) | Entry::Auto(_) => {
            rows.push(format!(
                "{} {} | attached {:?}",
                item.kind.as_str(),
                if item.disabled { "off" } else { "on" },
                canonical(&item.attached)
            ));
            rows.extend(print.iter().cloned());
        }
        Entry::Comment(comment) => rows.push(format!(
            "comment {} | {:?}",
            comment.style.as_str(),
            canonical(&comment.raw)
        )),
        Entry::Fixed(fixed) => rows.push(format!(
            "fixed | {:?}",
            fixed_raw.unwrap_or_else(|| canonical(&fixed.raw))
        )),
    }
    rows
}

/// Every layout item of `scenario`, as [`item_print`] prints it: what a
/// written file must read back as.
fn layout_prints(scenario: &Scenario, tags: &Tags<'_>) -> Result<Vec<String>, SerializeError> {
    let entries = Entries::of(scenario)?;
    let mut rows = Vec::new();
    for item in &scenario.layout {
        let entry = entries.resolve(item)?;
        let print = entry.print(tags)?;
        rows.extend(item_print(item, &entry, &print, None));
    }
    Ok(rows)
}

// ===========================================================================
// The legacy layout (plan 31 C8)
// ===========================================================================

/// The layout a scenario with NONE is written in: today's semantics before
/// plan 31, exactly.
///
/// - `~` rules and events claim the file's active `~` blocks by their
///   `rule:K` keys ([`assign_to`]), and fill the claimed blocks, each kind
///   among its own, in scenario order; the rest are appended.
/// - `=` rules claim the file's active `=` blocks by block number and text
///   ([`claim_auto_blocks`]), fill them in scenario order, and the rest are
///   appended after the `~` rules.
/// - A block nothing claims is removed.
/// - Every comment, disabled entry and piece of fixed text is kept where it
///   is: from the scenario when it states one with that key, else from the
///   file. A comment attached to a block stays with the BLOCK's place, as
///   comments between blocks always did.
/// - A `~` rule whose lines state no comment at all inherits its file
///   block's comments, line by line by id: a client that cannot edit
///   comments (one from before plan 31) cannot have meant to delete them.
///
/// It keeps the FFI, seeds and older clients correct, and is the only home
/// of the old algorithm.
#[must_use]
pub fn legacy_layout(doc: &ProjectionDoc, scenario: &Scenario) -> Scenario {
    let mut out = scenario.clone();
    unique_keys(&mut out);
    inherit_comments(doc, &mut out);

    let off: BTreeSet<(u8, &str)> = doc
        .pieces
        .iter()
        .filter(|piece| piece.disabled)
        .map(|piece| (kind_class(piece.kind), piece.key.as_str()))
        .collect();
    let is_off = |kind: ItemKind, key: &str| off.contains(&(kind_class(kind), key));

    // `~` slots, over the scenario's active entries only.
    let view = Scenario {
        lines: out
            .lines
            .iter()
            .filter(|line| !is_off(ItemKind::Rule, &line.group))
            .cloned()
            .collect(),
        events: out
            .events
            .iter()
            .filter(|event| !is_off(ItemKind::Event, &event.id))
            .cloned()
            .collect(),
        ..Scenario::default()
    };
    let tilde_blocks = doc
        .pieces
        .iter()
        .filter(|piece| kind_class(piece.kind) == 0 && !piece.disabled)
        .count();
    let Assignment { slots, appended } = assign_to(&view, tilde_blocks);
    let chunk_item = |chunk: &Chunk<'_>| match chunk {
        Chunk::Rule { key, .. } => (ItemKind::Rule, (*key).to_string()),
        Chunk::Event(event) => (ItemKind::Event, event.id.clone()),
    };
    let tilde_slots: BTreeMap<usize, (ItemKind, String)> = slots
        .iter()
        .map(|(block, (chunk, _))| (*block, chunk_item(chunk)))
        .collect();
    let tilde_appended: Vec<(ItemKind, String)> = appended.iter().map(chunk_item).collect();

    // `=` slots.
    let autos: Vec<&ScenarioAutoRule> = out
        .auto_rules
        .iter()
        .filter(|rule| !is_off(ItemKind::Auto, &rule.key))
        .collect();
    let auto_blocks: Vec<&Piece> = doc
        .pieces
        .iter()
        .filter(|piece| piece.kind == ItemKind::Auto && !piece.disabled)
        .collect();
    let (auto_slots, auto_appended) = claim_auto_blocks(&autos, &auto_blocks);

    let item =
        |kind: ItemKind, key: String, origin: Option<usize>, disabled, attached| LayoutItem {
            kind,
            key,
            origin,
            disabled,
            attached,
        };
    let mut layout = Vec::new();
    let mut restore: Vec<(ItemKind, String)> = Vec::new();
    let (mut tilde, mut auto) = (0usize, 0usize);
    for (index, piece) in doc.pieces.iter().enumerate() {
        let attached = canonical(&doc.text[piece.attached.clone()]);
        match (piece.kind, piece.disabled) {
            (ItemKind::Rule | ItemKind::Event, false) => {
                if let Some((kind, key)) = tilde_slots.get(&tilde) {
                    layout.push(item(*kind, key.clone(), Some(index), false, attached));
                }
                tilde += 1;
            }
            (ItemKind::Auto, false) => {
                if let Some(rule) = auto_slots.get(&auto) {
                    layout.push(item(
                        ItemKind::Auto,
                        rule.key.clone(),
                        Some(index),
                        false,
                        attached,
                    ));
                }
                auto += 1;
            }
            (kind, disabled) => {
                let key = piece.key.as_str();
                let stated = match kind {
                    ItemKind::Rule | ItemKind::Event => {
                        if out.lines.iter().any(|line| line.group == key) {
                            Some(ItemKind::Rule)
                        } else if out.events.iter().any(|event| event.id == key) {
                            Some(ItemKind::Event)
                        } else {
                            None
                        }
                    }
                    ItemKind::Auto => out
                        .auto_rules
                        .iter()
                        .any(|rule| rule.key == key)
                        .then_some(ItemKind::Auto),
                    ItemKind::Comment => out
                        .comments
                        .iter()
                        .any(|comment| comment.key == key)
                        .then_some(ItemKind::Comment),
                    ItemKind::Fixed => out
                        .fixed
                        .iter()
                        .any(|fixed| fixed.key == key)
                        .then_some(ItemKind::Fixed),
                };
                if stated.is_none() {
                    restore.push((kind, key.to_string()));
                }
                layout.push(item(
                    stated.unwrap_or(kind),
                    key.to_string(),
                    Some(index),
                    disabled,
                    attached,
                ));
            }
        }
    }
    let auto_appended: Vec<String> = auto_appended.iter().map(|rule| rule.key.clone()).collect();
    for (kind, key) in tilde_appended {
        layout.push(item(kind, key, None, false, String::new()));
    }
    for key in auto_appended {
        layout.push(item(ItemKind::Auto, key, None, false, String::new()));
    }
    for (kind, key) in restore {
        doc.read.copy_entry(kind, &key, &mut out);
    }
    out.layout = layout;
    out
}

impl Scenario {
    /// Copy this scenario's entry `key` of `kind` into `into`.
    fn copy_entry(&self, kind: ItemKind, key: &str, into: &mut Scenario) {
        match kind {
            ItemKind::Rule | ItemKind::Event => {
                into.lines
                    .extend(self.lines.iter().filter(|line| line.group == key).cloned());
                into.events
                    .extend(self.events.iter().filter(|event| event.id == key).cloned());
            }
            ItemKind::Auto => into.auto_rules.extend(
                self.auto_rules
                    .iter()
                    .filter(|rule| rule.key == key)
                    .cloned(),
            ),
            ItemKind::Comment => into.comments.extend(
                self.comments
                    .iter()
                    .filter(|comment| comment.key == key)
                    .cloned(),
            ),
            ItemKind::Fixed => {
                into.fixed
                    .extend(self.fixed.iter().filter(|fixed| fixed.key == key).cloned());
            }
        }
    }
}

/// Give every event and `=` rule a key a layout can name exactly once: a
/// client from before plan 31 sends `=` rules with none, and nothing stops
/// one sending two events with one id.
fn unique_keys(scenario: &mut Scenario) {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for (index, event) in scenario.events.iter_mut().enumerate() {
        if !seen.insert(event.id.clone()) {
            event.id = format!("{}#{index}", event.id);
            seen.insert(event.id.clone());
        }
    }
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for (index, rule) in scenario.auto_rules.iter_mut().enumerate() {
        if rule.key.is_empty() || !seen.insert(rule.key.clone()) {
            rule.key = format!("auto:in:{index}");
            seen.insert(rule.key.clone());
        }
    }
}

/// [`legacy_layout`]'s comment inheritance: a `~` rule or event keyed to a
/// block of `doc` whose lines state no comment at all takes that block's.
fn inherit_comments(doc: &ProjectionDoc, scenario: &mut Scenario) {
    let mut groups: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (index, line) in scenario.lines.iter().enumerate() {
        groups.entry(line.group.clone()).or_default().push(index);
    }
    for (group, members) in groups {
        let silent = members.iter().all(|&index| {
            scenario.lines[index].comment.is_empty()
                && scenario.lines[index].rule_comment.is_empty()
        });
        let theirs: Vec<&ScenarioLine> = doc
            .read
            .lines
            .iter()
            .filter(|line| line.group == group)
            .collect();
        if !silent || theirs.is_empty() {
            continue;
        }
        for index in members {
            let line = &mut scenario.lines[index];
            line.rule_comment = theirs[0].rule_comment.clone();
            if let Some(stated) = theirs.iter().find(|theirs| theirs.id == line.id) {
                line.comment = stated.comment.clone();
            }
        }
    }
    for event in &mut scenario.events {
        let silent =
            event.comment.is_empty() && event.posting_comments.iter().all(String::is_empty);
        let Some(theirs) = doc.read.events.iter().find(|theirs| theirs.id == event.id) else {
            continue;
        };
        if silent {
            event.comment = theirs.comment.clone();
            if theirs.postings.len() == event.postings.len() {
                event.posting_comments = theirs.posting_comments.clone();
            }
        }
    }
}

/// Which of the file's active `=` blocks each rule fills, and the rules left
/// over to append (plan 30 amendment 12, unchanged).
///
/// A rule CLAIMS a block: first one whose text it still is — its own
/// ([`ScenarioAutoRule::block`]), else any other holding exactly its text, so
/// an unchanged rule keeps its bytes even when its block number is stale or
/// absent — and failing both, an edited rule claims the block it names. Then
/// the claimed blocks are SLOTS filled in the scenario's order (the order
/// hledger applies the rules in), the rules left over are appended, and an
/// unclaimed block is removed.
fn claim_auto_blocks<'a>(
    rules: &[&'a ScenarioAutoRule],
    blocks: &[&Piece],
) -> (
    BTreeMap<usize, &'a ScenarioAutoRule>,
    Vec<&'a ScenarioAutoRule>,
) {
    let stated = |index: usize| blocks[index].print.first().map(String::as_str);
    let holds =
        |index: usize, rule: &ScenarioAutoRule| stated(index) == Some(&canonical(&rule.rule.raw));
    let mut claimed: BTreeSet<usize> = BTreeSet::new();
    let mut claims: Vec<Option<usize>> = vec![None; rules.len()];
    for (rule, claim) in rules.iter().zip(claims.iter_mut()) {
        if let Some(index) = rule.block.filter(|index| {
            *index < blocks.len() && holds(*index, rule) && !claimed.contains(index)
        }) {
            claimed.insert(index);
            *claim = Some(index);
        }
    }
    for (rule, claim) in rules.iter().zip(claims.iter_mut()) {
        if claim.is_none() {
            *claim =
                (0..blocks.len()).find(|index| !claimed.contains(index) && holds(*index, rule));
            if let Some(index) = *claim {
                claimed.insert(index);
            }
        }
    }
    for (rule, claim) in rules.iter().zip(claims.iter_mut()) {
        if claim.is_none()
            && let Some(index) = rule.block.filter(|index| *index < blocks.len())
            && claimed.insert(index)
        {
            *claim = Some(index);
        }
    }
    let mut slots = BTreeMap::new();
    let mut appended = Vec::new();
    let mut free = claimed.iter().copied();
    for rule in rules {
        match free.next() {
            Some(slot) => {
                slots.insert(slot, *rule);
            }
            None => appended.push(*rule),
        }
    }
    (slots, appended)
}

/// Where each chunk of a scenario goes, under [`legacy_layout`].
///
/// # Order is the scenario's (the what-if table's Move up / Move down)
///
/// The scenario's order — recurring rules by the FIRST APPEARANCE of their
/// group in `lines`, events in `events` order — is the order the file states
/// them in. The block a chunk claims (by its `rule:K` key) is not necessarily
/// where it is written: the claimed blocks are SLOTS, and the chunks fill them
/// in scenario order. So reordering two rows reorders their blocks while every
/// other piece (a comment, a transaction) stays where it was, and a deleted
/// block still removes its own slot rather than shifting every later block.
///
/// Rules and events fill separate slot sets. A file that interleaves the two —
/// a rule, a dated one-off, another rule — keeps that interleaving, because the
/// table orders each kind only among its own.
struct Assignment<'a> {
    /// Block index → the chunk written there, and whether that chunk CLAIMED
    /// this very block.
    slots: BTreeMap<usize, (Chunk<'a>, bool)>,
    /// Chunks with no slot, in scenario order: rules first, then events.
    appended: Vec<Chunk<'a>>,
}

/// One `~` block's worth of scenario.
enum Chunk<'a> {
    /// A recurring rule and its lines.
    Rule {
        key: &'a str,
        lines: Vec<&'a ScenarioLine>,
    },
    /// A dated one-off.
    Event(&'a ScenarioEvent),
}

impl<'a> Chunk<'a> {
    /// The id this chunk was read under: a rule's group, or an event's id.
    fn key(&self) -> &str {
        match self {
            Chunk::Rule { key, .. } => key,
            Chunk::Event(event) => &event.id,
        }
    }

    /// [`Chunk::key`], borrowed from the scenario rather than the chunk.
    fn into_key(self) -> &'a str {
        match self {
            Chunk::Rule { key, .. } => key,
            Chunk::Event(event) => &event.id,
        }
    }
}

/// Every chunk of `scenario` in the order the file states them: recurring
/// rules by the first appearance of their group in `lines`, and events.
///
/// Iterating a `BTreeMap` of groups instead would put `rule:10` before
/// `rule:2`, and this is the order the table shows them in.
fn chunks_of(scenario: &Scenario) -> (Vec<Chunk<'_>>, Vec<Chunk<'_>>) {
    let mut order: Vec<&str> = Vec::new();
    let mut groups: BTreeMap<&str, Vec<&ScenarioLine>> = BTreeMap::new();
    for line in &scenario.lines {
        if !groups.contains_key(line.group.as_str()) {
            order.push(&line.group);
        }
        groups.entry(&line.group).or_default().push(line);
    }
    let rules = order
        .into_iter()
        .map(|key| Chunk::Rule {
            key,
            lines: groups.remove(key).unwrap_or_default(),
        })
        .collect();
    let events = scenario.events.iter().map(Chunk::Event).collect();
    (rules, events)
}

/// Work out which of `blocks` existing `~` blocks each chunk of `scenario` is
/// written into, and which chunks go at EOF — see [`Assignment`].
///
/// A chunk CLAIMS block K when its key is `rule:K` and K is a block the file
/// has. First claimant wins, so two chunks can never both keep one block's
/// bytes. The claimed blocks of each kind are that kind's slots, and a block
/// nothing claims is removed.
fn assign_to<'a>(scenario: &'a Scenario, blocks: usize) -> Assignment<'a> {
    let (rules, events) = chunks_of(scenario);
    let mut claimed: BTreeSet<usize> = BTreeSet::new();
    let mut slots: BTreeMap<usize, (Chunk<'a>, bool)> = BTreeMap::new();
    let mut appended: Vec<Chunk<'a>> = Vec::new();
    for kind in [rules, events] {
        let claims: Vec<Option<usize>> = kind
            .iter()
            .map(|chunk| {
                block_of(chunk.key())
                    .filter(|index| *index < blocks)
                    .filter(|index| claimed.insert(*index))
            })
            .collect();
        let mut free: Vec<usize> = claims.iter().flatten().copied().collect();
        free.sort_unstable();
        let mut free = free.into_iter();
        for (chunk, claim) in kind.into_iter().zip(claims) {
            match free.next() {
                Some(slot) => {
                    slots.insert(slot, (chunk, claim == Some(slot)));
                }
                None => appended.push(chunk),
            }
        }
    }
    Assignment { slots, appended }
}

/// Each `~` block's key (a rule's group, an event's id) in the order a save
/// writes the blocks, which is the order hledger reads the rules in and so the
/// order `hledger print --forecast` lists two same-day generated transactions
/// in.
///
/// With a layout, it is the layout's order (plan 31 C5: the file's order).
/// Without one it is [`assign_to`]'s, with every `rule:K` key taken as a block
/// the file has: true of any scenario read from its file, whose keys ARE the
/// block indices. Keys with no slot follow, rules first, as a save appends
/// them.
pub(crate) fn block_order(scenario: &Scenario) -> Vec<&str> {
    let Assignment { slots, appended } = assign_to(scenario, usize::MAX);
    let legacy = slots
        .into_values()
        .map(|(chunk, _)| chunk)
        .chain(appended)
        .map(Chunk::into_key);
    if scenario.layout.is_empty() {
        return legacy.collect();
    }
    let mut keys: Vec<&str> = scenario
        .layout
        .iter()
        .filter(|item| matches!(item.kind, ItemKind::Rule | ItemKind::Event))
        .map(|item| item.key.as_str())
        .collect();
    let named: BTreeSet<&str> = keys.iter().copied().collect();
    keys.extend(legacy.filter(|key| !named.contains(key)));
    keys
}

/// Which `line:` value each row of a chunk states.
enum Tags<'a> {
    /// What a FILE states, which is all a reader can know: a tagged posting's
    /// id is its tag, and an untagged one's is derived (`rule:K:N`). Used for
    /// what an existing block says, and for what a written file reads back as.
    Literal,
    /// What a save writes: [`line_tag_values`] for the scenario being saved.
    Assigned(&'a BTreeMap<String, String>),
}

impl Tags<'_> {
    fn of<'b>(&'b self, key: &str, at: usize, line: &'b ScenarioLine) -> Option<&'b str> {
        match self {
            Tags::Literal => row_id_tag(key, at, &line.id),
            Tags::Assigned(values) => values.get(&line.id).map(String::as_str),
        }
    }
}

/// A comment as one line per string: trimmed, empty lines dropped — the form
/// both sides of a comparison are made in, and the form a save writes.
fn normal_comment(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// What one chunk SAYS, as a list of strings — one per row, headed by the
/// rule's own line.
///
/// This is the module's notion of "the same block". It carries every field the
/// file states and **no field the file derives**: not the group id, which is
/// the block's own position and renumbers whenever a block is added or removed,
/// and not a row's id unless a `line:` tag is actually written for it. Two
/// prints being equal means writing one over the other would change nothing.
/// Comments are fields the file states (plan 31 C6), so changing only one is
/// a change.
///
/// Used for two different jobs, which is why it is one function: deciding
/// whether a block needs rewriting at all, and proving the written file reads
/// back as the scenario that was asked for.
///
/// # Errors
/// [`SerializeError::Invalid`] when a field cannot be rendered.
fn chunk_print(chunk: &Chunk<'_>, tags: &Tags<'_>) -> Result<Vec<String>, SerializeError> {
    let mut rows = Vec::new();
    match chunk {
        Chunk::Rule { key, lines } => {
            let period = lines
                .first()
                .map_or("monthly", |line| line.period.raw.as_str());
            // Trimmed, as `rule_header` writes it: a description typed with a
            // trailing space is the same description.
            let note = lines.first().map_or("", |line| line.note.trim());
            let comment = lines
                .first()
                .map_or(String::new(), |line| normal_comment(&line.rule_comment));
            rows.push(format!("~ {period} | {note} | comment {comment:?}"));
            for (at, line) in lines.iter().enumerate() {
                rows.push(format!(
                    "{} {} | {} | line {} | growth {} | opening {} | every {} | counter {} | gains \
                     {} | comment {:?}",
                    line.account.0,
                    render_scenario_amount(&line.amount)?,
                    line.role.as_str(),
                    tags.of(key, at, line).unwrap_or("-"),
                    line.growth
                        .as_ref()
                        .map_or_else(|| "-".to_string(), render_growth),
                    line.opening
                        .as_ref()
                        .map(|opening| render_opening(line, opening))
                        .transpose()?
                        .unwrap_or_else(|| "-".to_string()),
                    line.period.raw,
                    line.counter
                        .as_ref()
                        .map_or("-", |counter| counter.0.as_str()),
                    line.gains.as_ref().map_or("-", |gains| gains.0.as_str()),
                    normal_comment(&line.comment),
                ));
            }
        }
        Chunk::Event(event) => {
            rows.push(format!(
                "~ {} | {} | counter {} | comment {:?}",
                event.date,
                event.description.trim(),
                event
                    .counter
                    .as_ref()
                    .map_or("-", |counter| counter.0.as_str()),
                normal_comment(&event.comment)
            ));
            for (index, posting) in event.postings.iter().enumerate() {
                for (at, amount) in posting.amounts.iter().enumerate() {
                    let comment = match at {
                        0 => event
                            .posting_comments
                            .get(index)
                            .map_or(String::new(), |comment| normal_comment(comment)),
                        _ => String::new(),
                    };
                    rows.push(format!(
                        "{} {} | comment {comment:?}",
                        posting.account.0,
                        render_scenario_amount(amount)?
                    ));
                }
            }
        }
    }
    Ok(rows)
}

/// The `line:` tag a posting in a FILE states, read back off its id: `None`
/// when the id is the one a reader derives from the block, `rule:K:N` for the
/// Nth posting of block K. Asked about a chunk's OWN key, so the answer does
/// not depend on where the block sits.
fn row_id_tag<'a>(key: &str, at: usize, id: &'a str) -> Option<&'a str> {
    (id != format!("{key}:{at}")).then_some(id)
}

/// The `line:` value each logical row WRITES, keyed by the row's model id.
///
/// # A tag only where one is needed
///
/// The tag exists to tie the segments of a STEP CHANGE — one logical row
/// spread over two or more `~` blocks — back into one row. A row with one
/// segment needs none: the reader derives its id from its position
/// (`rule:K:N`), and that id only has to be stable for as long as one load
/// lasts. So a row is tagged iff its id is on more than one line, and no other
/// row writes one — which also drops, on the next save, the `line: gap:…` tag
/// earlier versions put on every seeded row.
///
/// # Which value
///
/// An id that is already a readable value a person or an earlier save chose
/// (`payroll`) is kept, so a hand-written file is not churned. An id that is
/// a machine handle gets a short readable one instead, from its account's leaf
/// (`expenses:payroll` → `payroll`), de-duplicated with a suffix
/// (`payroll-2`). A machine handle is anything with a colon — the reader's
/// positional `rule:K:N`, the seed's `gap:<account>:<commodity>` — or the
/// table's own `line-N`/`group-N`/`event-N`.
///
/// **No written tag ever contains a colon**, so none can equal a derived
/// `rule:K:N`. That is what keeps reading stable across a reorder: a single
/// segment whose derived id happens to become `rule:3:0` after a move can never
/// be mistaken for one half of a step whose tag was written in another block.
fn line_tag_values(scenario: &Scenario) -> BTreeMap<String, String> {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    let mut order: Vec<&ScenarioLine> = Vec::new();
    for line in &scenario.lines {
        let count = counts.entry(line.id.as_str()).or_default();
        if *count == 0 {
            order.push(line);
        }
        *count += 1;
    }
    let steps: Vec<&ScenarioLine> = order
        .into_iter()
        .filter(|line| counts.get(line.id.as_str()).is_some_and(|count| *count > 1))
        .collect();

    // Kept values first, so a derived one can never take a name a kept one has.
    let mut taken: BTreeSet<String> = steps
        .iter()
        .filter(|line| keeps_its_id(&line.id))
        .map(|line| line.id.clone())
        .collect();
    steps
        .into_iter()
        .map(|line| {
            if keeps_its_id(&line.id) {
                return (line.id.clone(), line.id.clone());
            }
            let base = readable_id(&line.account.0);
            let value = std::iter::once(base.clone())
                .chain((2..).map(|n| format!("{base}-{n}")))
                .find(|candidate| !taken.contains(candidate))
                .unwrap_or(base);
            taken.insert(value.clone());
            (line.id.clone(), value)
        })
        .collect()
}

/// Whether a model id is already a readable `line:` value worth keeping — see
/// [`line_tag_values`].
fn keeps_its_id(id: &str) -> bool {
    let machine = id.contains(':')
        || ["line-", "group-", "event-"].iter().any(|prefix| {
            id.strip_prefix(prefix)
                .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
        });
    !machine && id.len() <= MAX_ID_BYTES && check_tag_value(id, "a line id").is_ok()
}

/// The longest id [`readable_id`] makes. Short on purpose: it is a handle a
/// person reads in the file, not a name.
const MAX_READABLE_ID: usize = 40;

/// `expenses:Team Payroll` → `team-payroll`: the account's leaf, lowercased,
/// ASCII alphanumerics only, every other run one `-`. `step` when nothing
/// survives (a leaf in another script).
fn readable_id(account: &str) -> String {
    let leaf = account.rsplit(':').next().unwrap_or_default();
    let mut slug = String::new();
    for c in leaf.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let clipped: String = slug.chars().take(MAX_READABLE_ID).collect();
    match clipped.trim_end_matches('-') {
        "" => "step".to_string(),
        readable => readable.to_string(),
    }
}

/// One posting line, before it is laid out.
struct Row<'a> {
    account: &'a str,
    /// `None` for the block's amountless counter posting.
    amount: Option<&'a Amount>,
    tags: String,
    /// Written `(account)`: a pre-plan-28 line with no counter-account.
    virtual_: bool,
    /// The user's own comment, one string per line (plan 31 C6).
    comment: Vec<&'a str>,
}

impl<'a> Row<'a> {
    /// The block's counter-account: a regular posting with no amount, which
    /// hledger infers to balance every regular posting above it.
    fn counter(account: &'a str) -> Self {
        Self {
            account,
            amount: None,
            tags: String::new(),
            virtual_: false,
            comment: Vec::new(),
        }
    }

    /// The account as the line writes it.
    fn rendered_account(&self) -> String {
        if self.virtual_ {
            wrapped(self.account)
        } else {
            self.account.to_string()
        }
    }
}

/// `(account)` — the pre-plan-28 form: an unbalanced virtual posting, as budget
/// goals are, whose cash effect the engine's legacy residual rule derives. A
/// line WITH a counter-account is written plain, and the block ends with the
/// amountless counter posting, so the rule is ordinary balanced hledger.
fn wrapped(account: &str) -> String {
    format!("({account})")
}

/// The ONE counter-account a block's lines share, if any.
///
/// A block has one amountless posting at most (hledger can infer only one), so
/// two lines of one rule with two different counter-accounts cannot be written
/// as one rule. Refused rather than silently picking one — the other line's
/// money would land somewhere else — exactly as two recurrences are.
fn block_counter<'a>(
    lines: &[&'a ScenarioLine],
) -> Result<Option<&'a AccountName>, SerializeError> {
    let mut found: Option<&AccountName> = None;
    for counter in lines.iter().filter_map(|line| line.counter.as_ref()) {
        match found {
            Some(seen) if seen != counter => {
                return Err(invalid(
                    "every line of one rule must share its counter-account; split them into \
                     separate rules to give them different ones",
                ));
            }
            _ => found = Some(counter),
        }
    }
    Ok(found)
}

/// hledger's one-letter `type:` code.
const fn type_letter(ty: AccountType) -> &'static str {
    match ty {
        AccountType::Asset => "A",
        AccountType::Liability => "L",
        AccountType::Equity => "E",
        AccountType::Revenue => "R",
        AccountType::Expense => "X",
        AccountType::Cash => "C",
        AccountType::Conversion => "V",
        AccountType::Gain => "G",
    }
}

/// `    (account)   $1,200.00  ; growth: 3%/yr`, then any continuation
/// comment lines.
///
/// # Where the user's comment goes (plan 31 C6)
///
/// Ledgeline's tags first, then the user's text: after `, ` on the same line
/// when its first line has no tag of its own, else on continuation lines. So
/// hledger's tag grammar, where a value runs to the next comma, can never read
/// the user's words as part of a Ledgeline tag's value, and a user's own
/// `owner: alex` stays a tag of its own.
fn render_row(row: &Row<'_>, width: usize) -> Result<Vec<String>, SerializeError> {
    check_account(row.account)?;
    let account = row.rendered_account();
    let Some(amount) = row.amount else {
        // The amountless counter posting: the account and nothing else.
        return Ok(vec![format!("{INDENT}{account}")]);
    };
    let amount = render_scenario_amount(amount)?;
    // `chars`, not `len`: a padding computed in bytes puts a multi-byte account
    // name's amount in the wrong column. Two spaces minimum, because that is
    // what separates an account from an amount.
    let pad = width.saturating_sub(account.chars().count()) + 2;
    let mut line = format!("{INDENT}{account}{}{amount}", " ".repeat(pad));
    let mut rest: &[&str] = &row.comment;
    if !row.tags.is_empty() {
        line.push_str("  ; ");
        line.push_str(&row.tags);
        if let Some(first) = rest.first().filter(|first| !has_tag(first)) {
            line.push_str(", ");
            line.push_str(first);
            rest = &rest[1..];
        }
    } else if let Some(first) = rest.first() {
        line.push_str("  ; ");
        line.push_str(first);
        rest = &rest[1..];
    }
    Ok(std::iter::once(line)
        .chain(rest.iter().map(|text| format!("{INDENT}{INDENT}; {text}")))
        .collect())
}

/// Whether a comment line holds a tag, by the parser's own rule.
fn has_tag(line: &str) -> bool {
    line.split(',')
        .any(|segment| parse::tag_in_segment(segment).is_some())
}

/// A user's comment (plan 31 C6) as the lines a save writes: trimmed, empty
/// ones dropped.
///
/// # Errors
/// [`SerializeError::Invalid`] for a control character, or for one of
/// Ledgeline's own tags, which the reader takes out of a comment and so could
/// never read back.
fn comment_lines<'a>(text: &'a str, what: &str) -> Result<Vec<&'a str>, SerializeError> {
    text.split('\n')
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| {
            if line.chars().any(|c| c.is_control() && c != '\t') {
                return Err(invalid(&format!(
                    "{what} may not contain a control character"
                )));
            }
            if line.split(',').any(is_ledgeline_tag) {
                return Err(invalid(&format!(
                    "{what} may not state Ledgeline's own `line:`, `growth:`, `opening:` or \
                     `gains:` tag; those are the row's own fields"
                )));
            }
            Ok(line)
        })
        .collect()
}

/// `~ PERIOD  DESCRIPTION`, or `~ PERIOD` when there is none.
fn rule_header(period: &str, description: &str) -> Result<String, SerializeError> {
    let period = period.trim();
    if period.is_empty() {
        return Err(invalid("a scenario line needs a recurrence"));
    }
    check_inline(period, "a recurrence")?;
    let description = description.trim();
    if description.is_empty() {
        return Ok(format!("~ {period}"));
    }
    if description.len() > MAX_NOTE_BYTES {
        return Err(invalid(&format!(
            "a note is {} bytes; the limit is {MAX_NOTE_BYTES}",
            description.len()
        )));
    }
    check_inline(description, "a note")?;
    Ok(format!("~ {period}  {description}"))
}

/// A posting's `; line: …, growth: …, opening: …` tags, or an empty string.
///
/// # The `growth:` tag is the asset marker, so an asset row always writes one
///
/// A [`LineRole::Asset`] row with no rate is written `; growth:` — the tag
/// present with an empty value. hledger reads that as the tag `growth` with the
/// value `""` (verified against 1.52: `tag:growth` matches it), and so does this
/// crate's own parser, so the row reads back as the asset row it is. Writing
/// `growth: 0%/yr` instead would be a rate the user never entered, and would
/// make a blank Growth cell come back filled after a save.
///
/// A [`LineRole::Flow`] row writes `growth:` only when it HAS one, exactly as
/// before — which is what makes every scenario file written before asset rows
/// existed keep its meaning byte for byte.
fn line_tags(tag: Option<&str>, line: &ScenarioLine) -> Result<String, SerializeError> {
    let mut parts = Vec::new();
    if let Some(id) = tag {
        check_tag_value(id, "a line id")?;
        if id.len() > MAX_ID_BYTES {
            return Err(invalid(&format!(
                "a line id is {} bytes; the limit is {MAX_ID_BYTES}",
                id.len()
            )));
        }
        parts.push(format!("line: {id}"));
    }
    match (&line.growth, line.role) {
        (Some(growth), _) => parts.push(format!("growth: {}", render_growth(growth))),
        (None, LineRole::Asset) => parts.push("growth:".to_string()),
        (None, LineRole::Flow) => {}
    }
    if let Some(opening) = &line.opening {
        parts.push(format!("opening: {}", render_opening(line, opening)?));
    }
    if let Some(gains) = &line.gains {
        if line.role != LineRole::Asset {
            return Err(invalid(
                "only an asset row may name a gains account: a flow line does not appreciate",
            ));
        }
        check_account(&gains.0)?;
        check_tag_value(&gains.0, "a gains account")?;
        parts.push(format!("gains: {}", gains.0));
    }
    Ok(parts.join(", "))
}

/// An `opening:` tag's value: a BARE number, at its own places.
///
/// Bare because hledger ends a tag's value at the next comma, so a grouped
/// `$200,000.00` would read back as `$200` — and because the commodity is
/// already on the row, in the contribution amount every asset row writes. That
/// is also why a mismatch is refused rather than silently rewritten: one row
/// cannot mean two commodities, and an override that quietly changed commodity
/// on a save is money.
fn render_opening(line: &ScenarioLine, opening: &Amount) -> Result<String, SerializeError> {
    if line.role != LineRole::Asset {
        return Err(invalid(
            "only an asset row may state an opening balance: a flow line has no balance to open",
        ));
    }
    if opening.commodity != line.amount.commodity {
        return Err(invalid(&format!(
            "an opening balance must be in the row's own commodity: the row is in \
             '{}' and the opening in '{}'",
            line.amount.commodity.0, opening.commodity.0
        )));
    }
    let rendered = render_dec_plain(opening.quantity);
    check_tag_value(&rendered, "an opening balance")?;
    Ok(rendered)
}

/// `3%/yr`, or `0.035/year` when the rate has too few decimal places to be a
/// whole percentage.
///
/// The `%` form is the one a human writes, so it is preferred — and it is
/// exact: a rate is a fraction, so `Dec::new(m, p)` is `m`×10⁻ᵖ, and shifting
/// the point two places left of the percent sign is `Dec::new(m, p - 2)`.
/// [`super::parse_growth`] shifts it back by exactly two, so the value that
/// comes out of the file is the value that went in, with no division anywhere.
fn render_growth(growth: &Growth) -> String {
    let unit = match growth.unit {
        GrowthUnit::Week => "wk",
        GrowthUnit::Month => "mo",
        GrowthUnit::Year => "yr",
    };
    match growth.rate.places.checked_sub(2) {
        Some(places) => format!(
            "{}%/{unit}",
            render_dec_plain(Dec::new(growth.rate.mantissa, places))
        ),
        // A rate like `0.1` (ten percent) has one decimal place, so there is no
        // exact percent spelling of it without inventing a place. `parse_growth`
        // reads the bare fraction too, so this round-trips unchanged.
        None => format!("{}/{unit}", render_dec_plain(growth.rate)),
    }
}

/// A [`Dec`] as a bare number, through the same renderer every amount uses so
/// the two cannot disagree about a decimal mark.
fn render_dec_plain(value: Dec) -> String {
    render_amount(&Amount {
        commodity: crate::model::Commodity(String::new()),
        quantity: value,
        style: crate::model::AmountStyle {
            side: crate::model::CommoditySide::Left,
            spaced: false,
            decimal_mark: Some('.'),
            digit_groups: None,
            precision: value.places,
        },
        cost: None,
    })
}

/// An amount, laid out at its own display precision.
///
/// The precision matters beyond looks: the engine rounds a growing amount back
/// to it at every step, so a `$1,875.00` line written as `$1875` would grow on
/// whole dollars from then on. `Dec::rounded` to the wider of the two is
/// lossless — it only ever adds places here, because the target is a maximum of
/// the value's own.
fn render_scenario_amount(amount: &Amount) -> Result<String, SerializeError> {
    let places = amount.style.precision.max(amount.quantity.places);
    let quantity = amount
        .quantity
        .rounded(places)
        .map_err(|error| invalid(&format!("an amount is out of range: {error}")))?;
    let rendered = render_amount(&Amount {
        quantity,
        // Grouping is deliberately dropped: `render_amount` omits it anyway,
        // and a comma in an amount is a comma in a comment's tag grammar one
        // careless edit away.
        style: crate::model::AmountStyle {
            digit_groups: None,
            precision: places,
            ..amount.style.clone()
        },
        ..amount.clone()
    });
    check_inline(&rendered, "an amount")?;
    Ok(rendered)
}

/// `; key: value`, refusing a value that would not read back.
fn header_line(key: &str, value: &str) -> Result<String, SerializeError> {
    let value = value.trim();
    if value.len() > MAX_HEADER_VALUE_BYTES {
        return Err(invalid(&format!(
            "the {key} header is {} bytes; the limit is {MAX_HEADER_VALUE_BYTES}",
            value.len()
        )));
    }
    check_inline(value, &format!("the {key} header"))?;
    // A comma would split the comment into two tag segments, so `; projection:
    // Plan B, revised` would read back as the name `Plan B`. Refused rather
    // than silently truncated — the name is also the filename.
    if value.contains(',') {
        return Err(invalid(&format!(
            "the {key} header may not contain a comma: hledger ends a tag's value at one, so the \
             rest would be dropped when the file is read back"
        )));
    }
    Ok(format!("; {key}: {value}"))
}

// ---------------------------------------------------------------------------
// Guards
// ---------------------------------------------------------------------------

/// Reject text that cannot go on one line of a journal.
fn check_inline(text: &str, what: &str) -> Result<(), SerializeError> {
    if text.chars().any(|c| c.is_ascii_control()) {
        return Err(invalid(&format!(
            "{what} may not contain a control character or a line break"
        )));
    }
    if text.contains(';') || text.contains('#') {
        return Err(invalid(&format!(
            "{what} may not contain `;` or `#`, which hledger reads as starting a comment"
        )));
    }
    Ok(())
}

/// Reject a value that would not survive hledger's tag grammar. A tag's value
/// runs to the next comma, and is trimmed.
fn check_tag_value(value: &str, what: &str) -> Result<(), SerializeError> {
    check_inline(value, what)?;
    if value.trim() != value || value.is_empty() {
        return Err(invalid(&format!(
            "{what} may not be empty or begin or end with whitespace: hledger trims a tag value, \
             so it would be written and then read back differently"
        )));
    }
    if value.contains(',') {
        return Err(invalid(&format!(
            "{what} may not contain a comma: hledger ends a tag's value at one"
        )));
    }
    Ok(())
}

/// Reject an account name this module will not write into a posting line.
///
/// The same rules `periodic::check_account` applies, for the same reason: each
/// one is a value that would make the written line read back as something other
/// than what was asked for.
pub(crate) fn check_account(account: &str) -> Result<(), SerializeError> {
    let bad = |why: &str| Err(invalid(&format!("an account name {why}")));
    if account.is_empty() {
        return bad("may not be empty");
    }
    if account.len() > MAX_ACCOUNT_BYTES {
        return bad(&format!(
            "is {} bytes; the limit is {MAX_ACCOUNT_BYTES}",
            account.len()
        ));
    }
    if account.trim() != account {
        return bad(
            "may not begin or end with whitespace: hledger trims it, so the value would be \
             written and then read back without it",
        );
    }
    if account.chars().any(|c| c.is_ascii_control()) {
        return bad("may not contain a control character");
    }
    if account.contains(';') || account.contains('#') {
        return bad("may not contain `;` or `#`, which hledger reads as starting a comment");
    }
    if account.contains("  ") || account.contains('\t') {
        return bad(
            "may not contain two consecutive spaces or a tab: hledger splits a posting line at \
             the first one, so the rest of the name would be read as an amount",
        );
    }
    if account.contains(['(', ')', '[', ']']) {
        return bad(
            "may not contain brackets: those mark a posting as virtual, which is a property of \
             the line rather than part of the name",
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// The header span
// ---------------------------------------------------------------------------

/// The leading run of header comment lines, as a byte range.
///
/// Only lines this module RECOGNIZES — the banner, and `projection:` /
/// `created:` / `updated:` — plus blank lines between them. It stops at the
/// first line that is anything else, so a user's own prose at the top of a file
/// is copied through rather than replaced by a header, and a header written
/// under that prose is rewritten in place.
///
/// An empty range at offset 0 means "there is no header", and the save inserts
/// one there.
fn header_span(text: &str) -> Range<usize> {
    let mut end = 0usize;
    let mut last_recognized = 0usize;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            // A blank line is only part of the header if a recognized line
            // follows it; carried provisionally and committed below.
            end += line.len();
            continue;
        }
        let Some(comment) = trimmed.strip_prefix([';', '#']) else {
            break;
        };
        if !is_header_comment(comment.trim()) {
            break;
        }
        end += line.len();
        last_recognized = end;
    }
    0..last_recognized
}

/// Whether a comment's text is one of the header's own lines.
fn is_header_comment(comment: &str) -> bool {
    if comment.eq_ignore_ascii_case(BANNER.trim_start_matches("; ")) {
        return true;
    }
    comment.split_once(':').is_some_and(|(key, _)| {
        matches!(
            key.trim().to_ascii_lowercase().as_str(),
            "projection" | "created" | "updated" | "cash"
        )
    })
}

// ===========================================================================
// The second opinion
// ===========================================================================

/// Write `scenario` into `doc`, then prove the result reads back as it.
///
/// This is the whole write, minus the I/O. The proof is the point: the renderer
/// is careful, and "careful" is not a property a user's money is entitled to
/// rely on. The new text is re-parsed as a journal from scratch, and what each
/// of its pieces says is compared, in LAYOUT ORDER, with what the scenario
/// asked for (plan 31 C5 guarantee 3): kinds, order, disabled flags, attached
/// comments, comment texts, and each entry's print — so a growth rate that
/// rendered into an unreadable tag, an amount whose precision moved, an
/// account name that split at a double space, a note that glued itself onto
/// the rule below, or an `=` rule whose text is two blocks fails here rather
/// than on disk.
///
/// The comparison is [`chunk_print`] and the pieces' texts, which carry every
/// field the FILE states and none it derives. It deliberately does not compare
/// whole [`Scenario`]s: `source` is a UI flag no file carries (a seeded row
/// saved to disk is, from then on, a row the user authored), and a key is a
/// piece's own position, which renumbers the moment a piece is added or
/// removed.
///
/// # Errors
/// [`SerializeError`] — the render refused, the result is not a journal, or it
/// does not read back as the scenario.
pub fn write_scenario(
    doc: &ProjectionDoc,
    scenario: &Scenario,
    source_name: &str,
) -> Result<String, SerializeError> {
    let resolved = doc.resolve(scenario);
    let (text, want) = doc.render(&resolved)?;
    confirm_round_trip(doc, &text, &want, source_name)?;
    Ok(text)
}

/// `text` with `\n` line ends written as `nl`.
fn with_newline(text: &str, nl: &str) -> String {
    if nl == "\n" {
        text.to_string()
    } else {
        text.replace('\n', nl)
    }
}

/// Re-read `text` and require its layout to print as `want`.
fn confirm_round_trip(
    doc: &ProjectionDoc,
    text: &str,
    want: &[String],
    source_name: &str,
) -> Result<(), SerializeError> {
    let written =
        scenario_from_text_with_styles(text, source_name, "", &doc.declared, &doc.inherited)?;
    // What the FILE states, read back with nothing assigned: a row whose tag
    // was written reads back with that tag as its id, and one with none reads
    // back with the id its position derives.
    let got = layout_prints(&written, &Tags::Literal)?;
    if want == got {
        return Ok(());
    }
    // The first DIFFERING row, not both whole scenarios: these strings reach a
    // user, and a pair of thousand-line dumps is not a diagnostic.
    let detail = want.iter().zip(&got).find(|(a, b)| a != b).map_or_else(
        || {
            format!(
                "{} rows were written and {} read back",
                want.len(),
                got.len()
            )
        },
        |(a, b)| format!("`{a}` was written but reads back as `{b}`"),
    );
    Err(SerializeError::NotRoundTripped(detail))
}

/// The fresh text of a brand-new file holding `scenario`.
///
/// # Errors
/// [`SerializeError`], as [`write_scenario`].
pub fn new_file(
    scenario: &Scenario,
    source_name: &str,
    declared: &BTreeMap<String, crate::reports::account_types::AccountType>,
) -> Result<String, SerializeError> {
    new_file_with_styles(scenario, source_name, declared, &[])
}

/// [`new_file`], declaring the main journal's styles for the commodities the
/// scenario uses (see [`ProjectionDoc::inheriting`]).
///
/// A new file holds the scenario and nothing else: its FIXED pieces (another
/// file's `account` directives, transactions, `include`s) are left out, as
/// Save As has always left them (plan 31, Scope).
///
/// # Errors
/// As [`new_file`].
pub fn new_file_with_styles(
    scenario: &Scenario,
    source_name: &str,
    declared: &BTreeMap<String, crate::reports::account_types::AccountType>,
    inherited: &[(crate::model::Commodity, crate::model::AmountStyle)],
) -> Result<String, SerializeError> {
    let scenario = Scenario {
        fixed: Vec::new(),
        layout: scenario
            .layout
            .iter()
            .filter(|item| item.kind != ItemKind::Fixed)
            .cloned()
            .collect(),
        ..scenario.clone()
    };
    write_scenario(
        &ProjectionDoc::empty(declared).inheriting(inherited),
        &scenario,
        source_name,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AccountName, AmountStyle, Commodity, CommoditySide};
    use crate::parse::parse_period_spec;
    // Only the tests build a `ScenarioLine` by hand; the writer reads `source`
    // nowhere, which is the whole point of the test at the bottom of this file.
    use crate::projections::LineSource;

    /// The plan's own worked example, verbatim from §"The file format".
    const EXAMPLE: &str = "\
; Ledgeline projection
; projection: Series A with a hiring ramp
; created: 2026-09-18
; updated: 2026-09-18

~ monthly  projection
    (revenues:salary)      $-12000  ; growth: 3%/yr
    (revenues:consulting)   $-2500
    (expenses:rent)          $4200  ; growth: 2%/yr
    (expenses:software)       $900

; A STEP: payroll jumps in April and grows from the new base.
~ monthly to 2027-04-01  projection
    (expenses:payroll)      $50000  ; line: payroll

~ monthly from 2027-04-01  projection
    (expenses:payroll)     $120000  ; line: payroll, growth: 5%/yr

; A ONE-OFF on the balance sheet: cash and equity, no net-income effect.
~ 2027-03-01  Series A
    (assets:cash)         $2000000
    (equity:preferred)   $-2000000

; A ONE-OFF on the P&L.
~ 2027-06-15  legal fees
    (expenses:legal)         $45000
";

    /// What the main journal declares. Empty is the ordinary case and is still
    /// a real classification: [`AccountTypes`] INFERS from a root name when
    /// nothing is declared, so `assets:brokerage` is an asset here for the same
    /// reason it is one everywhere else in the engine.
    fn declared() -> BTreeMap<String, crate::reports::account_types::AccountType> {
        BTreeMap::new()
    }

    /// What a file reads as, whole: layout, comments, fixed text and all.
    fn read_full(text: &str) -> Scenario {
        scenario_from_text(text, "p.journal", "fallback", &declared()).expect("the example parses")
    }

    /// What a client from before plan 31 holds after loading a file: the
    /// scenario as today's wire carries it (its active entries, no layout,
    /// no comments, no keys on `=` rules), so its save takes
    /// [`legacy_layout`].
    fn wire(scenario: Scenario) -> Scenario {
        let active = scenario.active();
        Scenario {
            lines: active
                .lines
                .into_iter()
                .map(|line| ScenarioLine {
                    comment: String::new(),
                    rule_comment: String::new(),
                    ..line
                })
                .collect(),
            events: active
                .events
                .into_iter()
                .map(|event| ScenarioEvent {
                    comment: String::new(),
                    posting_comments: Vec::new(),
                    ..event
                })
                .collect(),
            auto_rules: active
                .auto_rules
                .into_iter()
                .map(|rule| ScenarioAutoRule {
                    key: String::new(),
                    ..rule
                })
                .collect(),
            comments: Vec::new(),
            fixed: Vec::new(),
            layout: Vec::new(),
            ..active
        }
    }

    /// [`read_full`] through [`wire`]: what every test from before plan 31
    /// means by "the scenario the file reads as".
    fn read(text: &str) -> Scenario {
        wire(read_full(text))
    }

    fn usd(quantity: Dec, precision: u32) -> Amount {
        Amount {
            commodity: Commodity("$".to_string()),
            quantity,
            style: AmountStyle {
                side: CommoditySide::Left,
                spaced: false,
                decimal_mark: Some('.'),
                digit_groups: None,
                precision,
            },
            cost: None,
        }
    }

    fn line(group: &str, id: &str, account: &str, amount: Amount, period: &str) -> ScenarioLine {
        ScenarioLine {
            comment: String::new(),
            rule_comment: String::new(),
            counter: None,
            gains: None,
            id: id.to_string(),
            group: group.to_string(),
            role: LineRole::Flow,
            account: AccountName(account.to_string()),
            amount,
            period: parse_period_spec(period),
            growth: None,
            opening: None,
            note: String::new(),
            source: LineSource::Journal,
        }
    }

    /// An asset row: the amount is the CONTRIBUTION, `$0` when there is none.
    fn asset(
        group: &str,
        id: &str,
        account: &str,
        contribution: Amount,
        period: &str,
    ) -> ScenarioLine {
        ScenarioLine {
            role: LineRole::Asset,
            ..line(group, id, account, contribution, period)
        }
    }

    // -----------------------------------------------------------------------
    // Reading
    // -----------------------------------------------------------------------

    #[test]
    fn the_plan_s_example_reads_as_the_model_it_describes() {
        let scenario = read(EXAMPLE);
        assert_eq!(scenario.name, "Series A with a hiring ramp");
        assert_eq!(scenario.created.as_deref(), Some("2026-09-18"));
        assert_eq!(scenario.updated.as_deref(), Some("2026-09-18"));

        // Four recurring postings in block 0, then one in each of blocks 1 and 2.
        assert_eq!(scenario.lines.len(), 6);
        assert_eq!(scenario.lines[0].group, "rule:0");
        assert_eq!(scenario.lines[0].id, "rule:0:0");
        assert_eq!(scenario.lines[0].account.0, "revenues:salary");
        assert_eq!(scenario.lines[0].note, "projection");

        // The `growth:` tag, read as a FRACTION.
        assert_eq!(
            scenario.lines[0].growth,
            Some(Growth {
                rate: Dec::new(3, 2),
                unit: GrowthUnit::Year
            })
        );
        assert_eq!(scenario.lines[1].growth, None);

        // The step: two bounded segments of ONE logical row, sharing `line:`
        // and NOT sharing a group.
        let payroll: Vec<&ScenarioLine> = scenario
            .lines
            .iter()
            .filter(|line| line.id == "payroll")
            .collect();
        assert_eq!(payroll.len(), 2);
        assert_eq!(payroll[0].group, "rule:1");
        assert_eq!(payroll[1].group, "rule:2");
        assert_eq!(payroll[0].period.end.as_deref(), Some("2027-04-01"));
        assert_eq!(payroll[1].period.start.as_deref(), Some("2027-04-01"));

        // The two one-offs are EVENTS, and the balance-sheet one keeps both
        // of its legs under one date.
        assert_eq!(scenario.events.len(), 2);
        assert_eq!(scenario.events[0].id, "rule:3");
        assert_eq!(scenario.events[0].date, "2027-03-01");
        assert_eq!(scenario.events[0].description, "Series A");
        assert_eq!(scenario.events[0].postings.len(), 2);
        assert_eq!(scenario.events[1].date, "2027-06-15");
        assert_eq!(scenario.events[1].postings.len(), 1);
    }

    #[test]
    fn a_file_with_no_projection_marker_takes_its_name_from_the_caller() {
        // "A file without it still loads — really any journal file with
        // budget-like entries could be used — and takes its name from its
        // filename."
        let scenario = read("~ monthly  plan\n    (expenses:rent)  $100\n");
        assert_eq!(scenario.name, "fallback");
        assert_eq!(scenario.created, None);
        assert_eq!(scenario.lines.len(), 1);
    }

    #[test]
    fn a_bounded_once_rule_stays_a_line_rather_than_becoming_an_event() {
        // `~ from D to D2` fires once, but rewriting it as `~ D` would change
        // the rule the user wrote into a different one that behaves the same.
        let scenario = read("~ from 2027-01-01 to 2027-02-01  once\n    (expenses:x)  $5\n");
        assert_eq!(scenario.events.len(), 0);
        assert_eq!(scenario.lines.len(), 1);
        assert_eq!(
            scenario.lines[0].period.raw,
            "from 2027-01-01 to 2027-02-01"
        );
    }

    #[test]
    fn text_that_is_not_a_journal_is_an_error_rather_than_an_empty_scenario() {
        let error = scenario_from_text(
            "include ./definitely-not-here.journal\n",
            "p.journal",
            "",
            &declared(),
        );
        assert!(
            matches!(error, Err(SerializeError::Unreadable(_))),
            "{error:?}"
        );
    }

    // -----------------------------------------------------------------------
    // The round trip
    // -----------------------------------------------------------------------

    #[test]
    fn a_scenario_survives_text_and_back_unchanged() {
        // The round trip the plan's Testing table asks for: scenario → text →
        // parse → scenario, identical. Started from the file rather than from a
        // hand-built model, because a file is the canonical form and a
        // hand-built amount can carry a precision no file could state.
        let first = read(EXAMPLE);
        let text = write_scenario(
            &ProjectionDoc::parse(EXAMPLE, "p.journal", &declared()).unwrap(),
            &first,
            "p.journal",
        )
        .expect("it writes");
        let second = read(&text);
        assert_eq!(first, second);

        // And the text is a fixed point too, so a save that changes nothing
        // writes nothing.
        let again = write_scenario(
            &ProjectionDoc::parse(&text, "p.journal", &declared()).unwrap(),
            &second,
            "p.journal",
        )
        .expect("it writes");
        assert_eq!(text, again);
    }

    #[test]
    fn a_new_file_round_trips_and_states_its_header() {
        let scenario = Scenario {
            comments: Vec::new(),
            fixed: Vec::new(),
            layout: Vec::new(),
            auto_rules: Vec::new(),
            cash: None,
            name: "Plan B".to_string(),
            created: Some("2026-01-01".to_string()),
            updated: Some("2026-09-20".to_string()),
            lines: vec![
                ScenarioLine {
                    growth: Some(Growth {
                        rate: Dec::new(25, 3),
                        unit: GrowthUnit::Month,
                    }),
                    ..line(
                        "rule:0",
                        "rule:0:0",
                        "expenses:rent",
                        usd(Dec::new(420_000, 2), 2),
                        "monthly",
                    )
                },
                line(
                    "rule:0",
                    "rule:0:1",
                    "revenues:pay",
                    usd(Dec::new(-900_000, 2), 2),
                    "monthly",
                ),
            ],
            events: vec![ScenarioEvent {
                comment: String::new(),
                posting_comments: Vec::new(),
                counter: None,
                id: "rule:1".to_string(),
                date: "2027-03-01".to_string(),
                description: "Series A".to_string(),
                postings: vec![
                    virtual_posting(
                        AccountName("assets:cash".to_string()),
                        usd(Dec::new(200, 0), 0),
                    ),
                    virtual_posting(
                        AccountName("equity:preferred".to_string()),
                        usd(Dec::new(-200, 0), 0),
                    ),
                ],
            }],
        };
        let text = new_file(&scenario, "p.journal", &declared()).expect("it writes");
        assert!(text.starts_with(
            "; Ledgeline projection\n; projection: Plan B\n; created: 2026-01-01\n; updated: \
             2026-09-20\n\n"
        ));
        assert_eq!(read(&text), scenario);
    }

    #[test]
    fn growth_round_trips_at_every_shape_of_rate() {
        for (rate, unit, expected) in [
            (Dec::new(3, 2), GrowthUnit::Year, "3%/yr"),
            (Dec::new(25, 3), GrowthUnit::Month, "2.5%/mo"),
            (Dec::new(-1, 2), GrowthUnit::Week, "-1%/wk"),
            // Too few places for an exact percent spelling: the bare fraction
            // is written, and `parse_growth` reads that too.
            (Dec::new(1, 1), GrowthUnit::Year, "0.1/yr"),
        ] {
            let growth = Growth { rate, unit };
            assert_eq!(render_growth(&growth), expected);
            assert_eq!(
                super::super::parse_growth(&[("growth".to_string(), expected.to_string())]),
                Some(growth),
                "{expected}"
            );
        }
    }

    // -----------------------------------------------------------------------
    // Asset rows
    // -----------------------------------------------------------------------

    /// The format `plans/23-asset-growth.md` verified against hledger 1.52,
    /// read as the model it describes: a `$0` posting with a `growth:` tag on
    /// an asset account is an ASSET ROW whose contribution is zero.
    const ASSETS: &str = "\
; Ledgeline projection
; projection: Assets

~ monthly  projection
    (assets:brokerage)      $0.00  ; growth: 7%/yr
    (assets:savings)     $2000.00  ; growth: 4%/yr
    (assets:house)          $0.00  ; growth: 3%/yr, opening: 500000.00
    (expenses:rent)      $4200.00
";

    #[test]
    fn a_growth_tag_on_an_asset_account_reads_as_an_asset_row() {
        let scenario = read(ASSETS);
        let roles: Vec<(&str, LineRole)> = scenario
            .lines
            .iter()
            .map(|line| (line.account.0.as_str(), line.role))
            .collect();
        assert_eq!(
            roles,
            [
                ("assets:brokerage", LineRole::Asset),
                ("assets:savings", LineRole::Asset),
                ("assets:house", LineRole::Asset),
                // No `growth:` at all, so it is a flow whatever its account.
                ("expenses:rent", LineRole::Flow),
            ]
        );
        // The amount IS the contribution: `$0` for a balance that only
        // compounds, and the real figure for one that is paid into.
        assert_eq!(scenario.lines[0].amount.quantity, Dec::new(0, 2));
        assert_eq!(scenario.lines[1].amount.quantity, Dec::new(200_000, 2));
        assert_eq!(
            scenario.lines[0].growth,
            Some(Growth {
                rate: Dec::new(7, 2),
                unit: GrowthUnit::Year
            })
        );
        // The opening override, in the row's own commodity and style.
        assert_eq!(scenario.lines[0].opening, None);
        let house = scenario.lines[2].opening.as_ref().expect("an override");
        assert_eq!(house.quantity, Dec::new(50_000_000, 2));
        assert_eq!(house.commodity.0, "$");

        // It survives text and back, byte for byte and field for field.
        let text = write_scenario(
            &ProjectionDoc::parse(ASSETS, "p.journal", &declared()).unwrap(),
            &scenario,
            "p.journal",
        )
        .expect("it writes");
        // The one byte change an unchanged pre-plan-28 file gets on save: its
        // growing asset rows book appreciation to `equity:unrealized gains`,
        // which the main journal does not declare, so the FILE declares it
        // (plan 28 Decision 4) — under the header, set off by a blank line.
        let upgraded = ASSETS.replacen(
            "\n\n~ monthly",
            "\n\naccount equity:unrealized gains  ; type: E\n\n~ monthly",
            1,
        );
        assert_eq!(text, upgraded);
        assert_eq!(read(&text), scenario);
        // …and only once: the second save of the same scenario rewrites no byte.
        let again = write_scenario(
            &ProjectionDoc::parse(&text, "p.journal", &declared()).unwrap(),
            &scenario,
            "p.journal",
        )
        .expect("it writes again");
        assert_eq!(again, text, "an unchanged scenario rewrites no byte");
    }

    /// **The compatibility pin.** A posting with no `growth:` stays a flow line
    /// WHATEVER its account, so every scenario file written before asset rows
    /// existed keeps its exact current meaning — including the plan's own `$2M`
    /// raise into `assets:cash`.
    #[test]
    fn a_posting_with_no_growth_tag_is_still_a_flow_line() {
        let text = "\
~ monthly  transfers
    (assets:cash)         $2000.00
    (assets:brokerage)   $-2000.00  ; line: sweep
";
        let scenario = read(text);
        assert!(
            scenario
                .lines
                .iter()
                .all(|line| line.role == LineRole::Flow),
            "{:?}",
            scenario
                .lines
                .iter()
                .map(|line| (line.account.0.as_str(), line.role))
                .collect::<Vec<_>>()
        );
        assert!(scenario.lines.iter().all(|line| line.opening.is_none()));
        // …and the plan's own example, which posts a one-off to `assets:cash`,
        // still reads as the event it is.
        let plan = read(EXAMPLE);
        assert!(plan.lines.iter().all(|line| line.role == LineRole::Flow));
        // Rewriting it changes nothing, so an existing file is not rewritten
        // by the mere existence of this feature.
        let rewritten = write_scenario(
            &ProjectionDoc::parse(EXAMPLE, "p.journal", &declared()).unwrap(),
            &plan,
            "p.journal",
        )
        .expect("it writes");
        assert_eq!(rewritten, EXAMPLE);
    }

    /// A growing FLOW keeps its role: `growth:` alone does not make an asset
    /// row, or the plan's own `(expenses:rent) $4200 ; growth: 2%/yr` would
    /// silently become a stock.
    #[test]
    fn a_growth_tag_on_a_non_asset_account_stays_a_flow() {
        let scenario = read(EXAMPLE);
        let rent = scenario
            .lines
            .iter()
            .find(|line| line.account.0 == "expenses:rent")
            .expect("the rent line");
        assert_eq!(rent.role, LineRole::Flow);
        assert_eq!(
            rent.growth,
            Some(Growth {
                rate: Dec::new(2, 2),
                unit: GrowthUnit::Year
            })
        );
    }

    /// An asset row with NO rate writes `; growth:` — the tag present, its
    /// value empty. That is the marker, and it round-trips as `None` rather
    /// than coming back as a `0%` the user never typed.
    #[test]
    fn an_asset_row_with_no_rate_still_writes_the_marker_tag() {
        let scenario = Scenario {
            auto_rules: Vec::new(),
            name: "Flat".to_string(),
            lines: vec![asset(
                "rule:0",
                "rule:0:0",
                "assets:brokerage",
                usd(Dec::new(0, 2), 2),
                "monthly",
            )],
            ..Scenario::default()
        };
        let text = new_file(&scenario, "p.journal", &declared()).expect("it writes");
        assert!(
            text.contains("    (assets:brokerage)  $0.00  ; growth:\n"),
            "{text}"
        );
        let reread = read(&text);
        assert_eq!(reread.lines[0].role, LineRole::Asset);
        assert_eq!(reread.lines[0].growth, None);
        assert_eq!(reread, scenario);
    }

    /// The account types come from the caller, and a file's own `account`
    /// directives win over them — so a Spanish chart classifies, and a
    /// declaration inside the scenario file is enough on its own.
    #[test]
    fn the_asset_marker_reads_the_declared_type_not_the_name() {
        let text = "\
account activo:inversiones  ; type: A

~ monthly  plan
    (activo:inversiones)  $0.00  ; growth: 7%/yr
";
        // With nothing declared, the file's own directive is what classifies
        // it: `activo:` infers no type at all.
        let from_file = scenario_from_text(text, "p.journal", "x", &declared()).unwrap();
        assert_eq!(from_file.lines[0].role, LineRole::Asset);

        // Without the directive, and with nothing declared, there is no type to
        // read and the row stays a flow rather than being guessed at.
        let bare = text.replace("account activo:inversiones  ; type: A\n", "");
        let guessed = scenario_from_text(&bare, "p.journal", "x", &declared()).unwrap();
        assert_eq!(guessed.lines[0].role, LineRole::Flow);

        // …and the MAIN journal's declaration is enough by itself.
        let declared_by_journal: BTreeMap<String, crate::reports::account_types::AccountType> = [(
            "activo:inversiones".to_string(),
            crate::reports::account_types::AccountType::Asset,
        )]
        .into_iter()
        .collect();
        let from_journal =
            scenario_from_text(&bare, "p.journal", "x", &declared_by_journal).unwrap();
        assert_eq!(from_journal.lines[0].role, LineRole::Asset);
    }

    /// An opening balance the writer cannot state truthfully is refused, not
    /// silently rewritten. Both are money: a commodity that changed on a save
    /// is a different balance, and a flow line has no balance at all.
    #[test]
    fn an_opening_balance_that_cannot_be_written_is_refused() {
        let mut row = asset(
            "rule:0",
            "rule:0:0",
            "assets:brokerage",
            usd(Dec::new(0, 2), 2),
            "monthly",
        );
        row.opening = Some(Amount {
            commodity: Commodity("EUR".to_string()),
            ..usd(Dec::new(100_000, 2), 2)
        });
        let cross_commodity = Scenario {
            auto_rules: Vec::new(),
            lines: vec![row.clone()],
            ..Scenario::default()
        };
        assert!(
            matches!(
                new_file(&cross_commodity, "p.journal", &declared()),
                Err(SerializeError::Invalid(ref why))
                    if why.contains("commodity")
            ),
            "{:?}",
            new_file(&cross_commodity, "p.journal", &declared())
        );

        let on_a_flow = Scenario {
            auto_rules: Vec::new(),
            lines: vec![ScenarioLine {
                role: LineRole::Flow,
                opening: Some(usd(Dec::new(100_000, 2), 2)),
                ..row
            }],
            ..Scenario::default()
        };
        assert!(
            matches!(
                new_file(&on_a_flow, "p.journal", &declared()),
                Err(SerializeError::Invalid(ref why)) if why.contains("asset row")
            ),
            "{:?}",
            new_file(&on_a_flow, "p.journal", &declared())
        );
    }

    // -----------------------------------------------------------------------
    // Byte-level non-disturbance
    // -----------------------------------------------------------------------

    #[test]
    fn an_update_leaves_every_byte_outside_the_edited_block_alone() {
        const SOURCE: &str = "\
; Ledgeline projection
; projection: Plan
; created: 2026-01-01
; updated: 2026-01-01

; a comment the user wrote, between the header and the first rule
account expenses:rent    ; type: X

~ monthly  plan
    (expenses:rent)    $4200

; a comment BETWEEN two rules
~ monthly  plan
    (expenses:food)     $600

2026-01-01 a real transaction
    expenses:rent   $4200
    assets:cash    $-4200
";
        let doc = ProjectionDoc::parse(SOURCE, "p.journal", &declared()).unwrap();
        let mut scenario = read(SOURCE);
        assert_eq!(scenario.lines.len(), 2);
        // One number changes, and nothing else.
        scenario.lines[0].amount = usd(Dec::new(450_000, 2), 2);
        let text = write_scenario(&doc, &scenario, "p.journal").expect("it writes");

        for kept in [
            "; a comment the user wrote, between the header and the first rule\n",
            "account expenses:rent    ; type: X\n",
            "; a comment BETWEEN two rules\n",
            "2026-01-01 a real transaction\n    expenses:rent   $4200\n    assets:cash    $-4200\n",
            "    (expenses:food)     $600\n",
        ] {
            assert!(text.contains(kept), "lost `{kept}` from:\n{text}");
        }
        // The strongest form of the claim: the result is the SOURCE with
        // exactly one block's bytes replaced, and nothing else touched at all.
        // The untouched rule keeps its own five-space column, which is the
        // property a re-render of every block would quietly destroy.
        assert_eq!(
            text,
            SOURCE.replace(
                "~ monthly  plan\n    (expenses:rent)    $4200\n",
                "~ monthly  plan\n    (expenses:rent)  $4500.00\n"
            )
        );
        assert!(text.contains("    (expenses:food)     $600\n"), "{text}");
    }

    #[test]
    fn a_user_s_prose_above_the_header_is_not_replaced_by_it() {
        const SOURCE: &str = "; my own notes about this file\n; projection: Old\n\n~ monthly  p\n    (expenses:x)  $1\n";
        let mut scenario = read(SOURCE);
        // The prose is not a header line, so the header span is empty and the
        // new header is INSERTED above it rather than written over it.
        scenario.name = "New".to_string();
        let text = write_scenario(
            &ProjectionDoc::parse(SOURCE, "p.journal", &declared()).unwrap(),
            &scenario,
            "p.journal",
        )
        .unwrap();
        assert!(text.contains("; my own notes about this file\n"), "{text}");
        assert!(
            text.starts_with("; Ledgeline projection\n; projection: New\n"),
            "{text}"
        );
        assert_eq!(
            read(&text).name,
            "New",
            "and the stale `; projection: Old` below it does not win"
        );
    }

    #[test]
    fn a_group_the_scenario_dropped_has_its_block_removed() {
        const SOURCE: &str =
            "~ monthly  a\n    (expenses:a)  $1\n\n~ monthly  b\n    (expenses:b)  $2\n";
        let mut scenario = read(SOURCE);
        scenario.lines.retain(|line| line.group == "rule:1");
        let text = write_scenario(
            &ProjectionDoc::parse(SOURCE, "p.journal", &declared()).unwrap(),
            &scenario,
            "p.journal",
        )
        .unwrap();
        assert!(!text.contains("expenses:a"), "{text}");
        assert!(text.contains("(expenses:b)  $2"), "{text}");
        assert_eq!(read(&text).lines.len(), 1);
    }

    #[test]
    fn a_group_with_no_block_yet_is_appended() {
        const SOURCE: &str = "~ monthly  a\n    (expenses:a)  $1\n";
        let mut scenario = read(SOURCE);
        scenario.lines.push(line(
            "group-7",
            "group-7:0",
            "expenses:new",
            usd(Dec::new(50, 0), 0),
            "weekly",
        ));
        let text = write_scenario(
            &ProjectionDoc::parse(SOURCE, "p.journal", &declared()).unwrap(),
            &scenario,
            "p.journal",
        )
        .unwrap();
        assert!(
            text.contains("~ weekly\n    (expenses:new)  $50\n"),
            "{text}"
        );
        // And it comes back with a canonical group, as block 1.
        let reread = read(&text);
        assert_eq!(reread.lines[1].group, "rule:1");
    }

    // -----------------------------------------------------------------------
    // Refusals
    // -----------------------------------------------------------------------

    #[test]
    fn an_account_name_that_would_not_read_back_is_refused() {
        for account in [
            "expenses:a  b", // splits at the double space
            "expenses:a;b",  // starts a comment
            "expenses:(a)",  // brackets are a posting property
            " expenses:a",   // hledger trims it
            "expenses:a\tb", // splits at the tab
            "",
        ] {
            let scenario = Scenario {
                auto_rules: Vec::new(),
                lines: vec![line(
                    "rule:0",
                    "rule:0:0",
                    account,
                    usd(Dec::new(1, 0), 0),
                    "monthly",
                )],
                ..Scenario::default()
            };
            assert!(
                matches!(
                    new_file(&scenario, "p.journal", &declared()),
                    Err(SerializeError::Invalid(_))
                ),
                "accepted `{account}`"
            );
        }
    }

    #[test]
    fn a_name_with_a_comma_is_refused_rather_than_truncated() {
        // hledger ends a tag's value at a comma, so `; projection: Plan B,
        // revised` reads back as `Plan B` — and the name is also the filename.
        let scenario = Scenario {
            auto_rules: Vec::new(),
            name: "Plan B, revised".to_string(),
            ..Scenario::default()
        };
        assert!(matches!(
            new_file(&scenario, "p.journal", &declared()),
            Err(SerializeError::Invalid(_))
        ));
    }

    #[test]
    fn a_group_whose_lines_disagree_about_the_recurrence_is_refused() {
        let scenario = Scenario {
            auto_rules: Vec::new(),
            lines: vec![
                line(
                    "rule:0",
                    "a",
                    "expenses:a",
                    usd(Dec::new(1, 0), 0),
                    "monthly",
                ),
                line(
                    "rule:0",
                    "b",
                    "expenses:b",
                    usd(Dec::new(2, 0), 0),
                    "weekly",
                ),
            ],
            ..Scenario::default()
        };
        assert!(matches!(
            new_file(&scenario, "p.journal", &declared()),
            Err(SerializeError::Invalid(_))
        ));
    }

    #[test]
    fn an_id_that_would_not_survive_the_tag_grammar_is_never_written() {
        // An id is only written for a step change now, and a step whose id
        // would not survive the tag grammar is written under a readable one
        // instead — so neither shape can produce an unreadable `line:` tag.
        for id in ["a,b", "a;b", " a", ""] {
            let single = Scenario {
                auto_rules: Vec::new(),
                lines: vec![line(
                    "rule:0",
                    id,
                    "expenses:a",
                    usd(Dec::new(1, 0), 0),
                    "monthly",
                )],
                ..Scenario::default()
            };
            let text = new_file(&single, "p.journal", &declared()).expect("a single row writes");
            assert!(!text.contains("line:"), "{text}");

            let step = Scenario {
                auto_rules: Vec::new(),
                lines: vec![
                    line(
                        "rule:0",
                        id,
                        "expenses:a",
                        usd(Dec::new(1, 0), 0),
                        "monthly to 2027-04-01",
                    ),
                    line(
                        "rule:1",
                        id,
                        "expenses:a",
                        usd(Dec::new(2, 0), 0),
                        "monthly from 2027-04-01",
                    ),
                ],
                ..Scenario::default()
            };
            let text = new_file(&step, "p.journal", &declared()).expect("a step writes");
            assert_eq!(text.matches("; line: a\n").count(), 2, "`{id}`: {text}");
        }
    }

    /// Two segments of one logical row, in blocks `first` and `second`.
    fn step(first: &str, second: &str, id: &str, account: &str) -> [ScenarioLine; 2] {
        [
            line(
                first,
                id,
                account,
                usd(Dec::new(50, 0), 0),
                "monthly to 2027-04-01",
            ),
            line(
                second,
                id,
                account,
                usd(Dec::new(120, 0), 0),
                "monthly from 2027-04-01",
            ),
        ]
    }

    #[test]
    fn the_line_tag_is_written_only_for_a_step_change() {
        let mut lines = vec![
            // A single segment, whatever its id, writes no tag.
            line(
                "rule:0",
                "rule:0:0",
                "expenses:a",
                usd(Dec::new(1, 0), 0),
                "monthly",
            ),
            line(
                "rule:0",
                "payroll",
                "expenses:b",
                usd(Dec::new(2, 0), 0),
                "monthly",
            ),
            line(
                "gap:x",
                "gap:expenses:c:$",
                "expenses:c",
                usd(Dec::new(3, 0), 0),
                "monthly",
            ),
        ];
        // A step under a machine id gets a readable one from its account leaf…
        lines.extend(step(
            "rule:1",
            "group-1",
            "rule:1:0",
            "expenses:team:Payroll",
        ));
        // …a second step on a same-named leaf is de-duplicated…
        lines.extend(step(
            "rule:2",
            "group-2",
            "line-4",
            "expenses:contractors:payroll",
        ));
        // …and a readable id somebody chose is kept as it is.
        lines.extend(step("rule:3", "group-3", "hiring", "expenses:recruiting"));
        let scenario = Scenario {
            auto_rules: Vec::new(),
            lines,
            ..Scenario::default()
        };
        let text = new_file(&scenario, "p.journal", &declared()).unwrap();
        assert!(text.contains("(expenses:a)  $1\n"), "{text}");
        assert!(text.contains("(expenses:b)  $2\n"), "{text}");
        assert!(text.contains("(expenses:c)  $3\n"), "{text}");
        assert_eq!(text.matches("; line: payroll\n").count(), 2, "{text}");
        assert_eq!(text.matches("; line: payroll-2\n").count(), 2, "{text}");
        assert_eq!(text.matches("; line: hiring\n").count(), 2, "{text}");
        assert_eq!(text.matches("line:").count(), 6, "{text}");

        // And it reads back as three single rows and three two-segment ones.
        let reread = read(&text);
        let ids: BTreeSet<&str> = reread.lines.iter().map(|line| line.id.as_str()).collect();
        assert_eq!(
            ids,
            BTreeSet::from([
                "rule:0:0",
                "rule:0:1",
                "rule:1:0",
                "payroll",
                "payroll-2",
                "hiring"
            ])
        );
    }

    #[test]
    fn a_legacy_gap_tag_still_loads_and_is_dropped_on_the_next_save() {
        // Every seeded row used to be saved with `; line: gap:<account>:<commodity>`.
        const LEGACY: &str = "\
; Ledgeline projection
; projection: Old

~ monthly  unbudgeted — average over 2025-11-01 to 2026-10-22
    (expenses:food)  $450  ; line: gap:expenses:food:$

~ monthly  unbudgeted — average over 2025-11-01 to 2026-10-22
    (expenses:fuel)  $80  ; line: gap:expenses:fuel:$
";
        let scenario = read(LEGACY);
        assert_eq!(scenario.lines.len(), 2);
        assert_eq!(scenario.lines[0].id, "gap:expenses:food:$");
        assert_eq!(scenario.lines[1].id, "gap:expenses:fuel:$");

        let doc = ProjectionDoc::parse(LEGACY, "p.journal", &declared()).unwrap();
        let text = write_scenario(&doc, &scenario, "p.journal").expect("it writes");
        assert!(!text.contains("line:"), "{text}");
        assert!(text.contains("(expenses:food)  $450\n"), "{text}");
        let reread = read(&text);
        assert_eq!(reread.lines.len(), 2);
        assert_eq!(reread.lines[0].id, "rule:0:0");
        assert_eq!(reread.lines[1].id, "rule:1:0");
        // A fixed point from here on.
        let again = write_scenario(
            &ProjectionDoc::parse(&text, "p.journal", &declared()).unwrap(),
            &reread,
            "p.journal",
        )
        .unwrap();
        assert_eq!(text, again);
    }

    /// Rules, a comment between two of them, an event between two others.
    const ORDERED: &str = "\
; Ledgeline projection
; projection: Ordered

~ monthly  a
    (expenses:a)  $1

; about b
~ monthly  b
    (expenses:b)  $2

~ 2027-03-01  windfall
    (revenues:gift)  $-100

~ monthly  c
    (expenses:c)  $3
    (expenses:d)  $4
";

    /// The accounts of `text`'s postings, in file order.
    fn accounts_in(text: &str) -> Vec<String> {
        text.lines()
            .filter_map(|line| line.trim_start().strip_prefix('('))
            .filter_map(|rest| rest.split_once(')'))
            .map(|(account, _)| account.to_string())
            .collect()
    }

    #[test]
    fn rows_are_written_in_scenario_order_and_the_bytes_between_blocks_stay() {
        let mut scenario = read(ORDERED);
        // Move `c`'s rule to the top, and swap its two postings.
        let (c, rest): (Vec<_>, Vec<_>) = scenario
            .lines
            .into_iter()
            .partition(|line| line.group == "rule:3");
        scenario.lines = c.into_iter().rev().chain(rest).collect();

        let doc = ProjectionDoc::parse(ORDERED, "p.journal", &declared()).unwrap();
        let text = write_scenario(&doc, &scenario, "p.journal").expect("it writes");
        assert_eq!(
            accounts_in(&text),
            [
                "expenses:d",
                "expenses:c",
                "expenses:a",
                "revenues:gift",
                "expenses:b"
            ],
            "{text}"
        );
        // The comment and the event did not move: the RULES filled the rule
        // slots in their new order, around them.
        assert!(
            text.contains("; about b\n~ monthly  a\n    (expenses:a)  $1\n"),
            "{text}"
        );
        assert!(text.contains("~ 2027-03-01  windfall\n"), "{text}");
        let windfall = text.find("windfall").unwrap();
        assert!(text.find("(expenses:a)").unwrap() < windfall, "{text}");
        assert!(windfall < text.find("(expenses:b)").unwrap(), "{text}");

        // The order is what the next load shows, and the result is a fixed point.
        let reread = read(&text);
        let order: Vec<&str> = reread
            .lines
            .iter()
            .map(|line| line.account.0.as_str())
            .collect();
        assert_eq!(
            order,
            ["expenses:d", "expenses:c", "expenses:a", "expenses:b"]
        );
        let again = write_scenario(
            &ProjectionDoc::parse(&text, "p.journal", &declared()).unwrap(),
            &reread,
            "p.journal",
        )
        .unwrap();
        assert_eq!(text, again);
    }

    #[test]
    fn an_unmoved_scenario_rewrites_nothing() {
        let scenario = read(ORDERED);
        let doc = ProjectionDoc::parse(ORDERED, "p.journal", &declared()).unwrap();
        assert_eq!(
            write_scenario(&doc, &scenario, "p.journal").unwrap(),
            ORDERED
        );
    }

    #[test]
    fn a_reordered_step_still_reads_back_as_one_row() {
        // The caveat the tag rule answers: a single segment saved untagged
        // reads back as `rule:K:N` for wherever its block lands, so if a step's
        // tag could ever spell one of those, moving blocks around could weld
        // an unrelated row onto the step.
        let mut lines = vec![line(
            "rule:0",
            "rule:0:0",
            "expenses:solo",
            usd(Dec::new(7, 0), 0),
            "monthly",
        )];
        // A step whose model id is itself a `rule:K:N` — what splitting a
        // loaded single row produces.
        lines.extend(step("rule:1", "group-1", "rule:1:0", "expenses:payroll"));
        let first = new_file(
            &Scenario {
                auto_rules: Vec::new(),
                lines,
                ..Scenario::default()
            },
            "p.journal",
            &declared(),
        )
        .unwrap();

        // Move the solo row below the step, and save over the file.
        let mut scenario = read(&first);
        scenario.lines.rotate_left(1);
        let text = write_scenario(
            &ProjectionDoc::parse(&first, "p.journal", &declared()).unwrap(),
            &scenario,
            "p.journal",
        )
        .unwrap();
        let reread = read(&text);
        assert_eq!(
            accounts_in(&text),
            ["expenses:payroll", "expenses:payroll", "expenses:solo"]
        );
        let payroll: Vec<&ScenarioLine> = reread
            .lines
            .iter()
            .filter(|line| line.account.0 == "expenses:payroll")
            .collect();
        assert_eq!(payroll.len(), 2);
        assert_eq!(payroll[0].id, "payroll");
        assert_eq!(payroll[1].id, "payroll");
        assert_ne!(payroll[0].group, payroll[1].group);
        let solo = reread
            .lines
            .iter()
            .find(|line| line.account.0 == "expenses:solo")
            .unwrap();
        assert_eq!(solo.id, "rule:2:0");
    }

    #[test]
    fn a_description_is_the_rule_header_and_survives_a_round_trip() {
        const SOURCE: &str = "~ monthly  old words\n    (expenses:a)  $1\n    (expenses:b)  $2\n";
        let mut scenario = read(SOURCE);
        for line in &mut scenario.lines {
            line.note = "  rent, after the move  ".to_string();
        }
        let text = write_scenario(
            &ProjectionDoc::parse(SOURCE, "p.journal", &declared()).unwrap(),
            &scenario,
            "p.journal",
        )
        .unwrap();
        // ONE header for the rule, two spaces after the period, trimmed.
        assert!(
            text.contains("~ monthly  rent, after the move\n    (expenses:a)  $1\n"),
            "{text}"
        );
        let reread = read(&text);
        assert!(
            reread
                .lines
                .iter()
                .all(|line| line.note == "rent, after the move")
        );

        // Cleared, the header is the period alone.
        for line in &mut scenario.lines {
            line.note = String::new();
        }
        let text = write_scenario(
            &ProjectionDoc::parse(SOURCE, "p.journal", &declared()).unwrap(),
            &scenario,
            "p.journal",
        )
        .unwrap();
        assert!(text.contains("~ monthly\n"), "{text}");

        // And one that would read back as a comment is refused, not truncated.
        scenario.lines[0].note = "rent ; utilities".to_string();
        assert!(matches!(
            write_scenario(
                &ProjectionDoc::parse(SOURCE, "p.journal", &declared()).unwrap(),
                &scenario,
                "p.journal",
            ),
            Err(SerializeError::Invalid(_))
        ));
    }

    #[test]
    fn readable_ids_are_short_ascii_slugs_of_the_leaf() {
        assert_eq!(readable_id("expenses:Team Payroll"), "team-payroll");
        assert_eq!(readable_id("expenses:café"), "caf");
        assert_eq!(readable_id("expenses:日本"), "step");
        assert_eq!(readable_id("payroll"), "payroll");
        assert_eq!(
            readable_id(&format!("x:{}", "a".repeat(80))).len(),
            MAX_READABLE_ID
        );
        assert!(keeps_its_id("payroll"));
        assert!(keeps_its_id("payroll-2"));
        for machine in [
            "rule:3:0",
            "gap:expenses:food:$",
            "line-4",
            "group-12",
            "event-1",
            "a,b",
            "",
        ] {
            assert!(!keeps_its_id(machine), "{machine}");
        }
        assert!(keeps_its_id("line-up"));
    }

    #[test]
    fn a_display_precision_wider_than_the_value_is_written_out() {
        // The engine rounds a growing amount back to the display precision at
        // every step, so a `$1,875.00` line written as `$1875` would grow on
        // whole dollars from then on.
        let scenario = Scenario {
            auto_rules: Vec::new(),
            lines: vec![line(
                "rule:0",
                "rule:0:0",
                "expenses:a",
                usd(Dec::new(1875, 0), 2),
                "monthly",
            )],
            ..Scenario::default()
        };
        let text = new_file(&scenario, "p.journal", &declared()).unwrap();
        assert!(text.contains("$1875.00"), "{text}");
        assert_eq!(read(&text).lines[0].amount.style.precision, 2);
    }

    #[test]
    fn amounts_are_aligned_within_a_block_and_only_within_it() {
        let scenario = Scenario {
            auto_rules: Vec::new(),
            lines: vec![
                line(
                    "rule:0",
                    "rule:0:0",
                    "expenses:a",
                    usd(Dec::new(1, 0), 0),
                    "monthly",
                ),
                line(
                    "rule:0",
                    "rule:0:1",
                    "expenses:much:longer:name",
                    usd(Dec::new(2, 0), 0),
                    "monthly",
                ),
            ],
            ..Scenario::default()
        };
        let text = new_file(&scenario, "p.journal", &declared()).unwrap();
        let columns: Vec<usize> = text
            .lines()
            .filter(|line| line.contains('$'))
            .map(|line| line.find('$').unwrap_or_default())
            .collect();
        assert_eq!(columns.len(), 2);
        assert_eq!(columns[0], columns[1], "one column per block:\n{text}");
    }

    #[test]
    fn crlf_is_preserved() {
        const SOURCE: &str = "; projection: A\r\n\r\n~ monthly  p\r\n    (expenses:a)  $1\r\n";
        let scenario = read(SOURCE);
        let text = write_scenario(
            &ProjectionDoc::parse(SOURCE, "p.journal", &declared()).unwrap(),
            &scenario,
            "p.journal",
        )
        .unwrap();
        assert!(
            !text.contains("\n\n"),
            "no bare LF was introduced:\n{text:?}"
        );
        assert!(text.starts_with("; Ledgeline projection\r\n"), "{text:?}");
    }

    #[test]
    fn the_header_span_covers_only_lines_it_recognizes() {
        assert_eq!(header_span(""), 0..0);
        assert_eq!(header_span("account a\n"), 0..0);
        assert_eq!(header_span("; not a header\n; projection: x\n"), 0..0);
        assert_eq!(header_span("; projection: x\n\naccount a\n"), 0..16);
        assert_eq!(
            header_span("; Ledgeline projection\n; created: 2026-01-01\n"),
            0..45
        );
    }

    #[test]
    fn a_saved_scenario_has_no_seeded_lines_left() {
        // A seeded row saved to disk is, from then on, a row the user wrote.
        // `write_scenario` renders every line as an ordinary `~` posting and
        // `chunk_print` never reads `source`, so there is nothing in the file
        // that could come back as `Unbudgeted` — which is why the flip is two
        // lines here and not a function.
        let scenario = Scenario {
            auto_rules: Vec::new(),
            lines: vec![ScenarioLine {
                source: LineSource::Unbudgeted,
                ..line(
                    "rule:0",
                    "rule:0:0",
                    "expenses:a",
                    usd(Dec::new(1, 0), 0),
                    "monthly",
                )
            }],
            ..Scenario::default()
        };
        // Through the FILE, not through a helper that sets the field: the claim
        // is about what the format can carry, so asserting it on a value we just
        // assigned would pin nothing.
        let text = new_file(&scenario, "p.journal", &declared()).expect("it writes");
        let reread = read(&text);
        assert_eq!(reread.lines[0].source, LineSource::Journal);
    }

    // -----------------------------------------------------------------------
    // Plan 28: counter-accounts, gains accounts and the `; cash:` header
    // -----------------------------------------------------------------------

    /// The plan-28 format: a rule whose lines name a counter-account is plain
    /// balanced hledger — regular postings and ONE amountless counter posting —
    /// an asset row's gains account rides in a `gains:` tag, an event balances
    /// against its own amountless posting, and the header names the scenario's
    /// default cash account. All of it reads back field for field.
    const BALANCED: &str = "\
; Ledgeline projection
; projection: Balanced
; cash: assets:checking

account equity:unrealized gains  ; type: E

~ monthly  projection
    revenues:salary   $-12000.00  ; growth: 3%/yr
    expenses:rent       $4200.00
    assets:checking

~ monthly  house
    assets:house           $0.00  ; growth: 4%/yr, gains: equity:home gains
    assets:checking

~ 2027-03-01  Series A
    assets:checking  $2000000.00
    equity:preferred
";

    #[test]
    fn a_balanced_file_reads_its_counters_gains_and_cash_header() {
        let scenario = read(BALANCED);
        assert_eq!(
            scenario.cash,
            Some(AccountName("assets:checking".to_string()))
        );
        let counters: Vec<(&str, Option<&str>)> = scenario
            .lines
            .iter()
            .map(|line| {
                (
                    line.account.0.as_str(),
                    line.counter.as_ref().map(|c| c.0.as_str()),
                )
            })
            .collect();
        // The amountless posting is the counter, never a line of its own.
        assert_eq!(
            counters,
            [
                ("revenues:salary", Some("assets:checking")),
                ("expenses:rent", Some("assets:checking")),
                ("assets:house", Some("assets:checking")),
            ]
        );
        assert_eq!(scenario.lines[2].role, LineRole::Asset);
        assert_eq!(
            scenario.lines[2].gains,
            Some(AccountName("equity:home gains".to_string()))
        );
        assert_eq!(scenario.events.len(), 1);
        assert_eq!(
            scenario.events[0].counter,
            Some(AccountName("equity:preferred".to_string()))
        );
        assert_eq!(
            scenario.events[0].postings.len(),
            1,
            "the counter is not a posting"
        );

        // Unchanged, it rewrites no byte; read back, it is the same scenario.
        let text = write_scenario(
            &ProjectionDoc::parse(BALANCED, "p.journal", &declared()).unwrap(),
            &scenario,
            "p.journal",
        )
        .expect("it writes");
        assert_eq!(read(&text), scenario);
    }

    /// Written fresh, the counter is the block's last, amountless posting, the
    /// header carries `; cash:`, and a gains account nobody declares is
    /// declared in the scenario file (Decision 4) — the main journal untouched.
    #[test]
    fn a_new_file_writes_counters_gains_declarations_and_cash() {
        let mut scenario = read(BALANCED);
        scenario.lines[2].gains = None; // the default gains account
        let text = new_file(&scenario, "p.journal", &declared()).expect("it writes");
        assert!(text.contains("; cash: assets:checking\n"), "{text}");
        assert!(
            text.contains("\naccount equity:unrealized gains  ; type: E\n"),
            "{text}"
        );
        assert!(
            text.contains(
                "~ monthly  projection\n    revenues:salary  $-12000.00  ; growth: 3%/yr\n    \
                 expenses:rent    $4200.00\n    assets:checking\n"
            ),
            "{text}"
        );
        assert!(
            text.contains("    assets:checking  $2000000.00\n    equity:preferred\n"),
            "{text}"
        );
        assert_eq!(read(&text), scenario);

        // A gains account the MAIN journal declares is not declared again.
        let mut main = declared();
        main.insert(
            DEFAULT_GAINS_ACCOUNT.to_string(),
            crate::reports::account_types::AccountType::Equity,
        );
        let again = new_file(&scenario, "p.journal", &main).expect("it writes");
        assert!(
            !again.contains("account equity:unrealized gains"),
            "{again}"
        );
    }

    /// A pre-plan-28 file keeps its virtual form until a counter is set; then
    /// its lines become plain postings with the counter beneath them. A line
    /// with no counter in a rule that has one stays virtual, outside balancing,
    /// exactly as hledger treats it.
    #[test]
    fn a_counter_upgrades_a_legacy_block_and_mixed_lines_stay_virtual() {
        let legacy = "~ monthly  plan\n    (expenses:rent)      $4200.00\n    (revenues:salary)  $-12000.00\n";
        let mut scenario = read(legacy);
        assert!(scenario.lines.iter().all(|line| line.counter.is_none()));
        scenario.lines[0].counter = Some(AccountName("assets:checking".to_string()));
        let doc = ProjectionDoc::parse(legacy, "p.journal", &declared()).unwrap();
        let text = write_scenario(&doc, &scenario, "p.journal").expect("it writes");
        assert!(
            text.contains(
                "    expenses:rent      $4200.00\n    (revenues:salary)  $-12000.00\n    assets:checking\n"
            ),
            "{text}"
        );
        let reread = read(&text);
        assert_eq!(reread.lines[0].counter, scenario.lines[0].counter);
        assert_eq!(reread.lines[1].counter, None);
    }

    /// One rule cannot carry two counter-accounts — hledger infers only one
    /// amountless posting — so a save that asks for it is refused, as two
    /// recurrences in one rule are.
    #[test]
    fn two_counters_in_one_rule_are_refused() {
        let mut scenario = read(BALANCED);
        scenario.lines[1].counter = Some(AccountName("assets:savings".to_string()));
        let error = new_file(&scenario, "p.journal", &declared()).unwrap_err();
        assert!(error.to_string().contains("counter-account"), "{error}");
    }

    /// A gains account belongs to an asset row only.
    #[test]
    fn a_gains_account_on_a_flow_line_is_refused() {
        let mut scenario = read(BALANCED);
        scenario.lines[0].gains = Some(AccountName("equity:gains".to_string()));
        let error = new_file(&scenario, "p.journal", &declared()).unwrap_err();
        assert!(error.to_string().contains("gains"), "{error}");
    }

    // -----------------------------------------------------------------------
    // `=` auto-posting rules: identity by block (plan 30 amendment 12)
    // -----------------------------------------------------------------------

    const RULES: &str = "; Ledgeline projection\n\
        ; projection: Rules\n\
        \n\
        = expenses:wages tag:_generated-transaction\n\
        \x20   expenses:payroll-tax   *0.1\n\
        \x20   assets:checking\n\
        \n\
        ~ monthly  payroll\n\
        \x20   expenses:wages      $9000.00\n\
        \x20   assets:checking\n\
        \n\
        = revenue:sales   ; hand-aligned, with a comment\n\
        \x20     expenses:commission    *-0.035\n\
        \x20     assets:checking\n\
        \n\
        =revenue:other\n\
        \x20   (liabilities:tax)  *0.05\n";

    /// A rule as the server builds one from a client's text: no block.
    fn new_rule(raw: &str) -> ScenarioAutoRule {
        let journal = parse::parse_journal(raw, "rule.journal").expect("a rule");
        ScenarioAutoRule {
            key: String::new(),
            block: None,
            rule: journal.auto_rules.into_iter().next().expect("one rule"),
        }
    }

    fn save(before: &str, scenario: &Scenario) -> String {
        let doc = ProjectionDoc::parse(before, "p.journal", &declared()).expect("parses");
        write_scenario(&doc, scenario, "p.journal").expect("writes")
    }

    #[test]
    fn rules_read_with_their_block_and_an_untouched_save_is_byte_identical() {
        let scenario = read(RULES);
        let blocks: Vec<Option<usize>> = scenario.auto_rules.iter().map(|r| r.block).collect();
        assert_eq!(blocks, [Some(0), Some(1), Some(2)]);
        assert_eq!(save(RULES, &scenario), RULES);
        // A client from before rules carried a block sends only the text:
        // each claims the block holding exactly that text.
        let mut anonymous = scenario.clone();
        for rule in &mut anonymous.auto_rules {
            rule.block = None;
        }
        assert_eq!(save(RULES, &anonymous), RULES);
    }

    #[test]
    fn editing_one_rule_rewrites_only_its_block() {
        let mut scenario = read(RULES);
        scenario.auto_rules[1] = ScenarioAutoRule {
            block: Some(1),
            ..new_rule(
                "= ^revenue:sales(:|$) tag:_generated-transaction\n    expenses:commission  *-0.04\n    assets:checking      *0.04\n",
            )
        };
        let after = save(RULES, &scenario);
        assert_eq!(
            after,
            RULES.replace(
                "= revenue:sales   ; hand-aligned, with a comment\n      expenses:commission    *-0.035\n      assets:checking\n",
                "= ^revenue:sales(:|$) tag:_generated-transaction\n    expenses:commission  *-0.04\n    assets:checking      *0.04\n",
            )
        );
    }

    #[test]
    fn deleting_a_rule_removes_its_block_and_one_blank_line() {
        let mut middle = read(RULES);
        middle.auto_rules.remove(1);
        assert_eq!(
            save(RULES, &middle),
            RULES.replace(
                "= revenue:sales   ; hand-aligned, with a comment\n      expenses:commission    *-0.035\n      assets:checking\n\n",
                "",
            )
        );
        // The last block of the file takes the blank line before it.
        let mut last = read(RULES);
        last.auto_rules.pop();
        assert_eq!(
            save(RULES, &last),
            RULES.replace("\n=revenue:other\n    (liabilities:tax)  *0.05\n", "")
        );
        let mut first = read(RULES);
        first.auto_rules.remove(0);
        assert_eq!(
            save(RULES, &first),
            RULES.replace(
                "= expenses:wages tag:_generated-transaction\n    expenses:payroll-tax   *0.1\n    assets:checking\n\n",
                "",
            )
        );
    }

    #[test]
    fn deleting_the_rules_that_end_the_file_removes_them_and_one_blank_line() {
        const WAGES: &str = "= expenses:wages tag:_generated-transaction\n    expenses:payroll-tax   *0.1\n    assets:checking\n\n";
        let (kept, _) = RULES.split_once("\n= revenue:sales").expect("fixture");
        // The last two blocks are adjacent and end the file: they go together,
        // with the one blank line before them.
        let mut last_two = read(RULES);
        last_two.auto_rules.truncate(1);
        assert_eq!(save(RULES, &last_two), kept);
        let mut none = read(RULES);
        none.auto_rules.clear();
        assert_eq!(save(RULES, &none), kept.replace(WAGES, ""));
        // With a `~` block after them they are not at EOF, and each takes the
        // blank line after it.
        let bonus = "~ yearly  bonus\n    expenses:bonus      $100.00\n    assets:checking\n";
        let followed = format!("{RULES}\n{bonus}");
        let mut scenario = read(&followed);
        scenario.auto_rules.truncate(1);
        assert_eq!(save(&followed, &scenario), format!("{kept}\n{bonus}"));
    }

    #[test]
    fn a_new_rule_is_appended_and_the_rest_keep_their_bytes() {
        let mut scenario = read(RULES);
        let added = "= ^expenses:rent(:|$) tag:_generated-transaction\n    expenses:rent-tax  *0.1\n    assets:checking    *-0.1\n";
        scenario.auto_rules.push(new_rule(added));
        assert_eq!(save(RULES, &scenario), format!("{RULES}\n{added}"));
        // With no file at all (Save As), the rules arrive only in the model.
        let empty = ProjectionDoc::empty(&declared());
        let created = write_scenario(&empty, &scenario, "p.journal").expect("writes");
        assert_eq!(read(&created).auto_rules.len(), 4);
    }

    /// The page keeps its own scenario across a save, so after a save that
    /// deleted the first rule the others still carry their OLD block numbers.
    /// The next save must still leave every byte alone.
    #[test]
    fn stale_block_numbers_after_a_save_still_find_their_own_blocks() {
        let mut scenario = read(RULES);
        scenario.auto_rules.remove(0);
        let saved = save(RULES, &scenario);
        // Blocks 1 and 2 are now blocks 0 and 1 on disk; the scenario says 1, 2.
        assert_eq!(save(&saved, &scenario), saved);
        // And an edit to the rule still named by its stale number lands on it.
        scenario.auto_rules[1] = ScenarioAutoRule {
            block: Some(2),
            ..new_rule("=revenue:other\n    (liabilities:tax)  *0.07\n")
        };
        assert_eq!(save(&saved, &scenario), saved.replace("*0.05", "*0.07"));
    }

    /// Moving a rule moves its text, because hledger applies rules in file
    /// order; the blocks stay where they were and swap contents.
    #[test]
    fn reordering_rules_reorders_their_text() {
        let mut scenario = read(RULES);
        scenario.auto_rules.swap(0, 2);
        let after = save(RULES, &scenario);
        let order: Vec<String> = read(&after)
            .auto_rules
            .iter()
            .map(|rule| rule.rule.query.raw().to_string())
            .collect();
        assert_eq!(
            order,
            [
                "revenue:other",
                "revenue:sales",
                "expenses:wages tag:_generated-transaction"
            ]
        );
        // The `~` block between them is untouched.
        assert!(
            after.contains("\n~ monthly  payroll\n    expenses:wages      $9000.00\n"),
            "{after}"
        );
    }

    #[test]
    fn a_crlf_file_keeps_its_line_ends_in_kept_and_rewritten_blocks() {
        let crlf = RULES.replace('\n', "\r\n");
        let scenario = read(&crlf);
        assert_eq!(save(&crlf, &scenario), crlf);
        let mut edited = scenario.clone();
        edited.auto_rules[2] = ScenarioAutoRule {
            block: Some(2),
            ..new_rule("=revenue:other\n    (liabilities:tax)  *0.06\n")
        };
        assert_eq!(save(&crlf, &edited), crlf.replace("*0.05", "*0.06"));
    }

    /// A save is refused rather than written when the rules would not read
    /// back exactly, in order — here a "rule" whose text is two blocks.
    #[test]
    fn a_save_whose_rules_do_not_read_back_exactly_is_refused() {
        let mut scenario = read(RULES);
        let mut doubled = new_rule("= food\n    (x)  1\n");
        doubled.rule.raw = "= food\n    (x)  1\n\n= rent\n    (y)  1\n".to_string();
        scenario.auto_rules.push(doubled);
        let doc = ProjectionDoc::parse(RULES, "p.journal", &declared()).expect("parses");
        let error = write_scenario(&doc, &scenario, "p.journal").unwrap_err();
        assert!(
            matches!(error, SerializeError::NotRoundTripped(_)),
            "{error}"
        );
    }

    /// A scenario's own `account ; type:` declaration is handed back beside the
    /// scenario, and wins over the main journal's in the merged map.
    #[test]
    fn a_scenario_s_own_account_types_come_back_and_win_the_merge() {
        let text = "account hmrc:paye  ; type: L\naccount assets:cash  ; type: C\n\n~ monthly  plan\n    expenses:rent    $100.00\n    assets:cash\n";
        let declared = BTreeMap::from([
            ("assets:cash".to_string(), AccountType::Asset),
            ("expenses:rent".to_string(), AccountType::Expense),
        ]);
        let (_, own) = scenario_and_own_types_from_text_with_styles(
            text,
            "scenario.journal",
            "Scenario",
            &declared,
            &[],
        )
        .expect("reads");
        assert_eq!(
            own,
            BTreeMap::from([
                ("assets:cash".to_string(), AccountType::Cash),
                ("hmrc:paye".to_string(), AccountType::Liability),
            ])
        );
        assert_eq!(
            scenario_account_types(&declared, &own),
            BTreeMap::from([
                ("assets:cash".to_string(), AccountType::Cash),
                ("expenses:rent".to_string(), AccountType::Expense),
                ("hmrc:paye".to_string(), AccountType::Liability),
            ])
        );
    }
}
