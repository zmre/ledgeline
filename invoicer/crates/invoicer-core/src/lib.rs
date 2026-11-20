//! Ledgeline Invoicer's core: everything but UI and platform
//! (`invoicer/ARCHITECTURE.md` §2.2).
//!
//! So far:
//!
//! - [`table`]: the bill-table engine (§3.3) — parse with spans, recognize
//!   columns and rows, compute amounts, check, and write back aligned;
//! - [`booking`]: the pure booking split of a computed table into one line
//!   per account and deferral (§4.2).
//!
//! Plan 26 rules apply: no globals, no IO here, no platform `cfg`.

pub mod booking;
pub mod table;
