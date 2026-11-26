//! `ledgeline-invoicer` — Ledgeline Invoicer's binary (`invoicer/ARCHITECTURE.md`
//! §2.2, §6.4).
//!
//! The documents (invoicer PLAN 1c): `new`, `list`, `show`, `set`, `recalc`,
//! `check`, `next-number`, `config` and `customers list|show|new|import-csv`
//! ([`customers`]) write markdown only, never the journal (I2). The books (PLAN 2c): `send`, `void`,
//! `rebook`, `apply-payment`, `reconcile`, `receivables` and `recognize`.
//! The PDF (PLAN 3c): `pdf`, in a desktop build ([`pdf`], `app::pdf`).
//! Every command takes `--json`.
//!
//! This file parses arguments, opens the books and prints; each operation is
//! one `invoicer_core` function (`session`, `views`, `edit`, `books`,
//! `reconcile`, `receivables`), shared with the API ([`app`]).
//!
//! **CLI or app** (PLAN 3a): a subcommand is a job, run and exit, whatever
//! the build. With no subcommand, a build with the `server` feature runs the
//! app: `--server` is the headless server (e2e, port 5199), otherwise the
//! window (`gui`, the default build). A CLI-only build
//! (`--no-default-features`) asks for a command.
//!
//! Every write to the books is retried once when the journal or the file
//! changed between reading and writing (Ledgeline saved meanwhile): nothing
//! was written by the first attempt, and the second reads afresh.
//!
//! Exit codes: 0 done; 1 `check` or `reconcile` found errors; 2 the command
//! couldn't run (a usage error, books that don't open, a refused write).

#[cfg(feature = "server")]
mod app;
mod customers;
mod human;
mod pdf;
mod starters;

use clap::{ArgMatches, CommandFactory, FromArgMatches, Parser, Subcommand};
use invoicer_core::books::{
    self, ApplyPayment, BooksError, InvoiceBooks, Rebook, Recognize, Send, Void,
};
use invoicer_core::doc::{DocType, Status};
use invoicer_core::draft::{DraftRow, NewInvoice};
use invoicer_core::edit::{self, WriteError};
use invoicer_core::frontmatter::FieldEdit;
use invoicer_core::index::DocumentIndex;
use invoicer_core::index::ListFilter;
use invoicer_core::reconcile::{self, MirrorChange, ReconcileOptions};
use invoicer_core::session::Session;
use invoicer_core::table::books::is_iso_date;
use invoicer_core::templates;
use invoicer_core::tera_scan;
use invoicer_core::views::{self, ListQuery};
use ledgeline_core::vault::VaultPath;
use ledgeline_shell::AppDirs;
use serde::Serialize;
use serde_json::Value;
use std::fmt::Display;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// The env var naming a journal to open when none is given (as Ledgeline).
const FIXTURE_ENV: &str = "LEDGELINE_FIXTURE";

/// The command line. `about` is the package description, so `--help` opens by
/// naming the app.
#[derive(Parser, Debug)]
#[command(name = "ledgeline-invoicer", version, about, long_about = None)]
struct Cli {
    /// The books' main journal. Default: $LEDGELINE_FIXTURE, else the
    /// journal opened last in Ledgeline or the invoicer.
    #[arg(value_name = "JOURNAL")]
    journal: Option<PathBuf>,

    /// The books' main journal (the same as JOURNAL; usable after the
    /// command).
    #[arg(short = 'f', long = "file", value_name = "JOURNAL", global = true)]
    file: Option<PathBuf>,

    /// Print JSON (one value on stdout) instead of text.
    #[arg(long, global = true)]
    json: bool,

    /// The date to treat as today (YYYY-MM-DD): `new`'s default date, and
    /// what "overdue" is measured against. Default: the local date.
    #[arg(long, value_name = "DATE", global = true)]
    today: Option<String>,

    /// Run the headless server (API, SPA and MBR pages; no window), for
    /// e2e and scripts. Default port 5199.
    #[cfg(feature = "server")]
    #[arg(short = 's', long)]
    server: bool,

    /// The address the server binds (the app only).
    #[cfg(feature = "server")]
    #[arg(long, default_value = app::DEFAULT_HOST)]
    host: String,

    /// The port the server binds (the app only). Default: 5199 with
    /// --server, an ephemeral port for the window.
    #[cfg(feature = "server")]
    #[arg(long)]
    port: Option<u16>,

    /// DEV ONLY: let a browser SPA on this exact origin call the API
    /// cross-origin (e.g. `--allow-origin http://localhost:4273`).
    /// Repeatable; never a wildcard.
    #[cfg(feature = "server")]
    #[arg(long = "allow-origin", value_name = "ORIGIN")]
    allow_origin: Vec<String>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Create a draft invoice (a markdown file; the books are untouched).
    New(NewArgs),
    /// List documents, newest first.
    List(ListArgs),
    /// Show one document: its fields, its computed bill table, its problems.
    Show {
        /// A number (`105`) or a file in the books folder.
        document: String,
    },
    /// Set or remove frontmatter fields, by key, keeping everything else
    /// byte for byte.
    Set(SetArgs),
    /// Rewrite bill-table amounts that disagree with their arithmetic.
    Recalc(RecalcArgs),
    /// Validate every document, customer and the catalog. Exit 1 on errors.
    Check {
        /// Fail (exit 1) on warnings too.
        #[arg(long)]
        strict: bool,
    },
    /// Print the next invoice number (over the files and the books).
    NextNumber,
    /// Print the effective settings: [books] and [invoicer], words included.
    Config,
    /// Customers: person and organization notes tagged as customers.
    #[command(subcommand)]
    Customers(CustomersCommand),
    /// Send an invoice: book it (accrual books), then mark it sent.
    Send(SendArgs),
    /// Void an invoice: reverse its booking, then mark it void.
    Void(VoidArgs),
    /// Re-book a sent invoice after an edit (nothing when it already agrees).
    Rebook(RebookArgs),
    /// Cash books: list the deposits that might pay an invoice, apply one,
    /// or undo one.
    ApplyPayment(ApplyPaymentArgs),
    /// Mirror the books' payment state into each sent invoice's `paid:`, and
    /// report where the books and the files disagree. Exit 1 on errors.
    Reconcile(ReconcileArgs),
    /// Export an invoice's PDF: a sent one is kept in `pdf-dir` (earlier
    /// exports kept under their date), a draft goes to a temporary file.
    /// Needs the desktop build.
    Pdf(PdfArgs),
    /// Open invoices, aging, DSO and average days to pay.
    Receivables(ReceivablesArgs),
    /// Post the recognition entries due through a date (never after today).
    Recognize(RecognizeArgs),
}

