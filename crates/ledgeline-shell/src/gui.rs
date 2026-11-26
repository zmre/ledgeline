//! The desktop shell's building blocks (the `gui` feature): a tao window and a
//! wry webview rendering an app's in-process SPA, with muda menus and rfd
//! pickers.
//!
//! Boot sequence (mirrors the proven order in zmre/mbr-markdown-browser); an
//! app's own `gui` module strings these together around its state:
//!  1. Build the app's shared state.
//!  2. [`spawn_server`]: axum on `127.0.0.1:0` (ephemeral) on a Tokio runtime,
//!     reporting the ACTUAL bound port back over a `oneshot`, with the
//!     security posture pinned to that port.
//!  3. On the MAIN thread (native UI requires it): [`AppMenu::build`], then
//!     [`build_window`], then [`build_webview`] pointed at
//!     `http://127.0.0.1:<port>/`, with the token and the boot appearance
//!     handed over by initialization script ([`token_init_script`],
//!     [`appearance_init_script`]).
//!  4. The app runs the (diverging) tao event loop, routing menu ids through
//!     [`AppMenu::shortcut_for`] and the Linux keyboard through
//!     [`linux_shortcut_for`] to one implementation per [`Shortcut`]. The Tokio
//!     runtime keeps serving on its worker threads meanwhile.
//!
//! The webview's navigation policy has two layers, because wry's navigation
//! handler sees every frame's navigations and cannot tell them apart:
//!  - every frame stays on our origin ([`navigation_allowed`]), so an app's
//!    sandboxed same-origin frames (the invoicer's MBR pages) still load;
//!  - the MAIN frame shows only the app's own SPA, under
//!    [`AppIdentity::spa_base`] ([`crate::security::in_spa`]). That is
//!    enforced when the main frame commits to a page (wry's page-load
//!    handler, which only the main frame fires): any other page is replaced
//!    by the last SPA page it showed. The token's init script applies the
//!    same rule, so such a page never holds the token, even for the moment
//!    before it is replaced.
//!
//! Because the SPA is served same-origin and an app hot-swaps its state in
//! place, opening another document needs NO server restart: the ephemeral
//! port is stable for the whole session.
//!
//! [`token_init_script`]: crate::security::token_init_script

use std::cell::{OnceCell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};

use ledgeline_core::ui_prefs::BootAppearance;
use muda::{
    IsMenuItem, Menu, MenuId, MenuItem, PredefinedMenuItem, Submenu,
    accelerator::{Accelerator, Code, Modifiers},
};
#[cfg(target_os = "linux")]
use tao::keyboard::{Key, ModifiersState};
use tao::{
    dpi::LogicalSize,
    event_loop::EventLoop,
    window::{Window, WindowBuilder},
};
use url::Url;
use wry::{NewWindowResponse, PageLoadEvent, WebViewBuilder};

use crate::security::in_spa;
use crate::{AppDirs, AppIdentity, recents};

#[cfg(target_os = "macos")]
pub mod file_assoc;

/// A window, menu or webview that could not be built. The message is the
/// whole story ("creating webview: …"), for the app to show or log.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct GuiError(String);

/// Bind `host:port` on `runtime` and serve the router `router_for` builds for
/// the port actually bound (`0` = ephemeral, the GUI's default). The returned
/// receiver yields that port, or `None` when nothing is being served.
///
/// The router is built only once the port is known, because the `Host` guard
/// has to name it ([`crate::security::SecurityPlan::build`]). A router that
/// cannot be built — or a bound port that cannot be read back, which would
/// leave the guard unpinnable — means refusing to serve rather than serving
/// unguarded; the failure is logged and the caller sees `None`.
pub fn spawn_server<F>(
    app: &'static AppIdentity,
    runtime: &tokio::runtime::Runtime,
    host: &str,
    port: u16,
    router_for: F,
) -> tokio::sync::oneshot::Receiver<Option<u16>>
where
    F: FnOnce(u16) -> Result<axum::Router, String> + Send + 'static,
{
    let (port_tx, port_rx) = tokio::sync::oneshot::channel::<Option<u16>>();
    let addr = format!("{host}:{port}");
    // Spawned, not awaited: the app's diverging event loop never returns, so
    // the runtime keeps serving for the whole session.
    runtime.spawn(async move {
        let prefix = app.log_prefix;
        match tokio::net::TcpListener::bind(&addr).await {
            Ok(listener) => {
                let bound = listener.local_addr().map(|a| a.port()).ok();
                let _ = port_tx.send(bound);
                let router = match bound.map(router_for) {
                    Some(Ok(router)) => router,
                    Some(Err(error)) => {
                        eprintln!("{prefix}: security setup failed: {error}");
                        return;
                    }
                    // The bound port is unknown, so the Host guard cannot be
                    // pinned; refuse to serve rather than serve unguarded. The
                    // main thread already sees `None`.
                    None => return,
                };
                if let Err(error) = axum::serve(listener, router).await {
                    eprintln!("{prefix}: server error: {error}");
                }
            }
            Err(error) => {
                eprintln!("{prefix}: bind error on {addr}: {error}");
                let _ = port_tx.send(None);
            }
        }
    });
    port_rx
}

// Referenced only by the macOS app menu's About item, so it does not exist on
// other platforms (where it would be dead code under `-D warnings`).
#[cfg(target_os = "macos")]
fn about_metadata(app: &AppIdentity, version: &str) -> muda::AboutMetadata {
    muda::AboutMetadata {
        name: Some(app.name.to_string()),
        version: Some(version.to_string()),
        ..Default::default()
    }
}

/// Log (rather than crash on) a menu-construction failure — a degraded menu is
/// cosmetic; aborting startup would be far worse.
pub fn log_menu(app: &AppIdentity, what: &str, result: Result<(), muda::Error>) {
    if let Err(error) = result {
        eprintln!("{}: failed to build {what}: {error}", app.log_prefix);
    }
}

/// The modifier that means "the app's command key": Cmd on macOS, Ctrl on Linux
/// and Windows.
///
/// [`Modifiers::META`] is Cmd on macOS but the **Super/Windows key** elsewhere,
/// which belongs to the desktop rather than to us — a tiling compositor such as
/// Hyprland binds nearly the whole Super range, so a `Super+O` accelerator does
/// not merely render as a wrong-looking "Meta+O" in the menu, it never fires at
/// all. Every accelerator that is Cmd-something on macOS goes through this.
#[cfg(target_os = "macos")]
pub const COMMAND_MODIFIER: Modifiers = Modifiers::META;
#[cfg(not(target_os = "macos"))]
pub const COMMAND_MODIFIER: Modifiers = Modifiers::CONTROL;

/// An action reachable from more than one route.
///
/// The menu route and (on Linux) the keyboard route both resolve to one of
/// these and both funnel into a single `perform_shortcut` in the app, so each
/// action has exactly one implementation and the two routes cannot drift
/// apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shortcut {
    /// File → New… (Ledgeline: the new-ledger wizard, plan 39).
    New,
    /// File → Open…
    Open,
    Reload,
    Back,
    Forward,
    /// View → Zoom In / Zoom Out / Actual Size. The SPA owns the scale (its
    /// steps, and saving it as a preference), so these only tell it which way.
    Zoom(Zoom),
    /// Linux only — see `quit` in [`AppMenu::build`] for why this is not
    /// `PredefinedMenuItem::quit`.
    ///
    /// The variant itself is `cfg`-gated, not just its construction sites:
    /// macOS and Windows quit through muda's native predefined item, which
    /// works there, so nothing on those platforms ever builds a `Quit` and an
    /// ungated variant is dead code under `-D warnings`.
    #[cfg(target_os = "linux")]
    Quit,
}

