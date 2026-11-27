//! The Accounts page (plan 46 T-accounts): the time-account tree with each
//! tag resolved and where it comes from, this month's and the unbilled
//! hours, the accounts used but not declared, and the actions — declare,
//! set a tag (link a customer, billable, default item), archive, rename
//! (with a preview).
//!
//! Every write is to the accounts file (F-accounts layout, through
//! [`AccountsDoc`]) or, for a rename, to it and every time file
//! ([`rename_plan`]), verified and saved together.

use super::entries::Billed;
use super::{Qty, TimeBooks, TimeOpError, Written};
use crate::index::DocumentIndex;
use ledgeline_core::time::accounts::{AccountsDoc, by_segment};
use ledgeline_core::time::files::{TimeFiles, rename_plan};
use ledgeline_core::time::model::{Resolved, TimeBook, ancestors_and_self};
use ledgeline_core::time::{TagRole, TimeError, TimeQty, check_account};
use ledgeline_core::vault::VaultPath;
use serde::Serialize;
use std::collections::BTreeMap;

/// A tag value with where it is set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Inherited {
    /// The value.
    pub value: String,
    /// The account that declares it.
    pub from: String,
    /// Whether that is an ancestor (*inherited from …*).
    pub inherited: bool,
}

fn inherited(account: &str, resolved: Option<&Resolved>) -> Option<Inherited> {
    resolved.map(|r| Inherited {
        value: r.value.clone(),
        from: r.from.clone(),
        inherited: r.from != account,
    })
}

/// One row of the tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AccountRow {
    /// The full name.
    pub account: String,
    /// 0 for a top-level branch.
    pub depth: usize,
    /// Whether an `account` directive declares it.
    pub declared: bool,
    /// Whether entries use it directly.
    pub used: bool,
    /// Its own tags, as declared.
    pub tags: Vec<(String, String)>,
    /// `customer:`, resolved.
    pub customer: Option<Inherited>,
    /// Whether a customer note has that name.
    pub customer_found: bool,
    /// `billable:`, resolved (none means no).
    pub billable: Option<Inherited>,
    /// `item:`, resolved.
    pub item: Option<Inherited>,
    /// `archived:`, resolved.
    pub archived: Option<Inherited>,
    /// This month's time on it and below.
    pub month: Qty,
    /// Its unbilled time and below.
    pub unbilled: Qty,
    /// Everything on it and below.
    pub total: Qty,
    /// What deserves a look: a customer subtree with no `billable`.
    pub warnings: Vec<String>,
}

/// The Accounts page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AccountsView {
    /// Every account, parents first, sorted by segment.
    pub accounts: Vec<AccountRow>,
    /// Used but not declared.
    pub undeclared: Vec<String>,
    /// The accounts file.
    pub file: String,
    /// Whether it exists.
    pub file_exists: bool,
}

/// The tree, with `month` (`YYYY-MM`) for the month column.
#[must_use]
pub fn view(
    books: &TimeBooks,
    book: &TimeBook,
    index: &DocumentIndex,
    month: &str,
) -> AccountsView {
    let mut month_total: BTreeMap<String, TimeQty> = BTreeMap::new();
    let mut unbilled: BTreeMap<String, TimeQty> = BTreeMap::new();
    let mut total: BTreeMap<String, TimeQty> = BTreeMap::new();
    for entry in &book.entries {
        let in_month = entry.date.starts_with(month);
        let open = Billed::of(book, entry) == Billed::Unbilled;
        for at in ancestors_and_self(&entry.account) {
            let add = |map: &mut BTreeMap<String, TimeQty>| {
                let slot = map.entry(at.clone()).or_default();
                *slot = slot.plus(entry.quantity);
            };
            add(&mut total);
            if in_month {
                add(&mut month_total);
            }
            if open {
                add(&mut unbilled);
            }
        }
    }
    let customers = index.customers();
    let mut rows: Vec<AccountRow> = book
        .accounts
        .values()
        .map(|account| {
            let customer = inherited(&account.name, account.customer.as_ref());
            let customer_found = customer.as_ref().is_some_and(|c| {
                customers
                    .iter()
                    .any(|n| n.ledger_customer == c.value || n.contact.title == c.value)
            });
            let mut warnings = Vec::new();
            if account
                .customer
                .as_ref()
                .is_some_and(|c| c.from == account.name)
                && account.billable.is_none()
            {
                warnings.push(format!(
                    "{} links a customer but says nothing about billable, so its time is not \
                     billable; add billable: yes or no",
                    account.name
                ));
            }
            if customer.is_some() && !customer_found {
                warnings.push(format!(
                    "no customer note is called {}",
                    customer.as_ref().map_or("", |c| c.value.as_str())
                ));
            }
            let get = |map: &BTreeMap<String, TimeQty>| {
                map.get(&account.name).copied().unwrap_or_default().into()
            };
            AccountRow {
                depth: account.name.matches(':').count(),
                declared: account.declared,
                used: account.used,
                tags: account.tags.clone(),
                customer,
                customer_found,
                billable: inherited(&account.name, account.billable.as_ref()),
                item: inherited(&account.name, account.item.as_ref()),
                archived: inherited(&account.name, account.archived.as_ref()),
                month: get(&month_total),
                unbilled: get(&unbilled),
                total: get(&total),
                warnings,
                account: account.name.clone(),
            }
        })
        .collect();
    rows.sort_by(|a, b| by_segment(&a.account, &b.account));
    AccountsView {
        accounts: rows,
        undeclared: book.undeclared().into_iter().map(str::to_string).collect(),
        file: books.paths.relative(&books.paths.accounts),
        file_exists: books
            .vault()
            .stat(&books.paths.accounts)
            .ok()
            .flatten()
            .is_some(),
    }
}

