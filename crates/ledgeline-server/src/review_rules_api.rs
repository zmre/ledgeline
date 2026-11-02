//! "Remember as a rule" from the Journal view (plan 32 D27): which rules file
//! a journal transaction came from, and what a candidate rule would do to
//! that file's statement. The inference is
//! [`ledgeline_core::rules::infer`]; the live check reuses the import's rule
//! preview ([`import_review::preview_rules`], [`import_api::run_previews`],
//! [`import_review::compare_runs`]).
//!
//! # Routes
//!
//! - `GET /api/review/rules-for/{index}` — `{index, needsChoice, candidates}`.
//!   No candidates: no rules file books to the transaction's account, and the
//!   overlay hides the toggle. `needsChoice`: the D27 rules leave more than
//!   one file, none is `recommended`, and the user picks.
//! - `POST /api/review/rule-preview` — `{index, rulesId, rule}`, the import
//!   preview's rule shape. Against the newest CSV the rules file reads (its
//!   `source`, else the sibling CSV), when there is one:
//!   `{ok, basis: "csv", file, matched, total, alreadyCategorised, changed,
//!   changedTotal}`, the import preview's numbers over that file. Otherwise
//!   `{ok, basis: "journal", matched, total}`: of the journal's transactions
//!   funded from the file's `account1`, how many have a description the
//!   rule's patterns match. A rules file that `include`s another, or one
//!   hledger refuses with the rule in it, is `{ok: false, stderr}`.
//!
//! Saving the rule is the rules editor's own `PUT /api/rules/{id}`, exactly as
//! the import's "Remember as a rule" does; there is no write route here.
//!
//! Neither answer derives from the journal snapshot alone — a rules file, a
//! marker or a CSV can change without the journal changing — so both are
//! `no-store` with no `ETag`, like every rules and import route.

use crate::AppState;
use crate::edit_api::json_body;
use crate::error::AppError;
use crate::import_api::{self, WireDryRunFailed};
use crate::import_review::{self, WireRule, WireRulePreviewed};
use crate::journal_api::position_of;
use crate::redact::Redactor;
use crate::reports_api::compute;
use crate::security::no_store_json as no_store;
use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::response::Response;
use ledgeline_core::Journal;
use ledgeline_core::rules::infer::{self, RulesCandidate, RulesIndex};
use ledgeline_core::rules::{self, RulesDoc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

// ---------------------------------------------------------------------------
// GET /api/review/rules-for/{index}
// ---------------------------------------------------------------------------

/// One rules file that could have produced the transaction.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireRulesCandidate {
    rules_id: String,
    label: String,
    /// The family's display name (`citi` for `citi-2026.csv.rules`).
    family: String,
    /// How many candidates are in the family.
    family_size: usize,
    /// The year the file is named for, or null.
    year: Option<u16>,
    /// The account that made it a candidate.
    account1: String,
    /// Which signals matched, in plain words.
    reasons: Vec<String>,
    /// The file to save to unless the user picks another. At most one.
    recommended: bool,
    /// `description` when the file's `fields` names it, else null (match the
    /// whole record).
    match_field: Option<String>,
    /// The file's `fields` names, or null.
    fields: Option<Vec<String>>,
}

