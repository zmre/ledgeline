//! Ledgeline's desktop window (default `gui` feature): the shared shell's boot
//! pieces ([`ledgeline_shell::gui`]) strung together around [`AppState`].
//!
//! Boot sequence (the shell's, see its docs):
//!  1. Parse the journal → build the shared [`AppState`].
//!  2. [`shell::spawn_server`]: axum on `127.0.0.1:0` (ephemeral) on a Tokio
//!     runtime; the ACTUAL bound port comes back over a `oneshot`.
//!  3. On the MAIN thread (native UI requires it): the menu bar, the tao
//!     window, then the wry webview pointed at `http://127.0.0.1:<port>/`.
//!  4. muda menus drive actions: File→Open journal… (rfd picker → reparse +
//!     hot-swap the shared state + reload the webview) and View→Reload/Back/
//!     Forward/Zoom (`webview` script/navigation). The Tokio runtime keeps serving on
//!     its worker threads while the (diverging) event loop owns the main thread.
//!
//! What is Ledgeline's here, and not the shell's: the journal, File → New (the
//! plan 39 wizard), File → Export, the "Make Ledgeline the Default" command,
//! which outside pages may open in the system browser, and the event loop that
//! ties them to the state.
//!
//! Because the SPA is served same-origin and the journal is hot-swapped in place,
//! File→Open needs NO server restart: the ephemeral port is stable for the whole
//! session; we just republish the parsed journal and reload the page.
//!
//! Double-clicking a `.journal` in Finder arrives as [`Event::Opened`] and takes
//! the same hot-swap path — see that arm in [`run_event_loop`] for why the
//! document shows up AFTER startup has already parsed a different journal.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use ledgeline::{
    APP, AccessToken, AppState, DesktopHost, boot_appearance, recents, router_with_security,
    token_init_script,
};
use ledgeline_shell::gui::{
    self as shell, AppMenu, MenuSpec, RecentMenu, Shortcut, appearance_init_script, perf,
};
use muda::{IsMenuItem, MenuEvent};
use notify::RecommendedWatcher;
#[cfg(target_os = "linux")]
use tao::{
    event::ElementState,
    keyboard::{Key, ModifiersState},
};
use tao::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy},
};

use crate::{AppError, Cli};

// plan 39 export: File → Export → QIF… / CSV (posting rows)….
#[path = "gui_export.rs"]
mod export;

/// How many recent journals the File → "Open Recent" submenu shows (the on-disk
/// store keeps more; this is just the visible slice).
const RECENT_MENU_LIMIT: usize = 5;

/// The content types Ledgeline ▸ "Make Ledgeline the Default for Journal Files"
/// makes it the default for: every type in `assets/Info.plist.in`'s
/// `Owner`-ranked document type (a unit test holds the two lists together). The
/// `Alternate` ones (`.ledger`, `.j`) are left alone on purpose; see the
/// plist's comment.
#[cfg(target_os = "macos")]
const DEFAULT_CONTENT_TYPES: [&str; 2] = ["com.zmre.ledgeline.journal", "org.ledger-cli.journal"];

/// Ledgeline ▸ "Make Ledgeline the Default for Journal Files".
#[cfg(target_os = "macos")]
static MAKE_DEFAULT: shell::file_assoc::DefaultHandler = shell::file_assoc::DefaultHandler {
    app: &APP,
    menu_id: "make-default-for-journals",
    menu_label: "Make Ledgeline the Default for Journal Files",
    content_types: &DEFAULT_CONTENT_TYPES,
    done_title: "Ledgeline is now the default",
    done_text: "Double-clicking a .journal or .hledger file in Finder now opens it in Ledgeline.",
    failed_title: "Could not make Ledgeline the default",
};

/// Custom events routed through the tao event loop from other threads (the
/// global muda menu handler and the rfd file-picker thread).
enum UserEvent {
    Menu(MenuEvent),
    JournalPicked(PathBuf),
    // plan 39 wizard: the server has already bound its editor to this journal
    // (File → New, or the sample); the shell only has to catch up.
    JournalAdopted(PathBuf),
}