/// Which way a View-menu zoom goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zoom {
    In,
    Out,
    Reset,
}

/// The script that hands a zoom to the SPA: a `ledgeline:zoom` event on
/// `window`, which the shared theme code (`web/src/lib/theme/zoom.svelte.ts`)
/// listens for and turns into the same step the keyboard takes.
///
/// One press, one step. The menu's accelerators are the same keys the SPA's
/// keymap binds, and what keeps a single press from zooming twice is which of
/// the two sees it — per platform:
///
/// - **macOS**: WKWebView offers a Cmd key-equivalent to the PAGE first and only
///   walks the main menu if the page leaves it unhandled. The keymap
///   `preventDefault`s its zoom keys, so the menu item fires only when it is
///   clicked (or the page is not up to take the key).
/// - **Linux**: with the menu bar shown its `GtkAccelGroup` consumes the key
///   before the web view gets it; with the bar hidden (the default) the
///   accelerator is dead and only the page sees the key.
/// - **Windows**: WebView2 handles keys in its own browser process and wry does
///   not forward them to the host's accelerator table, so the menu is not
///   expected to see a key the page has focus for. (wry's default
///   `with_hotkeys_zoom(false)` also turns WebView2's own Ctrl +/- page zoom off,
///   which would otherwise stack a page zoom on top of the interface scale.)
///
/// None of that is testable from here, so the SPA also drops the same action
/// arriving from the other route within a few hundred milliseconds.
#[must_use]
pub const fn zoom_script(zoom: Zoom) -> &'static str {
    match zoom {
        Zoom::In => "window.dispatchEvent(new CustomEvent('ledgeline:zoom', {detail: 'in'}))",
        Zoom::Out => "window.dispatchEvent(new CustomEvent('ledgeline:zoom', {detail: 'out'}))",
        Zoom::Reset => "window.dispatchEvent(new CustomEvent('ledgeline:zoom', {detail: 'reset'}))",
    }
}

/// Kill the WebKit web process instead of letting it exit on its own.
///
/// The web process is a separate OS process. When the UI process goes away it
/// notices the IPC connection close and calls `exit()`, which runs `_dl_fini`
/// and the atexit handlers — and that unwind races the compositor/EGL threads
/// that are still live if the page has not finished loading. Observed aborting
/// there in two shapes, both heap corruption reported by glibc:
///
/// ```text
/// abort ← malloc_printerr ← _int_free ← __eglFini ← _dl_fini ← exit
/// abort ← malloc_printerr ← _int_free ← _dl_deallocate_tls ← __nptl_free_stacks
/// ```
///
/// `webkit_web_view_terminate_web_process()` SIGKILLs it instead, so those
/// handlers never run and the abort is impossible BY CONSTRUCTION rather than
/// by winning a race. Nothing is lost by being abrupt: we only call this once
/// the user has asked to quit, at which point the page is being thrown away.
///
/// Not a fix for the underlying WebKitGTK/glvnd bug, which also shows up in
/// other apps on this stack — just a refusal to walk into it.
#[cfg(target_os = "linux")]
pub fn stop_web_process(webview: &wry::WebView) {
    use webkit2gtk::WebViewExt;
    use wry::WebViewExtUnix;

    webview.webview().terminate_web_process();
}

/// Map a key press to an action, for the Linux keyboard route used while the
/// menu bar is hidden.
///
/// Pure, and takes the modifier state as a parameter rather than reading it
/// from a window, so the whole table is unit-testable without standing up an
/// event loop.
///
/// The modifier tests are exact rather than "contains": `Ctrl+Alt+O` is a
/// different chord from `Ctrl+O` and must not be mistaken for it. Shift is
/// deliberately not examined — it is already folded into `logical_key`, so a
/// shifted `o` simply arrives as `Key::Character("O")`, which is why the
/// character comparison is ASCII-case-insensitive.
#[cfg(target_os = "linux")]
#[must_use]
pub fn linux_shortcut_for(key: &Key<'_>, modifiers: ModifiersState) -> Option<Shortcut> {
    let ctrl = modifiers.control_key() && !modifiers.alt_key() && !modifiers.super_key();
    let alt = modifiers.alt_key() && !modifiers.control_key() && !modifiers.super_key();
    match key {
        // plan 39 wizard
        Key::Character(character) if ctrl && character.eq_ignore_ascii_case("n") => {
            Some(Shortcut::New)
        }
        Key::Character(character) if ctrl && character.eq_ignore_ascii_case("o") => {
            Some(Shortcut::Open)
        }
        Key::Character(character) if ctrl && character.eq_ignore_ascii_case("r") => {
            Some(Shortcut::Reload)
        }
        // Needed here as well as on the menu item: with the bar hidden the
        // menu's accelerator is dead, and the title bar's close button is
        // hidden too, so this is the only way out short of asking the
        // compositor to close the window.
        Key::Character(character) if ctrl && character.eq_ignore_ascii_case("q") => {
            Some(Shortcut::Quit)
        }
        Key::ArrowLeft if alt => Some(Shortcut::Back),
        Key::ArrowRight if alt => Some(Shortcut::Forward),
        _ => None,
    }
}

/// Stop GTK from swallowing `F10` for menu-bar traversal.
///
/// `gtk-menu-bar-accel` defaults to `F10`, and a *visible* menu bar consumes
/// that key to move keyboard focus into itself — so without this the toggle
/// works in one direction only: `F10` reveals the bar, and the next `F10` opens
/// the File menu instead of hiding it again.
///
/// `find_property` first because `set_property` PANICS on a name the object does
/// not have, and this property has been deprecated since GTK 3.10 — so it may
/// simply be gone in a future GTK. What is given up is GTK's keyboard route
/// *into* a visible bar; every item still has its own accelerator.
#[cfg(target_os = "linux")]
pub fn disable_gtk_menu_bar_accel() {
    use gtk::prelude::*;

    if let Some(settings) = gtk::Settings::default()
        && settings.find_property("gtk-menu-bar-accel").is_some()
    {
        settings.set_property("gtk-menu-bar-accel", None::<String>);
    }
}

/// Show or hide the desktop chrome: the in-window `GtkMenuBar` *and* the window
/// decorations (title bar with its close/minimise buttons).
///
/// Both are hidden by default on Linux and both come back together on `F10`.
/// They are toggled as ONE unit deliberately: the SPA already draws its own
/// header with the app name, so the title bar is duplicate furniture under a
/// tiling compositor — but a window with neither decorations nor a menu bar has
/// no close button and no File→Quit on a *floating* desktop, which would be a
/// trap. Tying them together guarantees there is always one key that brings
/// back a way out.
///
/// `gtk_widget_show_all` recurses the whole widget tree, so any later
/// `show_all()` on the window would undo a plain `hide()`. `set_no_show_all` is
/// what makes the hide stick; it has to be cleared again before showing, or the
/// bar would refuse to come back.
///
/// `gtk_menubar_for_gtk_window` takes `self` by value, but `Menu` is a `Clone`
/// handle around a shared inner, so the clone is a refcount bump rather than a
/// second menu.
#[cfg(target_os = "linux")]
pub fn set_window_chrome_visible(
    app: &AppIdentity,
    menu_bar: &Menu,
    window: &Window,
    visible: bool,
) {
    use gtk::prelude::*;
    use tao::platform::unix::WindowExtUnix;

    if let Some(bar) = menu_bar
        .clone()
        .gtk_menubar_for_gtk_window(window.gtk_window())
    {
        bar.set_no_show_all(!visible);
    }
    log_menu(
        app,
        "menu bar visibility",
        if visible {
            menu_bar.show_for_gtk_window(window.gtk_window())
        } else {
            menu_bar.hide_for_gtk_window(window.gtk_window())
        },
    );
    window.set_decorations(visible);
}

