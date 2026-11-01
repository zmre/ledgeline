//! Review before an import writes (plan 32 Phase 4: D23–D26, D30).
//!
//! An import's proposal is not in the journal yet, so nothing about it can be
//! edited through the journal editor. This module is how a review card edits
//! it anyway, while keeping "the preview is the bytes"
//! (`docs/development/imports-internals.md`):
//!
//! - **Overrides.** A review save during an import becomes an override,
//!   `{ordinal | id, fingerprint, ops}`, with the batch route's own ops. The
//!   server regenerates the proposal, checks each override's fingerprint
//!   against the regenerated transaction at that place, and applies the ops
//!   with the engine's own [`JournalEditor::apply_batch`] — over a memory vault
//!   holding the proposal text ([`StagedText`]). The dry run returns that
//!   result and the commit appends it, so the two cannot disagree. A stale
//!   fingerprint is a `409` naming the transaction.
//! - **Staged rows** ([`wire_txns`]): every proposed transaction in the review
//!   queue's row shape, with the fingerprint of its GENERATED text (the
//!   override key), its similarity key, and suggestions — the journal's own
//!   precedent plus D13a source 2, the import's rules file.
//! - **Raw records** ([`align_records`]): the CSV record behind each staged
//!   transaction, for the rule builder's match pattern (D25), or an honest
//!   `aligned: false` when they cannot be lined up.
//! - **The rule preview's rules** ([`preview_rules`]): a candidate `if` block
//!   appended through the rules model, never by hand.

use crate::error::AppError;
use crate::review_api::{EditOpIn, ReviewRow, WireSuggestion, check_op_list, engine_ops};
use ledgeline_core::model::{Journal, Transaction};
use ledgeline_core::review::{self, ReviewState, is_unknown, similarity_key};
use ledgeline_core::rules::{
    self, EditPlan, HledgerField, ItemBody, ItemKind, MatchScope, MatcherGroupSpec, MatcherSpec,
    RulesDoc, Slot,
};
use ledgeline_core::vault::{MemVault, VaultPath};
use ledgeline_core::view::rows::RowContext;
use ledgeline_core::{EditError, Fingerprint, JournalEditor};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::sync::Arc;

/// The most overrides one dry run or commit may carry: every row of a big
/// statement, reviewed, with room to spare.
pub(crate) const MAX_OVERRIDES: usize = 10_000;

/// The name the staged proposal is edited under, in its memory vault. Never a
/// path on disk, so an edit error naming it discloses nothing.
const STAGED_FILE: &str = "proposed.journal";

/// The most `changed` rows a rule preview lists (D25: enough to see that a
/// pattern is too broad).
pub(crate) const MAX_PREVIEW_CHANGES: usize = 50;

/// The `code` the rule preview's probe block writes, to tell which rows its
/// matchers select regardless of what the rule's own assignments change.
pub(crate) const PROBE_CODE: &str = "ledgeline-rule-preview-probe";

/// The longest a rule-preview matcher pattern or assignment value may be.
const MAX_RULE_TEXT_BYTES: usize = 4_096;

/// The most matchers or assignments one previewed rule may carry.
const MAX_RULE_PARTS: usize = 32;

// ===========================================================================
// Wire
// ===========================================================================

/// One CSV-import override: the staged transaction at `ordinal` (0-based, in
/// the proposal's own order), the fingerprint of the generated text it was
/// reviewed on, and the ops to apply to it.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WireOverride {
    pub(crate) ordinal: usize,
    pub(crate) fingerprint: String,
    pub(crate) ops: Vec<EditOpIn>,
}

/// One QuickBooks override (D30): keyed by the export's Trans # rather than
/// by position — it is stable, and already the dedupe key.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WireQbOverride {
    pub(crate) id: String,
    pub(crate) fingerprint: String,
    pub(crate) ops: Vec<EditOpIn>,
}

