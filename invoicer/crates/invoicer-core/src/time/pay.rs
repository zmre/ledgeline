//! Contractor time → an accounts-payable bill (plan 46 B-ap, T18).
//!
//! A person's time card links to a timekeeper note ([`super::link`]); the
//! note's frontmatter holds the defaults (rate, accounts, payee, terms).
//! [`preview`] lists what is still **needed** — the note itself, the rate,
//! the expense account, the payable account when the books have none or
//! several — and [`create`] can save the answers to the note (an explicit
//! action, [[settings-files-toml]]). The bill is plan 45's
//! [`Loaded::write_document`] with [`Direction::Expense`] and
//! [`PartyKind::Vendor`]: the payee (`pay_to`'s vendor, else the note's),
//! one line per time account (or one in total), and `reference: <person>
//! timesheet <period>`. Every included line is tagged `bill: <reference>`
//! first; when the bill can't be booked, the tags come off again.
//!
//! [`Loaded::write_document`]: ledgeline_core::accrual::ops::Loaded::write_document

use super::link::{Timekeeper, note_as_timekeeper, timekeeper_for, timekeepers};
use super::report::PeriodView;
use super::{
    EntryRef, Qty, TimeBooks, TimeOpError, entry_id, find_entries, hours_2, save_note_fields,
};
use crate::books::InvoiceBooks;
use crate::customers::ContactKind;
use crate::frontmatter::{self, FieldEdit};
use crate::index::DocumentIndex;
use ledgeline_core::Dec;
use ledgeline_core::accrual::invoice::{
    Direction, DocumentLine, DocumentSpec, money, tag_value_from,
};
use ledgeline_core::accrual::schedule::{Party, PartyKind};
use ledgeline_core::accrual::{AccrualRole, AccrualRoles, DUE_TAG, REFERENCE_TAG};
use ledgeline_core::model::Journal;
use ledgeline_core::reports::periods::{Interval, bucket_key};
use ledgeline_core::time::model::TimeBook;
use ledgeline_core::time::query::{self, Cadence, suggest_periods};
use ledgeline_core::time::{TagRole, TimeQty};
use ledgeline_core::vault::{Vault, VaultPath};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// One line per time account, or one in total.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PayLines {
    /// One line per time account (the default).
    #[default]
    Account,
    /// One line for everything.
    Total,
}

/// *Create note…* in the bill dialog: a timekeeper note for the person.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewTimekeeper {
    /// Its title (the payee's name).
    pub title: String,
    /// Person or organization.
    #[serde(default = "person_kind")]
    pub kind: ContactKind,
}

const fn person_kind() -> ContactKind {
    ContactKind::Person
}

const fn yes() -> bool {
    true
}

/// `POST /api/time/pay`, `time pay`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PayRequest {
    /// The `person:` value on the time cards.
    pub person: String,
    /// The first date (default: the main suggested period's).
    pub from: Option<String>,
    /// The last date, inclusive.
    pub to: Option<String>,
    /// Exactly these entries instead of a period.
    pub entries: Option<Vec<String>>,
    /// One line per account, or one in total.
    #[serde(default)]
    pub lines: PayLines,
    /// The note to use when none links the person (*Pick a note*): a
    /// `[[link]]` or title.
    pub note: Option<String>,
    /// Or create one (*Create note…*).
    pub create_note: Option<NewTimekeeper>,
    /// The hourly rate, overriding the note's `pay_rate`.
    pub rate: Option<String>,
    /// The expense account, overriding `expense_account`.
    pub expense_account: Option<String>,
    /// The payable account, overriding `payable_account`.
    pub payable_account: Option<String>,
    /// Net days, overriding `pay_terms`.
    pub terms: Option<u32>,
    /// Save the answers the note lacked to its frontmatter (default yes).
    #[serde(default = "yes")]
    pub save_to_note: bool,
    /// The bill's date (default today).
    pub date: Option<String>,
    /// The reference (default `<person> timesheet <period>`).
    pub reference: Option<String>,
    /// Preview only: write nothing.
    #[serde(default)]
    pub preview: bool,
}

