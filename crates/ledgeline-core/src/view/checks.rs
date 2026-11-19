//! Every problem the Problems badge counts and the drawer lists (plan 26 B4):
//! the engine diagnostics (unbalanced, failed assertions, account tags, the
//! three stock rules — [`wire::journal_to_all_diagnostics`]) followed by the
//! LOCAL rules: five that used to run in the browser over the whole downloaded
//! journal (`web/src/lib/checks/rules.ts`), then transfer matching's three
//! (plan 35 D10):
//!
//! | rule                  | severity | anchor      |
//! |-----------------------|----------|-------------|
//! | `pending`             | warning  | transaction |
//! | `uncategorized`       | warning  | transaction |
//! | `negative-cash`       | warning  | account     |
//! | `missing-description` | info     | transaction |
//! | `future-date`         | info     | transaction |
//! | `in-transit`          | info     | transaction |
//! | `broken-match`        | warning  | transaction |
//! | `unusual-clearing`    | info     | transaction |
//!
//! Plan 45's six recognition rules are [`recognition_findings`], a separate
//! pass because they read the recognition file and `closed-through` as well
//! as the journal; the caller chains them after these.
//!
//! Two of them read the calendar (`negative-cash` takes balances as of today,
//! `future-date` compares against it). "Today" is the CLIENT's local date,
//! passed in, so the server's time zone never changes an answer.
//!
//! Order is the TypeScript's: engine findings first (they are authoritative,
//! and the drawer groups by first appearance), then the local rules in the
//! table's order.

use super::Classes;
use super::cash::{BalanceKind, cash_balances};
use super::wire::{js_trim, plain_decimal};
use crate::assertions::{AssertionFailure, check_balance_assertions};
use crate::edit::render_amount;
use crate::matching::{
    self, BrokenReason, DEFAULT_IN_TRANSIT_DAYS, DEFAULT_WINDOW_DAYS, MATCH_TAG,
};
use crate::model::{Journal, Status, Transaction};
use crate::projections::is_generated;
use crate::reports::AccountType;
use crate::wire::{self, WireDiagnostic};
use serde::Serialize;
use std::collections::{HashMap, HashSet};

/// How bad a finding is. Ordered, so the worst is the maximum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// Worth knowing.
    Info,
    /// Probably a mistake.
    Warning,
    /// The journal is wrong.
    Error,
}

impl Severity {
    fn from_wire(text: &str) -> Option<Self> {
        match text {
            "info" => Some(Self::Info),
            "warning" => Some(Self::Warning),
            "error" => Some(Self::Error),
            _ => None,
        }
    }
}

/// One finding, anchored to EITHER a transaction (by its position in
/// `journal.transactions`) or an account — never both, never neither.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// 0-based position in `journal.transactions`.
    pub position: Option<usize>,
    /// The account, for an account-anchored finding.
    pub account: Option<String>,
    /// The rule id.
    pub rule: &'static str,
    /// Severity.
    pub severity: Severity,
    /// Human-readable message.
    pub message: String,
}

impl Finding {
    fn txn(position: usize, rule: &'static str, severity: Severity, message: String) -> Self {
        Self {
            position: Some(position),
            account: None,
            rule,
            severity,
            message,
        }
    }
}

/// The engine diagnostics as findings, cleaned exactly as the SPA's
/// `normalizeDiagnostics` cleaned them: an entry with a blank message, an
/// out-of-range position or no anchor at all is dropped, and exact duplicates
/// (same transaction, rule and message) collapse to one.
///
/// The holdings pass behind the stock findings is the slowest of the three
/// (about 210 ms at 200k transactions) and shares nothing with the balance and
/// assertion checks, so it runs on a scoped thread of its own while they run
/// here; the result is in exactly [`wire::journal_to_all_diagnostics`]'s order.
#[must_use]
pub fn engine_findings(journal: &Journal) -> Vec<Finding> {
    engine_checks(journal).findings
}

/// The engine pass's two outputs: the findings, and the raw balance-assertion
/// failures they were made from (for the running-balance marks, which need
/// the figures and must agree with the problem list).
#[derive(Debug, Default)]
pub struct EngineChecks {
    /// [`engine_findings`].
    pub findings: Vec<Finding>,
    /// [`check_balance_assertions`]' failures, in evaluation order; empty if
    /// the check overflowed (as the findings then have none either).
    pub assertion_failures: Vec<AssertionFailure>,
}

