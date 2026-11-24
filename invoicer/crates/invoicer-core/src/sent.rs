//! The `sent_hash` mirror (ARCHITECTURE §5.3, PLAN 3c): what a sent
//! invoice's file said when its PDF was last kept, so a later export can warn
//! that the PDF differs from what the customer has.
//!
//! **What is hashed.** The whole file, byte for byte, except the two mirror
//! keys that change without the invoice changing: `paid` (the reconciler's)
//! and `sent_hash` itself. A paid invoice is still the invoice that was sent;
//! a new row, rate, address or note is not. The template is not part of it:
//! the hash is of the invoice, not of its styling.
//!
//! **The hash.** FNV-1a, 64-bit (the same function as Ledgeline's
//! `content_hash`), written as 16 lowercase hex digits. It detects edits; it
//! is not a signature, and the file is the user's to edit by hand anyway.
//!
//! The mirror is written only by a kept export ([`record`]), conditional on
//! the file still hashing as it did when the export read it: an edit made
//! while the PDF was printing leaves the old value, so the next export warns.

use crate::doc::Invoice;
use crate::frontmatter::{self, FieldEdit};
use crate::index::DocumentIndex;
use ledgeline_core::vault::{Vault, VaultPath};
use serde::Serialize;
use serde_json::Value;

use crate::edit::WriteError;

/// The frontmatter key.
pub const SENT_HASH_KEY: &str = "sent_hash";

/// The keys left out of the hash: mirrors that move without the invoice
/// changing.
const UNHASHED: [&str; 2] = ["paid", SENT_HASH_KEY];

/// The hash of `source` as a kept export records it (see the module docs).
///
/// A file whose frontmatter the editor can't change safely is hashed whole:
/// [`record`] can't write its mirror either, so no comparison is ever made
/// against a hash taken the other way.
#[must_use]
pub fn hash(source: &str) -> String {
    let edits: Vec<FieldEdit> = UNHASHED
        .iter()
        .map(|key| FieldEdit::Remove((*key).to_string()))
        .collect();
    let basis = frontmatter::edit(source, &edits).unwrap_or_else(|_| source.to_string());
    format!("{:016x}", fnv1a_64(basis.as_bytes()))
}

/// The `sent_hash` the file holds, if any.
#[must_use]
pub fn recorded(invoice: &Invoice) -> Option<&str> {
    invoice
        .frontmatter()
        .str(SENT_HASH_KEY)
        .map(str::trim)
        .filter(|h| !h.is_empty())
}

/// Whether the file changed since its PDF was last kept: `None` when no
/// export was ever recorded.
#[must_use]
pub fn changed_since_sent(invoice: &Invoice) -> Option<bool> {
    recorded(invoice).map(|kept| kept != hash(invoice.source()))
}

/// What [`record`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Recorded {
    /// `sent_hash` was written.
    Written,
    /// It already said so.
    Unchanged,
    /// The file changed since the export read it; nothing was written.
    Stale,
}

/// Write `exported` (the [`hash`] of the file the PDF was made from) as the
/// document's `sent_hash`, unless the file no longer hashes to it. Only the
/// one key changes; the write is conditional on the revision read.
///
/// # Errors
/// [`WriteError`]: the file can't be read, isn't text, its frontmatter
/// can't be edited safely, or it changed between reading and writing.
pub fn record(
    index: &mut DocumentIndex,
    vault: &dyn Vault,
    path: &VaultPath,
    exported: &str,
) -> Result<Recorded, WriteError> {
    let (bytes, revision) = vault.read_pinned(path, None)?;
    let source = String::from_utf8(bytes).map_err(|_| WriteError::NotText(path.clone()))?;
    if hash(&source) != exported {
        return Ok(Recorded::Stale);
    }
    let invoice = Invoice::parse(source.as_str(), &index.settings().words);
    if recorded(&invoice) == Some(exported) {
        return Ok(Recorded::Unchanged);
    }
    let text = frontmatter::set(&source, SENT_HASH_KEY, Value::String(exported.to_string()))?;
    vault.replace(path, text.as_bytes(), &revision)?;
    index.refresh(vault, std::slice::from_ref(path));
    Ok(Recorded::Written)
}

/// FNV-1a, 64-bit: the offset basis `0xcbf29ce484222325`; for each byte,
/// XOR it in, then multiply by `0x100000001b3` modulo 2^64.
fn fnv1a_64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::table::words::Words;

    const SENT: &str =
        "---\ntype: invoice\ninvoice_num: 105\nstatus: sent\npaid: no\n---\n\nHello.\n";

    #[test]
    fn fnv_matches_the_published_vectors() {
        assert_eq!(fnv1a_64(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a_64(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a_64(b"foobar"), 0x8594_4171_f739_67e8);
    }

    #[test]
    fn the_hash_ignores_the_mirrors_and_nothing_else() {
        let base = hash(SENT);
        assert_eq!(base.len(), 16);
        assert!(
            base.chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        );
        // `paid` moves with the books, `sent_hash` is the mirror itself.
        assert_eq!(hash(&SENT.replace("paid: no", "paid: 2026-12-01")), base);
        assert_eq!(hash(&SENT.replace("paid: no\n", "")), base);
        let with_mirror = SENT.replace("paid: no\n", "paid: no\nsent_hash: 0123456789abcdef\n");
        assert_eq!(hash(&with_mirror), base);
        // Anything else is the invoice.
        assert_ne!(hash(&SENT.replace("Hello.", "Hello!")), base);
        assert_ne!(hash(&SENT.replace("status: sent", "status: void")), base);
        assert_ne!(
            hash(&SENT.replace("paid: no\n", "paid: no\nreference: PO 1\n")),
            base
        );
    }

    #[test]
    fn changed_since_sent_compares_with_the_recorded_hash() {
        let words = Words::default();
        assert_eq!(changed_since_sent(&Invoice::parse(SENT, &words)), None);
        let kept = SENT.replace(
            "paid: no\n",
            &format!("paid: no\nsent_hash: {}\n", hash(SENT)),
        );
        assert_eq!(
            changed_since_sent(&Invoice::parse(kept.as_str(), &words)),
            Some(false)
        );
        let paid = kept.replace("paid: no", "paid: 2026-12-01");
        assert_eq!(
            changed_since_sent(&Invoice::parse(paid.as_str(), &words)),
            Some(false)
        );
        let edited = kept.replace("Hello.", "Hello, edited.");
        assert_eq!(
            changed_since_sent(&Invoice::parse(edited.as_str(), &words)),
            Some(true)
        );
    }
}
