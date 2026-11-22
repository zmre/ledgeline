//! Undo records for **Apply payment** (cash books): what a later run needs to
//! put a deposit back as it was.
//!
//! Applying a payment rewrites a deposit Ledgeline imported; undoing it
//! writes the deposit's original text back exactly
//! ([`InvoiceBooks::undo_payment`]). That text exists nowhere else once the
//! apply is written, so each apply leaves an **undo record**: one small,
//! pretty-printed JSON file per applied deposit, in the books folder under
//! [`UNDO_DIR`] (`.ledgeline-invoicer/undo/`):
//!
//! - **Not in the journal.** The books hold only accounting; an undo record
//!   is the invoicer's own bookkeeping, and a hidden folder keeps it out of
//!   discovery (MBR and the invoicer skip hidden names) and out of the way.
//! - **Hand-inspectable.** It names the invoice, the deposit's date and
//!   description, and both texts, so a person can undo by hand too.
//! - **One file per apply**, named `apply-payment-<number>-<ref>.json`, where
//!   `<ref>` starts the reference of the deposit as the apply wrote it. A
//!   successful undo deletes its file; a stale one (the deposit was edited
//!   since) is refused by the books and kept, for a person to read.
//!
//! The file is written after the journal: if that write fails, the payment
//! stands and the caller reports that it can only be undone by hand.

use super::{BooksError, PaymentApplied, PaymentUndo};
use ledgeline_core::vault::{EntryKind, Vault, VaultError, VaultPath};
use serde::{Deserialize, Serialize};

/// Where undo records live, relative to the books folder.
pub const UNDO_DIR: &str = ".ledgeline-invoicer/undo";

/// The kind of every record written so far.
const APPLY_PAYMENT: &str = "apply-payment";

/// The largest undo record read.
const MAX_RECORD_BYTES: u64 = 4 * 1024 * 1024;

/// One applied payment's undo record, as written to its file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UndoRecord {
    /// What it undoes: `apply-payment`.
    pub kind: String,
    /// The invoice.
    pub number: String,
    /// The fingerprint of the deposit as the apply wrote it: the undo is
    /// refused if it changed since.
    pub reference: String,
    /// The day it was applied.
    pub applied_on: String,
    /// The deposit's date.
    pub deposit_date: String,
    /// The deposit's description.
    pub deposit_description: String,
    /// The deposit's text before the apply: what an undo writes back.
    pub original_text: String,
    /// The deposit's text after the apply.
    pub applied_text: String,
}

impl UndoRecord {
    /// The record of `applied`, made on `today`.
    #[must_use]
    pub fn of(applied: &PaymentApplied, today: &str) -> Self {
        Self {
            kind: APPLY_PAYMENT.to_string(),
            number: applied.number.clone(),
            reference: applied.undo.reference.clone(),
            applied_on: today.to_string(),
            deposit_date: applied.transaction.date.clone(),
            deposit_description: applied.transaction.description.clone(),
            original_text: applied.undo.original_text.clone(),
            applied_text: applied.transaction.text.clone(),
        }
    }

    /// What [`InvoiceBooks::undo_payment`](super::InvoiceBooks::undo_payment)
    /// takes.
    #[must_use]
    pub fn undo(&self) -> PaymentUndo {
        PaymentUndo {
            number: self.number.clone(),
            reference: self.reference.clone(),
            original_text: self.original_text.clone(),
        }
    }
}

