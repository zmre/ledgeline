//! Deposits (ARCHITECTURE §4.4): a deposit invoice books its amount to the
//! deposit account (I11), and the final invoice applies it with a fixed
//! adjustment row against that account, which debits the liability.
//!
//! [`InvoiceBooks::open_deposits`] lists what a customer still has on the
//! deposit account: every credit there tagged with the customer (a deposit
//! invoice booked, or a cash-books deposit applied), less every application.
//! Applications are counted from the documents that make them, so an
//! invoice not booked yet (a draft, or any cash-books invoice before its
//! payment) still uses its deposit up:
//!
//! - an invoice with a file is counted from its bill table (rows booked to
//!   the deposit account with a negative amount), never from the journal;
//! - a void invoice counts for nothing, its transactions included;
//! - a debit in the journal from anything else (a hand-written transfer)
//!   counts as it is.
//!
//! Each application uses up the deposit its row names (`Less deposit
//! (Invoice 104)` names 104, as a whole word) first, then the oldest.
//!
//! [`InvoiceBooks::apply_deposit`] writes that row into a draft, above its
//! final total, and recomputes the table.

use super::spec::deposit_account;
use super::{BooksError, Document, InvoiceBooks};
use crate::doc::Status;
use crate::draft;
use crate::index::DocumentIndex;
use crate::table::books::BooksEntry;
use crate::table::columns::Role;
use crate::table::gfm::{self, Grid, Table, escape_cell};
use crate::table::{BillTable, RowKind};
use ledgeline_core::Dec;
use ledgeline_core::accrual::{CUSTOMER_TAG, INVOICE_TAG, VOID_TAG, tag_value};
use ledgeline_core::vault::VaultPath;
use serde::Serialize;
use std::collections::BTreeMap;

/// One deposit with something left on it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OpenDeposit {
    /// The deposit invoice's number (`invoice:` on its transaction), when it
    /// has one.
    pub number: Option<String>,
    /// Its file, when it has one.
    pub path: Option<VaultPath>,
    /// The transaction's date.
    pub date: String,
    /// The transaction's description.
    pub description: String,
    /// What it put on the deposit account.
    #[serde(serialize_with = "super::dec_text")]
    pub credited: Dec,
    /// What is left of it.
    #[serde(serialize_with = "super::dec_text")]
    pub remaining: Dec,
}

/// A customer's deposits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OpenDeposits {
    /// The customer (`customer:` tag value).
    pub customer: String,
    /// The deposit account.
    pub account: String,
    /// The commodity.
    pub commodity: String,
    /// Every credit less every application. Negative when more was applied
    /// than deposited (a problem to look at).
    #[serde(serialize_with = "super::dec_text")]
    pub balance: Dec,
    /// The deposits with something left, oldest first.
    pub deposits: Vec<OpenDeposit>,
}

/// **Apply deposit**: write a row applying `deposit` to draft `number`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyDeposit {
    /// The draft.
    pub number: String,
    /// The deposit invoice's number (an [`OpenDeposit::number`]).
    pub deposit: String,
    /// How much; all that is left of the deposit when `None`, capped at the
    /// draft's total.
    pub amount: Option<Dec>,
}

/// What [`InvoiceBooks::apply_deposit`] wrote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DepositApplied {
    /// The draft.
    pub number: String,
    /// The file.
    pub path: VaultPath,
    /// The deposit.
    pub deposit: String,
    /// The amount applied (positive).
    #[serde(serialize_with = "super::dec_text")]
    pub amount: Dec,
    /// The row's description.
    pub row: String,
    /// The draft's new total.
    #[serde(serialize_with = "super::dec_text")]
    pub total: Dec,
}

/// Whether `text` holds `word` with no letter or digit on either side.
fn names(text: &str, word: &str) -> bool {
    !word.is_empty()
        && text.match_indices(word).any(|(at, _)| {
            let before = text[..at].chars().next_back();
            let after = text[at + word.len()..].chars().next();
            !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
        })
}

/// One application to allocate.
struct Use {
    date: String,
    description: String,
    amount: Dec,
}