#[derive(clap::Args, Debug)]
struct SendArgs {
    /// A number (`105`) or a file in the books folder.
    document: String,
    /// How it was delivered (`sent_via`): email, portal, mail…
    #[arg(long, value_name = "HOW")]
    via: Option<String>,
    /// Show what would be booked; write nothing.
    #[arg(long)]
    dry_run: bool,
}

#[derive(clap::Args, Debug)]
struct PdfArgs {
    /// A number (`105`) or a file in the books folder.
    document: String,
    /// Write the PDF here instead: a copy, so nothing is kept in `pdf-dir`
    /// and `sent_hash` is left alone. Its folder must exist.
    #[arg(short = 'o', long = "output", value_name = "PATH")]
    output: Option<PathBuf>,
}

#[derive(clap::Args, Debug)]
struct VoidArgs {
    /// A number (`105`) or a file in the books folder.
    document: String,
    /// The void's date (YYYY-MM-DD): the reversal's and `void_date`.
    /// Default: today.
    #[arg(long, value_name = "DATE")]
    date: Option<String>,
}

#[derive(clap::Args, Debug)]
struct RebookArgs {
    /// A number (`105`) or a file in the books folder.
    document: String,
    /// Show what is booked and what would be; write nothing.
    #[arg(long)]
    dry_run: bool,
}

#[derive(clap::Args, Debug)]
struct ApplyPaymentArgs {
    /// A number (`207`) or a file in the books folder.
    document: String,
    /// The deposit to apply (a candidate's reference, or its first 6+
    /// characters). Without it, the candidates are listed. With --undo, the
    /// applied deposit to undo.
    #[arg(long, value_name = "REF")]
    txn: Option<String>,
    /// Undo a payment applied to this invoice (the only one, or --txn's),
    /// writing the deposit back exactly as it was.
    #[arg(long)]
    undo: bool,
}

#[derive(clap::Args, Debug)]
struct ReconcileArgs {
    /// Numbers or files. Default: every sent invoice.
    documents: Vec<String>,
    /// Report what would change without writing.
    #[arg(long)]
    dry_run: bool,
}

#[derive(clap::Args, Debug)]
struct ReceivablesArgs {
    /// Show the aging buckets too (`--json` always includes them).
    #[arg(long)]
    aging: bool,
    /// The days sales outstanding window, in days. Default: `[invoicer]
    /// dso-days`, else 90.
    #[arg(long, value_name = "DAYS")]
    dso_days: Option<u32>,
}

#[derive(clap::Args, Debug)]
struct RecognizeArgs {
    /// Post entries dated on or before this (YYYY-MM-DD, never after
    /// today). Default: today.
    #[arg(long, value_name = "DATE")]
    through: Option<String>,
    /// Also fold the entries due in closed periods into one adjustment per
    /// schedule, dated today.
    #[arg(long)]
    include_closed: bool,
    /// List what is due; write nothing.
    #[arg(long)]
    dry_run: bool,
}

#[derive(clap::Args, Debug)]
struct NewArgs {
    /// The customer: a note's title, alias or `[[wikilink]]`.
    #[arg(long)]
    customer: String,
    /// The template path (`/invoices/tpl/x.html`). Default: the customer's
    /// `invoice_template`, else the first invoice template.
    #[arg(long)]
    template: Option<String>,
    /// The number. Default: the next one.
    #[arg(long)]
    number: Option<String>,
    /// The issue date (YYYY-MM-DD). Default: today.
    #[arg(long)]
    date: Option<String>,
    /// Net days. Default: the customer's `terms`.
    #[arg(long)]
    terms: Option<u32>,
    /// What the customer quotes back (`PO #105`).
    #[arg(long)]
    reference: Option<String>,
    /// A row: `DESC|QTY|RATE|ACCOUNT` (trailing fields optional). With no
    /// QTY, a money RATE is the row's amount; a percentage is an adjustment;
    /// a total word alone (`Subtotal`) is a total row. Repeatable; rows keep
    /// their order with --from-catalog's.
    #[arg(long, value_name = "SPEC")]
    row: Vec<String>,
    /// A row from the catalog: `CODE` or `CODE:QTY`. Repeatable.
    #[arg(long, value_name = "CODE[:QTY]")]
    from_catalog: Vec<String>,
    /// A custom frontmatter field: `key=value` or `key:=JSON`. Repeatable.
    #[arg(long, value_name = "KEY=VALUE")]
    field: Vec<String>,
    /// Print the draft without writing it.
    #[arg(long)]
    dry_run: bool,
}