/// What an app puts in the standard menu bar beyond the common items.
pub struct MenuSpec<'a> {
    /// The app's version, for About (macOS).
    pub version: &'a str,
    /// File → New's label (`"&New…"`).
    pub new_label: &'a str,
    /// File → Open's label (`"&Open journal…"`).
    pub open_label: &'a str,
    /// Items placed in the macOS app menu after About (Ledgeline's "Make
    /// Ledgeline the Default for Journal Files"). Ignored elsewhere.
    pub app_menu_items: &'a [&'a dyn IsMenuItem],
    /// Items placed in File right after Open Recent (Ledgeline's Export).
    pub after_recent: &'a [&'a dyn IsMenuItem],
}

/// The menu bar — the macOS app menu, then File / Edit / View — and the handles
/// for the custom items an app matches events against.
pub struct AppMenu {
    pub menu_bar: Menu,
    pub new: MenuItem,
    pub open: MenuItem,
    /// File → "Open Recent"; see [`RecentMenu`].
    pub recent: Submenu,
    pub reload: MenuItem,
    pub back: MenuItem,
    pub forward: MenuItem,
    pub zoom_in: MenuItem,
    pub zoom_out: MenuItem,
    pub actual_size: MenuItem,
    /// Linux only: muda's predefined Quit is inert on GTK, so we dispatch our
    /// own. Elsewhere the native predefined item handles it and there is
    /// nothing to match against.
    #[cfg(target_os = "linux")]
    pub quit: MenuItem,
}

impl AppMenu {
    /// Build the application menu bar.
    #[must_use]
    pub fn build(app: &AppIdentity, spec: &MenuSpec<'_>) -> Self {
        build_menu(app, spec)
    }

    /// Which action, if any, a muda menu id stands for. `None` means it may
    /// still be an Open Recent item ([`RecentMenu::picked`]) or one of the
    /// app's own.
    #[must_use]
    pub fn shortcut_for(&self, id: &MenuId) -> Option<Shortcut> {
        #[cfg(target_os = "linux")]
        if id == self.quit.id() {
            return Some(Shortcut::Quit);
        }
        if id == self.new.id() {
            Some(Shortcut::New)
        } else if id == self.open.id() {
            Some(Shortcut::Open)
        } else if id == self.reload.id() {
            Some(Shortcut::Reload)
        } else if id == self.back.id() {
            Some(Shortcut::Back)
        } else if id == self.forward.id() {
            Some(Shortcut::Forward)
        } else if id == self.zoom_in.id() {
            Some(Shortcut::Zoom(Zoom::In))
        } else if id == self.zoom_out.id() {
            Some(Shortcut::Zoom(Zoom::Out))
        } else if id == self.actual_size.id() {
            Some(Shortcut::Zoom(Zoom::Reset))
        } else {
            None
        }
    }
}

/// Build the application menu bar (macOS app menu + File/Edit/View).
fn build_menu(app: &AppIdentity, spec: &MenuSpec<'_>) -> AppMenu {
    let menu_bar = Menu::new();

    #[cfg(target_os = "macos")]
    let app_menu = {
        let app_menu = Submenu::new(app.name, true);
        let about = PredefinedMenuItem::about(None, Some(about_metadata(app, spec.version)));
        let separator = PredefinedMenuItem::separator();
        let tail = [
            PredefinedMenuItem::separator(),
            PredefinedMenuItem::services(None),
            PredefinedMenuItem::separator(),
            PredefinedMenuItem::hide(None),
            PredefinedMenuItem::hide_others(None),
            PredefinedMenuItem::show_all(None),
            PredefinedMenuItem::separator(),
            PredefinedMenuItem::quit(None),
        ];
        let mut items: Vec<&dyn IsMenuItem> = vec![&about, &separator];
        items.extend_from_slice(spec.app_menu_items);
        items.extend(tail.iter().map(|item| item as &dyn IsMenuItem));
        log_menu(app, "app menu", app_menu.append_items(&items));
        app_menu
    };
    #[cfg(not(target_os = "macos"))]
    let _ = (spec.version, spec.app_menu_items);

    let file_menu = Submenu::new("&File", true);
    let new = MenuItem::with_id(
        "new",
        spec.new_label,
        true,
        Some(Accelerator::new(COMMAND_MODIFIER, Code::KeyN)),
    );
    let open = MenuItem::with_id(
        "open",
        spec.open_label,
        true,
        Some(Accelerator::new(COMMAND_MODIFIER, Code::KeyO)),
    );
    // Populated on demand by `RecentMenu::rebuild` (empty until then).
    let recent = Submenu::new("Open &Recent", true);
    log_menu(
        app,
        "file menu",
        file_menu.append_items(&[&new, &open, &recent, &PredefinedMenuItem::separator()]),
    );
    // Right after Open Recent wherever that is.
    let after_recent = file_menu
        .items()
        .iter()
        .position(|item| item.id() == recent.id())
        .map_or(0, |index| index + 1);
    for (offset, item) in spec.after_recent.iter().enumerate() {
        log_menu(
            app,
            "file menu extras",
            file_menu.insert(*item, after_recent + offset),
        );
    }
    // muda implements only Separator/Copy/Cut/Paste/SelectAll/About on GTK —
    // `close_window` and `quit` are BOTH documented "Linux: Unsupported" and
    // are silently inert there. Shipping them on Linux put two dead entries in
    // the File menu and left the window-manager close button as the only way
    // out of the app, which stopped being obvious once the title bar was
    // hidden by default. So Linux gets a real menu item we dispatch ourselves,
    // and macOS/Windows keep the native ones that do work.
    #[cfg(not(target_os = "linux"))]
    log_menu(
        app,
        "file menu close",
        file_menu.append(&PredefinedMenuItem::close_window(Some("Close Window"))),
    );
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    log_menu(
        app,
        "file menu quit",
        file_menu.append_items(&[
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::quit(None),
        ]),
    );
    #[cfg(target_os = "linux")]
    let quit = {
        let quit = MenuItem::with_id(
            "quit",
            "&Quit",
            true,
            Some(Accelerator::new(COMMAND_MODIFIER, Code::KeyQ)),
        );
        log_menu(app, "file menu quit", file_menu.append(&quit));
        quit
    };

    // Standard clipboard items so Cmd/Ctrl+C/V/X work inside the webview.
    let edit_menu = Submenu::new("&Edit", true);
    log_menu(
        app,
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
        Some(Accelerator::new(COMMAND_MODIFIER, Code::KeyR)),
    );
    // Back/Forward are the one pair where the platforms disagree about the KEY
    // and not just the modifier: macOS browsers use Cmd+[ / Cmd+], while Linux
    // and Windows use Alt+← / Alt+→. So this is not a COMMAND_MODIFIER swap.
    #[cfg(target_os = "macos")]
    let (back_accel, forward_accel) = (
        Accelerator::new(COMMAND_MODIFIER, Code::BracketLeft),
        Accelerator::new(COMMAND_MODIFIER, Code::BracketRight),
    );
    #[cfg(not(target_os = "macos"))]
    let (back_accel, forward_accel) = (
        Accelerator::new(Modifiers::ALT, Code::ArrowLeft),
        Accelerator::new(Modifiers::ALT, Code::ArrowRight),
    );
    let back = MenuItem::with_id("back", "&Back", true, Some(back_accel));
    let forward = MenuItem::with_id("forward", "&Forward", true, Some(forward_accel));
    // Cmd/Ctrl `=` rather than `+`: `=` is the unshifted key on a US layout, the
    // one browsers put on Zoom In. The SPA's keymap also takes `+`, for layouts
    // with a dedicated `+` key; see `zoom_script` for why a press never counts
    // twice.
    let zoom_in = MenuItem::with_id(
        "zoom-in",
        "Zoom &In",
        true,
        Some(Accelerator::new(COMMAND_MODIFIER, Code::Equal)),
    );
    let zoom_out = MenuItem::with_id(
        "zoom-out",
        "Zoom &Out",
        true,
        Some(Accelerator::new(COMMAND_MODIFIER, Code::Minus)),
    );
    let actual_size = MenuItem::with_id(
        "actual-size",
        "&Actual Size",
        true,
        Some(Accelerator::new(COMMAND_MODIFIER, Code::Digit0)),
    );
    log_menu(
        app,
        "view menu",
        view_menu.append_items(&[
            &reload,
            &PredefinedMenuItem::separator(),
            &back,
            &forward,
            &PredefinedMenuItem::separator(),
            &zoom_in,
            &zoom_out,
            &actual_size,
        ]),
    );

    #[cfg(target_os = "macos")]
    log_menu(
        app,
        "menu bar",
        menu_bar.append_items(&[&app_menu, &file_menu, &edit_menu, &view_menu]),
    );
    #[cfg(not(target_os = "macos"))]
    log_menu(
        app,
        "menu bar",
        menu_bar.append_items(&[&file_menu, &edit_menu, &view_menu]),
    );

    AppMenu {
        menu_bar,
        new,
        open,
        recent,
        reload,
        back,
        forward,
        zoom_in,
        zoom_out,
        actual_size,
        #[cfg(target_os = "linux")]
        quit,
    }
}

