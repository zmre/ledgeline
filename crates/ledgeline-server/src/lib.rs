//! `ledgeline-server` library: the axum application that serves the Phase 2
//! read endpoints from a parsed journal.
//!
//! The binary ([`main`](../main.rs)) is a thin CLI wrapper around [`app`]; the
//! app is exposed here as a library so integration tests can drive the real
//! HTTP layer with `tower`'s `oneshot` (no sockets required).
//!
//! Each wire endpoint's JSON body is serialized once from the journal into
//! immutable [`Bytes`] and stored in a [`Snapshot`]; a request hands the buffer
//! straight to the response body, which is a refcount bump rather than a copy
//! (PERF-1 — holding `serde_json::Value` trees instead cost 2.7 GB and re-walked
//! them on every request). The biggest of them, `/transactions`, is serialized
//! on its first request rather than at build time, because the SPA no longer
//! reads it (plan 26 B5). The native report/budget endpoints ([`reports_api`])
//! and the journal-view routes ([`journal_api`]) instead depend on request query
//! params, so they are computed per request from the parsed [`Journal`] the same
//! snapshot holds, with the journal view's whole-journal derivations memoized in
//! the snapshot.
//!
//! Every snapshot carries an `ETag`, so the SPA's 30-second poll costs a `304`
//! and no body at all until the journal actually changes (PERF-2).
//!
//! The whole snapshot lives behind an [`ArcSwap`] so the parsed journal can be
//! HOT-SWAPPED at runtime (live-reload on file change; the desktop File→Open
//! action) without restarting the server or touching the router: handlers always
//! read the current snapshot, and a swap atomically publishes a fresh one.
//!
//! The WRITE path ([`edit_api`], Phase 5.2) is layered on top: a state built from
//! a journal *file* also holds an [`Arc`]-shared [`std::sync::Mutex`] over a
//! [`JournalEditor`]. Reads stay lock-free (they only touch the `ArcSwap`); an
//! edit serializes on the mutex, validates + saves through the editor, and then
//! rebuilds and republishes the snapshot so the read endpoints reflect the change
//! immediately. A state built without a path (the oneshot test helper [`app`])
//! has no editor, so the edit endpoints report that editing is disabled.
//!
//! ACCESS CONTROL lives in [`security`] and is applied by
//! [`router_with_security`]: a per-process bearer token on every wire and `/api`
//! route, a `Host` guard, an opt-in exact-origin CORS allowlist, and the response
//! security headers. [`app`] and [`router_with_state`] deliberately build the
//! router WITHOUT any of it, for the in-process test harnesses only — read the
//! threat model on [`security`] before putting either on a real socket.

mod account_api;
mod alias_api;
mod app_dirs;
mod auto_api;
mod auto_wire;
mod benchmarks_api;
mod budget_api;
mod commodity_profile;
mod conf_api;
mod edit_api;
mod error;
mod git;
mod hledger;
mod import_api;
mod journal_api;
mod location;
mod prefs;
mod prices_api;
mod profiles_api;
mod projection_cache;
mod projections_api;
mod qb_journal_api;
pub mod recents;
mod redact;
mod reports_api;
mod rules_api;
mod security;
mod sidecar;
mod spa;
mod stage;
mod workbook_sandbox;
mod yahoo;
mod yahoo_history;
mod yahoo_profile;

use arc_swap::ArcSwap;
use axum::{
    Router,
    body::{Body, Bytes},
    extract::State,
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use ledgeline_core::vault::{Vault, VaultPath};
use ledgeline_core::{EditError, Journal, JournalEditor, wire};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use tower_http::CompressionLevel;
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::compression::CompressionLayer;
use tower_http::set_header::SetResponseHeaderLayer;

pub use app_dirs::{AppDirs, CONFIG_DIR_ENV, HLEDGER_ENV, IGNORE_USER_CONF_ENV};
pub use location::JournalLocation;
pub use security::{
    AccessToken, ProcessToken, Security, SecurityError, TOKEN_ENV, token_from_env_or_random,
    token_init_script,
};
// `ledgeline import` (WP-16 Phase 3). The binary is a SEPARATE crate from this
// library, so the `pub(crate)` the rest of `import_api` uses would not reach it —
// these names are the whole of the CLI's surface, and the request/response
// wire types stay module-private behind them. `run_cli_import` deliberately
// takes `CliImport` (the `clap` derive) rather than loose arguments: one
// definition of what an import run can be asked to do. `CliRunReport` (WP-17
// Phase D) is the CSV/QuickBooks-Journal-branching return type `run_cli_import`
// now hands back; `CliQbReport`/`CliQbWritten` are its QuickBooks Journal
// variant's own payload, alongside the CSV path's pre-existing
// `CliImportReport`/`CliImportWritten`.
pub use import_api::{
    CliImport, CliImportReport, CliImportWritten, CliQbConflict, CliQbReport, CliQbWritten,
    CliRunReport, run_cli_import,
};
// Spreadsheet parsing runs in a resource-limited child process (see
// `workbook_sandbox`). The binary needs the worker entry point and the
// allocator that enforces its memory budget; the in-process integration suites
// need `set_worker_executable` to point the import routes at the real binary.
pub use workbook_sandbox::{WorkerAllocator, run_worker_if_requested, set_worker_executable};
// `PriceFeed` (+ `FetchedPrice`/`YahooError`, reachable through its method
// signature) is re-exported so the integration tests can build a fake and hand
// it to `AppState::with_price_source` — the only way `/api/prices/update` is
// tested without a live network call.
pub use yahoo::{FetchedPrice, PriceFeed, YahooError};
// Same arrangement for the Holdings pie's classification source: a fake
// `ProfileFeed` handed to `AppState::with_profile_source` is how
// `/api/holdings/profiles` is tested with no network.
pub use yahoo_profile::{AssetMix, ProfileError, ProfileFeed, YahooProfile};
// `HistoryFeed` likewise, for `AppState::with_history_source`: the benchmark
// overlay's integration tests hand it a fake instead of Yahoo Finance.
pub use yahoo_history::HistoryFeed;

/// An immutable, atomically-publishable view of one parsed journal: the parsed
/// [`Journal`] for the per-request report handlers, plus every wire endpoint's
/// body serialized once (so handler dispatch neither re-serializes nor copies).
///
/// The bodies are [`Bytes`], not `serde_json::Value` (PERF-1). A `Value` tree
/// cost 8.5× what the JSON text it represents does — 2.7 GB at 200k transactions
/// — held resident for the life of the snapshot, and every request deep-cloned
/// it (691 ms) and re-walked it through serde (242 ms). Serialized bytes are
/// serialized once at build time, and `Bytes::clone` is a refcount bump.
pub(crate) struct Snapshot {
    /// The journal every READ serves — reports, views, insights, holdings,
    /// checks. With `--auto` in force ([`Self::auto`]) it is the journal with
    /// its `=` rules applied ([`ledgeline_core::auto::apply_journal`]);
    /// otherwise it is [`Self::raw`] itself, the same `Arc`.
    pub(crate) journal: Arc<Journal>,
    /// The journal as its files spell it, without generated postings — what
    /// the [`JournalEditor`] holds. Read it for anything that maps a posting
    /// back to a line of a file.
    pub(crate) raw: Arc<Journal>,
    /// Whether `--auto` is in force, and where that was read (plan 30).
    pub(crate) auto: auto_api::AutoSetting,
    /// Where `journal` was read from: the vault every journal-tree read and
    /// write goes through. `None` for a state built from a journal with no
    /// backing storage ([`AppState::from_journal`]). Carried in the snapshot so
    /// a handler always pairs a journal with the storage it came from.
    pub(crate) location: Option<Arc<JournalLocation>>,
    /// This snapshot's `ETag`, identical across every endpoint it serves: all of
    /// them are derived from the same journal and change together. See
    /// [`next_etag`] for why it is a counter and not a content hash.
    pub(crate) etag: HeaderValue,
    pub(crate) version: Bytes,
    pub(crate) accountnames: Bytes,
    /// The hledger-web `/transactions` body, serialized on FIRST REQUEST rather
    /// than at build time (plan 26 B5). The SPA reads the journal through the
    /// windowed `/api/journal/*` routes and never asks for it, so only an
    /// external hledger-web client pays its 563 ms and 347 MB at 200k — once per
    /// snapshot, with bytes identical to what the eager build produced.
    pub(crate) transactions: std::sync::OnceLock<Bytes>,
    pub(crate) prices: Bytes,
    pub(crate) commodities: Bytes,
    pub(crate) accounts: Bytes,
    /// The `/api/journal` payload: which journal the user is looking at (see
    /// `ledgeline_core::title`), plus the whole-journal display metadata and this
    /// snapshot's `generation` ([`JournalInfoBody`]). Precomputed with the rest
    /// so it swaps atomically with them — opening a different journal must never
    /// leave a stale name sitting over fresh numbers — and built INLINE rather
    /// than on a thread of its own: two cheap passes over the postings and one
    /// over the account names, small beside `/transactions`.
    pub(crate) journal_info: Bytes,
    /// Lazily-built derivations for the journal-view routes ([`journal_api`]):
    /// computed at most once per snapshot, and dropped with it.
    pub(crate) view: Arc<journal_api::ViewCache>,
    /// The projection run's real side, built on first use and dropped with
    /// the snapshot, so a new journal can never be served a stale one
    /// (plan 29).
    pub(crate) projections: projection_cache::ProjectionCache,
    /// The `hledger.conf` in force for this journal and what Ledgeline honours
    /// from it (plan 30, Decision 10): read on first use through
    /// [`AppState::conf`], or handed in by the write that republished this
    /// snapshot. A new snapshot — a reload, an edit, a config write — reads it
    /// afresh, so it can never outlive the file it describes by more than one
    /// generation.
    pub(crate) conf: std::sync::OnceLock<Arc<conf_api::ConfState>>,
}

impl Snapshot {
    /// Where this snapshot's journal lives: the editor's vault when one is
    /// bound, else — for a state built from an already-parsed journal — the
    /// directory of its main source file on disk ([`JournalLocation::of_parsed`]).
    /// `None` only for a journal with no backing file at all.
    ///
    /// Every journal-tree read and write in a handler starts here, from the
    /// SAME snapshot as the journal it is about.
    pub(crate) fn storage(&self) -> Option<Arc<JournalLocation>> {
        self.location
            .clone()
            .or_else(|| JournalLocation::of_parsed(&self.journal).map(Arc::new))
    }
}

/// The `/api/journal` body: the engine-side metadata plus this snapshot's
/// `generation` — the ETag's opaque value without its quotes. A client compares
/// it for EQUALITY only (it is unique per snapshot and across restarts; it is
/// not ordered): a changed generation means every journal payload changed with
/// it, which is what lets a poller watch this one small route.
#[derive(Serialize)]
struct JournalInfoBody<'a> {
    #[serde(flatten)]
    info: wire::WireJournalInfo,
    generation: &'a str,
}