// plan 39 wizard
/// The shell's side of File → New ([`DesktopHost`]): the folder picker, and
/// the notice that the server opened a journal on its own.
///
/// The proxy is behind a mutex because the server calls this from its
/// blocking pool, and tao's proxy is `Send` but not `Sync` on every platform.
struct GuiHost {
    proxy: Mutex<EventLoopProxy<UserEvent>>,
}

impl DesktopHost for GuiHost {
    fn pick_folder(&self) -> Option<PathBuf> {
        rfd::FileDialog::new()
            .set_title("Choose an empty folder for the new ledger")
            .set_can_create_directories(true)
            .pick_folder()
    }

    fn journal_opened(&self, main: &Path) {
        let proxy = self.proxy.lock().unwrap_or_else(PoisonError::into_inner);
        let _ = proxy.send_event(UserEvent::JournalAdopted(main.to_path_buf()));
    }
}

/// GUI entry point: stand up the in-process server, then run the window.
pub(crate) fn run(cli: &Cli, app_dirs: ledgeline::AppDirs) -> Result<(), AppError> {
    perf::start();
    // Same posture the headless server takes (token + Host guard, non-loopback
    // binds refused), decided before the journal is opened. The token is NOT
    // printed here, and it is not in the served shell either: the WebView gets
    // it out of band, from `token_init_script`.
    let (process_token, security_plan) = crate::plan_security(cli)?;
    // Built before the server so the server's File → New routes can reach the
    // loop through a proxy (plan 39 wizard). Still on the main thread.
    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let host = Arc::new(GuiHost {
        proxy: Mutex::new(event_loop.create_proxy()),
    });
    // plan 39 wizard: with no journal named and none recent — a first run —
    // the window opens on the New ledger wizard over an empty, read-only
    // state rather than on a sample journal.
    let journal_path = crate::resolve_journal(cli, &app_dirs);
    let (state, editor_path) = match &journal_path {
        Some(journal_path) => {
            // Bind an editor to the file so the GUI's edit endpoints are live
            // (this is the primary mode). Canonicalize first — like
            // `run_server_blocking` — so the editor's save target and source
            // name match the watcher's canonical path.
            let editor_path = journal_path
                .canonicalize()
                .unwrap_or_else(|_| journal_path.clone());
            let state = AppState::from_journal_path(&editor_path)
                .map_err(|source| AppError::OpenEditor {
                    path: journal_path.display().to_string(),
                    source,
                })?
                .with_app_dirs(app_dirs);
            // Remember this journal as the most-recently-opened (canonical path).
            recents::record(state.app_dirs(), &editor_path);
            (state, Some(editor_path))
        }
        None => (no_journal_state()?.with_app_dirs(app_dirs), None),
    };
    let state = state.with_desktop(host).with_feed_fake_from_env();
    perf::stamp("journal_loaded");

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(AppError::Runtime)?;

    let host = cli.host.clone();
    // GUI mode picks an ephemeral port unless one was explicitly requested.
    let port = cli.port.unwrap_or(0);

    // The Host guard has to name the port we actually got, which is only
    // knowable once bound (GUI mode binds an ephemeral port), so the router is
    // built there.
    let server_state = state.clone();
    let port_rx = shell::spawn_server(&APP, &runtime, &host, port, move |bound| {
        security_plan
            .build(bound)
            .map(|security| router_with_security(server_state, security))
            .map_err(|error| error.to_string())
    });

    // Plan 41: the background price updater, on the same runtime and the same
    // state, so it follows File → Open. Idle unless `autoPrices` is on.
    let _auto_prices = runtime.spawn(ledgeline::run_auto_prices(state.clone()));
    // Plan 45: background recognition posting, idle unless `autoRecognize`.
    let _auto_recognize = runtime.spawn(ledgeline::run_auto_recognize(state.clone()));
    // Plan 43: background bank sync, idle unless `autoSync` is on.
    let _auto_sync = runtime.spawn(ledgeline::run_auto_sync(state.clone()));

    let bound_port = runtime
        .block_on(port_rx)
        .ok()
        .flatten()
        .ok_or(AppError::ServerStart)?;
    let url = format!("http://{host}:{bound_port}/");
    // Live-reload watcher (re-pointed on File→Open); `None` just disables reload.
    let watcher = match &editor_path {
        Some(path) => {
            eprintln!("ledgeline: serving {} at {url}", path.display());
            crate::spawn_watcher(path, state.clone()).ok()
        }
        None => {
            eprintln!("ledgeline: no journal yet; opening the New ledger wizard at {url}");
            None
        }
    };
    // plan 39 wizard: a first run starts on the wizard's page.
    let start = match editor_path {
        Some(_) => url.clone(),
        None => wizard_url(&url),
    };

    run_event_loop(
        event_loop,
        GuiContext {
            url,
            start,
            token: process_token.token,
            state,
            watcher,
            current: editor_path.unwrap_or_default(),
        },
    )
    // `runtime` and the server it spawned stay live on this frame while the
    // event loop runs.
}

