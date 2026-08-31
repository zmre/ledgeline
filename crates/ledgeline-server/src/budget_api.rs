//! The HTTP surface for **editing budget goals** — the `~` periodic rules that
//! `GET /api/budget` reports against.
//!
//! Four routes:
//!
//! - `GET /api/budget/lines` — every goal the open journal declares, per file.
//! - `PUT /api/budget/lines/{*journalId}` — one change to one file's goals.
//! - `POST /api/budget/file` — create a `budget.journal` and `include` it, for a
//!   journal that has nowhere to put a first goal.
//! - `GET /api/budget/reference` — what one account actually did, period by
//!   period, so a goal is set against history rather than from memory.
//!
//! # Why this is its own route family and not part of `reports_api`
//!
//! `/api/budget` is a read of a computed report. These are writes into the
//! user's journal, and they carry the whole safety apparatus that implies —
//! revisions, a write mutex, a whole-journal re-parse. Nothing about a report
//! endpoint describes that problem. The model here is `alias_api`, which solves
//! the identical one for `alias` directives, and the pipeline below is
//! deliberately the same pipeline.
//!
//! # The write path
//!
//! A journal is the most valuable file this application touches, so the write is
//! the narrowest one that still does the job:
//!
//! 1. **The handle resolves by set membership**, never by path arithmetic — by
//!    exact string equality against [`journals::targets`] over the journal this
//!    server has open. `--` and `..` never get a chance to mean anything because
//!    a handle that is not in the set is a `404`.
//! 2. **The revision is a [`Fingerprint`] over the file's raw bytes**, checked
//!    when the file is read and again immediately before the write, so a journal
//!    edited in vim underneath you is a `409` rather than a silent clobber.
//! 3. **The change is planned, not dictated.** The client asks for one goal to be
//!    a number; [`periodic::plan`] works out which lines have to move, including
//!    a counter-leg the client never mentioned. A client cannot ask for an
//!    unbalanced rule, because it cannot express one.
//! 4. **The rewrite is a span splice** ([`PeriodicDoc::apply`]) that touches only
//!    the amount extents of the lines the plan names.
//! 5. **[`PeriodicDoc::verify`] must agree**, and then the *whole journal* is
//!    re-parsed with the edited text in memory
//!    ([`parse::parse_journal_with_overrides`]) — which is also what proves the
//!    rules still balance, since the parser refuses a rule that does not.
//! 6. **The goal must read back as the number that was asked for.** `verify` is
//!    a text-shape check and deliberately does not parse amounts; this step does,
//!    against the real parse. It is the check that would catch a decimal mark
//!    rendered wrong for a `1.234,56 EUR` journal.
//! 7. **One [`atomic_write`]**, and it is the last statement that can have an
//!    effect. Every `?` above it is a decision to write nothing.
//!
//! # Signs, and why they are flipped here and nowhere else
//!
//! hledger writes income negative. A user budgeting $1,200 of interest income
//! types `1200`, and the journal must say `$-1200`. Whether to invert is a
//! function of the account's *type*, which comes from the journal's `account`
//! declarations and hledger's own inference — a fact neither the engine's editor
//! nor the browser has any business re-deriving. So it is decided once, here, by
//! [`inverted`], and every number crossing this boundary in either direction goes
//! through it. The wire carries both: `amount`, exactly as the file writes it,
//! and `entry`, the magnitude the user sees and types.
//!
//! # No absolute path is ever echoed
//!
//! Same rule as everywhere else in this crate. Errors quote the caller's own
//! `journalId`; a whole-journal parse failure is reported *without* the
//! diagnostic text, because `ParseError::Located` names the file it was reading
//! and that name is not the caller's to have.

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderName, HeaderValue, header};
use axum::response::{IntoResponse, Response};
use ledgeline_core::edit::{Fingerprint, atomic_write};
use ledgeline_core::model::{
    Amount, Commodity, Journal, PeriodExpr, PeriodicTransaction, PostingType,
};
use ledgeline_core::periodic::{
    BlockLock, GoalRequest, PeriodicBlock, PeriodicDoc, PeriodicError, period_word,
};
use ledgeline_core::reports::{
    AccountType, Interval, MixedAmount, ReferenceOpts, account_decls, account_reference,
    declared_types, resolve_account_type,
};
use ledgeline_core::{Dec, journals, parse, periodic};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path as FsPath, PathBuf};

use crate::AppState;
use crate::edit_api::{WireDecIn, dec_from_wire, infer_style, json_body};
use crate::error::{AppError, editing_disabled};
use crate::reports_api::{WireDec, WireMixed, compute, today_utc, wire_mixed};

/// The declared account-type table `resolve_account_type` reads. Named here
/// because it is threaded through every listing and an unnamed
/// `BTreeMap<String, AccountType>` in four signatures says nothing.
type DeclaredTypes = BTreeMap<String, AccountType>;

/// The file a first budget goal is offered a home in, and the `include` line
/// written for it.
///
/// A NAME, chosen by us, for a file that does not exist yet — which is the one
/// situation where `journals.rs`'s "no filename is ever inspected" rule does not
/// apply. That rule is about never *guessing what an existing file is for* from
/// what it is called; naming a file we are about to create is the opposite
/// problem. `budget.journal` is the name hledger's own documentation uses.
const BUDGET_FILE: &str = "budget.journal";

/// Longest accepted `journalId`, in bytes, and how many components it may have.
/// The same numbers `import_api`, `rules_api` and `alias_api` use, for the same
/// reason: a handle longer than the platform's own limit cannot name a file that
/// exists.
const MAX_ID_BYTES: usize = 1024;
/// See [`MAX_ID_BYTES`].
const MAX_ID_COMPONENTS: usize = 9;

/// How many reference periods one request may ask for. Three prior periods plus
/// the running one is the editor's own ask; this is the ceiling on a hostile
/// one, not a limit on the user.
const MAX_REFERENCE_PERIODS: usize = 60;

/// The reference window the editor asks for when it says nothing: four complete
/// periods plus the one now running.
///
/// Four rather than three because a budget is set against a *pattern*, and three
/// points is the fewest from which one can be claimed at all — a fourth is what
/// turns "these two were high" into "this is what it usually is". It is still
/// short enough to read at a glance in one row.
const DEFAULT_REFERENCE_PERIODS: usize = 5;

