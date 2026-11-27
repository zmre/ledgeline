//! Bill time → an invoice draft (plan 46 B-invoice, T10, T11, B-rates).
//!
//! [`preview`] groups a customer's unbilled time for a period (by account,
//! account × person, or account × note), suggests a catalog item per group
//! (the customer's `time_items` rule, the account's `item:`, the item the
//! customer is billed most), rounds each row (B-round) and builds the
//! timesheet table the customer's `time_detail` asks for. [`create`] makes
//! the draft through the CLI's `new` path ([`crate::draft::render`] then
//! [`crate::draft::create`]: one write) and tags every included line
//! `invoice: N` **first**; when the draft can't be written, the tags are
//! taken off again. [`release`] takes them off (a deleted draft) or turns
//! them into `nobill` (a void, when asked).

use super::link::{CustomerTime, Detail, customer_named, customer_time};
use super::report::catalog_item;
use super::round::Rounding;
use super::{
    EntryRef, Qty, TimeBooks, TimeOpError, Written, entries::Billed, entry_id, find_entries,
    hours_2, save_note_fields,
};
use crate::catalog::Catalog;
use crate::draft::{self, DraftRow, NewInvoice};
use crate::frontmatter::FieldEdit;
use crate::index::DocumentIndex;
use crate::table::gfm::escape_cell;
use ledgeline_core::Dec;
use ledgeline_core::reports::periods::{Interval, bucket_end, bucket_key, bucket_start};
use ledgeline_core::time::model::{TimeBook, TimeEntry};
use ledgeline_core::time::query::{self, suggest_periods};
use ledgeline_core::time::{TagRole, TimeQty};
use ledgeline_core::vault::{Vault, VaultPath};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

/// How the bill's rows group the time (B-invoice step 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Grouping {
    /// One row per account (the default).
    #[default]
    Account,
    /// One row per account and person (per-person rates, B-rates).
    AccountPerson,
    /// One row per account and note.
    AccountNote,
}

/// A choice for one group (B-invoice step 2), by its [`BillGroup::key`].
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GroupChoice {
    /// The group.
    pub key: String,
    /// The catalog code to bill it as.
    pub item: Option<String>,
    /// The rate, overriding the item's (`$250`).
    pub rate: Option<String>,
    /// The books cell, overriding the item's.
    pub account: Option<String>,
    /// The row's description.
    pub description: Option<String>,
    /// Remember the item: `item:` on the account (by account), or a
    /// `time_items` rule in the customer note (by account × person).
    #[serde(default)]
    pub remember: bool,
    /// Leave this group's time off the bill (it stays unbilled).
    #[serde(default)]
    pub exclude: bool,
}

/// `POST /api/time/bill`, `time bill`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BillRequest {
    /// The customer: the `customer:` value the time accounts carry, or the
    /// customer note's title or `[[link]]`.
    pub customer: String,
    /// The first date (default: the main suggested period's).
    pub from: Option<String>,
    /// The last date, inclusive.
    pub to: Option<String>,
    /// Exactly these entries instead of a period (Entries → *Bill
    /// selected*).
    pub entries: Option<Vec<String>>,
    /// The grouping.
    #[serde(default)]
    pub group: Grouping,
    /// Per-group choices.
    #[serde(default)]
    pub groups: Vec<GroupChoice>,
    /// The timesheet table (default: the customer's `time_detail`).
    pub detail: Option<Detail>,
    /// The invoice date (default today).
    pub date: Option<String>,
    /// `reference`.
    pub reference: Option<String>,
    /// The number (default: the next).
    pub number: Option<String>,
    /// The template.
    pub template: Option<String>,
    /// Net days.
    pub terms: Option<u32>,
    /// Preview only (the API's `preview`): write nothing.
    #[serde(default)]
    pub preview: bool,
}