/// Something the bill needs before it can be booked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Need {
    /// `note`, `rate`, `expense_account`, `payable_account`, `time`.
    pub field: &'static str,
    /// For people.
    pub message: String,
    /// What to choose from (notes, payable accounts).
    pub candidates: Vec<String>,
}

/// One line of the bill.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PayLine {
    /// The time account (`None` for one line in total).
    pub account: Option<String>,
    /// The time.
    pub quantity: Qty,
    /// The hours billed (two places).
    pub hours: String,
    /// The amount, when the rate reads.
    pub amount: Option<String>,
}

/// What the bill would be.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PayPreview {
    /// The person.
    pub person: String,
    /// The timekeeper note.
    pub note: Option<Timekeeper>,
    /// Who is paid (the `vendor:` tag): `pay_to`'s vendor, else the note's.
    pub payee: Option<String>,
    /// The `pay_to` note, when it resolves.
    pub pay_to: Option<String>,
    /// The period (inclusive).
    pub from: Option<String>,
    /// Its end.
    pub to: Option<String>,
    /// The entries.
    pub entries: Vec<String>,
    /// The lines.
    pub lines: Vec<PayLine>,
    /// The time.
    pub quantity: Qty,
    /// The rate.
    pub rate: Option<String>,
    /// The total.
    pub amount: Option<String>,
    /// The expense account.
    pub expense_account: Option<String>,
    /// The payable account.
    pub payable_account: Option<String>,
    /// Net days.
    pub terms: Option<u32>,
    /// The due date.
    pub due: Option<String>,
    /// The reference.
    pub reference: String,
    /// The date.
    pub date: String,
    /// The periods the person could be paid for.
    pub periods: Vec<PeriodView>,
    /// What is still needed.
    pub needs: Vec<Need>,
    /// Whether it can be booked.
    pub ready: bool,
}

/// What [`create`] booked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PayCreated {
    /// The bill's reference (the `bill:` tag on the time).
    pub reference: String,
    /// Its date.
    pub date: String,
    /// The payee.
    pub vendor: String,
    /// The total.
    pub amount: String,
    /// The transaction as written.
    pub text: String,
    /// The entries tagged.
    pub tagged: Vec<String>,
    /// The time files written.
    pub files: Vec<String>,
    /// The note keys saved.
    pub saved_to_note: Vec<String>,
    /// The note created, if one was.
    pub note_created: Option<VaultPath>,
}

/// The payable accounts the books have.
fn payable_accounts(journal: &Journal) -> Vec<String> {
    AccrualRoles::of(journal)
        .iter()
        .filter(|(_, role, _)| *role == AccrualRole::Payable)
        .map(|(name, _, _)| name.to_string())
        .collect()
}

/// `<person> timesheet <period>`: the month (`2026-11`) when the time is
/// all in one, else its first and last dates.
fn reference_for(person: &str, dates: &[&str]) -> String {
    let (Some(first), Some(last)) = (dates.iter().min(), dates.iter().max()) else {
        return person.to_string();
    };
    let month = bucket_key(first, Interval::Monthly);
    let period = if bucket_key(last, Interval::Monthly) == month {
        month
    } else {
        format!("{first} to {last}")
    };
    tag_value_from(&format!("{person} timesheet {period}")).unwrap_or_else(|| person.to_string())
}

/// The note `request` uses: the person's linked timekeeper, else the one
/// it names.
fn note_of(index: &DocumentIndex, request: &PayRequest) -> Option<Timekeeper> {
    timekeeper_for(index.site(), &request.person).or_else(|| {
        request
            .note
            .as_deref()
            .and_then(|link| note_as_timekeeper(index.site(), &VaultPath::root(), link))
    })
}

