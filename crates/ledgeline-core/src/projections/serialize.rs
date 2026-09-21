//! Scenario ⇄ journal text (plan 22, Phase 3, §"The file format").
//!
//! A scenario file is an ordinary hledger journal. Reading one is
//! [`crate::parse::parse_journal`] over that file alone plus a header scan;
//! writing one is this module, and it is written **by splicing**, never by
//! re-rendering the file.
//!
//! # Never round-trip through `hledger print`
//!
//! `hledger print` drops `~` rules entirely, so a file that went through it
//! would come back with every scenario line gone and nothing to say it had
//! happened. Ledgeline writes these files itself.
//!
//! # The discipline: an edit rewrites the spans it names, and nothing else
//!
//! The same rule [`crate::periodic`] and [`crate::aliases`] state. A save
//! rewrites exactly two kinds of span:
//!
//! - the **header comment block** at the top of the file, and only the lines of
//!   it this module recognizes (the banner, `projection:`, `created:`,
//!   `updated:`) — a user's own prose above or between them is copied through;
//! - one **`~` block's whole extent** per scenario group, located by
//!   [`crate::periodic::PeriodicDoc`], which is the one model in the crate that
//!   knows where a rule's bytes begin and end.
//!
//! Everything else — `account` directives, transactions, comments between two
//! rules, the bytes after the last rule — comes out the `&str` slice it went
//! in as. A new group is appended at EOF; a group the scenario no longer has
//! has its block removed.
//!
//! ## Why not `PeriodicDoc::plan`/`apply`/`verify`
//!
//! Plan 22 Phase 3 says to reuse them, and the *locating* half is reused:
//! [`PeriodicDoc::parse`] is what says where each block is. Its **edit
//! vocabulary** is not, and cannot be — it is `SetAmount` / `Delete` /
//! `AppendLine` / `AppendBlock`, one written amount at a time, built for a
//! budget editor that changes a number inside a rule somebody else authored.
//! A scenario save rewrites a rule's period, its description, its accounts and
//! its `growth:`/`line:` tags, and `AppendBlock` can only write a *bare*
//! [`PeriodExpr`] header — so `~ monthly from 2027-04-01`, which is half the
//! format, is not expressible in it at all. See amendment 28.
//!
//! The half of `verify` that matters is kept, and strengthened: the caller
//! re-parses the written text as a journal and requires the scenario to read
//! back as the scenario it asked for ([`confirm_round_trip`]).
//!
//! # The mapping between a file and a model
//!
//! One `~` block is one **group**, and the group's id is `rule:<block index>`
//! — the same spelling [`super::seed_scenario`] gives the journal's own rules,
//! so a seeded scenario and a loaded one are indistinguishable in shape.
//!
//! | file | model |
//! |---|---|
//! | `~ monthly  projection` with N postings | N [`ScenarioLine`]s sharing `group: "rule:K"` |
//! | `~ 2027-03-01  Series A` | one [`ScenarioEvent`] with `id: "rule:K"` |
//! | a posting's `; line: <id>` | that line's logical-row id (a step change's two segments share it) |
//! | a posting's `; growth: 3%/yr` | that line's [`Growth`] |
//! | a `; growth:` on an ASSET-typed account | [`LineRole::Asset`]: the amount is the CONTRIBUTION |
//! | an asset row's `; opening: 200000` | that row's balance override |
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

use std::collections::BTreeMap;
use std::ops::Range;

use crate::decimal::Dec;
use crate::edit::render_amount;
use crate::model::{Amount, PeriodKind, PeriodicTransaction, Posting};
use crate::parse::{self, ParseError};
use crate::periodic::{MAX_ACCOUNT_BYTES, MAX_DESCRIPTION_BYTES, PeriodicDoc};
use crate::projections::discovery::{Header, parse_header};
use crate::projections::{
    Growth, GrowthUnit, LineRole, LineSource, Scenario, ScenarioEvent, ScenarioLine,
    virtual_posting,
};
use crate::reports::account_types::{AccountTypes, account_decls_from, declared_types};
use crate::rules::Newline;

/// The indent a posting line gets. Four spaces, which is what hledger's own
/// documentation and every fixture in this repo use.
const INDENT: &str = "    ";