// ===========================================================================
// Wire types
// ===========================================================================

/// `GET /api/budget/lines` — every budget goal the open journal declares.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireBudgetLines {
    /// `false` means no journal is bound to an editor, so the screen is
    /// read-only and says why.
    editable: bool,
    /// The `journalId` a new goal should go to by default: the file already
    /// holding the most goals, or `None` when there are none anywhere.
    #[serde(skip_serializing_if = "Option::is_none")]
    default_target: Option<String>,
    /// Whether `POST /api/budget/file` would succeed — i.e. this journal has no
    /// goals at all and no `budget.journal` in the way. Drives the "create one"
    /// button, so the UI never offers a button that would 409.
    can_create_file: bool,
    /// The name that button would create, so the UI can say it out loud.
    create_file_name: &'static str,
    /// Every file declaring a `~` rule, in the order the parse read them.
    files: Vec<WireBudgetFile>,
}

/// One journal file's `~` rules.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireBudgetFile {
    /// The file's handle: its path relative to the include root, forward
    /// slashes. Never an absolute path.
    journal_id: String,
    /// The file's own name, for display.
    label: String,
    /// A fingerprint of the file's raw bytes. Echo it back in a `PUT` to prove
    /// the edit is against these bytes.
    revision: String,
    /// A regular file inside the include root; `false` means this file can be
    /// listed but not written.
    writable: bool,
    /// This file's `~` rules, in file order.
    rules: Vec<WireBudgetRule>,
}

/// One `~ PERIODEXPR  [DESCRIPTION]` rule.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireBudgetRule {
    /// 0-based position among this file's `~` rules — the handle an `add` names.
    block: usize,
    /// 1-based line of the `~` header.
    line: u32,
    /// `daily`|`weekly`|`monthly`|`quarterly`|`yearly`, or the raw text when it
    /// is a period Ledgeline does not model (then `locked` is set).
    period: String,
    /// The rule description; `--budget=DESCPAT` matches a substring of it.
    description: String,
    /// The sentence to show when this whole rule is read-only, else absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    locked: Option<&'static str>,
    /// The rule's goal lines, in file order.
    lines: Vec<WireBudgetLine>,
}

/// One goal: an account and an amount.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireBudgetLine {
    /// 0-based position among **this file's** goal lines, across every rule —
    /// the handle a `set`/`remove` names. A scan-time ordinal, not a durable id;
    /// the revision is what makes that safe.
    index: usize,
    /// 1-based file line.
    line: u32,
    /// The account, without any `(…)`/`[…]` wrapper.
    account: String,
    /// Whether the posting is written `(account)`, the unbalanced-virtual form
    /// every hledger budget example uses.
    unbalanced: bool,
    /// The amount exactly as the file writes it, signed as hledger signs it.
    /// Absent when the line has no written amount (the inferred leg).
    #[serde(skip_serializing_if = "Option::is_none")]
    amount: Option<WireMixed>,
    /// The magnitude the user sees and types — the amount, negated when the
    /// account is income-typed. Absent for the same lines `amount` is.
    #[serde(skip_serializing_if = "Option::is_none")]
    entry: Option<WireEntry>,
    /// Whether `entry` is the negation of `amount`, i.e. whether a number sent
    /// back for this line will be negated before it is written.
    inverted: bool,
    /// The sentence to show when this line is read-only, else absent. Set when
    /// EITHER the line or its rule is locked, so a client can drive an edit
    /// affordance off this one field.
    #[serde(skip_serializing_if = "Option::is_none")]
    locked: Option<&'static str>,
}

/// A single-commodity amount as the editor's number box holds it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireEntry {
    /// The commodity symbol, e.g. `$`.
    commodity: String,
    /// The exact quantity.
    value: WireDec,
}

/// `PUT /api/budget/lines/{*journalId}`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WireSaveBudget {
    /// The revision this edit was planned against.
    revision: String,
    /// The one change. Exactly one, deliberately: a change is planned into
    /// however many line rewrites it needs (see [`periodic::plan`]), and batching
    /// two gestures would mean planning each against a file the other has already
    /// moved.
    change: WireChange,
}

/// One change. `kind` is the tag, so an unknown one is a `400` rather than a
/// silently different edit.
///
/// Every `value` is an **entry magnitude** — what the user typed — and is
/// negated on the way in for an income-typed account. See the module docs.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind", deny_unknown_fields)]
pub(crate) enum WireChange {
    /// Set one existing goal's amount.
    Set {
        /// Which line, by [`WireBudgetLine::index`].
        index: usize,
        /// The new magnitude, in the line's own commodity.
        value: WireDecIn,
    },
    /// Remove one existing goal.
    Remove {
        /// Which line, by [`WireBudgetLine::index`].
        index: usize,
    },
    /// Add a goal to an existing rule.
    Add {
        /// Which rule, by [`WireBudgetRule::block`].
        block: usize,
        /// The account, without any wrapper.
        account: String,
        /// The magnitude.
        value: WireDecIn,
        /// The commodity, or absent for the journal's own.
        #[serde(default)]
        commodity: Option<String>,
    },
    /// Add a goal in a new rule, appended at the end of the file.
    AddRule {
        /// `daily`|`weekly`|`monthly`|`quarterly`|`yearly`.
        period: String,
        /// The rule description. May be empty.
        #[serde(default)]
        description: String,
        /// The account, without any wrapper.
        account: String,
        /// The magnitude.
        value: WireDecIn,
        /// The commodity, or absent for the journal's own.
        #[serde(default)]
        commodity: Option<String>,
    },
}

/// `POST /api/budget/file` — what was created.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireCreatedFile {
    /// The new file's handle, ready to be used as a `PUT` target.
    journal_id: String,
    /// The file's own name, for display.
    label: String,
    /// The `include` line that was appended to the main journal, verbatim, so
    /// the UI can show exactly what changed.
    included_as: String,
    /// The main journal's own handle, named because that file changed too.
    main_journal_id: String,
}

