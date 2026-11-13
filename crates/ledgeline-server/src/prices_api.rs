//! The HTTP surface for **stock price updates** — the Holdings tab's "Update
//! prices" button. A Rust port of the user's own
//! `update-prices.sh`/`get-historical-prices.sh` scripts, generalized so it
//! works with any hledger journal rather than one specific file layout.
//!
//! Three routes:
//!
//! - `GET /api/prices/status` — which currently-held symbols need a quote,
//!   which journal file(s) already hold `P` directives, whether a first
//!   `prices.journal` can be created, and what the automatic updater
//!   (`auto_prices`, plan 41) last did.
//! - `POST /api/prices/file` — create a `prices.journal` and `include` it, for
//!   a journal with nowhere yet to put a price.
//! - `POST /api/prices/update` — fetch every currently-held symbol's latest
//!   close from Yahoo Finance and append it to the named file.
//!
//! # Why this is its own route family and not part of `reports_api`
//!
//! `/api/prices/status` is a read of a computed report, same as everything in
//! `reports_api`. The other two are writes into the user's journal, carrying
//! the same safety apparatus that implies: `journals::targets`-resolved
//! handles, a `Fingerprint` and identity pin (a vault `Revision`) closing the
//! write's TOCTOU window, one `Vault::replace` (on the desktop
//! `pinned::replace`), a reparse-and-verify before it. The model is `budget_api`,
//! which solves the identical problem for `~` budget rules, and the pipeline
//! below is deliberately the same pipeline.
//!
//! # What's new here versus every other write path in this crate
//!
//! Every other write (`budget_api`, `alias_api`, `import_api`, `rules_api`)
//! turns a client's REQUEST into a journal edit. This one turns a NETWORK
//! response into one — the only place this codebase makes an outbound HTTP
//! call. `POST /api/prices/update` is therefore three phases, not one:
//!
//! 1. **Plan** (blocking pool, same as every other report/write handler here):
//!    resolve the target file, read it, and work out which symbols are
//!    currently held (via `ledgeline_core::holdings::compute_holdings`, the
//!    same engine the Stocks tab itself renders from) and what Yahoo ticker
//!    each one maps to (the journal's `; yahoo:TICKER` commodity tag, else the
//!    hledger symbol itself).
//! 2. **Fetch** (native async, NOT on the blocking pool — `compute`'s
//!    `spawn_blocking` closure cannot `.await`): one bounded-concurrency fan-out
//!    over [`crate::yahoo::PriceFeed`].
//! 3. **Apply** (blocking pool again): fold the fetched quotes into the file's
//!    text, skipping any symbol that already has a `P` line for the fetched
//!    date, reparse-and-verify, and write.
//!
//! The write mutex ([`AppState::import_writes`]) is held across all three
//! phases — including the `.await` in phase 2 — which is exactly what makes it
//! a `tokio::sync::Mutex` rather than a `std::sync::Mutex`, same reasoning as
//! `AppState`'s own doc comment gives for that field.
//!
//! The automatic updater (`auto_prices`) writes through the same
//! [`plan_update`] and [`apply_update`], which is why phase 3 takes a LIST of
//! closes per symbol: the button fetches one (the latest), the updater may
//! bring back the latest plus a close for each week it backfills.
//!
//! # No absolute path is ever echoed
//!
//! Same rule as everywhere else in this crate. Errors quote the caller's own
//! `journalId`; a whole-journal parse failure is reported without the
//! diagnostic text, because `ParseError::Located` names the file it was
//! reading and that name is not the caller's to have.

use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::response::Response;
use futures::stream::{self, StreamExt};
use ledgeline_core::edit::render_amount;
use ledgeline_core::holdings::{HoldingsScope, ScopeMode, compute_holdings};
use ledgeline_core::model::{Amount, Commodity, Journal};
use ledgeline_core::vault::{Revision, VaultError, VaultPath};
use ledgeline_core::{Dec, journals};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::path::Path as FsPath;
use std::sync::Arc;

use crate::AppState;
use crate::auto_prices::WireAutoStatus;
use crate::edit_api::{infer_style, json_body};
use crate::error::{AppError, editing_disabled};
use crate::location::JournalLocation;
use crate::reports_api::{WireDec, compute, today_utc};
use crate::security::no_store_json as no_store;
use crate::yahoo::{FetchedPrice, YahooError};

/// The file a first price update is offered a home in, and the `include` line
/// written for it.
const PRICES_FILE: &str = "prices.journal";

/// How many symbols are fetched from Yahoo Finance at once. Bounded so a large
/// portfolio does not open dozens of simultaneous connections; small enough
/// that a typical portfolio (a few dozen symbols at most) still finishes in
/// one or two round trips' worth of wall-clock time.
const FETCH_CONCURRENCY: usize = 5;

/// Longest accepted `journalId`, in bytes, and how many components it may
/// have. The same numbers `budget_api`, `import_api`, `rules_api` and
/// `alias_api` use, for the same reason: a handle longer than the platform's
/// own limit cannot name a file that exists.
const MAX_ID_BYTES: usize = 1024;
/// See [`MAX_ID_BYTES`].
const MAX_ID_COMPONENTS: usize = 9;

/// A ceiling on a freshly-fetched quote's own precision, applied BEFORE
/// [`infer_style`] ever sees it.
///
/// Yahoo's chart JSON encodes a close as an `f64`, and printing one back out
/// (`FetchedPrice` is built from it in `yahoo.rs`) routinely yields something
/// like `366.8500061035` for what is, to every human involved, `366.85` — a
/// binary-float artifact, not ten meaningful digits. Without this cap, a
/// COMMODITY WITH NO ESTABLISHED STYLE YET (nothing else in the journal prices
/// it) would carry that noise straight into `infer_style`'s fallback
/// `default_style(commodity, places)`, whose `precision` is exactly the
/// `places` it was handed — six is generous for anything Yahoo prices
/// (mutual-fund NAVs included) while discarding the float tail. The SECOND,
/// tighter round in [`apply_update`] — to the quote commodity's actual
/// established style, when the journal has one — is what turns this into the
/// clean two-decimal price a person would write.
const MAX_FETCHED_QUOTE_PLACES: u32 = 6;

// ===========================================================================
// Wire types
// ===========================================================================

