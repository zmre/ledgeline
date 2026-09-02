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
//! them on every request). The native report/budget endpoints ([`reports_api`])
//! instead depend on request query params, so they are computed per request from
//! the parsed [`Journal`] the same snapshot holds.
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

mod alias_api;
mod budget_api;
mod edit_api;
mod error;
mod git;
mod hledger;
mod import_api;
mod prefs;
mod prices_api;
mod qb_journal_api;
mod reports_api;
mod rules_api;
mod security;
mod spa;
mod stage;
mod yahoo;

use arc_swap::ArcSwap;
use axum::{
    Router,
    body::{Body, Bytes},
    extract::State,
    http::{HeaderMap, HeaderValue, StatusCode, Uri, header},
    middleware,
    response::{IntoResponse, Response},
    routing::{delete, get, post},
};
use ledgeline_core::{EditError, Journal, JournalEditor, wire};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use tower_http::CompressionLevel;
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::compression::CompressionLayer;
use tower_http::set_header::SetResponseHeaderLayer;

pub use security::{
    AccessToken, ProcessToken, Security, SecurityError, TOKEN_ENV, token_from_env_or_random,
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
    CliImport, CliImportReport, CliImportWritten, CliQbReport, CliQbWritten, CliRunReport,
    run_cli_import,
};
// `PriceFeed` (+ `FetchedPrice`/`YahooError`, reachable through its method
// signature) is re-exported so the integration tests can build a fake and hand
// it to `AppState::with_price_source` — the only way `/api/prices/update` is
// tested without a live network call.
pub use yahoo::{FetchedPrice, PriceFeed, YahooError};

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
    /// The parsed journal, shared with the per-request report handlers.
    pub(crate) journal: Arc<Journal>,
    /// This snapshot's `ETag`, identical across every endpoint it serves: all of
    /// them are derived from the same journal and change together. See
    /// [`next_etag`] for why it is a counter and not a content hash.
    pub(crate) etag: HeaderValue,
    pub(crate) version: Bytes,
    pub(crate) accountnames: Bytes,
    pub(crate) transactions: Bytes,
    pub(crate) prices: Bytes,
    pub(crate) commodities: Bytes,
    pub(crate) accounts: Bytes,
    /// The `{"diagnostics": [...]}` payload: every unbalanced transaction and
    /// failed balance assertion in the journal, plus the three stock findings
    /// (unknown cost basis, net-negative shares, unpriced position), each shaped
    /// like the SPA's `Problem`. Precomputed with the rest because all of them
    /// are whole-journal passes, and republished on every hot-swap so it never
    /// goes stale.
    ///
    /// The stock half used to be recomputed in the browser from a second, drifted
    /// copy of the holdings engine (DRY-1); it is served from here so the
    /// Problems drawer and the Holdings page cannot disagree about the same
    /// journal.
    pub(crate) diagnostics: Bytes,
    /// The `{"title": …, "file": …}` payload: which journal the user is looking
    /// at (see `ledgeline_core::title`). Precomputed with the rest so it swaps
    /// atomically with them — opening a different journal must never leave a
    /// stale name sitting over fresh numbers — but built INLINE rather than on a
    /// thread of its own: it is two `Option<String>`s read off data the parse
    /// already captured, next to no work beside the whole-journal passes above.
    pub(crate) journal_info: Bytes,
}