/// `?account=&interval=&count=&asOf=` — the history strip beside the amount box.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReferenceQuery {
    /// The account, matched inclusively against itself and its subaccounts.
    account: String,
    /// `daily`|`weekly`|`monthly`|`quarterly`|`yearly`. Defaults to monthly.
    interval: Option<String>,
    /// How many periods, newest last. Defaults to
    /// [`DEFAULT_REFERENCE_PERIODS`].
    count: Option<usize>,
    /// The inclusive "today" the newest period ends at. Defaults to today.
    as_of: Option<String>,
}

/// `GET /api/budget/reference` — one account's recent actuals.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireReference {
    /// Echoed back, so a late response can be matched to its request.
    account: String,
    /// Echoed back.
    interval: String,
    /// Whether these figures are negated relative to the journal — true for an
    /// income-typed account, and the same verdict a goal on this account gets.
    inverted: bool,
    /// The periods, oldest → newest.
    periods: Vec<WirePeriod>,
    /// The mean over the COMPLETE periods, oriented like `periods`. This is the
    /// figure a budget is actually set from; see `reference.rs` for why the
    /// running period is left out of it.
    average: WireMixed,
    /// How many periods `average` covers. **Zero means there is no average** —
    /// a different fact from an average of zero, and the one the UI must not
    /// print a number for.
    averaged_periods: usize,
}

/// One period's actuals.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WirePeriod {
    /// The bucket key, e.g. `2026-08`.
    key: String,
    /// The key rendered for a person, e.g. `Aug 2026`.
    label: String,
    /// Inclusive start.
    start: String,
    /// Inclusive end, clamped to the as-of date.
    end: String,
    /// Whether the period has finished. `false` means "so far".
    complete: bool,
    /// The subaccount-inclusive total, oriented the same way `inverted` says.
    total: WireMixed,
}

// ===========================================================================
// Handlers
// ===========================================================================

/// `Cache-Control: no-store`, no `ETag` — the same posture, and the same
/// reasoning, as the rules, import and alias routes: none of this is derived
/// from the journal snapshot's generation counter.
fn no_store<T: Serialize>(body: T) -> Response {
    const NO_STORE: (HeaderName, HeaderValue) =
        (header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    ([NO_STORE], Json(body)).into_response()
}

/// `GET /api/budget/lines` — every budget goal the open journal declares.
pub(crate) async fn index(State(state): State<AppState>) -> Result<Response, AppError> {
    let editable = state.editing_enabled();
    let Json(body) = compute(move || budget_lines(&state, editable)).await?;
    Ok(no_store(body))
}

/// `PUT /api/budget/lines/{*journalId}` — one change to one file's goals.
pub(crate) async fn save(
    State(state): State<AppState>,
    Path(id): Path<String>,
    payload: Result<Json<WireSaveBudget>, JsonRejection>,
) -> Result<Response, AppError> {
    // Shape first, before any filesystem call, so the route is decided on syntax
    // and is never an existence oracle.
    validate_journal_id(&id)?;
    let request = json_body(payload)?;
    if !state.editing_enabled() {
        return Err(editing_disabled());
    }
    // The journal write mutex, shared with imports and aliases: all three can
    // name the same file. Held across the `.await`, which is why it is a tokio
    // mutex.
    let guard = state.clone();
    let _write = guard.import_writes().lock().await;
    let Json(body) = compute(move || save_budget(&state, &id, &request)).await?;
    Ok(no_store(body))
}

/// `POST /api/budget/file` — create a `budget.journal` and `include` it.
pub(crate) async fn create_file(State(state): State<AppState>) -> Result<Response, AppError> {
    if !state.editing_enabled() {
        return Err(editing_disabled());
    }
    let guard = state.clone();
    let _write = guard.import_writes().lock().await;
    let Json(body) = compute(move || create_budget_file(&state)).await?;
    Ok(no_store(body))
}

/// `GET /api/budget/reference` — one account's recent actuals.
pub(crate) async fn reference(
    State(state): State<AppState>,
    Query(query): Query<ReferenceQuery>,
) -> Result<Response, AppError> {
    let snapshot = state.snapshot();
    let account = query.account.trim().to_string();
    if account.is_empty() {
        return Err(AppError::BadRequest(
            "account is required and may not be empty".to_string(),
        ));
    }
    let interval_text = query.interval.unwrap_or_else(|| "monthly".to_string());
    let interval = parse_interval(&interval_text)?;
    let count = query.count.unwrap_or(DEFAULT_REFERENCE_PERIODS);
    if count > MAX_REFERENCE_PERIODS {
        return Err(AppError::BadRequest(format!(
            "count {count} is out of range (expected 0..={MAX_REFERENCE_PERIODS})"
        )));
    }
    let as_of = match query.as_of {
        Some(raw) => checked_date(&raw)?,
        None => today_utc(),
    };

    let Json(body) = compute(move || {
        let journal = &snapshot.journal;
        let flip = inverted(journal, &account);
        let history = account_reference(
            &journal.transactions,
            &ReferenceOpts {
                account: &account,
                interval,
                count,
                as_of: &as_of,
            },
        )?;
        // Oriented HERE, not in the engine, and through ONE helper for the
        // periods and the average alike — an average shown the opposite way up
        // from the figures it averages would be worse than no average at all.
        let average = oriented(&history.average, flip)?;
        let periods = history
            .periods
            .into_iter()
            .map(|period| {
                // Oriented HERE, not in the engine: the engine reports what the
                // journal says, and which way round to show it is a fact about
                // the account's type. See the module docs.
                let total = oriented(&period.total, flip)?;
                Ok(WirePeriod {
                    key: period.key,
                    label: period.label,
                    start: period.start,
                    end: period.end,
                    complete: period.complete,
                    total: wire_mixed(&total),
                })
            })
            .collect::<Result<Vec<_>, AppError>>()?;
        Ok(WireReference {
            account,
            interval: interval_text,
            inverted: flip,
            average: wire_mixed(&average),
            averaged_periods: history.averaged,
            periods,
        })
    })
    .await?;
    Ok(no_store(body))
}

// ===========================================================================
// Reading
// ===========================================================================

/// Every journal file a budget goal lives in — or could.
///
/// Only these files are read. A journal that splits into forty year-files does
/// not cost forty `read`s per page load — the same economy `alias_api` keeps.
///
/// # Which files, and why not by name
///
/// **Every file that declares a `~` rule.** That is the whole answer for a
/// journal that already has a budget, and it needs no guessing: the parser
/// records which file each rule came from.
///
/// **When no file declares one**, a first goal still needs somewhere to go, so
/// the fallback is *every writable file the parse read that holds no
/// transactions* — and the root journal when there are none of those either,
/// because a goal has to land somewhere.
///
/// A transaction-free file is what `journals::targets` already calls a pure
/// directive file, identified from its CONTENT. That matters: a freshly created
/// `budget.journal` is empty of transactions and so is offered, without this
/// module ever asking what a file is called. `journals.rs` states at length why
/// no filename is ever inspected, and creating a file under a name we chose is
/// not a licence to start recognising it later — someone whose budget file is
/// called `plan.hledger` gets the same behaviour.
///
/// The cost of the fallback being a little wide (an `accounts.journal` is
/// offered too) is that the user picks from a two-item list once. The cost of it
/// being too narrow is that the "create a budget file" button leads nowhere,
/// which is the bug this rule was written to fix.
fn budget_lines(state: &AppState, editable: bool) -> Result<WireBudgetLines, AppError> {
    let snapshot = state.snapshot();
    let journal = &snapshot.journal;
    let targets = journals::targets(journal);
    let declared = declared_types(&account_decls(journal));

    let listed: Vec<&PathBuf> = if journal.periodic_transactions.is_empty() {
        let empty: Vec<&PathBuf> = journal
            .source_files
            .iter()
            .filter(|path| holds_no_transactions(&targets, journal, path))
            .collect();
        if empty.is_empty() {
            journal.source_files.first().into_iter().collect()
        } else {
            empty
        }
    } else {
        journal
            .source_files
            .iter()
            .filter(|path| {
                journal
                    .periodic_transactions
                    .iter()
                    .any(|rule| &&rule.source_file == path)
            })
            .collect()
    };

    let files: Vec<WireBudgetFile> = listed
        .into_iter()
        .filter_map(|path| {
            let target = targets
                .iter()
                .find(|target| journal_path(journal, &target.id).as_ref() == Some(path))?;
            let text = std::fs::read_to_string(path).ok()?;
            let doc = PeriodicDoc::parse(&text);
            let rules = rules_in(journal, path);
            Some(WireBudgetFile {
                rules: wire_rules(&doc, &rules, &declared),
                journal_id: target.id.clone(),
                label: target.label.clone(),
                revision: Fingerprint::of_bytes(text.as_bytes()).token(),
                writable: target.writable,
            })
        })
        .collect();

    // The busiest file wins. Ties — including the all-empty case — go to the LAST
    // one the parse read, which is where a freshly `include`d budget file sits,
    // because the `include` is appended at EOF. Derived from the journal's own
    // structure, never from a filename.
    let default_target = files
        .iter()
        .filter(|file| file.writable)
        .enumerate()
        .max_by_key(|(at, file)| {
            (
                file.rules
                    .iter()
                    .map(|rule| rule.lines.len())
                    .sum::<usize>(),
                *at,
            )
        })
        .map(|(_, file)| file.journal_id.clone());

    Ok(WireBudgetLines {
        editable,
        can_create_file: editable && can_create(journal).is_ok(),
        create_file_name: BUDGET_FILE,
        default_target,
        files,
    })
}

/// Whether `path` is a writable file the parse read that holds no transactions.
///
/// `journals::targets` already tallies both facts from content, so this is a
/// lookup rather than a second scan.
fn holds_no_transactions(
    targets: &[journals::JournalTarget],
    journal: &Journal,
    path: &PathBuf,
) -> bool {
    targets.iter().any(|target| {
        target.txn_count == 0
            && target.writable
            && journal_path(journal, &target.id).as_ref() == Some(path)
    })
}

/// The parsed `~` rules that came from `path`, in file order — the ones whose
/// ordinals line up with a [`PeriodicDoc`] over that file's text.
fn rules_in<'a>(journal: &'a Journal, path: &FsPath) -> Vec<&'a PeriodicTransaction> {
    journal
        .periodic_transactions
        .iter()
        .filter(|rule| rule.source_file == path)
        .collect()
}