/// `GET /api/prices/status`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WirePricesStatus {
    /// `false` means no journal is bound to an editor, so the button is
    /// disabled and the page says why.
    editable: bool,
    /// The commodity fetched prices are recorded in — the same base the
    /// Stocks tab itself values everything in.
    quote_commodity: String,
    /// Every currently-held symbol a quote will be fetched for.
    symbols: Vec<WirePriceSymbol>,
    /// The `journalId` an update should target by default: the file already
    /// holding the most `P` directives, or `None` when there are none anywhere.
    #[serde(skip_serializing_if = "Option::is_none")]
    default_target: Option<String>,
    /// Whether `POST /api/prices/file` would succeed — drives the "create
    /// prices.journal" affordance, so the UI never offers a button that would
    /// `409`.
    can_create_file: bool,
    /// The name that button would create, so the UI can say it out loud.
    create_file_name: &'static str,
    /// Every candidate file an update could target, best first.
    files: Vec<WirePricesFile>,
    /// The automatic updater: whether it is on for this ledger, and what it
    /// last did — for the quiet "Prices auto-updated 2 h ago" line.
    auto: WireAutoStatus,
}

/// One symbol that will be priced.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WirePriceSymbol {
    /// The hledger commodity symbol.
    pub(crate) symbol: String,
    /// The ticker it will be looked up as on Yahoo Finance: the commodity's
    /// `yahoo:` tag, else the symbol itself.
    pub(crate) yahoo_ticker: String,
}

/// One candidate (or, in the update response, the actual) target file.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WirePricesFile {
    /// The file's handle: its path relative to the include root, forward
    /// slashes. Never an absolute path.
    journal_id: String,
    /// The file's own name, for display.
    label: String,
    /// A regular file inside the include root; `false` means this file can be
    /// listed but not written to.
    writable: bool,
    /// How many `P` directives this file currently holds.
    price_count: usize,
}

/// `POST /api/prices/file` — what was created.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireCreatedPricesFile {
    /// The new file's handle, ready to be used as an update target.
    journal_id: String,
    /// The file's own name, for display.
    label: String,
    /// The `include` line that was appended to the main journal, verbatim, so
    /// the UI can show exactly what changed.
    included_as: String,
    /// The main journal's own handle, named because that file changed too.
    main_journal_id: String,
}

/// `POST /api/prices/update`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WireUpdateRequest {
    /// Which file to append fetched prices to, from [`WirePricesStatus`]'s
    /// `defaultTarget`/`files`.
    journal_id: String,
}

/// `POST /api/prices/update` — what happened, file and per-symbol.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireUpdateResponse {
    file: WirePricesFile,
    pub(crate) results: Vec<WirePriceResult>,
}

/// One symbol's outcome — the structured equivalent of what the bash scripts
/// printed to stderr per symbol.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WirePriceResult {
    symbol: String,
    yahoo_ticker: String,
    pub(crate) outcome: Outcome,
    /// The fetched date, present for `updated` and `duplicate`.
    #[serde(skip_serializing_if = "Option::is_none")]
    date: Option<String>,
    /// The fetched price, present for `updated` and `duplicate`.
    #[serde(skip_serializing_if = "Option::is_none")]
    price: Option<WireDec>,
}

/// What happened to one symbol.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Outcome {
    /// A new `P` line was fetched and appended.
    Updated,
    /// A `P` line for the fetched date already existed; nothing was written.
    Duplicate,
    /// Yahoo Finance had no usable quote for this ticker.
    NotFound,
    /// The request to Yahoo Finance itself failed (network, decode, shape).
    FetchError,
}

// ===========================================================================
// Handlers
// ===========================================================================

/// `GET /api/prices/status`.
pub(crate) async fn status(State(state): State<AppState>) -> Result<Response, AppError> {
    let editable = state.editing_enabled();
    let Json(body) = compute(move || prices_status(&state, editable)).await?;
    Ok(no_store(body))
}

/// `POST /api/prices/file` — create `prices.journal` and `include` it.
pub(crate) async fn create_file(State(state): State<AppState>) -> Result<Response, AppError> {
    if !state.editing_enabled() {
        return Err(editing_disabled());
    }
    let guard = state.clone();
    let _write = guard.import_writes().lock().await;
    let Json(body) = compute(move || create_prices_file(&state)).await?;
    Ok(no_store(body))
}

/// `POST /api/prices/update` — fetch and append. See the module docs for why
/// this is three phases rather than one `compute()` call.
pub(crate) async fn update(
    State(state): State<AppState>,
    payload: Result<Json<WireUpdateRequest>, JsonRejection>,
) -> Result<Response, AppError> {
    let request = json_body(payload)?;
    validate_journal_id(&request.journal_id)?;
    if !state.editing_enabled() {
        return Err(editing_disabled());
    }
    // The journal write mutex, shared with budget/import/alias writes: all can
    // name the same file. Held across the fetch phase's `.await`, which is why
    // it is a tokio mutex.
    let guard = state.clone();
    let _write = guard.import_writes().lock().await;

    let plan_state = state.clone();
    let id = request.journal_id.clone();
    let Json(plan) = compute(move || plan_update(&plan_state, &id)).await?;

    let source = Arc::clone(state.price_source());
    let fetched: Vec<FetchOutcome> = stream::iter(plan.symbols.clone())
        .map(|symbol| {
            let source = Arc::clone(&source);
            let as_of = plan.as_of.clone();
            async move {
                let result = source
                    .latest_close(&symbol.yahoo_ticker, &as_of)
                    .await
                    .map(|latest| latest.into_iter().collect());
                FetchOutcome { symbol, result }
            }
        })
        .buffer_unordered(FETCH_CONCURRENCY)
        .collect()
        .await;

    let write_state = state.clone();
    let Json(body) = compute(move || apply_update(&write_state, plan, fetched)).await?;
    Ok(no_store(body))
}

// ===========================================================================
// Reading: which symbols need a quote, and where prices already live
// ===========================================================================