/// The item a group bills as, and where the suggestion came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ItemView {
    /// The code.
    pub code: String,
    /// `choice`, `time-items`, `account` or `most-used`.
    pub source: &'static str,
    /// Whether the catalog has it.
    pub found: bool,
    /// The catalog's description.
    pub description: Option<String>,
    /// Its unit.
    pub unit: Option<String>,
    /// Its rate, as written.
    pub rate: Option<String>,
    /// Its books cell.
    pub books: Option<String>,
}

/// One row-to-be.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BillGroup {
    /// The key a [`GroupChoice`] names it by.
    pub key: String,
    /// The account.
    pub account: String,
    /// The person (by account × person).
    pub person: Option<String>,
    /// The note (by account × note).
    pub note: Option<String>,
    /// The entries.
    pub entries: Vec<String>,
    /// The actual time.
    pub actual: Qty,
    /// The rounded time (B-round).
    pub rounded: Qty,
    /// Rounded minus actual, in hours, plain decimal.
    pub difference: String,
    /// The row's Hours cell (two places).
    pub hours: String,
    /// The item.
    pub item: Option<ItemView>,
    /// The rate the row gets.
    pub rate: Option<String>,
    /// The books cell it gets.
    pub books: Option<String>,
    /// Its description.
    pub description: String,
    /// The row's amount, when its rate reads.
    pub amount: Option<String>,
    /// Left off the bill.
    pub excluded: bool,
    /// What it still needs before the draft can be made.
    pub needs: Vec<String>,
}

/// What a bill would be.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BillPreview {
    /// The customer.
    pub customer: CustomerTime,
    /// The period (inclusive), or `None` for a selection.
    pub from: Option<String>,
    /// Its end.
    pub to: Option<String>,
    /// The grouping.
    pub group: Grouping,
    /// The rows.
    pub groups: Vec<BillGroup>,
    /// The actual total.
    pub actual: Qty,
    /// The rounded total.
    pub rounded: Qty,
    /// The difference, in hours.
    pub difference: String,
    /// The bill's total, when every row's rate reads.
    pub amount: Option<String>,
    /// The timesheet kind.
    pub detail: Detail,
    /// The timesheet table's markdown (none for `detail: none`).
    pub timesheet: Option<String>,
    /// Every entry included.
    pub entries: Vec<String>,
    /// The periods the customer could bill (B-unbilled).
    pub periods: Vec<super::report::PeriodView>,
    /// Whether the draft can be made.
    pub ready: bool,
    /// What blocks it.
    pub needs: Vec<String>,
}

/// What [`create`] made.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BillCreated {
    /// The draft.
    pub path: VaultPath,
    /// Its number.
    pub number: String,
    /// Its total.
    pub total: Option<String>,
    /// The entries tagged `invoice: N`.
    pub tagged: Vec<String>,
    /// The time files written.
    pub files: Vec<String>,
    /// What was remembered (`item:` on accounts, `time_items` rules).
    pub remembered: Vec<String>,
    /// What the draft or the remembering fell back from.
    pub warnings: Vec<String>,
}

/// `2026-11` as `Nov 2026` when `from..=to` is that whole month, else
/// `2026-11-03 – 2026-11-20`.
fn period_label(books: &TimeBooks, from: &str, to: &str) -> String {
    let month = bucket_key(from, Interval::Monthly);
    let whole = bucket_start(&month).ok().as_deref() == Some(from)
        && bucket_end(&month).ok().as_deref() == Some(to);
    if whole {
        let name = books.config.words.month(from);
        let short: String = name.chars().take(3).collect();
        format!("{short} {}", &from[..4])
    } else if from == to {
        from.to_string()
    } else {
        format!("{from} – {to}")
    }
}

/// `tax-return` → `Tax return`.
fn humanize(segment: &str) -> String {
    let words = segment.replace(['-', '_'], " ");
    let mut chars = words.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

/// The customer's most-billed catalog code, over their invoices.
fn most_used_code(
    index: &DocumentIndex,
    vault: &dyn Vault,
    customer: &VaultPath,
) -> Option<String> {
    let words = &index.settings().words;
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for doc in index.billed_to(customer).into_iter().take(50) {
        let rows = doc.summary.as_ref().map_or(0, |s| s.rows);
        let Ok(text) = vault.read_to_string(&doc.path) else {
            continue;
        };
        let invoice = crate::doc::Invoice::parse(text, words);
        if let Ok(Some(table)) = invoice.bill_table() {
            for row in 0..rows {
                let code = table.cell(row, crate::table::columns::Role::Item).trim();
                if !code.is_empty() {
                    *counts.entry(code.to_string()).or_default() += 1;
                }
            }
        }
    }
    counts
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0)))
        .map(|(code, _)| code)
}