// plan 39 wizard
/// The state a first run serves while the wizard is up: an empty journal with
/// no file behind it, so nothing can be edited until a ledger is created or
/// opened.
fn no_journal_state() -> Result<AppState, AppError> {
    let empty = ledgeline_core::parse_journal("", "")
        .map_err(|error| AppError::Gui(format!("building an empty journal: {error}")))?;
    Ok(AppState::from_journal(&empty))
}

// plan 39 wizard
/// The New ledger wizard's page under the app's base url.
fn wizard_url(base: &str) -> String {
    format!("{base}new")
}

/// Everything the event loop owns after startup.
struct GuiContext {
    url: String,
    /// The page the window opens on: `url` itself, or the wizard on a first
    /// run (plan 39 wizard).
    start: String,
    /// The access token, handed to the WebView by [`token_init_script`].
    token: AccessToken,
    state: AppState,
    watcher: Option<RecommendedWatcher>,
    /// Canonical path of the journal currently open (excluded from the recents
    /// submenu, and updated on each File→Open / Open Recent). Empty while a
    /// first run has none.
    current: PathBuf,
}

/// The published docs site, one of the few places outside the app the SPA links to.
const DOCS_HOST: &str = "zmre.github.io";
const DOCS_PATH: &str = "/ledgeline/";

/// SimpleFIN's own sites (plan 43): the Bridge, where a user makes a setup
/// token, fixes a bank sign-in or revokes access, and the protocol's home page.
/// Any path on these hosts.
const SIMPLEFIN_HOSTS: [&str; 4] = [
    "beta-bridge.simplefin.org",
    "bridge.simplefin.org",
    "www.simplefin.org",
    "simplefin.org",
];

/// `candidate`, re-serialized, when it is a page the webview may hand to the
/// system browser: the published docs site, SimpleFIN's, or exactly one of
/// `saved` — the bank download pages saved for the open ledger (plan 43). Both
/// sides of that last test are re-serialized, so a saved `https://bank.example`
/// matches the `https://bank.example/` a click produces. Everything else stays
/// blocked: a link the SPA did not mean to show (or one injected into it) must
/// not become a way to launch arbitrary URLs.
fn external_url(candidate: &str, saved: impl FnOnce() -> Vec<String>) -> Option<String> {
    let url = shell::plain_https(candidate)?;
    let host = url.host_str()?;
    let known = (host == DOCS_HOST && url.path().starts_with(DOCS_PATH))
        || SIMPLEFIN_HOSTS.contains(&host)
        || saved()
            .iter()
            .filter_map(|page| shell::plain_https(page))
            .any(|page| page == url);
    known.then(|| url.to_string())
}