/// File → "Open Recent": the submenu, and the id→path map a click is
/// dispatched through.
pub struct RecentMenu {
    submenu: Submenu,
    map: RefCell<Vec<(MenuId, PathBuf)>>,
    /// How many recents the submenu shows (the on-disk store keeps more; this
    /// is just the visible slice).
    limit: usize,
    /// The disabled placeholder an empty submenu shows (`"No recent journals"`).
    empty_label: &'static str,
}

impl RecentMenu {
    #[must_use]
    pub fn new(submenu: Submenu, limit: usize, empty_label: &'static str) -> Self {
        Self {
            submenu,
            map: RefCell::new(Vec::new()),
            limit,
            empty_label,
        }
    }

    /// Repopulate the submenu from the recents store, excluding the
    /// currently-open document (`current`), and refresh the id→path map used to
    /// dispatch a click. Called at startup and after every successful open so
    /// the list always reflects the latest history.
    pub fn rebuild(&self, app: &AppIdentity, dirs: &AppDirs, current: Option<&Path>) {
        // Clear existing entries (remove index 0 until the submenu is empty) first.
        while self.submenu.remove_at(0).is_some() {}

        let recents: Vec<PathBuf> = recents::list(dirs)
            .into_iter()
            .filter(|path| Some(path.as_path()) != current)
            .take(self.limit)
            .collect();

        if recents.is_empty() {
            // A disabled placeholder so an empty submenu still reads clearly.
            log_menu(
                app,
                "open-recent submenu",
                self.submenu
                    .append(&MenuItem::new(self.empty_label, false, None)),
            );
            self.map.borrow_mut().clear();
            return;
        }

        let mut new_map = Vec::with_capacity(recents.len());
        for (index, path) in recents.iter().enumerate() {
            let item = MenuItem::with_id(
                format!("recent-{index}"),
                recents::display_label(path),
                true,
                None,
            );
            new_map.push((item.id().clone(), path.clone()));
            log_menu(app, "open-recent submenu", self.submenu.append(&item));
        }
        *self.map.borrow_mut() = new_map;
    }

    /// The document a click on `id` picks, if `id` is one of the submenu's.
    /// The map's borrow is released before this returns, so the caller can
    /// open it (which rebuilds the submenu) straight away.
    #[must_use]
    pub fn picked(&self, id: &MenuId) -> Option<PathBuf> {
        self.map
            .borrow()
            .iter()
            .find_map(|(item, path)| (item == id).then(|| path.clone()))
    }
}

/// Should any frame of the WebView be allowed to navigate to `candidate`?
///
/// Only inside our own in-process origin. The window has no address bar, so an
/// unconstrained WebView would happily render an attacker's page as if it were
/// the app — and that page would then be same-origin with nothing, but would
/// still be the user's whole visible UI. `base` is `http://host:port/`, and the
/// required trailing slash is what stops `http://127.0.0.1:5000@evil.example/`
/// from passing as a prefix match.
///
/// This is the rule for EVERY frame, because wry asks the same handler about
/// subframes and does not say which frame is navigating: an app's sandboxed
/// frames (the invoicer's MBR notes and invoice pages) are same-origin pages
/// outside its SPA, and must load. The main frame is held to the narrower
/// [`in_spa`] separately ([`MainFrame`]).
#[must_use]
pub fn navigation_allowed(base: &str, candidate: &str) -> bool {
    candidate == base.trim_end_matches('/') || candidate.starts_with(base)
}

pub use crate::security::{HANDOFF_PATH, plain_https, plain_mailto};

/// The link a hand-off asks the window to open ([`HANDOFF_PATH`]), decoded,
/// or `None` when `candidate` is not one: it must be on our origin
/// ([`navigation_allowed`]), at exactly the hand-off path, with a `url`
/// query value. The target is NOT vetted here; the caller hands it to the
/// app's allowlist like any outside link.
#[must_use]
pub fn handoff_target(base: &str, candidate: &str) -> Option<String> {
    if !navigation_allowed(base, candidate) {
        return None;
    }
    let url = Url::parse(candidate).ok()?;
    if url.path() != HANDOFF_PATH {
        return None;
    }
    url.query_pairs()
        .find(|(key, _)| key == "url")
        .map(|(_, value)| value.into_owned())
        .filter(|target| !target.is_empty())
}

/// Open a URL (already vetted by the app's allowlist) in the user's default
/// app for it: an https page in the browser, a `mailto:` in the mail app.
/// The URL is one argument to a fixed program — no shell.
pub fn open_in_browser(app: &AppIdentity, url: &str) {
    #[cfg(target_os = "macos")]
    let program = "/usr/bin/open";
    #[cfg(target_os = "linux")]
    let program = "xdg-open";
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        eprintln!(
            "{}: opening {url} is not supported on this platform",
            app.log_prefix
        );
        return;
    }
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    if let Err(error) = spawn_reaped(std::process::Command::new(program).arg(url)) {
        eprintln!("{}: could not open {url}: {error}", app.log_prefix);
    }
}

