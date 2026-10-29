//! The main journal's `=` auto-posting rules (plan 30 Phase 5).
//!
//! # Whether they apply
//!
//! hledger applies `=` rules only under `--auto`, and Ledgeline's one switch
//! for that is the `hledger.conf` in force: `--auto` in its general section
//! (plan 30 Decision 7). There is no Ledgeline preference beside it, so the
//! terminal and the app can never disagree. [`auto_setting`] is the ONE place
//! that reads it — [`crate::Snapshot`] calls it whenever a snapshot is built,
//! so any republish (an edit, a reload, a config write) picks up a change and,
//! through the new snapshot's generation, refreshes every client.
//!
//! When it is on, the snapshot's `journal` is [`auto::apply_journal`]'s
//! applied copy — what every report, view, insight, holding and check reads —
//! and its `raw` is the journal as written. The [`ledgeline_core::JournalEditor`]
//! never sees a generated posting at all: it holds its own parse of the files.
//!
//! # `GET /api/auto-rules`
//!
//! What Settings › Automatic postings lists:
//!
//! ```json
//! { "inForce": true, "conf": "hledger.conf", "confHijacked": false,
//!   "rules": [ { "index": 0, "file": "main.journal", "line": 3,
//!                "query": "revenue:sales", "raw": "= revenue:sales\n    (tax)  *0.1\n",
//!                "unsupported": null, "generated": 12,
//!                "problems": [ { "kind": "unknownAccount", "message": "…",
//!                                "txnIndex": null, "account": "tax" } ] } ] }
//! ```
//!
//! `file` is relative to the journal's directory, never absolute (security
//! layer 5, as every journal-file API). `conf` is the same relative handle
//! the import screen shows (`../hledger.conf`), or `null` when no config file
//! is in force. `index` is the rule's position in the journal's rule list —
//! what a generated posting's `generated.rule` names — and stays valid for
//! one snapshot.
//!
//! For the editor (plan 30 amendment 22) each rule also carries `journalId`
//! (its file's handle, or `null` when that file cannot be written), `block`
//! (its position among that file's own rules), `revision` (the file's
//! fingerprint) and `form` (the rule as `auto_form` edits it, or `null` when
//! a form cannot state it — the client then edits the text). `editable` says
//! whether this server writes at all, and `newRule` names the file a new
//! rule is appended to — the main journal — with its revision.
//!
//! # `PUT /api/auto-rules`
//!
//! One change to one file's rules, answered with the listing as it now is:
//!
//! ```json
//! { "journalId": "rules.journal", "revision": "…",
//!   "change": { "kind": "replace", "block": 0, "was": "= revenue:sales\n…",
//!               "rule": { "form": { … } } } }
//! ```
//!
//! `kind` is `add` (with `rule`; appended to the file), `replace` (`block`,
//! `was`, `rule`) or `remove` (`block`, `was`). `rule` is `{form}` or
//! `{raw}`, never both. `was` is the block's text as the client last saw it:
//! a rule moved underneath the client is a `409` even when the revision
//! somehow still matches. A rule whose postings can never balance
//! ([`unbalanced_group`]) is a `400`: hledger would refuse the whole journal
//! under `--auto` the first time it fired.
//!
//! The write is the budget editor's pipeline (`budget_api`'s module docs):
//! the handle resolves by membership in the journal's own files, the file is
//! read pinned and its fingerprint checked, the spans come from a parse of
//! exactly those bytes, the splice touches one block ([`auto_edit`]), the
//! whole journal is re-parsed with the result and its rules must read back
//! as [`auto_edit::expected_raws`] says, and one pinned `replace` is the last
//! thing that can have an effect. A form is rendered with the forecast-only
//! tag only when it asks for it: a main-journal rule usually applies to every
//! transaction.

use std::collections::HashMap;

use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::response::Response;
use ledgeline_core::auto::{self, RuleProblem};
use ledgeline_core::auto_balance::unbalanced_group;
use ledgeline_core::auto_edit::{self, RuleEdit, RuleEditError};
use ledgeline_core::auto_form::{form_from_rule, render};
use ledgeline_core::edit::Fingerprint;
use ledgeline_core::hledger_conf;
use ledgeline_core::model::{AutoPostingRule, Journal, PostingType};
use ledgeline_core::parse::JournalStyles;
use ledgeline_core::reports::{AccountTypes, account_decls, declared_types};
use ledgeline_core::vault::{VaultError, VaultPath};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::auto_wire::{AutoRuleFormIn, WireAutoRuleForm, form_from_wire, rule_from_text};
use crate::budget_api::{quoted, read_journal, stale, unresolved, validate_journal_id};
use crate::edit_api::json_body;
use crate::error::{AppError, editing_disabled};
use crate::location::JournalLocation;
use crate::security::no_store_json as no_store;