/// One file's rules as the wire carries them.
///
/// The scan and the parse are two readings of the same file. They agree by
/// construction (both skip `comment` blocks and consume the same body lines), but
/// a disagreement would mean showing a goal under the wrong rule, so a file whose
/// counts do not line up is reported as a rule-less file rather than guessed at.
fn wire_rules(
    doc: &PeriodicDoc,
    rules: &[&PeriodicTransaction],
    declared: &DeclaredTypes,
) -> Vec<WireBudgetRule> {
    if doc.blocks().len() != rules.len() {
        return Vec::new();
    }
    doc.blocks()
        .iter()
        .zip(rules)
        .map(|(block, rule)| WireBudgetRule {
            block: block.index,
            line: block.line,
            period: block
                .period
                .map_or_else(|| block.period_text.clone(), |p| period_word(p).to_string()),
            description: block.description.clone(),
            locked: block.lock.map(BlockLock::message),
            lines: block
                .lines
                .iter()
                .map(|at| wire_line(doc, *at, rule, declared))
                .collect(),
        })
        .collect()
}

/// One goal line as the wire carries it.
///
/// The amount is taken from the PARSED rule — `rule.postings[line.at]` — not
/// re-read from the text. That correspondence is the same one `periodic::plan`
/// relies on and `wire_rules` has already checked, and it is what lets this
/// module avoid re-implementing the number parser: only the parser knows whether
/// `1.234,56` in this journal is a thousand or a one.
fn wire_line(
    doc: &PeriodicDoc,
    at: usize,
    rule: &PeriodicTransaction,
    declared: &DeclaredTypes,
) -> WireBudgetLine {
    let line = &doc.lines()[at];
    let flip = inverted_with(declared, &line.account);
    // A line with no WRITTEN amount reports none, even though the parser has
    // inferred one for it: the inferred leg is a consequence of the other lines,
    // and showing it in an editable-looking box would invite an edit that is
    // refused. `GoalLock::Inferred` says the same thing in `locked`.
    let written = line
        .amount
        .as_ref()
        .and_then(|_| rule.postings.get(line.at))
        .and_then(|posting| posting.amounts.first());
    WireBudgetLine {
        index: line.index,
        line: line.line,
        account: line.account.clone(),
        unbalanced: line.ptype == PostingType::Virtual,
        amount: written.map(|amount| wire_mixed(&single(amount))),
        entry: written.map(|amount| WireEntry {
            commodity: amount.commodity.0.clone(),
            // `neg` overflows only at the `i128` boundary, which no amount that
            // came out of the parser can be at; the value itself is the honest
            // fallback rather than a panic.
            value: WireDec::from(if flip {
                amount.quantity.neg().unwrap_or(amount.quantity)
            } else {
                amount.quantity
            }),
        }),
        inverted: flip,
        locked: doc.line_lock(line.index),
    }
}