/// A staged transaction as the review overlay reads it: the review queue's
/// item shape (a journal row with its markers, fingerprint, key and
/// suggestions) plus where it sits in the import.
///
/// The row is the CURRENT transaction — overrides applied — while
/// `fingerprint` and `key` describe the GENERATED one: the fingerprint is what
/// an override must echo, and the key (D11) is computed from the original
/// description.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireStagedTxn {
    #[serde(flatten)]
    row: ReviewRow<'static>,
    fingerprint: String,
    key: Option<String>,
    suggestions: Vec<WireSuggestion>,
    /// CSV: the 0-based position in `entries`.
    #[serde(skip_serializing_if = "Option::is_none")]
    ordinal: Option<usize>,
    /// QuickBooks: the Trans #.
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    /// QuickBooks: `new`, `unchanged` or `changed` against the journal.
    #[serde(skip_serializing_if = "Option::is_none")]
    classification: Option<&'static str>,
    /// An override was applied to it.
    overridden: bool,
}

/// The raw CSV record behind each staged transaction (D25).
///
/// `rows[i]` belongs to `txns[i]` when `aligned`; when the two cannot be lined
/// up (a `skip`/`end` rule, a header line the rules file reads as data, a
/// proposal our parser cannot read) `aligned` is false and `rows` is empty, and
/// the rule builder falls back to whole-record matching.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireRecords {
    aligned: bool,
    /// The converted CSV's header row, or `null` for a file that has none.
    header: Option<Vec<String>>,
    /// The rules file's `fields` names, as written, or `null` when it has no
    /// `fields` list.
    fields: Option<Vec<String>>,
    rows: Vec<Vec<String>>,
}

/// One matcher of a previewed rule: `field` names a CSV field (`description`,
/// `%3` spelled `3`), absent or `null` matches the whole record.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WireRuleMatcher {
    #[serde(default)]
    field: Option<String>,
    pattern: String,
}

/// One assignment of a previewed rule.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WireRuleAssignment {
    field: String,
    value: String,
}

/// The rule a preview tries: one `if` block, its matchers OR-ed (hledger's
/// stacked form), its assignments applied to every row they select.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WireRule {
    matchers: Vec<WireRuleMatcher>,
    assignments: Vec<WireRuleAssignment>,
}

impl WireRule {
    /// Every matcher's pattern, in order — what the Journal view's fallback
    /// check (plan 32 D27) tests descriptions against.
    pub(crate) fn patterns(&self) -> Vec<&str> {
        self.matchers
            .iter()
            .map(|matcher| matcher.pattern.as_str())
            .collect()
    }
}

/// One row the previewed rule would change.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireRuleChange {
    pub(crate) ordinal: usize,
    pub(crate) before: String,
    pub(crate) after: String,
}

/// What a previewed rule would do to this import.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireRulePreviewed {
    /// Always `true`.
    pub(crate) ok: bool,
    /// Rows the rule's matchers select.
    pub(crate) matched: usize,
    /// Rows in this import.
    pub(crate) total: usize,
    /// Of `matched`, the ones another rule had already categorised.
    pub(crate) already_categorised: usize,
    /// Rows whose generated text the rule changes, at most
    /// [`MAX_PREVIEW_CHANGES`].
    pub(crate) changed: Vec<WireRuleChange>,
    /// How many rows it changes, listed or not.
    pub(crate) changed_total: usize,
}

// ===========================================================================
// The staged proposal, as an editable text
// ===========================================================================

/// An import's proposal held by the engine's own editor, over a memory vault:
/// what lets an override be applied with exactly the batch route's semantics,
/// and the result read back as text.
///
/// Fingerprints are the editor's own over each transaction's span text, so they
/// depend on that text alone — not on where it sits — and the dry run and the
/// commit, which regenerate the same proposal, agree on them.
pub(crate) struct StagedText {
    editor: JournalEditor,
    /// The proposal as generated, before any override.
    generated: Arc<Journal>,
    fingerprints: Vec<Fingerprint>,
    overridden: Vec<bool>,
}

/// One override, resolved: where it applies, what it must find there, its
/// ops, and the words a failure names it by.
pub(crate) struct StagedEdit<'a> {
    pub(crate) position: usize,
    pub(crate) fingerprint: &'a str,
    pub(crate) ops: &'a [EditOpIn],
    pub(crate) label: String,
}

