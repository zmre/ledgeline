//! The desktop window (`gui`): the shell's boot pieces
//! ([`ledgeline_shell::gui`]) strung together around the invoicer's state,
//! in Ledgeline's order (see `crates/ledgeline-server/src/gui.rs`):
//!
//! 1. Open the books (the positional journal, then recents) and reconcile.
//! 2. [`shell::spawn_server`]: the server on `127.0.0.1:0`, the router
//!    built for the port actually bound.
//! 3. On the main thread: menus, window, webview at `/app/`, with the token
//!    and the shared theme and scale handed over by initialization script.
//! 4. The event loop: File → Open / Open Recent swap the books (MBR server
//!    and watcher included) without restarting the server; PDF exports,
//!    Share… and Copy file run on it ([`super::desktop`], PLAN 3c).

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ledgeline_shell::AppDirs;
use ledgeline_shell::gui::{
    self as shell, AppMenu, MenuSpec, RecentMenu, Shortcut, appearance_init_script, perf,
};
use ledgeline_shell::recents;
use ledgeline_shell::security::token_init_script;
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

use super::books::OpenBooks;
use super::desktop::{self, DesktopHost, LoopNative, NativeEvent};
use super::native::Native;
use super::serve::plan_security;
use super::{APP, AppArgs, AppState, SPA_BASE, prefs, router, watch};

/// How many recent journals File → Open Recent shows.
const RECENT_MENU_LIMIT: usize = 5;

/// The published docs, the one outside site the window hands to the browser.
const DOCS_HOST: &str = "zmre.github.io";
const DOCS_PATH: &str = "/ledgeline/";

/// Events from other threads (the menu handler, the file picker).
enum UserEvent {
    Menu(MenuEvent),
    JournalPicked(PathBuf),
    /// A PDF, Share… or Copy file request from the API, or an export's
    /// progress ([`desktop`]).
    Native(NativeEvent),
}

/// GUI entry point.
pub(crate) fn run(args: &AppArgs, app_dirs: AppDirs) -> Result<(), String> {
    perf::start();
    let (process_token, security_plan) = plan_security(args)?;
    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("building the async runtime: {e}"))?;

    let books = match &args.journal {
        Some(journal) => match open_books(&runtime, journal, &app_dirs) {
            Ok(books) => Some(books),
            Err(error) => {
                show_open_error(journal, &error);
                None
            }
        },
        None => None,
    };
    perf::stamp("journal_loaded");
    let native: Arc<dyn Native> = Arc::new(LoopNative::new(
        event_loop.create_proxy(),
        UserEvent::Native,
        desktop::capabilities(true),
    ));
    let state = AppState::with_native(books.clone(), app_dirs, Some(native));

    let host = args.host.clone();
    let port = args.port.unwrap_or(0);
    let server_state = state.clone();
    let token = process_token.token.clone();
    let port_rx = shell::spawn_server(&APP, &runtime, &host, port, move |bound| {
        security_plan
            .build(bound)
            .map(|security| router(server_state, security, token, bound))
            .map_err(|error| error.to_string())
    });
    let bound = runtime
        .block_on(port_rx)
        .ok()
        .flatten()
        .ok_or("the in-process server did not report a bound port")?;
    let url = format!("http://{host}:{bound}/");
    let watcher = books.as_ref().and_then(|books| {
        eprintln!(
            "{}: serving {} at {url}",
            APP.log_prefix,
            books.journal().display()
        );
        start_watcher(books)
    });
    let current = books.map(|b| b.journal().to_path_buf()).unwrap_or_default();
    run_event_loop(
        event_loop,
        Context {
            url,
            token: process_token.token,
            state,
            runtime,
            watcher,
            current,
        },
    )
}

/// Open the books at `journal` (inside `runtime`: MBR spawns on it),
/// reconcile, and record them as the most recent.
fn open_books(
    runtime: &tokio::runtime::Runtime,
    journal: &Path,
    app_dirs: &AppDirs,
) -> Result<Arc<OpenBooks>, String> {
    let _entered = runtime.enter();
    let books = OpenBooks::open(journal, true)?;
    books.install_starters();
    books.reconcile_all();
    recents::record(app_dirs, books.journal());
    Ok(Arc::new(books))
}

