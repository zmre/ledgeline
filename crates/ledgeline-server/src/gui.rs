//! Native desktop shell (default `gui` feature): a tao window + wry webview that
//! renders the in-process SPA, with muda menus and an rfd file picker.
//!
//! Boot sequence (mirrors the proven order in zmre/mbr-markdown-browser):
//!  1. Parse the journal → build the shared [`AppState`].
//!  2. Spawn axum on `127.0.0.1:0` (ephemeral) on a Tokio runtime; read the
//!     ACTUAL bound port back over a `oneshot`.
//!  3. On the MAIN thread (native UI requires it): build the tao `EventLoop` +
//!     window, then the wry webview pointed at `http://127.0.0.1:<port>/`.
//!  4. muda menus drive actions: File→Open journal… (rfd picker → reparse +
//!     hot-swap the shared state + reload the webview) and View→Reload/Back/
//!     Forward (`webview` script/navigation). The Tokio runtime keeps serving on
//!     its worker threads while the (diverging) event loop owns the main thread.
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

use ledgeline_server::{AppState, router_with_security};
use muda::{
    Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu,
    accelerator::{Accelerator, Code, Modifiers},
};
use notify::RecommendedWatcher;
use tao::{
    dpi::LogicalSize,
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy},
    window::WindowBuilder,
};
use url::Url;
use wry::WebViewBuilder;

use crate::{AppError, Cli};

/// How many recent journals the File → "Open Recent" submenu shows (the on-disk
/// store keeps more; this is just the visible slice).
const RECENT_MENU_LIMIT: usize = 5;

/// Custom events routed through the tao event loop from other threads (the
/// global muda menu handler and the rfd file-picker thread).
enum UserEvent {
    Menu(MenuEvent),
    JournalPicked(PathBuf),
}

/// GUI entry point: stand up the in-process server, then run the window.
pub(crate) fn run(cli: &Cli) -> Result<(), AppError> {
    // Same posture the headless server takes (token + Host guard, non-loopback
    // binds refused), decided before the journal is opened. The token is NOT
    // printed here: the WebView picks it up from the served shell.
    let (_process_token, security_plan) = crate::plan_security(cli)?;
    let journal_path = crate::resolve_journal(cli);
    // Bind an editor to the file so the GUI's edit endpoints are live (this is the
    // primary mode). Canonicalize first — like `run_server_blocking` — so the
    // editor's save target and source name match the watcher's canonical path.
    let editor_path = journal_path
        .canonicalize()
        .unwrap_or_else(|_| journal_path.clone());
    let state =
        AppState::from_journal_path(&editor_path).map_err(|source| AppError::OpenEditor {
            path: journal_path.display().to_string(),
            source,
        })?;
    // Remember this journal as the most-recently-opened (canonical path).
    crate::recents::record(&editor_path);

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(AppError::Runtime)?;

    let host = cli.host.clone();
    // GUI mode picks an ephemeral port unless one was explicitly requested.
    let port = cli.port.unwrap_or(0);

    let (port_tx, port_rx) = tokio::sync::oneshot::channel::<Option<u16>>();
    let server_state = state.clone();
    let server_host = host.clone();

    // Spawn axum; the JoinHandle is kept alive on this frame (the diverging event
    // loop below never returns, so the runtime keeps serving for the session).
    let _server = runtime.spawn(async move {
        let addr = format!("{server_host}:{port}");
        match tokio::net::TcpListener::bind(&addr).await {
            Ok(listener) => {
                let bound = listener.local_addr().map(|a| a.port()).ok();
                let _ = port_tx.send(bound);
                // The Host guard has to name the port we actually got, which is
                // only knowable here (GUI mode binds an ephemeral port).
                let security = match bound.map(|port| security_plan.build(port)) {
                    Some(Ok(security)) => security,
                    Some(Err(error)) => {
                        eprintln!("ledgeline: security setup failed: {error}");
                        return;
                    }
                    // The bound port is unknown, so the Host guard cannot be
                    // pinned; refuse to serve rather than serve unguarded. The
                    // main thread already sees `None` and reports ServerStart.
                    None => return,
                };
                if let Err(error) =
                    axum::serve(listener, router_with_security(server_state, security)).await
                {
                    eprintln!("ledgeline: server error: {error}");
                }
            }
            Err(error) => {
                eprintln!("ledgeline: bind error on {addr}: {error}");
                let _ = port_tx.send(None);
            }
        }
    });

    let bound_port = runtime
        .block_on(port_rx)
        .ok()
        .flatten()
        .ok_or(AppError::ServerStart)?;
    let url = format!("http://{host}:{bound_port}/");
    eprintln!("ledgeline: serving {} at {url}", journal_path.display());

    // Live-reload watcher (re-pointed on File→Open); `None` just disables reload.
    let watcher = crate::spawn_watcher(&journal_path, state.clone()).ok();

    run_event_loop(GuiContext {
        url,
        state,
        watcher,
        current: editor_path,
    })
    // `runtime` and `_server` stay live on this frame while the event loop runs.
}