/// `GET /api/prices/status`'s whole body.
fn prices_status(state: &AppState, editable: bool) -> Result<WirePricesStatus, AppError> {
    let snapshot = state.snapshot();
    let journal = &snapshot.journal;
    let as_of = today_utc();
    let (symbols, quote) = symbols_to_price(journal, &as_of)?;

    // A journal with no backing file has no files to offer.
    let location = snapshot.storage();
    let tally = price_counts(journal);
    let files: Vec<WirePricesFile> = location
        .as_ref()
        .map(|location| {
            candidate_targets(location, journal)
                .iter()
                .map(|target| wire_file(location, journal, target, &tally))
                .collect()
        })
        .unwrap_or_default();
    let default_target = files
        .iter()
        .find(|file| file.writable)
        .map(|file| file.journal_id.clone());
    let auto = state
        .auto_prices()
        .wire_status(crate::ui_prefs::auto_price_prefs(state).enabled);

    Ok(WirePricesStatus {
        editable,
        quote_commodity: quote.0,
        symbols,
        default_target,
        can_create_file: editable
            && location
                .as_ref()
                .is_some_and(|location| can_create(location, journal).is_ok()),
        create_file_name: PRICES_FILE,
        files,
        auto,
    })
}

/// Every currently-held stock symbol (from the same engine the Stocks tab
/// renders from) with its resolved Yahoo ticker, plus the base commodity to
/// record fetched prices in.
///
/// Deliberately unscoped and always "as of today" — a price update is a
/// journal-wide fact, not something an account filter or a historical as-of
/// date should narrow.
fn symbols_to_price(
    journal: &Journal,
    as_of: &str,
) -> Result<(Vec<WirePriceSymbol>, Commodity), AppError> {
    let scope = HoldingsScope {
        accounts: BTreeSet::new(),
        mode: ScopeMode::Include,
        as_of: as_of.to_string(),
        gain_since: None,
        value_in: None,
    };
    let report = compute_holdings(
        &journal.transactions,
        &journal.prices,
        &journal.accounts,
        &journal.commodity_tags,
        &scope,
    )?;
    let quote = Commodity(report.base.clone());
    let symbols = report
        .holdings
        .iter()
        .map(|holding| WirePriceSymbol {
            yahoo_ticker: yahoo_ticker(&journal.commodity_tags, &holding.symbol),
            symbol: holding.symbol.clone(),
        })
        .collect();
    Ok((symbols, quote))
}

/// The Yahoo ticker for `symbol`: its commodity `yahoo:` tag, else the symbol
/// itself. Mirrors `holdings::engine`'s own `commodity_name_map` (built for the
/// `name:` tag) — the same shape, a different tag key. `profiles_api` looks
/// commodities up on Yahoo under the same mapping, so a ticker is resolved one
/// way everywhere.
pub(crate) fn yahoo_ticker(
    commodity_tags: &[(Commodity, Vec<(String, String)>)],
    symbol: &str,
) -> String {
    commodity_tags
        .iter()
        .find(|(commodity, _)| commodity.0 == symbol)
        .and_then(|(_, tags)| tags.iter().find(|(key, _)| key == "yahoo"))
        .map(|(_, value)| value.clone())
        .unwrap_or_else(|| symbol.to_string())
}

/// How many `P` directives each source file holds, keyed by resolved path.
fn price_counts(journal: &Journal) -> HashMap<&FsPath, usize> {
    journal
        .prices
        .iter()
        .fold(HashMap::new(), |mut tally, price| {
            *tally.entry(price.source_file.as_path()).or_insert(0) += 1;
            tally
        })
}

fn wire_file(
    location: &JournalLocation,
    journal: &Journal,
    target: &journals::JournalTarget,
    tally: &HashMap<&FsPath, usize>,
) -> WirePricesFile {
    let price_count = journal_path(location, journal, &target.id)
        .and_then(|path| tally.get(location.key(&path).as_path()).copied())
        .unwrap_or(0);
    WirePricesFile {
        journal_id: target.id.clone(),
        label: target.label.clone(),
        writable: target.writable,
        price_count,
    }
}

/// Every journal file an update could target, best first.
///
/// **Every file that already holds a `P` directive**, ranked by how many —
/// that is the whole answer for a journal that already prices things, and it
/// needs no guessing: [`PriceDirective::source_file`] records which file each
/// one came from.
///
/// **When none does**, the fallback is the same one `budget_api::budget_lines`
/// uses for its first goal: every writable file the parse read that holds no
/// transactions, else the root journal — a pure directive file is offered
/// (a freshly-created `prices.journal` included it), and a genuinely-empty
/// journal still has somewhere for its first price to go.
///
/// [`PriceDirective::source_file`]: ledgeline_core::model::PriceDirective::source_file
fn candidate_targets(
    location: &JournalLocation,
    journal: &Journal,
) -> Vec<journals::JournalTarget> {
    let targets = location.targets(journal);
    let tally = price_counts(journal);
    let counted: Vec<(journals::JournalTarget, usize)> = targets
        .iter()
        .cloned()
        .map(|target| {
            let count = journal_path(location, journal, &target.id)
                .and_then(|path| tally.get(location.key(&path).as_path()).copied())
                .unwrap_or(0);
            (target, count)
        })
        .collect();

    let mut priced: Vec<(journals::JournalTarget, usize)> = counted
        .into_iter()
        .filter(|(_, count)| *count > 0)
        .collect();
    if !priced.is_empty() {
        // Stable sort desc by price count; `targets()` already handed these to
        // us in a deterministic order, so ties keep it.
        priced.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
        return priced.into_iter().map(|(target, _)| target).collect();
    }

    let empty: Vec<journals::JournalTarget> = targets
        .iter()
        .filter(|target| target.txn_count == 0 && target.writable)
        .cloned()
        .collect();
    if !empty.is_empty() {
        return empty;
    }
    targets
        .into_iter()
        .filter(|target| target.is_root)
        .collect()
}

/// The file the automatic updater appends to: the same default the button
/// offers (the writable file holding the most `P` directives), but ONLY when
/// some file already holds prices.
///
/// `None` is the "no prices file yet" case, which the updater answers by doing
/// nothing. Where a journal's first price goes is a decision about the user's
/// files — the button asks before creating `prices.journal`, or appends to the
/// root journal only when the user clicks — and a background task must never
/// make it for them. Once a single `P` line exists anywhere, the user has
/// already made it.
pub(crate) fn auto_target(location: &JournalLocation, journal: &Journal) -> Option<String> {
    if journal.prices.is_empty() {
        return None;
    }
    candidate_targets(location, journal)
        .into_iter()
        .find(|target| target.writable)
        .map(|target| target.id)
}

// ===========================================================================
// Creating a prices file
// ===========================================================================

