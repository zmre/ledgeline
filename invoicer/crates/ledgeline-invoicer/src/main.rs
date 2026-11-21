//! `ledgeline-invoicer` — Ledgeline Invoicer's binary (`invoicer/ARCHITECTURE.md`
//! §2.2, §6.4).
//!
//! The read-only CLI (invoicer PLAN 1c): `new`, `list`, `show`, `set`,
//! `recalc`, `check`, `next-number`, `config` and `customers list|show`.
//! "Read-only" is about the books: `new`, `set` and `recalc` write markdown
//! documents, never the journal (I2). Every command takes `--json`.
//!
//! This file parses arguments, opens the books and prints; each operation is
//! one `invoicer_core` function (`session`, `views`, `edit`), shared with the
//! API to come. The GUI arrives in Phase 3.
//!
//! Exit codes: 0 done; 1 `check` found problems; 2 the command couldn't run
//! (a usage error, books that don't open, a refused write).

mod human;

use clap::{ArgMatches, CommandFactory, FromArgMatches, Parser, Subcommand};
use invoicer_core::doc::{DocType, Status};
use invoicer_core::draft::{DraftRow, NewInvoice};
use invoicer_core::edit::{self, WriteError};
use invoicer_core::frontmatter::FieldEdit;
use invoicer_core::index::ListFilter;
use invoicer_core::session::Session;
use invoicer_core::table::books::is_iso_date;
use invoicer_core::views::{self, ListQuery};
use ledgeline_core::vault::VaultPath;
use ledgeline_shell::AppDirs;
use serde::Serialize;
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
}

/// A failure that stops a command (exit 2).
struct Failure(String);

impl<E: Display> From<E> for Failure {
    fn from(error: E) -> Self {
        Self(error.to_string())
    }
}

/// Where the journal came from, for the note printed when it was a guess.
enum Source {
    Given,
    Env,
    Recent,
}

/// The journal to open: JOURNAL or `-f`, then `$LEDGELINE_FIXTURE`, then the
/// most recent journal (shared with Ledgeline). Never `$LEDGER_FILE`.
fn resolve_journal(cli: &Cli) -> Result<(PathBuf, Source), Failure> {
    if let (Some(a), Some(b)) = (&cli.journal, &cli.file)
        && a != b
    {
        return Err(Failure(format!(
            "two journals given: {} and -f {}",
            a.display(),
            b.display()
        )));
    }
    if let Some(path) = cli.journal.clone().or_else(|| cli.file.clone()) {
        return Ok((path, Source::Given));
    }
    if let Some(path) = std::env::var_os(FIXTURE_ENV).filter(|v| !v.is_empty()) {
        return Ok((PathBuf::from(path), Source::Env));
    }
    ledgeline_shell::recents::most_recent(&AppDirs::from_env())
        .map(|path| (path, Source::Recent))
        .ok_or_else(|| {
            Failure(format!(
                "no journal: pass one (ledgeline-invoicer -f books.journal list) or set ${FIXTURE_ENV}"
            ))
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
        Some(date) => Err(Failure(format!("--today {date}: not a date (YYYY-MM-DD)"))),
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
        edit::catalogs(session.index(), session.vault())
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

fn run_new(cli: &Cli, args: &NewArgs, matches: Option<&ArgMatches>) -> Result<ExitCode, Failure> {
    let mut session = open(cli)?;
    let rows = new_rows(&session, args, matches)?;
    let extra = args
        .field
        .iter()
        .map(|f| edit::assignment(f))
        .collect::<Result<Vec<_>, WriteError>>()?;
    let new = NewInvoice {
        template: args.template.clone(),
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
    }
    emit(
        cli.json,
        &Out {
            created: &created,
            problems,
        },
        |out| human::created(out.created, &out.problems),
    );
    Ok(ExitCode::SUCCESS)
}

fn parse_status(text: &str) -> Result<Status, Failure> {
    Status::parse(text).ok_or_else(|| Failure(format!("--status {text}: not a status")))
}

fn run(cli: &Cli, matches: Option<&ArgMatches>) -> Result<ExitCode, Failure> {
    let Some(command) = &cli.command else {
        return Err(Failure(
            "the invoicer's window arrives in a later release; run a command (see --help)"
                .to_string(),
        ));
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
                                .ok_or_else(|| Failure(format!("--type {t}: not a type")))
                        })
                        .transpose()?,
                    status: args.status.as_deref().map(parse_status).transpose()?,
                    customer: args.customer.clone(),
                },
                open: args.open,
                overdue: args.overdue,
            };
            let problems = views::problems(&session);
            let entries = views::list(session.index(), &problems, &query, &today(cli)?);
            emit(cli.json, &entries, |e| human::list(e));
            Ok(ExitCode::SUCCESS)
        }
        Command::Show { document } => {
            let session = open(cli)?;
            let path = document_path(&session, document)?;
            let doc = session
                .index()
                .doc(&path)
                .ok_or_else(|| Failure(format!("{path} isn't a document")))?;
            let problems = views::problems(&session);
            let detail = views::show(&session, doc, &problems, &today(cli)?)?;
            emit(cli.json, &detail, human::show);
            Ok(ExitCode::SUCCESS)
        }
        Command::Set(args) => {
            if args.assignments.is_empty() && args.unset.is_empty() {
                return Err(Failure(
                    "nothing to set: give key=value or --unset key".into(),
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
            emit(cli.json, &outcome, human::set);
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
            let next = session.next_number()?;
            #[derive(Serialize)]
            struct Out {
                next: String,
            }
            emit(
                cli.json,
                &Out {
                    next: next.to_string(),
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
            let all = views::customers(session.index());
            emit(cli.json, &all, |c| human::customers(c));
            Ok(ExitCode::SUCCESS)
        }
        Command::Customers(CustomersCommand::Show { name }) => {
            let session = open(cli)?;
            let customer = views::find_customer(session.index(), name).map_err(|e| match e {
                invoicer_core::customers::CustomerLookup::NotFound => {
                    Failure(format!("no customer is called {name:?}"))
                }
                invoicer_core::customers::CustomerLookup::NotACustomer(path) => Failure(format!(
                    "{path} isn't a person or organization tagged `{}`",
                    session.settings().words.customer_tag
                )),
            })?;
            let problems = views::problems(&session);
            let detail = views::customer_detail(&session, &customer, &problems, &today(cli)?);
            emit(cli.json, &detail, human::customer);
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn main() -> ExitCode {
    let matches = Cli::command().get_matches();
    let cli = match Cli::from_arg_matches(&matches) {
        Ok(cli) => cli,
        Err(e) => e.exit(),
    };
    let sub = matches.subcommand().map(|(_, m)| m);
    match run(&cli, sub) {
        Ok(code) => code,
        Err(Failure(message)) => {
            if cli.json {
                out(&format!("{}\n", serde_json::json!({ "error": message })));
            } else {
                eprintln!("ledgeline-invoicer: error: {message}");
            }
            ExitCode::from(2)
        }
    }
}
