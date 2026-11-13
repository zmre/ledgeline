//! Single-entry app exports: one row per transaction, an account column and a
//! category column (plan 39 Phase 5).
//!
//! Each row becomes a two-posting [`QbTransaction`]: the row's **account** (the
//! register it sits in) and its **category** as the counter account. Both are
//! raw source names. The server's multi-posting pipeline then asks the user to
//! map every one of them to a journal account with `alias` lines, so a category
//! like `Food & Dining: Groceries` becomes `expenses:food:groceries` exactly the
//! way a QuickBooks account name does.
//!
//! The plan suggested generating CSV rules files for these apps. We use the
//! multi-posting pipeline instead, because it does the category→account mapping
//! and the transfer pairing in one reviewed step, works across every account in
//! the export at once (a rules file serves one bank account), and gives the same
//! `id:` dedupe on re-import.
//!
//! # Presets and sign conventions
//!
//! | [`Preset`]          | Columns (by header name)                                                                 | Account-side amount        |
//! |---------------------|------------------------------------------------------------------------------------------|----------------------------|
//! | [`Preset::Ynab`]    | `Account, Flag, Date, Payee, Category Group/Category, Category Group, Category, Memo, Outflow, Inflow, Cleared` | `Inflow - Outflow`         |
//! | [`Preset::Monarch`] | `Date, Merchant, Category, Account, Original Statement, Notes, Amount, Tags` (`Owner`)  | `Amount` (negative = spend)|
//! | [`Preset::Copilot`] | `date, name, amount, type, category, account, account mask, excluded, notes`            | `-amount` (positive = spend), see below |
//!
//! The category posting is always the negation, so every transaction balances by
//! construction.
//!
//! Copilot's sign is calibrated from the file: if most rows whose `type` is
//! `income` are positive, positive means money in and the sign is not flipped.
//! With no income rows to go on, positive means spend.
//!
//! # Transfers
//!
//! A transfer between two exported accounts appears **twice**, once in each
//! account. Importing both rows would double it, so they are joined into one
//! transaction between the two accounts:
//!
//! - **YNAB** names the other side: payee `Transfer : Savings`. A row in
//!   `Checking` for `x` to `Savings` pairs with a row in `Savings` for `-x` to
//!   `Checking` on the same date. One of the two is dropped (the one that is not
//!   part of a split, else the later one). An unpaired transfer row posts to the
//!   named account itself.
//! - **Monarch** and **Copilot** only say *that* a row is a transfer: its
//!   category is `Transfer` or `Credit Card Payment` (Monarch), or its `type`
//!   contains `transfer` (Copilot). Such a row pairs with one in a different
//!   account for the opposite amount within [`PAIR_WINDOW_DAYS`] days, closest
//!   date first, then nearest in the file. The pair becomes one transaction
//!   dated on the outflow side. An unpaired row stays a normal two-posting
//!   transaction against its category, which the user maps (to a clearing
//!   account, say).
//!
//! # YNAB splits
//!
//! A split is written as consecutive rows in one account on one date whose memos
//! start `Split (1/3)`, `Split (2/3)`, `Split (3/3)`. Those rows become one
//! transaction: one posting to the account for their total and one per
//! category, each carrying its memo with the prefix removed. A run that does not
//! complete is left as separate transactions; grouping is cosmetic here, since
//! every line balances on its own.
//!
//! # Ids
//!
//! None of these apps exports a transaction id, so every id is the shared
//! content hash (date, payee, each posting's account and amount; see
//! `grouped_csv::content_id`) with an occurrence index, prefixed `ynab-`,
//! `monarch-` or `copilot-`. Editing a memo in the app does not change an id;
//! editing an amount, date, payee, account or category does.
//!
//! # What does not come through
//!
//! YNAB flags; Monarch tags, owner and original statement text (kept only as the
//! payee when there is no merchant); Copilot tags, recurring markers and
//! account masks beyond telling two same-named accounts apart. Budgets and
//! balance history are not transactions and are not read.