/// Start `command` without waiting for it, and reap it when it exits (on a
/// thread of its own), so a helper such as `open` never lingers as a
/// zombie for the rest of the app's life. `xdg-open` may run as long as the
/// program it opened, which is why nothing here waits.
///
/// Answers the child's process id.
///
/// # Errors
/// When the program can't be started.
pub fn spawn_reaped(command: &mut std::process::Command) -> std::io::Result<u32> {
    let mut child = command.spawn()?;
    let id = child.id();
    // The program runs either way; only the reaping needs the thread.
    let _ = std::thread::Builder::new()
        .name("reap-child".to_string())
        .spawn(move || {
            let _ = child.wait();
        });
    Ok(id)
}

/// Pick the one document to open from the URLs macOS delivered with a document
/// launch (tao's `Event::Opened`).
///
/// Split out as a pure function so it is testable without standing up an event
/// loop. `Url::to_file_path` is doing the real work: it rejects non-`file://`
/// URLs (a deeplink scheme we have not implemented — skipped, not fatal, so a
/// real document later in the same batch still wins) and it undoes the
/// percent-encoding, which matters because Finder escapes spaces and a journal
/// under "My Books" would otherwise resolve to a path that does not exist.
///
/// Only the FIRST resolvable path is returned: the apps are single-document,
/// single-window, so selecting several files in Finder and hitting Open cannot
/// be honoured in full. The caller logs the ones it drops.
#[must_use]
pub fn first_file_path(urls: &[Url]) -> Option<PathBuf> {
    urls.iter().find_map(|url| url.to_file_path().ok())
}

/// Seeds the SPA's first-paint cache (`ledgeline.appearance`, read by the
/// boot script in `app.html`) with the saved theme and scale (plan 39).
///
/// The cache lives in localStorage, which is per origin, and the desktop
/// origin's port changes on every launch, so without this a saved theme would
/// flash the default for a frame on each start. Seeded ONCE per webview
/// session (a sessionStorage marker): init scripts run again on every reload,
/// and by then the SPA has written newer values than these launch-time ones.
///
/// The key is shared by every app on the shell (the theme code is common), so
/// it is not part of [`AppIdentity`].
#[must_use]
pub fn appearance_init_script(origin: &str, appearance: &BootAppearance) -> String {
    let literal = |value: &str| serde_json::Value::from(value).to_string();
    let cache = serde_json::to_string(appearance).unwrap_or_else(|_| "null".to_string());
    format!(
        "if (window.location.origin === {origin}) {{ try {{ \
         if (!sessionStorage.getItem(\"ledgeline.appearance.seeded\")) {{ \
         localStorage.setItem(\"ledgeline.appearance\", {cache}); \
         sessionStorage.setItem(\"ledgeline.appearance.seeded\", \"1\"); }} \
         }} catch (e) {{}} }}",
        origin = literal(origin),
        cache = literal(&cache),
    )
}

/// The main frame's half of the navigation policy: it may show only the app's
/// own SPA ([`in_spa`] under [`AppIdentity::spa_base`]).
///
/// Fed every page the main frame commits to; it remembers the last one that
/// was the SPA, and names it as the page to return to when the main frame
/// lands anywhere else (an MBR page, `about:blank`). Pure state, so the
/// decision is testable without a webview.
#[derive(Debug)]
struct MainFrame {
    origin: String,
    spa_base: &'static str,
    last: String,
}

impl MainFrame {
    /// A main frame that opens on `start`, which must be the SPA.
    fn new(origin: String, spa_base: &'static str, start: &str) -> Result<Self, GuiError> {
        if !in_spa(&origin, spa_base, start) {
            return Err(GuiError(format!(
                "the start page {start} is outside the app's pages ({origin}{spa_base})"
            )));
        }
        Ok(Self {
            origin,
            spa_base,
            last: start.to_string(),
        })
    }

    /// The main frame committed to `page`. `None` when that is the SPA (and
    /// now the page to return to); otherwise the page to load instead.
    fn committed(&mut self, page: &str) -> Option<String> {
        if in_spa(&self.origin, self.spa_base, page) {
            page.clone_into(&mut self.last);
            None
        } else {
            Some(self.last.clone())
        }
    }
}

/// The webview's own origin for `url` (`http://127.0.0.1:<port>`), the value
/// every initialization script is guarded by.
///
/// # Errors
/// A [`GuiError`] when `url` does not parse.
pub fn webview_origin(url: &str) -> Result<String, GuiError> {
    Ok(Url::parse(url)
        .map_err(|error| GuiError(format!("parsing the webview url: {error}")))?
        .origin()
        .ascii_serialization())
}

/// Build the wry webview for `window`: the app at `url`, opened on `start`,
/// with `init_scripts` run at document start (the token, the appearance —
/// each guarded by its own origin check, the token's by the SPA's path too).
///
/// Every frame's navigation is confined to `url`'s origin
/// ([`navigation_allowed`]). A link outside it is offered to `open_external`,
/// which opens it in the user's default app (browser, mail) and returns
/// `true` when the app's allowlist allows it; the webview itself never leaves. The main frame may
/// show only the app's SPA, under [`AppIdentity::spa_base`]: when it commits
/// to any other page, even one on our origin, the last SPA page it showed is
/// loaded instead. A same-origin hand-off ([`handoff_target`]) is offered to
/// `open_external` too, and never loads. Pop-ups are always denied. Downloads ask where to save
/// ([`choose_download_destination`]); a download never commits a page, so the
/// main-frame rule does not touch it.
///
/// Answered in an [`Rc`] because the main-frame rule needs the webview back
/// to load that page; every `&wry::WebView` use derefs through it.
///
/// # Errors
/// A [`GuiError`] when `url` does not parse, when `start` is not under the
/// app's SPA base, or when the webview cannot be created.
pub fn build_webview<F>(
    app: &'static AppIdentity,
    window: &Window,
    url: &str,
    start: &str,
    init_scripts: Vec<String>,
    open_external: F,
) -> Result<Rc<wry::WebView>, GuiError>
where
    F: Fn(&str) -> bool + Clone + 'static,
{
    let base = url.to_string();
    let popup_base = base.clone();
    let main_frame = RefCell::new(MainFrame::new(webview_origin(url)?, app.spa_base, start)?);
    // Filled once the webview exists; the page-load handler cannot run before.
    let handle: Rc<OnceCell<Weak<wry::WebView>>> = Rc::default();
    let loader = Rc::clone(&handle);
    let (navigating, new_window) = (open_external.clone(), open_external);
    let mut builder = WebViewBuilder::new();
    for script in init_scripts {
        builder = builder.with_initialization_script(script);
    }
    let builder = builder
        .with_url(start)
        .with_navigation_handler(move |candidate| {
            // A link a sandboxed frame could not follow, handed off through
            // our own origin: the app's allowlist decides, the frame stays.
            if let Some(target) = handoff_target(&base, &candidate) {
                if !navigating(&target) {
                    eprintln!("{}: blocked hand-off of {target}", app.log_prefix);
                }
                return false;
            }
            let allowed = navigation_allowed(&base, &candidate);
            // An allowlisted external link leaves the webview where it is and
            // opens in the user's browser instead.
            if !allowed && !navigating(&candidate) {
                eprintln!("{}: blocked navigation to {candidate}", app.log_prefix);
            }
            allowed
        })
        // The main frame's rule. wry fires this for the main frame only (on
        // commit: `didCommitNavigation`, webkitgtk's `Committed`), with the
        // main frame's URL, so a subframe's page never reaches it.
        .with_on_page_load_handler(move |event, page| {
            if !matches!(event, PageLoadEvent::Started) {
                return;
            }
            // The borrow ends before `load_url`, which only queues the load.
            let Some(back) = main_frame.borrow_mut().committed(&page) else {
                return;
            };
            eprintln!(
                "{}: refused {page} in the main window; back to {back}",
                app.log_prefix
            );
            if let Some(webview) = loader.get().and_then(Weak::upgrade) {
                let _ = webview.load_url(&back);
            }
        })
        // No pop-ups at all: a new window would be a second WebView outside the
        // navigation handler above. An allowlisted link (`target=_blank`) goes
        // to the user's browser instead.
        .with_new_window_req_handler(move |candidate, _features| {
            if let Some(target) = handoff_target(&popup_base, &candidate) {
                if !new_window(&target) {
                    eprintln!("{}: blocked hand-off of {target}", app.log_prefix);
                }
                return NewWindowResponse::Deny;
            }
            if !new_window(&candidate) {
                eprintln!("{}: blocked new window for {candidate}", app.log_prefix);
            }
            NewWindowResponse::Deny
        })
        // Every xlsx export is a browser download. wry's default handler accepts
        // it into ~/Downloads with no sign that anything happened, so a click
        // looked like a no-op; ask where to save instead, the way a desktop app
        // does.
        .with_download_started_handler(move |_url, destination| {
            choose_download_destination(app, destination)
        })
        .with_download_completed_handler(|_url, path, success| {
            if !success {
                show_download_error(path.as_deref());
            }
        });
    // Dev-only: the launch/scroll probe, when `LEDGELINE_PERF` names one.
    let builder = match perf::script(app) {
        Some(script) => builder
            .with_initialization_script(script)
            .with_ipc_handler(|request| perf::on_message(request.body())),
        None => builder,
    };

    #[cfg(not(target_os = "linux"))]
    let webview = builder
        .build(window)
        .map_err(|error| GuiError(format!("creating webview: {error}")))?;
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
        .map_err(|error| GuiError(format!("creating webview: {error}")))?
    };
    let webview = Rc::new(webview);
    let _ = handle.set(Rc::downgrade(&webview));
    Ok(webview)
}