#[derive(clap::Args, Debug)]
struct ListArgs {
    /// Only this status (draft, sent, void).
    #[arg(long)]
    status: Option<String>,
    /// Only this customer (a title, link or billed name).
    #[arg(long)]
    customer: Option<String>,
    /// Only this type (invoice, estimate, credit-note).
    #[arg(long = "type", value_name = "TYPE")]
    doc_type: Option<String>,
    /// Only open documents: sent and not paid in full.
    #[arg(long)]
    open: bool,
    /// Only overdue documents: open and past due.
    #[arg(long)]
    overdue: bool,
}

#[derive(clap::Args, Debug)]
struct SetArgs {
    /// A number (`105`) or a file in the books folder.
    document: String,
    /// `key=value` (a whole number is written as one) or `key:=JSON`.
    #[arg(value_name = "KEY=VALUE")]
    assignments: Vec<String>,
    /// Remove a field. Repeatable.
    #[arg(long, value_name = "KEY")]
    unset: Vec<String>,
}

#[derive(clap::Args, Debug)]
struct RecalcArgs {
    /// Numbers or files. Default: every draft whose amounts disagree.
    documents: Vec<String>,
    /// Report what would change without writing.
    #[arg(long)]
    dry_run: bool,
}

#[derive(Subcommand, Debug)]
enum CustomersCommand {
    /// List customers.
    List,
    /// Show one customer, with its documents.
    Show {
        /// A title, alias or `[[wikilink]]`.
        name: String,
    },
    /// Create a customer note in `customers-dir`, as the app's Add customer
    /// does. Refuses a name some note already has.
    New(Box<customers::NewArgs>),
    /// Create customers from a CSV file, one per row. Nothing is written if
    /// any row doesn't read.
    #[command(after_help = customers::columns_help())]
    ImportCsv(customers::ImportCsvArgs),
}

