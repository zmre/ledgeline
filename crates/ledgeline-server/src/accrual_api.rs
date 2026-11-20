//! Plan 45 Phase 2: the accrual routes. Routing, caching and transport only —
//! every computation is `ledgeline_core::accrual`, and every write goes
//! through [`ledgeline_core::accrual::ops`], the same calls an invoicing app
//! makes (D-lib).
//!
//! | Route | Does |
//! |---|---|
//! | `GET /api/accrual` | the gate, roles, file, `closed-through`, counts |
//! | `GET /api/recognition/due` | months → groups → entries, ties, the closed section |
//! | `POST /api/recognition/post` | [`ops::Loaded::post_due`] |
//! | `GET /api/recognition/for/{index}` | one source transaction's schedules, renewal prefill |
//! | `POST /api/recognition/preview` | [`ops::Loaded::preview`] |
//! | `POST /api/recognition/save` | [`ops::Loaded::save`] |
//! | `POST /api/recognition/file` | [`ops::Loaded::create_recognition_file`] |
//! | `POST /api/books/closed-through` | [`ops::Books::set_closed_through`] |
//! | `GET /api/settlements` | proposals and open invoices/payments |
//! | `GET /api/settlements/for/{index}` | one transaction's settlement candidates |
//! | `POST /api/settlements/link`, `/link-all` | [`ops::Loaded::link_settlement`] |
//!
//! With the gate off (D-gate) every GET answers `{"enabled": false}` and every
//! write a `409`.
//!
//! # Writes
//!
//! A write holds the editor lock (so it serializes with every other edit),
//! loads the books afresh from the same vault through `ops`, lets `ops` read,
//! verify and write, and then re-syncs the bound editor from disk and
//! republishes — whatever `ops` managed to write is served, even a partial
//! `RulesNotWritten`.

use crate::edit_api::{
    self, AddRequest, bound, json_body, lock_editor, redacted, resync_from_disk,
};
use crate::error::AppError;
use crate::journal_api::{dated_etag, not_modified, ok_json, to_bytes};
use crate::reports_api::{compute, parse_date};
use crate::{AppState, Snapshot, generation_of, if_none_match_hits};
use axum::Json;
use axum::body::Bytes;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::response::Response;
use ledgeline_core::Dec;
use ledgeline_core::accrual::book::{BookCache, RecognitionBook, is_included, recognition_path};
use ledgeline_core::accrual::books_config::{self, BooksConfig};
use ledgeline_core::accrual::due::{DueEntry, DueSet, due, ties};
use ledgeline_core::accrual::index::RecognitionIndex;
use ledgeline_core::accrual::invoice::money;
use ledgeline_core::accrual::ops::{
    Books, Generation, Loaded, OpsError, PostDue, SavePlan, SaveRequest, SettleLink, SourceChange,
};
use ledgeline_core::accrual::restate::{Restatement, RowAction};
use ledgeline_core::accrual::schedule::{
    Party, PartyKind, Period, PeriodPart, Proration, RuleRole, Schedule, ScheduleLine,
    ScheduleSpec, Term, build,
};
use ledgeline_core::accrual::settle::{
    self, Fit, SettleCandidate, SettleProposal, SettleSettings, Side,
};
use ledgeline_core::accrual::{
    AccrualGate, AccrualRoles, Basis, Contract, GateReason, RoleSource, accrual_gate,
};
use ledgeline_core::edit::{Fingerprint, JournalEditor};
use ledgeline_core::matching::PostingRef;
use ledgeline_core::model::{Amount, Journal, Posting, Tindex, Transaction};
use ledgeline_core::parse::JournalStyles;
use ledgeline_core::vault::VaultPath;
use ledgeline_core::view::wire::WireDec;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

/// How many days back "Needs a schedule" looks by default (Risks: adoption
/// noise).
const NEEDS_SCHEDULE_LOOKBACK_DAYS: u32 = 90;

// ===========================================================================
// The per-snapshot view
// ===========================================================================

/// The recognition file as this snapshot found it.
#[derive(Debug, Clone)]
pub(crate) struct FileInfo {
    /// Its place in the vault.
    pub(crate) path: VaultPath,
    /// Its key: what the parser records, and what the watcher watches.
    pub(crate) key: PathBuf,
    /// Whether it exists.
    pub(crate) exists: bool,
    /// Whether the journal `include`s it (the `recognition-file-included`
    /// finding).
    pub(crate) included: bool,
    /// The fingerprint of the bytes read, when it exists.
    pub(crate) revision: Option<Fingerprint>,
}

/// Everything accrual a snapshot knows, built on first use and dropped with
/// the snapshot: `[books]`, the accounts' roles, the gate, the recognition
/// file, and (lazily again) the index and the journal's recognition rows.
#[derive(Debug)]
pub(crate) struct AccrualView {
    pub(crate) config: BooksConfig,
    pub(crate) roles: AccrualRoles,
    pub(crate) gate: AccrualGate,
    pub(crate) file: Option<FileInfo>,
    /// The parsed recognition file; `None` when it doesn't exist or doesn't
    /// read (then `problem` says why).
    pub(crate) recognition: Option<Arc<RecognitionBook>>,
    /// Why the books or the file couldn't be read, if they couldn't.
    pub(crate) problem: Option<String>,
    index: OnceLock<Arc<RecognitionIndex>>,
    rows: OnceLock<Arc<ledgeline_core::view::rows::RecognitionRows>>,
    empty: OnceLock<Arc<RecognitionBook>>,
}

impl AccrualView {
    /// Read `[books]` and the recognition file beside `journal` (through
    /// `storage`, the snapshot's vault), parsing the file through `cache`.
    pub(crate) fn build(
        journal: &Journal,
        storage: Option<&crate::location::JournalLocation>,
        cache: &BookCache,
    ) -> Self {
        let roles = AccrualRoles::of(journal);
        let mut problem = None;
        let location = storage;
        let config = match location {
            Some(location) => books_config::read(&**location.vault(), location.main())
                .unwrap_or_else(|error| {
                    problem = Some(error.to_string());
                    BooksConfig::default()
                }),
            None => BooksConfig::default(),
        };
        let gate = accrual_gate(&roles, config.basis);
        let mut recognition = None;
        let file = location.and_then(|location| {
            let path = match recognition_path(location.main(), config.recognition_file.as_deref()) {
                Ok(path) => path,
                Err(error) => {
                    problem = Some(error.to_string());
                    return None;
                }
            };
            let key = location.vault().key(&path);
            let included = is_included(journal, &key);
            let read = location.vault().read_pinned(&path, Some(256 * 1024 * 1024));
            let (exists, revision) = match read {
                Ok((bytes, _)) => match String::from_utf8(bytes) {
                    Ok(text) => {
                        match cache.get(&text, &key, &journal.commodity_styles) {
                            Ok(book) => recognition = Some(book),
                            Err(error) => problem = Some(error.to_string()),
                        }
                        (true, Some(Fingerprint::of_bytes(text.as_bytes())))
                    }
                    Err(_) => {
                        problem = Some("the recognition file is not UTF-8".to_string());
                        (true, None)
                    }
                },
                Err(error) if error.is_not_found() => (false, None),
                Err(error) => {
                    problem = Some(format!("the recognition file could not be read: {error}"));
                    (true, None)
                }
            };
            Some(FileInfo {
                path,
                key,
                exists,
                included,
                revision,
            })
        });
        Self {
            config,
            roles,
            gate,
            file,
            recognition,
            problem,
            index: OnceLock::new(),
            rows: OnceLock::new(),
            empty: OnceLock::new(),
        }
    }

    /// The recognition book, empty when there is none.
    pub(crate) fn book(&self) -> Arc<RecognitionBook> {
        self.recognition.clone().unwrap_or_else(|| {
            Arc::clone(self.empty.get_or_init(|| {
                Arc::new(RecognitionBook::missing(
                    &self
                        .file
                        .as_ref()
                        .map(|file| file.key.clone())
                        .unwrap_or_default(),
                ))
            }))
        })
    }

    /// The recognition index over `journal` (the snapshot's raw journal).
    pub(crate) fn index(&self, journal: &Journal) -> Arc<RecognitionIndex> {
        Arc::clone(self.index.get_or_init(|| {
            Arc::new(RecognitionIndex::build_with(
                journal,
                &self.book(),
                self.roles.clone(),
            ))
        }))
    }

    /// The journal's posted recognition entries, for row grouping.
    pub(crate) fn rows(
        &self,
        journal: &Journal,
    ) -> Arc<ledgeline_core::view::rows::RecognitionRows> {
        Arc::clone(
            self.rows
                .get_or_init(|| Arc::new(ledgeline_core::view::rows::RecognitionRows::of(journal))),
        )
    }

    /// The due set for `today`, under `closed-through`.
    pub(crate) fn due(&self, journal: &Journal, today: &str) -> DueSet {
        due(
            &self.index(journal),
            &self.book(),
            today,
            self.config.closed_through.as_deref(),
        )
    }
}

