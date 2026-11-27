//! `ledgeline-invoicer time …` (plan 46 T14): the tracker and billing from
//! the command line, so Raycast, Alfred, Stream Deck or a shortcut can drive
//! it before the app has global hotkeys. Every command takes `--json` and is
//! one [`invoicer_core::time`] call, shared with the app's `/api/time`.
//!
//! The tracker's person, pins and slot 0 are the app's (`ui-prefs.toml
//! [time]`, read as the app reads them), so `time toggle 2` toggles the
//! same slot the app shows.

use crate::{Cli, Failure, emit, open, today};
use invoicer_core::session::Session;
use invoicer_core::time::bill::{self, BillRequest, GroupChoice, Grouping, Release};
use invoicer_core::time::entries::{self, EntryQuery};
use invoicer_core::time::link::Detail;
use invoicer_core::time::pay::{self, PayLines, PayRequest};
use invoicer_core::time::report;
use invoicer_core::time::track::{self, TrackState};
use invoicer_core::time::{Locks, TimeBooks, TimeOpError, TimePrefs, parse_moment};
use ledgeline_core::time::book;
use ledgeline_core::time::timeclock::Moment;
use ledgeline_core::ui_prefs::{self as core, Patch, UiPrefsSession};
use ledgeline_shell::AppDirs;
use serde_json::{Value, json};
use std::fmt::Write as _;
use std::process::ExitCode;

/// `time`'s arguments.
#[derive(clap::Args, Debug)]
pub(crate) struct TimeArgs {
    /// The moment to treat as now (`YYYY-MM-DDTHH:MM:SS`). Default: the
    /// local clock.
    #[arg(long, value_name = "DATETIME", global = true)]
    now: Option<String>,
    /// Whose time (P-person). Default: `ui-prefs.toml [time] person`.
    #[arg(long, value_name = "NAME", global = true)]
    person: Option<String>,
    #[command(subcommand)]
    command: TimeCommand,
}

#[derive(clap::Subcommand, Debug)]
enum TimeCommand {
    /// What is running, the slots and today's totals.
    Status,
    /// Start timing an account (stopping whatever runs).
    Start {
        /// The time account (`clients:acme:advisory`).
        account: String,
    },
    /// Stop the running timer.
    Stop {
        /// Stop at this moment instead of now (`YYYY-MM-DDTHH:MM:SS`).
        #[arg(long, value_name = "DATETIME")]
        at: Option<String>,
        /// Discard it: bank nothing.
        #[arg(long)]
        discard: bool,
    },
    /// Toggle slot N (1–9, 0) or an account: stop it if it runs, else
    /// switch to it.
    Toggle {
        /// A slot number or an account.
        slot: String,
    },
    /// Add (or with a negative number take) minutes; on the running slot,
    /// move its start.
    Adjust {
        /// A slot number or an account.
        slot: String,
        /// Minutes, signed (`-5`). Default: the step.
        #[arg(allow_hyphen_values = true)]
        minutes: Option<i64>,
    },
    /// Set the note of a slot's (or an account's) line today.
    Note {
        /// A slot number or an account.
        slot: String,
        /// The note.
        note: String,
    },
    /// Entries: the time lines, filtered.
    Log(LogArgs),
    /// Unbilled time per customer, with the periods to bill.
    Unbilled,
    /// Unpaid time per contractor (timekeeper notes).
    Unpaid {
        /// Everyone's, not only timekeepers'.
        #[arg(long)]
        everyone: bool,
    },
    /// Set up time tracking: create the roll-up, the accounts file and the
    /// first tracker file, or adopt an existing file.
    Setup(SetupArgs),
    /// Bill a customer's time: preview, or create the draft.
    Bill(BillArgs),
    /// Pay a contractor's time: an accounts-payable bill.
    Pay(PayArgs),
    /// Take `invoice: N` off its time (or make it nobill): for an orphan
    /// tag; refused while invoice N is a draft or sent and not void (delete
    /// or void it instead). With `--bill REF`: void the accounts-payable
    /// bill REF in the books and unlock its time.
    Release {
        /// The invoice number.
        #[arg(required_unless_present = "bill", conflicts_with = "bill")]
        invoice: Option<String>,
        /// The accounts-payable bill's reference (`bill:` on its time).
        #[arg(long, value_name = "REF")]
        bill: Option<String>,
        /// Mark the time nobill instead of unbilled.
        #[arg(long)]
        nobill: bool,
    },
}

