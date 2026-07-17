//! `ledgeline-core` — the Ledgeline journal engine.
//!
//! Parses hledger journal files into an exact-decimal data model and serializes
//! a wire-compatible view of the journal. This crate ports the contracts
//! already proven in the TypeScript engine under `web/src/lib` (domain,
//! reports, holdings) and is verified against the hledger snapshots in
//! `fixtures/` (see the approved engine plan).
//!
//! Modules land per phase:
//! - `decimal` — exact `Dec` money type (Phase 1)
//! - `model`   — `Journal`/`Transaction`/`Posting`/`Amount` (Phase 1)
//! - `parse`   — journal-file parser (Phase 1)
//! - `wire`    — hledger 1.52 JSON serialization (Phase 1)
//! - `reports` / `budget` — native reports (Phase 3)
//! - `edit`    — ropey-based write path (Phase 5)

pub mod decimal;
pub mod model;
pub mod parse;
pub mod reports;
pub mod wire;

pub use decimal::{Dec, DecError};
pub use model::Journal;
pub use parse::{ParseError, parse_journal};
pub use reports::ReportError;

/// Parse `text` and serialize its transactions to the hledger-compatible JSON
/// array in one step.
///
/// # Errors
/// Returns [`ParseError`] if the journal cannot be parsed. (The subsequent
/// serialization never fails for finite, well-formed input.)
pub fn parse_to_transactions_value(
    text: &str,
    source_name: &str,
) -> Result<serde_json::Value, ParseError> {
    let journal = parse_journal(text, source_name)?;
    // `to_value` cannot fail for our finite, string-keyed structures.
    Ok(wire::journal_to_value(&journal).unwrap_or(serde_json::Value::Null))
}