/// The contents a new `prices.journal` is created with. A comment and nothing
/// else — the file's whole job is to be a place prices go, and writing a
/// speculative example price would be writing a number the user did not ask
/// for (same reasoning `budget_api::BUDGET_FILE_HEADER` gives).
const PRICES_FILE_HEADER: &str = "\
; Market prices.
;
; Each `P` line below records one commodity's price on a date. Ledgeline
; writes to this file from the Holdings tab's \"Update prices\" button; hledger
; reads it for valuation.
";

/// Whether a `prices.journal` may be created, and why not when it may not.
/// Mirrors `budget_api::can_create` exactly, keyed on `journal.prices` instead
/// of `journal.periodic_transactions`.
fn can_create(
    location: &JournalLocation,
    journal: &Journal,
) -> Result<(VaultPath, VaultPath), AppError> {
    let main = journal
        .source_files
        .first()
        .and_then(|key| location.path_of(key))
        .ok_or_else(|| AppError::BadRequest("no journal is open".to_string()))?;
    let root = main.parent().ok_or_else(|| {
        AppError::Internal("the open journal has no containing directory".to_string())
    })?;
    if !journal.prices.is_empty() {
        return Err(AppError::Conflict(
            "this journal already has price directives, so a new prices file was not created; \
             update the file that holds them"
                .to_string(),
        ));
    }
    let prices_file = root
        .child(PRICES_FILE)
        .map_err(|_| AppError::Internal(format!("{PRICES_FILE} is not a valid file name")))?;
    // `stat` never follows a link: a dangling symlink at the name is taken.
    if !matches!(location.vault().stat(&prices_file), Ok(None)) {
        return Err(already_exists());
    }
    Ok((main.clone(), prices_file))
}

/// The `409` for a name that is already taken — by the early check above, or by
/// the kernel's `O_EXCL` at the create itself.
fn already_exists() -> AppError {
    AppError::Conflict(format!(
        "a file called {PRICES_FILE} already sits beside your journal, and Ledgeline will not \
         write over it. Include it from your main journal yourself, or move it aside."
    ))
}

/// Create `prices.journal` and `include` it from the main journal. Same
/// write-then-include ordering and same before-either-write proof as
/// `budget_api::create_budget_file` — see its doc comment for the safety
/// argument, which applies here unchanged.
fn create_prices_file(state: &AppState) -> Result<WireCreatedPricesFile, AppError> {
    let snapshot = state.snapshot();
    let journal = &snapshot.journal;
    let location = snapshot
        .storage()
        .ok_or_else(|| AppError::BadRequest("no journal is open".to_string()))?;
    let (main, prices_file) = can_create(&location, journal)?;

    // Pinned, so the `include` below lands only on the main journal as read
    // here — never through a link swapped in since, never over an edit made in
    // between.
    let (main_text, main_revision) = read_pinned_text(&location, &main).map_err(|error| {
        AppError::unless_not_downloaded(
            &error,
            AppError::Internal(format!("your journal could not be read: {}", error.kind())),
        )
    })?;
    let newline = if main_text.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let include_line = format!("include {PRICES_FILE}");
    let lead = if main_text.is_empty() || main_text.ends_with('\n') {
        String::new()
    } else {
        newline.to_string()
    };
    let new_main = format!("{main_text}{lead}{include_line}{newline}");

    let overrides = HashMap::from([
        (main.clone(), new_main.clone()),
        (prices_file.clone(), PRICES_FILE_HEADER.to_string()),
    ]);
    if location.reparse(&overrides).is_err() {
        return Err(AppError::BadRequest(
            "including a prices file would make your journal unreadable, so nothing was written"
                .to_string(),
        ));
    }

    // `Vault::create_new` (on the desktop `O_CREAT | O_EXCL | O_NOFOLLOW`): the
    // "never overwritten" promise `can_create` checked is decided at the create
    // itself, so a file or a symlink planted since that check is refused rather
    // than written through.
    location
        .vault()
        .create_new(&prices_file, PRICES_FILE_HEADER.as_bytes())
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                return already_exists();
            }
            AppError::Internal(format!(
                "{PRICES_FILE} could not be created: {}. Nothing else was changed.",
                error.kind()
            ))
        })?;
    location
        .vault()
        .replace(&main, new_main.as_bytes(), &main_revision)
        .map_err(|error| {
            let reason = match error {
                VaultError::Changed { .. } => {
                    "it changed on disk while this was being done".to_string()
                }
                error => error.kind().to_string(),
            };
            AppError::Internal(format!(
                "{PRICES_FILE} was created, but your main journal could not be updated to \
                 include it: {}. Add `{include_line}` to it yourself, or delete {PRICES_FILE}.",
                reason
            ))
        })?;

    if let Some(Err(error)) = state.reopen_editor() {
        eprintln!(
            "ledgeline: the journal could not be re-read after creating a prices file: {error}"
        );
    }

    let after = state.snapshot();
    let targets = after
        .storage()
        .map(|location| location.targets(&after.journal))
        .unwrap_or_default();
    let main_journal_id = targets
        .iter()
        .find(|target| target.is_root)
        .map_or_else(String::new, |target| target.id.clone());
    Ok(WireCreatedPricesFile {
        journal_id: PRICES_FILE.to_string(),
        label: PRICES_FILE.to_string(),
        included_as: include_line,
        main_journal_id,
    })
}

// ===========================================================================
// Updating: plan (blocking) -> fetch (async) -> apply (blocking)
// ===========================================================================

/// Everything phase 2 (the async fetch) and phase 3 (the write) need, computed
/// once against a single snapshot so a concurrent edit mid-fetch is caught by
/// the fingerprint recheck in [`apply_update`] rather than silently mixed in.
pub(crate) struct UpdatePlan {
    id: String,
    /// The vault `path` is in — the plan's own, so the write after the fetch
    /// lands in the storage the read came from.
    location: Arc<JournalLocation>,
    path: VaultPath,
    text: String,
    /// The identity and bytes `text` was read as. The write, after the network
    /// round trip, lands only if the file is still exactly that.
    revision: Revision,
    as_of: String,
    quote: Commodity,
    symbols: Vec<WirePriceSymbol>,
}