/// Everything the event loop owns after startup.
struct GuiContext {
    url: String,
    state: AppState,
    watcher: Option<RecommendedWatcher>,
    /// Canonical path of the journal currently open (excluded from the recents
    /// submenu, and updated on each File→Open / Open Recent).
    current: PathBuf,
}

/// Menu bar plus the handles for the custom items we match events against.
struct AppMenu {
    menu_bar: Menu,
    open: MenuItem,
    /// File → "Open Recent" submenu; repopulated as journals are opened.
    recent: Submenu,
    reload: MenuItem,
    back: MenuItem,
    forward: MenuItem,
}

// Referenced only by the macOS app menu's About item, so it does not exist on
// other platforms (where it would be dead code under `-D warnings`).
#[cfg(target_os = "macos")]
fn about_metadata() -> muda::AboutMetadata {
    muda::AboutMetadata {
        name: Some("Ledgeline".to_string()),
        version: Some(env!("CARGO_PKG_VERSION").to_string()),
        ..Default::default()
    }
}

/// Log (rather than crash on) a menu-construction failure — a degraded menu is
/// cosmetic; aborting startup would be far worse.
fn log_menu(what: &str, result: Result<(), muda::Error>) {
    if let Err(error) = result {
        eprintln!("ledgeline: failed to build {what}: {error}");
    }
}

/// The modifier that means "the app's command key": Cmd on macOS, Ctrl on Linux
/// and Windows.
///
/// [`Modifiers::SUPER`] is Cmd on macOS but the **Super/Windows key** elsewhere,
/// which belongs to the desktop rather than to us — a tiling compositor such as
/// Hyprland binds nearly the whole Super range, so a `Super+O` accelerator does
/// not merely render as a wrong-looking "Meta+O" in the menu, it never fires at
/// all. Every accelerator that is Cmd-something on macOS goes through this.
#[cfg(target_os = "macos")]
const COMMAND_MODIFIER: Modifiers = Modifiers::SUPER;
#[cfg(not(target_os = "macos"))]
const COMMAND_MODIFIER: Modifiers = Modifiers::CONTROL;