/// The opaque text of a strong `ETag` (`"x-1"` → `x-1`).
fn generation_of(etag: &HeaderValue) -> &str {
    etag.to_str().unwrap_or_default().trim_matches('"')
}

impl Snapshot {
    /// Serialize every endpoint body from `journal`, once.
    ///
    /// Takes the journal as an [`Arc`] rather than a reference so the caller's
    /// already-parsed journal is shared, not deep-cloned (PERF-1b: the clone was
    /// 86 ms and 284 MB at 200k for a journal nobody mutates).
    ///
    /// The wire serializers cannot fail for finite, string-keyed journal data,
    /// so any (impossible) `serde_json` error collapses to the JSON body `null`
    /// — the same guarantee Phase 1 relies on in `parse_to_transactions_value`.
    ///
    /// Every payload here is a cheap pass; the expensive ones are LAZY.
    /// `/transactions` (563 ms of the old 1,048 ms build at 200k) is serialized
    /// on first request ([`Snapshot::transactions`]), so startup, every
    /// live-reload and every edit's republish no longer pay for a payload the
    /// SPA does not read.
    ///
    /// `/api/diagnostics` is NOT built here any more (plan 26 B4): it depends
    /// on the client's date, so it is computed per request and memoized in the
    /// snapshot's [`journal_api::ViewCache`] — which also takes its 270 ms of
    /// whole-journal passes off every startup and republish.
    ///
    /// `journal` is the RAW journal. Whether `--auto` is in force is read here,
    /// from the `hledger.conf` in force ([`auto_api::auto_setting`]), so every
    /// republish — an edit, a reload, a config write — picks up a change, and
    /// its new generation refreshes the clients (plan 30 Decision 7).
    fn from_journal(
        raw: Arc<Journal>,
        location: Option<Arc<JournalLocation>>,
        user: &ledgeline_core::hledger_conf::UserConfDirs,
    ) -> Self {
        // Minted FIRST, because `/api/journal` carries it as `generation`.
        let etag = next_etag();
        let auto = auto_api::auto_setting(
            location
                .clone()
                .or_else(|| JournalLocation::of_parsed(&raw).map(Arc::new))
                .as_deref(),
            user,
        );
        let journal = if auto.in_force && !raw.auto_rules.is_empty() {
            Arc::new(ledgeline_core::auto::apply_journal(&raw))
        } else {
            Arc::clone(&raw)
        };
        let rest = (
            json_bytes(&wire::version_value()),
            json_bytes(&wire::journal_to_accountnames(&journal)),
            json_bytes(&wire::journal_to_prices(&journal)),
            json_bytes(&wire::journal_to_commodities(&journal)),
            json_bytes(&wire::journal_to_accounts(&journal)),
            json_bytes(&JournalInfoBody {
                info: wire::journal_to_info(&journal),
                generation: generation_of(&etag),
            }),
        );
        let (version, accountnames, prices, commodities, accounts, journal_info) = rest;
        Self {
            etag,
            version,
            accountnames,
            transactions: std::sync::OnceLock::new(),
            prices,
            commodities,
            accounts,
            journal_info,
            view: journal_api::ViewCache::warmed(&journal),
            projections: projection_cache::ProjectionCache::default(),
            conf: std::sync::OnceLock::new(),
            journal,
            raw,
            auto,
            location,
        }
    }
}

/// The app directories an [`AppState`] starts with: [`AppDirs::from_env`]
/// without the user-level `hledger.conf` tiers. Only the desktop binary hands a
/// state `from_env()` itself ([`AppState::with_app_dirs`]), so a library caller
/// — every in-process test included — never reads somebody's
/// `~/.hledger.conf` by accident.
fn default_app_dirs() -> AppDirs {
    AppDirs {
        hledger_conf: ledgeline_core::hledger_conf::UserConfDirs::default(),
        ..AppDirs::from_env()
    }
}

/// Serialize one endpoint body to JSON bytes, collapsing the unreachable
/// `serde_json` error to the literal `null` rather than unwrapping.
fn json_bytes<T: Serialize>(value: &T) -> Bytes {
    serde_json::to_vec(value).map_or_else(|_| Bytes::from_static(b"null"), Bytes::from)
}

/// Mint the `ETag` for a freshly-built snapshot.
///
/// A per-process random prefix plus a monotonic counter, NOT a hash of the
/// bodies: hashing 347 MB of JSON would add hundreds of milliseconds to every
/// journal load, and the counter already answers the only question `If-None-Match`
/// asks — "is this the same snapshot you gave me?". The random prefix is what
/// keeps a client that cached generation 1 from a *previous* process being told
/// `304` for a different journal that happens to also be on generation 1.
///
/// It errs toward re-sending: an edit that produces byte-identical payloads still
/// bumps the counter, so a `304` can never serve stale data.
fn next_etag() -> HeaderValue {
    /// Randomized once per process, so ETags are unique across process restarts.
    static PREFIX: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    static GENERATION: AtomicU64 = AtomicU64::new(0);

    let prefix = PREFIX.get_or_init(|| {
        let mut seed = [0u8; 8];
        // A failed OS CSPRNG is not worth failing a journal load over: fall back
        // to pid + start time, which still differs between concurrent and
        // successive processes. The prefix only has to be unpredictable enough
        // to avoid collisions; nothing security-relevant rests on it.
        if getrandom::fill(&mut seed).is_err() {
            let nanos = std::time::UNIX_EPOCH
                .elapsed()
                .map_or(0, |since| since.subsec_nanos());
            seed = (u64::from(std::process::id()) << 32 | u64::from(nanos)).to_le_bytes();
        }
        u64::from_le_bytes(seed)
    });
    let generation = GENERATION.fetch_add(1, Ordering::Relaxed);
    HeaderValue::from_str(&format!("\"{prefix:016x}-{generation:x}\""))
        .unwrap_or_else(|_| HeaderValue::from_static("\"ledgeline\""))
}