/// The accrual view of `snapshot`, built on first use (it lives in the
/// snapshot's view cache, beside the problem list that reads it).
pub(crate) fn view(_state: &AppState, snapshot: &Snapshot) -> Arc<AccrualView> {
    snapshot.view.accrual(&snapshot.journal)
}

/// Plan 45's six Problems findings for `today`, when the helpers are on.
pub(crate) fn findings(
    accrual: &AccrualView,
    journal: &Journal,
    today: &str,
) -> Vec<ledgeline_core::view::checks::Finding> {
    if !accrual.gate.enabled {
        return Vec::new();
    }
    let index = accrual.index(journal);
    let book = accrual.book();
    let set = accrual.due(journal, today);
    ledgeline_core::view::checks::recognition_findings(
        &index,
        journal,
        &book,
        &set.closed,
        accrual.file.as_ref().is_some_and(|file| file.included),
    )
}

/// Whether the recognition file or `[books]` changed on disk since the
/// published snapshot read them — what a refresh with an unchanged journal
/// still has to republish for.
pub(crate) fn changed_on_disk(state: &AppState) -> bool {
    let snapshot = state.snapshot();
    let Some(seen) = snapshot.view.accrual_if_built() else {
        // Never read, so nothing stale is being served.
        return false;
    };
    let fresh = AccrualView::build(
        &snapshot.journal,
        snapshot.view.storage(),
        snapshot.view.book_cache(),
    );
    let revision = |view: &AccrualView| view.file.as_ref().and_then(|f| f.revision.clone());
    let exists = |view: &AccrualView| view.file.as_ref().is_some_and(|f| f.exists);
    revision(&seen) != revision(&fresh)
        || exists(&seen) != exists(&fresh)
        || seen.config != fresh.config
}

/// The recognition file's key for `snapshot`, to watch and to keep out of
/// projection discovery.
pub(crate) fn recognition_key(state: &AppState, snapshot: &Snapshot) -> Option<PathBuf> {
    view(state, snapshot)
        .file
        .as_ref()
        .map(|file| file.key.clone())
}

/// What projection discovery leaves out (D-guards): the recognition file.
pub(crate) fn excluded_from_projections(state: &AppState, snapshot: &Snapshot) -> Vec<PathBuf> {
    recognition_key(state, snapshot).into_iter().collect()
}

/// Today for an accrual request: the client's date may LOWER the server's
/// local date, never raise it (D6).
pub(crate) fn effective_today(client: Option<String>) -> Result<String, AppError> {
    let server = server_today();
    match parse_date("today", client, || server.clone())? {
        client if client.as_str() < server.as_str() => Ok(client),
        _ => Ok(server),
    }
}

/// The server's local date.
pub(crate) fn server_today() -> String {
    let now = i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs()),
    )
    .unwrap_or(0);
    crate::sync::auto::local_day(now, crate::sync::local_offset(now))
}

// ===========================================================================
// Wire shapes
// ===========================================================================

#[derive(Serialize)]
struct Disabled {
    enabled: bool,
}

fn disabled_body() -> Result<Bytes, AppError> {
    to_bytes(&Disabled { enabled: false })
}

fn refuse_disabled() -> AppError {
    AppError::Conflict(
        "the accrual helpers are off for this ledger: no account is a receivable, payable, \
         deferred-revenue or prepaid account, or [books] basis is \"cash\""
            .to_string(),
    )
}

/// `{commodity, quantity}`.
#[derive(Serialize, Clone)]
pub(crate) struct WireMoney {
    commodity: String,
    quantity: WireDec,
}

impl WireMoney {
    fn of(commodity: &str, quantity: Dec) -> Self {
        Self {
            commodity: commodity.to_string(),
            quantity: quantity.into(),
        }
    }

    fn amount(amount: &Amount) -> Self {
        Self::of(&amount.commodity.0, amount.quantity)
    }
}

/// `{kind: "customer"|"vendor", name}`.
#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireParty {
    kind: String,
    name: String,
}

impl WireParty {
    fn of(party: &Party) -> Self {
        Self {
            kind: match party.kind {
                PartyKind::Customer => "customer",
                PartyKind::Vendor => "vendor",
            }
            .to_string(),
            name: party.name.clone(),
        }
    }

    fn to_core(&self) -> Result<Party, AppError> {
        let kind = match self.kind.as_str() {
            "customer" => PartyKind::Customer,
            "vendor" => PartyKind::Vendor,
            other => {
                return Err(AppError::BadRequest(format!(
                    "a party is a customer or a vendor, not {other:?}"
                )));
            }
        };
        Ok(Party {
            kind,
            name: self.name.clone(),
        })
    }
}

/// `{account, amount}`.
#[derive(Serialize)]
struct WirePosting {
    account: String,
    amounts: Vec<WireMoney>,
}

impl WirePosting {
    fn of(posting: &Posting) -> Self {
        Self {
            account: posting.account.0.clone(),
            amounts: posting.amounts.iter().map(WireMoney::amount).collect(),
        }
    }
}

/// A schedule line, both ways: `{sourceAccount, targetAccount, total:
/// {commodity, quantity: "-1200.00"}, party}`. The quantity is decimal text
/// on the way in, the wire decimal on the way out.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireLine {
    source_account: String,
    target_account: String,
    total: WireMoney,
    #[serde(skip_serializing_if = "Option::is_none")]
    party: Option<WireParty>,
}

/// `{months: N}` or `{through: "YYYY-MM-DD"}`.
#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct WireTerm {
    #[serde(skip_serializing_if = "Option::is_none")]
    months: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    through: Option<String>,
}

impl WireTerm {
    fn of(term: &Term) -> Self {
        match term {
            Term::Months(months) => Self {
                months: Some(*months),
                through: None,
            },
            Term::Through(date) => Self {
                months: None,
                through: Some(date.clone()),
            },
        }
    }

    fn to_core(&self) -> Result<Term, AppError> {
        match (self.months, &self.through) {
            (Some(months), None) => Ok(Term::Months(months)),
            (None, Some(through)) => Ok(Term::Through(through.clone())),
            _ => Err(AppError::BadRequest(
                "a term is {months} or {through}, exactly one".to_string(),
            )),
        }
    }
}

fn proration_code(proration: Proration) -> &'static str {
    match proration {
        Proration::Anniversary => "anniversary",
        Proration::Calendar => "calendar",
    }
}

/// A schedule spec on the way out.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireSpec {
    id: String,
    lines: Vec<WireLine>,
    start: String,
    term: WireTerm,
    proration: &'static str,
    description: String,
    header_tags: Vec<(String, String)>,
}

impl WireSpec {
    fn of(spec: &ScheduleSpec) -> Self {
        Self {
            id: spec.id.clone(),
            lines: spec
                .lines
                .iter()
                .map(|line| WireLine {
                    source_account: line.source_account.clone(),
                    target_account: line.target_account.clone(),
                    total: WireMoney::amount(&line.total),
                    party: line.party.as_ref().map(WireParty::of),
                })
                .collect(),
            start: spec.start.clone(),
            term: WireTerm::of(&spec.term),
            proration: proration_code(spec.proration),
            description: spec.description.clone(),
            header_tags: spec.header_tags.clone(),
        }
    }
}

/// A schedule line on the way in: the total as decimal text.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LineIn {
    source_account: String,
    target_account: String,
    commodity: String,
    /// Signed as the source posting writes it: negative for deferred revenue.
    quantity: String,
    #[serde(default)]
    party: Option<WireParty>,
}

/// A schedule spec on the way in. `id` is optional for a new schedule: the
/// server draws a fresh one ([`Loaded::id_generator`]).
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SpecIn {
    #[serde(default)]
    id: Option<String>,
    lines: Vec<LineIn>,
    start: String,
    term: WireTerm,
    #[serde(default)]
    proration: Option<String>,
    description: String,
    #[serde(default)]
    header_tags: Vec<(String, String)>,
}

impl SpecIn {
    fn to_core(
        &self,
        journal: &Journal,
        fresh_id: &mut dyn FnMut() -> String,
    ) -> Result<ScheduleSpec, AppError> {
        let proration = match self.proration.as_deref() {
            None | Some("anniversary") => Proration::Anniversary,
            Some("calendar") => Proration::Calendar,
            Some(other) => {
                return Err(AppError::BadRequest(format!(
                    "proration is anniversary or calendar, not {other:?}"
                )));
            }
        };
        let lines = self
            .lines
            .iter()
            .map(|line| {
                let quantity = Dec::parse(&line.quantity, '.').map_err(|_| {
                    AppError::BadRequest(format!("{:?} is not an amount", line.quantity))
                })?;
                Ok(ScheduleLine {
                    source_account: line.source_account.clone(),
                    target_account: line.target_account.clone(),
                    total: money(journal, &line.commodity, quantity),
                    party: line.party.as_ref().map(WireParty::to_core).transpose()?,
                })
            })
            .collect::<Result<Vec<_>, AppError>>()?;
        Ok(ScheduleSpec {
            id: self.id.clone().unwrap_or_else(fresh_id),
            lines,
            start: self.start.clone(),
            term: self.term.to_core()?,
            proration,
            description: self.description.clone(),
            header_tags: self.header_tags.clone(),
        })
    }
}