/// The `--auto` switch as the config file in force sets it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct AutoSetting {
    /// Whether `=` rules apply to the main journal's transactions.
    pub(crate) in_force: bool,
    /// The config file's relative handle (`hledger.conf`, `../hledger.conf`),
    /// or `None` when there is none.
    pub(crate) conf: Option<String>,
    /// Whether that file replaces hledger's command word, so none of its
    /// options reach any command — `--auto` included.
    pub(crate) hijacked: bool,
}

/// Whether `--auto` is in force for the journal at `location`: the GENERAL
/// section of the `hledger.conf` hledger itself would use — the journal's
/// directory and each parent, then `~/.hledger.conf`, then the XDG file
/// (`user` supplies those two; empty in tests and on a sandboxed vault). A
/// command section's `--auto` applies to that command alone in hledger, and a
/// hijacked file reaches no command at all, so neither turns rules on here
/// ([`hledger_conf::auto_enabled`]).
pub(crate) fn auto_setting(
    location: Option<&JournalLocation>,
    user: &hledger_conf::UserConfDirs,
) -> AutoSetting {
    let Some(location) = location else {
        return AutoSetting::default();
    };
    let vault = location.vault();
    let Some(found) = hledger_conf::locate_conf_for(&**vault, &location.root_dir(), user) else {
        return AutoSetting::default();
    };
    let conf = Some(found.handle());
    let Ok(text) = found.read(&**vault) else {
        return AutoSetting {
            conf,
            ..AutoSetting::default()
        };
    };
    AutoSetting {
        in_force: hledger_conf::auto_enabled(&text),
        conf,
        hijacked: hledger_conf::hijacks_command(&text).is_some(),
    }
}

/// `GET /api/auto-rules`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireAutoRules {
    in_force: bool,
    conf: Option<String>,
    conf_hijacked: bool,
    /// Whether this server writes the journal at all.
    editable: bool,
    /// Where a new rule goes: the main journal file, or `null` when it cannot
    /// be written.
    new_rule: Option<WireFileRevision>,
    rules: Vec<WireRule>,
}