use crate::decimal::Dec;
use crate::grouped_csv::{
    Draft, GroupedCsvError, Header, ImportReport, Parsed, Table, cell, column_cells, day_number,
    decimal_mark, finish, header_of, money_at, owned, posting, prefix_of,
};
use crate::qb_journal::{ImportSource, ImportStatus, QbJournal, SourceAccountKind};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};

/// How far apart, in days, the two sides of a Monarch or Copilot transfer may be
/// dated. Banks post the two legs of one transfer a day or two apart, and
/// across a weekend more.
pub const PAIR_WINDOW_DAYS: i64 = 4;

/// How many same-amount candidates one transfer row is compared against. A
/// bound on hostile input (100 000 identical rows), not a policy: real files
/// never come near it.
const PAIR_SCAN_LIMIT: usize = 256;

/// Which single-entry app a file came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Preset {
    Ynab,
    Monarch,
    Copilot,
}

impl Preset {
    pub const ALL: [Self; 3] = [Self::Ynab, Self::Monarch, Self::Copilot];

    #[must_use]
    pub fn source(self) -> ImportSource {
        match self {
            Self::Ynab => ImportSource::Ynab,
            Self::Monarch => ImportSource::Monarch,
            Self::Copilot => ImportSource::Copilot,
        }
    }

    fn matches(self, header: &Header) -> bool {
        match self {
            Self::Ynab => header.has_all(&[
                "account",
                "payee",
                "categorygroupcategory",
                "outflow",
                "inflow",
            ]),
            Self::Monarch => header.has_all(&[
                "date",
                "merchant",
                "category",
                "account",
                "originalstatement",
                "amount",
            ]),
            Self::Copilot => header.has_all(&[
                "date",
                "name",
                "amount",
                "account",
                "accountmask",
                "excluded",
            ]),
        }
    }
}

/// Which single-entry preset, if any, `bytes` is.
///
/// Conservative: each rule needs a column no bank statement has. YNAB's
/// `Category Group/Category` with `Outflow`/`Inflow`; Monarch's
/// `Original Statement` with `Merchant` and `Category`; Copilot's
/// `account mask` with `excluded`.
#[must_use]
pub fn detect_preset(name: &str, bytes: &[u8]) -> Option<Preset> {
    let header = header_of(name, bytes)?;
    Preset::ALL
        .into_iter()
        .find(|preset| preset.matches(&header))
}

/// Is `bytes` any single-entry preset? See [`detect_preset`].
#[must_use]
pub fn detect(name: &str, bytes: &[u8]) -> bool {
    detect_preset(name, bytes).is_some()
}

/// Parse `bytes` as whichever preset its header matches.
///
/// # Errors
///
/// [`GroupedCsvError::NotRecognised`] when no preset matches; otherwise any
/// refusal of [`parse_preset`].
pub fn parse(bytes: &[u8]) -> Result<QbJournal, GroupedCsvError> {
    let table = Table::read(bytes)?;
    let header = Header::new(&table.header);
    let preset = Preset::ALL
        .into_iter()
        .find(|preset| preset.matches(&header))
        .ok_or(GroupedCsvError::NotRecognised)?;
    parse_table(preset, &table).map(|parsed| parsed.journal)
}

/// Parse delimited `bytes` as `preset`.
///
/// # Errors
///
/// Any [`GroupedCsvError`].
pub fn parse_preset(preset: Preset, bytes: &[u8]) -> Result<Parsed, GroupedCsvError> {
    parse_table(preset, &Table::read(bytes)?)
}

fn parse_table(preset: Preset, table: &Table) -> Result<Parsed, GroupedCsvError> {
    let mut report = table.report();
    let (lines, guess) = match preset {
        Preset::Ynab => ynab(table)?,
        Preset::Monarch => monarch(table)?,
        Preset::Copilot => copilot(table, &mut report)?,
    };
    // The accounts the app tracks; everything else a row posts to is a
    // category (or an unpaired transfer's raw account name).
    let tracked: Vec<String> = lines.iter().map(|line| line.account.clone()).collect();
    let drafts = match preset {
        Preset::Ynab => assemble_named(lines, &mut report),
        Preset::Monarch | Preset::Copilot => assemble_paired(lines, &mut report),
    };
    let mut journal = finish(drafts, guess, preset.source(), prefix_of(preset.source()))?;
    journal.account_kinds.extend(
        tracked
            .into_iter()
            .map(|account| (account, SourceAccountKind::Tracked)),
    );
    crate::qb_import::classify_remaining_by_sign(&mut journal);
    Ok(Parsed { journal, report })
}