fn part_code(part: PeriodPart) -> &'static str {
    match part {
        PeriodPart::Opening => "opening",
        PeriodPart::Regular => "regular",
        PeriodPart::Closing => "closing",
    }
}

fn role_code(role: RuleRole) -> &'static str {
    match role {
        RuleRole::Opening => "opening",
        RuleRole::Main => "main",
        RuleRole::Final => "final",
    }
}

/// A built schedule: its periods and its rules.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireSchedule {
    spec: WireSpec,
    periods: Vec<WirePeriod>,
    rules: Vec<WireRule>,
    renewal_date: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WirePeriod {
    date: String,
    part: &'static str,
    /// Per line, signed like the line's total.
    amounts: Vec<WireDec>,
    /// `posted`, `due`, `closed` (due in a closed period) or `future`; absent
    /// in a preview.
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<&'static str>,
    /// The posted entry's `tindex`, when posted.
    #[serde(skip_serializing_if = "Option::is_none")]
    entry: Option<u32>,
}

#[derive(Serialize)]
struct WireRule {
    role: &'static str,
    period: String,
    text: String,
}

impl WireSchedule {
    fn of(schedule: &Schedule) -> Self {
        Self {
            spec: WireSpec::of(&schedule.spec),
            periods: schedule
                .periods
                .iter()
                .map(|p| WirePeriod::of(p, None, None))
                .collect(),
            rules: schedule
                .rules
                .iter()
                .map(|rule| WireRule {
                    role: role_code(rule.role),
                    period: rule.period.clone(),
                    text: rule.text.clone(),
                })
                .collect(),
            renewal_date: schedule.renewal_date.clone(),
        }
    }
}

impl WirePeriod {
    fn of(period: &Period, status: Option<&'static str>, entry: Option<u32>) -> Self {
        Self {
            date: period.date.clone(),
            part: part_code(period.part),
            amounts: period.amounts.iter().map(|a| WireDec::from(*a)).collect(),
            status,
            entry,
        }
    }
}

fn action_code(action: RowAction) -> &'static str {
    match action {
        RowAction::Kept => "kept",
        RowAction::Unchanged => "unchanged",
        RowAction::Rewrite => "rewrite",
        RowAction::Delete => "delete",
    }
}

/// A restatement preview (D4).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireRestatement {
    id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    rule_change: Option<WireRuleChange>,
    rows: Vec<WireRestatementRow>,
    deletes: Vec<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    adjustment: Option<WireEntry>,
    total_after: Vec<WireDec>,
}

#[derive(Serialize)]
struct WireRuleChange {
    before: String,
    after: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireRestatementRow {
    index: u32,
    date: String,
    recognized: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    recognized_after: Option<String>,
    closed: bool,
    before: Vec<WireDec>,
    #[serde(skip_serializing_if = "Option::is_none")]
    after: Option<Vec<WireDec>>,
    action: &'static str,
}

/// A transaction a write will add (an entry or an adjustment).
#[derive(Serialize)]
struct WireEntry {
    date: String,
    description: String,
    tags: Vec<(String, String)>,
    postings: Vec<WirePosting>,
}

impl WireEntry {
    fn of(txn: &Transaction) -> Self {
        Self {
            date: txn.date.clone(),
            description: txn.description.clone(),
            tags: txn.tags.clone(),
            postings: txn.postings.iter().map(WirePosting::of).collect(),
        }
    }
}

impl WireRestatement {
    fn of(restatement: &Restatement) -> Self {
        let decs = |values: &[Dec]| values.iter().map(|v| WireDec::from(*v)).collect();
        Self {
            id: restatement.id.clone(),
            rule_change: restatement
                .rule_change
                .as_ref()
                .map(|change| WireRuleChange {
                    before: change.before.clone(),
                    after: change.after.clone(),
                }),
            rows: restatement
                .rows
                .iter()
                .map(|row| WireRestatementRow {
                    index: row.tindex.0,
                    date: row.date.clone(),
                    recognized: row.recognized.clone(),
                    recognized_after: row.recognized_after.clone(),
                    closed: row.closed,
                    before: decs(&row.before),
                    after: row.after.as_deref().map(decs),
                    action: action_code(row.action),
                })
                .collect(),
            deletes: restatement.deletes.iter().map(|t| t.0).collect(),
            adjustment: restatement.adjustment.as_ref().map(WireEntry::of),
            total_after: decs(&restatement.total_after),
        }
    }
}

/// A save's (or preview's) plan.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WirePlan<'a> {
    enabled: bool,
    /// The accrual books' generation the plan was made against: hand it back
    /// to `save` to refuse a stale preview.
    accrual_generation: &'a str,
    appended: Vec<WireSchedule>,
    restatements: Vec<WireRestatement>,
    creates_file: bool,
    /// The snapshot generation after a save; absent from a preview.
    #[serde(skip_serializing_if = "Option::is_none")]
    generation: Option<String>,
}

impl<'a> WirePlan<'a> {
    fn of(plan: &'a SavePlan, generation: Option<String>) -> Self {
        Self {
            enabled: true,
            accrual_generation: plan.generation.token(),
            appended: plan.appended.iter().map(WireSchedule::of).collect(),
            restatements: plan.restatements.iter().map(WireRestatement::of).collect(),
            creates_file: plan.creates_file,
            generation,
        }
    }
}

// ===========================================================================
// Errors and the write path
// ===========================================================================

/// An [`OpsError`] as HTTP.
fn ops_error(error: OpsError) -> AppError {
    let message = error.to_string();
    match error {
        OpsError::ExternalChange
        | OpsError::Stale
        | OpsError::FileExists(_)
        | OpsError::DuplicateDocument(_) => AppError::Conflict(message),
        OpsError::DocumentNotFound(_) | OpsError::Refused(_) => AppError::BadRequest(message),
        OpsError::FutureDate { .. }
        | OpsError::BadDate(_)
        | OpsError::FileMissing(_)
        | OpsError::Schedule(_)
        | OpsError::Restate(_)
        | OpsError::Settle(_)
        | OpsError::Document(_)
        | OpsError::Collision { .. } => AppError::BadRequest(message),
        OpsError::Edit(edit) => AppError::from(edit),
        OpsError::Book(_) | OpsError::Books(_) => AppError::Conflict(message),
        OpsError::RulesNotWritten(_) | OpsError::Storage(_) => AppError::Internal(message),
    }
}

/// Run one `ops` write under the editor lock, on a copy of the bound editor
/// ([`Books::load_from`]), and publish what landed. `generation`, when given,
/// must still be the served snapshot's.
///
/// - Success: the copy's save landed, so the copy IS the files — adopt it as
///   the bound editor and publish its journal. No second read of the journal
///   (this used to `load` a fresh editor and then re-open the bound one from
///   disk: two full parses, ~1.7 s of a Link at 200k transactions).
/// - Failure: the copy is dropped and the bound editor never saw the attempt.
///   If part of it still reached the disk (`RulesNotWritten` after the journal
///   saved), the bound editor re-syncs from disk.
///
/// Either way the bound editor and the served snapshot stay one journal, which
/// is what every fingerprint the routes hand out relies on.
fn write<T>(
    state: &AppState,
    generation: Option<&str>,
    write: impl FnOnce(&mut Loaded) -> Result<T, OpsError>,
) -> Result<T, AppError> {
    let mut guard = lock_editor(state)?;
    let editor: &JournalEditor = bound(&mut guard)?;
    let snapshot = state.snapshot();
    if let Some(expected) = generation
        && expected != generation_of(&snapshot.etag)
    {
        return Err(AppError::Conflict(
            "the journal changed since this was listed; refresh and try again".to_string(),
        ));
    }
    if !view(state, &snapshot).gate.enabled {
        return Err(refuse_disabled());
    }
    let location = snapshot
        .storage()
        .ok_or_else(|| AppError::Conflict("this journal has no backing file".to_string()))?;
    let books = Books::open_in(Arc::clone(location.vault()), location.main().clone());
    let outcome = books.load_from(editor).and_then(|mut loaded| {
        let value = write(&mut loaded)?;
        Ok((value, loaded.into_editor()))
    });
    // Publish whatever reached the disk — and nothing when nothing did, so a
    // refused or empty write leaves the generation (and every client's view)
    // as it was.
    match outcome {
        Ok((value, edited)) => {
            if Arc::ptr_eq(edited.journal(), editor.journal()) {
                // The journal is untouched; the recognition file or `[books]`
                // may not be.
                if changed_on_disk(state) {
                    drop(guard);
                    state.republish();
                }
            } else {
                state.replace_journal(edited.journal());
                *guard = Some(edited);
            }
            Ok(value)
        }
        Err(error) => {
            let journal_written = guard
                .as_ref()
                .is_some_and(|editor| !editor.changed_sources().is_empty());
            if journal_written {
                resync_from_disk(state, &mut guard)?;
            } else if changed_on_disk(state) {
                drop(guard);
                state.republish();
            }
            Err(ops_error(error))
        }
    }
}