/// What paying `request.person` would book.
///
/// # Errors
/// [`TimeOpError::BadRequest`], [`TimeOpError::NotFound`].
pub fn preview(
    book: &TimeBook,
    index: &DocumentIndex,
    journal: &Journal,
    request: &PayRequest,
    today: &str,
) -> Result<PayPreview, TimeOpError> {
    let person = request.person.trim();
    if person.is_empty() {
        return Err(TimeOpError::BadRequest(
            "unattributed time can't be paid: set its person first".to_string(),
        ));
    }
    let names = &book.tags;
    let theirs = |at: usize| {
        let entry = &book.entries[at];
        entry.person.as_deref() == Some(person)
            && !names.has(&entry.tags, TagRole::Bill)
            && !entry.quantity.is_zero()
    };
    let all: Vec<usize> = (0..book.entries.len()).filter(|&at| theirs(at)).collect();
    let note = note_of(index, request);
    let cadence = note
        .as_ref()
        .and_then(|n| index.site().note(&n.path))
        .and_then(|n| super::link::field_text(n.frontmatter.get("cadence")));
    let periods = suggest_periods(
        book,
        &query::Batch {
            quantity: TimeQty::ZERO,
            oldest: String::new(),
            newest: String::new(),
            entries: all.clone(),
        },
        Cadence::parse(cadence.as_deref()),
        today,
    );
    let (selected, from, to) = if let Some(ids) = &request.entries {
        let mut picked = Vec::new();
        for id in ids {
            for at in find_entries(book, id)? {
                if !theirs(at) {
                    return Err(TimeOpError::Conflict(format!(
                        "{id} isn't {person}'s unpaid time"
                    )));
                }
                picked.push(at);
            }
        }
        picked.sort_unstable();
        picked.dedup();
        (picked, None, None)
    } else {
        let first = periods.first();
        let from = request
            .from
            .clone()
            .or_else(|| first.map(|p| p.start.clone()));
        let to = request.to.clone().or_else(|| first.map(|p| p.end.clone()));
        let picked = all
            .iter()
            .copied()
            .filter(|&at| {
                let date = book.entries[at].date.as_str();
                from.as_deref().is_none_or(|f| date >= f) && to.as_deref().is_none_or(|t| date <= t)
            })
            .collect();
        (picked, from, to)
    };
    let mut needs = Vec::new();
    let creating = request.create_note.as_ref();
    if note.is_none() && creating.is_none() {
        needs.push(Need {
            field: "note",
            message: format!(
                "no note links {person}'s time: pick a person or organization note, or create one"
            ),
            candidates: timekeepers(index.site())
                .into_iter()
                .map(|k| k.title)
                .collect(),
        });
    }
    let pay_to = note
        .as_ref()
        .and_then(|n| n.pay_to.as_deref().map(|link| (n, link)))
        .and_then(|(n, link)| note_as_timekeeper(index.site(), &n.path, link));
    let payee = pay_to
        .as_ref()
        .map(|p| p.vendor_name().to_string())
        .or_else(|| note.as_ref().map(|n| n.vendor_name().to_string()))
        .or_else(|| creating.map(|c| c.title.trim().to_string()));
    let rate = request
        .rate
        .clone()
        .or_else(|| note.as_ref().and_then(|n| n.pay_rate.clone()));
    let style = index.money().style(None);
    let rate_value = rate
        .as_deref()
        .and_then(|r| style.parse(r).ok())
        .map(|m| m.value);
    match (&rate, rate_value) {
        (None, _) => needs.push(Need {
            field: "rate",
            message: format!("{person}'s hourly rate"),
            candidates: Vec::new(),
        }),
        (Some(rate), None) => needs.push(Need {
            field: "rate",
            message: format!(
                "the rate {rate} doesn't read as {} money",
                style.commodity()
            ),
            candidates: Vec::new(),
        }),
        _ => {}
    }
    let expense_account = request
        .expense_account
        .clone()
        .or_else(|| note.as_ref().and_then(|n| n.expense_account.clone()));
    if expense_account.is_none() {
        needs.push(Need {
            field: "expense_account",
            message: "the expense account the time is booked to".to_string(),
            candidates: Vec::new(),
        });
    }
    let payables = payable_accounts(journal);
    let payable_account = request
        .payable_account
        .clone()
        .or_else(|| note.as_ref().and_then(|n| n.payable_account.clone()))
        .or_else(|| (payables.len() == 1).then(|| payables[0].clone()));
    if payable_account.is_none() {
        needs.push(Need {
            field: "payable_account",
            message: if payables.is_empty() {
                "the books have no payable account: declare one (`; type: L`, named …payable)"
                    .to_string()
            } else {
                "which payable account the bill is owed on".to_string()
            },
            candidates: payables,
        });
    }
    if selected.is_empty() {
        needs.push(Need {
            field: "time",
            message: format!("{person} has no unpaid time in the period"),
            candidates: Vec::new(),
        });
    }
    let mut by_account: BTreeMap<Option<String>, TimeQty> = BTreeMap::new();
    for &at in &selected {
        let entry = &book.entries[at];
        let key = match request.lines {
            PayLines::Account => Some(entry.account.clone()),
            PayLines::Total => None,
        };
        let slot = by_account.entry(key).or_default();
        *slot = slot.plus(entry.quantity);
    }
    let mut total = Some(Dec::zero());
    let lines: Vec<PayLine> = by_account
        .into_iter()
        .map(|(account, quantity)| {
            let hours = hours_2(quantity);
            let amount = rate_value.and_then(|rate| {
                style
                    .round(rate.mul(Dec::parse(&hours, '.').ok()?).ok()?)
                    .ok()
            });
            total = match (total, amount) {
                (Some(t), Some(a)) => t.add(a).ok(),
                _ => None,
            };
            PayLine {
                account,
                quantity: quantity.into(),
                hours,
                amount: amount.map(|a| style.format(a)),
            }
        })
        .collect();
    let date = request.date.clone().unwrap_or_else(|| today.to_string());
    let terms = request
        .terms
        .or_else(|| note.as_ref().and_then(|n| n.pay_terms));
    let reference = request
        .reference
        .as_deref()
        .and_then(tag_value_from)
        .unwrap_or_else(|| {
            let dates: Vec<&str> = selected
                .iter()
                .map(|&at| book.entries[at].date.as_str())
                .collect();
            reference_for(person, &dates)
        });
    if journal.transactions.iter().any(|txn| {
        txn.tags
            .iter()
            .any(|(name, value)| name == REFERENCE_TAG && value.trim() == reference)
    }) {
        needs.push(Need {
            field: "reference",
            message: format!("a bill with reference {reference} is already booked"),
            candidates: Vec::new(),
        });
    }
    Ok(PayPreview {
        person: person.to_string(),
        payee,
        pay_to: pay_to.map(|p| p.title),
        note,
        from,
        to,
        entries: selected
            .iter()
            .map(|&at| entry_id(&book.entries[at]))
            .collect(),
        quantity: TimeQty::sum(selected.iter().map(|&at| book.entries[at].quantity)).into(),
        rate,
        amount: total.filter(|_| !lines.is_empty()).map(|t| style.format(t)),
        lines,
        expense_account,
        payable_account,
        due: terms.map(|t| crate::doc::due_from_terms(&date, i64::from(t))),
        terms,
        reference,
        date,
        periods: periods.iter().map(PeriodView::from).collect(),
        ready: needs.is_empty(),
        needs,
    })
}