impl StagedText {
    /// Hold `text` (journal text: an import's proposal, or rendered
    /// QuickBooks transactions).
    ///
    /// # Errors
    /// `500` when it does not parse — it is hledger's or the engine's own
    /// output, so that is a bug, not a request to refuse.
    pub(crate) fn parse(text: &str) -> Result<Self, AppError> {
        let vault = MemVault::new().with_file(STAGED_FILE, text);
        let path = VaultPath::new(STAGED_FILE)
            .map_err(|_| AppError::Internal("the staged proposal has no name".to_string()))?;
        let editor = JournalEditor::open_in(Arc::new(vault), path).map_err(|error| {
            AppError::Internal(format!(
                "the proposed transactions could not be read back for review: {error}"
            ))
        })?;
        let generated = Arc::clone(editor.journal());
        let indices: Vec<_> = generated.transactions.iter().map(|txn| txn.index).collect();
        let fingerprints = editor
            .transaction_fingerprints(&indices)
            .into_iter()
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| {
                AppError::Internal("a proposed transaction has no source text".to_string())
            })?;
        let overridden = vec![false; fingerprints.len()];
        Ok(Self {
            editor,
            generated,
            fingerprints,
            overridden,
        })
    }

    /// How many transactions it holds.
    pub(crate) fn len(&self) -> usize {
        self.fingerprints.len()
    }

    /// The proposal as generated.
    pub(crate) fn generated(&self) -> &Journal {
        &self.generated
    }

    /// The proposal with every override applied.
    pub(crate) fn current(&self) -> &Journal {
        self.editor.journal()
    }

    /// The text with every override applied — untouched transactions byte for
    /// byte as given.
    pub(crate) fn text(&self) -> String {
        self.editor.text()
    }

    /// The fingerprint token of the generated transaction at `position`.
    pub(crate) fn token(&self, position: usize) -> Option<String> {
        self.fingerprints.get(position).map(Fingerprint::token)
    }

    /// Apply `edits` as ONE batch. `styles` is the journal a `replace` op's
    /// amounts take their styles from (the open journal's, as the batch route
    /// does).
    ///
    /// # Errors
    /// - `409` naming the transaction when its fingerprint is not the one the
    ///   override echoes, or its place no longer exists — the proposal was
    ///   regenerated differently (a rules file changed underneath);
    /// - `400` for a place named twice, or an op that is refused, naming the
    ///   transaction.
    ///
    /// Nothing is applied on any error.
    pub(crate) fn apply(
        &mut self,
        edits: &[StagedEdit<'_>],
        styles: &Journal,
    ) -> Result<(), AppError> {
        if edits.is_empty() {
            return Ok(());
        }
        let mut seen = HashSet::new();
        let mut batch = Vec::with_capacity(edits.len());
        for edit in edits {
            check_op_list(edit.ops).map_err(|error| prefixed(error, &edit.label))?;
            if edit.ops.is_empty() {
                return Err(AppError::BadRequest(format!(
                    "{}: an override needs at least one op",
                    edit.label
                )));
            }
            if !seen.insert(edit.position) {
                return Err(AppError::BadRequest(format!(
                    "{} is overridden twice in one request",
                    edit.label
                )));
            }
            let (Some(txn), Some(fingerprint)) = (
                self.generated.transactions.get(edit.position),
                self.fingerprints.get(edit.position),
            ) else {
                return Err(stale_staged(&edit.label));
            };
            if fingerprint.token() != edit.fingerprint {
                return Err(stale_staged(&edit.label));
            }
            let ops = engine_ops(styles, edit.ops).map_err(|error| prefixed(error, &edit.label))?;
            batch.push((txn.index, fingerprint.clone(), ops));
        }
        self.editor
            .apply_batch(&batch)
            .map_err(|error| match error {
                EditError::BulkTransaction { index, source } => {
                    let label = edits
                        .get(index)
                        .map_or("a staged transaction", |edit| &edit.label);
                    match *source {
                        EditError::ExternalChange => stale_staged(label),
                        source => {
                            let message = format!("{label}: {source}");
                            AppError::from(source).with_message(message)
                        }
                    }
                }
                other => AppError::from(other),
            })?;
        for edit in edits {
            self.overridden[edit.position] = true;
        }
        Ok(())
    }

    /// Whether an override was applied at `position`.
    pub(crate) fn is_overridden(&self, position: usize) -> bool {
        self.overridden.get(position).copied().unwrap_or(false)
    }
}