/// Send `candidate` to the system browser when [`external_url`] allows it.
/// Returns whether it did. The page never loads in the app's own window.
fn open_if_external(state: &AppState, candidate: &str) -> bool {
    external_url(candidate, || ledgeline::saved_download_pages(state))
        .map(|url| shell::open_in_browser(&APP, &url))
        .is_some()
}

/// Show the rfd file picker on a background thread (it blocks) and forward the
/// choice back into the event loop.
fn spawn_file_picker(proxy: EventLoopProxy<UserEvent>) {
    std::thread::spawn(move || {
        if let Some(path) = rfd::FileDialog::new()
            .set_title("Open journal")
            .add_filter("hledger journal", &["journal", "hledger", "ledger", "j"])
            .add_filter("All files", &["*"])
            .pick_file()
        {
            let _ = proxy.send_event(UserEvent::JournalPicked(path));
        }
    });
}

/// Report a failed File→Open without disturbing the currently loaded journal.
fn show_open_error(path: &Path, error: &AppError) {
    shell::show_error(
        "Failed to open journal",
        format!(
            "Could not open {}:\n{error}\n\nThe current journal stays loaded.",
            path.display()
        ),
    );
}

/// Ledgeline's menu bar: the shell's standard one, with Export after Open
/// Recent and (macOS) "Make Ledgeline the Default" in the app menu.
fn build_menu() -> AppMenu {
    let export = export::submenu();
    let after_recent: [&dyn IsMenuItem; 1] = [&export];
    #[cfg(target_os = "macos")]
    let make_default = MAKE_DEFAULT.menu_item();
    #[cfg(target_os = "macos")]
    let app_menu_items: &[&dyn IsMenuItem] = &[&make_default];
    #[cfg(not(target_os = "macos"))]
    let app_menu_items: &[&dyn IsMenuItem] = &[];
    AppMenu::build(
        &APP,
        &MenuSpec {
            version: env!("CARGO_PKG_VERSION"),
            new_label: "&New…", // plan 39 wizard
            open_label: "&Open journal…",
            app_menu_items,
            after_recent: &after_recent,
        },
    )
}