/// Cheaply-cloneable application state: an atomically-swappable [`Snapshot`] for
/// the lock-free read path, plus an optional [`JournalEditor`] behind a mutex for
/// the write path.
///
/// Cloning shares both the swap cell and the editor mutex, so a clone handed to a
/// file watcher, the GUI, or an edit handler operates on the same journal: reads
/// stay lock-free, and the single editor mutex serializes all writers.
#[derive(Clone)]
pub struct AppState {
    inner: Arc<ArcSwap<Snapshot>>,
    /// The write path's editor, or `None` when the state was built from an
    /// already-parsed [`Journal`] with no backing file (the edit endpoints then
    /// report editing disabled). Held behind an `Arc<Mutex<…>>` so every clone
    /// shares one editor and writers serialize on it.
    editor: Arc<Mutex<Option<JournalEditor>>>,
    /// Serializes rules-file WRITES. The journal editor mutex above does not
    /// cover these (they never touch the editor), and without this two `PUT`s
    /// carrying the same valid revision could both pass their pre-write check
    /// and one update would be silently lost.
    ///
    /// A [`tokio::sync::Mutex`] because it is held across an `.await`: the whole
    /// scan-read-check-write sequence runs on the blocking pool through
    /// [`reports_api::compute`], so the guard necessarily crosses a yield point
    /// — which is exactly what a `std::sync::Mutex` guard may never do.
    rules_writes: Arc<tokio::sync::Mutex<()>>,
    /// Where a dropped statement lives between the upload and the import (WP-11).
    ///
    /// **Per state, and therefore per server session.** Two `AppState`s in one
    /// process — which is what the integration tests build — get two areas with
    /// independently randomized roots, so a `StageId` minted by one is unknown to
    /// the other twice over: it is absent from the map, and the directory it
    /// names is not even in the same tree. The whole area is removed when the
    /// last clone of this state drops.
    stages: Arc<stage::StageArea>,
    /// Where a staged, already-parsed QuickBooks Journal upload lives between
    /// the upload and the commit (WP-17 Phase B). Per state, for the same
    /// reason as [`Self::stages`]; a sibling of it rather than a variant added
    /// to it, because nothing here is ever written to disk or handed to a
    /// subprocess — see [`qb_journal_api::QbStageArea`].
    qb_stages: Arc<qb_journal_api::QbStageArea>,
    /// Serializes IMPORTS. Two concurrent commits into one journal would
    /// interleave hledger's appends and each other's `.latest` writes, and
    /// neither the editor mutex (imports do not go through the editor) nor
    /// [`Self::rules_writes`] covers that. A `tokio` mutex for the same reason
    /// as above: the guard is held across the blocking-pool `.await`.
    import_writes: Arc<tokio::sync::Mutex<()>>,
    /// Where `prices_api`'s `POST /api/prices/update` fetches quotes from.
    /// `Arc<dyn PriceFeed>` rather than a concrete `reqwest::Client` so
    /// [`AppState::with_price_source`] can swap in a fake for the integration
    /// tests — the only route in this crate that makes an outbound network
    /// call, and the only state field that is not itself journal-derived.
    price_source: Arc<dyn yahoo::PriceFeed>,
    /// Where `profiles_api` classifies holdings from (Yahoo's `quoteSummary`
    /// behind a crumb handshake), plus the cache of what it said. Swappable
    /// for a fake through [`AppState::with_profile_source`], like
    /// `price_source`.
    profiles: Arc<profiles_api::Profiles>,
    /// Where `benchmarks_api` fetches benchmark price HISTORY from — the
    /// second (and last) outbound-network field, swappable for the same reason
    /// as [`Self::price_source`] via [`AppState::with_history_source`].
    history_source: Arc<dyn yahoo_history::HistoryFeed>,
    /// The benchmark price cache (`benchmarks.prices.journal`), in memory and
    /// written through beside the journal — see [`sidecar`]. Its own lock, not
    /// [`Self::import_writes`]: the cache is not the journal, and a chart
    /// overlay must never hold up an import or a price update.
    benchmark_cache: Arc<sidecar::SidecarCache<benchmarks_api::BenchmarkCache>>,
    /// Where the app keeps its OWN state — preferences, the staging area's
    /// parent, the `hledger` override. Not journal content, so not the vault.
    /// Defaults to [`default_app_dirs`] (the desktop's
    /// lookups, minus `~/.hledger.conf`); the desktop binary and a sandboxed
    /// host replace it with [`AppState::with_app_dirs`].
    app_dirs: Arc<AppDirs>,
}

/// The real Yahoo feeds, over ONE `reqwest::Client` ([`yahoo::http_client`])
/// so they share its connection pool: the chart endpoint answers both the
/// latest close ([`yahoo::PriceFeed`]) and dividend-adjusted history
/// ([`yahoo_history::HistoryFeed`]), and `quoteSummary` classifies holdings.
struct YahooFeeds {
    prices: Arc<dyn yahoo::PriceFeed>,
    profiles: Arc<profiles_api::Profiles>,
    history: Arc<dyn yahoo_history::HistoryFeed>,
}

impl YahooFeeds {
    fn new() -> Self {
        let client = yahoo::http_client();
        let chart = Arc::new(yahoo::YahooClient::new(client.clone()));
        let transport = Arc::new(yahoo_profile::ReqwestTransport::new(client));
        Self {
            prices: chart.clone(),
            profiles: Arc::new(profiles_api::Profiles::new(Arc::new(
                yahoo_profile::YahooProfileClient::new(transport),
            ))),
            history: chart,
        }
    }
}

impl AppState {
    /// Build read-only state serving an already-parsed `journal`, with no backing
    /// file — the edit endpoints are disabled. Used by the oneshot test harness
    /// ([`app`]) and by callers that hot-swap journals in place without editing.
    ///
    /// This is the one constructor that must clone `journal`: it only borrows
    /// one. Every path that *owns* a parsed journal — [`from_journal_path`],
    /// [`reopen_editor`], [`rebind_editor`], [`replace_journal`] — shares the
    /// editor's `Arc` instead (PERF-1b).
    ///
    /// [`from_journal_path`]: Self::from_journal_path
    /// [`reopen_editor`]: Self::reopen_editor
    /// [`rebind_editor`]: Self::rebind_editor
    /// [`replace_journal`]: Self::replace_journal
    #[must_use]
    pub fn from_journal(journal: &Journal) -> Self {
        let yahoo = YahooFeeds::new();
        let app_dirs = Arc::new(default_app_dirs());
        Self {
            inner: Arc::new(ArcSwap::from_pointee(Snapshot::from_journal(
                Arc::new(journal.clone()),
                None,
                &app_dirs.hledger_conf,
            ))),
            editor: Arc::new(Mutex::new(None)),
            rules_writes: Arc::new(tokio::sync::Mutex::new(())),
            stages: Arc::new(stage::StageArea::new(app_dirs.temp_dir.clone())),
            qb_stages: Arc::new(qb_journal_api::QbStageArea::default()),
            import_writes: Arc::new(tokio::sync::Mutex::new(())),
            price_source: yahoo.prices,
            profiles: yahoo.profiles,
            history_source: yahoo.history,
            benchmark_cache: Arc::default(),
            app_dirs,
        }
    }

    /// Build editing-enabled state bound to the journal file at `path`: open a
    /// [`JournalEditor`] over it and serve the snapshot built from its parsed
    /// journal. The edit endpoints (`POST`/`DELETE /api/transactions`) are then
    /// live and mutate this file through the editor's validation + atomic write.
    ///
    /// The desktop form of [`Self::from_vault`]: the path is resolved once to
    /// its real location and the state is bound to a
    /// [`ledgeline_core::vault::LocalVault`] rooted at its directory
    /// ([`JournalEditor::open`]).
    ///
    /// # Errors
    /// [`EditError::Io`] if the file cannot be read, or [`EditError::Parse`] if it
    /// does not parse.
    pub fn from_journal_path(path: impl AsRef<Path>) -> Result<Self, EditError> {
        Ok(Self::from_editor(JournalEditor::open(path.as_ref())?))
    }

    /// Build editing-enabled state over the journal whose main file is `main`
    /// in `vault`. Every journal-tree read and write this state makes — the
    /// editor, the rules, budget, account, alias, price and projection files,
    /// import bookkeeping, sidecar caches — goes through `vault`.
    ///
    /// # Errors
    /// [`EditError::Parse`] if the journal cannot be read (a not-downloaded file
    /// included) or does not parse.
    pub fn from_vault(vault: Arc<dyn Vault>, main: VaultPath) -> Result<Self, EditError> {
        Ok(Self::from_editor(JournalEditor::open_in(vault, main)?))
    }

    fn from_editor(editor: JournalEditor) -> Self {
        let app_dirs = Arc::new(default_app_dirs());
        let snapshot = Snapshot::from_journal(
            Arc::clone(editor.journal()),
            Some(location_of(&editor)),
            &app_dirs.hledger_conf,
        );
        let yahoo = YahooFeeds::new();
        Self {
            inner: Arc::new(ArcSwap::from_pointee(snapshot)),
            editor: Arc::new(Mutex::new(Some(editor))),
            rules_writes: Arc::new(tokio::sync::Mutex::new(())),
            stages: Arc::new(stage::StageArea::new(app_dirs.temp_dir.clone())),
            qb_stages: Arc::new(qb_journal_api::QbStageArea::default()),
            import_writes: Arc::new(tokio::sync::Mutex::new(())),
            price_source: yahoo.prices,
            profiles: yahoo.profiles,
            history_source: yahoo.history,
            benchmark_cache: Arc::default(),
            app_dirs,
        }
    }