#[derive(clap::Args, Debug)]
struct LogArgs {
    /// From this date (YYYY-MM-DD).
    #[arg(long)]
    from: Option<String>,
    /// Through this date.
    #[arg(long)]
    to: Option<String>,
    /// One customer's.
    #[arg(long)]
    customer: Option<String>,
    /// `unbilled`, `invoiced`, `nobill`, `not-billable`, `billed`.
    #[arg(long)]
    billed: Option<String>,
    /// This account and below.
    #[arg(long)]
    account: Option<String>,
    /// Everyone's time, not only --person's.
    #[arg(long)]
    all: bool,
}

#[derive(clap::Args, Debug)]
struct SetupArgs {
    /// Use this existing roll-up or time file instead of creating ours.
    #[arg(long, value_name = "FILE")]
    adopt: Option<String>,
    /// Don't seed `clients:<customer>` accounts.
    #[arg(long)]
    no_seed: bool,
    /// Show what would be written.
    #[arg(long)]
    dry_run: bool,
}

#[derive(clap::Args, Debug)]
struct BillArgs {
    /// The customer.
    customer: String,
    /// From this date. Default: the suggested period's.
    #[arg(long)]
    from: Option<String>,
    /// Through this date.
    #[arg(long)]
    to: Option<String>,
    /// `account` (default), `account-person` or `account-note`.
    #[arg(long, default_value = "account")]
    group: String,
    /// A group's item: `GROUP=CODE` (repeatable).
    #[arg(long, value_name = "GROUP=CODE")]
    item: Vec<String>,
    /// A group's rate: `GROUP=$250` (repeatable).
    #[arg(long, value_name = "GROUP=RATE")]
    rate: Vec<String>,
    /// Remember each group's item (`item:` on the account).
    #[arg(long)]
    remember: bool,
    /// `none`, `daily` or `entries`. Default: the customer's.
    #[arg(long)]
    detail: Option<String>,
    /// The invoice date. Default: today.
    #[arg(long)]
    date: Option<String>,
    /// `reference`.
    #[arg(long)]
    reference: Option<String>,
    /// Show the bill; write nothing.
    #[arg(long)]
    dry_run: bool,
}

#[derive(clap::Args, Debug)]
struct PayArgs {
    /// The person (`person:` on the time cards).
    person: String,
    /// From this date. Default: the suggested period's.
    #[arg(long)]
    from: Option<String>,
    /// Through this date.
    #[arg(long)]
    to: Option<String>,
    /// The hourly rate (`$85`), when the note has none.
    #[arg(long)]
    rate: Option<String>,
    /// The expense account.
    #[arg(long)]
    expense_account: Option<String>,
    /// The payable account.
    #[arg(long)]
    payable_account: Option<String>,
    /// The note to use when none links the person.
    #[arg(long)]
    note: Option<String>,
    /// One line in total instead of one per account.
    #[arg(long)]
    total: bool,
    /// Don't save the answers to the note.
    #[arg(long)]
    no_save: bool,
    /// The bill's date. Default: today.
    #[arg(long)]
    date: Option<String>,
    /// Show the bill (and what it needs); write nothing.
    #[arg(long)]
    dry_run: bool,
}

fn time_failure(error: &TimeOpError) -> Failure {
    let mut message = error.to_string();
    if let TimeOpError::Note {
        suggestion: Some(suggestion),
        ..
    } = error
    {
        let _ = write!(message, " (for example: {suggestion:?})");
    }
    Failure(message, Some(error.code()))
}

/// A time error as a failure, its code kept.
fn fail(error: impl Into<TimeOpError>) -> Failure {
    time_failure(&error.into())
}

