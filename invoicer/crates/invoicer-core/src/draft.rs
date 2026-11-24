//! Creating a draft (ARCHITECTURE §5.2) and saving documents.
//!
//! A draft snapshots the customer and your company into frontmatter
//! (`invoicee_*`, `invoicer_*`), copies the customer's terms (with
//! `due_date = date + terms`), template and default snippets, builds the
//! bill table from rows or catalog items with every amount computed, and is
//! named by `[invoicer] filename`. Drafts never touch the books (I2).
//!
//! Writes go through ledgeline-core's [`Vault`]:
//!
//! - **Create** claims the name exclusively (an empty file, `O_EXCL`), then
//!   atomically replaces it, conditional on it still being that empty file.
//!   A crash leaves an empty note or the whole draft, never half of one, and
//!   two creators can't both win the name.
//! - **Save** replaces a document atomically only if it is still the
//!   revision the caller read (the content-hash check MBR's editor does).
//!
//! Every text written is verified first: it must read back as a document
//! with readable frontmatter and a bill table whose amounts agree.

use crate::catalog::CatalogItem;
use crate::customers::{self, CustomerLookup};
use crate::doc::{DocType, Invoice, InvoiceNumber};
use crate::frontmatter::{self, EditError};
use crate::index::DocumentIndex;
use crate::notes::Site;
use crate::numbering::NumberingError;
use crate::problem::{Problem, ProblemCode};
use crate::settings::Settings;
use crate::snippets;
use crate::table::books::is_iso_date;
use crate::table::gfm::{self, escape_cell};
use crate::table::words::Words;
use crate::table::{BillTable, TableError};
use crate::templates;
use ledgeline_core::vault::{Revision, Vault, VaultError, VaultPath};
use serde_json::Value;
use thiserror::Error;

/// One bill-table row to write. Blank fields are blank cells.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DraftRow {
    /// The catalog code.
    pub code: Option<String>,
    /// The description.
    pub description: String,
    /// The quantity.
    pub quantity: Option<String>,
    /// The unit.
    pub unit: Option<String>,
    /// The rate (money, or a signed percentage for an adjustment).
    pub rate: Option<String>,
    /// The amount, for rows that aren't computed (a fixed adjustment).
    pub amount: Option<String>,
    /// The books cell.
    pub books: Option<String>,
}

impl DraftRow {
    /// A row filled from a catalog item (§3.6): its code, description, unit,
    /// rate and resolved account, so the invoice is self-contained.
    #[must_use]
    pub fn from_catalog(item: &CatalogItem, quantity: &str) -> Self {
        Self {
            code: Some(item.code.clone()),
            description: item.description.clone(),
            quantity: Some(quantity.to_string()),
            unit: item.unit.clone(),
            rate: item.rate_text.clone(),
            amount: None,
            books: item.books_text.clone(),
        }
    }
}

/// What to create.
#[derive(Debug, Clone, PartialEq)]
pub struct NewInvoice {
    /// The document type (`invoice` today).
    pub doc_type: DocType,
    /// The customer: a `[[wikilink]]` or a title.
    pub customer: String,
    /// The template's `/`-rooted path; else the customer's
    /// `invoice_template`; else the first template for this type.
    pub template: Option<String>,
    /// The number; else the next one.
    pub number: Option<String>,
    /// The issue date (`today` is always a parameter: plan 26).
    pub date: String,
    /// Net days; else the customer's.
    pub terms: Option<u32>,
    /// `reference`.
    pub reference: Option<String>,
    /// The rows.
    pub rows: Vec<DraftRow>,
    /// Prose above the table.
    pub prose_before: Option<String>,
    /// Prose below the table; else the customer's default snippets.
    pub prose_after: Option<String>,
    /// Custom fields, written after the standard ones.
    pub extra: Vec<(String, Value)>,
}

impl NewInvoice {
    /// An invoice for `customer` dated `date`, everything else defaulted.
    #[must_use]
    pub fn new(customer: &str, date: &str) -> Self {
        Self {
            doc_type: DocType::Invoice,
            customer: customer.to_string(),
            template: None,
            number: None,
            date: date.to_string(),
            terms: None,
            reference: None,
            rows: Vec::new(),
            prose_before: None,
            prose_after: None,
            extra: Vec::new(),
        }
    }
}

/// A draft, rendered and verified, not yet written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Draft {
    /// Where it goes.
    pub path: VaultPath,
    /// Its number.
    pub number: InvoiceNumber,
    /// Its text.
    pub text: String,
}