/// [`engine_findings`], keeping the assertion failures from the same run.
#[must_use]
pub fn engine_checks(journal: &Journal) -> EngineChecks {
    let count = journal.transactions.len();
    let failures = check_balance_assertions(journal).unwrap_or_default();
    let all = std::thread::scope(|scope| {
        let stock = scope.spawn(|| wire::journal_to_stock_diagnostics(journal));
        let mut all = wire::diagnostics_with_assertions(journal, &failures);
        all.extend(wire::journal_to_auto_diagnostics(journal));
        all.extend(wire::journal_to_tag_diagnostics(journal));
        all.extend(wire::journal_to_saas_diagnostics(journal));
        // A join fails only on a panic in the pass, which would have failed
        // the request anyway; resume it here so it surfaces as one.
        all.extend(
            stock
                .join()
                .unwrap_or_else(|payload| std::panic::resume_unwind(payload)),
        );
        all
    });
    let mut seen: HashSet<(Option<usize>, &'static str, String)> = HashSet::new();
    let findings = all
        .into_iter()
        .filter_map(|diagnostic: WireDiagnostic| {
            let severity = Severity::from_wire(diagnostic.severity)?;
            if js_trim(&diagnostic.message).is_empty() {
                return None;
            }
            let (position, account) = match (diagnostic.txn_index, diagnostic.account) {
                (Some(position), _) if position < count => (Some(position), None),
                (Some(_), _) => return None,
                (None, Some(account)) if !js_trim(&account).is_empty() => (None, Some(account)),
                (None, _) => return None,
            };
            seen.insert((position, diagnostic.rule, diagnostic.message.clone()))
                .then_some(Finding {
                    position,
                    account,
                    rule: diagnostic.rule,
                    severity,
                    message: diagnostic.message,
                })
        })
        .collect();
    EngineChecks {
        findings,
        assertion_failures: failures,
    }
}

/// `*:unknown`, `*:uncategorized` (any depth, including bare), or a bare
/// top-level account whose TYPE is expense or revenue — decided by type, so a
/// chart of accounts rooted at `cogs:` or `gastos:` is caught too.
///
/// `pub(crate)` for [`crate::review`], whose "unknown" must be this exact
/// predicate so the review queue and the Problems drawer count the same rows.
pub(crate) fn is_uncategorized(account: &str, classes: &Classes) -> bool {
    let lowered = account.to_lowercase();
    let mut segments = lowered.split(':');
    let depth = segments.clone().count();
    if segments
        .next_back()
        .is_some_and(|last| last == "unknown" || last == "uncategorized")
    {
        return true;
    }
    depth == 1
        && (classes.is_type(account, AccountType::Expense)
            || classes.is_type(account, AccountType::Revenue))
}

/// The transaction-anchored local rules for one transaction, in rule order.
fn pending(position: usize, txn: &Transaction) -> Option<Finding> {
    (txn.status == Status::Pending).then(|| {
        Finding::txn(
            position,
            "pending",
            Severity::Warning,
            "transaction is marked pending (!)".to_string(),
        )
    })
}

fn uncategorized<'a>(
    position: usize,
    txn: &'a Transaction,
    verdicts: &mut HashMap<&'a str, bool>,
    classes: &Classes,
) -> Vec<Finding> {
    let mut seen: HashSet<&str> = HashSet::new();
    txn.postings
        .iter()
        .map(|posting| posting.account.0.as_str())
        .filter(|account| {
            // Memoized per account: a few hundred names behind every posting.
            *verdicts
                .entry(account)
                .or_insert_with(|| is_uncategorized(account, classes))
                && seen.insert(account)
        })
        .map(|account| {
            Finding::txn(
                position,
                "uncategorized",
                Severity::Warning,
                format!("posting to uncategorized account \"{account}\""),
            )
        })
        .collect()
}

