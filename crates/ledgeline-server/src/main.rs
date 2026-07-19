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

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Parser;
use ledgeline_core::{Journal, parse_journal};
use ledgeline_server::{AppState, router_with_state};
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

/// Resolve the journal path: positional arg → `$LEDGELINE_FIXTURE` → the default
/// dev fixture.
pub(crate) fn resolve_journal(cli: &Cli) -> PathBuf {
    cli.journal
        .clone()
        .or_else(|| std::env::var("LEDGELINE_FIXTURE").ok().map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from(DEFAULT_FIXTURE))
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
/// contents — this also republishes the snapshot. Our OWN saves also fire the
/// watcher; re-opening then just re-reads the identical bytes we wrote, which is
/// idempotent (a small, harmless redundancy). When no editor is bound (read-only
/// state), fall back to a plain reparse + hot-swap.
pub(crate) fn reload_journal(path: &Path, state: &AppState) {
    match state.reopen_editor() {
        Some(Ok(())) => eprintln!("ledgeline: reloaded {} (editor re-synced)", path.display()),
        Some(Err(error)) => eprintln!("ledgeline: reload skipped: {error}"),
        None => match parse_at(path) {
            Ok(journal) => {
                state.replace_journal(&journal);
                eprintln!("ledgeline: reloaded {}", path.display());
            }
            Err(error) => eprintln!("ledgeline: reload skipped: {error}"),
        },
    }
}

/// Watch the journal file for changes and hot-swap on each edit.
///
/// We watch the containing directory (non-recursively) and filter to the target
/// file name, which survives the atomic rename-into-place that most editors use
/// on save (a direct single-file watch would lose the inode). The returned
/// watcher must be kept alive for as long as watching is desired.
pub(crate) fn spawn_watcher(path: &Path, state: AppState) -> Result<RecommendedWatcher, AppError> {
    let target = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let dir = target
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let watched = target.clone();

    let mut watcher =
        notify::recommended_watcher(move |result: notify::Result<notify::Event>| match result {
            Ok(event) if event.paths.iter().any(|p| same_file_name(p, &watched)) => {
                reload_journal(&watched, &state);
            }
            Ok(_) => {}
            Err(error) => eprintln!("ledgeline: watch error: {error}"),
        })
        .map_err(AppError::Watch)?;
    watcher
        .watch(&dir, RecursiveMode::NonRecursive)
        .map_err(AppError::Watch)?;
    Ok(watcher)
}

/// Match a watch-event path against our target by file name. Events only come
/// from the single directory we watch, so the file name uniquely identifies it
/// even when the event path is not canonicalized.
fn same_file_name(candidate: &Path, target: &Path) -> bool {
    candidate.file_name() == target.file_name()
}

/// Headless mode: serve the API + embedded SPA on a fixed port with graceful
/// shutdown, hot-reloading the journal on file change.
fn run_server_blocking(cli: &Cli) -> Result<(), AppError> {
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

        // Live-reload watcher; held for the serve duration (dropping it stops
        // watching). A watcher failure only disables live reload.
        let _watcher = match spawn_watcher(&journal_path, state.clone()) {
            Ok(watcher) => Some(watcher),
            Err(error) => {
                eprintln!("ledgeline: live-reload disabled: {error}");
                None
            }
        };

        axum::serve(listener, router_with_state(state))
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
