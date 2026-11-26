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
//! - [`settings`]: `ledgeline.toml [invoicer]` (§3.8), and
//!   [`settings_edit`]: writing it from the Settings page;
//! - [`notes`]: discovery, behind an API shaped like MBR's `Site` (§3.1);
//! - [`customers`] (§3.4) and [`snippets`] (§3.7);
//! - [`customer_edit`]: creating and editing customer notes (PLAN 4b);
//! - [`customer_import`]: customers from columns (`customers new` flags,
//!   `customers import-csv` rows), through [`customer_edit`];
//! - [`doc`]: the invoice document and its validation (§3.2);
//! - [`catalog`] (§3.6) and [`templates`] (§3.5), with [`tera_scan`] (a
//!   light scan of Tera tags) and [`template_edit`] (the template editor,
//!   PLAN 4d);
//! - [`catalog_edit`] and [`snippet_edit`]: the catalog and snippets editors
//!   (PLAN 4c);
//! - [`numbering`] (§5.1);
//! - [`index`]: the in-memory document index (§5.6);
//! - [`draft`]: creating drafts and saving documents (§5.2);
//! - [`session`]: an open books folder (§6.4);
//! - [`views`]: what the read commands show, for the CLI and the API;
//! - [`edit`]: the commands that write documents (`new`, `set`, `recalc`);
//! - [`editor`]: the app editor's form of a document, and saving it (4a);
//! - [`books`]: the books bridge — what an invoice books, send, re-book,
//!   void, deposits, cash-books Apply payment and recognition (§4);
//! - [`reconcile`]: the books' payment state, the `paid:` mirror, and the
//!   books checks (§5.4, §4.3, §4.5);
//! - [`receivables`]: open invoices, aging, DSO (§4.6).
//!
//! Plan 26 rules apply: no globals, no platform `cfg`, and all IO through a
//! Ledgeline `Vault`.

pub mod booking;
pub mod books;
pub mod catalog;
pub mod catalog_edit;
pub mod customer_edit;
pub mod customer_import;
pub mod customers;
pub mod doc;
pub mod draft;
pub mod edit;
pub mod editor;
pub mod frontmatter;
pub mod index;
pub mod notes;
pub mod numbering;
mod par;
pub mod problem;
pub mod receivables;
pub mod reconcile;
pub mod sent;
pub mod session;
pub mod settings;
pub mod settings_edit;
pub mod snippet_edit;
pub mod snippets;
pub mod table;
pub mod template_edit;
pub mod templates;
pub mod tera_scan;
pub mod views;