/// One `Amount` as a one-commodity bag, for the wire's `MixedAmount` shape.
fn single(amount: &Amount) -> MixedAmount {
    MixedAmount::single(amount.commodity.clone(), amount.quantity)
}

// ===========================================================================
// Signs
// ===========================================================================

/// Whether goals on `account` are shown negated — i.e. whether it is
/// income-typed.
///
/// Revenue is the only inversion. An expense, an asset and a liability all read
/// naturally with hledger's own sign; only income is written negative while
/// everybody says "I earn twelve hundred".
fn inverted(journal: &Journal, account: &str) -> bool {
    inverted_with(&declared_types(&account_decls(journal)), account)
}

/// `amounts`, negated when the account they describe is income-typed.
///
/// One helper for every figure on the reference strip, so a period and the
/// average of those periods can never disagree about which way up they are.
fn oriented(amounts: &MixedAmount, flip: bool) -> Result<MixedAmount, AppError> {
    if !flip {
        return Ok(amounts.clone());
    }
    amounts.ma_neg().map_err(|error| {
        AppError::Internal(format!("the reference figure is out of range: {error}"))
    })
}

/// [`inverted`], with the declaration table already built. Called once per goal
/// line, so it must not rebuild the table each time.
fn inverted_with(declared: &DeclaredTypes, account: &str) -> bool {
    resolve_account_type(account, declared) == Some(AccountType::Revenue)
}

// ===========================================================================
// Writing
// ===========================================================================

/// The whole of `PUT`, synchronously. Every `?` is a decision not to write.
fn save_budget(
    state: &AppState,
    id: &str,
    request: &WireSaveBudget,
) -> Result<WireBudgetFile, AppError> {
    let snapshot = state.snapshot();
    let journal = &snapshot.journal;
    let target = journals::targets(journal)
        .into_iter()
        .find(|target| target.id == id)
        .ok_or_else(|| unresolved(id))?;
    if !target.writable {
        return Err(AppError::BadRequest(format!(
            "{} cannot be edited: a budget goal can only be written to a regular file inside the \
             journal's own directory, not a symlink or a directory",
            quoted(id)
        )));
    }
    let path = journal_path(journal, id).ok_or_else(|| unresolved(id))?;

    let text = read_journal(&path, id)?;
    let fingerprint = Fingerprint::of_bytes(text.as_bytes());
    // Checked BEFORE any index is resolved, so a client editing an older parse is
    // told the file moved rather than "there is no budget line 3" — which
    // describes the wrong problem and suggests the wrong fix.
    if fingerprint.token() != request.revision {
        return Err(stale(id));
    }

    let doc = PeriodicDoc::parse(&text);
    let rules: Vec<PeriodicTransaction> = rules_in(journal, &path).into_iter().cloned().collect();
    if doc.blocks().len() != rules.len() {
        return Err(stale(id));
    }

    let goal = goal_request(&doc, &rules, journal, &request.change)?;
    let plan = periodic::plan(&doc, &rules, &goal)?;
    let new_text = doc.apply(&plan)?;
    doc.verify(&plan, &new_text)?;

    // The engine proved the goal lines are what was asked for and that nothing
    // else moved. This proves the result is still a journal — and, because the
    // parser balances every `~` rule as it reads it, that every rule in the file
    // still balances. Only this crate knows which journal this file is part of,
    // which is why the check is here.
    if new_text != text {
        let overrides = HashMap::from([(path.clone(), new_text.clone())]);
        let reparsed = parse::parse_journal_with_overrides(&journal.source_name, &overrides)
            .map_err(|_| {
                // Deliberately no detail: `ParseError::Located` names the file it
                // was reading, and that name is not the caller's to have.
                AppError::BadRequest(format!(
                    "this change would make {} unreadable as part of your journal, so nothing was \
                     written",
                    quoted(id)
                ))
            })?;
        confirm_written_goal(&reparsed, &path, &goal, &new_text, id)?;
    }

    if new_text == text {
        // A no-op writes NOTHING. Writing byte-identical content still bumps
        // mtime, and a user's own watch loop would see a spurious change — the
        // same lesson `rules_api` records.
        return Ok(file_response(&target, &doc, fingerprint.token(), journal));
    }

    // Narrow the TOCTOU window from "the whole request" to "hash → rename".
    let before_write = Fingerprint::of_bytes(read_journal(&path, id)?.as_bytes());
    if !before_write.content_matches(&fingerprint) {
        return Err(stale(id));
    }

    atomic_write(&path, new_text.as_bytes()).map_err(|error| {
        // Only the `ErrorKind`: `atomic_write` builds a temp path from the
        // target, so its io errors can carry one.
        AppError::Internal(format!(
            "{} could not be written: {}. Nothing else was changed.",
            quoted(id),
            error.kind()
        ))
    })?;

    // The journal changed underneath the editor, so re-open it — otherwise the
    // next transaction edit sees a stale fingerprint and reports a conflict the
    // user did not cause. Done BEFORE the response is built, for the reason
    // `alias_api` records: the response describes the journal that now exists.
    if let Some(Err(error)) = state.reopen_editor() {
        eprintln!("ledgeline: the journal could not be re-read after a budget edit: {error}");
    }

    // The new revision comes from what we WROTE, never from a re-read: a re-read
    // could pick up somebody else's write and hand this client a token for bytes
    // it has never seen, which is exactly how the next save clobbers that person
    // silently.
    let revision = Fingerprint::of_bytes(new_text.as_bytes()).token();
    Ok(file_response(
        &target,
        &PeriodicDoc::parse(&new_text),
        revision,
        &state.snapshot().journal,
    ))
}