fn start_watcher(books: &Arc<OpenBooks>) -> Option<RecommendedWatcher> {
    match watch::spawn(Arc::clone(books)) {
        Ok(watcher) => Some(watcher),
        Err(error) => {
            eprintln!("{}: live reconcile disabled: {error}", APP.log_prefix);
            None
        }
    }
}

/// Everything the event loop owns.
struct Context {
    /// `http://127.0.0.1:<port>/`, stable for the session.
    url: String,
    token: ledgeline_shell::security::AccessToken,
    state: AppState,
    /// Kept alive (and used for File → Open) for the whole session.
    runtime: tokio::runtime::Runtime,
    watcher: Option<RecommendedWatcher>,
    /// The open journal (excluded from Open Recent); empty with none.
    current: PathBuf,
}

/// A page the window may hand to the system browser: the published docs.
fn external_url(candidate: &str) -> Option<String> {
    let url = shell::plain_https(candidate)?;
    (url.host_str() == Some(DOCS_HOST) && url.path().starts_with(DOCS_PATH))
        .then(|| url.to_string())
}

fn open_if_external(candidate: &str) -> bool {
    external_url(candidate)
        .map(|url| shell::open_in_browser(&APP, &url))
        .is_some()
}

/// The rfd picker, on its own thread (it blocks).
fn spawn_file_picker(proxy: EventLoopProxy<UserEvent>) {
    std::thread::spawn(move || {
        if let Some(path) = rfd::FileDialog::new()
            .set_title("Open the books' main journal")
            .add_filter("hledger journal", &["journal", "hledger", "ledger", "j"])
            .add_filter("All files", &["*"])
            .pick_file()
        {
            let _ = proxy.send_event(UserEvent::JournalPicked(path));
        }
    });
}

fn show_open_error(path: &Path, error: &str) {
    shell::show_error(
        "Failed to open the books",
        format!("Could not open {}:\n{error}", path.display()),
    );
}

fn build_menu() -> AppMenu {
    let none: [&dyn IsMenuItem; 0] = [];
    AppMenu::build(
        &APP,
        &MenuSpec {
            version: env!("CARGO_PKG_VERSION"),
            new_label: "&New Invoice…",
            open_label: "&Open Books…",
            app_menu_items: &none,
            after_recent: &none,
        },
    )
}