/// Edit the accounts file in `files` with `edit` (its text → new text).
fn edit_accounts(
    books: &TimeBooks,
    files: &mut TimeFiles,
    edit: impl FnOnce(&AccountsDoc) -> Result<String, TimeError>,
) -> Result<(), TimeOpError> {
    let path = books.paths.accounts.clone();
    if !files.exists(&path)? {
        return Err(TimeOpError::NotFound(format!(
            "the accounts file {} doesn't exist; set up time tracking, or create it and include it \
             from the roll-up",
            books.paths.relative(&path)
        )));
    }
    files.edit(&path, None, |text| edit(&AccountsDoc::parse(text)))?;
    Ok(())
}

/// Declare `name` with `tags` (T-accounts *Add child*, *Declare*).
///
/// # Errors
/// [`TimeOpError`].
pub fn declare(
    books: &TimeBooks,
    book: Option<&TimeBook>,
    name: &str,
    tags: &[(String, String)],
) -> Result<Written<()>, TimeOpError> {
    check_account(name)?;
    books.write(book, |_, files| {
        edit_accounts(books, files, |doc| doc.declare(name, tags))
    })
}

/// Set tag `role` on `name` to `value`, or remove it with `None` (*Link
/// customer*, *Billable*, *Default item*, *Archive*). An undeclared account
/// is declared with it.
///
/// # Errors
/// [`TimeOpError`].
pub fn set_tag(
    books: &TimeBooks,
    book: Option<&TimeBook>,
    name: &str,
    role: TagRole,
    value: Option<&str>,
) -> Result<Written<()>, TimeOpError> {
    check_account(name)?;
    let tag = books.config.tags.name(role).to_string();
    books.write(book, |_, files| {
        edit_accounts(books, files, |doc| {
            if doc.declaration(name).is_some() {
                doc.set_tag(name, &tag, value)
            } else {
                let tags: Vec<(String, String)> = value
                    .map(|v| vec![(tag.clone(), v.to_string())])
                    .unwrap_or_default();
                doc.declare(name, &tags)
            }
        })
    })
}

/// A rename's preview: each changed line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RenamePreview {
    /// The old name.
    pub from: String,
    /// The new name.
    pub to: String,
    /// The changed lines, by file.
    pub lines: Vec<RenamedLineView>,
    /// The files written (or to be).
    pub files: Vec<String>,
    /// Whether it was applied.
    pub applied: bool,
}

/// One changed line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RenamedLineView {
    /// The file.
    pub file: String,
    /// 1-based line.
    pub line: u32,
    /// Before.
    pub before: String,
    /// After.
    pub after: String,
}

/// Rename `from` (and everything below it) to `to` in every time file and
/// the accounts file; with `apply`, write it, else only preview.
///
/// # Errors
/// [`TimeOpError`].
pub fn rename(
    books: &TimeBooks,
    book: Option<&TimeBook>,
    from: &str,
    to: &str,
    apply: bool,
) -> Result<Written<RenamePreview>, TimeOpError> {
    check_account(from)?;
    check_account(to)?;
    let accounts_path: VaultPath = books.paths.accounts.clone();
    let preview_of =
        |book: &TimeBook, files: &mut TimeFiles| -> Result<RenamePreview, TimeOpError> {
            let accounts_text = if files.exists(&accounts_path)? {
                Some(files.text(&accounts_path)?.to_string())
            } else {
                None
            };
            let plan = rename_plan(
                book,
                accounts_text.as_deref().map(|text| (&accounts_path, text)),
                from,
                to,
            )?;
            if apply {
                plan.apply_to(files)?;
            }
            Ok(RenamePreview {
                from: from.to_string(),
                to: to.to_string(),
                lines: plan
                    .preview
                    .iter()
                    .map(|(file, line)| RenamedLineView {
                        file: books.paths.relative(file),
                        line: line.line,
                        before: line.before.clone(),
                        after: line.after.clone(),
                    })
                    .collect(),
                files: plan
                    .files
                    .iter()
                    .map(|(file, _)| books.paths.relative(file))
                    .collect(),
                applied: apply,
            })
        };
    books.write(book, preview_of)
}