/// Install `menu` and build the app's main window on `event_loop`.
///
/// Opens at a comfortable desktop size (mbr leaves this to the platform
/// default, which is too small for a data-dense GUI) with a floor so the
/// layout never collapses. The menu goes to `NSApp` before the window on macOS
/// and into the window after it elsewhere.
///
/// # Errors
/// A [`GuiError`] when the window cannot be created.
pub fn build_window<T>(
    app: &AppIdentity,
    event_loop: &EventLoop<T>,
    menu: &Menu,
) -> Result<Window, GuiError> {
    #[cfg(target_os = "macos")]
    menu.init_for_nsapp();

    let window_builder = WindowBuilder::new()
        .with_title(app.name)
        .with_inner_size(LogicalSize::new(1280.0, 832.0))
        .with_min_inner_size(LogicalSize::new(800.0, 600.0));
    // Linux opens with no title bar — the SPA draws its own header with the app
    // name, so the decorations are duplicate furniture under a tiling
    // compositor. Asked for HERE rather than switched off after creation so
    // there is no visible flash of chrome at startup; `F10` brings it back
    // together with the menu bar (see `set_window_chrome_visible`).
    // macOS and Windows keep their native decorations.
    #[cfg(target_os = "linux")]
    let window_builder = window_builder.with_decorations(false);
    let window = window_builder
        .build(event_loop)
        .map_err(|error| GuiError(format!("creating window: {error}")))?;

    #[cfg(target_os = "windows")]
    {
        use tao::platform::windows::WindowExtWindows;
        // SAFETY: called once, on the main thread, with this window's live HWND.
        unsafe {
            let _ = menu.init_for_hwnd(window.hwnd() as isize);
        }
    }
    #[cfg(target_os = "linux")]
    {
        use tao::platform::unix::WindowExtUnix;
        let _ = menu.init_for_gtk_window(window.gtk_window(), window.default_vbox());
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    let _ = menu;
    Ok(window)
}

/// Ask where to save a download, starting from wry's proposed destination (the
/// Downloads folder plus the page's suggested filename). `false` cancels it.
///
/// Runs on the main thread and blocks: WebKit wants the destination as the
/// return value, so this cannot hop to a background thread like a file picker.
/// The save panel already confirmed any overwrite, and WebKit fails a download
/// whose destination exists, so a file the user chose to replace is removed
/// first.
pub fn choose_download_destination(app: &AppIdentity, destination: &mut PathBuf) -> bool {
    let suggested = destination
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let dialog = rfd::FileDialog::new()
        .set_title("Save export")
        .set_file_name(suggested);
    let dialog = match destination.parent() {
        Some(dir) => dialog.set_directory(dir),
        None => dialog,
    };
    let Some(chosen) = dialog.save_file() else {
        return false;
    };
    if chosen.exists()
        && let Err(error) = std::fs::remove_file(&chosen)
    {
        eprintln!(
            "{}: could not replace {}: {error}",
            app.log_prefix,
            chosen.display()
        );
        return false;
    }
    *destination = chosen;
    true
}

/// Report a download WebKit could not finish (a full disk, a folder that went
/// away) — the save dialog closed, so silence would read as success.
pub fn show_download_error(path: Option<&Path>) {
    let description = match path {
        Some(path) => format!("Could not save {}.", path.display()),
        None => "The export could not be saved.".to_string(),
    };
    show_error("Export failed", description);
}

/// An error dialog, on its own thread so the event loop keeps running.
pub fn show_error(title: &'static str, description: String) {
    std::thread::spawn(move || {
        rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Error)
            .set_title(title)
            .set_description(description)
            .set_buttons(rfd::MessageButtons::Ok)
            .show();
    });
}

#[cfg(all(test, target_os = "linux"))]
mod linux_keyboard_tests {
    use super::{Shortcut, linux_shortcut_for};
    use tao::keyboard::{Key, ModifiersState};

    const CTRL: ModifiersState = ModifiersState::CONTROL;
    const ALT: ModifiersState = ModifiersState::ALT;
    const SHIFT: ModifiersState = ModifiersState::SHIFT;

    #[test]
    fn the_bound_chords_map_to_their_actions() {
        assert_eq!(
            linux_shortcut_for(&Key::Character("n"), CTRL),
            Some(Shortcut::New)
        );
        assert_eq!(
            linux_shortcut_for(&Key::Character("o"), CTRL),
            Some(Shortcut::Open)
        );
        assert_eq!(
            linux_shortcut_for(&Key::Character("r"), CTRL),
            Some(Shortcut::Reload)
        );
        assert_eq!(
            linux_shortcut_for(&Key::ArrowLeft, ALT),
            Some(Shortcut::Back)
        );
        assert_eq!(
            linux_shortcut_for(&Key::ArrowRight, ALT),
            Some(Shortcut::Forward)
        );
        assert_eq!(
            linux_shortcut_for(&Key::Character("q"), CTRL),
            Some(Shortcut::Quit)
        );
    }