struct Selection {
    entries: Vec<usize>,
    from: Option<String>,
    to: Option<String>,
}

/// Whether `entry` is `customer`'s unbilled time.
fn is_theirs(book: &TimeBook, entry: &TimeEntry, customer: &CustomerTime) -> bool {
    Billed::of(book, entry) == Billed::Unbilled
        && query::customer_of(book, entry)
            .is_some_and(|c| c == customer.ledger_customer || c == customer.title)
}

fn select(
    book: &TimeBook,
    customer: &CustomerTime,
    request: &BillRequest,
    today: &str,
) -> Result<Selection, TimeOpError> {
    if let Some(ids) = &request.entries {
        let mut entries = Vec::new();
        for id in ids {
            for at in find_entries(book, id)? {
                if !is_theirs(book, &book.entries[at], customer) {
                    return Err(TimeOpError::Conflict(format!(
                        "{id} isn't {}'s unbilled time",
                        customer.title
                    )));
                }
                entries.push(at);
            }
        }
        entries.sort_unstable();
        entries.dedup();
        return Ok(Selection {
            entries,
            from: None,
            to: None,
        });
    }
    let all: Vec<usize> = book
        .entries
        .iter()
        .enumerate()
        .filter(|(_, e)| !e.quantity.is_zero() && is_theirs(book, e, customer))
        .map(|(at, _)| at)
        .collect();
    let (from, to) = match (&request.from, &request.to) {
        (Some(from), Some(to)) => (from.clone(), to.clone()),
        (from, to) => {
            let batch = query::Batch {
                quantity: TimeQty::ZERO,
                oldest: String::new(),
                newest: String::new(),
                entries: all.clone(),
            };
            let suggested = suggest_periods(book, &batch, customer.cadence(), today);
            let first = suggested.first();
            (
                from.clone()
                    .or_else(|| first.map(|p| p.start.clone()))
                    .unwrap_or_else(|| "0000-01-01".to_string()),
                to.clone()
                    .or_else(|| first.map(|p| p.end.clone()))
                    .unwrap_or_else(|| today.to_string()),
            )
        }
    };
    for date in [&from, &to] {
        if !crate::table::books::is_iso_date(date) {
            return Err(TimeOpError::BadRequest(format!(
                "`{date}` isn't a date (YYYY-MM-DD)"
            )));
        }
    }
    Ok(Selection {
        entries: all
            .into_iter()
            .filter(|&at| {
                let date = book.entries[at].date.as_str();
                date >= from.as_str() && date <= to.as_str()
            })
            .collect(),
        from: Some(from),
        to: Some(to),
    })
}

