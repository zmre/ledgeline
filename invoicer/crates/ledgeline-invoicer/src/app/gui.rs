//! The desktop window (`gui`): the shell's boot pieces
//! ([`ledgeline_shell::gui`]) strung together around the invoicer's state,
//! in Ledgeline's order (see `crates/ledgeline-server/src/gui.rs`):
//!
//! 1. Open the books (the positional journal, then recents) and reconcile.
//! 2. [`shell::spawn_server`]: the server on `127.0.0.1:0`, the router
//!    built for the port actually bound.
//! 3. On the main thread: menus (with "Settings…", ⌘, / Ctrl+, opening
//!    `/app/settings`), window, webview at `/app/` (at `/app/settings` when
//!    a problem in `ledgeline.toml` kept the books shut), with the token and
//!    the shared theme and scale handed over by initialization script.
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
use muda::accelerator::{Accelerator, Code};
use muda::{IsMenuItem, MenuEvent, MenuItem};
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

use super::books::{OpenBooks, OpenFailure};
use super::desktop::{self, DesktopHost, LoopNative, NativeEvent};
use super::native::Native;
use super::serve::plan_security;
use super::{APP, AppArgs, AppState, SPA_BASE, Unopened, prefs, router, watch};

/// How many recent journals File → Open Recent shows.
const RECENT_MENU_LIMIT: usize = 5;

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

    let mut unopened = None;
    let books = match &args.journal {
        Some(journal) => match open_books(&runtime, journal, &app_dirs) {
            Ok(books) => Some(books),
            // A problem in `ledgeline.toml`: the window opens on the
            // Settings page, which says what it is.
            Err(OpenFailure {
                message,
                in_settings: Some(problem),
            }) => {
                eprintln!("{}: the books didn't open: {message}", APP.log_prefix);
                unopened = Some(Unopened {
                    journal: journal.clone(),
                    problem,
                });
                None
            }
            Err(error) => {
                show_open_error(journal, &error.message);
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
    if let Some(unopened) = unopened {
        state.set_unopened(unopened);
    }

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
) -> Result<Arc<OpenBooks>, OpenFailure> {
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

/// A link the window may hand to the system's default app, re-serialized:
/// any plain https page (the browser) or addressed `mailto:` (the mail
/// app). Notes and invoices are the user's own content, so their web and
/// email links open; everything else (http, ports, credentials, `file:`,
/// scripts) stays blocked.
fn external_url(candidate: &str) -> Option<String> {
    shell::plain_https(candidate)
        .or_else(|| shell::plain_mailto(candidate))
        .map(String::from)
}

/// The window's `open_external` ([`shell::build_webview`]): hand an
/// [`external_url`] to the default app and answer whether it did. The frame
/// or window the link was clicked in never leaves our origin. From the note
/// and invoice frames, an https link reaches this as the frame's own
/// navigation, which the SPA's CSP lets through (`APP.spa_frame_src`;
/// WebKit would refuse it before the navigation handler otherwise). A
/// mailto link arrives as a navigation of the main window instead: WebKit
/// refuses custom protocols from a sandboxed frame, so the SPA cancels the
/// click there and navigates the main window (`invoicer/web/src/lib/frameLinks.ts`).
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

/// The "Settings…" item's id.
const SETTINGS_ID: &str = "settings";

/// The menus, and the "Settings…" item (⌘, / Ctrl+,): in the app menu on
/// macOS, after Open Recent in File elsewhere.
fn build_menu() -> (AppMenu, MenuItem) {
    let none: [&dyn IsMenuItem; 0] = [];
    let settings = MenuItem::with_id(
        SETTINGS_ID,
        "&Settings…",
        true,
        Some(Accelerator::new(shell::COMMAND_MODIFIER, Code::Comma)),
    );
    let items: [&dyn IsMenuItem; 1] = [&settings];
    let (app_menu_items, after_recent): (&[&dyn IsMenuItem], &[&dyn IsMenuItem]) =
        if cfg!(target_os = "macos") {
            (&items, &none)
        } else {
            (&none, &items)
        };
    let menu = AppMenu::build(
        &APP,
        &MenuSpec {
            version: env!("CARGO_PKG_VERSION"),
            new_label: "&New Invoice…",
            open_label: "&Open Books…",
            app_menu_items,
            after_recent,
        },
    );
    (menu, settings)
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
    let (menu, settings_item) = build_menu();
    let window =
        shell::build_window(&APP, &event_loop, &menu.menu_bar).map_err(|e| e.to_string())?;
    perf::stamp("window_built");

    let appearance = prefs::boot_appearance(&ctx.state);
    let origin = shell::webview_origin(&ctx.url).map_err(|e| e.to_string())?;
    let home = format!("{}{}/", ctx.url, SPA_BASE.trim_start_matches('/'));
    // The SPA's Settings page (`/app/settings`).
    let settings_page = format!("{home}settings");
    // Books a problem in `ledgeline.toml` kept shut open on that page.
    let first_page = if ctx.state.unopened().is_some() {
        &settings_page
    } else {
        &home
    };
    let webview = shell::build_webview(
        &APP,
        &window,
        &ctx.url,
        first_page,
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
            Err(OpenFailure {
                message,
                in_settings: Some(problem),
            }) => {
                eprintln!(
                    "{}: could not open {}: {message}",
                    APP.log_prefix,
                    raw.display()
                );
                // The Settings page says what is wrong in `ledgeline.toml`;
                // Reload tries again once it is fixed.
                watcher.replace(None);
                state.set_unopened(Unopened {
                    journal: raw.to_path_buf(),
                    problem,
                });
                let _ = webview.load_url(&settings_page);
            }
            Err(error) => {
                eprintln!(
                    "{}: could not open {}: {error}",
                    APP.log_prefix,
                    raw.display()
                );
                show_open_error(raw, &error.message);
            }
        };

        let perform = |shortcut: Shortcut| -> bool {
            match shortcut {
                // The invoice editor's page (Phase 4a); the SPA routes it.
                Shortcut::New => {
                    let _ = webview.load_url(&format!("{home}new"));
                }
                Shortcut::Open => spawn_file_picker(picker_proxy.clone()),
                // Books a problem in `ledgeline.toml` kept shut: try again
                // (it may be fixed now).
                Shortcut::Reload => match state.unopened().filter(|_| state.books().is_none()) {
                    Some(unopened) => open(&unopened.journal),
                    None => {
                        let _ = webview.load_url(&home);
                    }
                },
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
                if menu_event.id == *settings_item.id() {
                    let _ = webview.load_url(&settings_page);
                } else if let Some(shortcut) = menu.shortcut_for(&menu_event.id) {
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
                let ctrl_only = modifiers.get().control_key()
                    && !modifiers.get().alt_key()
                    && !modifiers.get().super_key();
                if key.logical_key == Key::F10 {
                    let visible = !chrome_visible.get();
                    shell::set_window_chrome_visible(&APP, &menu_bar, &window, visible);
                    chrome_visible.set(visible);
                } else if !chrome_visible.get()
                    && ctrl_only
                    && key.logical_key == Key::Character(",")
                {
                    // The menu's accelerator is dead with the bar hidden.
                    let _ = webview.load_url(&settings_page);
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
    fn plain_https_and_mailto_links_go_to_the_default_apps() {
        for (link, opened) in [
            (
                "https://zmre.github.io/ledgeline/invoicer/",
                "https://zmre.github.io/ledgeline/invoicer/",
            ),
            ("https://example.com", "https://example.com/"),
            (
                "mailto:ap@example.com?subject=Invoice 108&body=Hi",
                "mailto:ap@example.com?subject=Invoice%20108&body=Hi",
            ),
        ] {
            assert_eq!(external_url(link).as_deref(), Some(opened), "{link}");
        }
    }

    #[test]
    fn other_links_stay_blocked() {
        for refused in [
            "http://example.com/",
            "https://example.com:8443/",
            "https://user@example.com/",
            "https://user:secret@example.com/",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "data:text/html,<p>hi</p>",
            "tel:+15551234567",
            "mailto:",
            "mailto:?subject=hi",
            "not a url",
        ] {
            assert_eq!(external_url(refused), None, "{refused}");
        }
    }
}