/// Answer a GET from the blocking pool, `304` when the client holds this
/// snapshot's body for this URL and date.
async fn answer(
    state: AppState,
    headers: &HeaderMap,
    today: &str,
    job: impl FnOnce(&AppState, &Snapshot, &AccrualView) -> Result<Bytes, AppError> + Send + 'static,
) -> Result<Response, AppError> {
    let snapshot = state.snapshot();
    let etag = dated_etag(&snapshot, today);
    if if_none_match_hits(headers, &etag) {
        return Ok(not_modified(etag));
    }
    let Json(body) = compute(move || {
        let accrual = view(&state, &snapshot);
        if !accrual.gate.enabled {
            return disabled_body();
        }
        job(&state, &snapshot, &accrual)
    })
    .await?;
    Ok(ok_json(etag, body))
}

// ===========================================================================
// GET /api/accrual
// ===========================================================================

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AccrualQuery {
    today: Option<String>,
    /// Days "Needs a schedule" looks back; default 90, `0` = everything.
    lookback: Option<u32>,
    /// The invoice matcher's lookback (the user's `settlementLookbackDays`);
    /// default 180, `0` = everything.
    settlement_lookback: Option<u32>,
    /// The matcher's tolerance in basis points (`settlementToleranceBps`);
    /// default 300.
    settlement_tolerance: Option<u32>,
    /// How many days a payment may precede its invoice
    /// (`settlementLeadDays`); default 4.
    settlement_lead: Option<u32>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AccrualBody<'a> {
    enabled: bool,
    generation: &'a str,
    basis: WireBasis,
    /// Why the gate is open ("because `liabilities:taxes payable` looks like
    /// a payable").
    reason: WireReason,
    roles: BTreeMap<&'a str, WireRole>,
    #[serde(skip_serializing_if = "Option::is_none")]
    file: Option<WireFile>,
    #[serde(skip_serializing_if = "Option::is_none")]
    closed_through: Option<&'a str>,
    counts: WireCounts,
    #[serde(skip_serializing_if = "Option::is_none")]
    problem: Option<&'a str>,
    warnings: &'a [String],
}

#[derive(Serialize)]
struct WireBasis {
    value: &'static str,
    /// `books` when `[books] basis` set it, else `inferred`.
    source: &'static str,
}

#[derive(Serialize)]
struct WireReason {
    kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    account: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    role: Option<&'static str>,
}

#[derive(Serialize)]
struct WireRole {
    role: &'static str,
    declared: bool,
}

#[derive(Serialize)]
struct WireFile {
    /// Relative to the journal's folder.
    path: String,
    exists: bool,
    included: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireCounts {
    due: usize,
    due_closed: usize,
    needs_schedule: usize,
    /// The CLEAR proposals (exact, unambiguous, with a number or party
    /// signal): what Link all takes, and what the badge counts.
    invoices: usize,
    /// The proposals to check by hand.
    invoices_check: usize,
}

fn relative(location_root: &VaultPath, path: &VaultPath) -> String {
    let root = location_root.as_str();
    path.as_str()
        .strip_prefix(root)
        .map(|rest| rest.trim_start_matches('/'))
        .filter(|rest| !rest.is_empty())
        .unwrap_or(path.as_str())
        .to_string()
}

/// `GET /api/accrual?today&lookback` — the gate and the counts the toolbar
/// badge and the editor read (Phase 2 table).
pub(crate) async fn accrual(
    State(state): State<AppState>,
    Query(query): Query<AccrualQuery>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let today = effective_today(query.today)?;
    let lookback = query.lookback.unwrap_or(NEEDS_SCHEDULE_LOOKBACK_DAYS);
    let settings = SettleQuery {
        today: None,
        lookback: query.settlement_lookback,
        tolerance: query.settlement_tolerance,
        lead: query.settlement_lead,
    }
    .settings();
    let day = today.clone();
    answer(state, &headers, &today, move |_, snapshot, accrual| {
        let journal = &snapshot.journal;
        let index = accrual.index(journal);
        let set = accrual.due(journal, &day);
        let needs = needs_schedule(journal, &index, &day, lookback).len();
        let proposals = settle::propose(journal, &accrual.roles, &settings, &day);
        let clear = proposals.iter().filter(|p| p.clear).count();
        let root = snapshot
            .storage()
            .map(|location| location.root_dir())
            .unwrap_or_default();
        let reason = match &accrual.gate.reason {
            GateReason::Basis(_) => WireReason {
                kind: "basis",
                account: None,
                role: None,
            },
            GateReason::Declared { account, role } => WireReason {
                kind: "declared",
                account: Some(account.clone()),
                role: Some(role.code()),
            },
            GateReason::Inferred { account, role } => WireReason {
                kind: "inferred",
                account: Some(account.clone()),
                role: Some(role.code()),
            },
            GateReason::NoAccrualAccounts => WireReason {
                kind: "none",
                account: None,
                role: None,
            },
        };
        to_bytes(&AccrualBody {
            enabled: true,
            generation: generation_of(&snapshot.etag),
            basis: WireBasis {
                value: accrual.config.basis.unwrap_or(Basis::Accrual).code(),
                source: if accrual.config.basis.is_some() {
                    "books"
                } else {
                    "inferred"
                },
            },
            reason,
            roles: accrual
                .roles
                .iter()
                .map(|(account, role, source)| {
                    (
                        account,
                        WireRole {
                            role: role.code(),
                            declared: source == RoleSource::Declared,
                        },
                    )
                })
                .collect(),
            file: accrual.file.as_ref().map(|file| WireFile {
                path: relative(&root, &file.path),
                exists: file.exists,
                included: file.included,
            }),
            closed_through: accrual.config.closed_through.as_deref(),
            counts: WireCounts {
                due: set.open.len(),
                due_closed: set.closed.len(),
                needs_schedule: needs,
                invoices: clear,
                invoices_check: proposals.len() - clear,
            },
            problem: accrual.problem.as_deref(),
            warnings: &accrual.config.warnings,
        })
    })
    .await
}

/// The deferrals with no schedule dated within `lookback` days of `today`
/// (`0` = all).
fn needs_schedule(
    journal: &Journal,
    index: &RecognitionIndex,
    today: &str,
    lookback: u32,
) -> Vec<PostingRef> {
    let since =
        (lookback > 0).then(|| ledgeline_core::reports::add_days(today, -i64::from(lookback)));
    index
        .needs_schedule()
        .iter()
        .copied()
        .filter(|at| {
            let date = journal.transactions[at.position].date.as_str();
            date <= today && since.as_deref().is_none_or(|since| date >= since)
        })
        .collect()
}

// ===========================================================================
// GET /api/recognition/needs
// ===========================================================================

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct NeedsBody<'a> {
    enabled: bool,
    generation: &'a str,
    /// The lookback applied, in days (`0` = everything).
    lookback: u32,
    /// Newest first: the deferred-revenue credits and prepaid debits with no
    /// `recognition:` tag, inside the lookback.
    items: Vec<WireSide>,
}

/// `GET /api/recognition/needs?today&lookback` — the **Needs a schedule**
/// tab (D6): the deferrals `/api/accrual` counts, one per posting, newest
/// first. Each opens the editor's Accrual section on its transaction.
pub(crate) async fn needs_route(
    State(state): State<AppState>,
    Query(query): Query<AccrualQuery>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let today = effective_today(query.today)?;
    let lookback = query.lookback.unwrap_or(NEEDS_SCHEDULE_LOOKBACK_DAYS);
    let day = today.clone();
    answer(state, &headers, &today, move |_, snapshot, accrual| {
        let journal = &snapshot.journal;
        let index = accrual.index(journal);
        let mut items: Vec<WireSide> = needs_schedule(journal, &index, &day, lookback)
            .into_iter()
            .map(|at| wire_side(journal, at, None))
            .collect();
        items.sort_by(|a, b| b.date.cmp(&a.date).then(b.index.cmp(&a.index)));
        to_bytes(&NeedsBody {
            enabled: true,
            generation: generation_of(&snapshot.etag),
            lookback,
            items,
        })
    })
    .await
}

// ===========================================================================
// GET /api/recognition/due
// ===========================================================================

#[derive(Deserialize)]
pub(crate) struct TodayQuery {
    today: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DueBody<'a> {
    enabled: bool,
    generation: &'a str,
    today: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    closed_through: Option<&'a str>,
    months: Vec<WireMonth>,
    closed: WireClosed,
}