/// The timesheet table (B-invoice step 4): `Date · Who · Work · Hours`,
/// one row per day and person (`daily`) or per entry, the actual hours.
fn timesheet(book: &TimeBook, entries: &[usize], detail: Detail) -> Option<String> {
    let work_of = |entry: &TimeEntry| {
        if entry.note.is_empty() {
            humanize(entry.account.rsplit(':').next().unwrap_or(&entry.account))
        } else {
            entry.note.clone()
        }
    };
    let rows: Vec<[String; 4]> = match detail {
        Detail::None => return None,
        Detail::Entries => entries
            .iter()
            .map(|&at| {
                let entry = &book.entries[at];
                [
                    entry.date.clone(),
                    entry.person.clone().unwrap_or_default(),
                    work_of(entry),
                    hours_2(entry.quantity),
                ]
            })
            .collect(),
        Detail::Daily => {
            let mut days: BTreeMap<(String, String), (TimeQty, Vec<String>)> = BTreeMap::new();
            for &at in entries {
                let entry = &book.entries[at];
                let day = days
                    .entry((entry.date.clone(), entry.person.clone().unwrap_or_default()))
                    .or_default();
                day.0 = day.0.plus(entry.quantity);
                let work = work_of(entry);
                if !day.1.contains(&work) {
                    day.1.push(work);
                }
            }
            days.into_iter()
                .map(|((date, who), (quantity, work))| {
                    [date, who, work.join("; "), hours_2(quantity)]
                })
                .collect()
        }
    };
    if rows.is_empty() {
        return None;
    }
    let mut text = String::from(
        "## Timesheet\n\n| Date | Who | Work | Hours |\n|:-----|:----|:-----|------:|\n",
    );
    for row in rows {
        let cells: Vec<String> = row.iter().map(|c| escape_cell(c)).collect();
        text.push_str(&format!("| {} |\n", cells.join(" | ")));
    }
    Some(text)
}