impl InvoiceBooks {
    /// The deposits the customer of invoice `number` has open on the deposit
    /// account, for applying to it (its own rows aside).
    ///
    /// # Errors
    /// As [`InvoiceBooks::document`] and [`InvoiceBooks::customer`].
    pub fn open_deposits(
        &self,
        index: &DocumentIndex,
        number: &str,
    ) -> Result<OpenDeposits, BooksError> {
        let document = self.document(index, number)?;
        let customer = self.customer(index, &document)?;
        let loaded = self.load()?;
        let journal = loaded.journal();
        let account = deposit_account(index.settings(), journal);
        let settings = index.settings();
        let money = index.money();
        let commodity = document.invoice.bill_table().ok().flatten().map_or_else(
            || money.style(None).commodity().to_string(),
            |t| money.for_table(t).commodity().to_string(),
        );

        // Every document by number, and the applications this customer's
        // other documents make.
        let mut statuses: BTreeMap<String, Option<Status>> = BTreeMap::new();
        let mut paths: BTreeMap<String, VaultPath> = BTreeMap::new();
        let mut uses: Vec<Use> = Vec::new();
        for doc in index.docs() {
            let Some(n) = doc.number() else { continue };
            statuses.insert(n.to_string(), doc.header.status);
            paths.insert(n.to_string(), doc.path.clone());
            let mentions_account = doc
                .summary
                .as_ref()
                .is_some_and(|s| s.accounts.contains(&account));
            if n == number || doc.header.status == Some(Status::Void) || !mentions_account {
                continue;
            }
            let Ok(other) = self.document(index, n) else {
                continue;
            };
            if self
                .customer(index, &other)
                .map_or(true, |c| c.ledger_customer != customer.ledger_customer)
            {
                continue;
            }
            let Ok(Some(evaluation)) = other.invoice.evaluate(&settings.words, money) else {
                continue;
            };
            for row in &evaluation.rows {
                let books = row.books.as_ref().map(BooksEntry::account);
                if let (Some(books), Some(amount)) = (books, row.amount)
                    && books.as_str() == account
                    && amount.mantissa < 0
                    && row.kind != RowKind::Total
                {
                    uses.push(Use {
                        date: other.invoice.header().date.clone().unwrap_or_default(),
                        description: row.at.description.clone(),
                        amount: amount.neg().map_err(|_| BooksError::Overflow)?,
                    });
                }
            }
        }

        // The journal: credits, and debits nothing above counts.
        let mut deposits: Vec<OpenDeposit> = Vec::new();
        for txn in &journal.transactions {
            let tagged = tag_value(&txn.tags, INVOICE_TAG).map(str::to_string);
            let status = tagged.as_ref().and_then(|n| statuses.get(n).copied());
            if status == Some(Some(Status::Void)) {
                continue;
            }
            for posting in &txn.postings {
                if posting.account.0 != account
                    || posting.generated.is_some()
                    || tag_value(&posting.tags, CUSTOMER_TAG)
                        != Some(customer.ledger_customer.as_str())
                {
                    continue;
                }
                let Some(amount) = posting.amounts.iter().find(|a| a.commodity.0 == commodity)
                else {
                    continue;
                };
                let q = amount.quantity;
                if q.mantissa < 0 {
                    let credited = q.neg().map_err(|_| BooksError::Overflow)?;
                    deposits.push(OpenDeposit {
                        number: tagged.clone(),
                        path: tagged.as_ref().and_then(|n| paths.get(n).cloned()),
                        date: txn.date.clone(),
                        description: txn.description.clone(),
                        credited,
                        remaining: credited,
                    });
                } else if q.mantissa > 0 && status.is_none() {
                    uses.push(Use {
                        date: txn.date.clone(),
                        description: txn.description.clone(),
                        amount: q,
                    });
                } else if q.mantissa > 0 && tag_value(&txn.tags, VOID_TAG).is_some() {
                    // The reversal of a credit whose file is gone.
                    uses.push(Use {
                        date: txn.date.clone(),
                        description: txn.description.clone(),
                        amount: q,
                    });
                }
            }
        }
        deposits.sort_by(|a, b| a.date.cmp(&b.date));
        uses.sort_by(|a, b| a.date.cmp(&b.date));

        let mut balance = Dec::zero();
        for deposit in &deposits {
            balance = balance
                .add(deposit.credited)
                .map_err(|_| BooksError::Overflow)?;
        }
        for used in &uses {
            balance = balance.sub(used.amount).map_err(|_| BooksError::Overflow)?;
            let mut left = used.amount;
            let named = deposits.iter().position(|d| {
                d.remaining.mantissa > 0
                    && d.number
                        .as_deref()
                        .is_some_and(|n| names(&used.description, n))
            });
            let order: Vec<usize> = named.into_iter().chain(0..deposits.len()).collect();
            for at in order {
                if left.mantissa <= 0 {
                    break;
                }
                let deposit = &mut deposits[at];
                if deposit.remaining.mantissa <= 0 {
                    continue;
                }
                let take = if deposit.remaining < left {
                    deposit.remaining
                } else {
                    left
                };
                deposit.remaining = deposit
                    .remaining
                    .sub(take)
                    .map_err(|_| BooksError::Overflow)?;
                left = left.sub(take).map_err(|_| BooksError::Overflow)?;
            }
        }
        deposits.retain(|d| d.remaining.mantissa > 0);
        Ok(OpenDeposits {
            customer: customer.ledger_customer,
            account,
            commodity,
            balance,
            deposits,
        })
    }

