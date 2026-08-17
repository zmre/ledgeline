//! `ledgeline` — a single binary that is either a native desktop app or a
//! headless HTTP server for one hledger journal.
//!
//! * `ledgeline [JOURNAL]` (default) opens a native window showing the built SPA
//!   with the API server running IN-PROCESS on an ephemeral same-origin port —
//!   no separate server, no `hledger-web`. (Requires the default `gui` feature.)
//! * `ledgeline --server [JOURNAL]` runs headless: just the axum API + embedded
//!   SPA on a fixed port (the historical behavior).
//!
//! Both modes parse the journal into an [`AppState`] whose journal is
//! hot-swappable, and both watch the journal file so an external edit reparses
//! and republishes without a restart (the SPA polls and refetches).

#[cfg(feature = "gui")]
mod gui;
mod recents;

use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::mpsc;
use std::time::Duration;

use clap::Parser;
use ledgeline_core::{Journal, parse_journal};
use ledgeline_server::{
    AppState, ProcessToken, Security, SecurityError, TOKEN_ENV, router_with_security,
    token_from_env_or_random,
};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};

const DEFAULT_HOST: &str = "127.0.0.1";
/// Fixed default port for headless `--server` mode (GUI mode uses an ephemeral
/// port instead — see [`gui`]).
const DEFAULT_SERVER_PORT: u16 = 5000;
const DEFAULT_FIXTURE: &str = "fixtures/sample.journal";

/// A single binary: native GUI (default) or headless API server (`--server`)
/// for one hledger journal.
#[derive(Parser, Debug)]
#[command(
    name = "ledgeline",
    version,
    about = "Ledgeline — a single-binary hledger GUI (default) or headless API server (--server).",
    long_about = None
)]
pub(crate) struct Cli {
    /// Journal to open (default: $LEDGELINE_FIXTURE, else fixtures/sample.journal).
    pub(crate) journal: Option<PathBuf>,

    /// Run headless: HTTP API + embedded SPA only, no desktop window.
    #[arg(short = 's', long)]
    pub(crate) server: bool,

    /// Address to bind.
    #[arg(long, default_value = DEFAULT_HOST)]
    pub(crate) host: String,

    /// Port to bind (default: 5000 for --server; an ephemeral port for the GUI).
    #[arg(long)]
    pub(crate) port: Option<u16>,

    /// DEV ONLY: let a browser SPA on this EXACT origin call the API
    /// cross-origin, e.g. `--allow-origin http://localhost:5173`. Repeatable.
    /// Never a wildcard. Without it the server is same-origin only, which is the
    /// posture the packaged app uses.
    #[arg(long = "allow-origin", value_name = "ORIGIN")]
    pub(crate) allow_origin: Vec<String>,
}

/// Fatal startup/runtime errors surfaced to the user via `main`.
#[derive(Debug, thiserror::Error)]
pub(crate) enum AppError {
    #[error("reading {path}: {source}")]
    Read {
        path: String,
        source: std::io::Error,
    },
    #[error("parsing {path}: {source}")]
    Parse {
        path: String,
        source: ledgeline_core::ParseError,
    },
    #[error("opening {path} for editing: {source}")]
    OpenEditor {
        path: String,
        source: ledgeline_core::EditError,
    },
    #[error("building the async runtime: {0}")]
    Runtime(std::io::Error),
    #[error("binding {addr}: {source}")]
    Bind {
        addr: String,
        source: std::io::Error,
    },
    #[error("serving HTTP: {0}")]
    Serve(std::io::Error),
    #[error("watching the journal: {0}")]
    Watch(notify::Error),
    #[error("{0}")]
    Security(#[from] SecurityError),
    #[error(
        "refusing to bind {host}: that is not a loopback address, and this server publishes your \
         whole journal for reading AND writing. Bind 127.0.0.1 instead, or — if you really mean to \
         expose it — set ${TOKEN_ENV} to a token you have chosen and pass it on every request."
    )]
    NonLoopbackBind { host: String },
    #[cfg(feature = "gui")]
    #[error("the in-process server did not report a bound port")]
    ServerStart,
    #[cfg(feature = "gui")]
    #[error("desktop GUI: {0}")]
    Gui(String),
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("ledgeline: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<(), AppError> {
    #[cfg(feature = "gui")]
    if !cli.server {
        return gui::run(&cli);
    }
    #[cfg(not(feature = "gui"))]
    if !cli.server {
        eprintln!(
            "ledgeline: built without the `gui` feature — running headless. Pass --server to silence this."
        );
    }
    run_server_blocking(&cli)
}