    /// Atomically replace the served journal (and its precomputed payloads).
    /// In-flight requests keep their snapshot; subsequent ones see the new data.
    ///
    /// `journal` is the journal AS WRITTEN — an editor's, or a fresh parse.
    /// The snapshot applies `=` rules itself when `--auto` is in force; to
    /// rebuild from what is already served, call [`Self::republish`].
    ///
    /// Takes the shared `Arc` — the shape [`JournalEditor::journal`] hands back —
    /// so republishing after an edit shares the editor's journal rather than
    /// deep-cloning it (PERF-1b).
    pub fn replace_journal(&self, journal: &Arc<Journal>) {
        let location = self.location();
        self.inner.store(Arc::new(Snapshot::from_journal(
            Arc::clone(journal),
            location,
            &self.app_dirs.hledger_conf,
        )));
    }

    /// Rebuild the snapshot from the journal it already holds — the RAW one,
    /// so `=` rules are applied afresh and never twice — and publish it under
    /// a new generation. What a change that is not in the journal's own files
    /// calls: `--auto` turned on or off in `hledger.conf` (plan 30 Decision
    /// 7) re-reads the setting here and refreshes every client.
    ///
    /// Holds the editor lock across the rebuild, as every edit's publish does,
    /// so it can never store a journal older than one an edit just published.
    pub fn republish(&self) {
        let guard = self.editor.lock().unwrap_or_else(PoisonError::into_inner);
        let snapshot = self.snapshot();
        let raw = guard.as_ref().map_or_else(
            || Arc::clone(&snapshot.raw),
            |editor| Arc::clone(editor.journal()),
        );
        self.inner.store(Arc::new(Snapshot::from_journal(
            raw,
            snapshot.location.clone(),
            &self.app_dirs.hledger_conf,
        )));
    }

    /// Re-open the bound editor from disk after an *external* change, republishing
    /// the snapshot from the freshly-read file so its rope, parsed journal, and
    /// external-change fingerprint all track what is now on disk.
    ///
    /// Returns `None` when no editor is bound (read-only state), so the file
    /// watcher can fall back to a plain reparse + hot-swap; `Some(Ok(()))` on a
    /// successful re-open, or `Some(Err(_))` if the file could not be re-read or
    /// re-parsed (the previous state is then kept).
    pub fn reopen_editor(&self) -> Option<Result<(), EditError>> {
        let mut guard = self.editor.lock().unwrap_or_else(PoisonError::into_inner);
        let editor = guard.as_mut()?;
        // `reopen`, not `open`: the real path stays pinned, so a symlink swapped
        // in since load is refused rather than followed.
        match editor.reopen() {
            Ok(reopened) => {
                self.inner.store(Arc::new(Snapshot::from_journal(
                    Arc::clone(reopened.journal()),
                    Some(location_of(&reopened)),
                    &self.app_dirs.hledger_conf,
                )));
                *editor = reopened;
                // The editor we just replaced may have been half-mutated by a
                // panic mid-edit (SEC-11). Now that it is gone, the poison flag
                // describes state that no longer exists, so clear it.
                self.editor.clear_poison();
                Some(Ok(()))
            }
            // Re-open failed, so the possibly half-mutated editor is all we have.
            // Unbind it rather than keeping it: a later edit against a poisoned,
            // partially-applied editor is how a bad write reaches the journal.
            // Callers treat `None` as read-only and fall back to a plain reparse.
            Err(error) => {
                *guard = None;
                Some(Err(error))
            }
        }
    }

    /// Rebind the editor to the journal file at `path`: open a fresh
    /// [`JournalEditor`] over it, republish the snapshot from its parsed journal,
    /// and swap it into the editor mutex. The desktop File→Open action uses this
    /// so subsequent edits target the newly-opened file (not the one the editor
    /// was previously bound to).
    ///
    /// # Errors
    /// [`EditError::Io`] if the file cannot be read, or [`EditError::Parse`] if it
    /// does not parse. On error the previously-bound editor and published snapshot
    /// are left unchanged.
    pub fn rebind_editor(&self, path: impl AsRef<Path>) -> Result<(), EditError> {
        // Open first so a failure leaves the current editor + snapshot untouched.
        self.bind_editor(JournalEditor::open(path.as_ref())?);
        Ok(())
    }

    /// [`Self::rebind_editor`] for a vault: open a fresh editor over `main` in
    /// `vault` and bind it. How a host without a desktop path (iOS) gets
    /// editing back after a failed reload unbound the editor — a file evicted
    /// mid-read, or a synced-in edit that did not parse — once the journal
    /// reads cleanly again.
    ///
    /// # Errors
    /// [`EditError::Parse`] (or `Io`) if the journal cannot be read or does not
    /// parse; the previous editor and snapshot are then left unchanged.
    pub fn rebind_vault(&self, vault: Arc<dyn Vault>, main: VaultPath) -> Result<(), EditError> {
        self.bind_editor(JournalEditor::open_in(vault, main)?);
        Ok(())
    }

    /// Publish `editor`'s journal and bind it as the editor.
    fn bind_editor(&self, editor: JournalEditor) {
        self.inner.store(Arc::new(Snapshot::from_journal(
            Arc::clone(editor.journal()),
            Some(location_of(&editor)),
            &self.app_dirs.hledger_conf,
        )));
        let mut guard = self.editor.lock().unwrap_or_else(PoisonError::into_inner);
        *guard = Some(editor);
        // The previously-bound editor is discarded wholesale, so any poison flag
        // from a panic during its lifetime no longer describes live state.
        self.editor.clear_poison();
    }

    /// The `hledger.conf` in force and what Ledgeline honours from it, as this
    /// snapshot holds it (read on first use). Reports and the `--auto`
    /// application read it from here; the settings screen reads it fresh
    /// ([`Self::refresh_conf`]).
    pub(crate) fn conf(&self) -> Arc<conf_api::ConfState> {
        let snapshot = self.snapshot();
        Arc::clone(snapshot.conf.get_or_init(|| {
            Arc::new(conf_api::ConfState::read(
                snapshot.storage().as_deref(),
                &self.app_dirs.hledger_conf,
            ))
        }))
    }

    /// What Ledgeline honours from the `hledger.conf` in force — general
    /// `--auto` and per-report `--depth` — for a host or another crate that
    /// needs it (the FFI, plan 30's Phase 5).
    #[must_use]
    pub fn conf_settings(&self) -> ledgeline_core::hledger_conf::ConfSettings {
        self.conf().settings.clone()
    }

    /// Read the config file afresh and, when it is not what the published
    /// snapshot holds, republish so every report sees it. Returns the fresh
    /// reading.
    ///
    /// The poll half of "an external change refreshes": the watcher covers
    /// the journal's directory, and this covers the rest (`~/.hledger.conf`, a
    /// parent directory, a host with no watcher).
    pub(crate) fn refresh_conf(&self) -> Arc<conf_api::ConfState> {
        let snapshot = self.snapshot();
        let fresh =
            conf_api::ConfState::read(snapshot.storage().as_deref(), &self.app_dirs.hledger_conf);
        match snapshot.conf.get() {
            Some(held) if held.differs_from(&fresh) => {
                self.republish_with_conf(fresh);
                self.conf()
            }
            Some(held) => Arc::clone(held),
            None => Arc::clone(snapshot.conf.get_or_init(|| Arc::new(fresh))),
        }
    }

    /// Publish a new snapshot of the CURRENT journal — the editor's when one is
    /// bound — carrying `conf`, so the generation changes and every client
    /// re-reads. Holding the editor lock means an edit landing meanwhile cannot
    /// be overwritten by the journal from before it.
    pub(crate) fn republish_with_conf(&self, conf: conf_api::ConfState) {
        let guard = self.editor.lock().unwrap_or_else(PoisonError::into_inner);
        let (journal, location) = match guard.as_ref() {
            Some(editor) => (Arc::clone(editor.journal()), Some(location_of(editor))),
            // The RAW journal: `snapshot.journal` already carries the rules'
            // postings when `--auto` was on, and the new conf decides afresh.
            None => {
                let snapshot = self.snapshot();
                (Arc::clone(&snapshot.raw), snapshot.location.clone())
            }
        };
        let snapshot = Snapshot::from_journal(journal, location, &self.app_dirs.hledger_conf);
        let _ = snapshot.conf.set(Arc::new(conf));
        self.inner.store(Arc::new(snapshot));
    }

    /// Every file a live-reload watcher should watch: the journal's sources
    /// ([`Self::source_files`]) plus the `hledger.conf` in force and the one a
    /// first settings write would create beside the journal — a change to
    /// either changes what the reports read.
    #[must_use]
    pub fn watched_files(&self) -> Vec<PathBuf> {
        let mut files = self.source_files();
        let location = self.snapshot().storage();
        for path in self.conf().watch_paths(location.as_deref()) {
            if !files.contains(&path) {
                files.push(path);
            }
        }
        files
    }

