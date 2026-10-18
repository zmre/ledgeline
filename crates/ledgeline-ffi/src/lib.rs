//! ledgeline-ffi — the UniFFI surface the Apple Swift shell links.
//!
//! Built as the `libledgeline_ffi.a` staticlib an iOS app links; the Swift
//! bindings are generated from it with `uniffi-bindgen` (proc-macros, no UDL).
//! See `README.md` for the contract with the Swift side.
//!
//! Two halves:
//!
//! - **Storage, always compiled** (depends on `ledgeline-core` alone):
//!   [`CoordinatedVault`] — a granted folder read with `std::fs` and written
//!   through the app's [`FileCoordinator`], with iCloud's not-downloaded files
//!   reported as such — and [`SingleFileVault`] for one file opened in place,
//!   plus the fail-fast [`iopolicy`] for dataless files.
//! - **The app, with the `mobile` feature** ([`mobile`]): [`mobile::LedgelineApp`],
//!   the desktop server's router run in-process behind the app's custom-scheme
//!   handler.
//!
//! The UniFFI exports are Apple targets only (`target_vendor = "apple"`, so
//! macOS too — which is where the host tests exercise them). Elsewhere the
//! crate is plain Rust, so `cargo build --workspace` and its tests work on any
//! host. Platform `cfg`s live here, never in core (plan 26).

#[cfg(target_vendor = "apple")]
uniffi::setup_scaffolding!();

mod coordinated_vault;
pub mod coordinator;
pub mod iopolicy;
mod single_file;

#[cfg(feature = "mobile")]
pub mod mobile;

pub use coordinated_vault::CoordinatedVault;
pub use coordinator::{CoordinatorError, FileCoordinator};
pub use single_file::SingleFileVault;