/// Prove the edited journal holds the number that was asked for.
///
/// `PeriodicDoc::verify` is a text-shape check and deliberately does not parse
/// amounts, so this is the step that would catch an amount rendered with the
/// wrong decimal mark for a `1.234,56 EUR` journal — written, read back as a
/// different number, and committed with a `200`. It runs against the real parse
/// of the whole journal, which is the only reading that counts.
fn confirm_written_goal(
    reparsed: &Journal,
    path: &FsPath,
    goal: &GoalRequest,
    new_text: &str,
    id: &str,
) -> Result<(), AppError> {
    let wanted = match goal {
        GoalRequest::Set { quantity, .. } => *quantity,
        GoalRequest::Add { amount, .. } | GoalRequest::AddBlock { amount, .. } => amount.quantity,
        // A removal has no amount to confirm; that it is gone is what the line
        // count in `verify` already established.
        GoalRequest::Remove { .. } => return Ok(()),
    };
    let doc = PeriodicDoc::parse(new_text);
    let rules = rules_in(reparsed, path);
    let account = match goal {
        GoalRequest::Set { index, .. } => doc
            .lines()
            .get(*index)
            .map(|line| line.account.clone())
            .ok_or_else(|| stale(id))?,
        GoalRequest::Add { account, .. } | GoalRequest::AddBlock { account, .. } => account.clone(),
        GoalRequest::Remove { .. } => unreachable!("returned above"),
    };
    let found = rules
        .iter()
        .flat_map(|rule| &rule.postings)
        .filter(|posting| posting.account.0 == account)
        .any(|posting| {
            posting
                .amounts
                .first()
                .is_some_and(|amount| amount.quantity == wanted)
        });
    if found {
        Ok(())
    } else {
        Err(AppError::Internal(format!(
            "the amount written to {} did not read back as the one requested, so nothing was \
             changed",
            quoted(id)
        )))
    }
}

/// The file listing a save answers with.
///
/// Note what it re-reads: nothing. The document is the text we just wrote and the
/// revision is its fingerprint, so the client's next save is against bytes this
/// response describes exactly.
fn file_response(
    target: &journals::JournalTarget,
    doc: &PeriodicDoc,
    revision: String,
    journal: &Journal,
) -> WireBudgetFile {
    let declared = declared_types(&account_decls(journal));
    let path = journal_path(journal, &target.id).unwrap_or_default();
    let rules = rules_in(journal, &path);
    WireBudgetFile {
        rules: wire_rules(doc, &rules, &declared),
        journal_id: target.id.clone(),
        label: target.label.clone(),
        revision,
        writable: target.writable,
    }
}

/// One wire change as the engine's request, with the sign put back the way
/// hledger writes it and the commodity/style resolved.
fn goal_request(
    doc: &PeriodicDoc,
    rules: &[PeriodicTransaction],
    journal: &Journal,
    change: &WireChange,
) -> Result<GoalRequest, AppError> {
    match change {
        WireChange::Set { index, value } => {
            let line = doc
                .lines()
                .get(*index)
                .ok_or_else(|| AppError::BadRequest(format!("there is no budget line {index}")))?;
            Ok(GoalRequest::Set {
                index: *index,
                quantity: signed(journal, &line.account, dec_from_wire(value)?)?,
            })
        }
        WireChange::Remove { index } => Ok(GoalRequest::Remove { index: *index }),
        WireChange::Add {
            block,
            account,
            value,
            commodity,
        } => {
            let account = account.trim().to_string();
            let quantity = signed(journal, &account, dec_from_wire(value)?)?;
            let block_ref = doc
                .blocks()
                .get(*block)
                .ok_or_else(|| AppError::BadRequest(format!("there is no budget rule {block}")))?;
            Ok(GoalRequest::Add {
                block: *block,
                amount: amount_for(
                    journal,
                    rules,
                    Some(block_ref),
                    commodity.as_deref(),
                    quantity,
                ),
                account,
            })
        }
        WireChange::AddRule {
            period,
            description,
            account,
            value,
            commodity,
        } => {
            let account = account.trim().to_string();
            let quantity = signed(journal, &account, dec_from_wire(value)?)?;
            let period = parse_period(period)?;
            let description = description.trim().to_string();
            // The rule this goal will actually land in, when one already states
            // this recurrence under this name — `periodic::plan` joins it rather
            // than opening a second block. The commodity is inferred from THAT
            // rule for the reason `amount_for` gives: a new goal in a EUR rule is
            // a EUR goal, and a `$` one appended beside it would leave a rule
            // whose postings are not all in one commodity, which is a rule
            // neither of us will edit again.
            let joined = doc
                .joinable_block(period, &description)
                .and_then(|block| doc.blocks().get(block));
            Ok(GoalRequest::AddBlock {
                period,
                amount: amount_for(journal, rules, joined, commodity.as_deref(), quantity),
                description,
                account,
            })
        }
    }
}

/// A user-typed magnitude as the journal must write it: negated for an
/// income-typed account. See the module docs.
fn signed(journal: &Journal, account: &str, value: Dec) -> Result<Dec, AppError> {
    if inverted(journal, account) {
        value
            .neg()
            .map_err(|error| AppError::BadRequest(format!("the amount is out of range: {error}")))
    } else {
        Ok(value)
    }
}

/// The full [`Amount`] a new goal is written as: the requested commodity, else
/// the one the rule it is joining already uses, else the journal's own — and a
/// display style inferred from the journal, never invented.
///
/// The style matters more than it looks: rendering a EUR amount with `.` when the
/// journal writes `,` produces a line that reads back as a different number.
/// `edit_api::infer_style` is the transaction editor's own inference, reused here
/// so the two cannot disagree.
fn amount_for(
    journal: &Journal,
    rules: &[PeriodicTransaction],
    block: Option<&PeriodicBlock>,
    requested: Option<&str>,
    quantity: Dec,
) -> Amount {
    let commodity = requested
        .map(str::trim)
        .filter(|symbol| !symbol.is_empty())
        .map(|symbol| Commodity(symbol.to_string()))
        .or_else(|| {
            // The rule being joined: a new grocery goal in a EUR rule is a EUR
            // goal, whatever the rest of the journal is denominated in.
            let block = block?;
            rules
                .get(block.index)?
                .postings
                .iter()
                .find_map(|posting| posting.amounts.first())
                .map(|amount| amount.commodity.clone())
        })
        .unwrap_or_else(|| journal_commodity(journal));
    let style = infer_style(journal, &commodity, quantity.places);
    Amount {
        commodity,
        quantity,
        style,
        cost: None,
    }
}