    /// Re-read the journal from its vault and republish it: re-open the bound
    /// editor, or — when none is bound — reparse through the vault the snapshot
    /// came from. The one reload path, used by the file watcher on a settled
    /// change and by [`Self::refresh`].
    ///
    /// # Errors
    /// [`EditError::Parse`] (or `Io`) if the journal could not be re-read; the
    /// previous snapshot is then kept.
    pub fn reload(&self) -> Result<Refreshed, EditError> {
        match self.reopen_editor() {
            Some(Ok(())) => Ok(Refreshed::Reloaded),
            Some(Err(error)) => Err(error),
            None => {
                let Some(location) = self.location() else {
                    return Ok(Refreshed::NoStorage);
                };
                let journal = location
                    .reparse(&std::collections::HashMap::new())
                    .map_err(EditError::Parse)?;
                self.replace_journal(&Arc::new(journal));
                Ok(Refreshed::Reloaded)
            }
        }
    }

    /// Check the journal's sources in its vault and [`reload`](Self::reload)
    /// only when one of them changed — the entry point for a host with no file
    /// watcher (iOS foregrounding, a pull-to-refresh). Content only: a file
    /// rewritten with identical bytes, or merely touched, is not a change.
    ///
    /// Without a bound editor there is no record of what was read, so the
    /// journal is reparsed unconditionally.
    ///
    /// # Errors
    /// As [`Self::reload`].
    pub fn refresh(&self) -> Result<Refreshed, EditError> {
        let unchanged = {
            let guard = self.editor.lock().unwrap_or_else(PoisonError::into_inner);
            guard
                .as_ref()
                .map(|editor| editor.changed_sources().is_empty())
        };
        match unchanged {
            // The journal is as it was; its config file may not be.
            Some(true) => {
                let before = self.snapshot().etag.clone();
                self.refresh_conf();
                Ok(if self.snapshot().etag == before {
                    Refreshed::Unchanged
                } else {
                    Refreshed::Reloaded
                })
            }
            _ => self.reload(),
        }
    }

    /// Where the served journal lives — its vault and main file — or `None` for
    /// a state with no backing storage.
    #[must_use]
    pub fn location(&self) -> Option<Arc<JournalLocation>> {
        self.snapshot().location.clone()
    }

    /// The current snapshot (a cheap atomic load; the returned `Arc` is a stable
    /// view for the duration of one request even if a swap happens meanwhile).
    pub(crate) fn snapshot(&self) -> Arc<Snapshot> {
        self.inner.load_full()
    }

    /// Every file the currently-published journal was parsed from: the main file
    /// plus every `include`d file (directive-only includes included), as resolved
    /// absolute paths. The live-reload watcher uses this to monitor the complete
    /// set of files an edit to any of which should trigger a reparse. Reflects the
    /// latest hot-swap, so re-reading it after a reload picks up include changes.
    #[must_use]
    pub fn source_files(&self) -> Vec<PathBuf> {
        self.snapshot().journal.source_files.clone()
    }

    /// The write-path editor mutex, shared by all clones. Used by [`edit_api`] to
    /// serialize edits; `None` inside means editing is disabled for this state.
    pub(crate) fn editor(&self) -> &Mutex<Option<JournalEditor>> {
        &self.editor
    }

    /// Whether a journal file is bound to an editor — i.e. whether this server
    /// may write at all. Read WITHOUT holding the guard across anything, so no
    /// caller is tempted to carry a `std::sync::MutexGuard` into an `.await`.
    ///
    /// A poisoned mutex still answers honestly: the flag says an earlier request
    /// panicked mid-edit, not that the binding went away, and [`edit_api`]'s
    /// `lock_editor` is what recovers from it (SEC-11).
    #[must_use]
    pub fn editing_enabled(&self) -> bool {
        self.editor
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some()
    }

    /// The rules-file write mutex, shared by all clones. See the field's own
    /// docs for why the journal editor's mutex does not cover these writes.
    pub(crate) fn rules_writes(&self) -> &tokio::sync::Mutex<()> {
        &self.rules_writes
    }

    /// Where the app keeps its own state. See the field's docs.
    #[must_use]
    pub fn app_dirs(&self) -> &AppDirs {
        &self.app_dirs
    }

    /// Use `dirs` for the app's own state instead of [`AppDirs::from_env`]. A
    /// sandboxed host (iOS) calls this right after construction, before the
    /// state serves anything; the desktop calls it with the `from_env` value it
    /// already built, so the binary and the library agree on one set.
    ///
    /// The staging area is replaced with an empty one under the new
    /// [`AppDirs::temp_dir`]. Call it before any upload — a stage made earlier
    /// would be dropped (and its directory removed) with the old area.
    #[must_use]
    pub fn with_app_dirs(mut self, dirs: AppDirs) -> Self {
        self.stages = Arc::new(stage::StageArea::new(dirs.temp_dir.clone()));
        self.app_dirs = Arc::new(dirs);
        // Re-read the journal's snapshot under the new user directories: the
        // first one was built before they were known, so a `--auto` in
        // `~/.hledger.conf` would otherwise not apply until the next edit.
        let snapshot = self.snapshot();
        self.inner.store(Arc::new(Snapshot::from_journal(
            Arc::clone(&snapshot.raw),
            snapshot.location.clone(),
            &self.app_dirs.hledger_conf,
        )));
        self
    }

    /// This session's staging area, shared by all clones. See the field's docs.
    pub(crate) fn stages(&self) -> &stage::StageArea {
        &self.stages
    }

    /// This session's staged QuickBooks Journal uploads, shared by all clones.
    /// See the field's docs.
    pub(crate) fn qb_stages(&self) -> &qb_journal_api::QbStageArea {
        &self.qb_stages
    }

    /// The import write mutex, shared by all clones. See the field's own docs
    /// for why neither of the other two covers an import.
    pub(crate) fn import_writes(&self) -> &tokio::sync::Mutex<()> {
        &self.import_writes
    }

    /// This state's quote source. `pub(crate)` — only `prices_api` reads it.
    pub(crate) fn price_source(&self) -> &Arc<dyn yahoo::PriceFeed> {
        &self.price_source
    }

    /// Substitute a different quote source, replacing the real Yahoo client.
    /// `pub`, not `pub(crate)`: the integration tests build this crate's
    /// `AppState` as an ordinary external dependency, not with this crate's own
    /// `#[cfg(test)]` in force, so a `pub(crate)` method would not be visible to
    /// them. Every other `AppState` field stays journal-derived; this is the one
    /// piece of state a caller — in practice, only a test — has any reason to
    /// override.
    #[must_use]
    pub fn with_price_source(mut self, source: Arc<dyn yahoo::PriceFeed>) -> Self {
        self.price_source = source;
        self
    }

    /// This state's classification source and cache. `pub(crate)` — only
    /// `profiles_api` reads it.
    pub(crate) fn profiles(&self) -> &Arc<profiles_api::Profiles> {
        &self.profiles
    }

    /// Substitute a different classification source (with a fresh, empty
    /// cache), replacing the real Yahoo client. `pub` for the integration
    /// tests, for the reason [`Self::with_price_source`] gives.
    #[must_use]
    pub fn with_profile_source(mut self, source: Arc<dyn yahoo_profile::ProfileFeed>) -> Self {
        self.profiles = Arc::new(profiles_api::Profiles::new(source));
        self
    }

    /// This state's benchmark history source. `pub(crate)` — only
    /// `benchmarks_api` reads it.
    pub(crate) fn history_source(&self) -> &Arc<dyn yahoo_history::HistoryFeed> {
        &self.history_source
    }

    /// The benchmark price cache, shared by all clones.
    pub(crate) fn benchmark_cache(&self) -> &sidecar::SidecarCache<benchmarks_api::BenchmarkCache> {
        &self.benchmark_cache
    }

    /// Substitute a different benchmark history source, replacing the real
    /// Yahoo client. `pub` for [`Self::with_price_source`]'s reason: only the
    /// integration tests, an external crate, call it.
    #[must_use]
    pub fn with_history_source(mut self, source: Arc<dyn yahoo_history::HistoryFeed>) -> Self {
        self.history_source = source;
        self
    }
}

/// What [`AppState::refresh`] / [`AppState::reload`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refreshed {
    /// Every source still holds the bytes last read; nothing was reparsed.
    Unchanged,
    /// The journal was re-read and the snapshot republished.
    Reloaded,
    /// The state has no backing storage to re-read.
    NoStorage,
}

/// The location an editor reads and writes.
fn location_of(editor: &JournalEditor) -> Arc<JournalLocation> {
    Arc::new(JournalLocation::new(
        Arc::clone(editor.vault()),
        editor.main().clone(),
    ))
}

