//! Customers from columns: the CLI's `customers new` flags and
//! `customers import-csv` rows (ARCHITECTURE §6.4), both mapped onto the
//! app's [`CustomerFields`] and written through [`customer_edit::prepare`]
//! and [`customer_edit::write_new`], so a customer made here is the same
//! file the app's Add customer writes for the same values.
//!
//! - [`Column`] is one field: its CSV header (forgiving on case, spaces,
//!   `-` and `_`) and how its text reads. [`fields_from`] maps
//!   `(Column, text)` pairs to the form.
//! - [`import_csv`] reads a CSV file's bytes: a header row naming columns,
//!   then one customer per row. Every row is checked before anything is
//!   written; one bad row writes nothing (all or nothing). With
//!   [`ImportOptions::skip_existing`], a row naming a customer that already
//!   exists is skipped rather than refused.
//!
//! The input is untrusted: at most [`MAX_IMPORT_BYTES`] and
//! [`MAX_IMPORT_ROWS`], UTF-8 (a byte-order mark is dropped), comma
//! separated, RFC 4180 quoting (a quoted field may hold commas, quotes
//! written twice, and line breaks).

use crate::customer_edit::{self, CustomerError, CustomerFields, CustomerSaved, NewCustomer};
use crate::customers::{BillingPortal, ContactKind, Labelled, is_customer};
use crate::index::DocumentIndex;
use crate::notes::normalize_name;
use ledgeline_core::vault::{Vault, VaultPath};
use serde::Serialize;
use thiserror::Error;

/// The largest CSV file read: far beyond any real customer list.
pub const MAX_IMPORT_BYTES: usize = 4 * 1024 * 1024;

/// The most customer rows one import takes.
pub const MAX_IMPORT_ROWS: usize = 10_000;

/// The label the email, phone and address are written with: `billing`, as
/// the app's Add customer writes them (the one invoices pick first).
pub const CONTACT_LABEL: &str = "billing";

/// One customer field, as a CSV column or a `customers new` flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Column {
    /// `title:`. Required.
    Name,
    /// `type:`: `person` or `organization` (blank: organization).
    Kind,
    /// The billing email.
    Email,
    /// The billing phone.
    Phone,
    /// The billing address (several lines in a quoted field).
    Address,
    /// A person's organization (`company: "[[Org]]"`).
    Company,
    /// `terms:`, net days.
    Terms,
    /// `revenue_account:`.
    RevenueAccount,
    /// `invoice_template:`.
    InvoiceTemplate,
    /// `ledger_customer:`, the books' name when it isn't the title.
    LedgerCustomer,
    /// `contracts:`, `;`-separated wikilinks or paths.
    Contracts,
    /// `billing_portal.name`.
    PortalName,
    /// `billing_portal.url`.
    PortalUrl,
    /// `billing_portal.login` (the username only).
    PortalLogin,
    /// `billing_portal.supplier_id`.
    PortalSupplierId,
    /// `billing_portal.requires_reference`: yes/no.
    PortalRequiresReference,
    /// `billing_portal.instructions`.
    PortalInstructions,
}

impl Column {
    /// Every column, in the order the help lists them.
    pub const ALL: [Self; 17] = [
        Self::Name,
        Self::Kind,
        Self::Email,
        Self::Phone,
        Self::Address,
        Self::Company,
        Self::Terms,
        Self::RevenueAccount,
        Self::InvoiceTemplate,
        Self::LedgerCustomer,
        Self::Contracts,
        Self::PortalName,
        Self::PortalUrl,
        Self::PortalLogin,
        Self::PortalSupplierId,
        Self::PortalRequiresReference,
        Self::PortalInstructions,
    ];