/// A writable journal file and the revision a write to it must match.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireFileRevision {
    journal_id: String,
    revision: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireRule {
    index: usize,
    file: String,
    line: u32,
    query: String,
    raw: String,
    /// Why the rule never fires, or `null`.
    unsupported: Option<String>,
    /// How many postings it generated in the journal as served (0 whenever
    /// `--auto` is off).
    generated: usize,
    problems: Vec<WireProblem>,
    /// Its file's handle, or `null` when that file cannot be written.
    journal_id: Option<String>,
    /// Its position among its own file's rules.
    block: usize,
    /// Its file's revision, or `null` with `journalId`.
    revision: Option<String>,
    /// The rule as a form, or `null` when a form cannot state it.
    form: Option<WireAutoRuleForm>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireProblem {
    /// `refused` | `unbalanced` | `unknownAccount`.
    kind: &'static str,
    message: String,
    /// The transaction's 0-based position, for the first two.
    txn_index: Option<usize>,
    /// The account, for `unknownAccount`.
    account: Option<String>,
}

/// `GET /api/auto-rules` — every `=` rule of the main journal, whether
/// `--auto` is in force, and what each rule did.
pub(crate) async fn rules(State(state): State<AppState>) -> Result<Response, AppError> {
    let Json(body) = crate::reports_api::compute(move || Ok(listing(&state))).await?;
    Ok(no_store(body))
}

/// The listing for the snapshot `state` serves now.
fn listing(state: &AppState) -> WireAutoRules {
    let snapshot = state.snapshot();
    let location = snapshot.storage();
    wire_rules(
        &snapshot.journal,
        location.as_deref(),
        &snapshot.auto,
        state.editing_enabled(),
    )
}

fn wire_rules(
    journal: &Journal,
    location: Option<&JournalLocation>,
    setting: &AutoSetting,
    editable: bool,
) -> WireAutoRules {
    let files = Files::new(journal, location);
    let types = AccountTypes::from_declared(declared_types(&account_decls(journal)));
    let mut blocks: HashMap<&std::path::Path, usize> = HashMap::new();
    let counts = auto::generated_counts(journal);
    let mut problems: Vec<Vec<WireProblem>> =
        journal.auto_rules.iter().map(|_| Vec::new()).collect();
    for finding in auto::rule_findings(journal) {
        let message = finding.message(journal);
        let problem = match finding.problem {
            RuleProblem::Refused { position, .. } => WireProblem {
                kind: "refused",
                message,
                txn_index: Some(position),
                account: None,
            },
            RuleProblem::Unbalanced { position, .. } => WireProblem {
                kind: "unbalanced",
                message,
                txn_index: Some(position),
                account: None,
            },
            RuleProblem::UnknownAccount { account } => WireProblem {
                kind: "unknownAccount",
                message,
                txn_index: None,
                account: Some(account),
            },
        };
        if let Some(slot) = problems.get_mut(finding.rule) {
            slot.push(problem);
        }
    }
    WireAutoRules {
        in_force: setting.in_force,
        conf: setting.conf.clone(),
        conf_hijacked: setting.hijacked,
        editable,
        new_rule: files.main(),
        rules: journal
            .auto_rules
            .iter()
            .zip(problems)
            .enumerate()
            .map(|(index, (rule, problems))| {
                let block = blocks.entry(rule.source_file.as_path()).or_default();
                let this = *block;
                *block += 1;
                let file = files.of(&rule.source_file);
                WireRule {
                    index,
                    file: file_id(location, &rule.source_file),
                    line: rule.source_span.0.line,
                    query: rule.query.raw().to_string(),
                    raw: rule.raw.clone(),
                    unsupported: rule.query.unsupported().map(ToString::to_string),
                    generated: counts.get(index).copied().unwrap_or(0),
                    problems,
                    journal_id: file.map(|file| file.journal_id.clone()),
                    block: this,
                    revision: file.map(|file| file.revision.clone()),
                    form: form_from_rule(rule, &types)
                        .as_ref()
                        .map(WireAutoRuleForm::from),
                }
            })
            .collect(),
    }
}

/// The journal's writable files, each with its handle and current revision,
/// read once per listing.
struct Files {
    /// `(the parse's key, the file)`, in the parse's order: the main file first.
    writable: Vec<(std::path::PathBuf, WireFileRevision)>,
}

impl Files {
    fn new(journal: &Journal, location: Option<&JournalLocation>) -> Self {
        let Some(location) = location else {
            return Self {
                writable: Vec::new(),
            };
        };
        let writable = location
            .targets(journal)
            .into_iter()
            .filter(|target| target.writable)
            .filter_map(|target| {
                let path = location.source_path(journal, &target.id)?;
                let (text, _) = read_journal(location, &path, &target.id).ok()?;
                Some((
                    location.key(&path),
                    WireFileRevision {
                        journal_id: target.id,
                        revision: Fingerprint::of_bytes(text.as_bytes()).token(),
                    },
                ))
            })
            .collect();
        Self { writable }
    }

    fn of(&self, key: &std::path::Path) -> Option<&WireFileRevision> {
        self.writable
            .iter()
            .find(|(path, _)| path == key)
            .map(|(_, file)| file)
    }

    /// The main journal file, when it is writable.
    fn main(&self) -> Option<WireFileRevision> {
        self.writable.first().map(|(_, file)| WireFileRevision {
            journal_id: file.journal_id.clone(),
            revision: file.revision.clone(),
        })
    }
}

// ===========================================================================
// Writing
// ===========================================================================

/// The `PUT` body.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SaveRule {
    journal_id: String,
    revision: String,
    change: ChangeIn,
}

/// One change to one file's rules.
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
enum ChangeIn {
    /// Append a rule to the file.
    Add { rule: RuleIn },
    /// Rewrite the file's `block`th rule, last seen as `was`.
    Replace {
        block: usize,
        was: String,
        rule: RuleIn,
    },
    /// Delete the file's `block`th rule, last seen as `was`.
    Remove { block: usize, was: String },
}