/// A failure that stops a command (exit 2), with a stable code for the books
/// errors ([`BooksError::code`]).
struct Failure(String, Option<&'static str>);

impl<E: Display> From<E> for Failure {
    fn from(error: E) -> Self {
        Self(error.to_string(), None)
    }
}
fn books_failure(error: BooksError) -> Failure {
    Failure(error.to_string(), Some(error.code()))
}

/// Run a books write, retried once when the files changed under it.
fn retrying<T>(op: impl FnMut() -> Result<T, BooksError>) -> Result<T, Failure> {
    let (result, retried) = books::retry_once(op);
    if retried {
        eprintln!(
            "ledgeline-invoicer: the books or the file changed while writing (nothing was \
             written); read them again and retried"
        );
    }
    result.map_err(books_failure)
}

/// The number of the document `target` names.
fn number_of(session: &Session, target: &str) -> Result<String, Failure> {
    let path = document_path(session, target)?;
    session
        .index()
        .doc(&path)
        .and_then(|d| d.number())
        .map(str::to_string)
        .ok_or_else(|| Failure(format!("{path} has no `invoice_num`"), None))
}

/// Refresh one invoice's `paid:` mirror after a write that moved its
/// payments (the books read afresh). `None` when nothing changed or the
/// books can't say.
fn mirror_one(
    books: &InvoiceBooks,
    index: &mut DocumentIndex,
    number: &str,
) -> Option<MirrorChange> {
    let loaded = books.load().ok()?;
    reconcile::reconcile(
        books,
        index,
        &loaded,
        &ReconcileOptions {
            only: Some(vec![number.to_string()]),
            dry_run: false,
        },
    )
    .changed
    .into_iter()
    .next()
}

/// Where the journal came from, for the note printed when it was a guess.
enum Source {
    Given,
    Env,
    Recent,
}

/// The journal to open: JOURNAL or `-f`, then `$LEDGELINE_FIXTURE`, then the
/// most recent journal (shared with Ledgeline), or none. Never
/// `$LEDGER_FILE`.
fn find_journal(cli: &Cli) -> Result<Option<(PathBuf, Source)>, Failure> {
    if let (Some(a), Some(b)) = (&cli.journal, &cli.file)
        && a != b
    {
        return Err(Failure(
            format!("two journals given: {} and -f {}", a.display(), b.display()),
            None,
        ));
    }
    if let Some(path) = cli.journal.clone().or_else(|| cli.file.clone()) {
        return Ok(Some((path, Source::Given)));
    }
    if let Some(path) = std::env::var_os(FIXTURE_ENV).filter(|v| !v.is_empty()) {
        return Ok(Some((PathBuf::from(path), Source::Env)));
    }
    Ok(ledgeline_shell::recents::most_recent(&AppDirs::from_env())
        .map(|path| (path, Source::Recent)))
}

/// [`find_journal`], where a command needs one.
fn resolve_journal(cli: &Cli) -> Result<(PathBuf, Source), Failure> {
    find_journal(cli)?.ok_or_else(|| {
        Failure(
            format!(
                "no journal: pass one (ledgeline-invoicer -f books.journal list) or set ${FIXTURE_ENV}"
            ),
            None,
        )
    })
}

fn open(cli: &Cli) -> Result<Session, Failure> {
    let (journal, source) = resolve_journal(cli)?;
    match source {
        Source::Given => {}
        Source::Env => eprintln!(
            "ledgeline-invoicer: using ${FIXTURE_ENV} ({})",
            journal.display()
        ),
        Source::Recent => eprintln!(
            "ledgeline-invoicer: using the journal opened last ({})",
            journal.display()
        ),
    }
    Ok(Session::open_local(&journal)?)
}

fn today(cli: &Cli) -> Result<String, Failure> {
    match &cli.today {
        Some(date) if is_iso_date(date) => Ok(date.clone()),
        Some(date) => Err(Failure(
            format!("--today {date}: not a date (YYYY-MM-DD)"),
            None,
        )),
        None => Ok(chrono::Local::now().format("%Y-%m-%d").to_string()),
    }
}

/// A document argument: a local file inside the books folder, a number, or
/// a vault path.
fn document_path(session: &Session, target: &str) -> Result<VaultPath, Failure> {
    let local = Path::new(target);
    if local.is_file()
        && let Ok(canonical) = std::fs::canonicalize(local)
        && let Ok(path) = session.vault().vault_path(&canonical)
        && session.index().doc(&path).is_some()
    {
        return Ok(path);
    }
    Ok(session.find(target)?.path.clone())
}

/// Write `text` to stdout. A closed pipe (`… | head`) is not an error: the
/// reader has what it wanted.
fn out(text: &str) {
    use std::io::Write as _;
    let mut stdout = std::io::stdout().lock();
    let _ = stdout
        .write_all(text.as_bytes())
        .and_then(|()| stdout.flush());
}

/// Print `value` as JSON, or as text through `text`.
fn emit<T: Serialize>(json: bool, value: &T, text: impl FnOnce(&T) -> String) {
    if json {
        match serde_json::to_string_pretty(value) {
            Ok(json) => out(&format!("{json}\n")),
            Err(e) => eprintln!("ledgeline-invoicer: error: {e}"),
        }
    } else {
        out(&text(value));
    }
}

/// `new`'s rows, in command-line order across --row and --from-catalog.
fn new_rows(
    session: &Session,
    args: &NewArgs,
    matches: Option<&ArgMatches>,
) -> Result<Vec<DraftRow>, Failure> {
    let words = &session.settings().words;
    let index_of = |id: &str| -> Vec<usize> {
        matches
            .and_then(|m| m.indices_of(id))
            .map(Iterator::collect)
            .unwrap_or_default()
    };
    let catalogs = if args.from_catalog.is_empty() {
        Vec::new()
    } else {
        edit::catalogs(session.index())
    };
    let typed = args
        .row
        .iter()
        .zip(index_of("row").into_iter().chain(0..))
        .map(|(spec, at)| edit::row_spec(spec, words).map(|row| (at, row)));
    let from_catalog = args
        .from_catalog
        .iter()
        .zip(index_of("from_catalog").into_iter().chain(0..))
        .map(|(spec, at)| edit::catalog_row(&catalogs, spec).map(|row| (at, row)));
    let mut rows: Vec<(usize, DraftRow)> = typed.chain(from_catalog).collect::<Result<_, _>>()?;
    rows.sort_by_key(|(at, _)| *at);
    Ok(rows.into_iter().map(|(_, row)| row).collect())
}

/// The starter template `new` uses (PLAN 3b): the template `--template` or
/// the customer's `invoice_template` names, when that is a starter's path in
/// `templates-dir`; else, when the books have no invoice template at all,
/// the default starter. Returns the `template:` value to write when it is
/// the default starter's (otherwise the draft picks it as before), and the
/// starter to copy in once the invoice is written. Nothing for an unknown
/// customer: the draft reports that.
fn starter_for_new(
    session: &Session,
    customer: &str,
    template: Option<&str>,
) -> (Option<String>, Option<&'static starters::Starter>) {
    let dir = &session.settings().templates_dir;
    let Ok(customer) = views::find_customer(session.index(), customer) else {
        return (None, None);
    };
    match template.map(str::to_string).or(customer.invoice_template) {
        Some(value) => (None, starters::named(dir, &value)),
        None => {
            let has_one = templates::discover(session.vault(), session.index().site())
                .iter()
                .any(|t| t.supports(DocType::Invoice));
            if has_one {
                (None, None)
            } else {
                (
                    starters::template_value(dir, starters::DEFAULT),
                    Some(starters::DEFAULT),
                )
            }
        }
    }
}

fn run_new(cli: &Cli, args: &NewArgs, matches: Option<&ArgMatches>) -> Result<ExitCode, Failure> {
    let mut session = open(cli)?;
    let rows = new_rows(&session, args, matches)?;
    let extra = args
        .field
        .iter()
        .map(|f| edit::assignment(f))
        .collect::<Result<Vec<_>, WriteError>>()?;
    let (starter_template, starter) =
        starter_for_new(&session, &args.customer, args.template.as_deref());
    let new = NewInvoice {
        template: args.template.clone().or(starter_template),
        number: args.number.clone(),
        terms: args.terms,
        reference: args.reference.clone(),
        rows,
        extra,
        ..NewInvoice::new(
            &args.customer,
            &args.date.clone().map_or_else(|| today(cli), Ok)?,
        )
    };
    let booked: Vec<String> = session.booked_numbers().to_vec();
    let booked: Vec<&str> = booked.iter().map(String::as_str).collect();
    let (index, vault) = session.parts_mut();
    let created = edit::create(index, vault, &new, &booked, args.dry_run)?;
    // The starter it uses is copied in now that the invoice is written
    // (first use; an existing file is never replaced), and indexed so
    // `check` knows the template at once.
    let copied = match starter.filter(|_| !args.dry_run) {
        Some(starter) => {
            let dir = session.settings().templates_dir.clone();
            let installed = starters::install(session.vault(), &dir, starter).map_err(|e| {
                Failure(format!("copying the {} template: {e}", starter.name), None)
            })?;
            let (index, vault) = session.parts_mut();
            index.refresh(vault, &installed.written_files());
            installed.written.then_some(installed.path)
        }
        None => None,
    };
    let problems: Vec<_> = if args.dry_run {
        Vec::new()
    } else {
        views::problems(&session)
            .into_iter()
            .filter(|p| p.file.as_ref() == Some(&created.path))
            .collect()
    };
    #[derive(Serialize)]
    struct Out<'a> {
        #[serde(flatten)]
        created: &'a edit::Created,
        problems: Vec<invoicer_core::problem::Problem>,
        /// The starter template this invoice uses, copied in on first use.
        #[serde(skip_serializing_if = "Option::is_none")]
        template_copied: Option<&'a VaultPath>,
    }
    emit(
        cli.json,
        &Out {
            created: &created,
            problems,
            template_copied: copied.as_ref(),
        },
        |out| {
            let mut text = human::created(out.created, &out.problems);
            if let Some(path) = out.template_copied {
                text.push_str(&format!("  copied the starter template to {path}\n"));
            }
            text
        },
    );
    Ok(ExitCode::SUCCESS)
}