/// Build the window and webview, then run the (diverging) event loop.
fn run_event_loop(
    event_loop: tao::event_loop::EventLoop<UserEvent>,
    ctx: Context,
) -> Result<(), String> {
    let menu_proxy = event_loop.create_proxy();
    MenuEvent::set_event_handler(Some(move |event| {
        let _ = menu_proxy.send_event(UserEvent::Menu(event));
    }));
    let menu = build_menu();
    let window =
        shell::build_window(&APP, &event_loop, &menu.menu_bar).map_err(|e| e.to_string())?;
    perf::stamp("window_built");

    let appearance = prefs::boot_appearance(&ctx.state);
    let origin = shell::webview_origin(&ctx.url).map_err(|e| e.to_string())?;
    let home = format!("{}{}/", ctx.url, SPA_BASE.trim_start_matches('/'));
    let webview = shell::build_webview(
        &APP,
        &window,
        &ctx.url,
        &home,
        vec![
            token_init_script(&APP, &origin, &ctx.token),
            appearance_init_script(&origin, &appearance),
        ],
        open_if_external,
    )
    .map_err(|e| e.to_string())?;
    perf::stamp("webview_built");

    let recent = RecentMenu::new(menu.recent.clone(), RECENT_MENU_LIMIT, "No recent books");

    #[cfg(target_os = "linux")]
    let menu_bar = menu.menu_bar.clone();
    #[cfg(target_os = "linux")]
    let chrome_visible = std::cell::Cell::new(false);
    #[cfg(target_os = "linux")]
    let modifiers = std::cell::Cell::new(ModifiersState::empty());
    #[cfg(target_os = "linux")]
    {
        shell::disable_gtk_menu_bar_accel();
        shell::set_window_chrome_visible(&APP, &menu_bar, &window, false);
    }

    let picker_proxy = event_loop.create_proxy();
    let mut host = DesktopHost::new(desktop::export_sink(
        event_loop.create_proxy(),
        UserEvent::Native,
    ));
    let Context {
        state,
        runtime,
        watcher,
        current,
        ..
    } = ctx;
    let watcher: RefCell<Option<RecommendedWatcher>> = RefCell::new(watcher);
    let current: RefCell<PathBuf> = RefCell::new(current);
    recent.rebuild(&APP, state.app_dirs(), Some(current.borrow().as_path()));

    event_loop.run(move |event, target, control_flow| {
        *control_flow = ControlFlow::Wait;

        // File → Open and Open Recent: open the new books, swap them in
        // (the old MBR server stops once its last request ends), re-point
        // the watcher, and reload the page.
        let open = |raw: &Path| match open_books(&runtime, raw, state.app_dirs()) {
            Ok(books) => {
                // The old watcher first: it holds the old books.
                watcher.replace(None);
                state.replace_books(Arc::clone(&books));
                watcher.replace(start_watcher(&books));
                current.replace(books.journal().to_path_buf());
                recent.rebuild(&APP, state.app_dirs(), Some(books.journal()));
                eprintln!("{}: opened {}", APP.log_prefix, books.journal().display());
                let _ = webview.load_url(&home);
            }
            Err(error) => {
                eprintln!(
                    "{}: could not open {}: {error}",
                    APP.log_prefix,
                    raw.display()
                );
                show_open_error(raw, &error);
            }
        };

        let perform = |shortcut: Shortcut| -> bool {
            match shortcut {
                // The invoice editor's page (Phase 4a); the SPA routes it.
                Shortcut::New => {
                    let _ = webview.load_url(&format!("{home}new"));
                }
                Shortcut::Open => spawn_file_picker(picker_proxy.clone()),
                Shortcut::Reload => {
                    let _ = webview.load_url(&home);
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
                if let Some(shortcut) = menu.shortcut_for(&menu_event.id) {
                    if perform(shortcut) {
                        #[cfg(target_os = "linux")]
                        shell::stop_web_process(&webview);
                        *control_flow = ControlFlow::Exit;
                    }
                } else if let Some(path) = recent.picked(&menu_event.id) {
                    open(&path);
                }
            }
            Event::UserEvent(UserEvent::JournalPicked(path)) => open(&path),
            Event::UserEvent(UserEvent::Native(event)) => host.handle(target, event, Some(&window)),
            // A document launch (Finder, `open -a`): macOS sends it as an
            // event, never in argv.
            Event::Opened { urls } => {
                if let Some(path) = shell::first_file_path(&urls) {
                    let already =
                        path.canonicalize().unwrap_or_else(|_| path.clone()) == *current.borrow();
                    if !already {
                        open(&path);
                    }
                }
            }
            #[cfg(target_os = "linux")]
            Event::WindowEvent {
                event: WindowEvent::ModifiersChanged(state),
                ..
            } => modifiers.set(state),
            // Linux: act on release, match the logical key (see Ledgeline's
            // gui.rs for why both were measured, not guessed).
            #[cfg(target_os = "linux")]
            Event::WindowEvent {
                event: WindowEvent::KeyboardInput { event: key, .. },
                ..
            } if key.state == ElementState::Released => {
                if key.logical_key == Key::F10 {
                    let visible = !chrome_visible.get();
                    shell::set_window_chrome_visible(&APP, &menu_bar, &window, visible);
                    chrome_visible.set(visible);
                } else if !chrome_visible.get()
                    && let Some(shortcut) =
                        shell::linux_shortcut_for(&key.logical_key, modifiers.get())
                    && perform(shortcut)
                {
                    shell::stop_web_process(&webview);
                    *control_flow = ControlFlow::Exit;
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
    fn only_the_docs_site_goes_to_the_system_browser() {
        assert!(external_url("https://zmre.github.io/ledgeline/invoicer/").is_some());
        for refused in [
            "http://zmre.github.io/ledgeline/",
            "https://zmre.github.io/other/",
            "https://zmre.github.io.evil.example/ledgeline/",
            "https://user@zmre.github.io/ledgeline/",
            "javascript:alert(1)",
        ] {
            assert_eq!(external_url(refused), None, "{refused}");
        }
    }
}