/// The `409` for an override whose transaction is not what it was reviewed on.
fn stale_staged(label: &str) -> AppError {
    AppError::Conflict(format!(
        "{label} changed since it was reviewed (the rules file or the statement changed \
         underneath); nothing was written — run the dry run again and re-apply the review edits"
    ))
}

/// `error`'s message with `label: ` in front, same status.
fn prefixed(error: AppError, label: &str) -> AppError {
    let message = format!("{label}: {error}");
    error.with_message(message)
}

/// The CSV overrides of a request, resolved against `staged`.
pub(crate) fn csv_edits<'a>(
    staged: &StagedText,
    overrides: &'a [WireOverride],
) -> Vec<StagedEdit<'a>> {
    overrides
        .iter()
        .map(|o| StagedEdit {
            position: o.ordinal,
            fingerprint: &o.fingerprint,
            ops: &o.ops,
            label: csv_label(staged, o.ordinal),
        })
        .collect()
}

/// "staged transaction 3 (2026-01-16 STARBUCKS)" — the ordinal, plus what it
/// was generated as when there is still one there.
fn csv_label(staged: &StagedText, ordinal: usize) -> String {
    staged.generated().transactions.get(ordinal).map_or_else(
        || format!("staged transaction {ordinal}"),
        |txn| {
            format!(
                "staged transaction {ordinal} ({} {})",
                txn.date, txn.description
            )
        },
    )
}

/// Refuse a request carrying more overrides than any import could.
pub(crate) fn check_override_count(count: usize) -> Result<(), AppError> {
    if count > MAX_OVERRIDES {
        return Err(AppError::BadRequest(format!(
            "a request may carry at most {MAX_OVERRIDES} overrides, got {count}"
        )));
    }
    Ok(())
}

// ===========================================================================
// Staged rows
// ===========================================================================

/// Where one staged transaction sits in its import, for [`wire_txns`].
pub(crate) enum StagedPlace {
    /// A CSV proposal's ordinal.
    Csv(usize),
    /// A QuickBooks transaction's Trans # and classification.
    Qb(String, &'static str),
}

/// Every staged transaction in the review row shape, in order.
///
/// Rows are judged against the OPEN journal (`snapshot`): its account
/// classes decide "unknown", its declared styles render the amounts, and its
/// review index gives the precedent. `rules` adds D13a source 2.
pub(crate) fn wire_txns(
    snapshot: &crate::Snapshot,
    staged: &StagedText,
    rules: Option<&RulesDoc>,
    place: impl Fn(usize) -> StagedPlace,
) -> Vec<WireStagedTxn> {
    let journal = &snapshot.journal;
    let classes = snapshot.view.classes(journal);
    let declared = snapshot.view.declared(journal);
    let review = snapshot.view.review(journal);
    let context = RowContext::new(journal, &declared);
    staged
        .current()
        .transactions
        .iter()
        .zip(&staged.generated().transactions)
        .enumerate()
        .map(|(position, (current, generated))| {
            let state = ReviewState::of(current);
            let rule_file = rules
                .map(|doc| review::rule_file_suggestions(doc, &generated.description, &classes))
                .unwrap_or_default();
            let (ordinal, id, classification) = match place(position) {
                StagedPlace::Csv(ordinal) => (Some(ordinal), None, None),
                StagedPlace::Qb(id, classification) => (None, Some(id), Some(classification)),
            };
            WireStagedTxn {
                row: ReviewRow::marked(
                    context.row(current, Vec::new()),
                    Some(&state),
                    is_unknown(current, &classes),
                ),
                fingerprint: staged.token(position).unwrap_or_default(),
                key: similarity_key(&generated.description),
                suggestions: review
                    .suggest_staged(journal, &classes, generated, rule_file)
                    .into_iter()
                    .map(WireSuggestion::from)
                    .collect(),
                ordinal,
                id,
                classification,
                overridden: staged.is_overridden(position),
            }
        })
        .collect()
}

// ===========================================================================
// Raw records (D25)
// ===========================================================================

/// What lines a proposed transaction up with a printed one: its date,
/// description and posting accounts. Amounts are left out on purpose — an
/// elided amount reads differently in the two — and the accounts are what an
/// alias rewrote identically in both.
#[derive(PartialEq, Eq)]
struct TxnKey<'a> {
    date: &'a str,
    description: &'a str,
    accounts: Vec<&'a str>,
}