// ---------------------------------------------------------------------------
// Rows
// ---------------------------------------------------------------------------

/// What a row's money goes against.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Counter {
    /// An ordinary category.
    Category(String),
    /// A YNAB transfer naming the other account.
    TransferTo(String),
    /// A Monarch/Copilot transfer: pair it if possible, else post to this
    /// category.
    TransferCandidate(String),
}

/// One row, read and dated.
#[derive(Debug, Clone)]
struct Line {
    row: usize,
    date: String,
    account: String,
    payee: Option<String>,
    counter: Counter,
    /// Positive = money into `account`.
    amount: Dec,
    memo: Option<String>,
    status: ImportStatus,
    /// YNAB `Split (n/N)`: `(n, N)`.
    split: Option<(usize, usize)>,
}

type Lines = (Vec<Line>, crate::rules::generate::DateFormatGuess);

/// Date every row with the file's one date format.
fn dated<T>(
    table: &Table,
    rows: Vec<(usize, &str, T)>,
    build: impl Fn(usize, String, T) -> Line,
) -> Result<Lines, GroupedCsvError> {
    let dates = crate::grouped_csv::Dates::guess(rows.iter().map(|(_, date, _)| *date))?;
    let lines = rows
        .into_iter()
        .map(|(index, date, rest)| {
            let row = table.row_number(index);
            dates.iso(date, row).map(|iso| build(row, iso, rest))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok((lines, dates.guess))
}

/// The required date cell of row `index`.
fn date_cell(table: &Table, index: usize, column: usize) -> Result<&str, GroupedCsvError> {
    cell(&table.rows[index], Some(column)).ok_or(GroupedCsvError::MissingDate {
        row: table.row_number(index),
    })
}

/// The required account cell of row `index`.
fn account_cell(table: &Table, index: usize, column: usize) -> Result<String, GroupedCsvError> {
    owned(&table.rows[index], Some(column)).ok_or(GroupedCsvError::MissingAccount {
        row: table.row_number(index),
    })
}

/// The category every app uses for a row it was never given one.
const UNCATEGORIZED: &str = "Uncategorized";

// ---------------------------------------------------------------------------
// YNAB
// ---------------------------------------------------------------------------

fn ynab(table: &Table) -> Result<Lines, GroupedCsvError> {
    let header = Header::new(&table.header);
    let account = header.require(&["account"], "Account")?;
    let date = header.require(&["date"], "Date")?;
    let payee = header.find(&["payee"]);
    let full_category = header.find(&["categorygroupcategory"]);
    let group = header.find(&["categorygroup"]);
    let category = header.find(&["category"]);
    let memo = header.find(&["memo"]);
    let outflow = header.require(&["outflow"], "Outflow")?;
    let inflow = header.require(&["inflow"], "Inflow")?;
    let cleared = header.find(&["cleared"]);

    let mark = decimal_mark(column_cells(table, &[Some(outflow), Some(inflow)]));
    let rows = (0..table.rows.len())
        .map(|index| {
            let row = &table.rows[index];
            let at = table.row_number(index);
            let out = money_at(cell(row, Some(outflow)), mark, at)?.unwrap_or_else(Dec::zero);
            let into = money_at(cell(row, Some(inflow)), mark, at)?.unwrap_or_else(Dec::zero);
            let amount = into
                .sub(out)
                .map_err(|_| GroupedCsvError::AmountOverflow { row: at })?;
            let who = owned(row, payee);
            let counter = match who.as_deref().and_then(transfer_target) {
                Some(target) => Counter::TransferTo(target.to_string()),
                None => Counter::Category(
                    owned(row, full_category)
                        .or_else(|| match (cell(row, group), cell(row, category)) {
                            (Some(g), Some(c)) => Some(format!("{g}: {c}")),
                            (g, c) => g.or(c).map(str::to_string),
                        })
                        .unwrap_or_else(|| {
                            // YNAB gives an account's opening row no category.
                            if who.as_deref() == Some("Starting Balance") {
                                "Starting Balance".to_string()
                            } else {
                                UNCATEGORIZED.to_string()
                            }
                        }),
                ),
            };
            let (split, note) = split_memo(cell(row, memo));
            let status = match cell(row, cleared).map(str::to_ascii_lowercase).as_deref() {
                Some("cleared" | "reconciled") => ImportStatus::Cleared,
                _ => ImportStatus::Unmarked,
            };
            Ok((
                index,
                date_cell(table, index, date)?,
                (
                    account_cell(table, index, account)?,
                    who,
                    counter,
                    amount,
                    note,
                    status,
                    split,
                ),
            ))
        })
        .collect::<Result<Vec<_>, GroupedCsvError>>()?;
    dated(
        table,
        rows,
        |row, date, (account, payee, counter, amount, memo, status, split)| Line {
            row,
            date,
            account,
            payee,
            counter,
            amount,
            memo,
            status,
            split,
        },
    )
}

/// `Transfer : Savings` → `Savings`.
fn transfer_target(payee: &str) -> Option<&str> {
    let rest = payee.strip_prefix("Transfer")?.trim_start();
    let target = rest.strip_prefix(':')?.trim();
    (!target.is_empty()).then_some(target)
}

/// `Split (2/3) lunch` → `(Some((2, 3)), Some("lunch"))`.
fn split_memo(memo: Option<&str>) -> (Option<(usize, usize)>, Option<String>) {
    let parsed = memo.and_then(|text| {
        let rest = text.strip_prefix("Split (")?;
        let (fraction, after) = rest.split_once(')')?;
        let (n, total) = fraction.split_once('/')?;
        let (n, total) = (n.trim().parse().ok()?, total.trim().parse().ok()?);
        ((1..=total).contains(&n) && total > 1).then(|| {
            let note = after.trim();
            ((n, total), (!note.is_empty()).then(|| note.to_string()))
        })
    });
    match parsed {
        Some((split, note)) => (Some(split), note),
        None => (None, memo.map(str::to_string)),
    }
}

/// Consecutive split rows (same account and date, same `N`, distinct `n`) as one
/// group each; every other row alone.
fn split_groups(lines: &[Line]) -> Vec<Vec<usize>> {
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut open: Option<(usize, HashSet<usize>)> = None;
    for (index, line) in lines.iter().enumerate() {
        let joins = match (&open, line.split, groups.last()) {
            (Some((total, seen)), Some((n, of)), Some(group)) => {
                let head = &lines[group[0]];
                of == *total
                    && !seen.contains(&n)
                    && head.account == line.account
                    && head.date == line.date
            }
            _ => false,
        };
        if joins {
            if let (Some(group), Some((_, seen)), Some((n, _))) =
                (groups.last_mut(), open.as_mut(), line.split)
            {
                group.push(index);
                seen.insert(n);
            }
        } else {
            groups.push(vec![index]);
            open = line.split.map(|(n, total)| (total, HashSet::from([n])));
        }
        if open
            .as_ref()
            .is_some_and(|(total, seen)| seen.len() >= *total)
        {
            open = None;
        }
    }
    groups
}

/// YNAB: group splits, drop the second side of each named transfer, and build
/// one transaction per group.
fn assemble_named(lines: Vec<Line>, report: &mut ImportReport) -> Vec<Draft> {
    let groups = split_groups(&lines);
    let group_of: Vec<usize> = groups
        .iter()
        .enumerate()
        .flat_map(|(g, members)| members.iter().map(move |&m| (m, g)))
        .fold(vec![0; lines.len()], |mut acc, (m, g)| {
            acc[m] = g;
            acc
        });

    // Pair `A → B, x` with `B → A, -x` on the same date.
    let key = |line: &Line, from: &str, to: &str, amount: Dec| {
        (
            line.date.clone(),
            from.to_string(),
            to.to_string(),
            render_key(amount),
        )
    };
    let mut waiting: HashMap<(String, String, String, String), VecDeque<usize>> = HashMap::new();
    let mut dropped: HashSet<usize> = HashSet::new();
    for (index, line) in lines.iter().enumerate() {
        let Counter::TransferTo(target) = &line.counter else {
            continue;
        };
        let Ok(opposite) = line.amount.neg() else {
            continue;
        };
        let partner = waiting
            .get_mut(&key(line, target, &line.account, opposite))
            .and_then(VecDeque::pop_front);
        match partner {
            Some(earlier) => {
                report.transfers_paired += 1;
                let single = |i: usize| groups[group_of[i]].len() == 1;
                // Keep the side that is part of a split; else keep the earlier.
                dropped.insert(if single(index) || !single(earlier) {
                    index
                } else {
                    earlier
                });
            }
            None => waiting
                .entry(key(line, &line.account, target, line.amount))
                .or_default()
                .push_back(index),
        }
    }
    report.transfers_unpaired += waiting.values().map(VecDeque::len).sum::<usize>();

    groups
        .into_iter()
        .filter_map(|members| {
            let kept: Vec<&Line> = members
                .iter()
                .filter(|m| !dropped.contains(m))
                .map(|&m| &lines[m])
                .collect();
            let head = *kept.first()?;
            let total = kept
                .iter()
                .try_fold(Dec::zero(), |sum, line| sum.add(line.amount))
                .ok()?;
            let counters = kept.iter().filter_map(|line| {
                let account = match &line.counter {
                    Counter::Category(name)
                    | Counter::TransferTo(name)
                    | Counter::TransferCandidate(name) => name,
                };
                Some(posting(account, line.amount.neg().ok()?, line.memo.clone()))
            });
            let postings: Vec<_> = std::iter::once(posting(&head.account, total, None))
                .chain(counters)
                .collect();
            let kind = if kept
                .iter()
                .all(|line| matches!(line.counter, Counter::TransferTo(_)))
            {
                "Transfer"
            } else if kept.len() > 1 {
                "Split"
            } else {
                "Transaction"
            };
            Some(Draft {
                id: None,
                row: head.row,
                date: head.date.clone(),
                kind: kind.to_string(),
                num: None,
                name: head.payee.clone(),
                postings,
                status: head.status,
            })
        })
        .collect()
}

fn render_key(amount: Dec) -> String {
    crate::edit::render_dec(amount.normalized(), '.')
}

// ---------------------------------------------------------------------------
// Monarch
// ---------------------------------------------------------------------------

/// Monarch's transfer categories (compared case-insensitively).
const MONARCH_TRANSFERS: [&str; 2] = ["transfer", "credit card payment"];

fn monarch(table: &Table) -> Result<Lines, GroupedCsvError> {
    let header = Header::new(&table.header);
    let date = header.require(&["date"], "Date")?;
    let merchant = header.find(&["merchant"]);
    let statement = header.find(&["originalstatement"]);
    let category = header.find(&["category"]);
    let account = header.require(&["account"], "Account")?;
    let notes = header.find(&["notes"]);
    let amount = header.require(&["amount"], "Amount")?;

    let mark = decimal_mark(column_cells(table, &[Some(amount)]));
    let rows = (0..table.rows.len())
        .map(|index| {
            let row = &table.rows[index];
            let at = table.row_number(index);
            let value = money_at(cell(row, Some(amount)), mark, at)?
                .ok_or(GroupedCsvError::MissingAmount { row: at })?;
            let name = owned(row, category).unwrap_or_else(|| UNCATEGORIZED.to_string());
            let counter = if MONARCH_TRANSFERS.contains(&name.to_ascii_lowercase().as_str()) {
                Counter::TransferCandidate(name)
            } else {
                Counter::Category(name)
            };
            Ok((
                index,
                date_cell(table, index, date)?,
                (
                    account_cell(table, index, account)?,
                    owned(row, merchant).or_else(|| owned(row, statement)),
                    counter,
                    value,
                    owned(row, notes),
                ),
            ))
        })
        .collect::<Result<Vec<_>, GroupedCsvError>>()?;
    dated(
        table,
        rows,
        |row, date, (account, payee, counter, amount, memo)| Line {
            row,
            date,
            account,
            payee,
            counter,
            amount,
            memo,
            status: ImportStatus::Unmarked,
            split: None,
        },
    )
}

// ---------------------------------------------------------------------------
// Copilot
// ---------------------------------------------------------------------------

fn copilot(table: &Table, report: &mut ImportReport) -> Result<Lines, GroupedCsvError> {
    let header = Header::new(&table.header);
    let date = header.require(&["date"], "date")?;
    let name = header.find(&["name"]);
    let amount = header.require(&["amount"], "amount")?;
    let kind = header.find(&["type"]);
    let category = header.find(&["category"]);
    let account = header.require(&["account"], "account")?;
    let mask = header.find(&["accountmask"]);
    let excluded = header.find(&["excluded"]);
    let notes = header.find(&["notes", "note"]);
    let status = header.find(&["status"]);

    let mark = decimal_mark(column_cells(table, &[Some(amount)]));
    let kept: Vec<usize> = (0..table.rows.len())
        .filter(|&index| {
            !cell(&table.rows[index], excluded).is_some_and(|flag| {
                matches!(flag.to_ascii_lowercase().as_str(), "true" | "yes" | "1")
            })
        })
        .collect();
    report.skipped_transactions += table.rows.len() - kept.len();

    let values = kept
        .iter()
        .map(|&index| {
            let at = table.row_number(index);
            money_at(cell(&table.rows[index], Some(amount)), mark, at)?
                .ok_or(GroupedCsvError::MissingAmount { row: at })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let type_of = |index: usize| {
        cell(&table.rows[index], kind)
            .map(str::to_ascii_lowercase)
            .unwrap_or_default()
    };
    // Calibrate the sign from income rows; default: positive = spend.
    let (income_positive, income_negative) = kept
        .iter()
        .zip(&values)
        .filter(|(index, _)| type_of(**index) == "income")
        .fold((0usize, 0usize), |(pos, neg), (_, value)| {
            if *value > Dec::zero() {
                (pos + 1, neg)
            } else {
                (pos, neg + 1)
            }
        });
    let positive_is_inflow = income_positive > income_negative;

    let rows = kept
        .iter()
        .zip(values)
        .map(|(&index, value)| {
            let row = &table.rows[index];
            let at = table.row_number(index);
            let into = if positive_is_inflow {
                value
            } else {
                value
                    .neg()
                    .map_err(|_| GroupedCsvError::AmountOverflow { row: at })?
            };
            let label = owned(row, category).unwrap_or_else(|| UNCATEGORIZED.to_string());
            let counter = if type_of(index).contains("transfer")
                || MONARCH_TRANSFERS.contains(&label.to_ascii_lowercase().as_str())
            {
                Counter::TransferCandidate(label)
            } else {
                Counter::Category(label)
            };
            let base = account_cell(table, index, account)?;
            let full = match cell(row, mask) {
                Some(mask) => format!("{base} {mask}"),
                None => base,
            };
            let state = match cell(row, status).map(str::to_ascii_lowercase).as_deref() {
                Some("pending") => ImportStatus::Pending,
                _ => ImportStatus::Unmarked,
            };
            Ok((
                index,
                date_cell(table, index, date)?,
                (
                    full,
                    owned(row, name),
                    counter,
                    into,
                    owned(row, notes),
                    state,
                ),
            ))
        })
        .collect::<Result<Vec<_>, GroupedCsvError>>()?;
    dated(
        table,
        rows,
        |row, date, (account, payee, counter, amount, memo, status)| Line {
            row,
            date,
            account,
            payee,
            counter,
            amount,
            memo,
            status,
            split: None,
        },
    )
}

// ---------------------------------------------------------------------------
// Pairing unnamed transfers (Monarch, Copilot)
// ---------------------------------------------------------------------------

/// Pair transfer candidates, then build one transaction per pair and per
/// remaining row, in file order.
fn assemble_paired(lines: Vec<Line>, report: &mut ImportReport) -> Vec<Draft> {
    let days: Vec<Option<i64>> = lines.iter().map(|line| day_number(&line.date)).collect();
    // Candidates bucketed by absolute amount, in file order.
    let mut buckets: HashMap<String, Vec<usize>> = HashMap::new();
    for (index, line) in lines.iter().enumerate() {
        if matches!(line.counter, Counter::TransferCandidate(_))
            && let Ok(size) = line.amount.abs()
        {
            buckets.entry(render_key(size)).or_default().push(index);
        }
    }
    let mut partner: Vec<Option<usize>> = vec![None; lines.len()];
    for index in 0..lines.len() {
        if partner[index].is_some()
            || !matches!(lines[index].counter, Counter::TransferCandidate(_))
        {
            continue;
        }
        let (line, Some(day)) = (&lines[index], days[index]) else {
            continue;
        };
        let Ok(size) = line.amount.abs() else {
            continue;
        };
        let Ok(opposite) = line.amount.neg() else {
            continue;
        };
        let best = buckets
            .get(&render_key(size))
            .into_iter()
            .flatten()
            .copied()
            .filter(|&other| other != index && partner[other].is_none())
            .take(PAIR_SCAN_LIMIT)
            .filter(|&other| {
                lines[other].account != line.account && lines[other].amount == opposite
            })
            .filter_map(|other| {
                let gap = (days[other]? - day).abs();
                (gap <= PAIR_WINDOW_DAYS).then_some((gap, other.abs_diff(index), other))
            })
            .min()
            .map(|(_, _, other)| other);
        if let Some(other) = best {
            partner[index] = Some(other);
            partner[other] = Some(index);
            report.transfers_paired += 1;
        }
    }
    report.transfers_unpaired += lines
        .iter()
        .zip(&partner)
        .filter(|(line, paired)| {
            paired.is_none() && matches!(line.counter, Counter::TransferCandidate(_))
        })
        .count();

    lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| match partner[index] {
            Some(other) if other < index => None,
            Some(other) => {
                let pair = &lines[other];
                let (out, into) = if line.amount < Dec::zero() {
                    (line, pair)
                } else {
                    (pair, line)
                };
                Some(Draft {
                    id: None,
                    row: line.row,
                    date: out.date.clone(),
                    kind: "Transfer".to_string(),
                    num: None,
                    name: out.payee.clone(),
                    postings: vec![
                        posting(&out.account, out.amount, out.memo.clone()),
                        posting(&into.account, into.amount, into.memo.clone()),
                    ],
                    status: out.status,
                })
            }
            None => {
                let counter = match &line.counter {
                    Counter::Category(name)
                    | Counter::TransferTo(name)
                    | Counter::TransferCandidate(name) => name,
                };
                Some(Draft {
                    id: None,
                    row: line.row,
                    date: line.date.clone(),
                    kind: "Transaction".to_string(),
                    num: None,
                    name: line.payee.clone(),
                    postings: vec![
                        posting(&line.account, line.amount, None),
                        posting(counter, line.amount.neg().ok()?, line.memo.clone()),
                    ],
                    status: line.status,
                })
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_ynab_transfer_payees() {
        assert_eq!(transfer_target("Transfer : Savings"), Some("Savings"));
        assert_eq!(transfer_target("Transfer: Visa Card"), Some("Visa Card"));
        assert_eq!(transfer_target("Transferwise"), None);
        assert_eq!(transfer_target("Transfer : "), None);
    }

    #[test]
    fn reads_ynab_split_memos() {
        assert_eq!(
            split_memo(Some("Split (2/3) lunch")),
            (Some((2, 3)), Some("lunch".to_string()))
        );
        assert_eq!(split_memo(Some("Split (1/2)")), (Some((1, 2)), None));
        assert_eq!(
            split_memo(Some("Split (4/3) odd")),
            (None, Some("Split (4/3) odd".to_string()))
        );
        assert_eq!(split_memo(Some("plain")), (None, Some("plain".to_string())));
        assert_eq!(split_memo(None), (None, None));
    }
}