/// The `{"diagnostics": [...]}` envelope. `wire` exposes the array and the
/// wrapped `Value` but not a wrapped *serializable*, and building the `Value`
/// only to re-serialize it is exactly what PERF-1 is about.
#[derive(Serialize)]
struct DiagnosticsBody<'a> {
    diagnostics: &'a [wire::WireDiagnostic],
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
    /// The eight payloads are independent read-only passes over the same journal,
    /// and `/transactions` alone is roughly half the work (563 ms of the 1,048 ms
    /// at 200k). Building it on one extra thread while the rest run here overlaps
    /// the two halves almost perfectly, which is what keeps this — the bulk of
    /// app startup, of every live-reload, and of every edit's republish — near
    /// the cost of its single most expensive payload.
    ///
    /// `/api/diagnostics` takes a thread of its own for the same reason. It grew
    /// a holdings pass when the stock findings moved out of the browser (DRY-1):
    /// 131 ms at 200k on top of the balance and assertion checks' 139 ms, which
    /// is enough to make the remainder — about 485 ms — overtake `/transactions`
    /// and become the critical path. On its own thread the three groups are
    /// 563 / 346 / 270 ms and the build still costs what its largest payload does.
    fn from_journal(journal: Arc<Journal>) -> Self {
        let (transactions, diagnostics, rest) = std::thread::scope(|scope| {
            let transactions = scope.spawn(|| json_bytes(&wire::journal_to_transactions(&journal)));
            let diagnostics = scope.spawn(|| {
                json_bytes(&DiagnosticsBody {
                    diagnostics: &wire::journal_to_all_diagnostics(&journal),
                })
            });
            let rest = (
                json_bytes(&wire::version_value()),
                json_bytes(&wire::journal_to_accountnames(&journal)),
                json_bytes(&wire::journal_to_prices(&journal)),
                json_bytes(&wire::journal_to_commodities(&journal)),
                json_bytes(&wire::journal_to_accounts(&journal)),
                json_bytes(&wire::journal_to_info(&journal)),
            );
            // The only way a join fails is a panic in the serializer, which would
            // have aborted the request anyway; resume it on this thread so
            // `CatchPanicLayer` still turns it into a 500 rather than a snapshot
            // silently missing a payload.
            let joined = |handle: std::thread::ScopedJoinHandle<'_, Bytes>| {
                handle
                    .join()
                    .unwrap_or_else(|payload| std::panic::resume_unwind(payload))
            };
            (joined(transactions), joined(diagnostics), rest)
        });
        let (version, accountnames, prices, commodities, accounts, journal_info) = rest;
        Self {
            etag: next_etag(),
            version,
            accountnames,
            transactions,
            prices,
            commodities,
            accounts,
            diagnostics,
            journal_info,
            journal,
        }
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
}