/// Phase 1: resolve the target file, read it, and work out what to fetch.
pub(crate) fn plan_update(state: &AppState, id: &str) -> Result<UpdatePlan, AppError> {
    let snapshot = state.snapshot();
    let journal = &snapshot.journal;
    let location = snapshot.storage().ok_or_else(|| unresolved(id))?;
    let target = location
        .targets(journal)
        .into_iter()
        .find(|target| target.id == id)
        .ok_or_else(|| unresolved(id))?;
    if !target.writable {
        return Err(AppError::BadRequest(format!(
            "{} cannot be updated: prices can only be written to a regular file inside the \
             journal's own directory, not a symlink or a directory",
            quoted(id)
        )));
    }
    let path = journal_path(&location, journal, id).ok_or_else(|| unresolved(id))?;
    let (text, revision) = read_journal(&location, &path, id)?;
    let as_of = today_utc();
    let (symbols, quote) = symbols_to_price(journal, &as_of)?;

    Ok(UpdatePlan {
        id: id.to_string(),
        location,
        path,
        text,
        revision,
        as_of,
        quote,
        symbols,
    })
}

/// One symbol's fetch, still unresolved into wire outcomes: every close to
/// record for it, ascending. Empty is "found nothing usable" (`not-found`).
pub(crate) struct FetchOutcome {
    pub(crate) symbol: WirePriceSymbol,
    pub(crate) result: Result<Vec<FetchedPrice>, YahooError>,
}

/// Phase 3: fold the fetched quotes into `plan`'s file and write it (if
/// anything actually changed). One [`WirePriceResult`] per close, or one per
/// symbol that brought none back.
///
/// New lines go into the file in date order among its existing `P` lines
/// ([`insert_price_lines`]), so a backfill lands where a person reading the
/// file would look for it rather than after this morning's close.
pub(crate) fn apply_update(
    state: &AppState,
    plan: UpdatePlan,
    fetched: Vec<FetchOutcome>,
) -> Result<WireUpdateResponse, AppError> {
    let snapshot = state.snapshot();
    let journal = &snapshot.journal;

    let mut results: Vec<WirePriceResult> = Vec::with_capacity(fetched.len());
    // Keyed `(date, symbol)` for the append order.
    let mut new_lines: Vec<((String, String), String)> = Vec::new();

    for FetchOutcome { symbol, result } in fetched {
        let prices = match result {
            Err(_error) => {
                results.push(no_price(symbol, Outcome::FetchError));
                continue;
            }
            Ok(prices) if prices.is_empty() => {
                results.push(no_price(symbol, Outcome::NotFound));
                continue;
            }
            Ok(prices) => prices,
        };
        for price in prices {
            // See `MAX_FETCHED_QUOTE_PLACES`: bound the raw quote BEFORE it
            // can influence a fallback style, then round again to whatever
            // precision the journal actually established for this commodity
            // (a no-op when that is already <= the bound).
            //
            // Rounded ABOVE the duplicate branch, not inside the write branch:
            // both outcomes report `price` on the wire, and a `duplicate`
            // echoing the raw `f64` tail while an `updated` echoed `228.00`
            // would be two different answers to one question.
            let bounded = round_to_places(price.quantity, MAX_FETCHED_QUOTE_PLACES);
            let style = infer_style(journal, &plan.quote, bounded.places);
            let quantity = round_to_places(bounded, style.precision);
            let outcome = if line_exists(&plan.text, &price.date, &symbol.symbol) {
                Outcome::Duplicate
            } else {
                let amount = Amount {
                    commodity: plan.quote.clone(),
                    quantity,
                    style,
                    cost: None,
                };
                let line = format!(
                    "P {} {} {}",
                    price.date,
                    symbol.symbol,
                    render_amount(&amount)
                );
                new_lines.push(((price.date.clone(), symbol.symbol.clone()), line));
                Outcome::Updated
            };
            results.push(WirePriceResult {
                symbol: symbol.symbol.clone(),
                yahoo_ticker: symbol.yahoo_ticker.clone(),
                outcome,
                date: Some(price.date),
                price: Some(WireDec::from(quantity)),
            });
        }
    }

    new_lines.sort_by(|a, b| a.0.cmp(&b.0));
    let new_text = if new_lines.is_empty() {
        plan.text.clone()
    } else {
        insert_price_lines(
            &plan.text,
            new_lines
                .iter()
                .map(|((date, _), line)| (date.as_str(), line.as_str())),
        )
    };

    if new_text != plan.text {
        let overrides = HashMap::from([(plan.path.clone(), new_text.clone())]);
        plan.location.reparse(&overrides).map_err(|_| {
            AppError::BadRequest(format!(
                "the fetched prices could not be added to {}: doing so would make it unreadable as \
                 part of your journal, so nothing was written",
                quoted(&plan.id)
            ))
        })?;

        // Pinned to the identity and bytes read before the fetch:
        // `Vault::replace` (on the desktop `pinned::replace`) re-checks both
        // through the descriptor it replaces, right up to the rename, and never
        // follows a link.
        plan.location
            .vault()
            .replace(&plan.path, new_text.as_bytes(), &plan.revision)
            .map_err(|error| match error {
                VaultError::Changed { .. } => stale(&plan.id),
                error => AppError::Internal(format!(
                    "{} could not be written: {}. Nothing else was changed.",
                    quoted(&plan.id),
                    error.kind()
                )),
            })?;

        if let Some(Err(error)) = state.reopen_editor() {
            eprintln!("ledgeline: the journal could not be re-read after a price update: {error}");
        }
    }

    let after = state.snapshot();
    let tally = price_counts(&after.journal);
    let location = after.storage().ok_or_else(|| unresolved(&plan.id))?;
    let target = location
        .targets(&after.journal)
        .into_iter()
        .find(|target| target.id == plan.id)
        .ok_or_else(|| unresolved(&plan.id))?;
    let file = wire_file(&location, &after.journal, &target, &tally);
    Ok(WireUpdateResponse { file, results })
}

/// A symbol's result when it brought back no close at all.
fn no_price(symbol: WirePriceSymbol, outcome: Outcome) -> WirePriceResult {
    WirePriceResult {
        symbol: symbol.symbol,
        yahoo_ticker: symbol.yahoo_ticker,
        outcome,
        date: None,
        price: None,
    }
}