fn parse_status(text: &str) -> Result<Status, Failure> {
    Status::parse(text).ok_or_else(|| Failure(format!("--status {text}: not a status"), None))
}

fn run(cli: &Cli, matches: Option<&ArgMatches>) -> Result<ExitCode, Failure> {
    let Some(command) = &cli.command else {
        return run_app(cli);
    };
    match command {
        Command::New(args) => run_new(cli, args, matches),
        Command::List(args) => {
            let session = open(cli)?;
            let query = ListQuery {
                filter: ListFilter {
                    doc_type: args
                        .doc_type
                        .as_deref()
                        .map(|t| {
                            DocType::parse(t)
                                .ok_or_else(|| Failure(format!("--type {t}: not a type"), None))
                        })
                        .transpose()?,
                    status: args.status.as_deref().map(parse_status).transpose()?,
                    customer: args.customer.clone(),
                },
                open: args.open,
                overdue: args.overdue,
                ..ListQuery::default()
            };
            let problems = views::document_problems(&session);
            let entries = views::list(
                session.index(),
                Some(session.book_states()),
                &problems,
                &query,
                &today(cli)?,
            );
            emit(cli.json, &entries, |e| human::list(e));
            Ok(ExitCode::SUCCESS)
        }
        Command::Show { document } => {
            let session = open(cli)?;
            let path = document_path(&session, document)?;
            let doc = session
                .index()
                .doc(&path)
                .ok_or_else(|| Failure(format!("{path} isn't a document"), None))?;
            let mut problems = views::document_problems(&session);
            if let Some(number) = doc.number() {
                problems.extend(views::books_problems(&session, Some(&[number.to_string()])));
            }
            let detail = views::show(&session, doc, &problems, &today(cli)?)?;
            emit(cli.json, &detail, human::show);
            Ok(ExitCode::SUCCESS)
        }
        Command::Set(args) => {
            if args.assignments.is_empty() && args.unset.is_empty() {
                return Err(Failure(
                    "nothing to set: give key=value or --unset key".into(),
                    None,
                ));
            }
            let mut session = open(cli)?;
            let path = document_path(&session, &args.document)?;
            let edits: Vec<FieldEdit> = args
                .assignments
                .iter()
                .map(|a| edit::assignment(a).map(|(k, v)| FieldEdit::Set(k, v)))
                .chain(args.unset.iter().map(|k| Ok(FieldEdit::Remove(k.clone()))))
                .collect::<Result<_, WriteError>>()?;
            let (index, vault) = session.parts_mut();
            let outcome = edit::set(index, vault, &path, &edits)?;
            // A `template:` naming a starter copies it in on first use (B1),
            // as `new` does; never over a file already there.
            let dir = session.settings().templates_dir.clone();
            let mut copied = None;
            for change in &edits {
                if let FieldEdit::Set(key, Value::String(template)) = change
                    && key == "template"
                {
                    let installed = starters::install_named(session.vault(), &dir, template)
                        .map_err(|e| Failure(format!("copying the starter template: {e}"), None))?;
                    if let Some(done) = installed.filter(|d| d.written) {
                        let (index, vault) = session.parts_mut();
                        index.refresh(vault, &done.written_files());
                        copied = Some(done.path);
                    }
                }
            }
            #[derive(Serialize)]
            struct Out<'a> {
                #[serde(flatten)]
                outcome: &'a edit::SetOutcome,
                /// The starter template it now names, copied in on first use.
                #[serde(skip_serializing_if = "Option::is_none")]
                template_copied: Option<&'a VaultPath>,
            }
            emit(
                cli.json,
                &Out {
                    outcome: &outcome,
                    template_copied: copied.as_ref(),
                },
                |out| {
                    let mut text = human::set(out.outcome);
                    if let Some(path) = out.template_copied {
                        text.push_str(&format!("  copied the starter template to {path}\n"));
                    }
                    text
                },
            );
            Ok(ExitCode::SUCCESS)
        }
        Command::Recalc(args) => {
            let mut session = open(cli)?;
            let targets = args
                .documents
                .iter()
                .map(|d| document_path(&session, d))
                .collect::<Result<Vec<_>, _>>()?;
            let (index, vault) = session.parts_mut();
            let report = edit::recalc(index, vault, &targets, args.dry_run);
            emit(cli.json, &report, |r| human::recalc(r, args.dry_run));
            Ok(if report.failed.is_empty() {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(2)
            })
        }
        Command::Check { strict } => {
            let session = open(cli)?;
            let report = views::check(&session);
            emit(cli.json, &report, human::check);
            let failed = report.counts.errors > 0 || (*strict && report.counts.warnings > 0);
            Ok(if failed {
                ExitCode::from(1)
            } else {
                ExitCode::SUCCESS
            })
        }
        Command::NextNumber => {
            let session = open(cli)?;
            let next = session.explain_next_number()?;
            /// `after` and `source` say what it follows (PLAN H19).
            #[derive(Serialize)]
            struct Out {
                next: String,
                after: Option<String>,
                source: &'static str,
            }
            emit(
                cli.json,
                &Out {
                    next: next.number.to_string(),
                    after: next.after,
                    source: next.source.as_str(),
                },
                |o| format!("{}\n", o.next),
            );
            Ok(ExitCode::SUCCESS)
        }
        Command::Config => {
            let session = open(cli)?;
            let view = views::config(&session);
            if cli.json {
                emit(true, &view, |_| String::new());
            } else {
                out(&human::config(&view)?);
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Customers(CustomersCommand::List) => {
            let session = open(cli)?;
            let all = views::customers(session.index(), Some(session.book_states()));
            emit(cli.json, &all, |c| human::customers(c));
            Ok(ExitCode::SUCCESS)
        }
        Command::Customers(CustomersCommand::Show { name }) => {
            let session = open(cli)?;
            let customer = views::find_customer(session.index(), name).map_err(|e| match e {
                invoicer_core::customers::CustomerLookup::NotFound => {
                    Failure(format!("no customer is called {name:?}"), None)
                }
                invoicer_core::customers::CustomerLookup::NotACustomer(path) => Failure(
                    format!(
                        "{path} isn't a person or organization tagged `{}`",
                        session.settings().words.customer_tag
                    ),
                    None,
                ),
            })?;
            let problems = views::document_problems(&session);
            let detail = views::customer_detail(&session, &customer, &problems, &today(cli)?);
            emit(cli.json, &detail, human::customer);
            Ok(ExitCode::SUCCESS)
        }
        Command::Customers(CustomersCommand::New(args)) => customers::run_new(cli, args),
        Command::Customers(CustomersCommand::ImportCsv(args)) => customers::run_import(cli, args),
        Command::Send(args) => run_send(cli, args),
        Command::Void(args) => run_void(cli, args),
        Command::Pdf(args) => run_pdf(cli, args),
        Command::Rebook(args) => run_rebook(cli, args),
        Command::ApplyPayment(args) => run_apply_payment(cli, args),
        Command::Reconcile(args) => run_reconcile(cli, args),
        Command::Receivables(args) => {
            let session = open(cli)?;
            let view = invoicer_core::receivables::receivables(
                session.index(),
                session.book_states(),
                &today(cli)?,
                args.dso_days.unwrap_or(session.settings().dso_days),
            );
            emit(cli.json, &view, |v| {
                human::receivables(v, session.index().money(), args.aging)
            });
            Ok(ExitCode::SUCCESS)
        }
        Command::Recognize(args) => {
            let session = open(cli)?;
            let books = session.invoice_books();
            let request = Recognize {
                today: today(cli)?,
                through: args.through.clone(),
                include_closed: args.include_closed,
                dry_run: args.dry_run,
                expected: None,
            };
            let done = retrying(|| books.recognize(&request))?;
            emit(cli.json, &done, human::recognized);
            Ok(ExitCode::SUCCESS)
        }
    }
}

/// No subcommand: the app (the window, or `--server`). `--json` means a
/// script is asking, and a script never wants a window: it needs a command.
#[cfg(feature = "server")]
fn run_app(cli: &Cli) -> Result<ExitCode, Failure> {
    if cli.json {
        return Err(Failure(
            "--json needs a command (see --help)".to_string(),
            None,
        ));
    }
    let args = app::AppArgs {
        journal: find_journal(cli)?.map(|(path, _)| path),
        server: cli.server,
        host: cli.host.clone(),
        port: cli.port,
        allow_origin: cli.allow_origin.clone(),
    };
    app::run(&args).map_err(|message| Failure(message, None))?;
    Ok(ExitCode::SUCCESS)
}

/// No subcommand in a CLI-only build.
#[cfg(not(feature = "server"))]
fn run_app(_cli: &Cli) -> Result<ExitCode, Failure> {
    Err(Failure(
        "built without the app (the `server` and `gui` features); run a command (see --help)"
            .to_string(),
        None,
    ))
}

fn run_send(cli: &Cli, args: &SendArgs) -> Result<ExitCode, Failure> {
    let mut session = open(cli)?;
    let number = number_of(&session, &args.document)?;
    let books = session.invoice_books();
    if args.dry_run {
        let preview = books
            .preview(session.index(), &number)
            .map_err(books_failure)?;
        emit(cli.json, &preview, |p| {
            human::preview(p, session.index().money())
        });
        return Ok(ExitCode::SUCCESS);
    }
    let request = Send {
        number,
        today: today(cli)?,
        sent_via: args.via.clone(),
        expected: None,
    };
    let (index, _) = session.parts_mut();
    let sent = retrying(|| books.send(index, &request))?;
    emit(cli.json, &sent, human::sent);
    Ok(ExitCode::SUCCESS)
}

fn run_pdf(cli: &Cli, args: &PdfArgs) -> Result<ExitCode, Failure> {
    let (journal, _) = resolve_journal(cli)?;
    // Resolve the invoice (and report a bad one) before any window system
    // is touched.
    let number = {
        let session = open(cli)?;
        number_of(&session, &args.document)?
    };
    let output = match &args.output {
        Some(path) if path.is_absolute() => Some(path.clone()),
        Some(path) => Some(std::env::current_dir()?.join(path)),
        None => None,
    };
    let exported = export_pdf(&journal, number, output, today(cli)?)?;
    emit(cli.json, &exported, human::exported);
    Ok(ExitCode::SUCCESS)
}

#[cfg(feature = "gui")]
fn export_pdf(
    journal: &Path,
    number: String,
    output: Option<PathBuf>,
    today: String,
) -> Result<pdf::Exported, Failure> {
    app::pdf::run_cli(
        journal,
        app::pdf::ExportRequest {
            target: number,
            output,
            temp: false,
            today,
        },
    )
    .map_err(|message| Failure(message, None))
}

#[cfg(not(feature = "gui"))]
fn export_pdf(
    _journal: &Path,
    _number: String,
    _output: Option<PathBuf>,
    _today: String,
) -> Result<pdf::Exported, Failure> {
    Err(Failure(pdf::NEEDS_GUI.to_string(), Some("needs-gui")))
}

fn run_void(cli: &Cli, args: &VoidArgs) -> Result<ExitCode, Failure> {
    let mut session = open(cli)?;
    let number = number_of(&session, &args.document)?;
    let books = session.invoice_books();
    let today = today(cli)?;
    let request = Void {
        number,
        date: args.date.clone().unwrap_or_else(|| today.clone()),
        today,
        expected: None,
    };
    let (index, _) = session.parts_mut();
    let voided = retrying(|| books.void(index, &request))?;
    emit(cli.json, &voided, human::voided);
    Ok(ExitCode::SUCCESS)
}

fn run_rebook(cli: &Cli, args: &RebookArgs) -> Result<ExitCode, Failure> {
    let mut session = open(cli)?;
    let number = number_of(&session, &args.document)?;
    let books = session.invoice_books();
    if args.dry_run {
        let preview = books
            .preview(session.index(), &number)
            .map_err(books_failure)?;
        emit(cli.json, &preview, |p| {
            human::preview(p, session.index().money())
        });
        return Ok(ExitCode::SUCCESS);
    }
    let request = Rebook {
        number: number.clone(),
        today: today(cli)?,
        expected: None,
    };
    let (index, _) = session.parts_mut();
    let rebooked = retrying(|| books.rebook(index, &request))?;
    // A new total can move the payment state (accrual: a link to check).
    let mirror = if rebooked.changed {
        mirror_one(&books, index, &number)
    } else {
        None
    };
    #[derive(Serialize)]
    struct Out {
        #[serde(flatten)]
        rebooked: books::Rebooked,
        mirror: Option<MirrorChange>,
    }
    emit(cli.json, &Out { rebooked, mirror }, |o| {
        human::rebooked(&o.rebooked, o.mirror.as_ref())
    });
    Ok(ExitCode::SUCCESS)
}

fn run_apply_payment(cli: &Cli, args: &ApplyPaymentArgs) -> Result<ExitCode, Failure> {
    let mut session = open(cli)?;
    let number = number_of(&session, &args.document)?;
    let books = session.invoice_books();
    let today = today(cli)?;
    if args.undo {
        return undo_payment(cli, &mut session, &books, &number, args.txn.as_deref());
    }
    let Some(reference) = &args.txn else {
        let options = books
            .payment_options(session.index(), &number, &today)
            .map_err(books_failure)?;
        emit(cli.json, &options, |o| {
            human::payment_options(o, session.index().money())
        });
        return Ok(ExitCode::SUCCESS);
    };
    let request = ApplyPayment {
        number: number.clone(),
        reference: reference.clone(),
        today: today.clone(),
        expected: None,
    };
    let money = session.index().money().clone();
    let (index, vault) = session.parts_mut();
    let applied = retrying(|| books.apply_payment(index, &request))?;
    let (undo_file, warning) = books::undo::keep(vault, &applied, &today);
    let warnings: Vec<String> = warning.into_iter().collect();
    let mirror = mirror_one(&books, index, &number);
    #[derive(Serialize)]
    struct Out {
        #[serde(flatten)]
        applied: books::PaymentApplied,
        undo_file: Option<VaultPath>,
        mirror: Option<MirrorChange>,
        warnings: Vec<String>,
    }
    emit(
        cli.json,
        &Out {
            applied,
            undo_file,
            mirror,
            warnings,
        },
        |o| {
            human::payment_applied(
                &o.applied,
                &money,
                o.undo_file.as_ref(),
                o.mirror.as_ref(),
                &o.warnings,
            )
        },
    );
    Ok(ExitCode::SUCCESS)
}

fn undo_payment(
    cli: &Cli,
    session: &mut Session,
    books: &InvoiceBooks,
    number: &str,
    txn: Option<&str>,
) -> Result<ExitCode, Failure> {
    let (index, vault) = session.parts_mut();
    let records: Vec<_> = books::undo::records(vault, number)
        .into_iter()
        .filter(|(_, r)| txn.is_none_or(|t| r.reference.starts_with(t.trim())))
        .collect();
    let (file, record) = match records.as_slice() {
        [] => {
            return Err(Failure(
                format!(
                    "no undo record for a payment applied to invoice {number}{} (in {}); undo it \
                     by hand from the deposit's text",
                    txn.map(|t| format!(" matching {t}")).unwrap_or_default(),
                    books::UNDO_DIR
                ),
                None,
            ));
        }
        [one] => one.clone(),
        many => {
            let listed: Vec<String> = many
                .iter()
                .map(|(_, r)| {
                    format!(
                        "{} ({} {})",
                        r.reference.chars().take(12).collect::<String>(),
                        r.deposit_date,
                        r.deposit_description
                    )
                })
                .collect();
            return Err(Failure(
                format!(
                    "{} payments are applied to invoice {number}; name one with --txn: {}",
                    many.len(),
                    listed.join(", ")
                ),
                None,
            ));
        }
    };
    let undo = record.undo();
    retrying(|| books.undo_payment(&undo, None))?;
    let mut warnings = Vec::new();
    if let Err(error) = books::undo::remove(vault, &file) {
        warnings.push(format!("the undo record {file} wasn't removed: {error}"));
    }
    let mirror = mirror_one(books, index, number);
    #[derive(Serialize)]
    struct Out {
        number: String,
        reference: String,
        deposit_date: String,
        deposit_description: String,
        restored_text: String,
        undo_file: VaultPath,
        mirror: Option<MirrorChange>,
        warnings: Vec<String>,
    }
    emit(
        cli.json,
        &Out {
            number: number.to_string(),
            reference: record.reference.clone(),
            deposit_date: record.deposit_date.clone(),
            deposit_description: record.deposit_description.clone(),
            restored_text: record.original_text.clone(),
            undo_file: file,
            mirror,
            warnings,
        },
        |o| {
            human::payment_undone(
                &o.number,
                &o.deposit_date,
                &o.deposit_description,
                o.mirror.as_ref(),
                &o.warnings,
            )
        },
    );
    Ok(ExitCode::SUCCESS)
}

fn run_reconcile(cli: &Cli, args: &ReconcileArgs) -> Result<ExitCode, Failure> {
    let mut session = open(cli)?;
    let only = if args.documents.is_empty() {
        None
    } else {
        Some(
            args.documents
                .iter()
                .map(|d| number_of(&session, d))
                .collect::<Result<Vec<_>, _>>()?,
        )
    };
    let books = session.invoice_books();
    let loaded = books.load().map_err(books_failure)?;
    let (index, _) = session.parts_mut();
    let report = reconcile::reconcile(
        &books,
        index,
        &loaded,
        &ReconcileOptions {
            only,
            dry_run: args.dry_run,
        },
    );
    emit(cli.json, &report, |r| human::reconciled(r, args.dry_run));
    Ok(if !report.failed.is_empty() {
        ExitCode::from(2)
    } else if report.counts.errors > 0 {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    })
}

/// Names a variable holding a secret, by the usual naming conventions
/// (`GITHUB_TOKEN`, `AWS_SECRET_ACCESS_KEY`, `OPENAI_API_KEY`,
/// `DATABASE_PASSWORD`). Ledgeline's own (`LEDGELINE_*`, such as the
/// invoicer's token variable, which the server reads later) are kept.
fn secret_name(name: &str) -> bool {
    const MARKERS: [&str; 9] = [
        "TOKEN",
        "SECRET",
        "PASSWORD",
        "PASSWD",
        "PASSPHRASE",
        "CREDENTIAL",
        "API_KEY",
        "APIKEY",
        "PRIVATE_KEY",
    ];
    let name = name.to_ascii_uppercase();
    !name.starts_with("LEDGELINE_")
        && (name.starts_with("AWS_")
            || name.ends_with("_KEY")
            || MARKERS.iter().any(|m| name.contains(m)))
}

/// SEC-2: every Tera engine offers templates `get_env(name=…)`, and the
/// engines that render invoices are MBR's (mbr-core =0.7.4), which gives no
/// way to replace that function. A template is a file people share, so
/// before the app or `pdf` renders any, the process forgets the variables
/// named like secrets ([`secret_name`]); `get_env` then finds nothing to
/// leak. Other variables stay readable until MBR can disable `get_env`.
fn forget_secret_env() {
    let secrets: Vec<std::ffi::OsString> = std::env::vars_os()
        .map(|(name, _)| name)
        .filter(|name| name.to_str().is_some_and(secret_name))
        .collect();
    for name in secrets {
        // SAFETY: called first thing in `main`, before this process starts
        // any thread, so nothing reads the environment concurrently.
        unsafe { std::env::remove_var(name) };
    }
}

fn main() -> ExitCode {
    let matches = Cli::command().get_matches();
    let cli = match Cli::from_arg_matches(&matches) {
        Ok(cli) => cli,
        Err(e) => e.exit(),
    };
    if matches!(cli.command, None | Some(Command::Pdf(_))) {
        forget_secret_env();
    }
    let sub = matches.subcommand().map(|(_, m)| m);
    match run(&cli, sub) {
        Ok(code) => code,
        Err(Failure(message, code)) => {
            if cli.json {
                let value = match code {
                    Some(code) => serde_json::json!({ "error": message, "code": code }),
                    None => serde_json::json!({ "error": message }),
                };
                out(&format!("{value}\n"));
            } else {
                eprintln!("ledgeline-invoicer: error: {message}");
            }
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::secret_name;

    #[test]
    fn secret_names_are_forgotten_and_others_kept() {
        for name in [
            "GITHUB_TOKEN",
            "AWS_SECRET_ACCESS_KEY",
            "AWS_PROFILE",
            "OPENAI_API_KEY",
            "anthropic_api_key",
            "DATABASE_PASSWORD",
            "NPM_AUTH_TOKEN",
            "GPG_PASSPHRASE",
            "STRIPE_KEY",
            "GOOGLE_APPLICATION_CREDENTIALS",
        ] {
            assert!(secret_name(name), "{name}");
        }
        for name in [
            "PATH",
            "HOME",
            "TMPDIR",
            "LANG",
            "XDG_CONFIG_HOME",
            "LEDGELINE_FIXTURE",
            "LEDGELINE_CONFIG_DIR",
            "LEDGELINE_INVOICER_TOKEN",
            "KEYBOARD_LAYOUT",
        ] {
            assert!(!secret_name(name), "{name}");
        }
    }
}