/// What a bill for `request` would be.
///
/// # Errors
/// [`TimeOpError::NotFound`] for an unknown customer;
/// [`TimeOpError::BadRequest`]; [`TimeOpError::Conflict`] for a selection
/// that isn't the customer's unbilled time.
pub fn preview(
    books: &TimeBooks,
    book: &TimeBook,
    index: &DocumentIndex,
    vault: &dyn Vault,
    request: &BillRequest,
    today: &str,
) -> Result<BillPreview, TimeOpError> {
    let customer = customer_named(index, &request.customer)
        .map(|c| customer_time(index.site(), &c))
        .ok_or_else(|| {
            TimeOpError::NotFound(format!("no customer note is called {}", request.customer))
        })?;
    let selection = select(book, &customer, request, today)?;
    let catalogs: Vec<Catalog> = index.catalogs().cloned().collect();
    let money = index.money().style(None);
    let most_used = most_used_code(index, vault, &customer.path);
    let choices: BTreeMap<&str, &GroupChoice> =
        request.groups.iter().map(|c| (c.key.as_str(), c)).collect();
    let mut grouped: BTreeMap<(String, Option<String>, Option<String>), Vec<usize>> =
        BTreeMap::new();
    for &at in &selection.entries {
        let entry = &book.entries[at];
        let key = match request.group {
            Grouping::Account => (entry.account.clone(), None, None),
            Grouping::AccountPerson => (entry.account.clone(), entry.person.clone(), None),
            Grouping::AccountNote => (entry.account.clone(), None, Some(entry.note.clone())),
        };
        grouped.entry(key).or_default().push(at);
    }
    let label = match (&selection.from, &selection.to) {
        (Some(from), Some(to)) => Some(period_label(books, from, to)),
        _ => None,
    };
    let rounding: Rounding = customer.rounding;
    let mut groups = Vec::new();
    let (mut actual_total, mut rounded_total) = (TimeQty::ZERO, TimeQty::ZERO);
    let mut amount_total = Some(Dec::zero());
    let mut included = Vec::new();
    for ((account, person, note), ats) in grouped {
        let key = [Some(account.as_str()), person.as_deref(), note.as_deref()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" | ");
        let choice = choices.get(key.as_str()).copied();
        let excluded = choice.is_some_and(|c| c.exclude);
        let dated: Vec<(&str, TimeQty)> = ats
            .iter()
            .map(|&at| (book.entries[at].date.as_str(), book.entries[at].quantity))
            .collect();
        let actual = TimeQty::sum(dated.iter().map(|(_, q)| *q));
        let rounded = rounding.row(&dated);
        let suggestion = choice
            .and_then(|c| c.item.clone())
            .map(|code| (code, "choice"))
            .or_else(|| {
                customer
                    .item_rule(&account, person.as_deref())
                    .map(|rule| (rule.item.clone(), "time-items"))
            })
            .or_else(|| {
                book.account(&account)
                    .and_then(|a| a.item.as_ref())
                    .map(|r| (r.value.clone(), "account"))
            })
            .or_else(|| most_used.clone().map(|code| (code, "most-used")));
        let item = suggestion.map(|(code, source)| {
            let found = catalog_item(&catalogs, &code);
            ItemView {
                found: found.is_some(),
                description: found.map(|i| i.description.clone()),
                unit: found.and_then(|i| i.unit.clone()),
                rate: found.and_then(|i| i.rate_text.clone()),
                books: found.and_then(|i| i.books_text.clone()),
                code,
                source,
            }
        });
        let rate = choice
            .and_then(|c| c.rate.clone())
            .or_else(|| item.as_ref().and_then(|i| i.rate.clone()));
        let books_cell = choice
            .and_then(|c| c.account.clone())
            .or_else(|| item.as_ref().and_then(|i| i.books.clone()));
        let leaf = book
            .account(&account)
            .and_then(|a| {
                a.tags
                    .iter()
                    .find(|(name, _)| name == "description")
                    .map(|(_, v)| v.clone())
            })
            .unwrap_or_else(|| humanize(account.rsplit(':').next().unwrap_or(&account)));
        let base = match (&note, &person) {
            (Some(note), _) if !note.is_empty() => note.clone(),
            (_, Some(person)) => format!("{leaf} ({person})"),
            _ => leaf,
        };
        let description = choice
            .and_then(|c| c.description.clone())
            .unwrap_or_else(|| {
                label
                    .as_ref()
                    .map_or_else(|| base.clone(), |label| format!("{base}, {label}"))
            });
        let hours = hours_2(rounded);
        let amount = rate
            .as_deref()
            .and_then(|rate| money.parse(rate).ok())
            .and_then(|rate| {
                let quantity = Dec::parse(&hours, '.').ok()?;
                money.round(rate.value.mul(quantity).ok()?).ok()
            });
        let mut needs = Vec::new();
        if let Some(item) = &item
            && !item.found
            && choice.and_then(|c| c.rate.as_ref()).is_none()
        {
            needs.push(format!("the catalog has no item {}", item.code));
        }
        if rate.is_none() {
            needs.push(format!("{key}: an item or a rate"));
        } else if amount.is_none() {
            needs.push(format!(
                "{key}: the rate {} doesn't read as money",
                rate.as_deref().unwrap_or("")
            ));
        }
        if !excluded {
            actual_total = actual_total.plus(actual);
            rounded_total = rounded_total.plus(rounded);
            amount_total = match (amount_total, amount) {
                (Some(total), Some(amount)) => total.add(amount).ok(),
                _ => None,
            };
            included.extend(ats.iter().copied());
        }
        groups.push(BillGroup {
            key,
            account,
            person,
            note,
            entries: ats.iter().map(|&at| entry_id(&book.entries[at])).collect(),
            actual: actual.into(),
            rounded: rounded.into(),
            difference: super::hours_text(rounded.minus(actual)),
            hours,
            item,
            rate,
            books: books_cell,
            description,
            amount: amount.map(|a| money.format(a)),
            excluded,
            needs: if excluded { Vec::new() } else { needs },
        });
    }
    included.sort_unstable();
    let detail = request.detail.unwrap_or(customer.detail);
    let mut needs: Vec<String> = groups.iter().flat_map(|g| g.needs.clone()).collect();
    if included.is_empty() {
        needs.push(format!(
            "{} has no unbilled time {}",
            customer.title,
            match (&selection.from, &selection.to) {
                (Some(from), Some(to)) => format!("from {from} to {to}"),
                _ => "in the selection".to_string(),
            }
        ));
    }
    let all: Vec<usize> = book
        .entries
        .iter()
        .enumerate()
        .filter(|(_, e)| is_theirs(book, e, &customer))
        .map(|(at, _)| at)
        .collect();
    let periods = suggest_periods(
        book,
        &query::Batch {
            quantity: TimeQty::ZERO,
            oldest: String::new(),
            newest: String::new(),
            entries: all,
        },
        customer.cadence(),
        today,
    )
    .iter()
    .map(super::report::PeriodView::from)
    .collect();
    Ok(BillPreview {
        timesheet: timesheet(book, &included, detail),
        entries: included
            .iter()
            .map(|&at| entry_id(&book.entries[at]))
            .collect(),
        actual: actual_total.into(),
        rounded: rounded_total.into(),
        difference: super::hours_text(rounded_total.minus(actual_total)),
        amount: amount_total.map(|a| money.format(a)),
        ready: needs.is_empty(),
        needs,
        detail,
        from: selection.from,
        to: selection.to,
        group: request.group,
        groups,
        periods,
        customer,
    })
}