    #[test]
    fn the_modifier_must_match_exactly() {
        // Unmodified keys belong to the page, not to us.
        assert_eq!(
            linux_shortcut_for(&Key::Character("o"), ModifiersState::empty()),
            None
        );
        assert_eq!(
            linux_shortcut_for(&Key::ArrowLeft, ModifiersState::empty()),
            None
        );

        // The two modifiers are not interchangeable.
        assert_eq!(linux_shortcut_for(&Key::Character("o"), ALT), None);
        assert_eq!(linux_shortcut_for(&Key::ArrowLeft, CTRL), None);

        // A SUPERset is a different chord: Ctrl+Alt+O must not fire Open, or a
        // desktop-level binding would be shadowed by ours.
        assert_eq!(linux_shortcut_for(&Key::Character("o"), CTRL | ALT), None);
        assert_eq!(linux_shortcut_for(&Key::ArrowLeft, ALT | CTRL), None);
        assert_eq!(
            linux_shortcut_for(&Key::Character("o"), CTRL | ModifiersState::SUPER),
            None
        );
    }

    #[test]
    fn shift_is_folded_into_the_logical_key_rather_than_examined() {
        // tao reports the SHIFTED meaning in `logical_key`, so Ctrl+Shift+O
        // arrives as an uppercase character with SHIFT still in the mask. It is
        // the same action, which is why the comparison is case-insensitive and
        // why SHIFT is not part of the exactness test.
        assert_eq!(
            linux_shortcut_for(&Key::Character("O"), CTRL | SHIFT),
            Some(Shortcut::Open)
        );
        assert_eq!(
            linux_shortcut_for(&Key::Character("R"), CTRL | SHIFT),
            Some(Shortcut::Reload)
        );
    }

    #[test]
    fn unbound_keys_are_left_to_the_page() {
        // F10 is handled by its OWN arm (it must work whether the menu bar is
        // shown or hidden), so it must not appear in this table.
        assert_eq!(linux_shortcut_for(&Key::F10, ModifiersState::empty()), None);
        // The SPA binds Ctrl+D / Ctrl+U for half-page scrolling; a native
        // accelerator here would take them away from the page.
        assert_eq!(linux_shortcut_for(&Key::Character("d"), CTRL), None);
        assert_eq!(linux_shortcut_for(&Key::Character("u"), CTRL), None);
        assert_eq!(
            linux_shortcut_for(&Key::Escape, ModifiersState::empty()),
            None
        );
    }
}

/// Dev-only launch and scroll probe (`docs/development/perf-baseline.md`).
///
/// Inert unless `LEDGELINE_PERF` names a probe script (`scripts/perf/probe.js`).
/// When it does, the script is injected into every page the WebView loads and
/// may report back over `window.ipc.postMessage`; each message is printed to
/// stderr as `PERF js <ms since launch> <json>`, the Rust-side milestones as
/// `PERF rust <label> <ms since launch>`, and a `{"ev":"scroll_done",…}`
/// message ends the process — so a measurement is one command, start to exit.
pub mod perf {
    use std::sync::OnceLock;
    use std::time::Instant;

    use crate::AppIdentity;

    /// The environment variable naming the probe script.
    const ENV: &str = "LEDGELINE_PERF";

    static START: OnceLock<Instant> = OnceLock::new();

    fn enabled() -> bool {
        std::env::var_os(ENV).is_some()
    }

    fn elapsed_ms() -> f64 {
        START.get_or_init(Instant::now).elapsed().as_secs_f64() * 1000.0
    }

    /// Fix the launch instant every other stamp is measured from.
    pub fn start() {
        START.get_or_init(Instant::now);
    }

    /// Print a Rust-side milestone, when probing.
    pub fn stamp(label: &str) {
        if enabled() {
            eprintln!("PERF rust {label} {:.1}", elapsed_ms());
        }
    }

    /// The probe script to inject, when `LEDGELINE_PERF` names a readable file.
    pub(super) fn script(app: &AppIdentity) -> Option<String> {
        let path = std::env::var_os(ENV)?;
        match std::fs::read_to_string(&path) {
            Ok(script) => Some(script),
            Err(error) => {
                eprintln!(
                    "{}: {ENV}: cannot read {}: {error}",
                    app.log_prefix,
                    path.to_string_lossy()
                );
                None
            }
        }
    }

