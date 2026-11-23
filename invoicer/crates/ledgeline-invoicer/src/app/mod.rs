//! The app (the `server` feature): one local HTTP server, either headless
//! (`--server`) or in-process behind the desktop window (`gui`), booted
//! through `ledgeline-shell` exactly as Ledgeline boots (invoicer PLAN 3a,
//! ARCHITECTURE §6.1).
//!
//! The one server's routes:
//!
//! | Path      | Served by                                                       |
//! |-----------|-----------------------------------------------------------------|
//! | `/api/*`  | The invoicer API ([`api`]): bearer token required.              |
//! | `/app/*`  | The invoicer SPA (`invoicer/web`, SvelteKit base `/app`).       |
//! | the rest  | MBR's router, unmodified ([`mbr`]): behind a session cookie.    |
//!
//! Every response passes the shell's Host guard and security headers. The
//! open books ([`books::OpenBooks`]) are swapped whole on File → Open, MBR
//! server included; the watcher ([`watch`]) feeds the reconciler.

pub(crate) mod api;
pub(crate) mod books;
#[cfg(feature = "gui")]
mod gui;
pub(crate) mod mbr;
pub(crate) mod prefs;
mod serve;
pub(crate) mod watch;

use std::path::PathBuf;
use std::sync::Arc;

use arc_swap::ArcSwapOption;
use axum::http::{StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Extension, Router};
use ledgeline_core::ui_prefs::UiPrefsSession;
use ledgeline_shell::security::{self, AccessToken, Security};
use ledgeline_shell::{AppDirs, AppIdentity, spa};
use rust_embed::RustEmbed;

use books::OpenBooks;
use mbr::MbrGate;

/// Who the invoicer is, to the shared shell.
///
/// The two `window` globals are Ledgeline's on purpose: they are the
/// contract of the shared API client (`packages/ui/src/api/client.ts`), which
/// reads `__LEDGELINE_TOKEN__` and the SPA's embedded marker. The token
/// variable is the invoicer's own, so a shell that exports Ledgeline's never
/// pins this app's token by accident.
pub(crate) static APP: AppIdentity = AppIdentity {
    name: "Ledgeline Invoicer",
    bundle_id: "com.zmre.ledgeline.invoicer",
    log_prefix: "ledgeline-invoicer",
    token_env: "LEDGELINE_INVOICER_TOKEN",
    token_global: "__LEDGELINE_TOKEN__",
    embedded_global: "__LEDGELINE_EMBEDDED__",
};

/// `--server`'s default port. Ledgeline's e2e uses 5099 and 4173, so the
/// invoicer's (5199, and 4273 for the SPA's vite server) let both suites run
/// side by side.
pub(crate) const DEFAULT_SERVER_PORT: u16 = 5199;

/// The default bind address.
pub(crate) const DEFAULT_HOST: &str = "127.0.0.1";

/// Where the SPA is mounted (`paths.base` in `invoicer/web/vite.config.ts`).
pub(crate) const SPA_BASE: &str = "/app";

/// The invoicer SPA, mirrored into `spa/` by build.rs.
#[derive(RustEmbed)]
#[folder = "$CARGO_MANIFEST_DIR/spa"]
struct SpaAssets;

/// What the app was asked to do: the server's half of the command line.
#[derive(Debug, Clone)]
pub(crate) struct AppArgs {
    /// The journal to open, already resolved (JOURNAL, `-f`, the fixture
    /// variable, then recents). `None` opens no books (the GUI only).
    pub journal: Option<PathBuf>,
    pub server: bool,
    pub host: String,
    pub port: Option<u16>,
    pub allow_origin: Vec<String>,
}

/// Run the app: the window, or with `--server` the headless server.
///
/// # Errors
/// A message for `main` to print.
pub(crate) fn run(args: &AppArgs) -> Result<(), String> {
    let app_dirs = AppDirs::from_env();
    #[cfg(feature = "gui")]
    if !args.server {
        return gui::run(args, app_dirs);
    }
    #[cfg(not(feature = "gui"))]
    if !args.server {
        return Err(
            "built without the `gui` feature: pass --server for the local server, or run a \
             command (see --help)"
                .to_string(),
        );
    }
    serve::run(args, app_dirs)
}

/// The state every route shares. Cheap to clone.
#[derive(Clone)]
pub(crate) struct AppState {
    inner: Arc<Shared>,
}

struct Shared {
    books: ArcSwapOption<OpenBooks>,
    app_dirs: AppDirs,
    prefs: UiPrefsSession,
}

impl AppState {
    /// State over `books` (or none yet). UI prefs go to the files unless
    /// [`prefs::IN_BROWSER_ENV`] says the browser keeps them (e2e).
    pub(crate) fn new(books: Option<Arc<OpenBooks>>, app_dirs: AppDirs) -> Self {
        Self {
            inner: Arc::new(Shared {
                books: ArcSwapOption::from(books),
                app_dirs,
                prefs: prefs::session_from_env(),
            }),
        }
    }

    /// The books open now, if any: a stable view for one request even if
    /// File → Open swaps them meanwhile.
    pub(crate) fn books(&self) -> Option<Arc<OpenBooks>> {
        self.inner.books.load_full()
    }

    /// Put `books` in place of whatever was open.
    #[cfg(feature = "gui")]
    pub(crate) fn replace_books(&self, books: Arc<OpenBooks>) {
        self.inner.books.store(Some(books));
    }

    pub(crate) fn app_dirs(&self) -> &AppDirs {
        &self.inner.app_dirs
    }

    pub(crate) fn ui_prefs(&self) -> &UiPrefsSession {
        &self.inner.prefs
    }
}

/// The whole router, for a server bound to `port` with `security` (the
/// token and Host guard, pinned to that port).
pub(crate) fn router(state: AppState, security: Security, token: AccessToken, port: u16) -> Router {
    let gate = match MbrGate::new(token, port) {
        Ok(gate) => Arc::new(gate),
        // No entropy for the session secret: refuse MBR's pages rather than
        // serve them ungated.
        Err(error) => {
            eprintln!("{}: MBR pages are off: {error}", APP.log_prefix);
            Arc::new(MbrGate::closed())
        }
    };
    // The token guard covers exactly the API routes (`route_layer`), as
    // Ledgeline's does.
    let api = security::protect(api::routes(), &security, &[]);
    let router = Router::new()
        .merge(api)
        .route(SPA_BASE, get(spa_page))
        .route(&format!("{SPA_BASE}/"), get(spa_page))
        .route(&format!("{SPA_BASE}/{{*rest}}"), get(spa_page))
        .fallback(mbr::fallback)
        .layer(Extension(gate));
    security::harden(router, security).with_state(state)
}

/// The SPA under [`SPA_BASE`]: the shell's SPA serving, with the base
/// taken off (SvelteKit's `paths.base` puts it on every asset URL).
async fn spa_page(uri: Uri) -> Response {
    let rest = uri.path().strip_prefix(SPA_BASE).unwrap_or_default();
    let inner = if rest.is_empty() { "/" } else { rest };
    match inner.parse::<Uri>() {
        Ok(inner) => spa::respond::<SpaAssets>(&APP, &inner),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

#[cfg(test)]
mod tests;
