//! The app (the `server` feature): one local HTTP server, either headless
//! (`--server`) or in-process behind the desktop window (`gui`), booted
//! through `ledgeline-shell` exactly as Ledgeline boots (invoicer PLAN 3a,
//! ARCHITECTURE §6.1).
//!
//! The one server's routes:
//!
//! | Path                     | Served by                                                    |
//! |--------------------------|--------------------------------------------------------------|
//! | `/api/*`                 | The invoicer API ([`api`]): bearer token required.           |
//! | `/app/*`                 | The invoicer SPA (`invoicer/web`, SvelteKit base `/app`).    |
//! | `/.ledgeline-invoicer/*` | Export links ([`mbr`]) and the preview page ([`preview`]).   |
//! | the rest                 | MBR's router, unmodified ([`mbr`]): behind a session cookie. |
//!
//! Every response passes the shell's Host guard and security headers. The
//! open books ([`books::OpenBooks`]) are swapped whole on File → Open, MBR
//! server included; the watcher ([`watch`]) feeds the reconciler.

pub(crate) mod api;
pub(crate) mod books;
pub(crate) mod customers;
pub(crate) mod dashboard;
#[cfg(feature = "gui")]
pub(crate) mod desktop;
pub(crate) mod editor;
pub(crate) mod edits;
pub(crate) mod files;
#[cfg(feature = "gui")]
mod gui;
pub(crate) mod library;
pub(crate) mod mbr;
pub(crate) mod native;
pub(crate) mod pdf;
pub(crate) mod prefs;
pub(crate) mod preview;
mod serve;
pub(crate) mod templates;
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
use native::Native;

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
    // `SPA_BASE` with its trailing slash: MBR's pages share the origin, so
    // the window shows, and the token reaches, only the SPA under `/app/`.
    spa_base: "/app/",
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
    /// The desktop (PDF, Share…, Copy file, the browser), when this
    /// process has an event loop to serve it ([`native`]).
    native: Option<Arc<dyn Native>>,
}

impl AppState {
    /// State over `books` (or none yet). UI prefs go to the files unless
    /// [`prefs::IN_BROWSER_ENV`] says the browser keeps them (e2e).
    #[cfg(test)]
    pub(crate) fn new(books: Option<Arc<OpenBooks>>, app_dirs: AppDirs) -> Self {
        Self::with_native(books, app_dirs, None)
    }

    /// [`AppState::new`], with the desktop's [`Native`] side.
    pub(crate) fn with_native(
        books: Option<Arc<OpenBooks>>,
        app_dirs: AppDirs,
        native: Option<Arc<dyn Native>>,
    ) -> Self {
        Self {
            inner: Arc::new(Shared {
                books: ArcSwapOption::from(books),
                app_dirs,
                prefs: prefs::session_from_env(),
                native,
            }),
        }
    }

    /// The desktop's native side, if this process has one.
    pub(crate) fn native(&self) -> Option<Arc<dyn Native>> {
        self.inner.native.clone()
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
    router_with_gate(state, security, gate)
}

/// [`router`] around a gate the caller keeps too (the CLI's `pdf` mints
/// its export link from it).
pub(crate) fn router_with_gate(state: AppState, security: Security, gate: Arc<MbrGate>) -> Router {
    // The token guard covers exactly the API routes (`route_layer`), as
    // Ledgeline's does.
    let api = security::protect(api::routes(), &security, &[]);
    let router = Router::new()
        .merge(api)
        .route(SPA_BASE, get(spa_page))
        .route(&format!("{SPA_BASE}/"), get(spa_page))
        .route(&format!("{SPA_BASE}/{{*rest}}"), get(spa_page))
        // The PDF export's one-time links (no token: the ticket is the key).
        .route(
            &format!("{}{{ticket}}", mbr::EXPORT_LINK_PREFIX),
            get(mbr::export_link),
        )
        // A preview through another template, as a page for the SPA's
        // frame (behind MBR's gate, under MBR's CSP).
        .route(preview::PREVIEW_PAGE, get(preview::page))
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
mod customers_tests;
#[cfg(test)]
mod dashboard_tests;
#[cfg(test)]
mod files_tests;
#[cfg(test)]
mod library_tests;
#[cfg(test)]
mod mbr_csp_tests;
#[cfg(test)]
mod payments_tests;
#[cfg(test)]
mod spa_shell_csp_tests;
#[cfg(test)]
mod tests;

#[cfg(test)]
mod identity_tests {
    use ledgeline_shell::security::{AccessToken, in_spa, token_init_script};

    use super::{APP, SPA_BASE};

    /// The shell's SPA base is the router's mount with its trailing slash, so
    /// the window and the token follow wherever the SPA is served.
    #[test]
    fn the_shells_spa_base_is_the_routers_mount() {
        assert_eq!(APP.spa_base, format!("{SPA_BASE}/"));
    }

    /// SEC-6: the window and the token reach the SPA only, never an MBR page
    /// on the same origin.
    #[test]
    fn only_the_spa_is_the_app() {
        let origin = "http://127.0.0.1:5199";
        assert!(in_spa(origin, APP.spa_base, "http://127.0.0.1:5199/app/"));
        assert!(in_spa(
            origin,
            APP.spa_base,
            "http://127.0.0.1:5199/app/new"
        ));
        assert!(!in_spa(origin, APP.spa_base, "http://127.0.0.1:5199/"));
        assert!(!in_spa(
            origin,
            APP.spa_base,
            "http://127.0.0.1:5199/customers/contracts/sow.html"
        ));
        let token = AccessToken::parse("token-for-unit-tests").expect("well-formed");
        let script = token_init_script(&APP, origin, &token);
        assert!(
            script.contains("window.location.pathname.startsWith(\"/app/\")"),
            "{script}"
        );
    }
}