/// Why a draft can't be made or a document can't be written. Nothing is
/// written when one is returned.
#[derive(Debug, Error)]
pub enum DraftError {
    /// The customer names no note.
    #[error("no customer note is called {0}")]
    UnknownCustomer(String),
    /// The customer names a note that isn't a customer.
    #[error("{0} isn't a person or organization tagged as a customer")]
    NotACustomer(VaultPath),
    /// The date isn't `YYYY-MM-DD`.
    #[error("`{0}` isn't a date (YYYY-MM-DD)")]
    BadDate(String),
    /// The number can't be a tag value.
    #[error("`{0}` can't be an invoice number (no commas or line breaks)")]
    BadNumber(String),
    /// The number is taken.
    #[error("invoice {number} already exists: {path}")]
    DuplicateNumber {
        /// The number.
        number: String,
        /// Where.
        path: VaultPath,
    },
    /// No next number.
    #[error(transparent)]
    Numbering(#[from] NumberingError),
    /// The filename pattern gives no usable path.
    #[error("the filename {0:?} can't be a file in the books folder")]
    BadPath(String),
    /// The frontmatter can't be written.
    #[error(transparent)]
    Frontmatter(#[from] EditError),
    /// The table can't be built.
    #[error(transparent)]
    Table(#[from] TableError),
    /// The text doesn't read back as a sound document.
    #[error("the document wouldn't read back cleanly: {}", .0.first().map_or_else(String::new, ToString::to_string))]
    Verify(Vec<Problem>),
    /// A file is already there.
    #[error("{0} already exists")]
    Exists(VaultPath),
    /// The vault refused (including [`VaultError::Changed`] for a save whose
    /// file changed since it was read).
    #[error(transparent)]
    Vault(#[from] VaultError),
}

/// A filename (or title) with the pattern's placeholders filled and
/// characters no file name should hold replaced.
fn fill_pattern(pattern: &str, date: &str, customer: &str, number: &str) -> String {
    let filled = pattern
        .replace("{date}", date)
        .replace("{customer}", customer)
        .replace("{invoice_num}", number);
    let cleaned: String = filled
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                '-'
            } else {
                c
            }
        })
        .collect();
    cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A number as frontmatter writes it: a YAML integer when it reads back the
/// same (`105`), else text (`"0042"`, `2026-0042`).
fn number_value(number: &InvoiceNumber) -> Value {
    number
        .as_str()
        .parse::<u64>()
        .ok()
        .filter(|n| n.to_string() == number.as_str())
        .map_or_else(
            || Value::String(number.to_string()),
            |n| Value::Number(n.into()),
        )
}

/// Multi-line text as a `|` block wants it: ending in a newline.
fn block_text(text: &str) -> Value {
    if text.contains('\n') && !text.ends_with('\n') {
        Value::String(format!("{text}\n"))
    } else {
        Value::String(text.to_string())
    }
}

/// The bill table for `rows`, every amount computed, aligned.
fn bill_table(
    rows: &[DraftRow],
    words: &Words,
    money: &crate::doc::Money,
) -> Result<String, DraftError> {
    let with_code = rows.iter().any(|r| r.code.is_some());
    let with_unit = rows.iter().any(|r| r.unit.is_some());
    let mut header: Vec<String> = Vec::new();
    if with_code {
        header.push(words.item.first().to_string());
    }
    header.push(words.description.first().to_string());
    header.push(words.quantity.first().to_string());
    if with_unit {
        header.push(words.unit.first().to_string());
    }
    header.extend([
        words.rate.first().to_string(),
        words.amount.first().to_string(),
        "*Account*".to_string(),
    ]);
    let cell = |text: &Option<String>| text.as_deref().map(escape_cell).unwrap_or_default();
    let line = |cells: Vec<String>| format!("| {} |\n", cells.join(" | "));
    // No delimiter row: the engine then aligns by role (numbers right) when
    // it writes the table.
    let mut text = line(header.clone());
    for row in rows {
        let mut cells = Vec::new();
        if with_code {
            cells.push(cell(&row.code));
        }
        cells.push(escape_cell(&row.description));
        cells.push(cell(&row.quantity));
        if with_unit {
            cells.push(cell(&row.unit));
        }
        cells.extend([cell(&row.rate), cell(&row.amount), cell(&row.books)]);
        text.push_str(&line(cells));
    }
    let mut total: Vec<String> = header.iter().map(|_| String::new()).collect();
    let rate_col = header.len() - 3;
    total[rate_col] = format!("**{}**", words.total.first());
    text.push_str(&line(total));

    let parsed = gfm::tables(&text).into_iter().next().ok_or_else(|| {
        DraftError::Verify(vec![Problem::error(
            ProblemCode::Table,
            "the new table doesn't read",
        )])
    })?;
    let table = BillTable::from_table(parsed, words)?.ok_or_else(|| {
        DraftError::Verify(vec![Problem::error(
            ProblemCode::NoBillTable,
            "the new table has no amount column",
        )])
    })?;
    let style = money.for_table(&table);
    let recalc = table.recompute(words, &style)?;
    Ok(format!("{}\n", recalc.table.render()))
}

/// The bodies of the customer's default snippets, joined.
fn default_snippets(site: &Site, vault: &dyn Vault, from: &VaultPath, links: &[String]) -> String {
    links
        .iter()
        .filter_map(|link| snippets::find(site, from, link))
        .filter_map(|snippet| snippet.body(vault).ok())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Render a new draft against the index (and `books`, the `invoice:`
/// numbers already booked, for Phase 2's numbering).
///
/// # Errors
/// [`DraftError`].
pub fn render(
    index: &DocumentIndex,
    vault: &dyn Vault,
    new: &NewInvoice,
    books: &[&str],
) -> Result<Draft, DraftError> {
    let settings: &Settings = index.settings();
    let words = &settings.words;
    let site = index.site();
    if !is_iso_date(&new.date) {
        return Err(DraftError::BadDate(new.date.clone()));
    }
    let dir = Settings::vault_path(&settings.invoices_dir)
        .map_err(|_| DraftError::BadPath(settings.invoices_dir.clone()))?;
    let from = dir.child("new.md").unwrap_or_else(|_| dir.clone());
    let customer =
        customers::find_customer(site, &from, &new.customer, words).map_err(|e| match e {
            CustomerLookup::NotFound => DraftError::UnknownCustomer(new.customer.clone()),
            CustomerLookup::NotACustomer(path) => DraftError::NotACustomer(path),
        })?;
    let number = match &new.number {
        Some(text) => {
            let number =
                InvoiceNumber::new(text).ok_or_else(|| DraftError::BadNumber(text.clone()))?;
            if let Some(doc) = index.by_number(number.as_str()).first() {
                return Err(DraftError::DuplicateNumber {
                    number: number.to_string(),
                    path: doc.path.clone(),
                });
            }
            if books.contains(&number.as_str()) {
                return Err(DraftError::DuplicateNumber {
                    number: number.to_string(),
                    path: VaultPath::root(),
                });
            }
            number
        }
        None => index.next_number(books.iter().copied())?,
    };
    let bill_to = customer.billing_contact(site, words);
    let company = customers::company(site, settings);
    let labels = &words.billing_labels;
    let template = new
        .template
        .clone()
        .or_else(|| customer.invoice_template.clone())
        .or_else(|| {
            templates::discover(vault, site)
                .into_iter()
                .find(|t| t.supports(new.doc_type))
                .map(|t| t.template_path())
        });
    let terms = new.terms.or(customer.terms);
    let due_date = terms.map(|t| ledgeline_core::reports::add_days(&new.date, i64::from(t)));

    let filename = fill_pattern(
        &settings.filename,
        &new.date,
        &bill_to.name,
        number.as_str(),
    );
    let filename = if filename.to_ascii_lowercase().ends_with(".md") {
        filename
    } else {
        format!("{filename}.md")
    };
    let path = dir
        .join(&filename)
        .ok()
        .filter(|p| {
            p.parent().as_ref() == Some(&dir)
                || (dir.is_root() && p.parent().is_none_or(|d| d.is_root()))
        })
        .ok_or_else(|| DraftError::BadPath(filename.clone()))?;
    let title = filename[..filename.len() - 3].to_string();

    let text = |s: &str| Value::String(s.to_string());
    let mut fields: Vec<(&str, Value)> = vec![
        ("title", text(&title)),
        ("type", text(new.doc_type.as_str())),
    ];
    fields.extend(template.as_deref().map(|t| ("template", text(t))));
    fields.push(("status", text("draft")));
    fields.push(("invoice_num", number_value(&number)));
    fields.push(("date", text(&new.date)));
    fields.extend(terms.map(|t| ("terms", Value::Number(t.into()))));
    fields.extend(due_date.as_deref().map(|d| ("due_date", text(d))));
    fields.extend(new.reference.as_deref().map(|r| ("reference", text(r))));
    fields.push(("customer", text(&customer.wikilink())));
    fields.push(("invoicee_name", text(&bill_to.name)));
    fields.extend(
        bill_to
            .email
            .as_deref()
            .map(|e| ("invoicee_email", text(e))),
    );
    fields.extend(
        bill_to
            .phone
            .as_deref()
            .map(|p| ("invoicee_phone", text(p))),
    );
    fields.extend(
        bill_to
            .address
            .as_deref()
            .map(|a| ("invoicee_address", block_text(a))),
    );
    if let Some(company) = &company {
        fields.push(("invoicer", text(&format!("[[{}]]", company.title))));
        fields.push(("invoicer_name", text(&company.title)));
        fields.extend(company.email(labels).map(|e| ("invoicer_email", text(e))));
        fields.extend(company.phone(labels).map(|p| ("invoicer_phone", text(p))));
        fields.extend(
            company
                .address(labels)
                .map(|a| ("invoicer_address", block_text(a))),
        );
    }
    fields.extend(new.extra.iter().map(|(k, v)| (k.as_str(), v.clone())));
    let frontmatter = frontmatter::render_block(&fields)?;

    let table = bill_table(&new.rows, words, index.money())?;
    let after = new.prose_after.clone().unwrap_or_else(|| {
        default_snippets(site, vault, &customer.contact.path, &customer.snippets)
    });
    let mut body = String::new();
    if let Some(before) = new
        .prose_before
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        body.push_str(before);
        body.push_str("\n\n");
    }
    body.push_str(&table);
    if !after.trim().is_empty() {
        body.push('\n');
        body.push_str(after.trim_end());
        body.push('\n');
    }
    let text = format!("{frontmatter}\n{body}");
    verify(&text, settings, index.money())?;
    Ok(Draft { path, number, text })
}

/// Refuse text that doesn't read back as a sound document: frontmatter that
/// doesn't parse, no bill table, or a table that doesn't read or add up.
pub(crate) fn verify(
    text: &str,
    settings: &Settings,
    money: &crate::doc::Money,
) -> Result<(), DraftError> {
    let invoice = Invoice::parse(text, &settings.words);
    let fatal: Vec<Problem> = invoice
        .problems(&settings.words, money)
        .into_iter()
        .filter(|p| {
            matches!(
                p.code,
                ProblemCode::Frontmatter
                    | ProblemCode::NoBillTable
                    | ProblemCode::Table
                    | ProblemCode::Arithmetic
            )
        })
        .collect();
    if fatal.is_empty() {
        Ok(())
    } else {
        Err(DraftError::Verify(fatal))
    }
}

/// Write a new draft and index it. See the module docs for how the write is
/// made atomic and exclusive.
///
/// # Errors
/// [`DraftError::Exists`] when the name is taken; [`DraftError::Vault`].
pub fn create(
    index: &mut DocumentIndex,
    vault: &dyn Vault,
    draft: &Draft,
) -> Result<Revision, DraftError> {
    if let Some(dir) = draft.path.parent() {
        vault.create_dir_all(&dir)?;
    }
    let claimed = vault.create_new_shared(&draft.path, b"").map_err(|e| {
        if e.kind() == std::io::ErrorKind::AlreadyExists {
            DraftError::Exists(draft.path.clone())
        } else {
            DraftError::Vault(e)
        }
    })?;
    let written = vault.replace(&draft.path, draft.text.as_bytes(), &claimed);
    let written = match written {
        Ok(revision) => revision,
        Err(e) => {
            // Give the name back; the empty claim is ours.
            let _ = vault.remove_file(&draft.path);
            return Err(e.into());
        }
    };
    index.refresh(vault, std::slice::from_ref(&draft.path));
    Ok(written)
}

/// Save a document: verified, then written atomically only if the file is
/// still the revision the caller read; then re-indexed.
///
/// # Errors
/// [`DraftError::Verify`]; [`DraftError::Vault`] with
/// [`VaultError::Changed`] when the file changed since it was read.
pub fn save(
    index: &mut DocumentIndex,
    vault: &dyn Vault,
    path: &VaultPath,
    text: &str,
    expected: &Revision,
) -> Result<Revision, DraftError> {
    verify(text, index.settings(), index.money())?;
    let written = vault.replace(path, text.as_bytes(), expected)?;
    index.refresh(vault, std::slice::from_ref(path));
    Ok(written)
}

#[cfg(test)]
mod tests;