/// A rule as text or as a form — exactly one.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RuleIn {
    #[serde(default)]
    raw: Option<String>,
    #[serde(default)]
    form: Option<AutoRuleFormIn>,
}

/// `PUT /api/auto-rules` — one change to one file's `=` rules.
pub(crate) async fn save(
    State(state): State<AppState>,
    payload: Result<Json<SaveRule>, JsonRejection>,
) -> Result<Response, AppError> {
    let request = json_body(payload)?;
    // Shape first, before any filesystem call.
    validate_journal_id(&request.journal_id)?;
    if !state.editing_enabled() {
        return Err(editing_disabled());
    }
    // The journal write mutex every journal writer shares.
    let guard = state.clone();
    let _write = guard.import_writes().lock().await;
    let Json(body) = crate::reports_api::compute(move || save_rule(&state, &request)).await?;
    Ok(no_store(body))
}

/// The whole of `PUT`, synchronously. Every `?` is a decision not to write.
fn save_rule(state: &AppState, request: &SaveRule) -> Result<WireAutoRules, AppError> {
    let id = request.journal_id.as_str();
    let snapshot = state.snapshot();
    let location = snapshot.storage().ok_or_else(|| unresolved(id))?;
    let target = location
        .targets(&snapshot.journal)
        .into_iter()
        .find(|target| target.id == id)
        .ok_or_else(|| unresolved(id))?;
    if !target.writable {
        return Err(AppError::BadRequest(format!(
            "{} cannot be edited: a rule can only be written to a regular file inside the \
             journal's own directory, not a symlink or a directory",
            quoted(id)
        )));
    }
    let path = location
        .source_path(&snapshot.journal, id)
        .ok_or_else(|| unresolved(id))?;
    let key = location.key(&path);

    let (text, revision) = read_journal(&location, &path, id)?;
    if Fingerprint::of_bytes(text.as_bytes()).token() != request.revision {
        return Err(stale(id));
    }

    // The spans come from a parse of exactly the bytes just read, never from
    // the snapshot's older one.
    let current = reparse(&location, &path, &text, id)?;
    let mine = rules_of(&current, &key);
    let before: Vec<String> = mine.iter().map(|rule| rule.raw.clone()).collect();
    let types = AccountTypes::from_declared(declared_types(&account_decls(&current)));
    // A form's amounts are written as this journal reads them: `*0,1` under
    // `decimal-mark ,`, where `*0.1` would read as 100%.
    let styles = JournalStyles::of(&current);
    let edit = match &request.change {
        ChangeIn::Add { rule } => RuleEdit::Append {
            text: block_text(rule, &types, &styles)?,
        },
        ChangeIn::Replace { block, was, rule } => {
            seen(&before, *block, was, id)?;
            RuleEdit::Replace {
                block: *block,
                text: block_text(rule, &types, &styles)?,
            }
        }
        ChangeIn::Remove { block, was } => {
            seen(&before, *block, was, id)?;
            RuleEdit::Remove { block: *block }
        }
    };
    let spans = auto_edit::block_spans(&text, mine.iter().copied());
    let new_text = auto_edit::apply(&text, &spans, &edit).map_err(|_| stale(id))?;
    if new_text == text {
        // A no-op writes nothing: byte-identical content still bumps mtime.
        return Ok(listing(state));
    }

    let reparsed = reparse(&location, &path, &new_text, id)?;
    let after: Vec<String> = rules_of(&reparsed, &key)
        .iter()
        .map(|rule| rule.raw.clone())
        .collect();
    let wanted = auto_edit::expected_raws(&before, &edit).map_err(|_| stale(id))?;
    if after != wanted {
        return Err(AppError::Internal(format!(
            "the rules written to {} did not read back as the ones requested, so nothing was \
             changed",
            quoted(id)
        )));
    }

    // Pinned to the identity and bytes read above: a file changed or swapped
    // since is a `409`, not a write.
    location
        .vault()
        .replace(&path, new_text.as_bytes(), &revision)
        .map_err(|error| match error {
            VaultError::Changed { .. } => stale(id),
            // Only the `ErrorKind`: a write error can carry the temp path.
            error => AppError::Internal(format!(
                "{} could not be written: {}. Nothing else was changed.",
                quoted(id),
                error.kind()
            )),
        })?;
    // Re-open so the next edit, and the listing below, see the new journal.
    if let Some(Err(error)) = state.reopen_editor() {
        eprintln!("ledgeline: the journal could not be re-read after a rule edit: {error}");
    }
    Ok(listing(state))
}