impl<'a> TxnKey<'a> {
    fn of(txn: &'a Transaction) -> Self {
        Self {
            date: &txn.date,
            description: &txn.description,
            accounts: txn
                .postings
                .iter()
                .map(|posting| posting.account.0.as_str())
                .collect(),
        }
    }

    /// The same three things out of one `hledger print -O json` transaction.
    fn of_json(txn: &'a Value) -> Option<Self> {
        Some(Self {
            date: txn.get("tdate")?.as_str()?,
            description: txn.get("tdescription")?.as_str()?,
            accounts: txn
                .get("tpostings")?
                .as_array()?
                .iter()
                .map(|posting| posting.get("paccount").and_then(Value::as_str))
                .collect::<Option<Vec<_>>>()?,
        })
    }
}

/// The CSV record behind each transaction of `proposal` (D25).
///
/// `printed` is `hledger print -O json` over the same CSV with the same rules
/// file and aliases — the WHOLE file, deduped by nothing. Each printed
/// transaction's `tsourcepos` line is its 1-based position among the
/// transactions the file produced, in FILE order (verified against hledger
/// 1.52, a newest-first file included). That is record N exactly when no
/// record was dropped on the way — no `if … skip`, no `end`, no header line a
/// rules file without `skip` reads as data — which is what the count check
/// says: as many printed transactions as data records, or no alignment at
/// all.
///
/// The proposal is the printed set minus what `.latest` and the id match took
/// out, in the same order, so each proposed transaction is matched to the next
/// printed one with the same date, description and accounts. Any that is not
/// found means the two cannot be lined up, and the answer is `aligned: false`
/// rather than a guess.
///
/// `proposal` is `None` when there is none our parser could read.
/// `canonical` is the staged canonical CSV (header on line 1); `skip` is the
/// rules file's own, which the copy hledger read was padded to — so it
/// consumes exactly the header when it is 1 or more, and nothing at 0.
pub(crate) fn align_records(
    printed: Option<&Value>,
    canonical: &str,
    skip: u32,
    proposal: Option<&Journal>,
    fields: Option<Vec<String>>,
    clip: impl Fn(&[String]) -> Vec<String>,
) -> WireRecords {
    let mut records = csv_records(canonical);
    let header = records.first().map(|header| clip(header));
    if skip > 0 && !records.is_empty() {
        records.remove(0);
    }
    let unaligned = |header| WireRecords {
        aligned: false,
        header,
        fields: fields.clone(),
        rows: Vec::new(),
    };
    let (Some(printed), Some(proposal)) = (printed.and_then(Value::as_array), proposal) else {
        return unaligned(header);
    };
    if printed.len() != records.len() {
        return unaligned(header);
    }
    let Some(keyed) = printed
        .iter()
        .map(|txn| {
            let line = txn
                .get("tsourcepos")?
                .get(0)?
                .get("sourceLine")?
                .as_u64()
                .and_then(|line| usize::try_from(line).ok())
                .filter(|line| (1..=records.len()).contains(line))?;
            Some((TxnKey::of_json(txn)?, line - 1))
        })
        .collect::<Option<Vec<_>>>()
    else {
        return unaligned(header);
    };
    let mut cursor = 0;
    let mut rows = Vec::with_capacity(proposal.transactions.len());
    for txn in &proposal.transactions {
        let key = TxnKey::of(txn);
        let Some(found) = keyed[cursor..]
            .iter()
            .position(|(printed, _)| *printed == key)
        else {
            return unaligned(header);
        };
        let (_, record) = &keyed[cursor + found];
        rows.push(clip(&records[*record]));
        cursor += found + 1;
    }
    WireRecords {
        aligned: true,
        header,
        fields,
        rows,
    }
}