impl From<RulesCandidate> for WireRulesCandidate {
    fn from(candidate: RulesCandidate) -> Self {
        Self {
            rules_id: candidate.id,
            label: candidate.label,
            family: candidate.family,
            family_size: candidate.family_size,
            year: candidate.year,
            account1: candidate.account1,
            reasons: candidate.reasons,
            recommended: candidate.recommended,
            match_field: candidate.match_field,
            fields: candidate.fields,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireRulesFor {
    index: u32,
    /// Candidates exist and none is recommended: the user must pick.
    needs_choice: bool,
    candidates: Vec<WireRulesCandidate>,
}

/// `GET /api/review/rules-for/{index}` — the rules files that could have
/// produced the transaction at `index`, best first (see the module docs).
pub(crate) async fn rules_for(
    State(state): State<AppState>,
    Path(index): Path<u32>,
) -> Result<Response, AppError> {
    let Json(body) = compute(move || run_rules_for(&state, index)).await?;
    Ok(no_store(body))
}

fn run_rules_for(state: &AppState, index: u32) -> Result<WireRulesFor, AppError> {
    let snapshot = state.snapshot();
    let at = position_of(&snapshot, index).ok_or_else(|| missing(index))?;
    let found = snapshot.storage().map(|location| {
        let discovery = rules::discover_in(Arc::clone(location.vault()), location.main());
        infer::rules_for(
            &snapshot.journal.transactions[at],
            &snapshot.view.classes(&snapshot.journal),
            &RulesIndex::build(&discovery),
        )
    });
    let found = found.unwrap_or_default();
    Ok(WireRulesFor {
        index,
        needs_choice: found.needs_choice,
        candidates: found
            .candidates
            .into_iter()
            .map(WireRulesCandidate::from)
            .collect(),
    })
}

fn missing(index: u32) -> AppError {
    AppError::NotFound(format!("no transaction with index {index}"))
}

// ---------------------------------------------------------------------------
// POST /api/review/rule-preview
// ---------------------------------------------------------------------------

/// `{index, rulesId, rule}`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WireJournalRulePreviewRequest {
    /// The card's transaction.
    index: u32,
    rules_id: String,
    rule: WireRule,
}

/// The live check against the rules file's newest CSV.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireCsvPreview {
    basis: &'static str,
    /// The CSV's file name — never a path.
    file: String,
    #[serde(flatten)]
    previewed: WireRulePreviewed,
}

/// The fallback, with no CSV at hand.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireJournalPreview {
    ok: bool,
    basis: &'static str,
    matched: usize,
    total: usize,
}

#[derive(Serialize)]
#[serde(untagged)]
enum WireJournalRulePreview {
    Csv(WireCsvPreview),
    Journal(WireJournalPreview),
    Failed(WireDryRunFailed),
}

/// `POST /api/review/rule-preview` — what a candidate rule would do to the
/// rules file's newest statement, or how many similar journal transactions
/// it would have matched (see the module docs). Writes nothing the user owns:
/// the rule goes into temporary copies of the rules file.
pub(crate) async fn rule_preview(
    State(state): State<AppState>,
    payload: Result<Json<WireJournalRulePreviewRequest>, JsonRejection>,
) -> Result<Response, AppError> {
    let request = json_body(payload)?;
    let Json(body) = compute(move || run_rule_preview(&state, &request)).await?;
    Ok(no_store(body))
}

fn run_rule_preview(
    state: &AppState,
    request: &WireJournalRulePreviewRequest,
) -> Result<WireJournalRulePreview, AppError> {
    let snapshot = state.snapshot();
    position_of(&snapshot, request.index).ok_or_else(|| missing(request.index))?;
    let location = snapshot
        .storage()
        .ok_or_else(|| import_api::unresolved("rules file", &request.rules_id))?;
    let discovery = rules::discover_in(Arc::clone(location.vault()), location.main());
    let found = import_api::resolve_rules(&discovery, &request.rules_id)?;
    // An unreadable file reads as empty, as the import's plan reads it: the
    // preview then fails on hledger's terms, or matches nothing.
    let doc = RulesDoc::parse(&found.read_text().unwrap_or_default());
    if import_review::has_include(&doc) {
        return Ok(WireJournalRulePreview::Failed(import_api::include_refusal(
            &request.rules_id,
        )));
    }
    // Rendered (and so validated) before deciding the basis, so a malformed
    // rule is the same 400 whether or not a CSV is at hand.
    let (candidate, probe) = import_review::preview_rules(&doc, &request.rule)?;
    let csv = discovery.data_file(&request.rules_id).and_then(|path| {
        Some((
            path.file_name()?.to_string(),
            location.vault().local_path(&path)?,
        ))
    });
    let classes = snapshot.view.classes(&snapshot.journal);
    match (csv, found.local_path()) {
        (Some((file, data)), Some(rules)) => csv_preview(
            state,
            &classes,
            &location,
            CsvRun {
                rules_id: &request.rules_id,
                rules: &rules,
                data: &data,
                file,
                candidate: &candidate,
                probe: &probe,
            },
        ),
        _ => Ok(journal_preview(
            &snapshot.journal,
            &classes,
            &doc,
            &request.rule,
        )),
    }
}

