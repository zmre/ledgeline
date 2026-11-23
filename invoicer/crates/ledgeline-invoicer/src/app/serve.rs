//! `--server`: the headless server, for e2e and scripting. The same router
//! and posture as the window's, on a fixed port ([`DEFAULT_SERVER_PORT`]),
//! printing the token the way Ledgeline's `--server` does.

use ledgeline_shell::AppDirs;
use ledgeline_shell::security::{self, ProcessToken, SecurityPlan};

use super::books::OpenBooks;
use super::{APP, AppArgs, AppState, DEFAULT_SERVER_PORT, SPA_BASE, router, watch};

/// Mint the token and decide the bind posture (Ledgeline's SEC-9): a
/// loopback bind gets the token and a Host guard pinned to the port; any
/// other bind is refused unless the token was chosen through
/// `$LEDGELINE_INVOICER_TOKEN`, and then the Host guard is off.
///
/// # Errors
/// A refused bind, a malformed token or `--allow-origin`.
pub(crate) fn plan_security(args: &AppArgs) -> Result<(ProcessToken, SecurityPlan), String> {
    let process_token =
        security::token_from_env_or_random(APP.token_env).map_err(|e| e.to_string())?;
    let plan = SecurityPlan::new(
        process_token.token.clone(),
        &args.host,
        args.allow_origin.clone(),
    );
    if !plan.is_loopback() {
        if !process_token.from_env {
            return Err(format!(
                "refusing to bind {}: that is not a loopback address, and this server publishes \
                 your books for reading and writing. Bind 127.0.0.1 instead, or set ${} to a \
                 token you have chosen and pass it on every request.",
                args.host, APP.token_env
            ));
        }
        eprintln!(
            "{}: WARNING: binding {}, which is not loopback; the Host check is off.",
            APP.log_prefix, args.host
        );
    }
    plan.build(args.port.unwrap_or(0))
        .map_err(|e| e.to_string())?;
    if !args.allow_origin.is_empty() {
        eprintln!(
            "{}: allowing cross-origin API access from: {}",
            APP.log_prefix,
            args.allow_origin.join(", ")
        );
    }
    Ok((process_token, plan))
}

/// Serve until Ctrl-C or SIGTERM.
pub(crate) fn run(args: &AppArgs, app_dirs: AppDirs) -> Result<(), String> {
    let (process_token, plan) = plan_security(args)?;
    let journal = args.journal.clone().ok_or_else(|| {
        "no journal: pass one (ledgeline-invoicer --server books.journal) or set \
         $LEDGELINE_FIXTURE"
            .to_string()
    })?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("building the async runtime: {e}"))?;
    let host = args.host.clone();
    let port = args.port.unwrap_or(DEFAULT_SERVER_PORT);
    runtime.block_on(async move {
        let books = OpenBooks::open(&journal, false)?;
        books.reconcile_all();
        let books = std::sync::Arc::new(books);
        // Never recorded in recents: `--server` runs over fixtures (e2e).
        let state = AppState::new(Some(std::sync::Arc::clone(&books)), app_dirs);
        let addr = format!("{host}:{port}");
        let listener = tokio::net::TcpListener::bind(&addr)
            .await
            .map_err(|e| format!("binding {addr}: {e}"))?;
        let bound = listener.local_addr().map(|a| a.port()).unwrap_or(port);
        let security = plan.build(bound).map_err(|e| e.to_string())?;
        let token = process_token.token.as_str();
        println!(
            "{prefix} listening on http://{host}:{bound}/ (journal: {journal})\n\
             {prefix}: open http://{host}:{bound}{SPA_BASE}/#token={token} in a browser\n\
             {prefix}: access token: {token}\n\
             {prefix}: send it as `Authorization: Bearer <token>` on every /api request{origin}",
            prefix = APP.log_prefix,
            journal = books.journal().display(),
            origin = if process_token.from_env {
                format!(" (from ${})", APP.token_env)
            } else {
                format!(" (set ${} to choose it yourself)", APP.token_env)
            },
        );
        let _watcher = match watch::spawn(std::sync::Arc::clone(&books)) {
            Ok(watcher) => Some(watcher),
            Err(error) => {
                eprintln!("{}: live reconcile disabled: {error}", APP.log_prefix);
                None
            }
        };
        let app = router(state, security, process_token.token.clone(), bound);
        axum::serve(listener, app)
            .with_graceful_shutdown(shutdown_signal())
            .await
            .map_err(|e| format!("serving HTTP: {e}"))
    })
}

/// Resolve on Ctrl-C or (Unix) SIGTERM.
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
    println!("{}: received shutdown signal, draining", APP.log_prefix);
}