/// The whole journal with `text` standing in for `path`.
fn reparse(
    location: &JournalLocation,
    path: &VaultPath,
    text: &str,
    id: &str,
) -> Result<Journal, AppError> {
    location
        .reparse(&HashMap::from([(path.clone(), text.to_string())]))
        .map_err(|_| {
            // No detail: a parse error names the file it was reading.
            AppError::BadRequest(format!(
                "this change would make {} unreadable as part of your journal, so nothing was \
                 written",
                quoted(id)
            ))
        })
}

/// The rules `journal` read from the file `key`, in order.
fn rules_of<'a>(journal: &'a Journal, key: &std::path::Path) -> Vec<&'a AutoPostingRule> {
    journal
        .auto_rules
        .iter()
        .filter(|rule| rule.source_file == key)
        .collect()
}

/// Require the client's `was` to be the block now at `block`.
fn seen(before: &[String], block: usize, was: &str, id: &str) -> Result<(), AppError> {
    match before.get(block) {
        Some(raw) if raw == was => Ok(()),
        _ => Err(stale(id)),
    }
}

/// A rule's block text: a form rendered by the engine, or text checked to be
/// exactly one `=` block. Either way read back through the same check, and
/// refused when its postings can never balance: hledger would refuse the
/// whole journal under `--auto` the first time it fired.
fn block_text(
    rule: &RuleIn,
    types: &AccountTypes,
    styles: &JournalStyles<'_>,
) -> Result<String, AppError> {
    let parsed = match (&rule.raw, &rule.form) {
        (Some(raw), None) => rule_from_text(raw, styles)?,
        (None, Some(form)) => {
            let text = render(&form_from_wire(form)?, types, styles).map_err(|error| {
                AppError::BadRequest(format!("an auto rule cannot be written: {error}"))
            })?;
            rule_from_text(&text, styles)?
        }
        _ => {
            return Err(AppError::BadRequest(
                "an auto rule must carry exactly one of `raw` and `form`".to_string(),
            ));
        }
    };
    if let Some(group) = unbalanced_group(&parsed) {
        return Err(AppError::BadRequest(unbalanced_message(group)));
    }
    Ok(parsed.raw)
}

/// Why a rule whose `group` cannot balance is not written, and what to do.
fn unbalanced_message(group: PostingType) -> String {
    let fix = match group {
        PostingType::BalancedVirtual => {
            "leave one of its [bracketed] postings without an amount, or give them amounts that \
             cancel"
        }
        PostingType::Regular | PostingType::Virtual => {
            "choose the account it is balanced against, or leave one posting without an amount"
        }
    };
    format!(
        "this rule's postings do not balance, and hledger refuses the whole journal under \
         --auto as soon as such a rule fires, so nothing was written: {fix}"
    )
}

impl From<RuleEditError> for AppError {
    fn from(error: RuleEditError) -> Self {
        Self::Conflict(error.to_string())
    }
}

/// `key`'s path relative to the journal's directory, or its bare name when it
/// is somehow outside it — never an absolute path.
fn file_id(location: Option<&JournalLocation>, key: &std::path::Path) -> String {
    let bare = || {
        key.file_name()
            .map_or_else(String::new, |name| name.to_string_lossy().into_owned())
    };
    let Some(location) = location else {
        return bare();
    };
    let Some(path) = location.path_of(key) else {
        return bare();
    };
    let root = location.root_dir();
    if root.is_root() {
        return path.as_str().to_string();
    }
    if path.starts_with(&root) && path != root {
        let prefix = root.as_str().len() + 1;
        return path
            .as_str()
            .get(prefix..)
            .map_or_else(bare, str::to_string);
    }
    bare()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_general_section_auto_counts() {
        assert!(hledger_conf::auto_enabled("--auto\n"));
        assert!(hledger_conf::auto_enabled(
            "# mine\n--strict --auto\n[balance]\n--tree\n"
        ));
        assert!(!hledger_conf::auto_enabled("[balance]\n--auto\n"));
        assert!(!hledger_conf::auto_enabled("# --auto\n"));
        assert!(!hledger_conf::auto_enabled("--auto-something\n"));
        assert!(!hledger_conf::auto_enabled(""));
    }
}
