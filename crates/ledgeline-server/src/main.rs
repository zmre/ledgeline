//! `ledgeline-server` — the local HTTP API server binary.
//!
//! Parses a journal once and serves the wire-compatible read endpoints
//! (`/version`, `/accountnames`, `/transactions`, `/prices`, `/commodities`,
//! `/accounts`) over axum, with permissive CORS and graceful shutdown. The app
//! itself lives in the crate library ([`ledgeline_server::app`]).

use std::path::PathBuf;
use std::process::ExitCode;

use ledgeline_core::parse_journal;
use ledgeline_server::app;

/// Parsed command-line configuration.
struct Args {
    journal: PathBuf,
    host: String,
    port: u16,
}

const DEFAULT_HOST: &str = "127.0.0.1";
const DEFAULT_PORT: u16 = 5000;
const DEFAULT_FIXTURE: &str = "fixtures/sample.journal";

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("ledgeline-server: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args = parse_args()?;

    let text = std::fs::read_to_string(&args.journal)
        .map_err(|e| format!("reading {}: {e}", args.journal.display()))?;
    // Record the absolute path in source positions (matches the snapshots and
    // the SPA's expectations); fall back to the given path if it can't resolve.
    let source_name = args
        .journal
        .canonicalize()
        .unwrap_or_else(|_| args.journal.clone())
        .to_string_lossy()
        .into_owned();
    let journal = parse_journal(&text, &source_name)?;

    let addr = format!("{}:{}", args.host, args.port);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .map_err(|e| format!("binding {addr}: {e}"))?;

    println!(
        "ledgeline-server listening on http://{addr} (journal: {})",
        args.journal.display()
    );
    axum::serve(listener, app(&journal))
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

/// Parse `--host`/`--port` flags and an optional positional journal path.
///
/// Journal precedence: positional arg → `$LEDGELINE_FIXTURE` → the default
/// `fixtures/sample.journal`.
fn parse_args() -> Result<Args, String> {
    let mut journal: Option<PathBuf> = None;
    let mut host = DEFAULT_HOST.to_string();
    let mut port = DEFAULT_PORT;

    let mut iter = std::env::args().skip(1);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--host" => {
                host = iter.next().ok_or("--host requires a value")?;
            }
            "--port" => {
                port = iter
                    .next()
                    .ok_or("--port requires a value")?
                    .parse()
                    .map_err(|_| "--port must be a number in 0..=65535".to_string())?;
            }
            "--help" | "-h" => {
                println!(
                    "usage: ledgeline-server [JOURNAL] [--host HOST] [--port PORT]\n  \
                     JOURNAL  path to a journal (default: ${{LEDGELINE_FIXTURE}} or {DEFAULT_FIXTURE})\n  \
                     --host   bind address (default: {DEFAULT_HOST})\n  \
                     --port   bind port (default: {DEFAULT_PORT})"
                );
                std::process::exit(0);
            }
            flag if flag.starts_with('-') => return Err(format!("unknown flag: {flag}")),
            positional => journal = Some(PathBuf::from(positional)),
        }
    }

    let journal = journal
        .or_else(|| std::env::var("LEDGELINE_FIXTURE").ok().map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from(DEFAULT_FIXTURE));

    Ok(Args {
        journal,
        host,
        port,
    })
}

/// Resolve when the process receives Ctrl-C or (on Unix) SIGTERM, so
/// `axum::serve` can drain in-flight requests before exiting.
async fn shutdown_signal() {
    let ctrl_c = async {
        // If the handler can't be installed we simply never trigger via Ctrl-C
        // (SIGTERM still works); we do not want to shut down prematurely.
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
    println!("ledgeline-server: received shutdown signal, draining");
}