/// The refs of `ids`, each still unbilled time of `customer` in `book`.
fn refs_of(
    book: &TimeBook,
    ids: &[String],
    customer: &CustomerTime,
) -> Result<BTreeSet<EntryRef>, TimeOpError> {
    let mut refs = BTreeSet::new();
    for id in ids {
        for at in find_entries(book, id)? {
            let entry = &book.entries[at];
            if !is_theirs(book, entry, customer) {
                return Err(TimeOpError::Conflict(format!(
                    "{id} changed since the preview (it is no longer unbilled); preview again"
                )));
            }
            refs.insert(EntryRef::of(entry));
        }
    }
    Ok(refs)
}

/// Make the draft `request` describes and tag its time `invoice: N`: the
/// tags first, then the draft; when the draft can't be written the tags come
/// off again. `booked` is the books' `invoice:` numbers (numbering).
///
/// # Errors
/// [`TimeOpError::Conflict`] when the preview isn't ready (its `needs`) or
/// the time changed; [`TimeOpError::Draft`]; [`TimeOpError`].
#[allow(clippy::too_many_arguments)]
pub fn create(
    books: &TimeBooks,
    book: &TimeBook,
    index: &mut DocumentIndex,
    vault: &dyn Vault,
    booked: &[&str],
    request: &BillRequest,
    today: &str,
) -> Result<BillCreated, TimeOpError> {
    let preview = preview(books, book, index, vault, request, today)?;
    if !preview.ready {
        return Err(TimeOpError::Conflict(format!(
            "the bill isn't ready: {}",
            preview.needs.join("; ")
        )));
    }
    let rows: Vec<DraftRow> = preview
        .groups
        .iter()
        .filter(|g| !g.excluded)
        .map(|g| DraftRow {
            code: g.item.as_ref().filter(|i| i.found).map(|i| i.code.clone()),
            description: g.description.clone(),
            quantity: Some(g.hours.clone()),
            unit: Some(
                g.item
                    .as_ref()
                    .and_then(|i| i.unit.clone())
                    .unwrap_or_else(|| "h".to_string()),
            ),
            rate: g.rate.clone(),
            amount: None,
            books: g.books.clone(),
        })
        .collect();
    let new = NewInvoice {
        template: request.template.clone(),
        number: request.number.clone(),
        terms: request.terms,
        reference: request.reference.clone(),
        rows,
        supporting: preview.timesheet.clone(),
        ..NewInvoice::new(
            &preview.customer.customer.wikilink(),
            request.date.as_deref().unwrap_or(today),
        )
    };
    let made = draft::render(index, vault, &new, booked)?;
    let number = made.number.to_string();
    let invoice_tag = books.config.tags.name(TagRole::Invoice).to_string();
    let customer = preview.customer.clone();
    let tagged = books.write(Some(book), |book, files| {
        let refs = refs_of(book, &preview.entries, &customer)?;
        for at in &refs {
            super::entries::set_tag(files, at, &invoice_tag, Some(&number))?;
        }
        Ok(refs.len())
    })?;
    if let Err(error) = draft::create(index, vault, &made) {
        let undone = release(books, None, &number, Release::Release);
        return Err(match undone {
            Ok(_) => error.into(),
            Err(undo) => TimeOpError::Conflict(format!(
                "the draft wasn't written ({error}), and the time is still tagged invoice: {number} \
                 ({undo}); release it from Unbilled"
            )),
        });
    }
    let mut warnings = made.warnings.clone();
    let mut remembered = Vec::new();
    for group in preview.groups.iter().filter(|g| !g.excluded) {
        if !request
            .groups
            .iter()
            .any(|c| c.key == group.key && c.remember)
        {
            continue;
        }
        let Some(item) = group.item.as_ref().map(|i| i.code.clone()) else {
            continue;
        };
        let result = match (&group.person, request.group) {
            (Some(person), Grouping::AccountPerson) => remember_rule(
                index,
                vault,
                &preview.customer,
                &group.account,
                person,
                &item,
            )
            .map(|()| {
                format!(
                    "{}: {} for {person} bills as {item}",
                    customer.title, group.account
                )
            }),
            _ if book
                .account(&group.account)
                .and_then(|a| a.item.as_ref())
                .map(|r| r.value.as_str())
                == Some(item.as_str()) =>
            {
                continue;
            }
            _ => super::accounts::set_tag(books, None, &group.account, TagRole::Item, Some(&item))
                .map(|_| format!("{}: item: {item}", group.account)),
        };
        match result {
            Ok(done) => remembered.push(done),
            Err(error) => warnings.push(format!("{} wasn't remembered: {error}", group.key)),
        }
    }
    let total = index
        .doc(&made.path)
        .and_then(|d| d.summary.as_ref())
        .map(|s| s.total_text.clone());
    Ok(BillCreated {
        path: made.path,
        number,
        total,
        tagged: preview.entries.clone(),
        files: tagged
            .saved
            .iter()
            .map(|p| books.paths.relative(p))
            .collect(),
        remembered,
        warnings,
    })
}