/// Round `value` to exactly `places` fractional digits, half-even. A no-op
/// when `value` already has `places` or fewer (this only ever REMOVES
/// precision `Dec` does not need — see [`MAX_FETCHED_QUOTE_PLACES`] and
/// [`apply_update`] for why a fetched quote is rounded, twice, through this).
fn round_to_places(value: Dec, places: u32) -> Dec {
    if value.places <= places {
        return value;
    }
    // Checked, not `pow`: from 39 dropped places up there is no `i128` divisor,
    // and an unchecked one panics. Such a value is under 10^38 / 10^39 in
    // magnitude, which is below half a unit of the target scale — zero.
    // (`Dec::rounded` is not a substitute: it rounds half away from zero.)
    let Some(divisor) = 10i128.checked_pow(value.places - places) else {
        return Dec::new(0, places);
    };
    let quotient = value.mantissa / divisor;
    let remainder = (value.mantissa % divisor).abs();
    let half = divisor / 2;
    let round_up = remainder > half || (remainder == half && quotient % 2 != 0);
    let mantissa = if round_up {
        quotient + value.mantissa.signum()
    } else {
        quotient
    };
    Dec::new(mantissa, places)
}

/// `text` with each `(date, line)` of `lines` inserted where it belongs in
/// date order among the file's `P` directives: straight after the LAST `P`
/// line dated on or before it, scanning from the end. `lines` must already be
/// in `(date, symbol)` order; each lands after the ones before it, so lines
/// added on one date stay together, after that date's existing lines, in
/// symbol order.
///
/// Why not just append: a price file is a dated history that people read and
/// diff, and a weekly backfill appended after today's close would put six
/// weeks of September below November. Inserting keeps the file in the order
/// it was (presumably) in.
///
/// What it never does is move anything already there. No existing line —
/// user `P` lines, comments, blank lines, other directives — is reordered or
/// rewritten; new lines only go BETWEEN lines, byte-exact everywhere else. So a
/// file that is already out of order stays exactly as out of order as it was;
/// scanning from the end means a new line lands after the last line that
/// could precede it, which in a file that is mostly appended to is where the
/// most recent history is. A new line older than every `P` line goes before
/// the first one; a file with no `P` lines at all is appended to, the one
/// position provably unable to change anything already in it (the argument
/// `budget_api::create_budget_file`'s `include` placement makes).
///
/// A `P` line inside a `comment` … `end comment` block is not a price and
/// is never inserted next to: a line placed there would parse fine and be
/// silently ignored.
fn insert_price_lines<'a>(text: &str, lines: impl Iterator<Item = (&'a str, &'a str)>) -> String {
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    // Each existing line WITH its own terminator, so joining them back is
    // byte-exact; `Some(date)` on the ones that are live `P` directives.
    let mut in_comment = false;
    let mut segments: Vec<(String, Option<String>)> = text
        .split_inclusive('\n')
        .map(|segment| {
            let content = segment.trim_end_matches(['\r', '\n']);
            let date = if in_comment {
                in_comment = content.trim_end() != "end comment";
                None
            } else if content.trim_end() == "comment" {
                in_comment = true;
                None
            } else {
                price_line_date(content)
            };
            (segment.to_string(), date)
        })
        .collect();

    for (date, line) in lines {
        let after_last_earlier = segments
            .iter()
            .rposition(|(_, existing)| existing.as_deref().is_some_and(|d| d <= date))
            .map(|index| index + 1);
        let before_first = || segments.iter().position(|(_, existing)| existing.is_some());
        let at = after_last_earlier
            .or_else(before_first)
            .unwrap_or(segments.len());
        // The line it follows may be the file's last, with no terminator.
        if let Some((previous, _)) = at.checked_sub(1).and_then(|i| segments.get_mut(i))
            && !previous.ends_with('\n')
        {
            previous.push_str(newline);
        }
        segments.insert(at, (format!("{line}{newline}"), Some(date.to_string())));
    }
    segments.into_iter().map(|(segment, _)| segment).collect()
}

/// The date of a live `P` directive line, as ISO `YYYY-MM-DD`, or `None` for
/// any other line. A directive starts at column 0 — an indented `P` is a
/// posting's content, not a price — and hledger accepts `/` and `.` as date
/// separators and unpadded months and days, so those are normalised for the
/// comparison (the line itself is never rewritten).
fn price_line_date(line: &str) -> Option<String> {
    let rest = line.strip_prefix('P')?;
    if !rest.starts_with([' ', '\t']) {
        return None;
    }
    let token = rest.split_whitespace().next()?;
    let mut parts = token.split(['-', '/', '.']);
    let (year, month, day) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() {
        return None;
    }
    let number = |part: &str, width: usize| {
        (!part.is_empty() && part.len() <= width && part.bytes().all(|b| b.is_ascii_digit()))
            .then(|| format!("{part:0>width$}"))
    };
    Some(format!(
        "{}-{}-{}",
        number(year, 4).filter(|year| year.len() == 4 && !year.starts_with('0'))?,
        number(month, 2)?,
        number(day, 2)?
    ))
}

/// Does `text` already have a `P DATE [TIME] SYMBOL ...` line for this exact
/// date and symbol? Line-based, mirroring the bash scripts' own dedup — a
/// false negative here just means a harmless duplicate `P` line (hledger keeps
/// the last-declared one for a tied date; see `PriceDb`'s own doc comment), so
/// this does not need to be parser-exact, only good enough to avoid the common
/// case of running the update twice in a row.
fn line_exists(text: &str, date: &str, symbol: &str) -> bool {
    text.lines().any(|line| {
        let Some(rest) = line.trim_start().strip_prefix('P') else {
            return false;
        };
        let mut tokens = rest.split_whitespace();
        if tokens.next() != Some(date) {
            return false;
        }
        // An optional `HH:MM[:SS]` clock token sits between the date and the
        // commodity (`parse_price_directive` accepts the same form).
        let next = tokens.next();
        let commodity_token = if next.is_some_and(|token| token.contains(':')) {
            tokens.next()
        } else {
            next
        };
        commodity_token.is_some_and(|token| token.trim_matches('"') == symbol)
    })
}

// ===========================================================================
// Handles, parsing and errors
// ===========================================================================

/// The path a `journalId` names, taken from the files the parse actually read.
/// Mirrors `budget_api::journal_path` exactly.
fn journal_path(location: &JournalLocation, journal: &Journal, id: &str) -> Option<VaultPath> {
    location.source_path(journal, id)
}

/// Layer 1: shape, before any filesystem call. Mirrors
/// `budget_api::validate_journal_id` exactly — duplicated rather than shared,
/// the existing convention across this route-family of modules.
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