/// The banner Ledgeline writes at the top of a file it created. Display only —
/// nothing keys off it, and a file without it loads identically.
const BANNER: &str = "; Ledgeline projection";

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
/// # Errors
/// [`SerializeError::Unreadable`] when the text is not a journal.
pub fn scenario_from_text(
    text: &str,
    source_name: &str,
    fallback_name: &str,
    declared: &BTreeMap<String, crate::reports::account_types::AccountType>,
) -> Result<Scenario, SerializeError> {
    let journal = parse::parse_journal(text, source_name)
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

    // The file's own `account` directives win over the caller's, so a scenario
    // that carries its declarations with it classifies the same way whether or
    // not a main journal is open.
    let mut types = declared.clone();
    types.extend(declared_types(&account_decls_from(&journal.accounts)));

    let header = parse_header(text);
    Ok(scenario_from_rules(
        &rules,
        &header,
        fallback_name,
        &AccountTypes::from_declared(types),
    ))
}

/// Build the model from a file's `~` rules and its header.
fn scenario_from_rules(
    rules: &[PeriodicTransaction],
    header: &Header,
    fallback_name: &str,
    types: &AccountTypes,
) -> Scenario {
    let mut lines = Vec::new();
    let mut events = Vec::new();
    for (index, rule) in rules.iter().enumerate() {
        let key = group_key(index);
        match event_date(rule) {
            Some(date) => events.push(ScenarioEvent {
                id: key,
                date,
                description: rule.description.clone(),
                // Flattened to one posting per amount, exactly as the wire
                // flattens an event on the way out — so a scenario that came
                // from a file and one that came back from a browser have the
                // same shape.
                postings: rule
                    .postings
                    .iter()
                    .flat_map(|posting| {
                        posting
                            .amounts
                            .iter()
                            .map(|amount| virtual_posting(posting.account.clone(), amount.clone()))
                    })
                    .collect(),
            }),
            None => lines.extend(super::lines_from_rule(&key, rule, types)),
        }
    }
    Scenario {
        name: header
            .name
            .clone()
            .unwrap_or_else(|| fallback_name.to_string()),
        created: header.created.clone(),
        updated: header.updated.clone(),
        lines,
        events,
    }
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
// Writing
// ===========================================================================

/// One projection file's text, with the spans a save rewrites located and what
/// each of them currently states.
///
/// Immutable: [`ProjectionDoc::apply`] returns a new `String` and never mutates
/// `self`, so a refused write cannot leave half a document behind.
#[derive(Debug, Clone)]
pub struct ProjectionDoc {
    text: String,
    newline: Newline,
    /// The leading run of RECOGNIZED header comment lines. Empty (and of zero
    /// width) when the file has none, in which case a header is inserted at
    /// that offset.
    header: Range<usize>,
    /// Each `~` block's whole extent, in file order, from [`PeriodicDoc`].
    blocks: Vec<Range<usize>>,
    /// What each block says, as [`chunk_print`] prints it.
    ///
    /// **This is what makes an unchanged block keep its own bytes.** Without
    /// it, every save would re-render every rule, and a user's column
    /// alignment, their two-space-versus-eight indent and their inline spacing
    /// would all be normalized away by an edit to a different rule. `None` for
    /// a block this module could not line up with a parsed rule, which makes it
    /// rewritten — the safe direction.
    current: Vec<Option<Vec<String>>>,
    /// The account types this document was read under, carried so that
    /// [`confirm_round_trip`] re-reads the written text the same way. Reading
    /// the result under a different classification would compare an asset row
    /// against the flow line it was not.
    declared: BTreeMap<String, crate::reports::account_types::AccountType>,
}

impl ProjectionDoc {
    /// Locate the spans of an existing file, and read what each block says.
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
        let doc = PeriodicDoc::parse(text);
        let blocks: Vec<Range<usize>> = doc
            .blocks()
            .iter()
            .map(|block| block.full.clone())
            .collect();
        let scenario = scenario_from_text(text, source_name, "", declared)?;

        // The blocks this module located, against the rules the journal parser
        // read. When they line up, each block's print is known and an unedited
        // one is left alone; when they do not, nothing is claimed to be known
        // and every block is rewritten.
        let mut current = vec![None; blocks.len()];
        let assignment = assign_to(&scenario, blocks.len());
        if assignment.appended.is_empty() {
            for (index, chunk) in &assignment.claimed {
                current[*index] = chunk_print(chunk).ok();
            }
        }

        Ok(Self {
            newline: Newline::detect(text),
            header: header_span(text),
            blocks,
            current,
            text: text.to_string(),
            declared: declared.clone(),
        })
    }

    /// An empty document, for a create.
    #[must_use]
    pub fn empty(declared: &BTreeMap<String, crate::reports::account_types::AccountType>) -> Self {
        Self {
            text: String::new(),
            newline: Newline::Lf,
            header: 0..0,
            blocks: Vec::new(),
            current: Vec::new(),
            declared: declared.clone(),
        }
    }

    /// How many `~` blocks the file has. The caller checks this against the
    /// parsed rule count, the way `budget_api` does, so a document this module
    /// located differently from the journal parser is a `409` rather than a
    /// write into the wrong bytes.
    #[must_use]
    pub fn block_count(&self) -> usize {
        self.blocks.len()
    }

    /// Render `scenario` into this document, rewriting only the header span and
    /// the `~` blocks whose content actually changed.
    ///
    /// # Errors
    /// [`SerializeError::Invalid`] for a field that cannot be written.
    pub fn apply(&self, scenario: &Scenario) -> Result<String, SerializeError> {
        let chunks = self.assign(scenario);
        let nl = self.newline.as_str();

        // Splices, in file order: the header first (it is at offset 0), then
        // one per block. All of them rendered before a byte is written, so an
        // invalid field refuses the whole save rather than half of it.
        let mut splices: Vec<(Range<usize>, String)> = Vec::new();
        let header = self.render_header(scenario)?;
        if header != self.text[self.header.clone()] {
            splices.push((self.header.clone(), header));
        }
        for (index, span) in self.blocks.iter().enumerate() {
            match chunks.claimed.get(&index) {
                Some(chunk) => {
                    // The one comparison that keeps an untouched rule's own
                    // bytes: what the scenario says about this block, against
                    // what the block already says. Equal means no splice at
                    // all, so the user's own layout survives an edit to a
                    // different rule.
                    if self.current[index].as_ref() == Some(&chunk_print(chunk)?) {
                        continue;
                    }
                    splices.push((span.clone(), self.render_chunk(chunk)?));
                }
                // The scenario no longer has this block: it goes.
                None => splices.push((span.clone(), String::new())),
            }
        }

        let mut out = String::with_capacity(self.text.len() + 256);
        let mut cursor = 0usize;
        for (span, body) in splices {
            // `PeriodicDoc`'s spans are ascending and non-overlapping, and the
            // header span ends at or before the first of them (it is the
            // leading comment block). A `debug_assert` rather than a branch:
            // if it were ever false the output would be silently scrambled.
            debug_assert!(span.start >= cursor, "spans must be ascending");
            out.push_str(&self.text[cursor..span.start]);
            out.push_str(&body);
            cursor = span.end;
        }
        out.push_str(&self.text[cursor..]);

        // Anything the file had no block for goes at EOF, which is where a new
        // rule goes in every other writer here.
        if !chunks.appended.is_empty() {
            if !out.is_empty() && !out.ends_with(nl) {
                out.push_str(nl);
            }
            for chunk in &chunks.appended {
                if !out.is_empty() && !out.ends_with(&format!("{nl}{nl}")) {
                    out.push_str(nl);
                }
                out.push_str(&self.render_chunk(chunk)?);
            }
        }
        Ok(out)
    }

    /// Work out which scenario chunk each existing block holds, and which
    /// chunks have no block yet.
    fn assign<'a>(&self, scenario: &'a Scenario) -> Assignment<'a> {
        assign_to(scenario, self.blocks.len())
    }

    /// The header comment block, as this save wants it written.
    ///
    /// Every line is rewritten on every save: `created:` is carried through
    /// from what was loaded, `updated:` is whatever the caller set. A blank
    /// line follows it when there is anything after it, so the header cannot be
    /// glued onto a directive.
    fn render_header(&self, scenario: &Scenario) -> Result<String, SerializeError> {
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
        // A blank line, unless the header is the whole file. `trim_start` on
        // the remainder rather than a length test: a file that already begins
        // its body with a blank line must not gain a second one on every save.
        let rest = &self.text[self.header.end..];
        if !rest.is_empty() && !rest.starts_with(['\n', '\r']) {
            out.push_str(nl);
        }
        Ok(out)
    }

    /// One `~` block, header line and postings, terminated.
    fn render_chunk(&self, chunk: &Chunk<'_>) -> Result<String, SerializeError> {
        let nl = self.newline.as_str();
        let (period, description, rows) = match chunk {
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
                let rows = lines
                    .iter()
                    .enumerate()
                    .map(|(at, line)| {
                        Ok(Row {
                            account: &line.account.0,
                            amount: &line.amount,
                            tags: line_tags(key, at, line)?,
                        })
                    })
                    .collect::<Result<Vec<_>, SerializeError>>()?;
                (period, description, rows)
            }
            Chunk::Event(event) => {
                let rows = event
                    .postings
                    .iter()
                    .flat_map(|posting: &Posting| {
                        posting.amounts.iter().map(move |amount| Row {
                            account: &posting.account.0,
                            amount,
                            // An event's postings carry no `line:` — the event
                            // IS the row, and its id is its block's index.
                            tags: String::new(),
                        })
                    })
                    .collect::<Vec<_>>();
                (event.date.as_str(), event.description.as_str(), rows)
            }
        };

        let mut out = String::new();
        out.push_str(&rule_header(period, description)?);
        out.push_str(nl);
        // One column for the whole block, so a rule reads as a table. Computed
        // from the rendered accounts rather than guessed, and deterministic, so
        // a scenario written twice produces identical bytes.
        let width = rows
            .iter()
            .map(|row| wrapped(row.account).chars().count())
            .max()
            .unwrap_or(0);
        for row in &rows {
            out.push_str(&render_row(row, width)?);
            out.push_str(nl);
        }
        Ok(out)
    }
}