/// Build the window + webview on the main thread and run the (diverging) event
/// loop until the window closes.
fn run_event_loop(
    event_loop: tao::event_loop::EventLoop<UserEvent>,
    ctx: GuiContext,
) -> Result<(), AppError> {
    // Route muda menu activations into the loop as user events.
    let menu_proxy = event_loop.create_proxy();
    MenuEvent::set_event_handler(Some(move |event| {
        let _ = menu_proxy.send_event(UserEvent::Menu(event));
    }));

    let menu = build_menu();
    let window = shell::build_window(&APP, &event_loop, &menu.menu_bar)
        .map_err(|error| AppError::Gui(error.to_string()))?;

    perf::stamp("window_built");
    let appearance = boot_appearance(&ctx.state);
    let origin =
        shell::webview_origin(&ctx.url).map_err(|error| AppError::Gui(error.to_string()))?;
    let navigating = ctx.state.clone();
    let webview = shell::build_webview(
        &APP,
        &window,
        &ctx.url,
        &ctx.start,
        vec![
            token_init_script(&origin, &ctx.token),
            appearance_init_script(&origin, &appearance),
        ],
        // A docs, SimpleFIN or saved download-page link leaves the webview
        // where it is and opens in the user's browser instead.
        move |candidate| open_if_external(&navigating, candidate),
    )
    .map_err(|error| AppError::Gui(error.to_string()))?;
    perf::stamp("webview_built");

    let recent = RecentMenu::new(menu.recent.clone(), RECENT_MENU_LIMIT, "No recent journals");

    // Linux: the in-window GtkMenuBar looks out of place on a modern desktop,
    // and under a tiling Wayland compositor there is no global menu bar to move
    // it to — so it starts HIDDEN, together with the window decorations, and
    // F10 toggles both.
    //
    // THE TRAP, and it is the opposite of what the obvious reading suggests:
    // hiding the bar disables every accelerator attached to it. muda adds its
    // GtkAccelGroup to the WINDOW, not to the bar, and `hide_for_gtk_window`
    // only calls `hide()` on the bar, so the shortcuts look like they must
    // survive. They do not — `gtk_menu_item_can_activate_accel` chains up the
    // widget ancestry and refuses when any ancestor is invisible. The accel
    // group stays attached and nothing on it activates. Measured both ways on a
    // running window; see the keyboard arm in the loop for the other half.
    //
    // Hence the keyboard route below, and hence it is gated on the bar being
    // hidden: exactly one route is live at a time.
    #[cfg(target_os = "linux")]
    let menu_bar = menu.menu_bar.clone();
    #[cfg(target_os = "linux")]
    let chrome_visible = std::cell::Cell::new(false);
    // tao reports modifiers as a separate event, so the keyboard arm needs the
    // last state rather than being able to read it off the key event.
    #[cfg(target_os = "linux")]
    let modifiers = std::cell::Cell::new(ModifiersState::empty());
    #[cfg(target_os = "linux")]
    {
        shell::disable_gtk_menu_bar_accel();
        shell::set_window_chrome_visible(&APP, &menu_bar, &window, false);
    }

    let picker_proxy = event_loop.create_proxy();

    // Session state owned by the loop. `url` is stable for the whole session
    // (the ephemeral port never changes), so reload/navigation always target it.
    // The live-reload watcher is pure RAII — held in a cell so File→Open can swap
    // it (dropping the old one) without an assignment the borrow checker reads as
    // dead across the `FnMut` boundary.
    let url = ctx.url;
    let state = ctx.state;
    let watcher: RefCell<Option<RecommendedWatcher>> = RefCell::new(ctx.watcher);
    // Canonical path of the open journal (excluded from the recents submenu),
    // refreshed on every open.
    let current: RefCell<PathBuf> = RefCell::new(ctx.current);
    {
        let current_ref = current.borrow();
        recent.rebuild(&APP, state.app_dirs(), Some(current_ref.as_path()));
    }

    event_loop.run(move |event, _target, control_flow| {
        *control_flow = ControlFlow::Wait;

        // Unified open path shared by File→Open and Open Recent: rebind the editor
        // to `raw` (opening a fresh editor, republishing its snapshot, swapping it
        // into the editor mutex) so edits target the new file, then re-point the
        // watcher, record it as most-recent, refresh the submenu, and reload the
        // page. Canonicalize to match the watcher's path, as at startup.
        //
        // `adopt` is the half that follows a successful rebind. File → New
        // (plan 39 wizard) runs it on its own: the server has already rebound
        // the editor, and the SPA navigates itself, so there is no page reload.
        let adopt = |editor_path: &Path| {
            watcher.replace(crate::spawn_watcher(editor_path, state.clone()).ok());
            recents::record(state.app_dirs(), editor_path);
            current.replace(editor_path.to_path_buf());
            recent.rebuild(&APP, state.app_dirs(), Some(editor_path));
            eprintln!("ledgeline: opened {}", editor_path.display());
        };
        let open_journal = |raw: &Path| {
            let editor_path = raw.canonicalize().unwrap_or_else(|_| raw.to_path_buf());
            match state.rebind_editor(&editor_path) {
                Ok(()) => {
                    adopt(&editor_path);
                    let _ = webview.load_url(&url);
                }
                Err(source) => {
                    let error = AppError::OpenEditor {
                        path: raw.display().to_string(),
                        source,
                    };
                    eprintln!("ledgeline: could not open {}: {error}", raw.display());
                    show_open_error(raw, &error);
                }
            }
        };

        // The ONE implementation of each action. Both the menu route and (on
        // Linux) the keyboard route call this, so the two cannot drift.
        //
        // Returns whether the app should exit, rather than touching
        // `control_flow` itself: `control_flow` is a `&mut` borrowed by the
        // enclosing `FnMut`, and capturing it here as well would not compile.
        let perform_shortcut = |shortcut: Shortcut| -> bool {
            match shortcut {
                // plan 39 wizard
                Shortcut::New => {
                    let _ = webview.load_url(&wizard_url(&url));
                }
                Shortcut::Open => spawn_file_picker(picker_proxy.clone()),
                Shortcut::Reload => {
                    let _ = webview.load_url(&url);
                }
                Shortcut::Back => {
                    let _ = webview.evaluate_script("history.back()");
                }
                Shortcut::Forward => {
                    let _ = webview.evaluate_script("history.forward()");
                }
                Shortcut::Zoom(zoom) => {
                    let _ = webview.evaluate_script(shell::zoom_script(zoom));
                }
                #[cfg(target_os = "linux")]
                Shortcut::Quit => return true,
            }
            false
        };

        match event {
            Event::UserEvent(UserEvent::Menu(menu_event)) => {
                // plan 39 export: runs on its own thread, against the open journal.
                if export::handle(&menu_event.id, &current.borrow()) {
                    return;
                }
                #[cfg(target_os = "macos")]
                if MAKE_DEFAULT.handle(&menu_event.id) {
                    return;
                }
                if let Some(shortcut) = menu.shortcut_for(&menu_event.id) {
                    if perform_shortcut(shortcut) {
                        #[cfg(target_os = "linux")]
                        shell::stop_web_process(&webview);
                        *control_flow = ControlFlow::Exit;
                    }
                } else if let Some(path) = recent.picked(&menu_event.id) {
                    // An Open Recent item; `picked` has released the map's
                    // borrow, so opening (which rebuilds the submenu) is safe.
                    open_journal(&path);
                }
                // PredefinedMenuItem events (quit, copy, …) are handled natively.
            }
            Event::UserEvent(UserEvent::JournalPicked(path)) => {
                open_journal(&path);
            }
            // plan 39 wizard
            Event::UserEvent(UserEvent::JournalAdopted(path)) => {
                adopt(&path);
            }
            // A document launch: Finder double-click, `open -a Ledgeline x.journal`,
            // or a drop on the Dock icon. macOS does NOT pass the document in argv
            // — it sends `application:openURLs:` — so this arm is the ONLY way that
            // path reaches us.
            //
            // Why the event is never missed, and why it still arrives "late": tao
            // installs this callback before it calls `NSApp run` (see its
            // `run_return`), and AppKit delivers `application:openURLs:` after
            // `applicationDidFinishLaunching`, so the ordering guarantees we are
            // listening by the time the document shows up. But `run()` above has by
            // then ALREADY resolved a journal from `$LEDGELINE_FIXTURE`/recents and
            // parsed it, which is why a Finder launch can flash the previous journal
            // before switching, and why the identity guard below is worth having.
            //
            // Not `cfg`-gated: only macOS emits `Opened`, but the variant exists on
            // every platform, so an unconditional arm compiles everywhere and is
            // simply dead elsewhere — cheaper than a `cfg` that can rot.
            Event::Opened { urls } => {
                // Never drop a requested document silently; say why it was skipped.
                for url in urls.iter().filter(|url| url.to_file_path().is_err()) {
                    eprintln!("ledgeline: ignoring {url}: not a file:// URL");
                }
                if let Some(path) = shell::first_file_path(&urls) {
                    for extra in urls
                        .iter()
                        .filter_map(|url| url.to_file_path().ok())
                        .skip(1)
                    {
                        eprintln!(
                            "ledgeline: ignoring {}: one journal per window",
                            extra.display()
                        );
                    }
                    // The COMMON case: the document double-clicked is the journal
                    // startup just parsed (it was the most-recent one), so opening
                    // it again would redo that work and needlessly reload the page.
                    // Compare canonically, the same normalization `open_journal`
                    // and the watcher use. The borrow ends with this block —
                    // `open_journal` re-borrows `current` and would panic otherwise
                    // (the Open Recent arm above drops its borrow for the same
                    // reason). This guard is deliberately ONLY here: File→Open and
                    // Open Recent are explicit user requests and keep their
                    // always-reload semantics, which double as a manual refresh.
                    let already_open = {
                        let canonical = path.canonicalize().unwrap_or_else(|_| path.clone());
                        canonical == *current.borrow()
                    };
                    if already_open {
                        eprintln!(
                            "ledgeline: {} is already open; keeping the parsed journal",
                            path.display()
                        );
                    } else {
                        open_journal(&path);
                    }
                }
            }
            // tao delivers modifier state as its own event, so it has to be
            // remembered for the key arm below to consult.
            #[cfg(target_os = "linux")]
            Event::WindowEvent {
                event: WindowEvent::ModifiersChanged(state),
                ..
            } => {
                modifiers.set(state);
            }
            // The Linux keyboard route. Two decisions here were MEASURED on this
            // stack rather than reasoned about, and both are easy to get wrong:
            //
            // * Act on `Released`, NOT `Pressed`. WebKitGTK hands a key press to
            //   the web process and, when the page leaves it unhandled,
            //   RE-DISPATCHES the same press to the toplevel so that window
            //   accelerators still work. So a `Pressed` arm fires TWICE for
            //   exactly the keys nothing in the page claims — which is precisely
            //   the set this arm wants. The release arrives exactly once either
            //   way.
            //
            // * Match on `logical_key`, NOT `physical_key`. `logical_key` is the
            //   key's meaning — what the user reads off the keycap, and what
            //   survives a remapped layout. `physical_key` is tao's mapping of
            //   the raw hardware keycode, and any input source that synthesises
            //   events on a keymap of its own fills it with whatever slot it
            //   happened to use (`wtype` reports `Escape` for both `F10` and
            //   `f`, while `logical_key` stays correct for both).
            #[cfg(target_os = "linux")]
            Event::WindowEvent {
                event: WindowEvent::KeyboardInput { event: key, .. },
                ..
            } if key.state == ElementState::Released => {
                if key.logical_key == Key::F10 {
                    // Deliberately NOT gated on visibility: the toggle is the one
                    // key that has to work in both states, so it gets its own arm.
                    let visible = !chrome_visible.get();
                    shell::set_window_chrome_visible(&APP, &menu_bar, &window, visible);
                    chrome_visible.set(visible);
                } else if !chrome_visible.get() {
                    // Gated. With the bar VISIBLE its GtkAccelGroup already fires
                    // these, so an ungated arm would run every shortcut twice the
                    // moment the bar came back.
                    if let Some(shortcut) =
                        shell::linux_shortcut_for(&key.logical_key, modifiers.get())
                        && perform_shortcut(shortcut)
                    {
                        shell::stop_web_process(&webview);
                        *control_flow = ControlFlow::Exit;
                    }
                }
            }
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                #[cfg(target_os = "linux")]
                shell::stop_web_process(&webview);
                *control_flow = ControlFlow::Exit;
            }
            _ => {}
        }
    });
}