/// The commodity this journal is denominated in: its `D` default-commodity
/// directive if it has one, else the commodity most of its postings are written
/// in, else `$`.
///
/// The `D` directive is preferred because it is the one place a journal *states*
/// its denomination in the author's own words — the same reasoning
/// `Journal::default_commodity` records. Frequency is the fallback, and `$` is
/// the fallback's fallback for a journal with no postings at all.
fn journal_commodity(journal: &Journal) -> Commodity {
    if let Some(declared) = &journal.default_commodity {
        return declared.clone();
    }
    let mut tally: HashMap<&str, usize> = HashMap::new();
    for posting in journal
        .transactions
        .iter()
        .flat_map(|txn| &txn.postings)
        .chain(
            journal
                .periodic_transactions
                .iter()
                .flat_map(|r| &r.postings),
        )
    {
        for amount in &posting.amounts {
            *tally.entry(amount.commodity.0.as_str()).or_default() += 1;
        }
    }
    // Largest count wins; ties break lexically, so the answer does not depend on
    // hash iteration order.
    tally
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(a.0)))
        .map_or_else(
            || Commodity("$".to_string()),
            |(sym, _)| Commodity(sym.to_string()),
        )
}

// ===========================================================================
// Creating a budget file
// ===========================================================================

/// The contents a new `budget.journal` is created with.
///
/// A comment and nothing else. The file's whole job is to be a place goals go,
/// and writing a speculative example goal into someone's books would be writing
/// a number they did not ask for.
const BUDGET_FILE_HEADER: &str = "\
; Budget goals.
;
; Each `~` rule below states what you plan to spend or earn per period. Ledgeline
; edits this file from the Budget tab; hledger reads it with `bal --budget`.
";

/// Whether a `budget.journal` may be created, and why not when it may not.
///
/// Two refusals, both about not surprising anyone:
///
/// - **The journal already declares `~` rules.** Then it already has a home for
///   goals, and adding a second one would split them across files for no reason
///   the user asked for.
/// - **Something already sits at `budget.journal`.** Never overwritten, never
///   appended to, not even when it is empty. A file we did not create is a file
///   whose contents are somebody's, and the failure mode of guessing wrong here
///   is destroying them.
fn can_create(journal: &Journal) -> Result<(PathBuf, PathBuf), AppError> {
    let main = journal
        .source_files
        .first()
        .ok_or_else(|| AppError::BadRequest("no journal is open".to_string()))?;
    let root = main.parent().ok_or_else(|| {
        AppError::Internal("the open journal has no containing directory".to_string())
    })?;
    if !journal.periodic_transactions.is_empty() {
        return Err(AppError::Conflict(
            "this journal already has budget rules, so a new budget file was not created; add \
             your goal to the file that holds them"
                .to_string(),
        ));
    }
    let budget = root.join(BUDGET_FILE);
    if budget.symlink_metadata().is_ok() {
        return Err(AppError::Conflict(format!(
            "a file called {BUDGET_FILE} already sits beside your journal, and Ledgeline will not \
             write over it. Include it from your main journal yourself, or move it aside."
        )));
    }
    Ok((main.clone(), budget))
}

/// Create `budget.journal` and `include` it from the main journal.
///
/// # Order, which is the whole safety argument
///
/// The new file is written **first**, and the `include` line **second**. An
/// `include` naming a file that is not there is a journal that does not parse —
/// so if the second write fails, the worst outcome is an unreferenced file
/// nobody reads. Done the other way round, a failed second write leaves the
/// user's journal broken.
///
/// Both are proved before either happens: the whole journal is re-parsed with
/// both texts in memory, so a main file that would not survive its own new
/// `include` line is a `400` with nothing written.
fn create_budget_file(state: &AppState) -> Result<WireCreatedFile, AppError> {
    let snapshot = state.snapshot();
    let journal = &snapshot.journal;
    let (main, budget) = can_create(journal)?;

    let main_text = std::fs::read_to_string(&main).map_err(|error| {
        AppError::Internal(format!("your journal could not be read: {}", error.kind()))
    })?;
    let newline = if main_text.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let include_line = format!("include {BUDGET_FILE}");
    // At EOF, and nowhere else: it is the one position provably unable to change
    // the meaning of anything already in the file. `aliases::insertion_point`
    // makes the same argument at length, and an `include` is the directive it
    // matters most for — one placed mid-file changes which directives are in
    // force for everything after it.
    let lead = if main_text.is_empty() || main_text.ends_with('\n') {
        String::new()
    } else {
        newline.to_string()
    };
    let new_main = format!("{main_text}{lead}{include_line}{newline}");

    // Prove BOTH files before writing EITHER.
    let overrides = HashMap::from([
        (main.clone(), new_main.clone()),
        (budget.clone(), BUDGET_FILE_HEADER.to_string()),
    ]);
    if parse::parse_journal_with_overrides(&journal.source_name, &overrides).is_err() {
        return Err(AppError::BadRequest(
            "including a budget file would make your journal unreadable, so nothing was written"
                .to_string(),
        ));
    }

    atomic_write(&budget, BUDGET_FILE_HEADER.as_bytes()).map_err(|error| {
        AppError::Internal(format!(
            "{BUDGET_FILE} could not be created: {}. Nothing else was changed.",
            error.kind()
        ))
    })?;
    atomic_write(&main, new_main.as_bytes()).map_err(|error| {
        AppError::Internal(format!(
            "{BUDGET_FILE} was created, but your main journal could not be updated to include it: \
             {}. Add `{include_line}` to it yourself, or delete {BUDGET_FILE}.",
            error.kind()
        ))
    })?;

    if let Some(Err(error)) = state.reopen_editor() {
        eprintln!(
            "ledgeline: the journal could not be re-read after creating a budget file: {error}"
        );
    }

    let journal = state.snapshot().journal.clone();
    let targets = journals::targets(&journal);
    let main_journal_id = targets
        .iter()
        .find(|target| target.is_root)
        .map_or_else(String::new, |target| target.id.clone());
    Ok(WireCreatedFile {
        journal_id: BUDGET_FILE.to_string(),
        label: BUDGET_FILE.to_string(),
        included_as: include_line,
        main_journal_id,
    })
}