/// The ui-prefs context the app uses, from the files (never the browser).
fn with_prefs<T>(session: &Session, f: impl FnOnce(&core::Context<'_>) -> T) -> T {
    let (ledger, ledger_id) = crate::matching::ledger_of(&session.shared_vault(), session.main());
    let prefs = UiPrefsSession::default();
    let ctx = core::Context {
        schema: &crate::matching::SCHEMA,
        session: &prefs,
        global_file: AppDirs::from_env().config_file(core::GLOBAL_FILE),
        ledger,
        ledger_id: Some(ledger_id),
    };
    f(&ctx)
}

fn prefs_of(session: &Session, args: &TimeArgs) -> TimePrefs {
    let mut prefs = with_prefs(session, |ctx| {
        TimePrefs::from_prefs(core::current(ctx).prefs())
    });
    if let Some(person) = &args.person {
        prefs.person = Some(person.trim().to_string()).filter(|p| !p.is_empty());
    }
    prefs
}

fn save_prefs(session: &Session, time: Value) {
    let result = Patch::from_json(&crate::matching::SCHEMA, &json!({ "time": time }))
        .map_err(|e| format!("{e:?}"))
        .and_then(|patch| {
            with_prefs(session, |ctx| core::apply(ctx, &patch)).map_err(|e| format!("{e:?}"))
        });
    if let Err(error) = result {
        eprintln!("ledgeline-invoicer: the time preference wasn't saved: {error}");
    }
}

fn now_of(args: &TimeArgs) -> Result<Moment, Failure> {
    match &args.now {
        Some(text) => parse_moment(text)
            .ok_or_else(|| Failure(format!("--now {text}: not YYYY-MM-DDTHH:MM:SS"), None)),
        None => {
            let local = chrono::Local::now();
            Ok(Moment::new(
                local.format("%Y-%m-%d").to_string(),
                invoicer_core::time::parse_time_of_day(&local.format("%H:%M:%S").to_string())
                    .unwrap_or(0),
            ))
        }
    }
}

fn time_books(session: &Session) -> Result<TimeBooks, Failure> {
    TimeBooks::open(session.shared_vault(), session.main().clone()).map_err(fail)
}

/// A slot number (0–9) or an account, as an account.
fn account_of(books: &TimeBooks, prefs: &TimePrefs, slot: &str) -> Result<(String, bool), Failure> {
    match slot.trim().parse::<u8>() {
        Ok(key) if key <= 9 => {
            let book = books.require().map_err(fail)?;
            track::slot_accounts(&book, prefs)
                .into_iter()
                .find(|(k, _, _)| *k == key)
                .and_then(|(_, _, a)| a)
                .map(|a| (a, false))
                .ok_or_else(|| Failure(format!("slot {key} is empty"), Some("conflict")))
        }
        _ => Ok((slot.trim().to_string(), true)),
    }
}

fn status_text(state: &TrackState) -> String {
    let mut text = String::new();
    if !state.configured {
        text.push_str("Time tracking isn't set up: run `ledgeline-invoicer time setup`.\n");
        return text;
    }
    match &state.running {
        Some(r) => {
            let _ = writeln!(
                text,
                "▶ running  {}  {}  since {}",
                r.label, r.elapsed.text, r.since
            );
        }
        None => {
            let _ = writeln!(text, "■ stopped");
            if let Some(last) = &state.last {
                let key = last
                    .slot
                    .map(|k| format!(" (slot {k})"))
                    .unwrap_or_default();
                let _ = writeln!(text, "  last: {}{key}", last.label);
            }
        }
    }
    if let Some(guard) = &state.guard {
        let _ = writeln!(
            text,
            "  long-running: {}{}",
            guard.age.text,
            if guard.previous_day {
                ", from an earlier day"
            } else {
                ""
            }
        );
    }
    let _ = writeln!(text, "Today {}: {}", state.today, state.today_total.text);
    for slot in &state.slots {
        let lamp = if slot.running { "▶" } else { "○" };
        let _ = writeln!(
            text,
            "  {} {lamp} {:<36} {}",
            slot.key,
            slot.label.as_deref().unwrap_or("·"),
            if slot.today.seconds > 0 {
                slot.today.text.as_str()
            } else {
                "—"
            }
        );
    }
    text
}

fn track_out(
    cli: &Cli,
    books: &TimeBooks,
    done: &invoicer_core::time::Written<track::TrackOutcome>,
    state: &TrackState,
) {
    let value = json!({
        "done": done.done,
        "files": done.saved.iter().map(|p| books.paths.relative(p)).collect::<Vec<_>>(),
        "retried": done.retried,
        "included": done.included,
        "state": state,
    });
    emit(cli.json, &value, |_| {
        let mut text = String::new();
        if let Some(stopped) = &done.done.stopped {
            let minutes: i64 = stopped.days.iter().map(|(_, m)| m).sum();
            let _ = writeln!(
                text,
                "{} {} ({minutes}m)",
                if stopped.discarded {
                    "Discarded"
                } else {
                    "Stopped"
                },
                stopped.account
            );
        }
        if let Some(started) = &done.done.started {
            let _ = writeln!(text, "Started {started}");
        }
        if let Some(applied) = done.done.applied {
            let _ = writeln!(text, "Adjusted by {applied}m");
        }
        text.push_str(&status_text(state));
        text
    });
}

/// Run `time …`.
pub(crate) fn run(cli: &Cli, args: &TimeArgs) -> Result<ExitCode, Failure> {
    let mut session = open(cli)?;
    let books = time_books(&session)?;
    let prefs = prefs_of(&session, args);
    let person = prefs.person.clone();
    let person = person.as_deref();
    let now = now_of(args)?;
    let after = |books: &TimeBooks,
                 prefs: &TimePrefs,
                 done: &track::TrackOutcome|
     -> Result<TrackState, Failure> {
        let book = books.load().map_err(fail)?;
        let stopped = done.stopped.as_ref().map(|s| s.account.as_str());
        Ok(track::state(books, book.as_ref(), prefs, &now, stopped))
    };
    match &args.command {
        TimeCommand::Status => {
            let book = books.load().map_err(fail)?;
            let state = track::state(&books, book.as_ref(), &prefs, &now, None);
            emit(cli.json, &state, status_text);
        }
        TimeCommand::Start { account } => {
            let done = track::start(&books, None, person, account, &now).map_err(fail)?;
            track_out(cli, &books, &done, &after(&books, &prefs, &done.done)?);
        }
        TimeCommand::Stop { at, discard } => {
            let at = match at {
                Some(text) => parse_moment(text).ok_or_else(|| {
                    Failure(format!("--at {text}: not YYYY-MM-DDTHH:MM:SS"), None)
                })?,
                None => now.clone(),
            };
            let done = track::stop(&books, None, person, &at, *discard).map_err(fail)?;
            track_out(cli, &books, &done, &after(&books, &prefs, &done.done)?);
        }
        TimeCommand::Toggle { slot } => {
            let (account, chosen) = account_of(&books, &prefs, slot)?;
            let done = track::toggle(&books, None, person, &account, &now).map_err(fail)?;
            if chosen && done.done.started.is_some() {
                let book = books.require().map_err(fail)?;
                let in_slots = track::slot_accounts(&book, &prefs)
                    .iter()
                    .any(|(k, _, a)| *k != 0 && a.as_deref() == Some(account.as_str()));
                if !in_slots {
                    save_prefs(&session, json!({ "other": account }));
                }
            }
            let prefs = prefs_of(&session, args);
            track_out(cli, &books, &done, &after(&books, &prefs, &done.done)?);
        }
        TimeCommand::Adjust { slot, minutes } => {
            let (account, _) = account_of(&books, &prefs, slot)?;
            let done = track::adjust(
                &books,
                None,
                person,
                &account,
                minutes.unwrap_or(prefs.step),
                &now,
            )
            .map_err(fail)?;
            track_out(cli, &books, &done, &after(&books, &prefs, &done.done)?);
        }
        TimeCommand::Note { slot, note } => {
            let (account, _) = account_of(&books, &prefs, slot)?;
            let done =
                track::note(&books, None, person, &account, &now.date, note).map_err(fail)?;
            let value = json!({ "account": account, "note": note, "files": done.saved.iter().map(|p| books.paths.relative(p)).collect::<Vec<_>>() });
            emit(cli.json, &value, |_| format!("Noted {account}: {note}\n"));
        }
        TimeCommand::Log(log) => {
            let book = books.require().map_err(fail)?;
            let query = EntryQuery {
                from: log.from.clone(),
                to: log.to.clone(),
                person: if log.all { None } else { prefs.person.clone() },
                customer: log.customer.clone(),
                billed: log.billed.clone(),
                account: log.account.clone(),
                ..EntryQuery::default()
            };
            let locks = Locks::of(session.index(), session.booked_numbers());
            let list = entries::list(&book, &query, &locks);
            emit(cli.json, &list, |list| {
                let mut text = String::new();
                for e in &list.entries {
                    let billing = e
                        .invoice
                        .as_deref()
                        .map(|n| format!("invoice {n}"))
                        .or_else(|| e.nobill.as_ref().map(|_| "nobill".to_string()))
                        .unwrap_or_default();
                    let _ = writeln!(
                        text,
                        "{}  {:<8} {:<32} {:>8}  {}  {}",
                        e.date,
                        e.person.as_deref().unwrap_or(""),
                        e.account,
                        e.quantity.text,
                        e.note,
                        billing
                    );
                }
                let _ = writeln!(text, "{} entries, {}", list.entries.len(), list.total.text);
                text
            });
        }
        TimeCommand::Unbilled => {
            let book = books.require().map_err(fail)?;
            let locks = Locks::of(session.index(), session.booked_numbers());
            let view = report::unbilled(&book, session.index(), &locks.known, &today(cli)?);
            emit(cli.json, &view, |view| {
                let mut text = String::new();
                for c in &view.customers {
                    let next = c
                        .periods
                        .first()
                        .map(|p| format!("  bill {} to {}: {}", p.from, p.to, p.quantity.text))
                        .unwrap_or_default();
                    let _ = writeln!(text, "{:<24} {:>9}{next}", c.customer, c.quantity.text);
                }
                for n in &view.needs_customer {
                    let _ = writeln!(text, "needs a customer: {} {}", n.account, n.quantity.text);
                }
                for o in &view.orphans {
                    let _ = writeln!(
                        text,
                        "no invoice {} for {} tagged lines",
                        o.invoice,
                        o.entries.len()
                    );
                }
                text
            });
        }
        TimeCommand::Unpaid { everyone } => {
            let book = books.require().map_err(fail)?;
            let view = report::unpaid(&book, session.index(), &today(cli)?, *everyone);
            emit(cli.json, &view, |view| {
                view.iter()
                    .map(|p| format!("{:<16} {:>9}\n", p.person, p.quantity.text))
                    .collect()
            });
        }
        TimeCommand::Setup(setup) => {
            if let Some(file) = &setup.adopt {
                let plan = book::adopt_plan(session.vault(), session.main(), file).map_err(fail)?;
                let written = if setup.dry_run || !plan.needs_setting {
                    false
                } else {
                    book::set_time_file(session.vault(), session.main(), &plan.file)
                        .map_err(fail)?
                };
                let value = json!({
                    "file": plan.file,
                    "needs_setting": plan.needs_setting,
                    "written": written,
                    "missing": plan.missing,
                });
                emit(cli.json, &value, |_| {
                    let mut out = format!("Using {} as the time roll-up\n", plan.file);
                    if !plan.missing.is_empty() {
                        out.push_str(&format!("For hledger -s (strict), add to {}:\n", plan.file));
                        for line in &plan.missing {
                            out.push_str(&format!("  {line}\n"));
                        }
                    }
                    out
                });
                return Ok(ExitCode::SUCCESS);
            }
            let customers: Vec<String> = if setup.no_seed {
                Vec::new()
            } else {
                session
                    .index()
                    .customers()
                    .into_iter()
                    .map(|c| c.ledger_customer)
                    .collect()
            };
            let names: Vec<&str> = customers.iter().map(String::as_str).collect();
            let seed = if setup.no_seed {
                Vec::new()
            } else {
                book::default_seed(&names, &books.config)
            };
            let plan = book::setup_plan(
                session.vault(),
                session.main(),
                &books.config,
                person,
                &now.date,
                &seed,
            )
            .map_err(fail)?;
            if !setup.dry_run {
                book::create(session.vault(), &plan).map_err(fail)?;
                if args.person.is_some() {
                    save_prefs(&session, json!({ "person": person }));
                }
            }
            let files: Vec<Value> = plan
                .files
                .iter()
                .map(|(path, text)| json!({ "path": books.paths.relative(path), "text": text }))
                .collect();
            let value = json!({ "files": files, "person": person, "written": !setup.dry_run });
            emit(cli.json, &value, |_| {
                plan.files
                    .iter()
                    .map(|(path, _)| {
                        format!(
                            "{} {}\n",
                            if setup.dry_run {
                                "would create"
                            } else {
                                "created"
                            },
                            books.paths.relative(path)
                        )
                    })
                    .collect()
            });
        }
        TimeCommand::Bill(args) => return run_bill(cli, &mut session, &books, args),
        TimeCommand::Pay(args) => {
            let book = books.require().map_err(fail)?;
            let request = PayRequest {
                person: args.person.clone(),
                from: args.from.clone(),
                to: args.to.clone(),
                entries: None,
                lines: if args.total {
                    PayLines::Total
                } else {
                    PayLines::Account
                },
                note: args.note.clone(),
                create_note: None,
                rate: args.rate.clone(),
                expense_account: args.expense_account.clone(),
                payable_account: args.payable_account.clone(),
                terms: None,
                save_to_note: !args.no_save,
                date: args.date.clone(),
                reference: None,
                preview: args.dry_run,
            };
            let today = today(cli)?;
            let preview = pay::preview(&book, session.index(), session.journal(), &request, &today)
                .map_err(fail)?;
            if args.dry_run || !preview.ready {
                let code = if preview.ready { 0 } else { 2 };
                emit(cli.json, &preview, |p| {
                    let mut text = format!(
                        "{} {} for {}: {}\n",
                        p.reference,
                        p.quantity.text,
                        p.payee.as_deref().unwrap_or("?"),
                        p.amount.as_deref().unwrap_or("?")
                    );
                    for need in &p.needs {
                        let _ = writeln!(text, "  needs {}: {}", need.field, need.message);
                    }
                    text
                });
                return Ok(ExitCode::from(code));
            }
            let invoice_books = session.invoice_books();
            let (index, vault) = session.parts_mut();
            let created = pay::create(
                &books,
                &book,
                index,
                vault,
                &invoice_books,
                &request,
                &today,
            )
            .map_err(fail)?;
            emit(cli.json, &created, |c| {
                format!(
                    "Booked {} to {}: {}\n{}",
                    c.reference, c.vendor, c.amount, c.text
                )
            });
        }
        TimeCommand::Release {
            invoice,
            bill: bill_ref,
            nobill,
        } => {
            let how = if *nobill {
                Release::Nobill
            } else {
                Release::Release
            };
            if let Some(reference) = bill_ref {
                let invoice_books = session.invoice_books();
                let done =
                    pay::unlock_bill(&books, None, &invoice_books, reference, how, &today(cli)?)
                        .map_err(fail)?;
                emit(cli.json, &done, |d| {
                    format!(
                        "{}Released {} lines of bill {}\n",
                        if d.voided {
                            format!("Voided bill {} in the books. ", d.bill)
                        } else {
                            String::new()
                        },
                        d.released,
                        d.bill
                    )
                });
                return Ok(ExitCode::SUCCESS);
            }
            let invoice = invoice.as_deref().unwrap_or_default();
            let done = bill::release_by_hand(
                &books,
                None,
                session.index(),
                session.journal(),
                invoice,
                how,
            )
            .map_err(fail)?;
            let value = json!({ "invoice": invoice, "released": done.done, "how": how });
            emit(cli.json, &value, |_| {
                format!("Released {} lines of invoice {invoice}\n", done.done)
            });
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// `KEY=VALUE` pairs.
fn pairs(specs: &[String], what: &str) -> Result<Vec<(String, String)>, Failure> {
    specs
        .iter()
        .map(|spec| {
            spec.rsplit_once('=')
                .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
                .ok_or_else(|| Failure(format!("--{what} {spec:?}: write GROUP=VALUE"), None))
        })
        .collect()
}

fn run_bill(
    cli: &Cli,
    session: &mut Session,
    books: &TimeBooks,
    args: &BillArgs,
) -> Result<ExitCode, Failure> {
    let book = books.require().map_err(fail)?;
    let group = match args.group.as_str() {
        "account" => Grouping::Account,
        "account-person" => Grouping::AccountPerson,
        "account-note" => Grouping::AccountNote,
        other => {
            return Err(Failure(
                format!("--group {other}: account, account-person or account-note"),
                None,
            ));
        }
    };
    let detail = args
        .detail
        .as_deref()
        .map(|d| {
            Detail::parse(d)
                .ok_or_else(|| Failure(format!("--detail {d}: none, daily or entries"), None))
        })
        .transpose()?;
    let mut groups: Vec<GroupChoice> = Vec::new();
    for (key, code) in pairs(&args.item, "item")? {
        groups.push(GroupChoice {
            key,
            item: Some(code),
            remember: args.remember,
            ..GroupChoice::default()
        });
    }
    for (key, rate) in pairs(&args.rate, "rate")? {
        match groups.iter_mut().find(|g| g.key == key) {
            Some(choice) => choice.rate = Some(rate),
            None => groups.push(GroupChoice {
                key,
                rate: Some(rate),
                ..GroupChoice::default()
            }),
        }
    }
    let today = today(cli)?;
    let request = BillRequest {
        customer: args.customer.clone(),
        from: args.from.clone(),
        to: args.to.clone(),
        group,
        groups,
        detail,
        date: args.date.clone(),
        reference: args.reference.clone(),
        preview: args.dry_run,
        ..BillRequest::default()
    };
    if args.dry_run {
        let preview = bill::preview(
            books,
            &book,
            session.index(),
            session.vault(),
            &request,
            &today,
        )
        .map_err(fail)?;
        emit(cli.json, &preview, |p| {
            let mut text = format!(
                "{} {}: {} actual, {} billed, {}\n",
                p.customer.title,
                match (&p.from, &p.to) {
                    (Some(f), Some(t)) => format!("{f} to {t}"),
                    _ => "selection".to_string(),
                },
                p.actual.text,
                p.rounded.text,
                p.amount.as_deref().unwrap_or("?")
            );
            for g in &p.groups {
                let _ = writeln!(
                    text,
                    "  {:<40} {:>6} h  {:>8}  {}",
                    g.description,
                    g.hours,
                    g.rate.as_deref().unwrap_or("?"),
                    g.amount.as_deref().unwrap_or("?")
                );
            }
            for need in &p.needs {
                let _ = writeln!(text, "  needs: {need}");
            }
            text
        });
        return Ok(ExitCode::SUCCESS);
    }
    let customer = invoicer_core::time::link::customer_named(session.index(), &request.customer)
        .map(|c| c.wikilink())
        .unwrap_or_else(|| request.customer.clone());
    let (starter_template, starter) = crate::starter_for_new(session, &customer, None);
    let request = BillRequest {
        template: starter_template,
        ..request
    };
    let booked: Vec<String> = session.booked_numbers().to_vec();
    let booked: Vec<&str> = booked.iter().map(String::as_str).collect();
    let (index, vault) = session.parts_mut();
    let mut created =
        bill::create(books, &book, index, vault, &booked, &request, &today).map_err(fail)?;
    if let Some(starter) = starter
        && let Err(warning) = crate::starters::copy_in(session, starter)
    {
        created.warnings.push(warning);
    }
    emit(cli.json, &created, |c| {
        let mut text = format!(
            "Created draft {} ({}), {}; tagged {} entries\n",
            c.number,
            c.path,
            c.total.as_deref().unwrap_or("?"),
            c.tagged.len()
        );
        for r in &c.remembered {
            let _ = writeln!(text, "  remembered {r}");
        }
        for w in &c.warnings {
            let _ = writeln!(text, "  warning: {w}");
        }
        text
    });
    Ok(ExitCode::SUCCESS)
}

/// After `void`: release the invoice's time (or make it nobill), reported
/// in the output; a failure is a warning (the void is done).
pub(crate) fn release_after_void(session: &Session, number: &str, how: Release) -> Value {
    let result = TimeBooks::open(session.shared_vault(), session.main().clone())
        .and_then(|books| bill::release(&books, None, number, how).map(|w| w.done));
    match result {
        Ok(lines) => json!({ "released": lines, "how": how }),
        Err(error) => {
            eprintln!("ledgeline-invoicer: invoice {number}'s time wasn't released: {error}");
            json!({ "released": 0, "error": error.to_string(), "code": error.code() })
        }
    }
}