/// Add `{account, person, item}` to the customer note's `time_items`.
fn remember_rule(
    index: &mut DocumentIndex,
    vault: &dyn Vault,
    customer: &CustomerTime,
    account: &str,
    person: &str,
    item: &str,
) -> Result<(), TimeOpError> {
    let mut rules: Vec<Value> = customer
        .items
        .iter()
        .filter(|r| !(r.account == account && r.person.as_deref() == Some(person)))
        .map(|r| {
            let mut rule = json!({ "account": r.account, "item": r.item });
            if let Some(person) = &r.person {
                rule["person"] = json!(person);
            }
            rule
        })
        .collect();
    rules.push(json!({ "account": account, "person": person, "item": item }));
    save_note_fields(
        index,
        vault,
        &customer.path,
        &[FieldEdit::Set(
            "time_items".to_string(),
            Value::Array(rules),
        )],
    )
}

/// What releasing an invoice's time does (T10, B-invoice lifecycle).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Release {
    /// Take the `invoice:` tag off: the time is unbilled again (a deleted
    /// draft; a void, by default).
    #[default]
    Release,
    /// Replace it with `nobill: invoice N void`: written off (a void, when
    /// asked).
    Nobill,
}

/// Take `invoice: number` off every line that carries it, or turn it into
/// `nobill`. Returns how many lines changed (0 when none carry it).
///
/// # Errors
/// [`TimeOpError`].
pub fn release(
    books: &TimeBooks,
    book: Option<&TimeBook>,
    number: &str,
    how: Release,
) -> Result<Written<usize>, TimeOpError> {
    if !books.is_set_up() {
        return Ok(Written {
            done: 0,
            saved: Vec::new(),
            retried: false,
            included: Vec::new(),
        });
    }
    books.write(book, |book, files| {
        let names = &book.tags;
        let refs: BTreeSet<EntryRef> = book
            .entries
            .iter()
            .filter(|entry| entry.tag(names, TagRole::Invoice) == Some(number))
            .map(EntryRef::of)
            .collect();
        for at in &refs {
            super::entries::set_tag(files, at, names.name(TagRole::Invoice), None)?;
            if how == Release::Nobill {
                super::entries::set_tag(
                    files,
                    at,
                    names.name(TagRole::Nobill),
                    Some(&format!("invoice {number} void")),
                )?;
            }
        }
        Ok(refs.len())
    })
}