/// `text` as a file-name part: letters, digits, `-`, `_` and `.` kept, the
/// rest `_`.
fn file_part(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn dir() -> VaultPath {
    VaultPath::new(UNDO_DIR).unwrap_or_else(|_| VaultPath::root())
}

fn storage(path: &VaultPath, source: VaultError) -> BooksError {
    BooksError::Storage {
        path: path.clone(),
        source,
    }
}

/// Write `record` to its file under [`UNDO_DIR`]; returns the file.
///
/// # Errors
/// [`BooksError::Storage`].
pub fn save(vault: &dyn Vault, record: &UndoRecord) -> Result<VaultPath, BooksError> {
    let dir = dir();
    let reference: String = record.reference.chars().take(12).collect();
    let name = format!(
        "{}-{}-{}.json",
        record.kind,
        file_part(&record.number),
        file_part(&reference)
    );
    let path = dir
        .join(&name)
        .map_err(|e| storage(&dir, VaultError::from_io(&dir, std::io::Error::other(e))))?;
    vault
        .create_dir_all(&dir)
        .map_err(|source| storage(&dir, source))?;
    let mut text = serde_json::to_string_pretty(record).map_err(|e| {
        storage(
            &path,
            VaultError::from_io(&path, std::io::Error::other(e.to_string())),
        )
    })?;
    text.push('\n');
    vault
        .write_atomic(&path, text.as_bytes())
        .map_err(|source| storage(&path, source))?;
    Ok(path)
}

/// Every readable undo record for invoice `number`, oldest applied first
/// (then by file). A missing folder is none; an unreadable file is skipped.
#[must_use]
pub fn records(vault: &dyn Vault, number: &str) -> Vec<(VaultPath, UndoRecord)> {
    let Ok(entries) = vault.list_dir(&dir(), Some(10_000)) else {
        return Vec::new();
    };
    let mut out: Vec<(VaultPath, UndoRecord)> = entries
        .into_iter()
        .filter(|e| e.kind == EntryKind::File && e.path.as_str().ends_with(".json"))
        .filter_map(|e| {
            let bytes = vault.read_prefix(&e.path, MAX_RECORD_BYTES).ok()?;
            let record: UndoRecord = serde_json::from_slice(&bytes).ok()?;
            (record.kind == APPLY_PAYMENT && record.number == number).then_some((e.path, record))
        })
        .collect();
    out.sort_by(|a, b| (a.1.applied_on.as_str(), &a.0).cmp(&(b.1.applied_on.as_str(), &b.0)));
    out
}

/// Delete an undo record (after its undo landed).
///
/// # Errors
/// [`BooksError::Storage`].
pub fn remove(vault: &dyn Vault, path: &VaultPath) -> Result<(), BooksError> {
    vault
        .remove_file(path)
        .map_err(|source| storage(path, source))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::books::BookedTransaction;
    use ledgeline_core::Dec;
    use ledgeline_core::vault::MemVault;

    fn applied(number: &str, reference: &str) -> PaymentApplied {
        PaymentApplied {
            number: number.into(),
            transaction: BookedTransaction {
                date: "2026-11-12".into(),
                description: "WIRE".into(),
                text: "2026-11-12 WIRE  ; invoice: 202\n".into(),
            },
            received: Dec::new(50_000, 2),
            applied: Vec::new(),
            remaining: Dec::zero(),
            undo: PaymentUndo {
                number: number.into(),
                reference: reference.into(),
                original_text: "2026-11-12 WIRE\n".into(),
            },
        }
    }

    #[test]
    fn records_round_trip_by_number_and_are_removed() {
        let vault = MemVault::new();
        let first = save(
            &vault,
            &UndoRecord::of(&applied("202", "abcdef0123456789"), "2026-11-17"),
        )
        .expect("saves");
        assert_eq!(
            first.as_str(),
            ".ledgeline-invoicer/undo/apply-payment-202-abcdef012345.json"
        );
        save(
            &vault,
            &UndoRecord::of(&applied("2026/7", "ffff"), "2026-11-18"),
        )
        .expect("saves");
        let found = records(&vault, "202");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].1.undo().original_text, "2026-11-12 WIRE\n");
        assert_eq!(found[0].1.deposit_description, "WIRE");
        let other = records(&vault, "2026/7");
        assert_eq!(
            other[0].0.as_str(),
            ".ledgeline-invoicer/undo/apply-payment-2026_7-ffff.json"
        );
        remove(&vault, &first).expect("removes");
        assert!(records(&vault, "202").is_empty());
        assert!(records(&MemVault::new(), "202").is_empty());
    }
}