#[cfg(test)]
mod tests {
    use super::external_url;

    #[test]
    fn only_docs_and_simplefin_pages_go_to_the_system_browser() {
        for allowed in [
            "https://zmre.github.io/ledgeline/journal/transfer-matching/",
            "https://zmre.github.io/ledgeline/",
            "https://beta-bridge.simplefin.org/",
            "https://bridge.simplefin.org/simplefin/create",
            "https://www.simplefin.org/protocol.html",
        ] {
            assert_eq!(external_url(allowed, Vec::new).as_deref(), Some(allowed));
        }
        for refused in [
            "http://zmre.github.io/ledgeline/journal/",
            "https://zmre.github.io/other/",
            "https://zmre.github.io/ledgeline",
            "https://zmre.github.io.evil.example/ledgeline/",
            "https://evil.example/ledgeline/",
            "https://user@zmre.github.io/ledgeline/",
            "https://zmre.github.io:8443/ledgeline/",
            "http://beta-bridge.simplefin.org/",
            "https://beta-bridge.simplefin.org.evil.example/",
            "https://evil-beta-bridge.simplefin.org/",
            "https://user:secret@beta-bridge.simplefin.org/simplefin",
            "https://beta-bridge.simplefin.org:8443/",
            "file:///ledgeline/",
            "javascript:alert(1)",
            "not a url",
        ] {
            assert_eq!(external_url(refused, Vec::new), None, "{refused}");
        }
    }

