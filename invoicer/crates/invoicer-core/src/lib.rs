//! Ledgeline Invoicer's core: everything but UI and platform
//! (`invoicer/ARCHITECTURE.md` §2.2).
//!
//! So far:
//!
//! - [`table`]: the bill-table engine (§3.3) — parse with spans, recognize
//!   columns and rows, compute amounts, check, and write back aligned;
//! - [`booking`]: the pure booking split of a computed table into one line
//!   per account and deferral (§4.2);
//! - [`frontmatter`]: read losslessly, edit by key (§5.5);
//! - [`problem`]: what `check` reports;
//! - [`settings`]: `ledgeline.toml [invoicer]` (§3.8);
//! - [`notes`]: discovery, behind an API shaped like MBR's `Site` (§3.1);
//! - [`customers`] (§3.4) and [`snippets`] (§3.7);
//! - [`doc`]: the invoice document and its validation (§3.2);
//! - [`catalog`] (§3.6) and [`templates`] (§3.5);
//! - [`numbering`] (§5.1);
//! - [`index`]: the in-memory document index (§5.6);
//! - [`draft`]: creating drafts and saving documents (§5.2);
//! - [`books`]: the books bridge — what an invoice books, send, re-book,
//!   void, deposits and cash-books Apply payment (§4).
//!
//! Plan 26 rules apply: no globals, no platform `cfg`, and all IO through a
//! Ledgeline `Vault`.

pub mod booking;
pub mod books;
pub mod catalog;
pub mod customers;
pub mod doc;
pub mod draft;
pub mod frontmatter;
pub mod index;
pub mod notes;
pub mod numbering;
mod par;
pub mod problem;
pub mod settings;
pub mod snippets;
pub mod table;
pub mod templates;