/// Where each chunk of a scenario goes.
struct Assignment<'a> {
    /// Block index → the chunk that rewrites it.
    claimed: BTreeMap<usize, Chunk<'a>>,
    /// Chunks with no block of their own, in scenario order.
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

/// Work out which of `blocks` existing `~` blocks each chunk of `scenario`
/// rewrites, and which chunks have no block yet.
///
/// A chunk claims block K when its key is `rule:K` and K is a block the file
/// has. First claimant wins — two chunks cannot share one block's bytes — and
/// the loser is appended, which is visible rather than lost.
fn assign_to<'a>(scenario: &'a Scenario, blocks: usize) -> Assignment<'a> {
    let mut claimed: BTreeMap<usize, Chunk<'a>> = BTreeMap::new();
    let mut appended: Vec<Chunk<'a>> = Vec::new();

    // Recurring lines, grouped by `group` in FIRST-APPEARANCE order. Iterating
    // a `BTreeMap` instead would put `rule:10` before `rule:2`, and the order
    // chunks are appended in is the order the table shows them.
    let mut order: Vec<&str> = Vec::new();
    let mut groups: BTreeMap<&str, Vec<&ScenarioLine>> = BTreeMap::new();
    for line in &scenario.lines {
        if !groups.contains_key(line.group.as_str()) {
            order.push(&line.group);
        }
        groups.entry(&line.group).or_default().push(line);
    }

    let mut place =
        |key: &str, chunk: Chunk<'a>| match block_of(key).filter(|index| *index < blocks) {
            Some(index) if !claimed.contains_key(&index) => {
                claimed.insert(index, chunk);
            }
            _ => appended.push(chunk),
        };
    for key in order {
        place(
            key,
            Chunk::Rule {
                key,
                lines: groups.remove(key).unwrap_or_default(),
            },
        );
    }
    for event in &scenario.events {
        place(&event.id, Chunk::Event(event));
    }
    Assignment { claimed, appended }
}

