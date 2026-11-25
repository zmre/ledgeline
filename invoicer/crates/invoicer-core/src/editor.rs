//! The invoice editor's model (invoicer PLAN 4a, ARCHITECTURE §6.3): a
//! document as an editable [`Form`], and the text a form makes of it.
//!
//! The app's editor never sends markdown: it sends the form, and the text is
//! made here, from the file it was read from, so everything the form doesn't
//! hold stays as it was:
//!
//! - **Frontmatter** changes by key ([`crate::frontmatter::edit`]): only the
//!   header fields whose value changed, and the custom fields the form added,
//!   changed or dropped. Comments, order, quoting and unknown keys stay.
//!   Changing the customer re-snapshots `invoicee_*` from the new customer
//!   note (ARCHITECTURE §3.2), as `new` does.
//! - **The bill table** is the form's grid (its columns as headers, its rows
//!   as raw markdown cells), written aligned with every computed amount
//!   recomputed, as every save does (§3.3). An unchanged grid whose amounts
//!   agree keeps its bytes.
//! - **Prose** above and below the table is replaced only when it changed
//!   (compared trimmed, as `show` gives it).
//!
//! `status` is never in the form: it is a lifecycle step (send, void).
//! The number follows `set`'s rules (a draft's only, never one in use).
//! Saving is [`save`]: conditional on the revision the editor read, and
//! verified, like every write (§5.2).

use crate::customers::{self, CustomerLookup};
use crate::doc::{Invoice, InvoiceNumber, Status};
use crate::draft::{self, DraftError};
use crate::edit::{self, WriteError};
use crate::frontmatter::{self, EditError, FieldEdit};
use crate::index::DocumentIndex;
use crate::table::books::is_iso_date;
use crate::table::columns::{Columns, Role};
use crate::table::gfm::{self, Grid, Table, escape_cell};
use crate::table::words::Words;
use crate::table::{BillTable, TableError};
use ledgeline_core::vault::{Revision, Vault, VaultError, VaultPath};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

/// The frontmatter keys the invoicer owns (the header, the snapshots and the
/// mirrors). Every other key is a custom field ([`Form::fields`]).
pub const KNOWN_KEYS: [&str; 26] = [
    "title",
    "type",
    "template",
    "status",
    "invoice_num",
    "date",
    "terms",
    "due_date",
    "reference",
    "customer",
    "invoicee_name",
    "invoicee_email",
    "invoicee_phone",
    "invoicee_address",
    "invoicer",
    "invoicer_name",
    "invoicer_email",
    "invoicer_phone",
    "invoicer_address",
    "invoicer_logo",
    "paid",
    "sent_date",
    "sent_via",
    "void_date",
    "deposit_for",
    "sent_hash",
];

/// A document as the editor edits it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Form {
    /// `customer:`, as written (`"[[Customer X]]"`); a title works too.
    pub customer: String,
    /// `template:`.
    pub template: Option<String>,
    /// `invoice_num`.
    pub number: String,
    /// `date`.
    pub date: String,
    /// `terms` (net days).
    pub terms: Option<u32>,
    /// `due_date`; `None` writes `date + terms` (or nothing without terms).
    pub due_date: Option<String>,
    /// `reference`.
    pub reference: Option<String>,
    /// `deposit_for`.
    pub deposit_for: Option<String>,
    /// Custom fields, in file order.
    #[serde(default)]
    pub fields: Vec<CustomField>,
    /// The bill table; `None` leaves the file's as it is.
    pub table: Option<GridForm>,
    /// Prose above the bill table.
    #[serde(default)]
    pub prose_before: String,
    /// Prose below it (supporting tables included).
    #[serde(default)]
    pub prose_after: String,
}

/// One custom frontmatter field.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CustomField {
    /// The key.
    pub key: String,
    /// The value, as JSON (a YAML list or map arrives as one, unchanged).
    pub value: Value,
}

/// The bill table as a grid: headers and raw markdown cells.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GridForm {
    /// The columns, in order.
    pub columns: Vec<ColumnForm>,
    /// The body rows, one cell per column (raw markdown: `**Subtotal**`,
    /// `-10%`, `$800`). Amount cells of computed rows are rewritten.
    pub rows: Vec<Vec<String>>,
}