/// Create a timekeeper note for `person` (*Create note…*): beside the other
/// timekeeper notes, else in the customers folder.
fn create_note(
    index: &mut DocumentIndex,
    vault: &dyn Vault,
    person: &str,
    new: &NewTimekeeper,
) -> Result<VaultPath, TimeOpError> {
    let title = new.title.trim();
    if title.is_empty() || title.contains(['/', '\\']) || title.starts_with('.') {
        return Err(TimeOpError::BadRequest(format!(
            "`{title}` can't be a note's title"
        )));
    }
    let dir = timekeepers(index.site())
        .first()
        .and_then(|k| k.path.parent())
        .or_else(|| crate::settings::Settings::vault_path(&index.settings().customers_dir).ok())
        .unwrap_or_else(VaultPath::root);
    let path = dir
        .join(&format!("{title}.md"))
        .map_err(|_| TimeOpError::BadRequest(format!("`{title}` can't be a file name")))?;
    let fields: Vec<(&str, Value)> = vec![
        ("title", Value::String(title.to_string())),
        ("type", Value::String(new.kind.as_str().to_string())),
        (
            "tags",
            Value::Array(vec![Value::String(super::TIMEKEEPER_TAG.to_string())]),
        ),
        ("time_person", Value::String(person.to_string())),
    ];
    let text =
        frontmatter::render_block(&fields).map_err(|e| TimeOpError::Storage(e.to_string()))?;
    vault
        .create_dir_all(&dir)
        .map_err(|e| TimeOpError::Storage(e.to_string()))?;
    vault.create_new(&path, text.as_bytes()).map_err(|e| {
        if e.kind() == std::io::ErrorKind::AlreadyExists {
            TimeOpError::Conflict(format!("{path} already exists: pick it instead"))
        } else {
            TimeOpError::Storage(e.to_string())
        }
    })?;
    index.refresh(vault, std::slice::from_ref(&path));
    Ok(path)
}

