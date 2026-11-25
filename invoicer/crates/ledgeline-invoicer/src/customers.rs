//! `customers new` and `customers import-csv` (ARCHITECTURE §6.4): the
//! flags and the CSV's columns are the same [`Column`]s, mapped onto the
//! app's customer form and written by the same core call as the app's Add
//! customer ([`invoicer_core::customer_edit`] through
//! [`invoicer_core::customer_import`]), so the files are the same bytes.

use crate::{Cli, Failure, emit, human, open, out};
use invoicer_core::customer_edit::{self, CustomerSaved};
use invoicer_core::customer_import::{
    self, Column, ImportError, ImportOptions, MAX_IMPORT_BYTES, fields_from,
};
use serde::Serialize;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// `customers new`'s flags: the app's customer form, one flag per
/// [`Column`].
#[derive(clap::Args, Debug)]
pub(crate) struct NewArgs {
    /// The customer's name: the note's title, what invoices link
    /// (`[[NAME]]`), and the books' `customer:` tag unless
    /// --ledger-customer says otherwise.
    #[arg(value_name = "NAME")]
    name: String,
    /// `person` or `organization`.
    #[arg(long, value_name = "KIND", default_value = "organization")]
    kind: String,
    /// The billing email (written labelled `billing`).
    #[arg(long, value_name = "EMAIL")]
    email: Option<String>,
    /// The billing phone (labelled `billing`).
    #[arg(long, value_name = "PHONE")]
    phone: Option<String>,
    /// The billing address (labelled `billing`); lines as line breaks
    /// (`$'1 Main St\nSpringfield'`).
    #[arg(long, value_name = "TEXT")]
    address: Option<String>,
    /// A person's organization, written as `[[ORG]]`.
    #[arg(long, value_name = "ORG")]
    company: Option<String>,
    /// Payment terms, in net days.
    #[arg(long, value_name = "DAYS")]
    terms: Option<String>,
    /// The default revenue account for its invoices' rows.
    #[arg(long, value_name = "ACCOUNT")]
    revenue_account: Option<String>,
    /// The default invoice template (`/invoices/tpl/x.html`).
    #[arg(long, alias = "template", value_name = "PATH")]
    invoice_template: Option<String>,
    /// The `customer:` tag in the books, when it isn't NAME.
    #[arg(long, value_name = "NAME")]
    ledger_customer: Option<String>,
    /// A contract: a `[[wikilink]]` or a path (`/`-rooted from the books
    /// folder). Repeatable.
    #[arg(long, value_name = "LINK|PATH")]
    contract: Vec<String>,
    /// The billing portal's name (Coupa, Ariba, …).
    #[arg(long, value_name = "NAME")]
    portal_name: Option<String>,
    /// The billing portal's URL (https://…).
    #[arg(long, value_name = "URL")]
    portal_url: Option<String>,
    /// The portal login: the username only, never a password.
    #[arg(long, value_name = "USER")]
    portal_login: Option<String>,
    /// Your vendor number in the portal.
    #[arg(long, value_name = "ID")]
    portal_supplier_id: Option<String>,
    /// The portal rejects invoices with no reference (PO).
    #[arg(long)]
    portal_requires_reference: bool,
    /// How to submit through the portal (markdown).
    #[arg(long, value_name = "TEXT")]
    portal_instructions: Option<String>,
    /// Print the note without writing it.
    #[arg(long)]
    dry_run: bool,
}

impl NewArgs {
    /// The flags as `(Column, text)` pairs, as a CSV row would give them.
    fn pairs(&self) -> Vec<(Column, &str)> {
        let optional = [
            (Column::Email, &self.email),
            (Column::Phone, &self.phone),
            (Column::Address, &self.address),
            (Column::Company, &self.company),
            (Column::Terms, &self.terms),
            (Column::RevenueAccount, &self.revenue_account),
            (Column::InvoiceTemplate, &self.invoice_template),
            (Column::LedgerCustomer, &self.ledger_customer),
            (Column::PortalName, &self.portal_name),
            (Column::PortalUrl, &self.portal_url),
            (Column::PortalLogin, &self.portal_login),
            (Column::PortalSupplierId, &self.portal_supplier_id),
            (Column::PortalInstructions, &self.portal_instructions),
        ];
        let requires = if self.portal_requires_reference {
            "yes"
        } else {
            ""
        };
        [
            (Column::Name, self.name.as_str()),
            (Column::Kind, self.kind.as_str()),
        ]
        .into_iter()
        .chain(
            optional
                .into_iter()
                .filter_map(|(column, value)| value.as_deref().map(|v| (column, v))),
        )
        .chain(
            self.contract
                .iter()
                .map(|c| (Column::Contracts, c.as_str())),
        )
        .chain([(Column::PortalRequiresReference, requires)])
        .collect()
    }
}

