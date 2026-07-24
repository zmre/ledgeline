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

/// Watch every file the journal depends on — the main file plus each `include`d
/// file — and hot-swap on any edit to one of them.
///
/// We watch each containing directory (non-recursively) rather than the files
/// directly, which survives the atomic rename-into-place most editors use on save
/// (a direct single-file watch would lose the inode). Events are filtered against
/// the journal's *current* source-file set, re-read on each event so that adding
/// or removing an `include` is honored without recreating the watcher. The
/// returned watcher must be kept alive for as long as watching is desired.
pub(crate) fn spawn_watcher(path: &Path, state: AppState) -> Result<RecommendedWatcher, AppError> {
    let target = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let callback_state = state.clone();
    let callback_target = target.clone();

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
                    reload_journal(&callback_target, &callback_state);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A unique temp directory for one test, created fresh.
    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ledgeline_watch_{tag}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn event_matches_source_by_dir_and_name() {
        let dir = temp_dir("match");
        let main = dir.join("main.journal");
        std::fs::write(&main, "").unwrap();
        let sources = vec![main.canonicalize().unwrap()];

        // The watched file matches whether the event path is canonical or not.
        assert!(event_matches_source(&main, &sources));
        assert!(event_matches_source(
            &main.canonicalize().unwrap(),
            &sources
        ));

        // A different name in the same directory does NOT match.
        let sibling = dir.join("other.journal");
        std::fs::write(&sibling, "").unwrap();
        assert!(!event_matches_source(&sibling, &sources));

        // A same-named file in a DIFFERENT directory does NOT match (the reason we
        // compare the directory, not just the file name).
        let other_dir = temp_dir("match_other");
        let twin = other_dir.join("main.journal");
        std::fs::write(&twin, "").unwrap();
        assert!(!event_matches_source(&twin, &sources));
    }

    #[test]
    fn event_matches_source_falls_back_to_name_when_dir_unresolvable() {
        // If the candidate's directory can't be resolved (e.g. a transient path
        // under a nonexistent parent), we keep the event by name rather than drop a
        // real change.
        let dir = temp_dir("fallback");
        let main = dir.join("main.journal");
        std::fs::write(&main, "").unwrap();
        let sources = vec![main.canonicalize().unwrap()];

        let ghost = dir.join("does-not-exist").join("main.journal");
        assert!(event_matches_source(&ghost, &sources));
    }

    #[test]
    fn current_sources_lists_includes_and_appends_missing_main() {
        let dir = temp_dir("sources");
        let inc = dir.join("inc.journal");
        let main = dir.join("main.journal");
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
        let ghost = dir.join("ghost.journal");
        assert!(current_sources(&state, &ghost).contains(&ghost));
    }
}