/// Book the bill `request` describes and tag its time `bill: <reference>`:
/// the tags first, the bill second, the tags taken off when the bill can't
/// be written. The note is created and the answers saved first, when asked.
///
/// # Errors
/// [`TimeOpError::Conflict`] when it isn't ready (the preview's `needs`);
/// [`TimeOpError::Books`]; [`TimeOpError`].
pub fn create(
    books: &TimeBooks,
    book: &TimeBook,
    index: &mut DocumentIndex,
    vault: &dyn Vault,
    invoice_books: &InvoiceBooks,
    request: &PayRequest,
    today: &str,
) -> Result<PayCreated, TimeOpError> {
    let mut loaded = invoice_books.load()?;
    let first = preview(book, index, loaded.journal(), request, today)?;
    if !first.ready {
        return Err(TimeOpError::Conflict(format!(
            "the bill isn't ready: {}",
            first
                .needs
                .iter()
                .map(|n| n.message.as_str())
                .collect::<Vec<_>>()
                .join("; ")
        )));
    }
    let note_created = match (&first.note, &request.create_note) {
        (None, Some(new)) => Some(create_note(index, vault, &first.person, new)?),
        _ => None,
    };
    let mut saved_to_note = Vec::new();
    let note_path = first
        .note
        .as_ref()
        .map(|n| n.path.clone())
        .or_else(|| note_created.clone());
    if request.save_to_note
        && let Some(path) = &note_path
    {
        let keeper = first.note.clone();
        let missing = |current: Option<&String>| current.is_none();
        let mut edits = Vec::new();
        let mut save = |key: &str, value: Value, current_missing: bool| {
            if current_missing {
                edits.push(FieldEdit::Set(key.to_string(), value));
                saved_to_note.push(key.to_string());
            }
        };
        if let Some(rate) = &first.rate {
            save(
                "pay_rate",
                Value::String(rate.clone()),
                missing(keeper.as_ref().and_then(|k| k.pay_rate.as_ref())),
            );
        }
        if let Some(account) = &first.expense_account {
            save(
                "expense_account",
                Value::String(account.clone()),
                missing(keeper.as_ref().and_then(|k| k.expense_account.as_ref())),
            );
        }
        if let (Some(account), Some(_)) = (&first.payable_account, &request.payable_account) {
            save(
                "payable_account",
                Value::String(account.clone()),
                missing(keeper.as_ref().and_then(|k| k.payable_account.as_ref())),
            );
        }
        if let (Some(terms), Some(_)) = (first.terms, request.terms) {
            save(
                "pay_terms",
                Value::from(terms),
                keeper.as_ref().is_none_or(|k| k.pay_terms.is_none()),
            );
        }
        if !edits.is_empty() {
            save_note_fields(index, vault, path, &edits)?;
        }
    }
    let style = index.money().style(None);
    let commodity = style.commodity().to_string();
    let journal = loaded.journal();
    let expense = first.expense_account.clone().unwrap_or_default();
    let lines: Vec<DocumentLine> = first
        .lines
        .iter()
        .filter_map(|line| {
            let amount = style.parse(line.amount.as_deref()?).ok()?.value;
            let mut posting =
                DocumentLine::new(expense.clone(), money(journal, &commodity, amount));
            posting.tags.push(("hours".to_string(), line.hours.clone()));
            if let Some(account) = &line.account {
                posting.tags.push(("time".to_string(), account.clone()));
            }
            Some(posting)
        })
        .collect();
    let vendor = first.payee.clone().unwrap_or_else(|| first.person.clone());
    let mut tags = vec![(REFERENCE_TAG.to_string(), first.reference.clone())];
    if let Some(due) = &first.due {
        tags.push((DUE_TAG.to_string(), due.clone()));
    }
    let spec = DocumentSpec {
        direction: Direction::Expense,
        date: first.date.clone(),
        description: format!("{vendor} | {}", first.reference),
        number: None,
        party: Some(Party {
            kind: PartyKind::Vendor,
            name: vendor.clone(),
        }),
        settlement_account: first.payable_account.clone().unwrap_or_default(),
        lines,
        tags,
    };
    let bill_tag = books.config.tags.name(TagRole::Bill).to_string();
    let reference = first.reference.clone();
    let person = first.person.clone();
    let tagged = books.write(Some(book), |book, files| {
        let mut refs = BTreeSet::new();
        for id in &first.entries {
            for at in find_entries(book, id)? {
                let entry = &book.entries[at];
                if entry.person.as_deref() != Some(person.as_str())
                    || book.tags.has(&entry.tags, TagRole::Bill)
                {
                    return Err(TimeOpError::Conflict(format!(
                        "{id} changed since the preview; preview again"
                    )));
                }
                refs.insert(EntryRef::of(entry));
            }
        }
        // Last line first: removing a tag can remove the continuation
        // line it was on, which would shift every later line of the file.
        for at in refs.iter().rev() {
            super::entries::set_tag(files, at, &bill_tag, Some(&reference))?;
        }
        Ok(refs.len())
    })?;
    // The journal may have moved on since it was read (Ledgeline saved):
    // read it again and write once more, as every books write does.
    let written = match loaded.write_document(&spec, today, false, None) {
        Err(ledgeline_core::accrual::ops::OpsError::ExternalChange) => invoice_books
            .load()
            .map_err(TimeOpError::from)
            .and_then(|mut fresh| {
                fresh
                    .write_document(&spec, today, false, None)
                    .map_err(|e| TimeOpError::Books(e.into()))
            }),
        other => other.map_err(|e| TimeOpError::Books(e.into())),
    };
    let (transaction, _) = match written {
        Ok(done) => done,
        Err(error) => {
            let undone = release_bill(books, &reference);
            return Err(match undone {
                Ok(_) => error,
                Err(undo) => TimeOpError::Conflict(format!(
                    "the bill wasn't booked ({error}), and the time is still tagged bill: \
                     {reference} ({undo})"
                )),
            });
        }
    };
    Ok(PayCreated {
        text: ledgeline_core::edit::format_transaction(&transaction),
        reference: first.reference.clone(),
        date: first.date.clone(),
        amount: first.amount.clone().unwrap_or_default(),
        vendor,
        tagged: first.entries.clone(),
        files: tagged
            .saved
            .iter()
            .map(|p| books.paths.relative(p))
            .collect(),
        saved_to_note,
        note_created,
    })
}

/// Take `bill: reference` off every line that carries it.
///
/// # Errors
/// [`TimeOpError`].
pub fn release_bill(books: &TimeBooks, reference: &str) -> Result<usize, TimeOpError> {
    let written = books.write(None, |book, files| {
        let names = &book.tags;
        let refs: BTreeSet<EntryRef> = book
            .entries
            .iter()
            .filter(|entry| entry.tag(names, TagRole::Bill) == Some(reference))
            .map(EntryRef::of)
            .collect();
        // Last line first: removing a tag can remove the continuation
        // line it was on, which would shift every later line of the file.
        for at in refs.iter().rev() {
            super::entries::set_tag(files, at, names.name(TagRole::Bill), None)?;
        }
        Ok(refs.len())
    })?;
    Ok(written.done)
}