    #[test]
    fn a_saved_download_page_goes_to_the_system_browser_and_nothing_near_it() {
        let saved = || {
            vec![
                "https://bank.example".to_string(),
                " https://card.example/statements?type=csv ".to_string(),
                "http://plain.example/".to_string(),
            ]
        };
        assert_eq!(
            external_url("https://bank.example/", saved).as_deref(),
            Some("https://bank.example/")
        );
        assert!(external_url("https://card.example/statements?type=csv", saved).is_some());
        for refused in [
            "https://bank.example/other",
            "https://card.example/statements",
            "https://bank.example.evil.example/",
            "https://user@bank.example/",
            "https://bank.example:8443/",
            "http://bank.example/",
            "http://plain.example/",
            "https://plain.example/",
        ] {
            assert_eq!(external_url(refused, saved), None, "{refused}");
        }
    }

    /// Ledgeline's WebView reads exactly this global; the shell builds it from
    /// [`ledgeline::APP`].
    #[test]
    fn the_token_script_hands_over_ledgelines_global() {
        let token = ledgeline::AccessToken::parse("token-for-unit-tests").expect("well-formed");
        assert_eq!(
            ledgeline::token_init_script("http://127.0.0.1:5000", &token),
            "if (window.location.origin === \"http://127.0.0.1:5000\") \
             { window.__LEDGELINE_TOKEN__ = \"token-for-unit-tests\"; }"
        );
    }
}