/// The records of `text`, an RFC 4180 CSV as [`ledgeline_core::convert::to_csv`]
/// writes it: comma separated, `"`-quoted where needed with `""` for a quote,
/// LF (or CRLF) terminated. A line with nothing on it is not a record — hledger
/// drops blank lines before it counts — and a `""` line (a one-column table's
/// empty record) is one.
fn csv_records(text: &str) -> Vec<Vec<String>> {
    let mut records = Vec::new();
    let mut record: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut touched = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    chars.next();
                    field.push('"');
                }
                '"' => quoted = false,
                other => field.push(other),
            }
            continue;
        }
        match c {
            '"' => {
                quoted = true;
                touched = true;
            }
            ',' => {
                record.push(std::mem::take(&mut field));
                touched = true;
            }
            '\r' if chars.peek() == Some(&'\n') => {}
            '\n' => {
                if touched || !field.is_empty() {
                    record.push(std::mem::take(&mut field));
                    records.push(std::mem::take(&mut record));
                }
                touched = false;
            }
            other => {
                field.push(other);
                touched = true;
            }
        }
    }
    if touched || !field.is_empty() {
        record.push(field);
        records.push(record);
    }
    records
}

// ===========================================================================
// The rule preview's rules (D25)
// ===========================================================================

/// The two rules texts a preview runs: the file with the candidate rule
/// appended, and the file with a PROBE appended — the same matchers, assigning
/// only `code` [`PROBE_CODE`] — which says which rows the matchers select
/// whatever the rule's own assignments do (a row another rule already put in
/// that account is matched, and unchanged).
///
/// Both are rendered through the rules model ([`RulesDoc::apply`] with an
/// inserted [`ItemBody::IfBlock`], then [`RulesDoc::verify`]), so a pattern or
/// value the model would not write into a rules file is refused here too.
/// Appended LAST, because hledger's last matching block wins — which is also
/// where a save puts it.
///
/// # Errors
/// `400` for a malformed rule: no matcher, no assignment, too many or too
/// long, an unknown field name, or anything the model refuses.
pub(crate) fn preview_rules(doc: &RulesDoc, rule: &WireRule) -> Result<(String, String), AppError> {
    if rule.matchers.is_empty() || rule.assignments.is_empty() {
        return Err(AppError::BadRequest(
            "a rule needs at least one matcher and one assignment".to_string(),
        ));
    }
    if rule.matchers.len() > MAX_RULE_PARTS || rule.assignments.len() > MAX_RULE_PARTS {
        return Err(AppError::BadRequest(format!(
            "a rule may have at most {MAX_RULE_PARTS} matchers and {MAX_RULE_PARTS} assignments"
        )));
    }
    let too_long = rule
        .matchers
        .iter()
        .map(|matcher| &matcher.pattern)
        .chain(rule.assignments.iter().map(|assignment| &assignment.value))
        .any(|text| text.len() > MAX_RULE_TEXT_BYTES);
    if too_long {
        return Err(AppError::BadRequest(format!(
            "a rule's patterns and values may be at most {MAX_RULE_TEXT_BYTES} bytes each"
        )));
    }
    // One OR-group per matcher: the stacked `if` hledger reads as "any of".
    let groups: Vec<MatcherGroupSpec> = rule
        .matchers
        .iter()
        .map(|matcher| MatcherGroupSpec {
            matchers: vec![MatcherSpec {
                scope: matcher
                    .field
                    .as_ref()
                    .filter(|field| !field.trim().is_empty())
                    .map_or(MatchScope::WholeRecord, |field| {
                        MatchScope::Field(field.trim().to_string())
                    }),
                pattern: matcher.pattern.clone(),
            }],
        })
        .collect();
    let assignments = rule
        .assignments
        .iter()
        .map(|assignment| {
            let field = rules::hledger_field(assignment.field.trim()).ok_or_else(|| {
                AppError::BadRequest(format!(
                    "{:?} is not an hledger CSV rules field name",
                    assignment.field
                ))
            })?;
            if matches!(field, HledgerField::Control(_)) {
                return Err(AppError::BadRequest(
                    "a previewed rule assigns fields; `skip` and `end` are not assignments"
                        .to_string(),
                ));
            }
            Ok((field, assignment.value.clone()))
        })
        .collect::<Result<Vec<_>, AppError>>()?;
    let appended = |assignments: Vec<(HledgerField, String)>| -> Result<String, AppError> {
        let mut plan = EditPlan::keep_all(doc);
        plan.order.push(Slot::Insert(ItemBody::IfBlock {
            groups: groups.clone(),
            assignments,
            control: None,
        }));
        let text = doc.apply(&plan)?;
        doc.verify(&plan, &text)?;
        Ok(text)
    };
    let candidate = appended(assignments)?;
    let probe = appended(vec![(HledgerField::Code, PROBE_CODE.to_string())])?;
    Ok((candidate, probe))
}