/// A cash account whose balance is negative today, per commodity. Anchored to
/// the ACCOUNT (the running total is at fault, not one entry). Scope matches
/// the Balances view exactly — the same [`cash_balances`] pass — so a red row
/// there always has a finding here. Liabilities are never checked: a negative
/// balance is what owing money looks like.
fn negative_cash(journal: &Journal, classes: &Classes, today: &str) -> Vec<Finding> {
    cash_balances(journal, classes, today)
        .balances
        .into_iter()
        .filter(|balance| balance.kind == BalanceKind::Cash)
        .flat_map(|balance| {
            let as_of = balance.as_of;
            let account = balance.account;
            balance
                .amounts
                .into_iter()
                .filter(|(_, qty)| qty.mantissa < 0)
                .map(move |(commodity, qty)| Finding {
                    position: None,
                    account: Some(account.clone()),
                    rule: "negative-cash",
                    severity: Severity::Warning,
                    message: format!(
                        "cash balance is negative: {commodity} {} as of {as_of}",
                        plain_decimal(qty)
                    ),
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

fn missing_description(position: usize, txn: &Transaction) -> Option<Finding> {
    js_trim(&txn.description).is_empty().then(|| {
        Finding::txn(
            position,
            "missing-description",
            Severity::Info,
            "transaction has no description".to_string(),
        )
    })
}

fn future_date(position: usize, txn: &Transaction, today: &str) -> Option<Finding> {
    (txn.date.as_str() > today).then(|| {
        Finding::txn(
            position,
            "future-date",
            Severity::Info,
            format!("transaction is dated in the future ({})", txn.date),
        )
    })
}

/// Plan 35 D10 "In transit": a clearing posting older than the flag age with
/// no proposed partner. Uses the default window and flag age (D8): those are
/// per-browser settings, and this list is computed per journal and `today`,
/// not per client; the Find Matches overlay applies the client's own.
fn in_transit(journal: &Journal, today: &str) -> Vec<Finding> {
    matching::in_transit(journal, DEFAULT_WINDOW_DAYS, DEFAULT_IN_TRANSIT_DAYS, today)
        .into_iter()
        .map(|candidate| {
            Finding::txn(
                candidate.at.position,
                "in-transit",
                Severity::Info,
                format!(
                    "{} on {} has been in transit since {} with no matching transfer; \
                     Find match can link it by hand",
                    render_amount(candidate.amount),
                    candidate.account,
                    candidate.date
                ),
            )
        })
        .collect()
}

/// Plan 35 D10 "Broken match": a `match` id that is not a valid link (A3: N ≥ 2
/// clearing postings on one account, in N transactions, summing to zero). One finding per transaction carrying it.
fn broken_match(journal: &Journal) -> Vec<Finding> {
    matching::links(journal)
        .broken()
        .iter()
        .flat_map(|link| {
            let why = match link.reason {
                BrokenReason::Count(1) => "is on only one posting".to_string(),
                BrokenReason::Count(n) => format!("is on {n} postings"),
                BrokenReason::NotClearing => "is on a posting outside a clearing account".into(),
                BrokenReason::DifferentAccounts => {
                    "links postings on different clearing accounts".into()
                }
                BrokenReason::SameTransaction => "is on two postings of one transaction".into(),
                BrokenReason::DoNotCancel => "links amounts that do not sum to zero".into(),
            };
            let message = format!("{MATCH_TAG}:{} {why}", link.id);
            let positions: HashSet<usize> = link.postings.iter().map(|at| at.position).collect();
            let mut positions: Vec<usize> = positions.into_iter().collect();
            positions.sort_unstable();
            positions.into_iter().map(move |position| {
                Finding::txn(position, "broken-match", Severity::Warning, message.clone())
            })
        })
        .collect()
}

/// Plan 35 D10 "Unusual clearing posting": a cost or balance assertion on a
/// clearing posting means it can never be matched.
fn unusual_clearing(journal: &Journal) -> Vec<Finding> {
    matching::unusual_clearing_postings(journal)
        .into_iter()
        .map(|at| {
            let account = &journal.transactions[at.position].postings[at.posting]
                .account
                .0;
            Finding::txn(
                at.position,
                "unusual-clearing",
                Severity::Info,
                format!(
                    "posting to clearing account \"{account}\" has a cost or balance \
                     assertion, so it can never be matched"
                ),
            )
        })
        .collect()
}

/// The local rules over `journal`, in rule order (all `pending` findings,
/// then all `uncategorized`, …), as of the client's `today`.
///
/// A forecast transaction (one a `~` rule generated, tagged
/// `generated-transaction`) is nobody's to fix — it is re-made from its rule
/// on every read — so no transaction rule looks at it, whatever its postings
/// (an `=` rule's output on it included). A posting an `=` rule generated on a
/// REAL transaction is checked like any other (plan 32 amendment A).
#[must_use]
pub fn local_findings(journal: &Journal, classes: &Classes, today: &str) -> Vec<Finding> {
    let txns = || {
        journal
            .transactions
            .iter()
            .enumerate()
            .filter(|(_, txn)| !is_generated(txn))
    };
    let pending = txns().filter_map(|(at, txn)| pending(at, txn));
    let mut verdicts: HashMap<&str, bool> = HashMap::new();
    let uncategorized: Vec<Finding> = txns()
        .flat_map(|(at, txn)| uncategorized(at, txn, &mut verdicts, classes))
        .collect();
    let negative = negative_cash(journal, classes, today);
    let missing = txns().filter_map(|(at, txn)| missing_description(at, txn));
    let future = txns().filter_map(|(at, txn)| future_date(at, txn, today));
    pending
        .chain(uncategorized)
        .chain(negative)
        .chain(missing)
        .chain(future)
        .chain(in_transit(journal, today))
        .chain(broken_match(journal))
        .chain(unusual_clearing(journal))
        .collect()
}

/// Every finding for one journal and one `today`, indexed for the two ways
/// clients read them: the whole list (the drawer) and per transaction (a row's
/// flag dot).
#[derive(Debug, Clone, Default)]
pub struct Diagnostics {
    /// Engine findings first, then the local rules.
    pub findings: Vec<Finding>,
    /// Transaction position → indices into `findings`, in list order.
    by_position: HashMap<usize, Vec<usize>>,
}

impl Diagnostics {
    /// Combine the engine findings (computed once per journal) with the local
    /// ones (once per journal per `today`).
    #[must_use]
    pub fn new(engine: &[Finding], local: Vec<Finding>) -> Self {
        let findings: Vec<Finding> = engine.iter().cloned().chain(local).collect();
        let mut by_position: HashMap<usize, Vec<usize>> = HashMap::new();
        for (index, finding) in findings.iter().enumerate() {
            if let Some(position) = finding.position {
                by_position.entry(position).or_default().push(index);
            }
        }
        Self {
            findings,
            by_position,
        }
    }

    /// The worst severity present, or `None` when clean.
    #[must_use]
    pub fn max_severity(&self) -> Option<Severity> {
        self.findings.iter().map(|finding| finding.severity).max()
    }

    /// The findings anchored to the transaction at `position`.
    pub fn for_position(&self, position: usize) -> impl Iterator<Item = &Finding> {
        self.by_position
            .get(&position)
            .into_iter()
            .flatten()
            .map(|index| &self.findings[*index])
    }
}

/// One finding on the wire. A transaction-anchored entry carries the
/// transaction's 1-based `tindex` (what every other route addresses it by),
/// its date and description (so a list can be drawn without the journal), and
/// the legacy 0-based `txnIndex` position; an account-anchored one carries
/// `account` instead.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WireFinding<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    txn_index: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tindex: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    date: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    account: Option<&'a str>,
    rule: &'static str,
    severity: Severity,
    message: &'a str,
}

/// A row's embedded flag: the finding without its anchor (the row is the anchor).
#[derive(Debug, Clone, Serialize)]
pub struct WireFlag<'a> {
    rule: &'static str,
    severity: Severity,
    message: &'a str,
}

impl<'a> From<&'a Finding> for WireFlag<'a> {
    fn from(finding: &'a Finding) -> Self {
        Self {
            rule: finding.rule,
            severity: finding.severity,
            message: &finding.message,
        }
    }
}

/// The `/api/diagnostics` payload: `{count, maxSeverity, diagnostics}`.
/// `count` and `maxSeverity` always describe the WHOLE list, even when
/// `diagnostics` is truncated by a `limit` (the badge asks for none).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticsBody<'a> {
    count: usize,
    max_severity: Option<Severity>,
    diagnostics: Vec<WireFinding<'a>>,
}

/// Shape `diagnostics` for the wire, keeping at most `limit` entries.
#[must_use]
pub fn diagnostics_body<'a>(
    journal: &'a Journal,
    diagnostics: &'a Diagnostics,
    limit: Option<usize>,
) -> DiagnosticsBody<'a> {
    let entries = diagnostics
        .findings
        .iter()
        .take(limit.unwrap_or(usize::MAX))
        .map(|finding| {
            let txn = finding
                .position
                .and_then(|position| journal.transactions.get(position));
            WireFinding {
                txn_index: finding.position,
                tindex: txn.map(|txn| txn.index.0),
                date: txn.map(|txn| txn.date.as_str()),
                description: txn.map(|txn| txn.description.as_str()),
                account: finding.account.as_deref(),
                rule: finding.rule,
                severity: finding.severity,
                message: &finding.message,
            }
        })
        .collect();
    DiagnosticsBody {
        count: diagnostics.findings.len(),
        max_severity: diagnostics.max_severity(),
        diagnostics: entries,
    }
}