/// What one chunk SAYS, as a list of strings — one per row, headed by the
/// rule's own line.
///
/// This is the module's notion of "the same block". It carries every field the
/// file states and **no field the file derives**: not the group id, which is
/// the block's own position and renumbers whenever a block is added or removed,
/// and not a row's id unless a `line:` tag is actually written for it. Two
/// prints being equal means writing one over the other would change nothing.
///
/// Used for two different jobs, which is why it is one function: deciding
/// whether a block needs rewriting at all, and proving the written file reads
/// back as the scenario that was asked for.
///
/// # Errors
/// [`SerializeError::Invalid`] when a field cannot be rendered.
fn chunk_print(chunk: &Chunk<'_>) -> Result<Vec<String>, SerializeError> {
    let mut rows = Vec::new();
    match chunk {
        Chunk::Rule { key, lines } => {
            let period = lines
                .first()
                .map_or("monthly", |line| line.period.raw.as_str());
            let note = lines.first().map_or("", |line| line.note.as_str());
            rows.push(format!("~ {period} | {note}"));
            for (at, line) in lines.iter().enumerate() {
                rows.push(format!(
                    "{} {} | {} | line {} | growth {} | opening {} | every {}",
                    line.account.0,
                    render_scenario_amount(&line.amount)?,
                    line.role.as_str(),
                    row_id_tag(key, at, &line.id).unwrap_or("-"),
                    line.growth
                        .as_ref()
                        .map_or_else(|| "-".to_string(), render_growth),
                    line.opening
                        .as_ref()
                        .map(|opening| render_opening(line, opening))
                        .transpose()?
                        .unwrap_or_else(|| "-".to_string()),
                    line.period.raw,
                ));
            }
        }
        Chunk::Event(event) => {
            rows.push(format!("~ {} | {}", event.date, event.description));
            for posting in &event.postings {
                for amount in &posting.amounts {
                    rows.push(format!(
                        "{} {}",
                        posting.account.0,
                        render_scenario_amount(amount)?
                    ));
                }
            }
        }
    }
    Ok(rows)
}