/// Build the application menu bar (macOS app menu + File/Edit/View).
fn build_menu() -> AppMenu {
    let menu_bar = Menu::new();

    #[cfg(target_os = "macos")]
    let app_menu = {
        let app_menu = Submenu::new("Ledgeline", true);
        log_menu(
            "app menu",
            app_menu.append_items(&[
                &PredefinedMenuItem::about(None, Some(about_metadata())),
                &PredefinedMenuItem::separator(),
                &PredefinedMenuItem::services(None),
                &PredefinedMenuItem::separator(),
                &PredefinedMenuItem::hide(None),
                &PredefinedMenuItem::hide_others(None),
                &PredefinedMenuItem::show_all(None),
                &PredefinedMenuItem::separator(),
                &PredefinedMenuItem::quit(None),
            ]),
        );
        app_menu
    };

    let file_menu = Submenu::new("&File", true);
    let open = MenuItem::with_id(
        "open",
        "&Open journal…",
        true,
        Some(Accelerator::new(Some(COMMAND_MODIFIER), Code::KeyO)),
    );
    // Populated on demand by `rebuild_recent` (empty until then).
    let recent = Submenu::new("Open &Recent", true);
    log_menu(
        "file menu",
        file_menu.append_items(&[
            &open,
            &recent,
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::close_window(Some("Close Window")),
        ]),
    );
    #[cfg(not(target_os = "macos"))]
    log_menu(
        "file menu quit",
        file_menu.append_items(&[
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::quit(None),
        ]),
    );

    // Standard clipboard items so Cmd/Ctrl+C/V/X work inside the webview.
    let edit_menu = Submenu::new("&Edit", true);
    log_menu(
        "edit menu",
        edit_menu.append_items(&[
            &PredefinedMenuItem::undo(None),
            &PredefinedMenuItem::redo(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::cut(None),
            &PredefinedMenuItem::copy(None),
            &PredefinedMenuItem::paste(None),
            &PredefinedMenuItem::select_all(None),
        ]),
    );

    let view_menu = Submenu::new("&View", true);
    let reload = MenuItem::with_id(
        "reload",
        "&Reload",
        true,
        Some(Accelerator::new(Some(COMMAND_MODIFIER), Code::KeyR)),
    );
    // Back/Forward are the one pair where the platforms disagree about the KEY
    // and not just the modifier: macOS browsers use Cmd+[ / Cmd+], while Linux
    // and Windows use Alt+← / Alt+→. So this is not a COMMAND_MODIFIER swap.
    #[cfg(target_os = "macos")]
    let (back_accel, forward_accel) = (
        Accelerator::new(Some(COMMAND_MODIFIER), Code::BracketLeft),
        Accelerator::new(Some(COMMAND_MODIFIER), Code::BracketRight),
    );
    #[cfg(not(target_os = "macos"))]
    let (back_accel, forward_accel) = (
        Accelerator::new(Some(Modifiers::ALT), Code::ArrowLeft),
        Accelerator::new(Some(Modifiers::ALT), Code::ArrowRight),
    );
    let back = MenuItem::with_id("back", "&Back", true, Some(back_accel));
    let forward = MenuItem::with_id("forward", "&Forward", true, Some(forward_accel));
    log_menu(
        "view menu",
        view_menu.append_items(&[&reload, &PredefinedMenuItem::separator(), &back, &forward]),
    );

    #[cfg(target_os = "macos")]
    log_menu(
        "menu bar",
        menu_bar.append_items(&[&app_menu, &file_menu, &edit_menu, &view_menu]),
    );
    #[cfg(not(target_os = "macos"))]
    log_menu(
        "menu bar",
        menu_bar.append_items(&[&file_menu, &edit_menu, &view_menu]),
    );

    AppMenu {
        menu_bar,
        open,
        recent,
        reload,
        back,
        forward,
    }
}

/// Repopulate the File → "Open Recent" submenu from the recents store, excluding
/// the currently-open journal (`current`), and refresh the id→path map used to
/// dispatch a click. Called at startup and after every successful open so the
/// list always reflects the latest history.
fn rebuild_recent(
    submenu: &Submenu,
    map: &RefCell<Vec<(MenuId, PathBuf)>>,
    current: Option<&Path>,
) {
    // Clear existing entries (remove index 0 until the submenu is empty) first.
    while submenu.remove_at(0).is_some() {}

    let recents: Vec<PathBuf> = crate::recents::list()
        .into_iter()
        .filter(|path| Some(path.as_path()) != current)
        .take(RECENT_MENU_LIMIT)
        .collect();

    if recents.is_empty() {
        // A disabled placeholder so an empty submenu still reads clearly.
        log_menu(
            "open-recent submenu",
            submenu.append(&MenuItem::new("No recent journals", false, None)),
        );
        map.borrow_mut().clear();
        return;
    }

    let mut new_map = Vec::with_capacity(recents.len());
    for (index, path) in recents.iter().enumerate() {
        let item = MenuItem::with_id(
            format!("recent-{index}"),
            crate::recents::display_label(path),
            true,
            None,
        );
        new_map.push((item.id().clone(), path.clone()));
        log_menu("open-recent submenu", submenu.append(&item));
    }
    *map.borrow_mut() = new_map;
}

/// Should the WebView be allowed to navigate to `candidate`?
///
/// Only inside our own in-process origin. The window has no address bar, so an
/// unconstrained WebView would happily render an attacker's page as if it were
/// the app — and that page would then be same-origin with nothing, but would
/// still be the user's whole visible UI. `base` is `http://host:port/`, and the
/// required trailing slash is what stops `http://127.0.0.1:5000@evil.example/`
/// from passing as a prefix match.
fn navigation_allowed(base: &str, candidate: &str) -> bool {
    candidate == base.trim_end_matches('/') || candidate.starts_with(base)
}

/// Pick the one journal to open from the URLs macOS delivered with a document
/// launch ([`Event::Opened`]).
///
/// Split out as a pure function so it is testable without standing up an event
/// loop. `Url::to_file_path` is doing the real work: it rejects non-`file://`
/// URLs (a deeplink scheme we have not implemented — skipped, not fatal, so a
/// real document later in the same batch still wins) and it undoes the
/// percent-encoding, which matters because Finder escapes spaces and a journal
/// under "My Books" would otherwise resolve to a path that does not exist.
///
/// Only the FIRST resolvable path is returned: Ledgeline is single-journal,
/// single-window, so selecting several files in Finder and hitting Open cannot
/// be honoured in full. The caller logs the ones it drops.
fn first_journal_path(urls: &[Url]) -> Option<PathBuf> {
    urls.iter().find_map(|url| url.to_file_path().ok())
}

/// Build the wry webview for `window`, pointed at `url`.
fn build_webview(window: &tao::window::Window, url: &str) -> Result<wry::WebView, AppError> {
    let base = url.to_string();
    let builder = WebViewBuilder::new()
        .with_url(url)
        .with_navigation_handler(move |candidate| {
            let allowed = navigation_allowed(&base, &candidate);
            if !allowed {
                eprintln!("ledgeline: blocked navigation to {candidate}");
            }
            allowed
        });

    #[cfg(not(target_os = "linux"))]
    let webview = builder
        .build(window)
        .map_err(|error| AppError::Gui(format!("creating webview: {error}")))?;
    #[cfg(target_os = "linux")]
    let webview = {
        use tao::platform::unix::WindowExtUnix;
        // `build_gtk` lives on this extension trait; it must be in scope (Linux).
        use wry::WebViewBuilderExtUnix;
        // The WebView goes in the window's DEFAULT VBOX, not in the window.
        // A GtkApplicationWindow is a GtkBin — exactly one child — and tao has
        // already put a GtkBox there, which is also where muda's menu bar goes
        // (`init_for_gtk_window`). Handing the WebView to the window instead
        // makes GTK refuse it and silently drop it, leaving a menu bar with no
        // page under it plus a `Gtk-WARNING: … can only contain one widget at a
        // time; it already contains a widget of type GtkBox`. Ordering is no
        // escape: the vbox exists from window creation, so whichever widget is
        // offered to the window loses.
        //
        // Packing works out because the two crates agree on the box: wry
        // recognises a `gtk::Box` and uses `pack_start(webview, true, true, 0)`
        // so the page expands, while muda packs the bar `(false, false)` and
        // reorders it to position 0. The `None` arm is unreachable unless the
        // window was built `with_default_vbox(false)`, and keeps the match
        // total without a panic.
        match window.default_vbox() {
            Some(vbox) => builder.build_gtk(vbox),
            None => builder.build_gtk(window.gtk_window()),
        }
        .map_err(|error| AppError::Gui(format!("creating webview: {error}")))?
    };
    Ok(webview)
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
    let description = format!(
        "Could not open {}:\n{error}\n\nThe current journal stays loaded.",
        path.display()
    );
    std::thread::spawn(move || {
        rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Error)
            .set_title("Failed to open journal")
            .set_description(description)
            .set_buttons(rfd::MessageButtons::Ok)
            .show();
    });
}

