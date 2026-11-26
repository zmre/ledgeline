//! The shell Ledgeline's apps share: everything between "a router of API
//! routes plus a built SPA" and "a hardened local server in a desktop window".
//!
//! - [`security`] — the per-process bearer token, the `Host` guard, the
//!   exact-origin CORS allowlist and the security headers (CSP included), plus
//!   [`security::harden`] / [`security::protect`], which put them on a router.
//! - [`spa`] — serving an app's `RustEmbed`-ed SPA same-origin, with the
//!   embedded-mode marker injected and a per-response CSP.
//! - [`app_dirs`] — where an app keeps its own state ([`AppDirs`]).
//! - [`recents`] — the recently-opened store, shared by every app.
//! - [`gui`] (the `gui` feature) — the desktop boot pieces: the in-process
//!   server on an ephemeral port, the window, the standard menus, the wry
//!   webview with its navigation policy, and the platform glue.
//!
//! The UI-preferences data model lives one layer down, in
//! [`ledgeline_core::ui_prefs`], because it is data, not runtime.
//!
//! Nothing here knows which app it is serving: an app describes itself once
//! with an [`AppIdentity`] and hands it to the pieces that need a name.

pub mod app_dirs;
#[cfg(feature = "gui")]
pub mod gui;
pub mod recents;
pub mod security;
pub mod spa;

pub use app_dirs::AppDirs;

/// Who the shell is serving: the names an app shows, logs under and hands to
/// its SPA.
///
/// The SPA reads the two globals at startup, so they are part of the app's
/// contract with its own web build and must never change for an app that has
/// shipped (Ledgeline's are `__LEDGELINE_TOKEN__` and
/// `__LEDGELINE_EMBEDDED__`). The shell embeds them in scripts after checking
/// nothing, so they must be plain JavaScript identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppIdentity {
    /// The product name: the window title, the macOS app menu, About.
    pub name: &'static str,
    /// The macOS bundle identifier (`CFBundleIdentifier`).
    pub bundle_id: &'static str,
    /// The prefix of every line the shell writes to stderr (`ledgeline: …`).
    pub log_prefix: &'static str,
    /// The environment variable a caller-chosen access token is read from.
    pub token_env: &'static str,
    /// The `window` global the access token is handed to the SPA in.
    pub token_global: &'static str,
    /// The `window` global that tells the SPA it is served embedded.
    pub embedded_global: &'static str,
    /// The path the app's SPA is served under, with a leading and a trailing
    /// slash: `/` when the SPA is the whole origin (Ledgeline), `/app/` when
    /// other content shares it (the invoicer, beside MBR's pages).
    ///
    /// The desktop window shows only documents under it
    /// ([`gui::build_webview`]), and only they are handed the token
    /// ([`security::token_init_script`]). [`security::in_spa`] is the test.
    pub spa_base: &'static str,
}
