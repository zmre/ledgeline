//! The commands that write documents (ARCHITECTURE §5.2, §5.5): `new`,
//! `set` and `recalc`. Each is one library function the CLI and the API
//! share. They write markdown only, through the vault, verified and
//! conditional on the file being what was read; the books are never touched
//! (drafts never touch the books, I2).
//!
//! Two rules keep a document and its books from drifting apart before Phase
//! 2's send/void/rebook exist:
//!
//! - `set` refuses `status` (a lifecycle change: send and void) and `paid`
//!   (a mirror the reconciler writes, I7), and refuses a new `invoice_num`
//!   on anything but a draft (the number is how the books find it, §4.3).
//! - `recalc` over every document rewrites drafts only; a sent or void
//!   document is recalculated only when named.

use crate::catalog::{self, Catalog};
use crate::doc::{Invoice, InvoiceNumber, Status};
use crate::draft::{self, DraftError, DraftRow, NewInvoice};
use crate::frontmatter::{self, EditError, FieldEdit};
use crate::index::DocumentIndex;
use crate::problem::Problem;
use crate::table::TableError;
use crate::table::gfm::apply_edits;
use crate::table::money::plain;
use crate::table::words::Words;
use ledgeline_core::vault::{Revision, Vault, VaultError, VaultPath};
use serde::Serialize;
use serde_json::Value;
use thiserror::Error;

/// Frontmatter keys `set` refuses, and why.
const PROTECTED: [(&str, &str); 2] = [
    (
        "status",
        "status changes are lifecycle steps: send and void (Phase 2) book them",
    ),
    (
        "paid",
        "paid is a mirror of the books, written only by the reconciler",
    ),
];