/// Build an UNAUTHENTICATED router for a parsed `journal`.
///
/// Convenience constructor for the `tower::oneshot` integration tests, which
/// drive routes in-process with no socket. It applies [`Security::open`] — no
/// token, no `Host` guard, no CORS — so a bound server must use
/// [`router_with_security`] instead.
pub fn app(journal: &Journal) -> Router {
    router_with_state(AppState::from_journal(journal))
}

/// Build an UNAUTHENTICATED router from precomputed [`AppState`].
///
/// Same caveat as [`app`]: [`Security::open`] means anything that can reach the
/// socket can read and rewrite the journal. Only for in-process test harnesses.
pub fn router_with_state(state: AppState) -> Router {
    router_with_security(state, Security::open())
}

/// Build the router that a bound server should serve: every wire and `/api`
/// route behind the bearer token, every response behind the `Host` guard, the
/// security headers, and the panic catcher.
///
/// Layer order matters and is asserted by the integration tests. Each `.layer`
/// wraps what came before it, so the LAST call is the outermost:
/// * The security headers are outermost, so every response carries them —
///   including the `500` the panic catcher synthesises.
/// * `CatchPanicLayer` sits just inside them, so a panic anywhere below — the
///   guards included — becomes a `500` rather than a dropped connection (SEC-2).
/// * The `Host` guard wraps routing and the SPA fallback both, so a rebound host
///   cannot even fetch the shell.
/// * CORS (installed only when [`Security::allow_origins`] was given exact
///   origins) is a `route_layer`, so it covers exactly the wire and `/api`
///   routes and NEVER the SPA fallback. It still sits inside the `Host` guard,
///   so preflights are answered only for requests that were addressed to us
///   properly in the first place.
/// * The token guard is a `route_layer` too, installed *before* CORS so that it
///   is the inner of the two: a preflight carries no `Authorization`, so the
///   CORS layer has to be able to answer it without the token guard seeing it.
///   Like CORS it covers exactly the routes below and never the SPA shell or its
///   assets, which must stay reachable to bootstrap.
///
/// # Why CORS is a `route_layer` and not a `layer` (SEC-12)
///
/// It used to be `.layer(...)`, applied after `.fallback(...)`, and so it
/// covered the fallback as well. The fallback is the SPA shell, and the shell
/// then carried the access token in its body. That combination meant a single
/// `fetch("http://127.0.0.1:<port>/")` from any page on an allowlisted dev
/// origin — another Vite project, a malicious dev dependency — was answered
/// with `Access-Control-Allow-Origin`, so the browser handed that page the
/// token, and with it full read/write on the user's journal. `--allow-origin`
/// is meant to open the API to a dev SPA; it must not also publish the
/// credential. (The shell no longer carries the token at all — see
/// `spa.rs` — but CORS stays off it regardless.)
///
/// The documented cross-origin flows do not need the shell: `just dev` and
/// `web/playwright.config.ts` both serve the SPA from vite/`preview` and pin the
/// token through `$LEDGELINE_TOKEN`, which the specs seed into `localStorage`
/// alongside `serverUrl`. They only ever fetch the routes below.
///
/// Compression (PERF-2) sits between the `Host` guard and routing, so it sees
/// every route's body and the SPA's assets, but never a response the guards
/// refused. It is opt-in per request — a client that sends no `Accept-Encoding`
/// still gets the identity bytes straight out of the snapshot.
pub fn router_with_security(state: AppState, security: Security) -> Router {
    let router = Router::new()
        .route("/version", get(version))
        .route("/accountnames", get(accountnames))
        .route("/transactions", get(transactions))
        .route("/prices", get(prices))
        .route("/commodities", get(commodities))
        .route("/accounts", get(accounts))
        // Journal-wide problems: the engine diagnostics plus the local check
        // rules, as of the client's `?today=` (see `journal_api`). A NATIVE
        // route, not a wire one: `/transactions` is a byte-parity emulation of
        // hledger-web's endpoint and has nowhere to carry a sibling field.
        .route("/api/diagnostics", get(journal_api::diagnostics))
        // Plan 30 Phase 5: the main journal's `=` rules and whether `--auto`
        // is in force (Settings › Automatic postings), and (amendment 22) their
        // editor's one write.
        // `PUT` rewrites one of them in the user's journal, so it is token-gated
        // with the GET (`auto_endpoints.rs` pins the 401).
        .route("/api/auto-rules", get(auto_api::rules).put(auto_api::save))
        // Which journal is open: its derived title and its main file's bare
        // name. A NATIVE route for the same reason as `/api/diagnostics` — the
        // wire routes are byte-parity emulations of hledger-web's and have
        // nowhere to carry this. Registered above the `route_layer` below so it
        // is token-gated with everything else: it is the only route that says
        // anything at all about the journal's own file.
        .route("/api/journal", get(journal_info))
        // Journal-view routes (plan 26 B3-B5): see `journal_api`.
        .route("/api/balances/cash", get(journal_api::cash_balances))
        .route("/api/journal/rows", get(journal_api::rows))
        .route("/api/journal/summary", get(journal_api::summary_route))
        .route("/api/journal/chart", get(journal_api::chart_route))
        .route("/api/journal/position", get(journal_api::position_route))
        .route("/api/reports/balancesheet", get(reports_api::balancesheet))
        // The grouped/valued three-box balance sheet. A SIBLING route, not a
        // mode of the one above: that one is a flat hledger-parity shape with a
        // committed golden, and this one answers a different question.
        .route(
            "/api/reports/balancesheet/grouped",
            get(reports_api::balancesheet_grouped),
        )
        // The same grouped sheet at the end of each period: the Reports tab's
        // multi-period mode, in the pivoted shape the projection run returns.
        .route(
            "/api/reports/balancesheet/columns",
            get(reports_api::balancesheet_columns),
        )
        .route(
            "/api/reports/incomestatement",
            get(reports_api::incomestatement),
        )
        // The grouped/valued adaptive-GAAP P&L, a sibling of the flat route
        // above for the same reason the balance sheet has one.
        .route(
            "/api/reports/incomestatement/grouped",
            get(reports_api::incomestatement_grouped),
        )
        // The two money-flow graphs the P&L tab draws above its boxes. Its own
        // route rather than fields on the grouped report: it is a second pass
        // over every posting, and the panels that show it are collapsible.
        .route(
            "/api/reports/incomestatement/flows",
            get(reports_api::incomestatement_flows),
        )
        .route("/api/reports/cashflow", get(reports_api::cashflow))
        .route("/api/reports/accounts", get(reports_api::accounts))
        .route("/api/reports/networth", get(reports_api::networth))
        .route("/api/insights", get(reports_api::insights_report))
        .route("/api/subscriptions", get(reports_api::subscriptions))
        .route("/api/budget", get(reports_api::budget))
        // The unbudgeted categories under those bars. A sibling route, not a
        // field on the report above: the report is refetched on every control
        // change and after every goal save, and this one answers a section the
        // Budget tab opens collapsed.
        //
        // Registered HERE, with the reports, rather than beside
        // `/api/budget/lines` below — it reads the journal and writes nothing,
        // exactly like its neighbours. Either position is above the
        // `route_layer`, which is what matters; `budget_endpoints.rs` pins the
        // 401 for it alongside the editor's four.
        .route("/api/budget/gaps", get(reports_api::budget_gaps_report))
        // Projections: run a what-if, and seed one from the journal.
        //
        // `run` is a POST that WRITES NOTHING — the scenario travels in the body
        // so that unsaved edits project (decision 3 of
        // `plans/22-projections.md`), not because anything is being stored. It
        // is registered HERE, above the `route_layer`, for the same reason its
        // read-only neighbours are: below it the route would serve a projection
        // of the user's finances with no bearer token at all.
        // `projection_endpoints.rs` pins the 401 for both.
        //
        // The body limit is set on THIS route alone, exactly as
        // `/api/import/stage` sets its own — here it is LOWERED from axum's
        // 2 MiB default to what a scenario can plausibly need, because the body
        // is held while a `compute` slot is claimed.
        .route(
            "/api/projections/run",
            post(projections_api::run).route_layer(axum::extract::DefaultBodyLimit::max(
                projections_api::MAX_BODY_BYTES,
            )),
        )
        .route("/api/projections/seed", get(projections_api::seed))
        // Projection scenario FILES (plan 22, Phase 3): list, read, save.
        //
        // THE SAME PLACEMENT TRAP as the rules routes below, and the same
        // consequence: `PUT /api/projections/{*id}` is a write primitive over
        // a file in the user's journal directory, and below the `route_layer`
        // it would be reachable with no bearer token at all.
        // `projection_endpoints.rs` pins the 401 for all three.
        //
        // The two fixed segments above (`run`, `seed`) MUST stay registered
        // before `{*id}`: axum matches a literal ahead of a wildcard, so the
        // order is not what disambiguates them — but keeping them adjacent and
        // in this order is what makes that readable. An id always ends in
        // `.journal`, so neither literal is a reachable id anyway.
        //
        // A save's body is `{revision, scenario}`: the same scenario `run`
        // takes plus one fingerprint string, so `run`'s limit fits it too and
        // the two cannot disagree about how big a scenario may be. (The GET
        // carries no body, so the limit costs it nothing.)
        .route("/api/projections", get(projections_api::index))
        .route(
            "/api/projections/{*id}",
            get(projections_api::document)
                .put(projections_api::save)
                .route_layer(axum::extract::DefaultBodyLimit::max(
                    projections_api::MAX_BODY_BYTES,
                )),
        )
        .route("/api/holdings", get(reports_api::holdings))
        .route("/api/holdings/profiles", get(profiles_api::profiles))
        .route(
            "/api/holdings/series",
            get(reports_api::holdings_series_report),
        )
        .route(
            "/api/holdings/other",
            get(reports_api::other_holdings_report),
        )
        .route(
            "/api/holdings/other/series",
            get(reports_api::other_holdings_series_report),
        )
        // Benchmark overlay for the Stocks value-over-time chart. A GET, but it
        // may fetch from Yahoo Finance and write the benchmark cache beside the
        // journal, so it belongs above the token guard with everything else.
        .route("/api/holdings/benchmarks", get(benchmarks_api::benchmarks))
        .route(
            "/api/holdings/benchmarks/catalog",
            get(benchmarks_api::catalog),
        )
        // Write path (Phase 5.2+): add / delete / replace (PUT) / partial-edit
        // (PATCH) a transaction through the editor.
        .route("/api/transactions", post(edit_api::add_transaction))
        .route(
            "/api/transactions/{index}",
            get(journal_api::transaction)
                .delete(edit_api::delete_transaction)
                .put(edit_api::replace_transaction)
                .patch(edit_api::patch_transaction),
        )
        // CSV import rules (Imports, steps 7-8): list, read, preview, save.
        //
        // THESE MUST STAY ABOVE the `route_layer` below. `route_layer` covers
        // exactly the routes registered before it, so a route added *after* it
        // is reachable with no bearer token at all — and `PUT /api/rules/{id}`
        // is a write primitive over a file in the user's journal directory.
        // `rules_endpoints.rs` pins the 401, in the same shape as the SEC-1
        // tests, so moving these lines fails a test rather than shipping.
        //
        // `preview` is on its own PREFIX rather than `/api/rules/{*id}/preview`
        // because axum 0.8's matcher refuses a catch-all with anything after it
        // ("Insertion failed due to conflict"), and a greedy `{*id}` would
        // swallow the suffix even if it did not. The id semantics are identical
        // on both prefixes: the same string, validated and resolved the same way.
        .route("/api/rules", get(rules_api::index))
        .route(
            "/api/rules/{*id}",
            get(rules_api::document).put(rules_api::save),
        )
        .route("/api/rules-preview/{*id}", get(rules_api::preview))
        // Drafting a rules file for a dropped CSV that has none (WP-16 Phase
        // 2). A SIBLING prefix for the same reason `rules-preview` is one, and
        // deliberately not a `POST` on `/api/rules` — the id it takes names a
        // file that does not exist yet, so it belongs to no id-keyed route.
        //
        // This one WRITES NOTHING; the follow-up `PUT /api/rules/{*id}` above
        // does, when the user is happy with the draft. It is still inside the
        // token layer, because it reads the staged upload and the journal's own
        // directory tree.
        .route("/api/rules-create", post(rules_api::create))
        // Enhanced imports (WP-11): capabilities, upload, dry-run, commit,
        // re-sort, and the preferences store.
        //
        // THE SAME PLACEMENT TRAP AS THE RULES ROUTES ABOVE, and a worse one to
        // fall into: `POST /api/import/commit` writes a CSV into the user's
        // journal directory and appends to a journal file. Below the
        // `route_layer` it would do that with no bearer token at all.
        // `import_endpoints.rs::every_import_route_requires_the_token` pins the
        // 401 for every one of these.
        .route("/api/import/capabilities", get(import_api::capabilities))
        .route(
            "/api/import/stage",
            // The ONLY route with a RAISED body limit (the projection routes
            // above set a lower one), and it is raised on this route alone
            // rather than globally: every other endpoint takes a small JSON
            // body, and a global limit would lift the ceiling on all of them for
            // the sake of one. `route_layer` here applies to just this route's
            // method handler.
            post(import_api::stage).route_layer(axum::extract::DefaultBodyLimit::max(
                stage::MAX_UPLOAD_BYTES,
            )),
        )
        .route("/api/import/dry-run", post(import_api::dry_run))
        .route("/api/import/commit", post(import_api::commit))
        // The no-rules-file path: keep the converted CSV, import nothing. Its
        // own route rather than a commit with null handles — a dry-run with no
        // rules file has nothing to propose, so nullable handles would encode a
        // state that cannot happen. It writes a file into the user's journal
        // directory, so it belongs above the `route_layer` with the rest.
        .route("/api/import/save-csv", post(import_api::save_csv))
        .route("/api/import/sort", post(import_api::sort_journal))
        // The QuickBooks Journal import (WP-17 Phase B): the same upload route
        // above already detects and stages this second format; these two read
        // the staged parse and write the journal. THE SAME PLACEMENT TRAP as
        // the routes above — `POST /api/import/qb-journal/commit` writes
        // straight into the user's journal. Below the `route_layer` it would
        // do that with no bearer token at all.
        .route(
            "/api/import/qb-journal/{stageId}",
            get(qb_journal_api::preview),
        )
        .route(
            "/api/import/qb-journal/commit",
            post(qb_journal_api::commit),
        )
        // The command-line-parity fix: install the journal's aliases into an
        // `hledger.conf` beside it. A THIRD write target, and the only one that
        // is not a journal or a CSV, so it belongs above the `route_layer` for
        // the same reason as everything else here. It never writes outside the
        // journal's own directory — see `import_api::resolve_conf`.
        .route(
            "/api/import/hledger-conf",
            post(import_api::write_hledger_conf),
        )
        .route(
            "/api/prefs",
            get(import_api::prefs_get).put(import_api::prefs_put),
        )
        // Settings › hledger config (plan 30, Decision 10): the config file in
        // force, and `--auto` / `--depth` written into it. THE SAME PLACEMENT
        // TRAP as the routes above: the `PUT` rewrites a file every hledger
        // command the user runs reads — possibly `~/.hledger.conf`. Below the
        // `route_layer` it would do that unauthenticated.
        // `conf_endpoints.rs::every_conf_route_requires_the_token` pins the 401.
        .route("/api/hledger-conf", get(conf_api::get).put(conf_api::put))
        // Account aliases (enhanced imports): the mapping table an import
        // forwards to `hledger --alias`, listed and edited in place.
        //
        // THE SAME PLACEMENT TRAP, and the worst instance of it in the file:
        // `PUT /api/aliases/{*id}` rewrites a line of the user's JOURNAL, which
        // is the most valuable file this application touches. Below the
        // `route_layer` it would do that unauthenticated.
        // `alias_endpoints.rs::every_alias_route_requires_the_token` pins the 401.
        .route("/api/aliases", get(alias_api::index))
        .route("/api/aliases/{*id}", axum::routing::put(alias_api::save))
        // Account list editor (Settings): `account` declarations, listed and
        // edited in place, plus the one-time creation of an `accounts.journal`
        // for a ledger that has nowhere to put a first one.
        //
        // THE SAME PLACEMENT TRAP as the routes above, twice over:
        // `PUT /api/accounts/{*id}` rewrites a line of the user's JOURNAL, and
        // `POST /api/accounts/file` CREATES a file beside it and appends an
        // `include` to the main journal. Below the `route_layer` both would
        // happen unauthenticated.
        // `account_endpoints.rs::every_account_route_requires_the_token` pins
        // the 401 for all three.
        .route("/api/accounts", get(account_api::index))
        .route("/api/accounts/{*id}", axum::routing::put(account_api::save))
        .route("/api/accounts/file", post(account_api::create_file))
        // Budget goals: the `~` periodic rules `/api/budget` reports against,
        // listed and edited in place, plus the two routes the editor needs
        // around them — the per-account history strip, and the one-time
        // creation of a `budget.journal` for a ledger that has nowhere to put a
        // first goal.
        //
        // THE SAME PLACEMENT TRAP as the routes above, twice over:
        // `PUT /api/budget/lines/{*id}` rewrites a line of the user's JOURNAL,
        // and `POST /api/budget/file` CREATES a file beside it and appends an
        // `include` to the main journal. Below the `route_layer` both would
        // happen unauthenticated.
        // `budget_endpoints.rs::every_budget_route_requires_the_token` pins the
        // 401 for all four.
        .route("/api/budget/lines", get(budget_api::index))
        .route(
            "/api/budget/lines/{*id}",
            axum::routing::put(budget_api::save),
        )
        .route("/api/budget/file", post(budget_api::create_file))
        .route("/api/budget/reference", get(budget_api::reference))
        // Stock price updates (Holdings tab): which symbols need a quote and
        // where prices already live, creating a first `prices.journal`, and
        // fetching + appending quotes from Yahoo Finance.
        //
        // THE SAME PLACEMENT TRAP as the routes above: `POST /api/prices/file`
        // CREATES a file beside the journal and appends an `include`, and
        // `POST /api/prices/update` rewrites a line of the user's JOURNAL.
        // Below the `route_layer` both would happen unauthenticated.
        // `prices_endpoints.rs::every_prices_route_requires_the_token` pins the
        // 401 for all three.
        .route("/api/prices/status", get(prices_api::status))
        .route("/api/prices/file", post(prices_api::create_file))
        .route("/api/prices/update", post(prices_api::update))
        // Token-gate exactly the routes registered above. `route_layer` skips
        // the fallback, which is what lets the browser fetch the shell before it
        // has any credential to present.
        .route_layer(middleware::from_fn_with_state(
            security.clone(),
            security::token_guard,
        ));

    // CORS for the allowlisted origins, over exactly the routes above. A second
    // `route_layer` wraps the first, so this ends up OUTSIDE the token guard —
    // which is required, because a preflight carries no `Authorization` and must
    // be answered rather than refused.
    //
    // `route_layer`, never `layer`: see this function's docs. The default
    // posture is no CORS layer at all, so a browser refuses to hand any
    // cross-origin page our responses (SEC-1).
    let router = match security.cors_layer() {
        Some(cors) => router.route_layer(cors),
        None => router,
    };

    // Everything else (the SPA shell, its embedded assets, and client-side deep
    // links) is served same-origin ONLY; the explicit routes above win, and
    // neither `route_layer` above reaches this.
    let router = router.fallback(spa::fallback);

    // `Fastest` (gzip level 1), not the default level 6: the /transactions body
    // is hundreds of megabytes and this server's usual peer is the SPA on the
    // same machine, where a second of CPU spent squeezing out a few more percent
    // is a straight loss. Level 1 still gets the bulk of the ~20:1 ratio this
    // payload compresses at, and it is what makes a LAN or `--allow-origin`
    // client affordable.
    let router = router.layer(CompressionLayer::new().quality(CompressionLevel::Fastest));

    router
        .layer(middleware::from_fn_with_state(
            security,
            security::host_guard,
        ))
        .layer(CatchPanicLayer::new())
        .layer(SetResponseHeaderLayer::overriding(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::REFERRER_POLICY,
            HeaderValue::from_static("no-referrer"),
        ))
        // `if_not_present` so the SPA shell's own policy — which additionally
        // hashes the inline scripts it just rendered — is left alone.
        .layer(SetResponseHeaderLayer::if_not_present(
            header::CONTENT_SECURITY_POLICY,
            security::base_csp(),
        ))
        .with_state(state)
}