/// The "Make Ledgeline the Default" command against the app's real plist.
#[cfg(all(test, target_os = "macos"))]
mod file_assoc_tests {
    use std::path::Path;

    use super::{DEFAULT_CONTENT_TYPES, MAKE_DEFAULT};

    /// The plist template, read at run time: `include_str!` would reach outside
    /// the published crate. The Nix source filter keeps this one file.
    fn info_plist() -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/Info.plist.in");
        std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("reading {}: {error}", path.display()))
    }

    /// Text between `<key>{key}</key>` and the next top-level key.
    fn section<'a>(plist: &'a str, key: &str, next_key: &str) -> &'a str {
        let start = plist
            .find(&format!("<key>{key}</key>"))
            .unwrap_or_else(|| panic!("{key} missing from Info.plist.in"));
        let end = plist[start..]
            .find(&format!("<key>{next_key}</key>"))
            .map_or(plist.len(), |offset| start + offset);
        &plist[start..end]
    }

    #[test]
    fn the_plist_claims_every_type_the_command_sets_at_owner_rank() {
        let plist = info_plist();
        let document_types = section(
            &plist,
            "CFBundleDocumentTypes",
            "UTExportedTypeDeclarations",
        );
        let owner = document_types
            .split("</dict>")
            .find(|dict| dict.contains("<string>Owner</string>"))
            .expect("an Owner-ranked document type");
        let claimed: Vec<&str> = owner
            .split("<string>")
            .skip(1)
            .filter_map(|rest| rest.split("</string>").next())
            .filter(|value| value.contains('.'))
            .collect();
        assert_eq!(claimed, DEFAULT_CONTENT_TYPES);
        assert_eq!(MAKE_DEFAULT.content_types, DEFAULT_CONTENT_TYPES);
    }

    #[test]
    fn the_plist_bundle_id_is_the_one_the_command_names() {
        assert!(info_plist().contains(&format!(
            "<key>CFBundleIdentifier</key>\n  <string>{}</string>",
            MAKE_DEFAULT.app.bundle_id
        )));
    }
}