    /// One message from the probe: printed, and the end of the run on `scroll_done`.
    pub(super) fn on_message(body: &str) {
        eprintln!("PERF js {:.1} {body}", elapsed_ms());
        if body.contains("\"ev\":\"scroll_done\"") {
            std::process::exit(0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BootAppearance, HANDOFF_PATH, MainFrame, Zoom, appearance_init_script, first_file_path,
        handoff_target, navigation_allowed, plain_https, webview_origin, zoom_script,
    };
    use std::path::PathBuf;
    use url::Url;

    #[cfg(unix)]
    #[test]
    fn spawn_reaped_leaves_no_zombie() {
        // Whether process `id` still exists (a zombie counts until it is
        // reaped), or `None` when this platform can't tell. No `PATH` lookup:
        // the Nix build sandbox has no `ps` on Linux, so Linux reads `/proc`.
        fn still_there(id: u32) -> Option<bool> {
            if cfg!(target_os = "linux") {
                return Some(std::path::Path::new(&format!("/proc/{id}")).exists());
            }
            let found = std::process::Command::new("/bin/ps")
                .args(["-o", "stat=", "-p", &id.to_string()])
                .output()
                .ok()?;
            Some(!String::from_utf8_lossy(&found.stdout).trim().is_empty())
        }

        let id = super::spawn_reaped(&mut std::process::Command::new("true")).expect("spawns");
        if still_there(std::process::id()) != Some(true) {
            eprintln!("skipped: this platform can't list processes");
            return;
        }
        let gone = (0..200).any(|_| {
            if still_there(id) == Some(false) {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
            false
        });
        assert!(gone, "process {id} was never reaped");
    }

    #[test]
    fn appearance_script_seeds_the_boot_cache_once_on_our_origin() {
        let appearance = BootAppearance {
            light: "catppuccin-latte",
            dark: "catppuccin-mocha",
            scale: 125,
        };
        let script = appearance_init_script("http://127.0.0.1:5555", &appearance);
        assert!(script.starts_with(r#"if (window.location.origin === "http://127.0.0.1:5555")"#));
        assert!(script.contains(
            r#"localStorage.setItem("ledgeline.appearance", "{\"light\":\"catppuccin-latte\",\"dark\":\"catppuccin-mocha\",\"scale\":125}")"#
        ));
        assert!(script.contains(r#"sessionStorage.getItem("ledgeline.appearance.seeded")"#));
    }

    #[test]
    fn zoom_scripts_dispatch_the_event_the_spa_listens_for() {
        // The SPA's `ZOOM_EVENT` and `isZoomAction` (web/src/lib/theme/zoom.svelte.ts).
        for (zoom, detail) in [(Zoom::In, "in"), (Zoom::Out, "out"), (Zoom::Reset, "reset")] {
            let script = zoom_script(zoom);
            assert!(script.starts_with("window.dispatchEvent(new CustomEvent('ledgeline:zoom'"));
            assert!(
                script.contains(&format!("{{detail: '{detail}'}}")),
                "{script}"
            );
        }
    }

    #[test]
    fn plain_https_refuses_ports_credentials_and_other_schemes() {
        assert!(plain_https(" https://example.com/x ").is_some());
        for refused in [
            "http://example.com/",
            "https://user@example.com/",
            "https://user:secret@example.com/",
            "https://example.com:8443/",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "not a url",
        ] {
            assert!(plain_https(refused).is_none(), "{refused}");
        }
    }

    #[test]
    fn the_webview_origin_is_scheme_host_and_port() {
        assert_eq!(
            webview_origin("http://127.0.0.1:5000/").expect("parses"),
            "http://127.0.0.1:5000"
        );
        assert!(webview_origin("not a url").is_err());
    }

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
        // Opaque-origin documents are never ours, in any frame.
        for refused in [
            "about:blank",
            "about:srcdoc",
            "data:text/html,<script>alert(1)</script>",
            "javascript:alert(1)",
            "https://127.0.0.1:5000/",
            "http://localhost:5000/",
        ] {
            assert!(!navigation_allowed(BASE, refused), "{refused}");
        }
    }

    /// Any frame may still load our origin's other pages: the invoicer's
    /// sandboxed MBR frames are exactly that. The main frame is held to the
    /// SPA by [`MainFrame`] instead.
    #[test]
    fn frames_may_load_same_origin_pages_outside_the_spa() {
        assert!(navigation_allowed(
            BASE,
            "http://127.0.0.1:5000/notes/acme.md"
        ));
        assert!(navigation_allowed(
            BASE,
            "http://127.0.0.1:5000/.ledgeline-invoicer/preview?x=1"
        ));
    }

    const ORIGIN: &str = "http://127.0.0.1:5000";

    /// The string the invoicer's SPA writes (`invoicer/web/src/lib/frameLinks.ts`).
    #[test]
    fn the_handoff_path_is_pinned() {
        assert_eq!(HANDOFF_PATH, "/.ledgeline/open");
    }

    #[test]
    fn a_same_origin_handoff_names_its_decoded_target() {
        assert_eq!(
            handoff_target(
                BASE,
                "http://127.0.0.1:5000/.ledgeline/open?url=mailto%3Aap%40example.com%3Fsubject%3DInvoice%2520108"
            )
            .as_deref(),
            Some("mailto:ap@example.com?subject=Invoice%20108")
        );
    }

    #[test]
    fn anything_else_is_no_handoff() {
        for candidate in [
            "http://127.0.0.1:5001/.ledgeline/open?url=mailto%3Aa%40b.c",
            "http://localhost:5000/.ledgeline/open?url=mailto%3Aa%40b.c",
            "https://127.0.0.1:5000/.ledgeline/open?url=mailto%3Aa%40b.c",
            "http://127.0.0.1:5000/.ledgeline/open/x?url=mailto%3Aa%40b.c",
            "http://127.0.0.1:5000/.ledgeline/opener?url=mailto%3Aa%40b.c",
            "http://127.0.0.1:5000/.ledgeline/open",
            "http://127.0.0.1:5000/.ledgeline/open?link=mailto%3Aa%40b.c",
            "http://127.0.0.1:5000/.ledgeline/open?url=",
            "http://127.0.0.1:5000/notes/acme.md?url=mailto%3Aa%40b.c",
            "http://127.0.0.1:5000/app/",
            "mailto:a@b.c",
        ] {
            assert_eq!(handoff_target(BASE, candidate), None, "{candidate}");
        }
    }

    #[test]
    fn the_main_frame_must_start_in_the_spa() {
        assert!(MainFrame::new(ORIGIN.into(), "/app/", "http://127.0.0.1:5000/app/").is_ok());
        assert!(MainFrame::new(ORIGIN.into(), "/", "http://127.0.0.1:5000/new").is_ok());
        for outside in [
            "http://127.0.0.1:5000/",
            "http://127.0.0.1:5000/notes/acme.md",
            "http://127.0.0.1:5001/app/",
            "about:blank",
        ] {
            assert!(
                MainFrame::new(ORIGIN.into(), "/app/", outside).is_err(),
                "{outside}"
            );
        }
    }

    /// SEC-6: the main frame shows only the SPA. Any other page it commits
    /// to, on our origin or not, sends it back to the last SPA page it showed.
    #[test]
    fn the_main_frame_returns_to_its_last_spa_page_from_anything_else() {
        let home = "http://127.0.0.1:5000/app/";
        let mut frame = MainFrame::new(ORIGIN.into(), "/app/", home).expect("starts in the SPA");
        assert_eq!(frame.committed(home), None);
        assert_eq!(
            frame.committed("http://127.0.0.1:5000/app/invoices/7"),
            None
        );
        for refused in [
            "http://127.0.0.1:5000/customers/contracts/sow.html",
            "http://127.0.0.1:5000/app/../notes/acme.md",
            "http://127.0.0.1:5000/app/%2e%2e/notes/acme.md",
            "http://127.0.0.1:5000/.mbr/notes",
            "http://127.0.0.1:5000/api/books",
            "http://127.0.0.1:5001/app/",
            "http://evil.example/app/",
            "about:blank",
            "data:text/html,x",
            "javascript:alert(1)",
        ] {
            assert_eq!(
                frame.committed(refused).as_deref(),
                Some("http://127.0.0.1:5000/app/invoices/7"),
                "{refused}"
            );
        }
        // A refused page is not remembered: the SPA page still is.
        assert_eq!(frame.committed("http://127.0.0.1:5000/app/customers"), None);
        assert_eq!(
            frame
                .committed("http://127.0.0.1:5000/notes/x.md")
                .as_deref(),
            Some("http://127.0.0.1:5000/app/customers")
        );
    }

    /// Ledgeline's SPA is the whole origin: every page on it is fine, so its
    /// window behaves exactly as before.
    #[test]
    fn a_whole_origin_spa_main_frame_refuses_only_other_origins() {
        let mut frame = MainFrame::new(ORIGIN.into(), "/", "http://127.0.0.1:5000/")
            .expect("starts in the SPA");
        for page in [
            "http://127.0.0.1:5000/reports?tab=1",
            "http://127.0.0.1:5000/settings",
            "http://127.0.0.1:5000/new",
        ] {
            assert_eq!(frame.committed(page), None, "{page}");
        }
        assert_eq!(
            frame.committed("about:blank").as_deref(),
            Some("http://127.0.0.1:5000/new")
        );
    }

    #[test]
    fn document_launch_resolves_the_first_usable_file_url() {
        // The ordinary Finder double-click.
        assert_eq!(
            first_file_path(&urls(&["file:///tmp/main.journal"])),
            Some(PathBuf::from("/tmp/main.journal"))
        );

        // Finder percent-encodes spaces; `to_file_path` is what decodes them back
        // into the path that actually exists on disk.
        assert_eq!(
            first_file_path(&urls(&["file:///Users/me/My%20Books/main.journal"])),
            Some(PathBuf::from("/Users/me/My Books/main.journal"))
        );

        // A deeplink we do not implement is skipped rather than opened.
        assert_eq!(
            first_file_path(&urls(&["https://example.com/main.journal"])),
            None
        );

        // No URLs at all (nothing to open) must not panic.
        assert_eq!(first_file_path(&[]), None);

        // A non-file URL must not shadow a real document behind it in the batch.
        assert_eq!(
            first_file_path(&urls(&[
                "ledgeline://open",
                "file:///tmp/second.journal",
                "file:///tmp/third.journal",
            ])),
            Some(PathBuf::from("/tmp/second.journal"))
        );
    }
}