#[derive(Serialize)]
struct WireMonth {
    month: String,
    count: usize,
    totals: Vec<WireMoney>,
    groups: Vec<WireGroup>,
    ties: Vec<WireTie>,
}

#[derive(Serialize)]
struct WireGroup {
    #[serde(skip_serializing_if = "Option::is_none")]
    party: Option<WireParty>,
    entries: Vec<WireDue>,
}

#[derive(Serialize)]
struct WireDue {
    id: String,
    recognized: String,
    description: String,
    postings: Vec<WirePosting>,
}

#[derive(Serialize)]
struct WireTie {
    account: String,
    commodity: String,
    before: WireDec,
    after: WireDec,
    scheduled: WireDec,
    ties: bool,
}

#[derive(Serialize)]
struct WireClosed {
    count: usize,
    totals: Vec<WireMoney>,
    entries: Vec<WireDue>,
}

fn wire_due(book: &RecognitionBook, entry: &DueEntry) -> WireDue {
    let rule = book.rules.get(entry.rule);
    WireDue {
        id: entry.id.clone(),
        recognized: entry.recognized.clone(),
        description: rule.map(|r| r.description.clone()).unwrap_or_default(),
        postings: rule
            .map(|r| r.postings.iter().map(WirePosting::of).collect())
            .unwrap_or_default(),
    }
}

/// What a set of due entries recognizes, per commodity: the debit sides.
fn totals(book: &RecognitionBook, entries: &[&DueEntry]) -> Vec<WireMoney> {
    let mut sums: BTreeMap<String, Dec> = BTreeMap::new();
    for entry in entries {
        if let Some(rule) = book.rules.get(entry.rule) {
            for amount in rule.postings.iter().flat_map(|p| &p.amounts) {
                if amount.quantity.mantissa > 0 {
                    let sum = sums
                        .entry(amount.commodity.0.clone())
                        .or_insert(Dec::zero());
                    *sum = sum.add(amount.quantity).unwrap_or(*sum);
                }
            }
        }
    }
    sums.iter().map(|(c, q)| WireMoney::of(c, *q)).collect()
}

/// `GET /api/recognition/due?today` — the catch-up table (D6): months, then
/// customers/vendors, each entry with its postings; the month's ties; and
/// the closed section.
pub(crate) async fn due_route(
    State(state): State<AppState>,
    Query(query): Query<TodayQuery>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let today = effective_today(query.today)?;
    let day = today.clone();
    answer(state, &headers, &today, move |_, snapshot, accrual| {
        let journal = &snapshot.journal;
        let index = accrual.index(journal);
        let book = accrual.book();
        let set = accrual.due(journal, &day);
        let all_ties = ties(journal, &index, &book, &set);
        let mut months: BTreeMap<&str, Vec<&DueEntry>> = BTreeMap::new();
        for entry in &set.open {
            months
                .entry(&entry.recognized[..7])
                .or_default()
                .push(entry);
        }
        let months = months
            .into_iter()
            .map(|(month, entries)| {
                let mut groups: Vec<(Option<Party>, Vec<&DueEntry>)> = Vec::new();
                for entry in &entries {
                    match groups.iter_mut().find(|(party, _)| *party == entry.party) {
                        Some((_, members)) => members.push(entry),
                        None => groups.push((entry.party.clone(), vec![entry])),
                    }
                }
                WireMonth {
                    month: month.to_string(),
                    count: entries.len(),
                    totals: totals(&book, &entries),
                    groups: groups
                        .into_iter()
                        .map(|(party, members)| WireGroup {
                            party: party.as_ref().map(WireParty::of),
                            entries: members.iter().map(|e| wire_due(&book, e)).collect(),
                        })
                        .collect(),
                    ties: all_ties
                        .iter()
                        .filter(|tie| tie.month == month)
                        .map(|tie| WireTie {
                            account: tie.account.clone(),
                            commodity: tie.commodity.clone(),
                            before: tie.before.into(),
                            after: tie.after.into(),
                            scheduled: tie.scheduled.into(),
                            ties: tie.ties,
                        })
                        .collect(),
                }
            })
            .collect();
        let closed: Vec<&DueEntry> = set.closed.iter().collect();
        to_bytes(&DueBody {
            enabled: true,
            generation: generation_of(&snapshot.etag),
            today: &day,
            closed_through: accrual.config.closed_through.as_deref(),
            months,
            closed: WireClosed {
                count: closed.len(),
                totals: totals(&book, &closed),
                entries: closed.iter().map(|e| wire_due(&book, e)).collect(),
            },
        })
    })
    .await
}