    /// The header it is written as (and listed under).
    #[must_use]
    pub fn header(self) -> &'static str {
        match self {
            Self::Name => "name",
            Self::Kind => "kind",
            Self::Email => "email",
            Self::Phone => "phone",
            Self::Address => "address",
            Self::Company => "company",
            Self::Terms => "terms",
            Self::RevenueAccount => "revenue_account",
            Self::InvoiceTemplate => "invoice_template",
            Self::LedgerCustomer => "ledger_customer",
            Self::Contracts => "contracts",
            Self::PortalName => "portal_name",
            Self::PortalUrl => "portal_url",
            Self::PortalLogin => "portal_login",
            Self::PortalSupplierId => "portal_supplier_id",
            Self::PortalRequiresReference => "portal_requires_reference",
            Self::PortalInstructions => "portal_instructions",
        }
    }

    /// Other headers it answers to: the frontmatter key, where it differs.
    fn aliases(self) -> &'static [&'static str] {
        match self {
            Self::Name => &["title"],
            Self::Kind => &["type"],
            Self::InvoiceTemplate => &["template"],
            Self::Contracts => &["contract"],
            _ => &[],
        }
    }

    /// The column a CSV header names: case, surrounding spaces, and runs of
    /// spaces, `-` or `_` don't matter (`Revenue Account`,
    /// `revenue-account` and `REVENUE_ACCOUNT` are all `revenue_account`).
    #[must_use]
    pub fn from_header(header: &str) -> Option<Self> {
        let key = header_key(header);
        Self::ALL
            .into_iter()
            .find(|c| c.header() == key || c.aliases().contains(&key.as_str()))
    }

    /// Every accepted header, comma separated: what a refusal lists.
    #[must_use]
    pub fn listed() -> String {
        Self::ALL.map(Self::header).join(", ")
    }
}

/// A header folded for matching: trimmed, lower case, each run of white
/// space, `-` or `_` one `_`.
fn header_key(header: &str) -> String {
    header
        .trim()
        .to_lowercase()
        .split(|c: char| c.is_whitespace() || c == '-' || c == '_')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("_")
}

/// `person` or `organization` (any case; `org` for short). Blank is the
/// default, organization, as in the app's Add customer.
///
/// # Errors
/// Anything else, with the accepted values.
pub fn parse_kind(text: &str) -> Result<ContactKind, String> {
    match text.trim().to_lowercase().as_str() {
        "" | "organization" | "organisation" | "org" => Ok(ContactKind::Organization),
        "person" => Ok(ContactKind::Person),
        other => Err(format!("kind {other:?}: person or organization, please")),
    }
}

/// Yes or no: `yes`/`no`, `true`/`false`, `y`/`n`, `1`/`0` (any case);
/// blank is no.
///
/// # Errors
/// Anything else.
pub fn parse_yes_no(what: &str, text: &str) -> Result<bool, String> {
    match text.trim().to_lowercase().as_str() {
        "" | "no" | "n" | "false" | "0" => Ok(false),
        "yes" | "y" | "true" | "1" => Ok(true),
        other => Err(format!("{what} {other:?}: yes or no, please")),
    }
}

/// Net days: a whole number, or blank for none.
///
/// # Errors
/// Anything else (the core caps it at 3650 days).
pub fn parse_terms(text: &str) -> Result<Option<u32>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    text.parse::<u32>()
        .map(Some)
        .map_err(|_| format!("terms {text:?}: whole net days, please (30)"))
}