/// Whether `doc` `include`s another rules file. A preview runs a COPY of the
/// file from a temporary directory, where hledger would resolve a relative
/// `include` against the wrong place — so such a file is not previewed.
pub(crate) fn has_include(doc: &RulesDoc) -> bool {
    doc.items()
        .iter()
        .any(|item| matches!(item.kind, ItemKind::Include(_)))
}

/// The source text of each transaction of `journal`, parsed from `text`: its
/// `source_span` lines.
pub(crate) fn txn_texts(text: &str, journal: &Journal) -> Vec<String> {
    let lines: Vec<&str> = text.lines().collect();
    journal
        .transactions
        .iter()
        .map(|txn| {
            let start = (txn.source_span.0.line as usize).saturating_sub(1);
            let end = (txn.source_span.1.line as usize)
                .saturating_sub(1)
                .clamp(start, lines.len());
            lines.get(start..end).unwrap_or_default().join("\n")
        })
        .collect()
}

/// Compare a preview's three runs, each already parsed and filtered to the
/// rows this import would write: as the rules file stands (`before`), with
/// the candidate rule (`after`), and with the probe (`probe`).
///
/// `None` when the three do not hold the same number of transactions — the
/// rule changed which rows import, which an `if` block without `skip`/`end`
/// cannot do, so the comparison would be meaningless.
pub(crate) fn compare_runs(
    before: (&str, &Journal),
    after: (&str, &Journal),
    probe: &Journal,
    classes: &ledgeline_core::view::Classes,
) -> Option<WireRulePreviewed> {
    let total = before.1.transactions.len();
    if after.1.transactions.len() != total || probe.transactions.len() != total {
        return None;
    }
    let matched: Vec<usize> = probe
        .transactions
        .iter()
        .enumerate()
        .filter(|(_, txn)| txn.code == PROBE_CODE)
        .map(|(at, _)| at)
        .collect();
    let already_categorised = matched
        .iter()
        .filter(|&&at| !is_unknown(&before.1.transactions[at], classes))
        .count();
    let befores = txn_texts(before.0, before.1);
    let afters = txn_texts(after.0, after.1);
    let changes: Vec<usize> = (0..total).filter(|&at| befores[at] != afters[at]).collect();
    Some(WireRulePreviewed {
        ok: true,
        matched: matched.len(),
        total,
        already_categorised,
        changed_total: changes.len(),
        changed: changes
            .into_iter()
            .take(MAX_PREVIEW_CHANGES)
            .map(|ordinal| WireRuleChange {
                ordinal,
                before: befores[ordinal].clone(),
                after: afters[ordinal].clone(),
            })
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_records_reads_what_to_csv_writes() {
        let text =
            "Date,Description,Amount\n01/02/2026,\"A, B\",-1\n\n\"say \"\"hi\"\"\",x,\n\"\"\n";
        assert_eq!(
            csv_records(text),
            vec![
                vec!["Date", "Description", "Amount"],
                vec!["01/02/2026", "A, B", "-1"],
                vec!["say \"hi\"", "x", ""],
                vec![""],
            ]
        );
        assert_eq!(
            csv_records("a,b\r\nc,d"),
            vec![vec!["a", "b"], vec!["c", "d"]]
        );
    }

    fn printed(rows: &[(u64, &str, &str)]) -> Value {
        Value::Array(
            rows.iter()
                .map(|(line, date, description)| {
                    serde_json::json!({
                        "tdate": date,
                        "tdescription": description,
                        "tpostings": [{"paccount": "assets:bank"}, {"paccount": "expenses:unknown"}],
                        "tsourcepos": [{"sourceLine": line}, {"sourceLine": line}],
                    })
                })
                .collect(),
        )
    }

    fn proposal(rows: &[(&str, &str)]) -> Journal {
        let text: String = rows
            .iter()
            .map(|(date, description)| {
                format!("{date} {description}\n    assets:bank  $-1\n    expenses:unknown  $1\n\n")
            })
            .collect();
        ledgeline_core::parse_journal(&text, "proposed").expect("parses")
    }

    fn rows(records: &WireRecords) -> Vec<String> {
        records.rows.iter().map(|row| row.join("|")).collect()
    }

    #[test]
    fn records_follow_the_source_line_not_the_print_order() {
        // A newest-first file: hledger prints oldest first, and each printed
        // transaction's source line still names its own record.
        let canonical = "Date,Desc\n2026-01-03,C\n2026-01-02,B\n2026-01-01,A\n";
        let json = printed(&[
            (3, "2026-01-01", "A"),
            (2, "2026-01-02", "B"),
            (1, "2026-01-03", "C"),
        ]);
        // `.latest` kept only the last two.
        let records = align_records(
            Some(&json),
            canonical,
            1,
            Some(&proposal(&[("2026-01-02", "B"), ("2026-01-03", "C")])),
            None,
            <[String]>::to_vec,
        );
        assert!(records.aligned);
        assert_eq!(rows(&records), ["2026-01-02|B", "2026-01-03|C"]);
        assert_eq!(
            records.header,
            Some(vec!["Date".to_string(), "Desc".to_string()])
        );
    }

    #[test]
    fn a_dropped_record_or_an_unmatched_transaction_is_no_alignment() {
        let canonical = "Date,Desc\n2026-01-01,A\n2026-01-02,SKIPPED\n2026-01-03,C\n";
        // A `skip` rule ate one record: two printed for three records.
        let json = printed(&[(1, "2026-01-01", "A"), (2, "2026-01-03", "C")]);
        let records = align_records(
            Some(&json),
            canonical,
            1,
            Some(&proposal(&[("2026-01-01", "A")])),
            None,
            <[String]>::to_vec,
        );
        assert!(!records.aligned);
        assert!(records.rows.is_empty());

        // Counts agree, but a proposed transaction is nowhere in the print.
        let json = printed(&[
            (1, "2026-01-01", "A"),
            (2, "2026-01-02", "B"),
            (3, "2026-01-03", "C"),
        ]);
        let records = align_records(
            Some(&json),
            canonical,
            1,
            Some(&proposal(&[("2026-01-09", "Z")])),
            None,
            <[String]>::to_vec,
        );
        assert!(!records.aligned);

        // `skip 0` reads the header as a record too.
        let records = align_records(
            Some(&json),
            canonical,
            0,
            Some(&proposal(&[("2026-01-01", "A")])),
            None,
            <[String]>::to_vec,
        );
        assert!(!records.aligned);
    }

    #[test]
    fn a_preview_rule_is_rendered_by_the_rules_model_and_probed_by_code() {
        let doc = RulesDoc::parse(
            "skip 1\nfields date, description, amount\n\nif ACME\n    account2 income:salary\n",
        );
        let rule: WireRule = serde_json::from_value(serde_json::json!({
            "matchers": [{"field": "description", "pattern": "hardware.*store"}, {"pattern": "HOME DEPOT"}],
            "assignments": [{"field": "account2", "value": "expenses:home"}],
        }))
        .expect("decodes");
        let (candidate, probe) = preview_rules(&doc, &rule).expect("renders");
        assert!(candidate.starts_with(doc.text()), "{candidate}");
        assert!(
            candidate.contains("%description hardware.*store"),
            "{candidate}"
        );
        assert!(candidate.contains("HOME DEPOT"), "{candidate}");
        assert!(candidate.contains("account2 expenses:home"), "{candidate}");
        assert!(probe.contains(&format!("code {PROBE_CODE}")), "{probe}");
        assert!(!probe.contains("expenses:home"), "{probe}");

        let bad: WireRule = serde_json::from_value(serde_json::json!({
            "matchers": [{"pattern": "x"}],
            "assignments": [{"field": "acount2", "value": "y"}],
        }))
        .expect("decodes");
        assert!(preview_rules(&doc, &bad).is_err());
        let empty: WireRule =
            serde_json::from_value(serde_json::json!({"matchers": [], "assignments": []}))
                .expect("decodes");
        assert!(preview_rules(&doc, &empty).is_err());
    }
}