/// Serve one precomputed body from the current snapshot as
/// `application/json`, or `304 Not Modified` when the client's `If-None-Match`
/// already names this snapshot.
///
/// `Bytes::clone` is a refcount bump, so the 347 MB `/transactions` body is
/// handed to the response without a copy and without re-serializing (PERF-1).
/// `Cache-Control: no-cache` is "you may store this, but revalidate every time",
/// which is exactly the contract the `ETag` needs: journal data must never be
/// served from a cache without asking us first (PERF-2).
fn serve(snapshot: &Snapshot, headers: &HeaderMap, body: &Bytes) -> Response {
    let builder = Response::builder()
        .header(header::ETAG, snapshot.etag.clone())
        .header(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    let response = if if_none_match_hits(headers, &snapshot.etag) {
        builder.status(StatusCode::NOT_MODIFIED).body(Body::empty())
    } else {
        builder
            .status(StatusCode::OK)
            .header(
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            )
            .body(Body::from(body.clone()))
    };
    // `Response::builder` only fails on a malformed header, and every header
    // here is either a constant or an already-validated `HeaderValue`.
    response.unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

/// Does the request's `If-None-Match` name `etag`?
///
/// RFC 9110 §13.1.2: the field is `*` or a comma-separated list of entity tags,
/// and a `GET` compares them *weakly* — so a `W/` prefix on either side is
/// ignored. Ours are always strong, but a proxy may re-tag them.
fn if_none_match_hits(headers: &HeaderMap, etag: &HeaderValue) -> bool {
    let Some(field) = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
    else {
        return false;
    };
    let opaque = |tag: &str| tag.trim().trim_start_matches("W/").to_string();
    let ours = opaque(etag.to_str().unwrap_or_default());
    field
        .split(',')
        .any(|candidate| candidate.trim() == "*" || opaque(candidate) == ours)
}

// Each handler serves its endpoint's body from the current snapshot. The journal
// is parsed and each body serialized once per snapshot (in
// `Snapshot::from_journal`); a request only bumps a refcount.
async fn version(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let snapshot = state.snapshot();
    serve(&snapshot, &headers, &snapshot.version)
}

async fn accountnames(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let snapshot = state.snapshot();
    serve(&snapshot, &headers, &snapshot.accountnames)
}

/// `/transactions` is serialized on first request (see
/// [`Snapshot::transactions`]): on the blocking pool, so the 563 ms at 200k
/// never stalls a runtime worker, and at most once per snapshot. A revalidation
/// that hits needs no body at all, so it is answered before any of that.
async fn transactions(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let snapshot = state.snapshot();
    if let Some(body) = snapshot.transactions.get() {
        return serve(&snapshot, &headers, body);
    }
    if if_none_match_hits(&headers, &snapshot.etag) {
        return serve(&snapshot, &headers, &Bytes::new());
    }
    let worker = Arc::clone(&snapshot);
    let built = reports_api::compute(move || {
        Ok(worker
            .transactions
            .get_or_init(|| json_bytes(&wire::journal_to_transactions(&worker.journal)))
            .clone())
    })
    .await;
    match built {
        Ok(body) => serve(&snapshot, &headers, &body.0),
        Err(error) => error.into_response(),
    }
}

async fn prices(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let snapshot = state.snapshot();
    serve(&snapshot, &headers, &snapshot.prices)
}

async fn commodities(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let snapshot = state.snapshot();
    serve(&snapshot, &headers, &snapshot.commodities)
}

async fn accounts(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let snapshot = state.snapshot();
    serve(&snapshot, &headers, &snapshot.accounts)
}

async fn journal_info(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let snapshot = state.snapshot();
    serve(&snapshot, &headers, &snapshot.journal_info)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers_with(value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::IF_NONE_MATCH,
            HeaderValue::from_str(value).expect("test header is valid"),
        );
        headers
    }

    /// The whole risk of a conditional GET is answering `304` for data the
    /// client does NOT already have, so this is the matcher's contract in full.
    #[test]
    fn if_none_match_matches_exactly_the_forms_rfc_9110_defines() {
        let ours = HeaderValue::from_static("\"abc-1\"");

        // Hits: exact, weak on either side, `*`, and anywhere in a list.
        for field in [
            "\"abc-1\"",
            "W/\"abc-1\"",
            "*",
            "\"other\", \"abc-1\"",
            "\"abc-1\", \"other\"",
            "  \"abc-1\"  ",
        ] {
            assert!(
                if_none_match_hits(&headers_with(field), &ours),
                "If-None-Match: {field} should match {ours:?}"
            );
        }

        // Misses: a different tag, a PREFIX of ours, ours as a prefix of theirs,
        // and an unquoted spelling. Each of these answering 304 would strand the
        // client on data it never received.
        for field in ["\"abc-2\"", "\"abc\"", "\"abc-11\"", "abc-1", "\"\""] {
            assert!(
                !if_none_match_hits(&headers_with(field), &ours),
                "If-None-Match: {field} must NOT match {ours:?}"
            );
        }

        // No header at all is never a match.
        assert!(!if_none_match_hits(&HeaderMap::new(), &ours));
    }

    /// A generation counter is only safe if it never repeats within a process.
    #[test]
    fn every_snapshot_gets_a_distinct_quoted_tag() {
        let tags: std::collections::BTreeSet<String> = (0..64)
            .map(|_| {
                let tag = next_etag();
                let text = tag.to_str().expect("ASCII").to_string();
                assert!(
                    text.starts_with('"') && text.ends_with('"'),
                    "an entity-tag must be quoted: {text}"
                );
                text
            })
            .collect();
        assert_eq!(tags.len(), 64, "generation counter must not repeat");
    }
}