/// The form for `(column, text)` pairs: what the app's Add customer would
/// send for the same values. A blank value is no value; a repeated column
/// (`contracts`) adds to the list, any other repeated column's last value
/// wins. The core checks the values themselves when the note is prepared.
///
/// # Errors
/// A kind, terms or yes/no that doesn't read.
pub fn fields_from(pairs: &[(Column, &str)]) -> Result<CustomerFields, String> {
    let given = |text: &str| Some(text.trim().to_string()).filter(|t| !t.is_empty());
    let billing = |text: &str| {
        given(text).map(|value| {
            vec![Labelled {
                label: Some(CONTACT_LABEL.to_string()),
                value,
            }]
        })
    };
    let mut fields = CustomerFields {
        kind: Some(ContactKind::Organization),
        ..CustomerFields::default()
    };
    let mut portal = BillingPortal::default();
    let mut contracts: Vec<String> = Vec::new();
    for &(column, text) in pairs {
        match column {
            Column::Name => fields.title = Some(text.to_string()),
            Column::Kind => fields.kind = Some(parse_kind(text)?),
            Column::Email => fields.emails = billing(text),
            Column::Phone => fields.phones = billing(text),
            Column::Address => fields.addresses = billing(text),
            Column::Company => fields.company = given(text),
            Column::Terms => fields.terms = parse_terms(text)?.map(Some),
            Column::RevenueAccount => fields.revenue_account = given(text),
            Column::InvoiceTemplate => fields.invoice_template = given(text),
            Column::LedgerCustomer => fields.ledger_customer = given(text),
            Column::Contracts => contracts.extend(text.split(';').filter_map(given)),
            Column::PortalName => portal.name = given(text),
            Column::PortalUrl => portal.url = given(text),
            Column::PortalLogin => portal.login = given(text),
            Column::PortalSupplierId => portal.supplier_id = given(text),
            Column::PortalRequiresReference => {
                portal.requires_reference = parse_yes_no("portal_requires_reference", text)?;
            }
            Column::PortalInstructions => {
                portal.instructions = Some(text.trim_end().to_string()).filter(|t| !t.is_empty());
            }
        }
    }
    if !contracts.is_empty() {
        fields.contracts = Some(contracts);
    }
    if portal != BillingPortal::default() {
        fields.billing_portal = Some(Some(portal));
    }
    Ok(fields)
}

// ---------------------------------------------------------------------------
// The import
// ---------------------------------------------------------------------------

/// How [`import_csv`] runs.
#[derive(Debug, Clone, Copy, Default)]
pub struct ImportOptions {
    /// Skip a row whose customer already exists instead of refusing the
    /// file.
    pub skip_existing: bool,
    /// Check and render every row; write nothing.
    pub dry_run: bool,
}

/// A customer an import created (or, on a dry run, would create).
#[derive(Debug, Clone, Serialize)]
pub struct ImportedRow {
    /// The CSV line its row starts on (the header is line 1).
    pub line: u64,
    /// What the write answered ([`CustomerSaved`]); `written` is false on a
    /// dry run.
    #[serde(flatten)]
    pub saved: CustomerSaved,
    /// The note's text, on a dry run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

/// A row skipped because its customer exists ([`ImportOptions::skip_existing`]).
#[derive(Debug, Clone, Serialize)]
pub struct SkippedRow {
    /// The CSV line its row starts on.
    pub line: u64,
    /// The row's name.
    pub title: String,
    /// The customer note already there.
    pub path: VaultPath,
}

/// What an import did.
#[derive(Debug, Clone, Serialize)]
pub struct ImportReport {
    /// Nothing was written.
    pub dry_run: bool,
    /// The customers created, in file order.
    pub created: Vec<ImportedRow>,
    /// The rows skipped as existing.
    pub skipped: Vec<SkippedRow>,
}

/// A row that doesn't import.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RowError {
    /// The CSV line it starts on.
    pub line: u64,
    /// Its name, as written (empty when it has none).
    pub name: String,
    /// Why.
    pub error: String,
}

/// Why an import wrote nothing (or, for [`ImportError::Write`], stopped
/// part way).
#[derive(Debug, Error)]
pub enum ImportError {
    /// Past [`MAX_IMPORT_BYTES`].
    #[error("the file is larger than {limit} bytes; split it")]
    TooLarge {
        /// The cap.
        limit: usize,
    },
    /// Not UTF-8.
    #[error("line {line}: the file isn't UTF-8 text; save it as CSV UTF-8")]
    NotText {
        /// Where the first bad byte is.
        line: u64,
    },
    /// The CSV itself doesn't read.
    #[error("{0}")]
    Csv(String),
    /// No header row.
    #[error("the file is empty: give a header row (name, email, …) and a row per customer")]
    Empty,
    /// The header row doesn't read.
    #[error("{0}")]
    Header(String),
    /// Past [`MAX_IMPORT_ROWS`].
    #[error("more than {limit} customers in one file; split it")]
    TooManyRows {
        /// The cap.
        limit: usize,
    },
    /// Rows that don't import: nothing was written.
    #[error("{}, so nothing was written:\n{}", plural(.0.len()), lines(.0))]
    Rows(Vec<RowError>),
    /// A write failed after others succeeded (the folder changed under the
    /// import); `written` are the ones made.
    #[error("line {line}: {error} ({} written before it)", written.len())]
    Write {
        /// The row that failed.
        line: u64,
        /// Why.
        error: CustomerError,
        /// The customers already written.
        written: Vec<ImportedRow>,
    },
}