// ===========================================================================
// Handles, parsing and errors
// ===========================================================================

/// The path a `journalId` names, taken from the files the parse actually read.
///
/// Security layer 2, and the same one `alias_api::journal_path` implements:
/// `root.join(id)` appears here, and the very next thing that happens to the
/// result is a membership test against [`Journal::source_files`] — the set of
/// files the include guard already admitted. A handle that does not name one of
/// them resolves to nothing, so no path this function returns was invented by
/// string arithmetic.
fn journal_path(journal: &Journal, id: &str) -> Option<PathBuf> {
    let root = journal.source_files.first()?.parent()?;
    let candidate = root.join(id);
    journal
        .source_files
        .iter()
        .find(|source| *source == &candidate)
        .cloned()
}

/// Layer 1: shape, before any filesystem call.
///
/// The same rules `alias_api::validate_journal_id` applies: no `..`, no leading
/// `/`, no `\`, no `:`, no control character. A hostile handle never reaches the
/// filesystem, and 400-vs-404 is decided on syntax rather than on existence.
fn validate_journal_id(id: &str) -> Result<(), AppError> {
    let refuse = |why: &str| {
        Err(AppError::BadRequest(format!(
            "{} is not a journal id: {why}",
            quoted(id)
        )))
    };
    if id.is_empty() {
        return refuse("it is empty");
    }
    if id.len() > MAX_ID_BYTES {
        return refuse("it is longer than any path this system can hold");
    }
    if id.starts_with('/') || id.contains('\\') || id.contains(':') {
        return refuse("it must be a relative path with forward slashes");
    }
    if id.chars().any(char::is_control) {
        return refuse("it contains a control character");
    }
    let components: Vec<&str> = id.split('/').collect();
    if components.len() > MAX_ID_COMPONENTS {
        return refuse("it has more path components than the journal scan produces");
    }
    if components
        .iter()
        .any(|part| part.is_empty() || *part == "." || *part == "..")
    {
        return refuse("every component must be a plain file or directory name");
    }
    Ok(())
}

/// A period expression, refusing anything Ledgeline does not model rather than
/// writing a rule whose recurrence it cannot state.
fn parse_period(raw: &str) -> Result<PeriodExpr, AppError> {
    match raw.trim() {
        "daily" => Ok(PeriodExpr::Daily),
        "weekly" => Ok(PeriodExpr::Weekly),
        "monthly" => Ok(PeriodExpr::Monthly),
        "quarterly" => Ok(PeriodExpr::Quarterly),
        "yearly" => Ok(PeriodExpr::Yearly),
        other => Err(AppError::BadRequest(format!(
            "unknown budget period '{other}' (expected daily|weekly|monthly|quarterly|yearly)"
        ))),
    }
}

/// A report interval for the reference strip.
fn parse_interval(raw: &str) -> Result<Interval, AppError> {
    match raw.trim() {
        "daily" => Ok(Interval::Daily),
        "weekly" => Ok(Interval::Weekly),
        "monthly" => Ok(Interval::Monthly),
        "quarterly" => Ok(Interval::Quarterly),
        "yearly" => Ok(Interval::Yearly),
        other => Err(AppError::BadRequest(format!(
            "unknown interval '{other}' (expected daily|weekly|monthly|quarterly|yearly)"
        ))),
    }
}

/// An ISO date, or a `400`.
fn checked_date(raw: &str) -> Result<String, AppError> {
    let ok = raw.len() == 10
        && raw.as_bytes()[4] == b'-'
        && raw.as_bytes()[7] == b'-'
        && raw
            .bytes()
            .enumerate()
            .all(|(at, byte)| at == 4 || at == 7 || byte.is_ascii_digit());
    if ok {
        Ok(raw.to_string())
    } else {
        Err(AppError::BadRequest(format!(
            "invalid asOf '{}': expected YYYY-MM-DD",
            quoted(raw)
        )))
    }
}

/// Read one journal file, reporting only what the caller already knows.
fn read_journal(path: &FsPath, id: &str) -> Result<String, AppError> {
    std::fs::read_to_string(path).map_err(|error| {
        AppError::Internal(format!(
            "{} could not be read: {}",
            quoted(id),
            error.kind()
        ))
    })
}

/// A `404` that quotes only the caller's own handle. Every resolution failure
/// returns the same string, so the route is not an existence oracle.
fn unresolved(id: &str) -> AppError {
    AppError::NotFound(format!("no journal file called {}", quoted(id)))
}

/// The one `409`. One message for every staleness check, deliberately: which
/// check tripped is a function of *when* somebody else wrote, and that is not
/// something to leak.
fn stale(id: &str) -> AppError {
    AppError::Conflict(format!(
        "{} changed on disk since you opened it, so nothing was written. Reload it and re-apply \
         your edit.",
        quoted(id)
    ))
}

/// A caller-supplied handle, escaped and clipped for an error body.
fn quoted(value: &str) -> String {
    /// Long enough to recognise your own id, short enough that a hostile one
    /// cannot make a large response.
    const MAX_CHARS: usize = 120;
    let clipped: String = value.chars().take(MAX_CHARS).collect();
    let ellipsis = if clipped.chars().count() < value.chars().count() {
        "…"
    } else {
        ""
    };
    format!("{clipped:?}{ellipsis}")
}

impl From<PeriodicError> for AppError {
    /// Every periodic error is the caller's: a stale index, a locked construct,
    /// an ambiguous counter-leg, or a value this module will not write. The one
    /// exception is [`PeriodicError::RoundTripMismatch`], which is **ours** —
    /// given `apply`'s own output the only way to reach it is a bug in the engine
    /// — so it is a `500`.
    fn from(error: PeriodicError) -> Self {
        match error {
            PeriodicError::RoundTripMismatch => Self::Internal(error.to_string()),
            other => Self::BadRequest(other.to_string()),
        }
    }
}