/// The `line:` tag value a row needs, or `None` when the id is the one a reader
/// derives from the block: `rule:K:N` for the Nth posting of block K.
///
/// Writing the tag always would put a machine handle on every line of every
/// file for the sake of the few that need one — and the few that need one are
/// exactly the step changes the tag exists for. Both sides of a round-trip
/// comparison ask this about their OWN key, which is what makes the answer
/// stable when a block's index has moved.
fn row_id_tag<'a>(key: &str, at: usize, id: &'a str) -> Option<&'a str> {
    (id != format!("{key}:{at}")).then_some(id)
}

/// One posting line, before it is laid out.
struct Row<'a> {
    account: &'a str,
    amount: &'a Amount,
    tags: String,
}

/// `(account)` — every projection posting is an unbalanced virtual one, exactly
/// as budget goals are. There is no funding leg and none is wanted; the
/// engine's residual rule says how the cash effect is derived.
fn wrapped(account: &str) -> String {
    format!("({account})")
}

/// `    (account)   $1,200.00  ; growth: 3%/yr`
fn render_row(row: &Row<'_>, width: usize) -> Result<String, SerializeError> {
    check_account(row.account)?;
    let account = wrapped(row.account);
    let amount = render_scenario_amount(row.amount)?;
    // `chars`, not `len`: a padding computed in bytes puts a multi-byte account
    // name's amount in the wrong column. Two spaces minimum, because that is
    // what separates an account from an amount.
    let pad = width.saturating_sub(account.chars().count()) + 2;
    let mut line = format!("{INDENT}{account}{}{amount}", " ".repeat(pad));
    if !row.tags.is_empty() {
        line.push_str("  ; ");
        line.push_str(&row.tags);
    }
    Ok(line)
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
fn line_tags(key: &str, at: usize, line: &ScenarioLine) -> Result<String, SerializeError> {
    let mut parts = Vec::new();
    if let Some(id) = row_id_tag(key, at, &line.id) {
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
fn check_account(account: &str) -> Result<(), SerializeError> {
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
            "projection" | "created" | "updated"
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
/// of its blocks says is compared with what the scenario asked for — so a
/// growth rate that rendered into an unreadable tag, an amount whose precision
/// moved, or an account name that split at a double space fails here rather
/// than on disk.
///
/// The comparison is [`chunk_print`], which carries every field the FILE states
/// and none it derives. It deliberately does not compare whole [`Scenario`]s:
/// `source` is a UI flag no file carries (a seeded row saved to disk is, from
/// then on, a row the user authored), and a group id is a block's own position,
/// which renumbers the moment a block is added or removed.
///
/// # Errors
/// [`SerializeError`] — the render refused, the result is not a journal, or it
/// does not read back as the scenario.
pub fn write_scenario(
    doc: &ProjectionDoc,
    scenario: &Scenario,
    source_name: &str,
) -> Result<String, SerializeError> {
    let text = doc.apply(scenario)?;
    confirm_round_trip(doc, &text, scenario, source_name)?;
    Ok(text)
}

/// Re-read `text` and require its blocks to state `scenario`.
fn confirm_round_trip(
    doc: &ProjectionDoc,
    text: &str,
    scenario: &Scenario,
    source_name: &str,
) -> Result<(), SerializeError> {
    let want = prints(&doc.assign(scenario))?;
    let written = scenario_from_text(text, source_name, "", &doc.declared)?;
    // Against a document with NO blocks, so every chunk of the re-read scenario
    // is printed in file order — the order `want` is already in.
    let got = prints(&assign_to(&written, 0))?;
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

/// Every chunk of an assignment, printed, in the order they land in the file:
/// the claimed blocks by index, then the appended ones.
fn prints(assignment: &Assignment<'_>) -> Result<Vec<String>, SerializeError> {
    let mut out = Vec::new();
    for chunk in assignment
        .claimed
        .values()
        .chain(assignment.appended.iter())
    {
        out.extend(chunk_print(chunk)?);
    }
    Ok(out)
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
    write_scenario(&ProjectionDoc::empty(declared), scenario, source_name)
}

/// A scenario as it is once loaded from a file: every line authored, every
/// group and id canonical.
///
/// Exposed for tests and for the one caller that wants to compare what it is
/// about to write with what is already there.
#[must_use]
pub fn authored(mut scenario: Scenario) -> Scenario {
    for line in &mut scenario.lines {
        line.source = LineSource::Journal;
    }
    scenario
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AccountName, AmountStyle, Commodity, CommoditySide};
    use crate::parse::parse_period_spec;

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

    fn read(text: &str) -> Scenario {
        scenario_from_text(text, "p.journal", "fallback", &declared()).expect("the example parses")
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
        let second =
            scenario_from_text(&text, "p.journal", "fallback", &declared()).expect("it re-reads");
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
        assert_eq!(
            scenario_from_text(&text, "p.journal", "x", &declared()).unwrap(),
            scenario
        );
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
        assert_eq!(text, ASSETS, "an unchanged scenario rewrites no byte");
        assert_eq!(read(&text), scenario);
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
        let reread = scenario_from_text(&text, "p.journal", "x", &declared()).unwrap();
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
    fn an_id_that_would_not_survive_the_tag_grammar_is_refused() {
        for id in ["a,b", "a;b", " a", ""] {
            let scenario = Scenario {
                lines: vec![line(
                    "rule:0",
                    id,
                    "expenses:a",
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
                "accepted `{id}`"
            );
        }
    }

    #[test]
    fn the_line_tag_is_written_only_when_the_id_is_not_derivable() {
        let scenario = Scenario {
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
                    "payroll",
                    "expenses:b",
                    usd(Dec::new(2, 0), 0),
                    "monthly",
                ),
            ],
            ..Scenario::default()
        };
        let text = new_file(&scenario, "p.journal", &declared()).unwrap();
        assert!(text.contains("(expenses:a)  $1\n"), "{text}");
        assert!(
            text.contains("(expenses:b)  $2  ; line: payroll\n"),
            "{text}"
        );
    }

    #[test]
    fn a_display_precision_wider_than_the_value_is_written_out() {
        // The engine rounds a growing amount back to the display precision at
        // every step, so a `$1,875.00` line written as `$1875` would grow on
        // whole dollars from then on.
        let scenario = Scenario {
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
    fn authored_clears_the_seeded_flag() {
        // A seeded row saved to disk is, from then on, a row the user wrote.
        let scenario = authored(Scenario {
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
        });
        assert_eq!(scenario.lines[0].source, LineSource::Journal);
    }
}