// ===========================================================================
// POST /api/recognition/post
// ===========================================================================

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PostIn {
    /// The snapshot generation the client listed.
    generation: String,
    /// Post entries dated on or before this; today when absent. A date after
    /// today is refused.
    #[serde(default)]
    through: Option<String>,
    /// Only these; every open due entry when absent.
    #[serde(default)]
    entries: Option<Vec<EntryRef>>,
    /// Also fold the closed periods' entries into one adjustment per
    /// schedule, dated today.
    #[serde(default)]
    include_closed: bool,
    /// The client's today, which may only lower the server's.
    #[serde(default)]
    today: Option<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EntryRef {
    id: String,
    recognized: String,
}

#[derive(Serialize)]
pub(crate) struct PostedBody {
    enabled: bool,
    generation: String,
    posted: Vec<EntryRef>,
    adjustments: Vec<String>,
}

/// `POST /api/recognition/post` — **Post through ‹date›** (D6): one
/// `add_transactions`, placed by date; what is already posted is dropped, so a
/// repeat writes nothing. `409` on a stale generation.
pub(crate) async fn post(
    State(state): State<AppState>,
    payload: Result<Json<PostIn>, JsonRejection>,
) -> Result<Json<PostedBody>, AppError> {
    let request = json_body(payload)?;
    let today = effective_today(request.today.clone())?;
    let task_state = state.clone();
    compute(move || {
        let state = task_state;
        let posted = redacted(
            &state,
            write(&state, Some(&request.generation), |loaded| {
                loaded.post_due(&PostDue {
                    today: today.clone(),
                    through: request.through.clone(),
                    only: request.entries.as_ref().map(|entries| {
                        entries
                            .iter()
                            .map(|e| (e.id.clone(), e.recognized.clone()))
                            .collect()
                    }),
                    include_closed: request.include_closed,
                    expected: None,
                })
            }),
        )?;
        Ok(PostedBody {
            enabled: true,
            generation: generation_of(&state.snapshot().etag).to_string(),
            posted: posted
                .entries
                .into_iter()
                .map(|(id, recognized)| EntryRef { id, recognized })
                .collect(),
            adjustments: posted.adjustments,
        })
    })
    .await
}

// ===========================================================================
// GET /api/recognition/for/{index}
// ===========================================================================

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ForBody<'a> {
    enabled: bool,
    generation: &'a str,
    schedules: Vec<WireScheduleStatus>,
    /// The deferral lines of the transaction with no schedule yet (posting
    /// positions).
    unscheduled: Vec<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    renewal: Option<WireRenewal>,
    /// A customer/vendor guessed from the payee: offered, never applied.
    #[serde(skip_serializing_if = "Option::is_none")]
    party_suggestion: Option<WireParty>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireScheduleStatus {
    spec: WireSpec,
    periods: Vec<WirePeriod>,
    rules: Vec<WireRule>,
    renewal_date: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireRenewal {
    /// The party's latest schedule, to prefill from.
    prefill: WireSpec,
    /// `renewal` with an earlier schedule.
    contract: &'static str,
}

/// `GET /api/recognition/for/{index}?today` — the schedules of one source
/// transaction: each period's status (posted, due, closed, future), the
/// renewal prefill for its party, and a party suggestion for an
/// unattributed one.
pub(crate) async fn for_transaction(
    State(state): State<AppState>,
    Path(tindex): Path<u32>,
    Query(query): Query<TodayQuery>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let today = effective_today(query.today)?;
    let day = today.clone();
    answer(state, &headers, &today, move |_, snapshot, accrual| {
        let journal = &snapshot.journal;
        let position = journal
            .transactions
            .iter()
            .position(|t| t.index.0 == tindex)
            .ok_or_else(|| AppError::NotFound(format!("no transaction with index {tindex}")))?;
        let txn = &journal.transactions[position];
        let index = accrual.index(journal);
        let book = accrual.book();
        let styles = JournalStyles::of(journal);
        let set = accrual.due(journal, &day);
        let ids: Vec<&str> = index
            .schedules()
            .iter()
            .filter(|s| {
                s.sources
                    .iter()
                    .any(|source| source.at.position == position)
            })
            .map(|s| s.id.as_str())
            .collect();
        let schedules =
            ids.iter()
                .filter_map(|id| {
                    let spec = index.spec_of(id, &book)?;
                    let schedule = build(&spec, &styles).ok()?;
                    index.schedule(id).filter(|info| !info.rules.is_empty())?;
                    Some(WireScheduleStatus {
                        spec: WireSpec::of(&spec),
                        periods: schedule
                            .periods
                            .iter()
                            .map(|period| {
                                let posted = index.posted(id, &period.date).first().copied();
                                let state =
                                    match posted {
                                        Some(_) => "posted",
                                        None if set.closed.iter().any(|e| {
                                            e.id == *id && e.recognized == period.date
                                        }) =>
                                        {
                                            "closed"
                                        }
                                        None if set.open.iter().any(|e| {
                                            e.id == *id && e.recognized == period.date
                                        }) =>
                                        {
                                            "due"
                                        }
                                        None if period.date.as_str() <= day.as_str() => "covered",
                                        None => "future",
                                    };
                                WirePeriod::of(
                                    period,
                                    Some(state),
                                    posted.map(|at| journal.transactions[at].index.0),
                                )
                            })
                            .collect(),
                        rules: schedule
                            .rules
                            .iter()
                            .map(|rule| WireRule {
                                role: role_code(rule.role),
                                period: rule.period.clone(),
                                text: rule.text.clone(),
                            })
                            .collect(),
                        renewal_date: schedule.renewal_date.clone(),
                    })
                })
                .collect();
        let unscheduled = index
            .needs_schedule()
            .iter()
            .filter(|at| at.position == position)
            .map(|at| at.posting)
            .collect();
        let party = index
            .schedules()
            .iter()
            .filter(|s| ids.contains(&s.id.as_str()))
            .find_map(|s| s.party().cloned())
            .or_else(|| {
                txn.postings
                    .iter()
                    .find_map(|p| party_tag(&p.tags))
                    .or_else(|| party_tag(&txn.tags))
            });
        let renewal = party.as_ref().and_then(|party| {
            let earlier = index
                .schedules_of(party)
                .into_iter()
                .rfind(|s| !ids.contains(&s.id.as_str()))?;
            Some(WireRenewal {
                prefill: WireSpec::of(&index.spec_of(&earlier.id, &book)?),
                contract: Contract::Renewal.code(),
            })
        });
        let party_suggestion = party
            .is_none()
            .then(|| index.party_for_payee(&txn.description))
            .flatten()
            .map(WireParty::of);
        to_bytes(&ForBody {
            enabled: true,
            generation: generation_of(&snapshot.etag),
            schedules,
            unscheduled,
            renewal,
            party_suggestion,
        })
    })
    .await
}

fn party_tag(tags: &[(String, String)]) -> Option<Party> {
    [
        ("customer", PartyKind::Customer),
        ("vendor", PartyKind::Vendor),
    ]
    .into_iter()
    .find_map(|(name, kind)| {
        ledgeline_core::accrual::tag_value(tags, name)
            .filter(|v| !v.is_empty())
            .map(|v| Party {
                kind,
                name: v.to_string(),
            })
    })
}

// ===========================================================================
// POST /api/recognition/preview and /save
// ===========================================================================

/// The source transaction's change.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub(crate) enum SourceIn {
    /// Add a new source transaction.
    Add { transaction: AddRequest },
    /// Replace the transaction `index`, still at `fingerprint`.
    Replace {
        index: u32,
        fingerprint: String,
        transaction: AddRequest,
    },
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SaveIn {
    #[serde(default)]
    source: Option<SourceIn>,
    schedules: Vec<SpecIn>,
    #[serde(default)]
    create_file: bool,
    /// From the preview (`accrualGeneration`): refuse if the books changed.
    #[serde(default)]
    accrual_generation: Option<String>,
    #[serde(default)]
    today: Option<String>,
}

/// The core save request for `request`, against `loaded`'s journal.
fn save_request(loaded: &Loaded, request: &SaveIn, today: &str) -> Result<SaveRequest, AppError> {
    let journal = loaded.journal();
    let source = match &request.source {
        None => SourceChange::Unchanged,
        Some(SourceIn::Add { transaction }) => {
            SourceChange::Add(Box::new(edit_api::build_transaction(journal, transaction)?))
        }
        Some(SourceIn::Replace {
            index,
            fingerprint,
            transaction,
        }) => {
            let current = loaded
                .editor()
                .transaction_fingerprint(Tindex(*index))
                .ok_or_else(|| AppError::NotFound(format!("no transaction with index {index}")))?;
            if current.token() != *fingerprint {
                return Err(AppError::Conflict(format!(
                    "transaction #{index} changed since it was opened; reload it"
                )));
            }
            SourceChange::Replace {
                tindex: Tindex(*index),
                fingerprint: current,
                transaction: Box::new(edit_api::build_transaction(journal, transaction)?),
            }
        }
    };
    let mut ids = loaded.id_generator();
    let schedules = request
        .schedules
        .iter()
        .map(|spec| spec.to_core(journal, &mut || ids.next_id()))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(SaveRequest {
        source,
        schedules,
        today: today.to_string(),
        create_file: request.create_file,
        expected: request
            .accrual_generation
            .clone()
            .map(Generation::from_token),
    })
}

/// `POST /api/recognition/preview` — what a save would write: the new
/// schedules (periods and rule text) and the restatement of any existing one
/// (D4's preview). Writes nothing.
pub(crate) async fn preview(
    State(state): State<AppState>,
    payload: Result<Json<SaveIn>, JsonRejection>,
) -> Result<Response, AppError> {
    let request = json_body(payload)?;
    let today = effective_today(request.today.clone())?;
    let Json(body) = compute(move || {
        let snapshot = state.snapshot();
        if !view(&state, &snapshot).gate.enabled {
            return disabled_body();
        }
        let location = snapshot
            .storage()
            .ok_or_else(|| AppError::Conflict("this journal has no backing file".to_string()))?;
        let books = Books::open_in(Arc::clone(location.vault()), location.main().clone());
        // A copy of the bound editor, not a fresh parse: the editor form asks
        // for a preview on every pause in typing.
        let loaded = {
            let mut guard = lock_editor(&state)?;
            books.load_from(bound(&mut guard)?)
        }
        .map_err(ops_error)?;
        let core = save_request(&loaded, &request, &today)?;
        let plan = redacted(&state, loaded.preview(&core).map_err(ops_error))?;
        to_bytes(&WirePlan::of(&plan, None))
    })
    .await?;
    Ok(crate::journal_api::ok_json(state_free_etag(), body))
}

/// A fresh tag for a body that is never cached (a POST answer).
fn state_free_etag() -> axum::http::HeaderValue {
    axum::http::HeaderValue::from_static("\"accrual-preview\"")
}

/// `POST /api/recognition/save` — the D-editor save: the source transaction,
/// then each schedule's rule group (new) or restatement (existing). Creates the
/// recognition file when `createFile` says so. Journal first, rules last.
pub(crate) async fn save(
    State(state): State<AppState>,
    payload: Result<Json<SaveIn>, JsonRejection>,
) -> Result<Response, AppError> {
    let request = json_body(payload)?;
    let today = effective_today(request.today.clone())?;
    let task_state = state.clone();
    let Json(body) = compute(move || {
        let state = task_state;
        let plan = redacted(
            &state,
            write(&state, None, |loaded| {
                let core = save_request(loaded, &request, &today)
                    .map_err(|error| OpsError::Storage(error.to_string()))?;
                loaded.save(&core)
            }),
        )?;
        let generation = generation_of(&state.snapshot().etag).to_string();
        to_bytes(&WirePlan::of(&plan, Some(generation)))
    })
    .await?;
    state.recognize().wake();
    Ok(crate::journal_api::ok_json(state_free_etag(), body))
}

// ===========================================================================
// POST /api/recognition/file and /api/books/closed-through
// ===========================================================================

#[derive(Serialize)]
pub(crate) struct FileBody {
    enabled: bool,
    generation: String,
    path: String,
}

/// `POST /api/recognition/file` — create the recognition file (D1): `409`
/// when something is already there.
pub(crate) async fn create_file(State(state): State<AppState>) -> Result<Json<FileBody>, AppError> {
    let task_state = state.clone();
    compute(move || {
        let state = task_state;
        let path = redacted(&state, write(&state, None, Loaded::create_recognition_file))?;
        let snapshot = state.snapshot();
        let root = snapshot.storage().map(|l| l.root_dir()).unwrap_or_default();
        Ok(FileBody {
            enabled: true,
            generation: generation_of(&snapshot.etag).to_string(),
            path: relative(&root, &path),
        })
    })
    .await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ClosedThroughIn {
    /// `YYYY-MM-DD`, or `null` to clear it.
    date: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ClosedThroughBody {
    generation: String,
    closed_through: Option<String>,
}

/// `POST /api/books/closed-through` — **Close books through…**: set or clear
/// `[books] closed-through` in `ledgeline.toml` (creating it, D-settings).
pub(crate) async fn closed_through(
    State(state): State<AppState>,
    payload: Result<Json<ClosedThroughIn>, JsonRejection>,
) -> Result<Json<ClosedThroughBody>, AppError> {
    let request = json_body(payload)?;
    let task_state = state.clone();
    compute(move || {
        let state = task_state;
        let location = state
            .snapshot()
            .storage()
            .ok_or_else(|| AppError::Conflict("this journal has no backing file".to_string()))?;
        Books::open_in(Arc::clone(location.vault()), location.main().clone())
            .set_closed_through(request.date.as_deref())
            .map_err(ops_error)?;
        state.republish();
        let snapshot = state.snapshot();
        Ok(ClosedThroughBody {
            generation: generation_of(&snapshot.etag).to_string(),
            closed_through: view(&state, &snapshot).config.closed_through.clone(),
        })
    })
    .await
}

// ===========================================================================
// Settlements
// ===========================================================================

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SettleQuery {
    today: Option<String>,
    lookback: Option<u32>,
    /// Basis points of the invoice; default 300 (3%, Q3).
    tolerance: Option<u32>,
    /// Days a payment may precede its invoice; default 4.
    lead: Option<u32>,
}

impl SettleQuery {
    fn settings(&self) -> SettleSettings {
        let defaults = SettleSettings::default();
        SettleSettings {
            lookback_days: self.lookback.unwrap_or(defaults.lookback_days),
            tolerance_bps: self.tolerance.unwrap_or(defaults.tolerance_bps),
            lead_days: self.lead.unwrap_or(defaults.lead_days),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireSide {
    index: u32,
    posting: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    fingerprint: Option<String>,
    date: String,
    description: String,
    account: String,
    amount: WireMoney,
    #[serde(skip_serializing_if = "Option::is_none")]
    invoice: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    party: Option<WireParty>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireFit {
    kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    difference: Option<WireDec>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireProposal {
    invoice: WireSide,
    payment: WireSide,
    fit: WireFit,
    by_number: bool,
    by_party: bool,
    gap_days: u32,
    /// The payment came before the invoice (within the lead days).
    early: bool,
    ambiguous: bool,
    clear: bool,
    /// For a short payment: the account last used for fees on this
    /// receivable/payable, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    fee_account: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SettlementsBody<'a> {
    enabled: bool,
    generation: &'a str,
    counts: WireSettleCounts,
    proposals: Vec<WireProposal>,
    open: WireOpen,
}

#[derive(Serialize)]
struct WireSettleCounts {
    clear: usize,
    check: usize,
}

#[derive(Serialize)]
struct WireOpen {
    invoices: Vec<WireSide>,
    payments: Vec<WireSide>,
}

fn fit_wire(fit: Fit) -> WireFit {
    match fit {
        Fit::Exact => WireFit {
            kind: "exact",
            difference: None,
        },
        Fit::Short { fee } => WireFit {
            kind: "short",
            difference: Some(fee.into()),
        },
        Fit::Over { excess } => WireFit {
            kind: "over",
            difference: Some(excess.into()),
        },
    }
}

/// The fingerprints of `indices` from `editor`, when it holds exactly
/// `snapshot`'s journal; `None`s otherwise (no token is better than a wrong
/// one).
fn fingerprints(
    editor: Option<&JournalEditor>,
    snapshot: &Snapshot,
    indices: &[Tindex],
) -> Vec<Option<String>> {
    match editor {
        Some(editor)
            if Arc::ptr_eq(editor.journal(), &snapshot.raw)
                || Arc::ptr_eq(editor.journal(), &snapshot.journal) =>
        {
            editor
                .transaction_fingerprints(indices)
                .iter()
                .map(|f| f.as_ref().map(Fingerprint::token))
                .collect()
        }
        _ => vec![None; indices.len()],
    }
}

/// Fingerprints for the snapshot a job read.
type Tokens<'a> = &'a dyn Fn(&[Tindex]) -> Vec<Option<String>>;

/// Answer a GET whose body carries fingerprints: like [`answer`], but the
/// snapshot is read UNDER the editor lock and fingerprinted before it is
/// released — the pattern `review_api::fingerprinted` follows — so the rows
/// and their tokens always describe the same journal.
///
/// Reading the snapshot first and locking only for the fingerprints (as this
/// route once did) lost every token whenever a publish landed in between: the
/// file watcher reloads after each of our own writes, and the list the client
/// re-read right after a Link came back with no fingerprints at all, so every
/// other Link stayed disabled.
async fn answer_fingerprinted(
    state: AppState,
    headers: &HeaderMap,
    today: &str,
    job: impl FnOnce(&Snapshot, &AccrualView, Tokens<'_>) -> Result<Bytes, AppError> + Send + 'static,
) -> Result<Response, AppError> {
    if if_none_match_hits(headers, &dated_etag(&state.snapshot(), today)) {
        return Ok(not_modified(dated_etag(&state.snapshot(), today)));
    }
    let today = today.to_string();
    let Json((etag, body)) = compute(move || {
        let guard = lock_editor(&state)?;
        let snapshot = state.snapshot();
        let etag = dated_etag(&snapshot, &today);
        let accrual = view(&state, &snapshot);
        if !accrual.gate.enabled {
            return Ok((etag, disabled_body()?));
        }
        let tokens = |indices: &[Tindex]| fingerprints(guard.as_ref(), &snapshot, indices);
        let body = job(&snapshot, &accrual, &tokens)?;
        drop(guard);
        Ok((etag, body))
    })
    .await?;
    Ok(ok_json(etag, body))
}

fn wire_side(journal: &Journal, at: PostingRef, fingerprint: Option<String>) -> WireSide {
    let txn = &journal.transactions[at.position];
    let posting = &txn.postings[at.posting];
    WireSide {
        index: txn.index.0,
        posting: at.posting,
        fingerprint,
        date: posting.date.clone().unwrap_or_else(|| txn.date.clone()),
        description: txn.description.clone(),
        account: posting.account.0.clone(),
        amount: posting
            .amounts
            .first()
            .map_or_else(|| WireMoney::of("", Dec::zero()), WireMoney::amount),
        invoice: ledgeline_core::accrual::tag_value(&txn.tags, "invoice").map(str::to_string),
        party: party_tag(&posting.tags)
            .or_else(|| party_tag(&txn.tags))
            .as_ref()
            .map(WireParty::of),
    }
}

/// The proposals and the open candidates, wired, for `scope` (all, or one
/// transaction's).
fn settlements_body(
    tokens_of: Tokens<'_>,
    snapshot: &Snapshot,
    accrual: &AccrualView,
    settings: &SettleSettings,
    today: &str,
    scope: Option<usize>,
) -> Result<Bytes, AppError> {
    let journal = &snapshot.journal;
    let proposals: Vec<SettleProposal> = settle::propose(journal, &accrual.roles, settings, today)
        .into_iter()
        .filter(|p| scope.is_none_or(|at| p.invoice.position == at || p.payment.position == at))
        .collect();
    let candidates: Vec<SettleCandidate<'_>> =
        settle::candidates(journal, &accrual.roles, settings, today)
            .into_iter()
            .filter(|c| scope.is_none_or(|at| c.at.position == at))
            .filter(|c| {
                !proposals
                    .iter()
                    .any(|p| p.invoice == c.at || p.payment == c.at)
            })
            .collect();
    let refs: Vec<PostingRef> = proposals
        .iter()
        .flat_map(|p| [p.invoice, p.payment])
        .chain(candidates.iter().map(|c| c.at))
        .collect();
    let tokens = tokens_of(&refs.iter().map(|at| at.txn).collect::<Vec<_>>());
    let token_of = |at: PostingRef| {
        refs.iter()
            .position(|r| *r == at)
            .and_then(|i| tokens[i].clone())
    };
    let wired: Vec<WireProposal> = proposals
        .iter()
        .map(|p| WireProposal {
            invoice: wire_side(journal, p.invoice, token_of(p.invoice)),
            payment: wire_side(journal, p.payment, token_of(p.payment)),
            fit: fit_wire(p.fit),
            by_number: p.by_number,
            by_party: p.by_party,
            gap_days: p.gap_days,
            early: p.early,
            ambiguous: p.ambiguous,
            clear: p.clear,
            fee_account: matches!(p.fit, Fit::Short { .. })
                .then(|| {
                    let account = &journal.transactions[p.invoice.position].postings
                        [p.invoice.posting]
                        .account
                        .0;
                    settle::fee_account_for(journal, account)
                })
                .flatten(),
        })
        .collect();
    let side = |which: Side| {
        candidates
            .iter()
            .filter(|c| c.side == which)
            .map(|c| wire_side(journal, c.at, token_of(c.at)))
            .collect()
    };
    to_bytes(&SettlementsBody {
        enabled: true,
        generation: generation_of(&snapshot.etag),
        counts: WireSettleCounts {
            clear: wired.iter().filter(|p| p.clear).count(),
            check: wired.iter().filter(|p| !p.clear).count(),
        },
        proposals: wired,
        open: WireOpen {
            invoices: side(Side::Invoice),
            payments: side(Side::Payment),
        },
    })
}

/// `GET /api/settlements?today&lookback&tolerance` — invoice ↔ payment
/// proposals (D-settle) and the invoices and payments with no partner.
pub(crate) async fn settlements(
    State(state): State<AppState>,
    Query(query): Query<SettleQuery>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let today = effective_today(query.today.clone())?;
    let settings = query.settings();
    let day = today.clone();
    answer_fingerprinted(state, &headers, &today, move |snapshot, accrual, tokens| {
        settlements_body(tokens, snapshot, accrual, &settings, &day, None)
    })
    .await
}

/// `GET /api/settlements/for/{index}` — the proposals and candidates of one
/// transaction, with no lookback.
pub(crate) async fn settlements_for(
    State(state): State<AppState>,
    Path(tindex): Path<u32>,
    Query(query): Query<SettleQuery>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let today = effective_today(query.today.clone())?;
    let settings = SettleSettings {
        lookback_days: 0,
        ..query.settings()
    };
    let day = today.clone();
    answer_fingerprinted(state, &headers, &today, move |snapshot, accrual, tokens| {
        let position = snapshot
            .journal
            .transactions
            .iter()
            .position(|t| t.index.0 == tindex)
            .ok_or_else(|| AppError::NotFound(format!("no transaction with index {tindex}")))?;
        settlements_body(tokens, snapshot, accrual, &settings, &day, Some(position))
    })
    .await
}

/// `{index, posting, fingerprint}`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SideIn {
    index: u32,
    posting: usize,
    fingerprint: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct LinkIn {
    invoice: SideIn,
    payment: SideIn,
    #[serde(default)]
    fee_account: Option<String>,
    #[serde(default)]
    tolerance: Option<u32>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LinkBody {
    generation: String,
    match_id: String,
}

/// The posting `side` names in `loaded`'s journal, checked against its
/// fingerprint.
fn side_ref(loaded: &Loaded, side: &SideIn) -> Result<PostingRef, OpsError> {
    let journal = loaded.journal();
    let position = journal
        .transactions
        .iter()
        .position(|t| t.index.0 == side.index)
        .ok_or(OpsError::Stale)?;
    let current = loaded
        .editor()
        .transaction_fingerprint(Tindex(side.index))
        .ok_or(OpsError::Stale)?;
    if current.token() != side.fingerprint {
        return Err(OpsError::Stale);
    }
    Ok(PostingRef {
        position,
        txn: Tindex(side.index),
        posting: side.posting,
    })
}

/// `POST /api/settlements/link` — link one invoice to its payment, with the
/// fee edit for a short payment (`feeAccount` required then). `409` when a
/// side changed since it was listed.
pub(crate) async fn link(
    State(state): State<AppState>,
    payload: Result<Json<LinkIn>, JsonRejection>,
) -> Result<Json<LinkBody>, AppError> {
    let request = json_body(payload)?;
    let task_state = state.clone();
    compute(move || {
        let state = task_state;
        let id = redacted(
            &state,
            write(&state, None, |loaded| {
                let invoice = side_ref(loaded, &request.invoice)?;
                let payment = side_ref(loaded, &request.payment)?;
                loaded.link_settlement(
                    invoice,
                    payment,
                    request.fee_account.as_deref(),
                    request.tolerance.unwrap_or(settle::DEFAULT_TOLERANCE_BPS),
                )
            }),
        )?;
        Ok(LinkBody {
            generation: generation_of(&state.snapshot().etag).to_string(),
            match_id: id,
        })
    })
    .await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct LinkAllIn {
    generation: String,
    #[serde(default)]
    lookback: Option<u32>,
    #[serde(default)]
    tolerance: Option<u32>,
    /// Days a payment may precede its invoice, as the list was read with.
    #[serde(default)]
    lead: Option<u32>,
    #[serde(default)]
    today: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LinkAllBody {
    generation: String,
    linked: usize,
    match_ids: Vec<String>,
}

/// `POST /api/settlements/link-all` — link every **clear** proposal (exact,
/// unambiguous, with an invoice-number or party signal) EXACTLY as listed at
/// `generation`: `409` when the journal has moved on since, so what is linked
/// is what the user saw and confirmed. The proposals are worked out once,
/// under the write lock, and linked in one batch — one reparse, one save, all
/// or nothing ([`Loaded::link_settlements`]). Nothing is re-proposed, so a
/// pair that only turns clear once its rival is linked waits for the next
/// listing.
pub(crate) async fn link_all(
    State(state): State<AppState>,
    payload: Result<Json<LinkAllIn>, JsonRejection>,
) -> Result<Json<LinkAllBody>, AppError> {
    let request = json_body(payload)?;
    let today = effective_today(request.today.clone())?;
    let task_state = state.clone();
    compute(move || {
        let state = task_state;
        let settings = SettleQuery {
            today: None,
            lookback: request.lookback,
            tolerance: request.tolerance,
            lead: request.lead,
        }
        .settings();
        let ids = redacted(
            &state,
            write(&state, Some(&request.generation), |loaded| {
                let roles = AccrualRoles::of(loaded.journal());
                let links: Vec<SettleLink> =
                    settle::propose(loaded.journal(), &roles, &settings, &today)
                        .into_iter()
                        .filter(|proposal| proposal.clear)
                        .map(|proposal| SettleLink {
                            invoice: proposal.invoice,
                            payment: proposal.payment,
                            fee_account: None,
                        })
                        .collect();
                loaded.link_settlements(&links, settings.tolerance_bps)
            }),
        )?;
        Ok(LinkAllBody {
            generation: generation_of(&state.snapshot().etag).to_string(),
            linked: ids.len(),
            match_ids: ids,
        })
    })
    .await
}

// ===========================================================================
// Auto-recognize (D6 "Auto-post", Q10)
// ===========================================================================

/// The `autoRecognize` loop's per-process state.
#[derive(Debug, Default)]
pub(crate) struct RecognizeAuto {
    /// `(snapshot generation, local day)` of the last run: nothing to do
    /// again until either moves — an edit, an external refresh (Q10), a new
    /// schedule, or midnight.
    last: std::sync::Mutex<Option<(String, String)>>,
    wake: tokio::sync::Notify,
}

impl RecognizeAuto {
    /// Look now (a schedule was just saved).
    pub(crate) fn wake(&self) {
        self.wake.notify_one();
    }
}

/// How often the loop looks: cheap when nothing changed.
const RECOGNIZE_EVERY: std::time::Duration = std::time::Duration::from_secs(60);

/// What one auto-recognize pass did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutoRecognized {
    /// `autoRecognize` is off, the gate is off, or there is no editor.
    Off,
    /// Nothing changed since the last pass.
    Unchanged,
    /// Posted this many entries.
    Posted(usize),
}

/// One pass: when `autoRecognize` is on and the books or the day moved since
/// the last pass, post every open due entry through `today` (never a closed
/// period, never a future date).
pub(crate) fn run_once(state: &AppState, today: &str) -> Result<AutoRecognized, AppError> {
    if !crate::ui_prefs::auto_recognize(state) {
        return Ok(AutoRecognized::Off);
    }
    let snapshot = state.snapshot();
    if !view(state, &snapshot).gate.enabled {
        return Ok(AutoRecognized::Off);
    }
    let key = (generation_of(&snapshot.etag).to_string(), today.to_string());
    let auto = state.recognize();
    {
        let last = auto
            .last
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if last.as_ref() == Some(&key) {
            return Ok(AutoRecognized::Unchanged);
        }
    }
    let posted = write(state, None, |loaded| {
        loaded.post_due(&PostDue {
            today: today.to_string(),
            ..PostDue::default()
        })
    })?;
    let now = (
        generation_of(&state.snapshot().etag).to_string(),
        today.to_string(),
    );
    *auto
        .last
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(now);
    Ok(AutoRecognized::Posted(posted.entries.len()))
}

/// The background loop: a pass shortly after start-up, then every minute or
/// on a wake. Each pass re-reads the setting, so turning it off needs nothing
/// cancelled.
pub(crate) async fn run_forever(state: AppState) {
    let auto = state.recognize_arc();
    tokio::select! {
        () = tokio::time::sleep(std::time::Duration::from_secs(5)) => {}
        () = auto.wake.notified() => {}
    }
    loop {
        let pass_state = state.clone();
        let today = server_today();
        match tokio::task::spawn_blocking(move || run_once(&pass_state, &today)).await {
            Ok(Ok(_)) => {}
            Ok(Err(error)) => {
                eprintln!("ledgeline: automatic recognition could not be posted: {error}");
            }
            Err(error) => eprintln!("ledgeline: the automatic recognition pass panicked: {error}"),
        }
        tokio::select! {
            () = tokio::time::sleep(RECOGNIZE_EVERY) => {}
            () = auto.wake.notified() => {}
        }
    }
}