/// One column: its header, and the role the header gives it (informative:
/// the role is always read from the header again).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ColumnForm {
    /// The header text, as written (`*Account*` for the books column).
    pub header: String,
    /// `description`, `quantity`, `rate`, `amount`, `date`, `unit`, `item`,
    /// `books` or `other`.
    #[serde(default)]
    pub role: String,
}

/// Why a form can't become the document's text. Nothing is written when one
/// is returned.
#[derive(Debug, Error)]
pub enum EditorError {
    /// A field's value is unusable.
    #[error("{field}: {reason}")]
    Invalid {
        /// The field (`date`, `fields[2]`, `table`).
        field: String,
        /// Why.
        reason: String,
    },
    /// The customer names no customer note.
    #[error("{0}")]
    Customer(String),
    /// The file changed since the editor read it.
    #[error("the file changed on disk since it was opened; reload it or keep your version")]
    Changed {
        /// The file's current revision token.
        current: String,
    },
    /// The document's status doesn't allow this edit.
    #[error("invoice {number} is {status}: {hint}")]
    Status {
        /// The number.
        number: String,
        /// `sent` or `void`.
        status: &'static str,
        /// What to do instead.
        hint: &'static str,
    },
    /// The number or another frontmatter edit was refused.
    #[error(transparent)]
    Write(#[from] WriteError),
    /// The frontmatter can't be edited safely.
    #[error(transparent)]
    Frontmatter(#[from] EditError),
    /// The table doesn't read.
    #[error(transparent)]
    Table(#[from] TableError),
    /// Saving refused (verification, a concurrent change).
    #[error(transparent)]
    Draft(#[from] DraftError),
    /// The file can't be read or written.
    #[error(transparent)]
    Vault(#[from] VaultError),
}

impl EditorError {
    fn invalid(field: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::Invalid {
            field: field.into(),
            reason: reason.into(),
        }
    }

    /// A short, stable code for the API (`changed`, `invalid`, …).
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::Changed { .. }
            | Self::Draft(DraftError::Vault(VaultError::Changed { .. }))
            | Self::Vault(VaultError::Changed { .. }) => "changed",
            Self::Status { status: "void", .. } => "void",
            Self::Status { .. } => "confirm-rebook",
            Self::Write(
                WriteError::NumberLocked { .. }
                | WriteError::DuplicateNumber { .. }
                | WriteError::BadNumber(_)
                | WriteError::Protected { .. },
            ) => "refused",
            Self::Draft(DraftError::Verify(_)) | Self::Table(_) | Self::Frontmatter(_) => "invalid",
            Self::Invalid { .. } | Self::Customer(_) => "bad-request",
            Self::Write(_) | Self::Draft(_) | Self::Vault(_) => "storage",
        }
    }
}

pub(crate) fn role_name(role: Role) -> &'static str {
    match role {
        Role::Description => "description",
        Role::Quantity => "quantity",
        Role::Rate => "rate",
        Role::Amount => "amount",
        Role::Date => "date",
        Role::Unit => "unit",
        Role::Item => "item",
        Role::Books => "books",
        Role::Other => "other",
    }
}

/// The text of a header field: a string, or a number as the file writes it.
fn field_text(invoice: &Invoice, key: &str) -> Option<String> {
    match invoice.frontmatter().get(key)? {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
    .filter(|s| !s.trim().is_empty())
}

/// The editor's form of `invoice`.
#[must_use]
pub fn form_of(invoice: &Invoice) -> Form {
    let header = invoice.header();
    let fields = frontmatter::key_lines(invoice.source())
        .unwrap_or_default()
        .into_iter()
        .map(|k| k.key)
        .filter(|key| !KNOWN_KEYS.contains(&key.as_str()))
        .filter_map(|key| {
            let value = invoice.frontmatter().get(&key)?.clone();
            Some(CustomField { key, value })
        })
        .collect();
    let table = invoice.bill_table().ok().flatten().map(|table| {
        let grid = &table.table().grid;
        GridForm {
            columns: grid
                .header
                .iter()
                .enumerate()
                .map(|(col, header)| ColumnForm {
                    header: header.clone(),
                    role: role_name(table.columns().role(col)).to_string(),
                })
                .collect(),
            rows: grid.rows.clone(),
        }
    });
    Form {
        customer: field_text(invoice, "customer").unwrap_or_default(),
        template: header.template.clone(),
        number: header
            .number
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_default(),
        date: header.date.clone().unwrap_or_default(),
        terms: header.terms.and_then(|t| u32::try_from(t).ok()),
        due_date: header.due_date.clone(),
        reference: header.reference.clone(),
        deposit_for: header.deposit_for.clone(),
        fields,
        table,
        prose_before: invoice.prose_before().trim().to_string(),
        prose_after: invoice.prose_after().trim().to_string(),
    }
}

fn blank(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// A one-line text field: `Set` when it differs, `Remove` when emptied.
fn text_edit(edits: &mut Vec<FieldEdit>, invoice: &Invoice, key: &str, wanted: Option<&str>) {
    let wanted = blank(wanted);
    if wanted == field_text(invoice, key) {
        return;
    }
    edits.push(match wanted {
        Some(text) => FieldEdit::Set(key.to_string(), Value::String(text)),
        None => FieldEdit::Remove(key.to_string()),
    });
}

/// Whether `key` can be a custom field the form writes.
fn custom_key(key: &str) -> bool {
    let mut chars = key.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// The frontmatter edits `form` makes to the document at `path`.
fn header_edits(
    index: &DocumentIndex,
    path: &VaultPath,
    invoice: &Invoice,
    form: &Form,
) -> Result<Vec<FieldEdit>, EditorError> {
    let words = &index.settings().words;
    let mut edits = Vec::new();

    // The number: `set`'s rules (a draft's only; never one in use).
    let number = form.number.trim();
    let current = invoice.header().number.as_ref().map(InvoiceNumber::as_str);
    if Some(number) != current {
        let parsed = InvoiceNumber::new(number)
            .ok_or_else(|| EditorError::invalid("number", "it can't be an invoice number"))?;
        let change = FieldEdit::Set("invoice_num".to_string(), draft::number_value(&parsed));
        edit::check_new_number(index, path, invoice.header(), &change)?;
        edits.push(change);
    }

    // The dates.
    let date = form.date.trim();
    if !is_iso_date(date) {
        return Err(EditorError::invalid("date", "not a date (YYYY-MM-DD)"));
    }
    text_edit(&mut edits, invoice, "date", Some(date));
    let terms = form.terms.map(i64::from);
    if terms != invoice.header().terms {
        edits.push(match form.terms {
            Some(t) => FieldEdit::Set("terms".to_string(), Value::Number(t.into())),
            None => FieldEdit::Remove("terms".to_string()),
        });
    }
    let due = match blank(form.due_date.as_deref()) {
        Some(due) if is_iso_date(&due) => Some(due),
        Some(_) => return Err(EditorError::invalid("due_date", "not a date (YYYY-MM-DD)")),
        None => form
            .terms
            .map(|t| crate::doc::due_from_terms(date, i64::from(t))),
    };
    text_edit(&mut edits, invoice, "due_date", due.as_deref());

    // The customer, and its snapshot when it changed.
    let customer = form.customer.trim();
    if customer.is_empty() {
        return Err(EditorError::invalid("customer", "choose a customer"));
    }
    if Some(customer) != field_text(invoice, "customer").as_deref().map(str::trim) {
        let site = index.site();
        let found = customers::find_customer(site, path, customer, words).map_err(|e| {
            EditorError::Customer(match e {
                CustomerLookup::NotFound => format!("no customer note is called {customer}"),
                CustomerLookup::NotACustomer(p) => {
                    format!("{p} isn't a person or organization tagged as a customer")
                }
            })
        })?;
        let bill_to = found.billing_contact(site, words);
        edits.push(FieldEdit::Set(
            "customer".to_string(),
            Value::String(found.wikilink()),
        ));
        text_edit(&mut edits, invoice, "invoicee_name", Some(&bill_to.name));
        text_edit(
            &mut edits,
            invoice,
            "invoicee_email",
            bill_to.email.as_deref(),
        );
        text_edit(
            &mut edits,
            invoice,
            "invoicee_phone",
            bill_to.phone.as_deref(),
        );
        match bill_to.address.as_deref() {
            Some(address) => edits.push(FieldEdit::Set(
                "invoicee_address".to_string(),
                draft::block_text(address),
            )),
            None => text_edit(&mut edits, invoice, "invoicee_address", None),
        }
    }

    text_edit(&mut edits, invoice, "template", form.template.as_deref());
    text_edit(&mut edits, invoice, "reference", form.reference.as_deref());
    text_edit(
        &mut edits,
        invoice,
        "deposit_for",
        form.deposit_for.as_deref(),
    );

    // Custom fields: set what changed, remove what the form dropped.
    for (at, field) in form.fields.iter().enumerate() {
        let key = field.key.trim();
        if KNOWN_KEYS.contains(&key) {
            return Err(EditorError::invalid(
                format!("fields[{at}]"),
                format!("`{key}` is an invoice field, not a custom one"),
            ));
        }
        if !custom_key(key) {
            return Err(EditorError::invalid(
                format!("fields[{at}]"),
                format!("`{key}` can't be a field name (letters, digits, _ and -)"),
            ));
        }
        if form.fields[..at].iter().any(|f| f.key.trim() == key) {
            return Err(EditorError::invalid(
                format!("fields[{at}]"),
                format!("`{key}` is there twice"),
            ));
        }
        if invoice.frontmatter().get(key) != Some(&field.value) {
            edits.push(FieldEdit::Set(key.to_string(), field.value.clone()));
        }
    }
    let kept: Vec<&str> = form.fields.iter().map(|f| f.key.trim()).collect();
    edits.extend(
        invoice
            .frontmatter()
            .fields
            .keys()
            .filter(|key| !KNOWN_KEYS.contains(&key.as_str()) && !kept.contains(&key.as_str()))
            .map(|key| FieldEdit::Remove(key.clone())),
    );
    Ok(edits)
}

/// A cell as a table can hold it: one line, pipes escaped.
fn cell_text(text: &str) -> String {
    escape_cell(&text.replace(['\r', '\n'], " "))
}

/// The bill table `grid` makes, every computed amount recomputed, written
/// aligned; `None` when it is the table as written (and its amounts agree).
fn table_text(
    index: &DocumentIndex,
    written: Option<&BillTable>,
    grid: &GridForm,
    words: &Words,
) -> Result<Option<String>, EditorError> {
    let header: Vec<String> = grid
        .columns
        .iter()
        .map(|c| c.header.trim().to_string())
        .collect();
    if header.is_empty() {
        return Err(EditorError::invalid("table", "it has no columns"));
    }
    let width = header.len();
    let mut rows = Vec::with_capacity(grid.rows.len());
    for (at, row) in grid.rows.iter().enumerate() {
        if row.iter().skip(width).any(|cell| !cell.trim().is_empty()) {
            return Err(EditorError::invalid(
                format!("table row {}", at + 1),
                format!("{} cells, but the table has {width} columns", row.len()),
            ));
        }
        rows.push(
            (0..width)
                .map(|col| row.get(col).map_or_else(String::new, |c| cell_text(c)))
                .collect::<Vec<_>>(),
        );
    }
    let same_header = written.is_some_and(|t| t.table().grid.header == header);
    let roles = Columns::recognize(&header, words)
        .map_err(|problem| EditorError::invalid("table", problem.to_string()))?
        .ok_or_else(|| {
            EditorError::invalid("table", "it needs an amount column (Amount, Total)")
        })?;
    let (alignments, layout) = match written {
        Some(t) if same_header => (t.table().grid.alignments.clone(), t.table().layout.clone()),
        _ => (
            roles
                .roles()
                .iter()
                .map(|r| r.default_alignment())
                .collect(),
            gfm::Layout::fresh(),
        ),
    };
    let unchanged = same_header && written.is_some_and(|t| t.table().grid.rows == rows);
    let table = Table {
        grid: Grid {
            header,
            alignments,
            rows,
        },
        layout,
        source: None,
    };
    let bill = BillTable::from_table(table, words)?.ok_or_else(|| {
        EditorError::invalid("table", "it needs an amount column (Amount, Total)")
    })?;
    let recalc = bill.recompute(words, &index.money().for_table(&bill))?;
    if unchanged && recalc.changes.is_empty() {
        return Ok(None);
    }
    Ok(Some(recalc.table.render()))
}

/// The text `form` makes of `invoice` (the file at `path`): see the module
/// docs. Only reads; [`save`] writes it.
///
/// # Errors
/// [`EditorError`] for a form that can't be written.
pub fn apply(
    index: &DocumentIndex,
    path: &VaultPath,
    invoice: &Invoice,
    form: &Form,
) -> Result<String, EditorError> {
    let words = &index.settings().words;
    let source = invoice.source();
    let layout = invoice.layout();
    let written = invoice.bill_table().map_err(Clone::clone)?;

    let before_range = layout.prose_before.clone();
    let before = if form.prose_before.trim() == invoice.prose_before().trim() {
        source[before_range.clone()].to_string()
    } else if form.prose_before.trim().is_empty() {
        "\n".to_string()
    } else {
        format!("\n{}\n\n", form.prose_before.trim())
    };
    let table = match (&form.table, &layout.bill_table) {
        (Some(grid), range) => match table_text(index, written, grid, words)? {
            Some(text) => Some(text),
            None => range.as_ref().map(|r| source[r.clone()].to_string()),
        },
        (None, Some(range)) => Some(source[range.clone()].to_string()),
        (None, None) => None,
    };
    let after = if layout.bill_table.is_none() {
        // No table in the file: its prose is all "before".
        String::new()
    } else if form.prose_after.trim() == invoice.prose_after().trim() {
        source[layout.prose_after.clone()].to_string()
    } else if form.prose_after.trim().is_empty() {
        "\n".to_string()
    } else {
        format!("\n\n{}\n", form.prose_after.trim())
    };
    let mut body = String::with_capacity(source.len() + 64);
    body.push_str(&source[..before_range.start]);
    body.push_str(&before);
    if let Some(table) = table {
        if layout.bill_table.is_none() && !body.ends_with("\n\n") {
            body.push('\n');
        }
        body.push_str(&table);
        if layout.bill_table.is_none() {
            body.push('\n');
        }
    }
    body.push_str(&after);
    if layout.bill_table.is_some() && !after.is_empty() && !body.ends_with('\n') {
        body.push('\n');
    }
    let edits = header_edits(index, path, invoice, form)?;
    Ok(frontmatter::edit(&body, &edits)?)
}

/// What [`save`] did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Saved {
    /// The file.
    pub path: VaultPath,
    /// Its number now.
    pub number: Option<String>,
    /// Its status.
    pub status: Option<Status>,
    /// Whether the file was rewritten (`false`: the form was the file).
    pub written: bool,
    /// The file's revision now (the editor's next `revision`).
    pub revision: String,
}

/// Save `form` over the document at `path`, if the file is still the
/// revision `expected` (a [`Revision::token`]) the editor read: verified,
/// atomic, conditional on that revision, re-indexed (§5.2).
///
/// A void document is refused; a sent one only with `sent_ok` (its books
/// must be re-booked after: the caller checks the closed period first and
/// re-books after, see the app).
///
/// # Errors
/// [`EditorError::Changed`] when the file changed since `expected`;
/// [`EditorError::Status`]; as [`apply`]; [`EditorError::Draft`] when it
/// wouldn't read back.
pub fn save(
    index: &mut DocumentIndex,
    vault: &dyn Vault,
    path: &VaultPath,
    form: &Form,
    expected: &str,
    sent_ok: bool,
) -> Result<Saved, EditorError> {
    let (bytes, revision) = vault.read_pinned(path, None)?;
    if revision.token() != expected {
        return Err(EditorError::Changed {
            current: revision.token(),
        });
    }
    let source = String::from_utf8(bytes)
        .map_err(|_| EditorError::invalid("file", format!("{path} isn't UTF-8 text")))?;
    let invoice = Invoice::parse(source.as_str(), &index.settings().words);
    let number = invoice
        .header()
        .number
        .as_ref()
        .map_or_else(String::new, ToString::to_string);
    match invoice.header().status {
        Some(Status::Void) => {
            return Err(EditorError::Status {
                number,
                status: "void",
                hint: "a void invoice stays as it was voided; reissue it as a new invoice",
            });
        }
        Some(Status::Sent) if !sent_ok => {
            return Err(EditorError::Status {
                number,
                status: "sent",
                hint: "its amount is booked, so saving re-books it; confirm to save and re-book",
            });
        }
        _ => {}
    }
    let text = apply(index, path, &invoice, form)?;
    let (written, revision) = if text == source {
        (false, revision)
    } else {
        (true, draft::save(index, vault, path, &text, &revision)?)
    };
    let doc = index.doc(path);
    Ok(Saved {
        path: path.clone(),
        number: doc.and_then(|d| d.number()).map(str::to_string),
        status: doc.and_then(|d| d.header.status),
        written,
        revision: revision.token(),
    })
}

/// The revision token of `text` (what [`save`] compares against), for an
/// editor that read the file itself.
#[must_use]
pub fn revision_of(text: &str) -> String {
    Revision::of_bytes(text.as_bytes()).token()
}

#[cfg(test)]
mod tests;