// ---------------------------------------------------------------------------
// Recognition (plan 45)
// ---------------------------------------------------------------------------

/// Plan 45's six recognition findings, in this order:
///
/// | rule                        | severity | anchor |
/// |-----------------------------|----------|--------|
/// | `recognition-file-included` | warning  | the first schedule's source transaction, else its account |
/// | `recognition-duplicate`     | warning  | each transaction carrying the `(recognition, recognized)` |
/// | `recognition-sum`           | warning  | the source transaction, or the posted entry |
/// | `recognition-orphan`        | info     | the entry or source transaction, or the rule's account |
/// | `recognition-closed-due`    | warning  | the schedule's source transaction, else its account |
/// | `recognition-rule-account`  | warning  | the undeclared account |
///
/// `included` says whether the journal includes the recognition file;
/// `closed_due` is [`crate::accrual::due::DueSet::closed`] for the client's
/// `today` and `closed-through`.
#[must_use]
pub fn recognition_findings(
    index: &crate::accrual::index::RecognitionIndex,
    journal: &Journal,
    book: &crate::accrual::book::RecognitionBook,
    closed_due: &[crate::accrual::due::DueEntry],
    included: bool,
) -> Vec<Finding> {
    use crate::accrual::index::Issue;
    let anchor = |id: &str| -> (Option<usize>, Option<String>) {
        let schedule = index.schedule(id);
        let position = schedule
            .and_then(|s| s.sources.first())
            .map(|source| source.at.position);
        let account = schedule
            .and_then(|s| s.rules.first())
            .and_then(|&rule| book.rules.get(rule))
            .and_then(|rule| rule.postings.first())
            .map(|posting| posting.account.0.clone());
        match position {
            Some(position) => (Some(position), None),
            None => (None, account),
        }
    };
    let finding = |(position, account): (Option<usize>, Option<String>),
                   rule: &'static str,
                   severity: Severity,
                   message: String| {
        (position.is_some() || account.is_some()).then_some(Finding {
            position,
            account,
            rule,
            severity,
            message,
        })
    };
    let mut out: Vec<Finding> = Vec::new();
    if included {
        let first = index
            .schedules()
            .iter()
            .map(|s| anchor(&s.id))
            .find(|(p, a)| p.is_some() || a.is_some());
        if let Some(at) = first {
            out.extend(finding(
                at,
                "recognition-file-included",
                Severity::Warning,
                "the journal includes the recognition file, so hledger and the Budget tab will \
                 treat every schedule as a budget goal; read it with -f beside the journal instead"
                    .to_string(),
            ));
        }
    }
    let issues = index.issues(journal, book);
    for issue in &issues {
        if let Issue::Duplicate {
            id,
            recognized,
            positions,
        } = issue
        {
            out.extend(positions.iter().map(|&position| {
                Finding::txn(
                    position,
                    "recognition-duplicate",
                    Severity::Warning,
                    format!(
                        "{} transactions are the recognition entry of schedule {id} for \
                         {recognized}; delete all but one",
                        positions.len()
                    ),
                )
            }));
        }
    }
    for issue in &issues {
        match issue {
            Issue::SumMismatch {
                id,
                position,
                detail,
            } => out.extend(finding(
                position.map_or_else(|| anchor(id), |p| (Some(p), None)),
                "recognition-sum",
                Severity::Warning,
                format!("schedule {id}: {detail}; review the restatement"),
            )),
            Issue::EntryMismatch {
                id,
                position,
                detail,
            } => out.push(Finding::txn(
                *position,
                "recognition-sum",
                Severity::Warning,
                format!("this recognition entry of schedule {id} doesn't match it: {detail}"),
            )),
            _ => {}
        }
    }
    for issue in &issues {
        match issue {
            Issue::OrphanEntry { id, position } => out.push(Finding::txn(
                *position,
                "recognition-orphan",
                Severity::Info,
                format!("schedule {id} has no rule in the recognition file"),
            )),
            Issue::OrphanSource { id, at } => out.push(Finding::txn(
                at.position,
                "recognition-orphan",
                Severity::Info,
                format!(
                    "this posting names schedule {id}, which has no rule in the recognition file"
                ),
            )),
            Issue::OrphanRule { id } => out.extend(finding(
                anchor(id),
                "recognition-orphan",
                Severity::Info,
                format!("schedule {id} has rules but no source posting carries recognition:{id}"),
            )),
            _ => {}
        }
    }
    out.extend(closed_due.iter().filter_map(|entry| {
        finding(
            anchor(&entry.id),
            "recognition-closed-due",
            Severity::Warning,
            format!(
                "schedule {} is due on {} in a closed period and hasn't been posted; post it as \
                 an adjustment",
                entry.id, entry.recognized
            ),
        )
    }));
    for issue in &issues {
        if let Issue::RuleAccount { id, account } = issue {
            out.push(Finding {
                position: None,
                account: Some(account.clone()),
                rule: "recognition-rule-account",
                severity: Severity::Warning,
                message: format!(
                    "schedule {id} posts to {account}, which isn't declared; strict hledger would \
                     refuse its entries"
                ),
            });
        }
    }
    out
}