/// The default [`yahoo::PriceFeed`]: the real Yahoo Finance chart endpoint,
/// over one shared `reqwest::Client` (connection pooling — a fresh client per
/// request would re-negotiate TLS on every symbol).
fn default_price_source() -> Arc<dyn yahoo::PriceFeed> {
    Arc::new(yahoo::YahooClient::new(reqwest::Client::new()))
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
        Self {
            inner: Arc::new(ArcSwap::from_pointee(Snapshot::from_journal(Arc::new(
                journal.clone(),
            )))),
            editor: Arc::new(Mutex::new(None)),
            rules_writes: Arc::new(tokio::sync::Mutex::new(())),
            stages: Arc::new(stage::StageArea::default()),
            qb_stages: Arc::new(qb_journal_api::QbStageArea::default()),
            import_writes: Arc::new(tokio::sync::Mutex::new(())),
            price_source: default_price_source(),
        }
    }

    /// Build editing-enabled state bound to the journal file at `path`: open a
    /// [`JournalEditor`] over it and serve the snapshot built from its parsed
    /// journal. The edit endpoints (`POST`/`DELETE /api/transactions`) are then
    /// live and mutate this file through the editor's validation + atomic write.
    ///
    /// # Errors
    /// [`EditError::Io`] if the file cannot be read, or [`EditError::Parse`] if it
    /// does not parse.
    pub fn from_journal_path(path: impl AsRef<Path>) -> Result<Self, EditError> {
        let editor = JournalEditor::open(path.as_ref())?;
        let snapshot = Snapshot::from_journal(Arc::clone(editor.journal()));
        Ok(Self {
            inner: Arc::new(ArcSwap::from_pointee(snapshot)),
            editor: Arc::new(Mutex::new(Some(editor))),
            rules_writes: Arc::new(tokio::sync::Mutex::new(())),
            stages: Arc::new(stage::StageArea::default()),
            qb_stages: Arc::new(qb_journal_api::QbStageArea::default()),
            import_writes: Arc::new(tokio::sync::Mutex::new(())),
            price_source: default_price_source(),
        })
    }

    /// Atomically replace the served journal (and its precomputed payloads).
    /// In-flight requests keep their snapshot; subsequent ones see the new data.
    ///
    /// Takes the shared `Arc` — the shape [`JournalEditor::journal`] hands back —
    /// so republishing after an edit shares the editor's journal rather than
    /// deep-cloning it (PERF-1b).
    pub fn replace_journal(&self, journal: &Arc<Journal>) {
        self.inner
            .store(Arc::new(Snapshot::from_journal(Arc::clone(journal))));
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
        match JournalEditor::open(editor.path().to_path_buf()) {
            Ok(reopened) => {
                self.inner.store(Arc::new(Snapshot::from_journal(Arc::clone(
                    reopened.journal(),
                ))));
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
        let editor = JournalEditor::open(path.as_ref())?;
        self.inner.store(Arc::new(Snapshot::from_journal(Arc::clone(
            editor.journal(),
        ))));
        let mut guard = self.editor.lock().unwrap_or_else(PoisonError::into_inner);
        *guard = Some(editor);
        // The previously-bound editor is discarded wholesale, so any poison flag
        // from a panic during its lifetime no longer describes live state.
        self.editor.clear_poison();
        Ok(())
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
    pub(crate) fn editing_enabled(&self) -> bool {
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
///   cannot fetch the shell that carries the token.
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
/// carries the access token in its body. That combination meant a single
/// `fetch("http://127.0.0.1:<port>/")` from any page on an allowlisted dev
/// origin — another Vite project, a malicious dev dependency — was answered
/// with `Access-Control-Allow-Origin`, so the browser handed that page the
/// token, and with it full read/write on the user's journal. `--allow-origin`
/// is meant to open the API to a dev SPA; it must not also publish the
/// credential.
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
    let spa_token = security.token();
    let router = Router::new()
        .route("/version", get(version))
        .route("/accountnames", get(accountnames))
        .route("/transactions", get(transactions))
        .route("/prices", get(prices))
        .route("/commodities", get(commodities))
        .route("/accounts", get(accounts))
        // Journal-wide diagnostics (unbalanced transactions, failed balance
        // assertions). A NATIVE route, not a wire one: `/transactions` is a
        // byte-parity emulation of hledger-web's endpoint and is a bare JSON
        // array, so it has nowhere to carry a sibling field.
        .route("/api/diagnostics", get(diagnostics))
        // Which journal is open: its derived title and its main file's bare
        // name. A NATIVE route for the same reason as `/api/diagnostics` — the
        // wire routes are byte-parity emulations of hledger-web's and have
        // nowhere to carry this. Registered above the `route_layer` below so it
        // is token-gated with everything else: it is the only route that says
        // anything at all about the journal's own file.
        .route("/api/journal", get(journal_info))
        .route("/api/reports/balancesheet", get(reports_api::balancesheet))
        // The grouped/valued three-box balance sheet. A SIBLING route, not a
        // mode of the one above: that one is a flat hledger-parity shape with a
        // committed golden, and this one answers a different question.
        .route(
            "/api/reports/balancesheet/grouped",
            get(reports_api::balancesheet_grouped),
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
        .route("/api/reports/networth", get(reports_api::networth))
        .route("/api/insights", get(reports_api::insights_report))
        .route("/api/subscriptions", get(reports_api::subscriptions))
        .route("/api/budget", get(reports_api::budget))
        .route("/api/holdings", get(reports_api::holdings))
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
        // Write path (Phase 5.2+): add / delete / replace (PUT) / partial-edit
        // (PATCH) a transaction through the editor.
        .route("/api/transactions", post(edit_api::add_transaction))
        .route(
            "/api/transactions/{index}",
            delete(edit_api::delete_transaction)
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
            // The ONLY route with a raised body limit, and it is raised on this
            // route alone rather than globally: every other endpoint takes a
            // small JSON body, and a global limit would lift the ceiling on all
            // of them for the sake of one. `route_layer` here applies to just
            // this route's method handler.
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
        // the fallback, which is what lets the browser fetch the shell (and the
        // token inside it) before it has any credential to present.
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
    let router = router.fallback(move |uri: Uri| {
        let token = spa_token.clone();
        async move { spa::fallback(uri, token).await }
    });

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

async fn transactions(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let snapshot = state.snapshot();
    serve(&snapshot, &headers, &snapshot.transactions)
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

async fn diagnostics(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let snapshot = state.snapshot();
    serve(&snapshot, &headers, &snapshot.diagnostics)
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