/// Read one journal file, reporting only what the caller already knows, with
/// the [`Revision`] a later write of it must still match — identity and
/// fingerprint from one read.
///
/// Never follows a symlink: the path is the real one the snapshot resolved, so
/// a link there now was swapped in since, and that is a stale revision.
fn read_journal(
    location: &JournalLocation,
    path: &VaultPath,
    id: &str,
) -> Result<(String, Revision), AppError> {
    read_pinned_text(location, path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::InvalidInput {
            return stale(id);
        }
        AppError::Internal(format!(
            "{} could not be read: {}",
            quoted(id),
            error.kind()
        ))
    })
}

/// A journal file's text and the [`Revision`] a later write of it must still
/// match — identity and fingerprint from one read, never through a link at the
/// final component. As an `io::Error` so callers branch on its kind: a symlink
/// or non-regular file is `InvalidInput`, non-UTF-8 is `InvalidData`.
fn read_pinned_text(
    location: &JournalLocation,
    path: &VaultPath,
) -> std::io::Result<(String, Revision)> {
    let (bytes, revision) = location
        .vault()
        .read_pinned(path, None)
        .map_err(std::io::Error::from)?;
    let text = String::from_utf8(bytes)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    Ok((text, revision))
}

/// A `404` that quotes only the caller's own handle.
fn unresolved(id: &str) -> AppError {
    AppError::NotFound(format!("no journal file called {}", quoted(id)))
}

/// A `409`: the file changed on disk between the plan and the write.
fn stale(id: &str) -> AppError {
    AppError::Conflict(format!(
        "{} changed on disk while prices were being fetched, so nothing was written. Try again.",
        quoted(id)
    ))
}