/// Why a write command refused. Nothing is written when one is returned.
#[derive(Debug, Error)]
pub enum WriteError {
    /// `key=value` doesn't read.
    #[error("{0:?} isn't key=value (or key:=JSON)")]
    BadAssignment(String),
    /// A `key:=` value isn't JSON.
    #[error("{key}: the value isn't JSON: {message}")]
    BadJson {
        /// The key.
        key: String,
        /// The parser's message.
        message: String,
    },
    /// The key is one `set` leaves alone.
    #[error("`{key}` can't be set: {reason}")]
    Protected {
        /// The key.
        key: String,
        /// Why.
        reason: &'static str,
    },
    /// A number change on a document that isn't a draft.
    #[error("`invoice_num` can't change once {number} is {status}: the books find it by number")]
    NumberLocked {
        /// The number.
        number: String,
        /// Its status.
        status: &'static str,
    },
    /// A number that can't be a tag value.
    #[error("`{0}` can't be an invoice number (no commas or line breaks)")]
    BadNumber(String),
    /// A number already used.
    #[error("invoice {number} already exists: {path}")]
    DuplicateNumber {
        /// The number.
        number: String,
        /// Where.
        path: String,
    },
    /// A `--row` that doesn't read.
    #[error("row {spec:?}: {reason}")]
    BadRow {
        /// The spec.
        spec: String,
        /// Why.
        reason: String,
    },
    /// A catalog code no catalog has.
    #[error("no catalog item has the code {0:?}")]
    UnknownItem(String),
    /// The frontmatter can't be edited safely.
    #[error(transparent)]
    Frontmatter(#[from] EditError),
    /// The bill table can't be read or computed.
    #[error(transparent)]
    Table(#[from] TableError),
    /// Creating or saving refused (verification, a concurrent change, …).
    #[error(transparent)]
    Draft(#[from] DraftError),
    /// The file isn't UTF-8 text.
    #[error("{0} isn't UTF-8 text")]
    NotText(VaultPath),
    /// The file can't be read or written.
    #[error(transparent)]
    Vault(#[from] VaultError),
}

fn read_text(vault: &dyn Vault, path: &VaultPath) -> Result<(String, Revision), WriteError> {
    let (bytes, revision) = vault.read_pinned(path, None)?;
    let text = String::from_utf8(bytes).map_err(|_| WriteError::NotText(path.clone()))?;
    Ok((text, revision))
}

/// Read `key=value` (text; a whole number becomes a YAML integer when it
/// reads back the same, so `terms=30` is a number and `invoice_num=0042`
/// stays text) or `key:=JSON` (any value: `tags:=["a","b"]`,
/// `flag:=true`).
///
/// # Errors
/// [`WriteError::BadAssignment`], [`WriteError::BadJson`].
pub fn assignment(text: &str) -> Result<(String, Value), WriteError> {
    let (key, value) = text
        .split_once('=')
        .ok_or_else(|| WriteError::BadAssignment(text.to_string()))?;
    if let Some(key) = key.strip_suffix(':') {
        let key = key.trim();
        if key.is_empty() {
            return Err(WriteError::BadAssignment(text.to_string()));
        }
        let parsed = serde_json::from_str(value).map_err(|e| WriteError::BadJson {
            key: key.to_string(),
            message: e.to_string(),
        })?;
        return Ok((key.to_string(), parsed));
    }
    let key = key.trim();
    if key.is_empty() {
        return Err(WriteError::BadAssignment(text.to_string()));
    }
    let number = value
        .parse::<i64>()
        .ok()
        .filter(|n| n.to_string() == value)
        .map(|n| Value::Number(n.into()));
    Ok((
        key.to_string(),
        number.unwrap_or_else(|| Value::String(value.to_string())),
    ))
}

/// What `set` did.
#[derive(Debug, Clone, Serialize)]
pub struct SetOutcome {
    /// The file.
    pub path: VaultPath,
    /// Its number after the edit.
    pub number: Option<String>,
    /// The keys whose values changed (set, added or removed).
    pub changed: Vec<String>,
    /// Whether the file was rewritten (`false` when nothing changed).
    pub written: bool,
    /// The document's own problems after the edit.
    pub problems: Vec<Problem>,
}

/// `set`: apply `edits` to the frontmatter of the document at `path`, by
/// key (§5.5), and save it.
///
/// The text written is verified as every save is, except that a document
/// whose bill table already failed verification (a hand edit `recalc`
/// hasn't fixed) can still have its frontmatter edited: `set` makes nothing
/// worse, and the frontmatter edit is itself verified key by key.
///
/// # Errors
/// [`WriteError`].
pub fn set(
    index: &mut DocumentIndex,
    vault: &dyn Vault,
    path: &VaultPath,
    edits: &[FieldEdit],
) -> Result<SetOutcome, WriteError> {
    let (source, revision) = read_text(vault, path)?;
    let words = &index.settings().words;
    let before = Invoice::parse(source.as_str(), words);
    let header = before.header().clone();
    for change in edits {
        let key = match change {
            FieldEdit::Set(key, _) | FieldEdit::Remove(key) => key.as_str(),
        };
        if let Some((key, reason)) = PROTECTED.iter().find(|(k, _)| *k == key) {
            return Err(WriteError::Protected {
                key: (*key).to_string(),
                reason,
            });
        }
        if key == "invoice_num" {
            check_new_number(index, path, &header, change)?;
        }
    }
    let text = frontmatter::edit(&source, edits)?;
    let after = Invoice::parse(text.as_str(), words);
    let changed: Vec<String> = edits
        .iter()
        .map(|c| match c {
            FieldEdit::Set(key, _) | FieldEdit::Remove(key) => key.clone(),
        })
        .filter(|key| before.frontmatter().get(key) != after.frontmatter().get(key))
        .fold(Vec::new(), |mut keys, key| {
            if !keys.contains(&key) {
                keys.push(key);
            }
            keys
        });
    let written = text != source;
    if written {
        let settings = index.settings();
        if draft::verify(&source, settings, index.money()).is_ok() {
            draft::save(index, vault, path, &text, &revision)?;
        } else {
            vault.replace(path, text.as_bytes(), &revision)?;
            index.refresh(vault, std::slice::from_ref(path));
        }
    }
    let doc = index.doc(path);
    Ok(SetOutcome {
        path: path.clone(),
        number: doc.and_then(|d| d.number()).map(str::to_string),
        changed,
        written,
        problems: doc.map(|d| d.problems.clone()).unwrap_or_default(),
    })
}

fn check_new_number(
    index: &DocumentIndex,
    path: &VaultPath,
    header: &crate::doc::Header,
    change: &FieldEdit,
) -> Result<(), WriteError> {
    let current = header.number.as_ref().map(InvoiceNumber::as_str);
    let wanted = match change {
        FieldEdit::Remove(_) => None,
        FieldEdit::Set(_, Value::String(s)) => Some(s.trim().to_string()),
        FieldEdit::Set(_, Value::Number(n)) => Some(n.to_string()),
        FieldEdit::Set(_, other) => return Err(WriteError::BadNumber(other.to_string())),
    };
    if wanted.as_deref() == current {
        return Ok(());
    }
    if let Some(status) = header.status.filter(|s| *s != Status::Draft) {
        return Err(WriteError::NumberLocked {
            number: current.unwrap_or("").to_string(),
            status: status.as_str(),
        });
    }
    let Some(wanted) = wanted else {
        return Ok(());
    };
    let number = InvoiceNumber::new(&wanted).ok_or(WriteError::BadNumber(wanted))?;
    match index
        .by_number(number.as_str())
        .into_iter()
        .find(|d| &d.path != path)
    {
        Some(other) => Err(WriteError::DuplicateNumber {
            number: number.to_string(),
            path: other.path.to_string(),
        }),
        None => Ok(()),
    }
}

/// One amount `recalc` rewrote (or would).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AmountChange {
    /// 1-based row.
    pub row: usize,
    /// 1-based line.
    pub line: usize,
    /// The row's description.
    pub description: String,
    /// The amount cell before.
    pub before: String,
    /// The amount cell after.
    pub after: String,
}

/// What `recalc` did to one document.
#[derive(Debug, Clone, Serialize)]
pub struct Recalculated {
    /// The file.
    pub path: VaultPath,
    /// Its number.
    pub number: Option<String>,
    /// Its status.
    pub status: Option<Status>,
    /// The amounts rewritten.
    pub changes: Vec<AmountChange>,
    /// Whether the file was rewritten (`false` for a dry run or no change).
    pub written: bool,
}

/// A document `recalc` left alone, or couldn't do.
#[derive(Debug, Clone, Serialize)]
pub struct NotRecalculated {
    /// The file.
    pub path: VaultPath,
    /// Its number.
    pub number: Option<String>,
    /// Why.
    pub reason: String,
}

/// What `recalc` did.
#[derive(Debug, Clone, Default, Serialize)]
pub struct RecalcReport {
    /// Documents whose amounts changed (or would, on a dry run).
    pub changed: Vec<Recalculated>,
    /// Documents with wrong amounts that weren't rewritten: sent or void
    /// ones when no document was named.
    pub skipped: Vec<NotRecalculated>,
    /// Documents whose table can't be computed.
    pub failed: Vec<NotRecalculated>,
}

/// Recompute one document's amounts and, unless `dry_run`, save them.
///
/// # Errors
/// [`WriteError`].
pub fn recalc_one(
    index: &mut DocumentIndex,
    vault: &dyn Vault,
    path: &VaultPath,
    dry_run: bool,
) -> Result<Recalculated, WriteError> {
    let (source, revision) = read_text(vault, path)?;
    let words = index.settings().words.clone();
    let invoice = Invoice::parse(source.as_str(), &words);
    let header = invoice.header().clone();
    let mut out = Recalculated {
        path: path.clone(),
        number: header.number.as_ref().map(ToString::to_string),
        status: header.status,
        changes: Vec::new(),
        written: false,
    };
    let Some(table) = invoice.bill_table().map_err(Clone::clone)? else {
        return Ok(out);
    };
    let recalc = table.recompute(&words, &index.money().for_table(table))?;
    if recalc.changes.is_empty() {
        return Ok(out);
    }
    out.changes = recalc
        .changes
        .iter()
        .map(|c| AmountChange {
            row: c.at.index + 1,
            line: c.at.line,
            description: plain(&c.at.description).trim().to_string(),
            before: c.before.trim().to_string(),
            after: c.after.trim().to_string(),
        })
        .collect();
    if !dry_run {
        let text = apply_edits(&source, &recalc.table.edits(&source)?);
        draft::save(index, vault, path, &text, &revision)?;
        out.written = true;
    }
    Ok(out)
}

/// `recalc`: the documents at `targets`, any status; or, with none, every
/// document whose written amounts disagree, drafts only (the rest are
/// reported as skipped).
#[must_use]
pub fn recalc(
    index: &mut DocumentIndex,
    vault: &dyn Vault,
    targets: &[VaultPath],
    dry_run: bool,
) -> RecalcReport {
    let mut report = RecalcReport::default();
    let chosen: Vec<VaultPath> = if targets.is_empty() {
        let (drafts, others): (Vec<_>, Vec<_>) = index
            .docs()
            .filter(|d| d.summary.as_ref().is_some_and(|s| s.discrepancies > 0))
            .partition(|d| d.header.status == Some(Status::Draft));
        report.skipped = others
            .into_iter()
            .map(|d| NotRecalculated {
                path: d.path.clone(),
                number: d.number().map(str::to_string),
                reason: format!(
                    "it is {}; name it to recalculate it (then re-book it)",
                    d.header.status.map_or("not a draft", Status::as_str)
                ),
            })
            .collect();
        drafts.into_iter().map(|d| d.path.clone()).collect()
    } else {
        targets.to_vec()
    };
    for path in chosen {
        let number = index
            .doc(&path)
            .and_then(|d| d.number())
            .map(str::to_string);
        match recalc_one(index, vault, &path, dry_run) {
            Ok(done) if done.changes.is_empty() => {}
            Ok(done) => report.changed.push(done),
            Err(e) => report.failed.push(NotRecalculated {
                path,
                number,
                reason: e.to_string(),
            }),
        }
    }
    report
}

/// `text` split at each `|` that isn't escaped as `\|`.
fn split_unescaped(text: &str) -> Vec<String> {
    // A command-line argument can't hold NUL, so it can stand in for `\|`.
    const HOLD: char = '\u{0}';
    text.replace("\\|", &HOLD.to_string())
        .split('|')
        .map(|part| part.replace(HOLD, "|"))
        .collect()
}

/// A bill-table row from `DESC|QTY|RATE|ACCOUNT` (trailing fields may be
/// left off; `\|` is a literal pipe):
///
/// - `Senior architect hours|10|$800|revenue:consulting` — an item;
/// - `Discount||-10%` — an adjustment (no quantity, a percentage);
/// - `Less deposit||-$5,000|liabilities:customer-deposits` — with no
///   quantity, a money rate is the row's amount (a fixed adjustment when
///   signed, an item as written otherwise: `Shipping||$12`);
/// - `Subtotal` — a subtotal or total word alone writes a total row.
///
/// # Errors
/// [`WriteError::BadRow`].
pub fn row_spec(spec: &str, words: &Words) -> Result<DraftRow, WriteError> {
    let bad = |reason: &str| WriteError::BadRow {
        spec: spec.to_string(),
        reason: reason.to_string(),
    };
    let fields = split_unescaped(spec);
    if fields.len() > 4 {
        return Err(bad("too many fields: DESC|QTY|RATE|ACCOUNT"));
    }
    let field = |i: usize| {
        fields
            .get(i)
            .map(|f| f.trim().to_string())
            .filter(|f| !f.is_empty())
    };
    let description = field(0).unwrap_or_default();
    let (quantity, rate, books) = (field(1), field(2), field(3));
    let is_total = words.subtotal.matches(&description) || words.total.matches(&description);
    if is_total && quantity.is_none() && rate.is_none() && books.is_none() {
        return Ok(DraftRow {
            rate: Some(format!("**{description}**")),
            ..DraftRow::default()
        });
    }
    if description.is_empty() {
        return Err(bad("a row needs a description"));
    }
    let (rate, amount) = match (&quantity, rate) {
        (None, Some(rate)) if !rate.ends_with('%') => (None, Some(rate)),
        (_, rate) => (rate, None),
    };
    Ok(DraftRow {
        description,
        quantity,
        rate,
        amount,
        books,
        ..DraftRow::default()
    })
}

/// A row from a catalog item: `CODE` or `CODE:QTY` (quantity 1 when
/// unsaid), from the first catalog that has the code.
///
/// # Errors
/// [`WriteError::UnknownItem`], [`WriteError::BadRow`].
pub fn catalog_row(catalogs: &[Catalog], spec: &str) -> Result<DraftRow, WriteError> {
    let (code, quantity) = spec.rsplit_once(':').unwrap_or((spec, "1"));
    let (code, quantity) = (code.trim(), quantity.trim());
    if code.is_empty() || quantity.is_empty() {
        return Err(WriteError::BadRow {
            spec: spec.to_string(),
            reason: "write CODE or CODE:QTY".to_string(),
        });
    }
    catalogs
        .iter()
        .find_map(|c| c.item(code))
        .map(|item| DraftRow::from_catalog(item, quantity))
        .ok_or_else(|| WriteError::UnknownItem(code.to_string()))
}

/// Every catalog in the books folder (problems are `check`'s to report).
#[must_use]
pub fn catalogs(index: &DocumentIndex, vault: &dyn Vault) -> Vec<Catalog> {
    catalog::load(vault, index.site(), &index.settings().words, index.money()).0
}

/// What `new` made.
#[derive(Debug, Clone, Serialize)]
pub struct Created {
    /// The file.
    pub path: VaultPath,
    /// Its number.
    pub number: InvoiceNumber,
    /// The amount due.
    pub total: Option<String>,
    /// Whether it was written (`false` for a dry run).
    pub written: bool,
    /// The text (always on a dry run).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

/// `new`: render a draft (numbered over the files and `booked`) and, unless
/// `dry_run`, create it.
///
/// # Errors
/// [`WriteError::Draft`].
pub fn create(
    index: &mut DocumentIndex,
    vault: &dyn Vault,
    new: &NewInvoice,
    booked: &[&str],
    dry_run: bool,
) -> Result<Created, WriteError> {
    let made = draft::render(index, vault, new, booked)?;
    let total = Invoice::parse(made.text.as_str(), &index.settings().words)
        .summary(&index.settings().words, index.money())
        .map(|s| s.total_text);
    if !dry_run {
        draft::create(index, vault, &made)?;
    }
    Ok(Created {
        path: made.path,
        number: made.number,
        total,
        written: !dry_run,
        text: dry_run.then_some(made.text),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::Money;
    use crate::settings::Settings;
    use ledgeline_core::vault::MemVault;

    const DRAFT: &str = "---\ntitle: A\ntype: invoice\ntemplate: /t.html\nstatus: draft\ninvoice_num: 7\ndate: 2026-11-01\ncustomer: \"[[C]]\"\n---\n\n| Description | Qty | Rate | Amount |\n|---|--:|--:|--:|\n| Work | 2 | $10 | $5 |\n| | | **Total due** | **$5** |\n";

    fn open(files: &[(&str, &str)]) -> (MemVault, DocumentIndex) {
        let vault = files
            .iter()
            .fold(MemVault::new(), |v, (p, t)| v.with_file(p, t));
        let index = DocumentIndex::open(&vault, Settings::default(), Money::default());
        (vault, index)
    }

    fn path(p: &str) -> VaultPath {
        VaultPath::new(p).expect("path")
    }

    #[test]
    fn assignments_type_whole_numbers_and_json() {
        assert_eq!(
            assignment("terms=30").expect("ok"),
            ("terms".into(), Value::from(30))
        );
        assert_eq!(
            assignment("invoice_num=0042").expect("ok").1,
            Value::from("0042")
        );
        assert_eq!(
            assignment("reference=PO #1=2").expect("ok"),
            ("reference".into(), Value::from("PO #1=2"))
        );
        assert_eq!(
            assignment("flags:=[1,\"a\"]").expect("ok").1,
            serde_json::json!([1, "a"])
        );
        assert!(assignment("nokey").is_err());
        assert!(assignment("=x").is_err());
        assert!(assignment("x:={").is_err());
    }

    #[test]
    fn set_edits_by_key_and_guards_the_lifecycle() {
        let (vault, mut index) = open(&[("a.md", DRAFT)]);
        let a = path("a.md");
        let done = set(
            &mut index,
            &vault,
            &a,
            &[FieldEdit::Set("reference".into(), Value::from("PO 9"))],
        )
        .expect("sets");
        assert!(done.written);
        assert_eq!(done.changed, ["reference"]);
        let text = vault.read_to_string(&a).expect("reads");
        assert!(text.contains("reference: PO 9\n"), "{text}");
        // The table's wrong amount is untouched by a frontmatter edit.
        assert!(text.contains("| $5 |"));
        let again = set(
            &mut index,
            &vault,
            &a,
            &[FieldEdit::Set("reference".into(), Value::from("PO 9"))],
        )
        .expect("no-op");
        assert!(!again.written && again.changed.is_empty());
        for key in ["status", "paid"] {
            let refused = set(
                &mut index,
                &vault,
                &a,
                &[FieldEdit::Set(key.into(), Value::from("sent"))],
            );
            assert!(
                matches!(refused, Err(WriteError::Protected { .. })),
                "{key}"
            );
        }
        let renumbered = set(
            &mut index,
            &vault,
            &a,
            &[FieldEdit::Set("invoice_num".into(), Value::from(8))],
        )
        .expect("a draft renumbers");
        assert_eq!(renumbered.number.as_deref(), Some("8"));
    }

    #[test]
    fn set_refuses_numbers_in_use_and_on_sent_documents() {
        let sent = DRAFT
            .replace("status: draft", "status: sent")
            .replace("invoice_num: 7", "invoice_num: 9");
        let (vault, mut index) = open(&[("a.md", DRAFT), ("b.md", &sent)]);
        let taken = set(
            &mut index,
            &vault,
            &path("a.md"),
            &[FieldEdit::Set("invoice_num".into(), Value::from(9))],
        );
        assert!(matches!(taken, Err(WriteError::DuplicateNumber { .. })));
        let locked = set(
            &mut index,
            &vault,
            &path("b.md"),
            &[FieldEdit::Set("invoice_num".into(), Value::from(10))],
        );
        assert!(matches!(locked, Err(WriteError::NumberLocked { .. })));
        assert_eq!(vault.read_to_string(&path("b.md")).expect("reads"), sent);
    }

    #[test]
    fn recalc_rewrites_drafts_and_skips_sent_unless_named() {
        let sent = DRAFT
            .replace("status: draft", "status: sent")
            .replace("invoice_num: 7", "invoice_num: 9");
        let (vault, mut index) = open(&[("a.md", DRAFT), ("b.md", &sent)]);
        let dry = recalc(&mut index, &vault, &[], true);
        assert_eq!(dry.changed.len(), 1);
        assert!(!dry.changed[0].written);
        assert_eq!(vault.read_to_string(&path("a.md")).expect("reads"), DRAFT);

        let report = recalc(&mut index, &vault, &[], false);
        assert_eq!(report.changed.len(), 1);
        assert_eq!(report.changed[0].changes[0].after, "$20.00");
        assert_eq!(report.skipped.len(), 1);
        assert_eq!(report.skipped[0].path, path("b.md"));
        let text = vault.read_to_string(&path("a.md")).expect("reads");
        assert!(text.contains("$20.00"), "{text}");

        let named = recalc(&mut index, &vault, &[path("b.md")], false);
        assert_eq!(named.changed.len(), 1);
        assert!(recalc(&mut index, &vault, &[], false).changed.is_empty());
    }

    #[test]
    fn row_specs() {
        let words = Words::default();
        let row = row_spec("Senior hours|10|$800|revenue:consulting", &words).expect("item");
        assert_eq!(row.quantity.as_deref(), Some("10"));
        assert_eq!(row.rate.as_deref(), Some("$800"));
        assert_eq!(row.books.as_deref(), Some("revenue:consulting"));
        let discount = row_spec("Discount||-10%", &words).expect("adjustment");
        assert_eq!(discount.rate.as_deref(), Some("-10%"));
        assert_eq!(discount.amount, None);
        let deposit =
            row_spec("Less deposit||-$5,000|liabilities:deposits", &words).expect("fixed");
        assert_eq!(deposit.rate, None);
        assert_eq!(deposit.amount.as_deref(), Some("-$5,000"));
        let total = row_spec("subtotal", &words).expect("total");
        assert_eq!(total.rate.as_deref(), Some("**subtotal**"));
        assert_eq!(
            row_spec(r"A \| B|1|$2", &words).expect("pipe").description,
            "A | B"
        );
        assert!(row_spec("|1|$2", &words).is_err());
        assert!(row_spec("a|1|2|b|c", &words).is_err());
    }
}