/// Resolve the journal path: positional arg → `$LEDGELINE_FIXTURE` → the
/// most-recently-opened journal that still exists → the default dev fixture. So
/// `ledgeline` with no args re-opens the last journal you used.
pub(crate) fn resolve_journal(cli: &Cli) -> PathBuf {
    cli.journal
        .clone()
        .or_else(|| std::env::var("LEDGELINE_FIXTURE").ok().map(PathBuf::from))
        .or_else(recents::most_recent)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_FIXTURE))
}

/// Is `host` a loopback address (or the `localhost` name)? Anything else — a LAN
/// address, or the `0.0.0.0` / `::` wildcards — publishes the journal beyond this
/// machine.
fn is_loopback_host(host: &str) -> bool {
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    bare.eq_ignore_ascii_case("localhost")
        || bare
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

/// The access-control decisions taken at startup, before the socket is bound.
///
/// Split from [`Security`] itself because the `Host` guard has to name the port
/// we ACTUALLY bound, and `--port 0` (the GUI's default) only reveals that at
/// bind time — so the plan is made early and [`SecurityPlan::build`] finishes it.
pub(crate) struct SecurityPlan {
    token: ledgeline_server::AccessToken,
    /// Whether `--host` resolved to loopback; drives the `Host` guard.
    loopback: bool,
    /// Exact origins from `--allow-origin`, already validated.
    origins: Vec<String>,
}

impl SecurityPlan {
    /// Finish the plan now that `port` is known.
    fn build(&self, port: u16) -> Result<Security, AppError> {
        let base = if self.loopback {
            Security::local(self.token.clone(), port)
        } else {
            Security::any_host(self.token.clone())
        };
        Ok(base.allow_origins(&self.origins)?)
    }
}

/// SEC-9: mint this process's access token and decide the bind posture.
///
/// A loopback bind gets the full treatment — token plus a `Host` guard pinned to
/// the bound port. A non-loopback bind is REFUSED unless the operator set
/// `$LEDGELINE_TOKEN` themselves, which is the only way to say "yes, I mean to
/// expose this, and I know the credential"; even then it earns a loud warning
/// and the `Host` guard comes off, because the legitimate `Host` values are then
/// unknowable.
pub(crate) fn plan_security(cli: &Cli) -> Result<(ProcessToken, SecurityPlan), AppError> {
    let process_token = token_from_env_or_random()?;
    let loopback = is_loopback_host(&cli.host);
    if !loopback && !process_token.from_env {
        return Err(AppError::NonLoopbackBind {
            host: cli.host.clone(),
        });
    }
    if !loopback {
        eprintln!(
            "ledgeline: WARNING — binding {host}, which is not loopback. Anything that can reach \
             this port can read and rewrite your journal once it learns the token, and the \
             DNS-rebinding Host check is off because the expected Host is unknown.",
            host = cli.host
        );
    }
    let plan = SecurityPlan {
        token: process_token.token.clone(),
        loopback,
        origins: cli.allow_origin.clone(),
    };
    // Surface a bad --allow-origin at startup rather than at the first request.
    plan.build(cli.port.unwrap_or(0))?;
    if !cli.allow_origin.is_empty() {
        eprintln!(
            "ledgeline: allowing cross-origin API access from: {}",
            cli.allow_origin.join(", ")
        );
    }
    Ok((process_token, plan))
}

/// Read + parse a journal file, recording its absolute path as the source name
/// (matches the wire snapshots and the SPA's expectations).
pub(crate) fn parse_at(path: &Path) -> Result<Journal, AppError> {
    let text = std::fs::read_to_string(path).map_err(|source| AppError::Read {
        path: path.display().to_string(),
        source,
    })?;
    let source_name = path
        .canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .into_owned();
    parse_journal(&text, &source_name).map_err(|source| AppError::Parse {
        path: path.display().to_string(),
        source,
    })
}

/// React to an on-disk change of `path` (best-effort: an error is logged and the
/// previous data is kept, so a mid-edit save never crashes).
///
/// When the state has an editor bound (the write path is enabled), re-open it so
/// its rope, parsed journal, and external-change fingerprint track the new file
/// contents — this also republishes the snapshot. When no editor is bound
/// (read-only state), fall back to a plain reparse + hot-swap.
///
/// This is the EXPENSIVE half of live reload — a full reparse plus a full
/// snapshot rebuild, ~1.7 s at 200k transactions — so [`watch_loop`] is careful
/// to call it once per settled change and never for a file that did not actually
/// change (PERF-4).
pub(crate) fn reload_journal(path: &Path, state: &AppState) {
    match state.reopen_editor() {
        Some(Ok(())) => eprintln!("ledgeline: reloaded {} (editor re-synced)", path.display()),
        Some(Err(error)) => eprintln!("ledgeline: reload skipped: {error}"),
        None => match parse_at(path) {
            Ok(journal) => {
                state.replace_journal(&std::sync::Arc::new(journal));
                eprintln!("ledgeline: reloaded {}", path.display());
            }
            Err(error) => eprintln!("ledgeline: reload skipped: {error}"),
        },
    }
}

/// How long the watcher waits for the filesystem to go quiet before reloading.
///
/// One editor save is several filesystem events — write a temp file, rename it
/// into place, restore the mode — and a `notify` backend may report each of them
/// separately. 250 ms is long enough to swallow a whole save (and the burst that
/// our own `POST /api/transactions` produces) while staying well under the
/// threshold where a reload feels delayed.
const WATCH_DEBOUNCE: Duration = Duration::from_millis(250);

/// A file's identity for change detection: its length and a hash of its RAW
/// BYTES. Not its mtime, and above all not any rendered text.
///
/// Two rules meet here. Ledgeline formats amounts to two display decimals, so a
/// check built on displayed text silently ignores every edit below that — a
/// sub-cent fee, a share count's third decimal — which is why this reads the
/// file, not the rendering. And `JournalEditor`'s own fingerprint deliberately
/// dropped mtime (DL-3): an unchanged timestamp proves nothing, because
/// mtime-preserving copy tools exist, and a timestamp-only touch is not a change
/// at all. Keying off the bytes gets both directions right — no missed edit, and
/// no 1.1-second reparse for a save that changed nothing.
type FileStamp = (PathBuf, u64, u64);

/// FNV-1a 64-bit, the same non-cryptographic hash `JournalEditor` uses for its
/// external-change fingerprint. Hashing 26 MB costs a few tens of milliseconds
/// against the ~1.1 s reload it can avoid.
fn content_hash(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// Stamp every file the journal is currently built from, in a stable order.
///
/// A missing or unreadable file stamps as empty rather than aborting, so a save
/// caught mid-rename simply looks different from both its neighbours and
/// resolves on the next event.
fn stamp_sources(sources: &[PathBuf]) -> Vec<FileStamp> {
    let mut stamps: Vec<FileStamp> = sources
        .iter()
        .map(|path| {
            let bytes = std::fs::read(path).unwrap_or_default();
            (path.clone(), bytes.len() as u64, content_hash(&bytes))
        })
        .collect();
    stamps.sort();
    stamps
}

/// Coalesce watch events and reload at most once per settled change (PERF-4).
///
/// Runs on its own thread for the life of the watcher. It blocks for the next
/// event, then keeps draining until [`WATCH_DEBOUNCE`] passes with nothing new —
/// so a burst of events from one save collapses into one wake-up. It then
/// compares the source files' stamps against the last reload's and returns
/// without doing anything when the bytes are unchanged, which is what makes a
/// no-op save, a `touch`, and a rewrite with identical content all free.
///
/// Before the fix this was one full reparse + snapshot rebuild PER EVENT.
fn watch_loop(events: &mpsc::Receiver<()>, target: &Path, state: &AppState) {
    let mut last = stamp_sources(&current_sources(state, target));
    while events.recv().is_ok() {
        // Drain the rest of the burst: keep extending the window until the
        // filesystem has been quiet for a full debounce interval.
        while events.recv_timeout(WATCH_DEBOUNCE).is_ok() {}

        let sources = current_sources(state, target);
        let current = stamp_sources(&sources);
        if current == last {
            continue;
        }
        reload_journal(target, state);
        // Re-stamp AFTER the reload: reloading can change the source set (an
        // `include` added or removed), and a save that lands during the reload
        // must still be seen as a change by the next round.
        last = stamp_sources(&current_sources(state, target));
    }
}

/// Watch every file the journal depends on — the main file plus each `include`d
/// file — and hot-swap on any edit to one of them.
///
/// We watch each containing directory (non-recursively) rather than the files
/// directly, which survives the atomic rename-into-place most editors use on save
/// (a direct single-file watch would lose the inode). Events are filtered against
/// the journal's *current* source-file set, re-read on each event so that adding
/// or removing an `include` is honored without recreating the watcher. The
/// returned watcher must be kept alive for as long as watching is desired.
///
/// The notify callback does no work beyond filtering: it hands matching events
/// to a debouncing thread ([`watch_loop`]) that coalesces a save's burst into one
/// reload. Reloading straight from the callback — as this did before PERF-4 —
/// meant a full reparse and snapshot rebuild per raw filesystem event, and also
/// blocked the notify thread for seconds at a time on a large journal.
pub(crate) fn spawn_watcher(path: &Path, state: AppState) -> Result<RecommendedWatcher, AppError> {
    let target = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let callback_state = state.clone();
    let callback_target = target.clone();
    let (sender, receiver) = mpsc::channel::<()>();

    let mut watcher =
        notify::recommended_watcher(move |result: notify::Result<notify::Event>| match result {
            Ok(event) => {
                // Re-read the source set every event so a newly-`include`d file is
                // honored (and a removed one dropped) without recreating the
                // watcher; the edit that changes the set lives in an already-watched
                // file, so its own save is what refreshes this.
                let sources = current_sources(&callback_state, &callback_target);
                if event
                    .paths
                    .iter()
                    .any(|p| event_matches_source(p, &sources))
                {
                    // A closed receiver means the debouncer is gone (shutdown);
                    // dropping the event is then the correct response.
                    let _ = sender.send(());
                }
            }
            Err(error) => eprintln!("ledgeline: watch error: {error}"),
        })
        .map_err(AppError::Watch)?;

    // Watch the main file's directory with `?` — its failure disables live reload,
    // preserving the prior single-file behavior. Each additional include directory
    // is best-effort so one unwatchable path doesn't sink reload for the rest.
    let main_dir = watch_dir(&target);
    watcher
        .watch(&main_dir, RecursiveMode::NonRecursive)
        .map_err(AppError::Watch)?;
    let mut watched_dirs: Vec<PathBuf> = vec![main_dir];
    for dir in current_sources(&state, &target)
        .iter()
        .map(|s| watch_dir(s))
    {
        if watched_dirs.contains(&dir) {
            continue;
        }
        match watcher.watch(&dir, RecursiveMode::NonRecursive) {
            Ok(()) => watched_dirs.push(dir),
            Err(error) => eprintln!("ledgeline: not watching {}: {error}", dir.display()),
        }
    }

    // Detached: it ends when the returned watcher is dropped, which drops the
    // callback holding the sender and closes the channel.
    std::thread::Builder::new()
        .name("ledgeline-watch".to_string())
        .spawn(move || watch_loop(&receiver, &target, &state))
        .map_err(|error| AppError::Watch(notify::Error::io(error)))?;
    Ok(watcher)
}

/// The directory to watch for a source file: its parent, or `.` when it has none.
fn watch_dir(source: &Path) -> PathBuf {
    source
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// The journal's current source files (main + `include`s) as resolved paths,
/// always including `main` even if the published journal has not recorded it yet
/// (e.g. read-only state with an empty snapshot).
fn current_sources(state: &AppState, main: &Path) -> Vec<PathBuf> {
    let mut sources = state.source_files();
    if !sources.iter().any(|s| s == main) {
        sources.push(main.to_path_buf());
    }
    sources
}

/// Match a watch-event path against one of the journal's source files by file
/// name and parent directory. Matching the directory too (not just the name, as a
/// single-directory watch could) keeps a same-named file in another watched
/// include directory from triggering a spurious reload. The parent still exists
/// across an editor's atomic rename-into-place even when the file itself
/// momentarily does not, so this stays robust on save. Source paths are already
/// resolved, so only the candidate's directory needs canonicalizing.
fn event_matches_source(candidate: &Path, sources: &[PathBuf]) -> bool {
    let name = candidate.file_name();
    let candidate_dir = candidate.parent().and_then(|d| d.canonicalize().ok());
    sources.iter().any(|source| {
        source.file_name() == name
            && match (candidate_dir.as_deref(), source.parent()) {
                (Some(cand_dir), Some(src_dir)) => cand_dir == src_dir,
                // If the candidate's directory can't be resolved right now, fall
                // back to the file-name match rather than dropping a real event.
                _ => true,
            }
    })
}

/// Headless mode: serve the API + embedded SPA on a fixed port with graceful
/// shutdown, hot-reloading the journal on file change.
fn run_server_blocking(cli: &Cli) -> Result<(), AppError> {
    // Decide the security posture BEFORE touching the journal, so a refused
    // non-loopback bind (SEC-9) fails fast and never opens the file.
    let (process_token, security_plan) = plan_security(cli)?;
    let journal_path = resolve_journal(cli);
    // Bind an editor to the file so the write endpoints (`POST`/`DELETE
    // /api/transactions`) are live. Canonicalize first so the editor's save target
    // and recorded source name match the watcher's canonical path and the
    // historical snapshot source name.
    let editor_path = journal_path
        .canonicalize()
        .unwrap_or_else(|_| journal_path.clone());
    let state =
        AppState::from_journal_path(&editor_path).map_err(|source| AppError::OpenEditor {
            path: journal_path.display().to_string(),
            source,
        })?;
    // Remember this journal as the most-recently-opened (canonical path).
    recents::record(&editor_path);
    let host = cli.host.clone();
    let port = cli.port.unwrap_or(DEFAULT_SERVER_PORT);

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(AppError::Runtime)?;

    runtime.block_on(async move {
        let addr = format!("{host}:{port}");
        let listener = tokio::net::TcpListener::bind(&addr)
            .await
            .map_err(|source| AppError::Bind {
                addr: addr.clone(),
                source,
            })?;
        let bound = listener.local_addr().map(|a| a.port()).unwrap_or(port);
        println!(
            "ledgeline listening on http://{host}:{bound}/ (journal: {})",
            journal_path.display()
        );
        // Headless mode exists to be driven by something else — a browser at a
        // different origin, the e2e harness, a script — so the token has to be
        // discoverable. The GUI never prints it: its WebView reads it from the
        // page. Anyone who can see this terminal can read the journal anyway.
        println!(
            "ledgeline: access token: {}\nledgeline: send it as `Authorization: Bearer <token>` on \
             every /api and wire request{}",
            process_token.token.as_str(),
            if process_token.from_env {
                format!(" (from ${TOKEN_ENV})")
            } else {
                format!(" (set ${TOKEN_ENV} to choose it yourself)")
            }
        );
        let security = security_plan.build(bound)?;

        // Live-reload watcher; held for the serve duration (dropping it stops
        // watching). A watcher failure only disables live reload.
        let _watcher = match spawn_watcher(&journal_path, state.clone()) {
            Ok(watcher) => Some(watcher),
            Err(error) => {
                eprintln!("ledgeline: live-reload disabled: {error}");
                None
            }
        };

        axum::serve(listener, router_with_security(state, security))
            .with_graceful_shutdown(shutdown_signal())
            .await
            .map_err(AppError::Serve)
    })
}

/// Resolve when the process receives Ctrl-C or (on Unix) SIGTERM, so
/// `axum::serve` can drain in-flight requests before exiting.
async fn shutdown_signal() {
    let ctrl_c = async {
        if tokio::signal::ctrl_c().await.is_err() {
            std::future::pending::<()>().await;
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut stream) => {
                stream.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
    println!("ledgeline: received shutdown signal, draining");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A unique temp directory for one test, created fresh.
    /// A scratch directory that cleans itself up and cannot collide.
    ///
    /// The obvious spelling — a fixed `$TMPDIR/ledgeline_watch_{tag}` that is
    /// `remove_dir_all`ed on entry — is wrong in a way that only shows up under
    /// load: two test binaries running at once (a second `cargo test` in
    /// another shell, or a CI matrix sharing a runner) delete each other's
    /// fixture mid-test. The loser then reads a file that is suddenly absent,
    /// reports size 0 and the FNV hash of empty input, and looks like a bug in
    /// the watcher's change detection rather than two tests colliding over a
    /// directory name.
    ///
    /// `TempDir` gives a unique path per call AND removes it on drop, so the
    /// caller must bind it for the length of the test — which is why every
    /// caller says `let dir = ...` and then `dir.path()`.
    fn temp_dir(tag: &str) -> tempfile::TempDir {
        tempfile::Builder::new()
            .prefix(&format!("ledgeline_watch_{tag}_"))
            .tempdir()
            .expect("a scratch directory")
    }

    /// PERF-4's whole mechanism: a save is detected from the file's RAW BYTES,
    /// and an event that changes none of them is dropped rather than paid for
    /// with a full reparse + snapshot rebuild.
    ///
    /// Two standing constraints are checked here. A change check must never key
    /// off display-rounded text — rendering rounds to two decimals, so a
    /// sub-cent edit would look identical and be silently discarded. And per
    /// DL-3 it must not key off mtime either, in EITHER direction: an
    /// mtime-preserving rewrite is still a change, and a timestamp-only touch is
    /// still not one.
    #[test]
    fn source_stamps_track_bytes_not_timestamps() {
        let dir = temp_dir("stamp");
        let main = dir.path().join("main.journal");
        std::fs::write(&main, "2024-01-01 x\n    a  $1.00\n    b\n").unwrap();
        let sources = vec![main.canonicalize().unwrap()];

        let first = stamp_sources(&sources);
        // Re-stamping an untouched file is stable — this is what makes a spurious
        // event free.
        assert_eq!(
            first,
            stamp_sources(&sources),
            "an untouched file must stamp the same"
        );

        // A length-preserving edit BELOW the two-decimal display rounding: the
        // exact case a text-derived fingerprint would miss.
        std::fs::write(&main, "2024-01-01 x\n    a  $1.001\n    b\n").unwrap();
        let edited = stamp_sources(&sources);
        assert_ne!(first, edited, "a sub-cent edit must be visible");

        // Rewriting the identical bytes is NOT a change, however the mtime moved.
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&main, "2024-01-01 x\n    a  $1.001\n    b\n").unwrap();
        assert_eq!(
            edited,
            stamp_sources(&sources),
            "a no-op resave must not cost a reload"
        );

        // A file that vanishes mid-rename stamps as empty rather than panicking.
        std::fs::remove_file(&main).unwrap();
        let missing = stamp_sources(&sources);
        assert_ne!(edited, missing);
        assert_eq!(missing.len(), 1, "a missing source still occupies its slot");
    }

    /// Source order must not decide whether a change is seen: the journal's
    /// `source_files` order can shift when an `include` moves.
    #[test]
    fn source_stamps_are_order_independent() {
        let dir = temp_dir("stamp_order");
        let one = dir.path().join("one.journal");
        let two = dir.path().join("two.journal");
        std::fs::write(&one, "account a\n").unwrap();
        std::fs::write(&two, "account b\n").unwrap();
        let forward = vec![one.canonicalize().unwrap(), two.canonicalize().unwrap()];
        let backward = vec![two.canonicalize().unwrap(), one.canonicalize().unwrap()];
        assert_eq!(stamp_sources(&forward), stamp_sources(&backward));
    }

    #[test]
    fn event_matches_source_by_dir_and_name() {
        let dir = temp_dir("match");
        let main = dir.path().join("main.journal");
        std::fs::write(&main, "").unwrap();
        let sources = vec![main.canonicalize().unwrap()];

        // The watched file matches whether the event path is canonical or not.
        assert!(event_matches_source(&main, &sources));
        assert!(event_matches_source(
            &main.canonicalize().unwrap(),
            &sources
        ));

        // A different name in the same directory does NOT match.
        let sibling = dir.path().join("other.journal");
        std::fs::write(&sibling, "").unwrap();
        assert!(!event_matches_source(&sibling, &sources));

        // A same-named file in a DIFFERENT directory does NOT match (the reason we
        // compare the directory, not just the file name).
        let other_dir = temp_dir("match_other");
        let twin = other_dir.path().join("main.journal");
        std::fs::write(&twin, "").unwrap();
        assert!(!event_matches_source(&twin, &sources));
    }

    #[test]
    fn event_matches_source_falls_back_to_name_when_dir_unresolvable() {
        // If the candidate's directory can't be resolved (e.g. a transient path
        // under a nonexistent parent), we keep the event by name rather than drop a
        // real change.
        let dir = temp_dir("fallback");
        let main = dir.path().join("main.journal");
        std::fs::write(&main, "").unwrap();
        let sources = vec![main.canonicalize().unwrap()];

        let ghost = dir.path().join("does-not-exist").join("main.journal");
        assert!(event_matches_source(&ghost, &sources));
    }

    #[test]
    fn current_sources_lists_includes_and_appends_missing_main() {
        let dir = temp_dir("sources");
        let inc = dir.path().join("inc.journal");
        let main = dir.path().join("main.journal");
        std::fs::write(&inc, "account assets:bank\n").unwrap();
        std::fs::write(
            &main,
            "include inc.journal\n2024-01-01 x\n    expenses:a  $1\n    assets:bank\n",
        )
        .unwrap();
        let journal = parse_at(&main).unwrap();
        let state = AppState::from_journal(&journal);
        let canonical_main = main.canonicalize().unwrap();

        // Both the main file and the directive-only include are watched.
        let sources = current_sources(&state, &canonical_main);
        assert!(sources.contains(&canonical_main));
        assert!(sources.contains(&inc.canonicalize().unwrap()));

        // Defensive fallback: a main path the published journal doesn't list is
        // still appended so its directory is always watched.
        let ghost = dir.path().join("ghost.journal");
        assert!(current_sources(&state, &ghost).contains(&ghost));
    }
}