/// A caller-supplied handle, escaped and clipped for an error body.
fn quoted(value: &str) -> String {
    const MAX_CHARS: usize = 120;
    let clipped: String = value.chars().take(MAX_CHARS).collect();
    let ellipsis = if clipped.chars().count() < value.chars().count() {
        "…"
    } else {
        ""
    };
    format!("{clipped:?}{ellipsis}")
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use ledgeline_core::model::{Commodity as CoreCommodity, PriceDirective};
    use std::path::PathBuf;

    fn price_directive(date: &str, commodity: &str, source_file: &str) -> PriceDirective {
        use ledgeline_core::decimal::Dec;
        use ledgeline_core::model::{Amount, AmountStyle, CommoditySide};
        PriceDirective {
            date: date.to_string(),
            commodity: CoreCommodity(commodity.to_string()),
            price: Amount {
                commodity: CoreCommodity("$".to_string()),
                quantity: Dec::new(100, 2),
                style: AmountStyle {
                    side: CommoditySide::Left,
                    spaced: false,
                    decimal_mark: Some('.'),
                    digit_groups: None,
                    precision: 2,
                },
                cost: None,
            },
            source_file: PathBuf::from(source_file),
        }
    }

    #[test]
    fn yahoo_ticker_falls_back_to_the_symbol_with_no_tag() {
        let tags = vec![];
        assert_eq!(yahoo_ticker(&tags, "AAPL"), "AAPL");
    }

    #[test]
    fn yahoo_ticker_reads_the_commoditys_yahoo_tag() {
        let tags = vec![(
            CoreCommodity("BRK'B".to_string()),
            vec![("yahoo".to_string(), "BRK-B".to_string())],
        )];
        assert_eq!(yahoo_ticker(&tags, "BRK'B"), "BRK-B");
    }

    #[test]
    fn yahoo_ticker_ignores_other_tags_on_the_same_commodity() {
        let tags = vec![(
            CoreCommodity("VTI".to_string()),
            vec![("name".to_string(), "Vanguard Total Market".to_string())],
        )];
        assert_eq!(yahoo_ticker(&tags, "VTI"), "VTI");
    }

    #[test]
    fn price_counts_tallies_per_source_file() {
        let prices = vec![
            price_directive("2026-01-01", "AAPL", "/j/a.journal"),
            price_directive("2026-02-01", "AAPL", "/j/a.journal"),
            price_directive("2026-01-01", "VTI", "/j/b.journal"),
        ];
        let journal = Journal {
            source_name: "/j/main.journal".to_string(),
            source_files: vec![],
            transactions: vec![],
            periodic_transactions: vec![],
            auto_rules: vec![],
            accounts: vec![],
            aliases: vec![],
            commodity_styles: vec![],
            commodity_tags: vec![],
            prices,
            default_commodity: None,
            decimal_mark: None,
            leading_comment: None,
        };
        let tally = price_counts(&journal);
        assert_eq!(tally.get(FsPath::new("/j/a.journal")), Some(&2));
        assert_eq!(tally.get(FsPath::new("/j/b.journal")), Some(&1));
        assert_eq!(tally.get(FsPath::new("/j/c.journal")), None);
    }

    #[test]
    fn round_to_places_is_a_no_op_when_already_short_enough() {
        assert_eq!(round_to_places(Dec::new(2280, 2), 2), Dec::new(2280, 2));
        assert_eq!(round_to_places(Dec::new(228, 0), 2), Dec::new(228, 0));
    }

    /// The exact bug this exists to fix: Yahoo's chart JSON round-trips a
    /// close through `f64` and yields a ten-decimal artifact instead of the
    /// two-decimal price a person would write.
    #[test]
    fn round_to_places_strips_a_binary_float_artifact() {
        assert_eq!(
            round_to_places(Dec::new(3_668_500_061_035, 10), 2),
            Dec::new(36685, 2)
        );
    }

    #[test]
    fn round_to_places_rounds_half_even() {
        // 2.125 at 2 places: the discarded digit is exactly half (5), and the
        // kept digit (2) is even, so it rounds DOWN to 2.12.
        assert_eq!(round_to_places(Dec::new(2125, 3), 2), Dec::new(212, 2));
        // 2.135: the kept digit (3) is odd, so the tie rounds UP to 2.14.
        assert_eq!(round_to_places(Dec::new(2135, 3), 2), Dec::new(214, 2));
    }

    #[test]
    fn round_to_places_rounds_up_past_the_midpoint() {
        assert_eq!(round_to_places(Dec::new(2287, 3), 2), Dec::new(229, 2));
    }

    #[test]
    fn round_to_places_is_total_past_the_i128_divisor() {
        // 10^39 has no `i128`; the unchecked `pow` panicked here.
        assert_eq!(round_to_places(Dec::new(i128::MAX, 41), 2), Dec::new(0, 2));
        assert_eq!(
            round_to_places(Dec::new(i128::MIN, u32::MAX), 0),
            Dec::new(0, 0)
        );
    }

    #[test]
    fn line_exists_matches_date_and_symbol_exactly() {
        let text = "P 2026-06-30 AAPL $228.00\nP 2026-06-30 VTI $268.90\n";
        assert!(line_exists(text, "2026-06-30", "AAPL"));
        assert!(line_exists(text, "2026-06-30", "VTI"));
        assert!(!line_exists(text, "2026-07-01", "AAPL"));
        assert!(!line_exists(text, "2026-06-30", "TSLA"));
    }

    #[test]
    fn line_exists_skips_an_optional_clock_time_token() {
        let text = "P 2026-06-30 12:00:00 AAPL $228.00\n";
        assert!(line_exists(text, "2026-06-30", "AAPL"));
    }

    #[test]
    fn line_exists_is_false_on_an_empty_file() {
        assert!(!line_exists("", "2026-06-30", "AAPL"));
    }

    fn insert(text: &str, lines: &[(&str, &str)]) -> String {
        insert_price_lines(text, lines.iter().copied())
    }

    #[test]
    fn a_file_with_no_price_lines_is_appended_to() {
        assert_eq!(
            insert("; prices\n", &[("2026-07-01", "P 2026-07-01 AAPL $228.00")]),
            "; prices\nP 2026-07-01 AAPL $228.00\n"
        );
        assert_eq!(
            insert("", &[("2026-07-01", "P 2026-07-01 AAPL $228.00")]),
            "P 2026-07-01 AAPL $228.00\n"
        );
    }

    /// The last line had no newline: one is added before the new line, and
    /// the file ends with one after.
    #[test]
    fn inserting_after_a_last_line_with_no_newline_adds_one() {
        assert_eq!(
            insert(
                "P 2026-06-01 AAPL $220.00",
                &[("2026-07-01", "P 2026-07-01 AAPL $228.00")]
            ),
            "P 2026-06-01 AAPL $220.00\nP 2026-07-01 AAPL $228.00\n"
        );
    }

    #[test]
    fn a_line_goes_into_the_middle_after_its_date_group() {
        let text = "\
P 2026-06-01 AAPL $220.00
P 2026-06-08 AAPL $221.00
P 2026-06-08 VTI $260.00
P 2026-06-15 AAPL $222.00
";
        assert_eq!(
            insert(
                text,
                &[
                    ("2026-06-08", "P 2026-06-08 MSFT $400.00"),
                    ("2026-06-10", "P 2026-06-10 AAPL $221.50"),
                ]
            ),
            "\
P 2026-06-01 AAPL $220.00
P 2026-06-08 AAPL $221.00
P 2026-06-08 VTI $260.00
P 2026-06-08 MSFT $400.00
P 2026-06-10 AAPL $221.50
P 2026-06-15 AAPL $222.00
"
        );
    }

    /// Older than everything: before the first `P` line, after the header
    /// comment and blank line, which stay where they are.
    #[test]
    fn a_line_older_than_every_price_goes_before_the_first() {
        let text = "; Market prices.\n\nP 2026-06-08 AAPL $221.00\n";
        assert_eq!(
            insert(text, &[("2026-05-29", "P 2026-05-29 AAPL $219.00")]),
            "; Market prices.\n\nP 2026-05-29 AAPL $219.00\nP 2026-06-08 AAPL $221.00\n"
        );
    }

    /// Newer than everything: after the last `P` line — before a trailing
    /// comment that follows it, which stays last.
    #[test]
    fn a_line_newer_than_every_price_goes_after_the_last() {
        let text = "P 2026-06-08 AAPL $221.00\n\n; end of prices\n";
        assert_eq!(
            insert(text, &[("2026-07-01", "P 2026-07-01 AAPL $228.00")]),
            "P 2026-06-08 AAPL $221.00\nP 2026-07-01 AAPL $228.00\n\n; end of prices\n"
        );
    }

    /// An out-of-order file is not sorted: the new line goes after the LAST
    /// line dated on or before it, scanning from the end, and every existing
    /// line, comment and blank line stays put.
    #[test]
    fn an_out_of_order_file_is_left_alone_and_scanned_from_the_end() {
        let text = "\
P 2026-06-01 AAPL $220.00
; mid-file note

P 2026-06-20 AAPL $225.00
P 2026-06-05 AAPL $221.00
P 2026-06-30 AAPL $228.00
";
        assert_eq!(
            insert(text, &[("2026-06-10", "P 2026-06-10 AAPL $223.00")]),
            "\
P 2026-06-01 AAPL $220.00
; mid-file note

P 2026-06-20 AAPL $225.00
P 2026-06-05 AAPL $221.00
P 2026-06-10 AAPL $223.00
P 2026-06-30 AAPL $228.00
"
        );
    }

    #[test]
    fn crlf_files_stay_crlf() {
        assert_eq!(
            insert(
                "P 2026-06-01 AAPL $220.00\r\nP 2026-06-30 AAPL $228.00\r\n",
                &[("2026-06-15", "P 2026-06-15 AAPL $224.00")]
            ),
            "P 2026-06-01 AAPL $220.00\r\nP 2026-06-15 AAPL $224.00\r\nP 2026-06-30 AAPL $228.00\r\n"
        );
    }

    /// A `P` line inside a comment block, or indented, is not a price.
    #[test]
    fn commented_and_indented_price_lines_are_not_anchors() {
        let text = "\
P 2026-06-01 AAPL $220.00
comment
P 2026-06-30 AAPL $999.00
end comment
2026-06-02 note
    P 2026-06-29 is not a directive  ; posting comment
";
        let out = insert(text, &[("2026-07-01", "P 2026-07-01 AAPL $228.00")]);
        assert!(
            out.starts_with("P 2026-06-01 AAPL $220.00\nP 2026-07-01 AAPL $228.00\ncomment\n"),
            "{out}"
        );
    }

    #[test]
    fn price_line_dates_are_normalised_for_comparison() {
        assert_eq!(
            price_line_date("P 2026/6/3 AAPL $1").as_deref(),
            Some("2026-06-03")
        );
        assert_eq!(
            price_line_date("P\t2026.06.30 12:00 AAPL $1").as_deref(),
            Some("2026-06-30")
        );
        assert_eq!(price_line_date("Price list"), None);
        assert_eq!(price_line_date(" P 2026-06-30 AAPL $1"), None);
        assert_eq!(price_line_date("P AAPL $1"), None);
    }
}