/// What one CSV-basis preview runs over.
struct CsvRun<'a> {
    rules_id: &'a str,
    /// The rules file as it is.
    rules: &'a std::path::Path,
    /// The statement it reads.
    data: &'a std::path::Path,
    /// The statement's file name.
    file: String,
    /// The rules text with the candidate rule appended, and with the probe.
    candidate: &'a str,
    probe: &'a str,
}

/// The CSV basis: the import preview's three runs, as `hledger print` over the
/// statement on disk (no `.latest`: these rows were imported already, and
/// every one of them is what the rule would meet next time), compared the way
/// the import compares them.
fn csv_preview(
    state: &AppState,
    classes: &ledgeline_core::view::Classes,
    location: &crate::location::JournalLocation,
    run: CsvRun<'_>,
) -> Result<WireJournalRulePreview, AppError> {
    let hledger = import_api::resolve_hledger(state.app_dirs())?;
    let aliases = import_api::csv_aliases(state, location);
    let failed = |error: std::io::Error| {
        AppError::Internal(format!("a rule could not be tried: {}", error.kind()))
    };
    let scratch = state.stages().scratch().map_err(failed)?;
    let candidate_path = scratch.path().join("candidate.rules");
    let probe_path = scratch.path().join("probe.rules");
    for (path, text) in [(&candidate_path, run.candidate), (&probe_path, run.probe)] {
        std::fs::write(path, text).map_err(failed)?;
    }
    let mut redactor = Redactor::default()
        .hide(hledger.path(), "hledger")
        .hide(run.rules, run.rules_id)
        .hide(&candidate_path, run.rules_id)
        .hide(&probe_path, run.rules_id)
        .hide(run.data, &run.file)
        .hide_prefix(&state.app_dirs().temp_dir);
    if let Ok(root) = location.local_path(&location.root_dir(), "trying a rule") {
        redactor = redactor.hide_prefix(&root);
    }
    let outputs = import_api::run_previews(
        [run.rules, &candidate_path, &probe_path],
        |rules| import_api::print_invocation(&hledger, run.data, rules, &aliases),
        &redactor,
    )?;
    let [before, after, probe] = match outputs {
        Ok(outputs) => outputs,
        Err(failed) => return Ok(WireJournalRulePreview::Failed(failed)),
    };
    let parse = |text: &str| {
        ledgeline_core::parse_journal(text, "previewed").map_err(|_| {
            AppError::Internal("a rule preview's output could not be read back".to_string())
        })
    };
    let (parsed_before, parsed_after, parsed_probe) =
        (parse(&before)?, parse(&after)?, parse(&probe)?);
    Ok(import_review::compare_runs(
        (&before, &parsed_before),
        (&after, &parsed_after),
        &parsed_probe,
        classes,
    )
    .map_or_else(
        || {
            WireJournalRulePreview::Failed(WireDryRunFailed {
                ok: false,
                stderr: "that rule changes which rows the statement holds, so it cannot be \
                         previewed row by row"
                    .to_string(),
            })
        },
        |previewed| {
            WireJournalRulePreview::Csv(WireCsvPreview {
                basis: "csv",
                file: run.file,
                previewed,
            })
        },
    ))
}

/// The journal basis (D27's fallback): the journal's real transactions funded
/// from the file's `account1`, and how many of them have a description one of
/// the rule's patterns matches, read as hledger reads a matcher. A file with
/// no single `account1` considers nothing.
fn journal_preview(
    journal: &Journal,
    classes: &ledgeline_core::view::Classes,
    doc: &RulesDoc,
    rule: &WireRule,
) -> WireJournalRulePreview {
    let Some(account1) = infer::effective_account1(doc) else {
        return WireJournalRulePreview::Journal(WireJournalPreview {
            ok: true,
            basis: "journal",
            matched: 0,
            total: 0,
        });
    };
    match infer::journal_matches(journal, classes, &account1, &rule.patterns()) {
        Ok(found) => WireJournalRulePreview::Journal(WireJournalPreview {
            ok: true,
            basis: "journal",
            matched: found.matched,
            total: found.total,
        }),
        Err(error) => WireJournalRulePreview::Failed(WireDryRunFailed {
            ok: false,
            stderr: format!(
                "that pattern is not a regular expression this check can read: {error}"
            ),
        }),
    }
}