    /// **Apply deposit** (§4.4): write a `Less deposit (Invoice N)` row
    /// booked to the deposit account, for the amount asked (else all that is
    /// left, capped at the draft's total), above the draft's final total, and
    /// recompute the table. Drafts only: a sent invoice is edited in the
    /// editor and re-booked.
    ///
    /// # Errors
    /// [`BooksError::Status`] for an invoice that isn't a draft;
    /// [`BooksError::Refused`] for a deposit that isn't open, an amount over
    /// what is left or over the total, or a bill table without a books
    /// column; [`BooksError::Draft`] when the file changed since it was read.
    pub fn apply_deposit(
        &self,
        index: &mut DocumentIndex,
        request: &ApplyDeposit,
    ) -> Result<DepositApplied, BooksError> {
        let document = self.document(index, &request.number)?;
        if document.invoice.header().status != Some(Status::Draft) {
            return Err(BooksError::Status {
                number: request.number.clone(),
                status: document.status_text(),
                hint: "a deposit is applied to a draft (edit a sent invoice, then re-book it)"
                    .into(),
            });
        }
        let open = self.open_deposits(index, &request.number)?;
        let deposit = open
            .deposits
            .iter()
            .find(|d| d.number.as_deref() == Some(request.deposit.as_str()))
            .ok_or_else(|| {
                BooksError::Refused(format!(
                    "{} has no open deposit numbered {}",
                    open.customer, request.deposit
                ))
            })?;
        let settings = index.settings();
        let money = index.money();
        let table = document
            .invoice
            .bill_table()
            .ok()
            .flatten()
            .cloned()
            .ok_or_else(|| BooksError::Refused(format!("{} has no bill table", document.path)))?;
        let style = money.for_table(&table);
        let evaluation = table
            .evaluate(&settings.words, &style)
            .map_err(|e| BooksError::Refused(format!("{}: {e}", document.path)))?;
        let amount = match request.amount {
            Some(amount) if amount > deposit.remaining || amount.mantissa <= 0 => {
                return Err(BooksError::Refused(format!(
                    "deposit {} has {} left; apply more than zero and no more than that",
                    request.deposit,
                    style.format(deposit.remaining)
                )));
            }
            Some(amount) => amount,
            None if deposit.remaining < evaluation.total => deposit.remaining,
            None => evaluation.total,
        };
        if amount > evaluation.total {
            return Err(BooksError::Refused(format!(
                "applying {} would take invoice {} below zero ({})",
                style.format(amount),
                request.number,
                style.format(evaluation.total)
            )));
        }
        let columns = table.columns();
        let books_col = columns.find(Role::Books).ok_or_else(|| {
            BooksError::Refused(format!(
                "{}'s bill table has no books column (an italic *Account* header, last) for the \
                 deposit row's account",
                document.path
            ))
        })?;
        let description = format!(
            "{} (Invoice {})",
            settings.words.deposit.first(),
            request.deposit
        );
        let negative = amount.neg().map_err(|_| BooksError::Overflow)?;
        let width = table.table().grid.header.len();
        let row: Vec<String> = (0..width)
            .map(|col| match columns.role(col) {
                Role::Description => escape_cell(&description),
                Role::Amount => escape_cell(&style.format(negative)),
                _ if col == books_col => escape_cell(&open.account),
                _ => String::new(),
            })
            .collect();
        let at = evaluation
            .final_total
            .unwrap_or(table.table().grid.rows.len());
        let mut rows = table.table().grid.rows.clone();
        rows.insert(at, row);
        let grown = Table {
            grid: Grid {
                rows,
                ..table.table().grid.clone()
            },
            ..table.table().clone()
        };
        let grown = BillTable::from_table(grown, &settings.words)
            .map_err(|e| BooksError::Refused(e.to_string()))?
            .ok_or_else(|| BooksError::Refused("the bill table was lost".into()))?;
        let recalc = grown
            .recompute(&settings.words, &style)
            .map_err(|e| BooksError::Refused(e.to_string()))?;
        let edits = recalc
            .table
            .edits(document.invoice.source())
            .map_err(|e| BooksError::Refused(e.to_string()))?;
        let text = gfm::apply_edits(document.invoice.source(), &edits);
        let path = document.path.clone();
        save(self, index, &document, &text)?;
        Ok(DepositApplied {
            number: request.number.clone(),
            path,
            deposit: request.deposit.clone(),
            amount,
            row: description,
            total: recalc.evaluation.total,
        })
    }
}

fn save(
    books: &InvoiceBooks,
    index: &mut DocumentIndex,
    document: &Document,
    text: &str,
) -> Result<(), BooksError> {
    draft::save(
        index,
        books.vault(),
        &document.path,
        text,
        &document.revision,
    )?;
    Ok(())
}