/// Build the window + webview on the main thread and run the (diverging) event
/// loop until the window closes.
fn run_event_loop(ctx: GuiContext) -> Result<(), AppError> {
    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();

    // Route muda menu activations into the loop as user events.
    let menu_proxy = event_loop.create_proxy();
    MenuEvent::set_event_handler(Some(move |event| {
        let _ = menu_proxy.send_event(UserEvent::Menu(event));
    }));

    let menu = build_menu();
    #[cfg(target_os = "macos")]
    menu.menu_bar.init_for_nsapp();

    // Open at a comfortable desktop size (mbr leaves this to the platform
    // default, which is too small for a data-dense GUI); clamp the floor so the
    // report layout never collapses.
    let window = WindowBuilder::new()
        .with_title("Ledgeline")
        .with_inner_size(LogicalSize::new(1280.0, 832.0))
        .with_min_inner_size(LogicalSize::new(800.0, 600.0))
        .build(&event_loop)
        .map_err(|error| AppError::Gui(format!("creating window: {error}")))?;

    #[cfg(target_os = "windows")]
    {
        use tao::platform::windows::WindowExtWindows;
        // SAFETY: called once, on the main thread, with this window's live HWND.
        unsafe {
            let _ = menu.menu_bar.init_for_hwnd(window.hwnd() as isize);
        }
    }
    #[cfg(target_os = "linux")]
    {
        use tao::platform::unix::WindowExtUnix;
        let _ = menu
            .menu_bar
            .init_for_gtk_window(window.gtk_window(), window.default_vbox());
    }

    let webview = build_webview(&window, &ctx.url)?;

    let open_id = menu.open.id().clone();
    let reload_id = menu.reload.id().clone();
    let back_id = menu.back.id().clone();
    let forward_id = menu.forward.id().clone();
    let recent_submenu = menu.recent.clone();

    let picker_proxy = event_loop.create_proxy();

    // Session state owned by the loop. `url` is stable for the whole session
    // (the ephemeral port never changes), so reload/navigation always target it.
    // The live-reload watcher is pure RAII — held in a cell so File→Open can swap
    // it (dropping the old one) without an assignment the borrow checker reads as
    // dead across the `FnMut` boundary.
    let url = ctx.url;
    let state = ctx.state;
    let watcher: RefCell<Option<RecommendedWatcher>> = RefCell::new(ctx.watcher);
    // Canonical path of the open journal (excluded from the recents submenu) and
    // the id→path map for its items, both refreshed on every open.
    let current: RefCell<PathBuf> = RefCell::new(ctx.current);
    let recent_map: RefCell<Vec<(MenuId, PathBuf)>> = RefCell::new(Vec::new());
    {
        let current_ref = current.borrow();
        rebuild_recent(&recent_submenu, &recent_map, Some(current_ref.as_path()));
    }

    event_loop.run(move |event, _target, control_flow| {
        *control_flow = ControlFlow::Wait;

        // Unified open path shared by File→Open and Open Recent: rebind the editor
        // to `raw` (opening a fresh editor, republishing its snapshot, swapping it
        // into the editor mutex) so edits target the new file, then re-point the
        // watcher, record it as most-recent, refresh the submenu, and reload the
        // page. Canonicalize to match the watcher's path, as at startup.
        let open_journal = |raw: &Path| {
            let editor_path = raw.canonicalize().unwrap_or_else(|_| raw.to_path_buf());
            match state.rebind_editor(&editor_path) {
                Ok(()) => {
                    watcher.replace(crate::spawn_watcher(&editor_path, state.clone()).ok());
                    crate::recents::record(&editor_path);
                    current.replace(editor_path.clone());
                    rebuild_recent(&recent_submenu, &recent_map, Some(editor_path.as_path()));
                    let _ = webview.load_url(&url);
                    eprintln!("ledgeline: opened {}", editor_path.display());
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

        match event {
            Event::UserEvent(UserEvent::Menu(menu_event)) => {
                if menu_event.id == open_id {
                    spawn_file_picker(picker_proxy.clone());
                } else if menu_event.id == reload_id {
                    let _ = webview.load_url(&url);
                } else if menu_event.id == back_id {
                    let _ = webview.evaluate_script("history.back()");
                } else if menu_event.id == forward_id {
                    let _ = webview.evaluate_script("history.forward()");
                } else {
                    // Maybe an Open Recent item; the borrow is released before we
                    // open (which re-borrows the map to rebuild the submenu).
                    let picked = recent_map
                        .borrow()
                        .iter()
                        .find_map(|(id, path)| (*id == menu_event.id).then(|| path.clone()));
                    if let Some(path) = picked {
                        open_journal(&path);
                    }
                }
                // PredefinedMenuItem events (quit, copy, …) are handled natively.
            }
            Event::UserEvent(UserEvent::JournalPicked(path)) => {
                open_journal(&path);
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
                if let Some(path) = first_journal_path(&urls) {
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
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                *control_flow = ControlFlow::Exit;
            }
            _ => {}
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{first_journal_path, navigation_allowed};
    use std::path::PathBuf;
    use url::Url;

    const BASE: &str = "http://127.0.0.1:5000/";

    fn urls(raw: &[&str]) -> Vec<Url> {
        raw.iter()
            .map(|raw| Url::parse(raw).expect("test URL parses"))
            .collect()
    }

    #[test]
    fn navigation_is_confined_to_our_own_origin() {
        assert!(navigation_allowed(BASE, "http://127.0.0.1:5000/"));
        assert!(navigation_allowed(BASE, "http://127.0.0.1:5000"));
        assert!(navigation_allowed(
            BASE,
            "http://127.0.0.1:5000/reports?tab=1"
        ));

        assert!(!navigation_allowed(BASE, "https://evil.example/"));
        assert!(!navigation_allowed(BASE, "file:///etc/passwd"));
        // A different local port is a different app, not ours.
        assert!(!navigation_allowed(BASE, "http://127.0.0.1:5001/"));
        // The trailing slash is what makes the prefix test safe: userinfo and
        // longer-port tricks must not read as our origin.
        assert!(!navigation_allowed(
            BASE,
            "http://127.0.0.1:5000@evil.example/"
        ));
        assert!(!navigation_allowed(BASE, "http://127.0.0.1:50000/"));
        assert!(!navigation_allowed(
            BASE,
            "http://127.0.0.1:5000.evil.example/"
        ));
    }

    #[test]
    fn document_launch_resolves_the_first_usable_file_url() {
        // The ordinary Finder double-click.
        assert_eq!(
            first_journal_path(&urls(&["file:///tmp/main.journal"])),
            Some(PathBuf::from("/tmp/main.journal"))
        );

        // Finder percent-encodes spaces; `to_file_path` is what decodes them back
        // into the path that actually exists on disk.
        assert_eq!(
            first_journal_path(&urls(&["file:///Users/me/My%20Books/main.journal"])),
            Some(PathBuf::from("/Users/me/My Books/main.journal"))
        );

        // A deeplink we do not implement is skipped rather than opened.
        assert_eq!(
            first_journal_path(&urls(&["https://example.com/main.journal"])),
            None
        );

        // No URLs at all (nothing to open) must not panic.
        assert_eq!(first_journal_path(&[]), None);

        // A non-file URL must not shadow a real document behind it in the batch.
        assert_eq!(
            first_journal_path(&urls(&[
                "ledgeline://open",
                "file:///tmp/second.journal",
                "file:///tmp/third.journal",
            ])),
            Some(PathBuf::from("/tmp/second.journal"))
        );
    }
}