fn plural(n: usize) -> String {
    if n == 1 {
        "1 row doesn't read".to_string()
    } else {
        format!("{n} rows don't read")
    }
}

fn lines(errors: &[RowError]) -> String {
    errors
        .iter()
        .map(|e| {
            if e.name.trim().is_empty() {
                format!("  line {}: {}", e.line, e.error)
            } else {
                format!("  line {} ({}): {}", e.line, e.name.trim(), e.error)
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The line a byte offset falls on (1-based).
fn line_at(bytes: &[u8], offset: usize) -> u64 {
    Lines::default().at(bytes, offset)
}

/// Line numbers for increasing byte offsets, counted once over the file
/// (the CSV reader's own line count skips blank lines).
#[derive(Default)]
struct Lines {
    offset: usize,
    newlines: u64,
}

impl Lines {
    /// The 1-based line `offset` is on; offsets must not decrease.
    fn at(&mut self, bytes: &[u8], offset: usize) -> u64 {
        let offset = offset.min(bytes.len());
        if offset > self.offset {
            self.newlines += bytes[self.offset..offset]
                .iter()
                .filter(|&&b| b == b'\n')
                .count() as u64;
            self.offset = offset;
        }
        self.newlines + 1
    }
}

/// The header row's columns.
fn columns(header: &csv::StringRecord) -> Result<Vec<Column>, ImportError> {
    let mut out: Vec<Column> = Vec::new();
    for raw in header {
        let Some(column) = Column::from_header(raw) else {
            return Err(ImportError::Header(format!(
                "unknown column {:?}; the columns are: {}",
                raw.trim(),
                Column::listed()
            )));
        };
        if out.contains(&column) {
            return Err(ImportError::Header(format!(
                "the column {:?} is given twice",
                column.header()
            )));
        }
        out.push(column);
    }
    if !out.contains(&Column::Name) {
        return Err(ImportError::Header(
            "no name column: the header row must name one (name, email, …)".to_string(),
        ));
    }
    Ok(out)
}

/// A row ready to write, or skipped.
enum Planned {
    Create { line: u64, new: NewCustomer },
    Skip(SkippedRow),
}

/// The names a new note will answer to: its title and its file name, folded
/// as wikilinks fold them.
fn names_of(new: &NewCustomer) -> [String; 2] {
    [
        normalize_name(&new.title),
        normalize_name(&customer_edit::file_stem(&new.title)),
    ]
}

/// One row checked against the index and the vault: created, skipped, or
/// why not.
fn plan_row(
    index: &DocumentIndex,
    vault: &dyn Vault,
    columns: &[Column],
    record: &csv::StringRecord,
    line: u64,
    options: &ImportOptions,
) -> Result<Planned, String> {
    if record.len() != columns.len() {
        let fields = if record.len() == 1 { "field" } else { "fields" };
        return Err(format!(
            "{} {fields}, but the header has {}",
            record.len(),
            columns.len()
        ));
    }
    let pairs: Vec<(Column, &str)> = columns.iter().copied().zip(record.iter()).collect();
    let fields = fields_from(&pairs)?;
    match customer_edit::prepare(index, &fields) {
        Ok(new) => match vault.stat(&new.path) {
            Ok(None) => Ok(Planned::Create { line, new }),
            Ok(Some(_)) => Err(CustomerError::Exists(new.path).to_string()),
            Err(e) => Err(e.to_string()),
        },
        Err(CustomerError::NameTaken { title, path }) if options.skip_existing => {
            let words = &index.settings().words;
            if index
                .site()
                .note(&path)
                .is_some_and(|n| is_customer(n, words))
            {
                Ok(Planned::Skip(SkippedRow { line, title, path }))
            } else {
                Err(format!(
                    "a note named {title:?} already exists ({path}) and isn't a customer; \
                     choose another name"
                ))
            }
        }
        Err(e) => Err(e.to_string()),
    }
}

/// Import customers from a CSV file's bytes: a header row naming
/// [`Column`]s, then a customer per row (blank rows are skipped). Every row
/// is checked first; if any doesn't read, or two rows name the same
/// customer, nothing is written ([`ImportError::Rows`] lists them all).
///
/// # Errors
/// [`ImportError`]: the file (too large, not UTF-8, malformed, too many
/// rows), its header (an unknown or repeated column, no `name`), its rows,
/// or a write that failed part way.
pub fn import_csv(
    index: &mut DocumentIndex,
    vault: &dyn Vault,
    bytes: &[u8],
    options: &ImportOptions,
) -> Result<ImportReport, ImportError> {
    if bytes.len() > MAX_IMPORT_BYTES {
        return Err(ImportError::TooLarge {
            limit: MAX_IMPORT_BYTES,
        });
    }
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    if let Err(e) = std::str::from_utf8(bytes) {
        return Err(ImportError::NotText {
            line: line_at(bytes, e.valid_up_to()),
        });
    }
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .from_reader(bytes);
    let mut records = reader.records();
    let csv_error = |e: csv::Error| {
        let line = e.position().map(|p| line_at(bytes, p.byte() as usize));
        ImportError::Csv(match line {
            Some(line) => format!("line {line}: the CSV doesn't read: {e}"),
            None => format!("the CSV doesn't read: {e}"),
        })
    };
    let header = records
        .next()
        .ok_or(ImportError::Empty)?
        .map_err(csv_error)?;
    let columns = columns(&header)?;

    let mut planned: Vec<Planned> = Vec::new();
    let mut errors: Vec<RowError> = Vec::new();
    // Each name a planned row answers to, with its line.
    let mut taken: Vec<(String, u64)> = Vec::new();
    let mut rows = 0usize;
    let mut line_of = Lines::default();
    for record in records {
        let record = record.map_err(csv_error)?;
        if record.iter().all(|field| field.trim().is_empty()) {
            continue;
        }
        rows += 1;
        if rows > MAX_IMPORT_ROWS {
            return Err(ImportError::TooManyRows {
                limit: MAX_IMPORT_ROWS,
            });
        }
        // The reader's position for a record can be the start of the blank
        // lines before it: step over those to the record's own line.
        let start = record.position().map_or(0, |p| p.byte() as usize);
        let start = bytes[start.min(bytes.len())..]
            .iter()
            .position(|b| !matches!(b, b'\r' | b'\n'))
            .map_or(bytes.len(), |skip| start + skip);
        let line = line_of.at(bytes, start);
        let name = columns
            .iter()
            .position(|c| *c == Column::Name)
            .and_then(|at| record.get(at))
            .unwrap_or_default()
            .to_string();
        match plan_row(index, vault, &columns, &record, line, options) {
            Ok(Planned::Create { line, new }) => {
                let names = names_of(&new);
                if let Some((_, earlier)) = taken.iter().find(|(n, _)| names.contains(n)) {
                    errors.push(RowError {
                        line,
                        name,
                        error: format!("the same customer as line {earlier}"),
                    });
                    continue;
                }
                taken.extend(names.into_iter().map(|n| (n, line)));
                planned.push(Planned::Create { line, new });
            }
            Ok(skip @ Planned::Skip(_)) => planned.push(skip),
            Err(error) => errors.push(RowError { line, name, error }),
        }
    }
    if !errors.is_empty() {
        return Err(ImportError::Rows(errors));
    }

    let mut report = ImportReport {
        dry_run: options.dry_run,
        created: Vec::new(),
        skipped: Vec::new(),
    };
    for plan in planned {
        match plan {
            Planned::Skip(skipped) => report.skipped.push(skipped),
            Planned::Create { line, new } if options.dry_run => report.created.push(ImportedRow {
                line,
                saved: new.saved(false),
                text: Some(new.text),
            }),
            Planned::Create { line, new } => match customer_edit::write_new(index, vault, &new) {
                Ok(saved) => report.created.push(ImportedRow {
                    line,
                    saved,
                    text: None,
                }),
                Err(error) => {
                    return Err(ImportError::Write {
                        line,
                        error,
                        written: report.created,
                    });
                }
            },
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests;
