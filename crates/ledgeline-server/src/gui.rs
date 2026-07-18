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

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use ledgeline_server::{AppState, router_with_state};
use muda::{
    AboutMetadata, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu,
    accelerator::{Accelerator, Code, Modifiers},
};
use notify::RecommendedWatcher;
use tao::{
    dpi::LogicalSize,
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy},
    window::WindowBuilder,
};
use wry::WebViewBuilder;

use crate::{AppError, Cli};

/// Custom events routed through the tao event loop from other threads (the
/// global muda menu handler and the rfd file-picker thread).
enum UserEvent {
    Menu(MenuEvent),
    JournalPicked(PathBuf),
}

/// GUI entry point: stand up the in-process server, then run the window.
pub(crate) fn run(cli: &Cli) -> Result<(), AppError> {
    let journal_path = crate::resolve_journal(cli);
    let journal = crate::parse_at(&journal_path)?;
    let state = AppState::from_journal(&journal);

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
                if let Err(error) = axum::serve(listener, router_with_state(server_state)).await {
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
    })
    // `runtime` and `_server` stay live on this frame while the event loop runs.
}

/// Everything the event loop owns after startup.
struct GuiContext {
    url: String,
    state: AppState,
    watcher: Option<RecommendedWatcher>,
}

/// Menu bar plus the handles for the custom items we match events against.
struct AppMenu {
    menu_bar: Menu,
    open: MenuItem,
    reload: MenuItem,
    back: MenuItem,
    forward: MenuItem,
}

fn about_metadata() -> AboutMetadata {
    AboutMetadata {
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
        Some(Accelerator::new(Some(Modifiers::SUPER), Code::KeyO)),
    );
    log_menu(
        "file menu",
        file_menu.append_items(&[
            &open,
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
        Some(Accelerator::new(Some(Modifiers::SUPER), Code::KeyR)),
    );
    let back = MenuItem::with_id(
        "back",
        "&Back",
        true,
        Some(Accelerator::new(Some(Modifiers::SUPER), Code::BracketLeft)),
    );
    let forward = MenuItem::with_id(
        "forward",
        "&Forward",
        true,
        Some(Accelerator::new(Some(Modifiers::SUPER), Code::BracketRight)),
    );
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
        reload,
        back,
        forward,
    }
}

/// Build the wry webview for `window`, pointed at `url`.
fn build_webview(window: &tao::window::Window, url: &str) -> Result<wry::WebView, AppError> {
    let builder = WebViewBuilder::new().with_url(url);

    #[cfg(not(target_os = "linux"))]
    let webview = builder
        .build(window)
        .map_err(|error| AppError::Gui(format!("creating webview: {error}")))?;
    #[cfg(target_os = "linux")]
    let webview = {
        use tao::platform::unix::WindowExtUnix;
        builder
            .build_gtk(window.gtk_window())
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

    let picker_proxy = event_loop.create_proxy();

    // Session state owned by the loop. `url` is stable for the whole session
    // (the ephemeral port never changes), so reload/navigation always target it.
    // The live-reload watcher is pure RAII — held in a cell so File→Open can swap
    // it (dropping the old one) without an assignment the borrow checker reads as
    // dead across the `FnMut` boundary.
    let url = ctx.url;
    let state = ctx.state;
    let watcher: RefCell<Option<RecommendedWatcher>> = RefCell::new(ctx.watcher);

    event_loop.run(move |event, _target, control_flow| {
        *control_flow = ControlFlow::Wait;

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
                }
                // PredefinedMenuItem events (quit, copy, …) are handled natively.
            }
            Event::UserEvent(UserEvent::JournalPicked(path)) => match crate::parse_at(&path) {
                Ok(journal) => {
                    // Hot-swap in place: no server restart (same ephemeral port).
                    state.replace_journal(&journal);
                    // Re-point live-reload at the new file (drops the old watcher).
                    watcher.replace(crate::spawn_watcher(&path, state.clone()).ok());
                    let _ = webview.load_url(&url);
                    eprintln!("ledgeline: opened {}", path.display());
                }
                Err(error) => {
                    eprintln!("ledgeline: could not open {}: {error}", path.display());
                    show_open_error(&path, &error);
                }
            },
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
