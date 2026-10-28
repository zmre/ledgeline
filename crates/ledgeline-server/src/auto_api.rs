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

use axum::Json;
use axum::extract::State;
use ledgeline_core::auto::{self, RuleProblem};
use ledgeline_core::hledger_conf;
use ledgeline_core::model::Journal;
use serde::Serialize;

use crate::AppState;
use crate::error::AppError;
use crate::location::JournalLocation;

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
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireAutoRules {
    in_force: bool,
    conf: Option<String>,
    conf_hijacked: bool,
    rules: Vec<WireRule>,
}

#[derive(Debug, Serialize)]
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
}

#[derive(Debug, Serialize)]
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
pub(crate) async fn rules(State(state): State<AppState>) -> Result<Json<WireAutoRules>, AppError> {
    let snapshot = state.snapshot();
    crate::reports_api::compute(move || {
        let location = snapshot.storage();
        Ok(wire_rules(
            &snapshot.journal,
            location.as_deref(),
            &snapshot.auto,
        ))
    })
    .await
}

fn wire_rules(
    journal: &Journal,
    location: Option<&JournalLocation>,
    setting: &AutoSetting,
) -> WireAutoRules {
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
        rules: journal
            .auto_rules
            .iter()
            .zip(problems)
            .enumerate()
            .map(|(index, (rule, problems))| WireRule {
                index,
                file: file_id(location, &rule.source_file),
                line: rule.source_span.0.line,
                query: rule.query.raw().to_string(),
                raw: rule.raw.clone(),
                unsupported: rule.query.unsupported().map(ToString::to_string),
                generated: counts.get(index).copied().unwrap_or(0),
                problems,
            })
            .collect(),
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