/// The columns `customers import-csv` reads, for its `--help`.
pub(crate) fn columns_help() -> String {
    let describe = |column: Column| match column {
        Column::Name => "required: the note's title, what invoices link ([[Name]]); alias title",
        Column::Kind => "person or organization (blank: organization); alias type",
        Column::Email => "the billing email",
        Column::Phone => "the billing phone",
        Column::Address => "the billing address; a quoted field may hold line breaks",
        Column::Company => "a person's organization, written as [[Org]]",
        Column::Terms => "payment terms, net days",
        Column::RevenueAccount => "the default revenue account",
        Column::InvoiceTemplate => "the default template (/invoices/tpl/x.html); alias template",
        Column::LedgerCustomer => "the books' customer: tag, when it isn't the name",
        Column::Contracts => "[[wikilinks]] or paths, separated by ;  alias contract",
        Column::PortalName => "the billing portal's name (Coupa, Ariba, …)",
        Column::PortalUrl => "the portal's URL (https://…)",
        Column::PortalLogin => "the portal username (never a password)",
        Column::PortalSupplierId => "your vendor number in the portal",
        Column::PortalRequiresReference => "yes/no: the portal rejects invoices with no PO",
        Column::PortalInstructions => "how to submit through the portal",
    };
    let width = Column::ALL
        .iter()
        .map(|c| c.header().len())
        .max()
        .unwrap_or(0);
    let rows: String = Column::ALL
        .iter()
        .map(|&c| format!("  {:width$}  {}\n", c.header(), describe(c)))
        .collect();
    format!(
        "The first row names the columns; each row after it is one customer.\n\
         Headers match whatever the case, spaces, - or _ (`Revenue Account` is\n\
         revenue_account); any other header is refused. Blank cells are left out.\n\
         Columns:\n\n{rows}\n\
         UTF-8, comma separated, quoted as spreadsheets write it (a quoted field\n\
         may hold commas, \"\" for a quote, and line breaks). At most {} MiB and\n\
         {} rows. Every row is checked first: if any doesn't read, or names a\n\
         customer that exists (without --skip-existing), nothing is written.",
        MAX_IMPORT_BYTES / (1024 * 1024),
        customer_import::MAX_IMPORT_ROWS,
    )
}

/// `customers import-csv`'s arguments.
#[derive(clap::Args, Debug)]
pub(crate) struct ImportCsvArgs {
    /// The CSV file. (Not named `file`: that is the global `-f`'s id.)
    #[arg(value_name = "FILE")]
    csv: PathBuf,
    /// Skip rows whose customer already exists, instead of writing nothing.
    #[arg(long)]
    skip_existing: bool,
    /// Check every row and list what would be created; write nothing.
    #[arg(long)]
    dry_run: bool,
}

/// `customers new --json`: what the app's create answers, plus `text` on a
/// dry run (as `new --dry-run`).
#[derive(Serialize)]
pub(crate) struct NewOut {
    #[serde(flatten)]
    pub saved: CustomerSaved,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

pub(crate) fn run_new(cli: &Cli, args: &NewArgs) -> Result<ExitCode, Failure> {
    let fields = fields_from(&args.pairs()).map_err(|e| Failure(e, None))?;
    let mut session = open(cli)?;
    let made = if args.dry_run {
        let new = customer_edit::prepare(session.index(), &fields)?;
        NewOut {
            saved: new.saved(false),
            text: Some(new.text),
        }
    } else {
        let (index, vault) = session.parts_mut();
        NewOut {
            saved: customer_edit::create(index, vault, &fields)?,
            text: None,
        }
    };
    emit(cli.json, &made, human::customer_created);
    Ok(ExitCode::SUCCESS)
}

/// The CSV file's bytes, at most [`MAX_IMPORT_BYTES`] (checked before
/// reading, and while reading in case it grows).
fn read_capped(file: &Path) -> Result<Vec<u8>, Failure> {
    let shown = file.display();
    let handle = std::fs::File::open(file).map_err(|e| Failure(format!("{shown}: {e}"), None))?;
    let meta = handle
        .metadata()
        .map_err(|e| Failure(format!("{shown}: {e}"), None))?;
    if !meta.is_file() {
        return Err(Failure(format!("{shown} isn't a file"), None));
    }
    let too_large = || {
        Failure(
            format!(
                "{shown}: {}",
                ImportError::TooLarge {
                    limit: MAX_IMPORT_BYTES
                }
            ),
            None,
        )
    };
    if meta.len() > MAX_IMPORT_BYTES as u64 {
        return Err(too_large());
    }
    let mut bytes = Vec::new();
    handle
        .take(MAX_IMPORT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| Failure(format!("{shown}: {e}"), None))?;
    if bytes.len() > MAX_IMPORT_BYTES {
        return Err(too_large());
    }
    Ok(bytes)
}

pub(crate) fn run_import(cli: &Cli, args: &ImportCsvArgs) -> Result<ExitCode, Failure> {
    let bytes = read_capped(&args.csv)?;
    let mut session = open(cli)?;
    let options = ImportOptions {
        skip_existing: args.skip_existing,
        dry_run: args.dry_run,
    };
    let (index, vault) = session.parts_mut();
    match customer_import::import_csv(index, vault, &bytes, &options) {
        Ok(report) => {
            emit(cli.json, &report, human::customers_imported);
            Ok(ExitCode::SUCCESS)
        }
        Err(error) => Ok(import_failed(cli, &args.csv, &error)),
    }
}

/// A refused import (exit 2): `{"error", "code"}` with `--json`, plus `rows`
/// (each `{line, name, error}`) when rows didn't read, or `created` when a
/// write failed part way; otherwise the message on stderr.
fn import_failed(cli: &Cli, file: &Path, error: &ImportError) -> ExitCode {
    let message = format!("{}: {error}", file.display());
    if cli.json {
        let mut value = serde_json::json!({ "error": message });
        let (code, extra) = match error {
            ImportError::Rows(rows) => ("invalid", Some(("rows", serde_json::json!(rows)))),
            ImportError::Write { written, .. } => {
                ("write", Some(("created", serde_json::json!(written))))
            }
            ImportError::Header(_) => ("header", None),
            ImportError::TooLarge { .. } | ImportError::TooManyRows { .. } => ("too-large", None),
            ImportError::NotText { .. } | ImportError::Csv(_) | ImportError::Empty => {
                ("file", None)
            }
        };
        value["code"] = code.into();
        if let Some((key, extra)) = extra {
            value[key] = extra;
        }
        out(&format!("{value}\n"));
    } else {
        eprintln!("ledgeline-invoicer: error: {message}");
    }
    ExitCode::from(2)
}
