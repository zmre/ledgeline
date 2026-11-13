---
title: Importing from other apps
description: Bring a whole book over from Xero, GnuCash, hledger CSV, YNAB, Monarch or Copilot.
order: 45
tags: [imports, xero, gnucash, ynab, monarch, copilot, migration]
---

# Importing from other apps

Ledgeline can read the transaction exports of several accounting and budgeting apps and turn them into hledger transactions. These imports work like the [QuickBooks import](quickbooks.md), not like a bank statement:

- Every account and category in the export appears in a mapping step, where you choose the journal account each one becomes. Ledgeline remembers your choices as `alias` lines.
- Each transaction gets an `id:` tag. Importing a later, overlapping export skips the transactions that are already in your journal.
- Nothing is written if any part of the file can't be read. The error names the row to look at.

Ledgeline recognises these exports from their column headings, whatever the file is called. A CSV that doesn't match one of them goes through the usual rules-file flow.

There are two kinds of export:

- **One row per posting** (Xero, GnuCash, hledger). A transaction with several accounts spans several rows that share a transaction number. Ledgeline groups those rows back into one transaction, so splits survive intact.
- **One row per transaction** (YNAB, Monarch, Copilot). Each row has an account and a category. Ledgeline writes a two-posting transaction: one posting to the account, one to the category. You map each category to an expense, income or other account. Transfers between your own accounts appear in both accounts' rows, and Ledgeline joins the two halves into one transaction.

Every export must hold a single currency. If yours holds more than one, export each currency separately.

## Xero

Ledgeline reads the **Journal Report** and the **General Ledger Detail** report.

1. In Xero, go to **Accounting → Reports** and open **Journal Report** or **General Ledger Detail**.
2. Choose the date range. Make sure the **Journal ID** or **Journal Number** column is shown. Ledgeline needs it to group lines into journals.
3. Export the report and save it as CSV.

What comes through: every journal with its date, its lines (account code and name, description, debit, credit), its reference, its source type (Spend Money, Receivable Invoice, Manual Journal and so on) and, in General Ledger Detail, its contact. Accounts appear in the mapping step as code plus name, such as `200 Sales`. The `id:` tag is `xero-` plus the journal number.

When you edit a transaction, Xero records a reversing journal and a new one. Both are imported, so the totals are right but the edit history is visible. Report heading and total rows are skipped.

What is lost: tax rates (the tax amounts arrive as their own lines to your tax account), tracking categories and attachments.

## GnuCash

1. In GnuCash, choose **File → Export → Export Transactions to CSV**.
2. Keep the default settings. Either layout works, simple or multi-line.
3. Select all accounts and the date range, then save.

What comes through: each transaction's date, number, description and splits, with each split's full account name (`Expenses:Groceries`), memo and value. A transaction with a split marked reconciled or cleared is imported as cleared (`*`). Voided transactions are skipped. The `id:` tag is `gnucash-` plus GnuCash's transaction ID, which stays the same between exports.

What is lost: transaction notes, split actions, reconcile dates, prices and scheduled transactions. Ledgeline matches column headings in English, so an export from GnuCash set to another language isn't recognised.

## hledger CSV

hledger can write a journal as one CSV row per posting:

```
hledger -f main.journal print -O csv > book.csv
```

Ledgeline reads that file back, including from hledger versions that put `debit` before `credit`. It keeps the date, status, code, description, accounts, amounts and posting comments.

A transaction that already has an `id:` tag keeps it, so re-importing a file made from your own journal skips everything that is already there. Other transactions get a generated `id:` built from the date, description, accounts and amounts. hledger's `txnidx` column only numbers rows within one report, so Ledgeline doesn't use it as an id.

What is lost: secondary dates, transaction comments (apart from `id:`), per-posting status marks, and prices. A journal with more than one commodity is refused. Convert it to one with `print -B` or `print -X`, or export each commodity separately.

## YNAB

1. In YNAB, open the plan menu and choose **Export Plan**.
2. Unzip the download and import the file whose name ends in **Register.csv**. The other file is the budget, which isn't imported.

Each row's **Account** is the register account. Its **Category Group/Category** (`Everyday Expenses: Groceries`) is the other side. **Inflow** adds to the account and **Outflow** takes from it. Rows marked Cleared or Reconciled are imported as cleared. A starting-balance row with no category maps to a category called `Starting Balance`, which is usually your opening-balances equity account.

Split transactions arrive as consecutive rows with memos starting `Split (1/3)`, `Split (2/3)` and so on. They become one transaction with a posting for each part, and the `Split (n/N)` prefix is removed from each memo.

A transfer shows as a payee such as `Transfer : Savings` in both accounts. Ledgeline keeps one copy, posted between the two accounts. If the other account isn't in the export, the transfer posts to an account with that name, and you map it like any other.

What is lost: flags, budget amounts and goals.

## Monarch

1. In Monarch, open **Transactions**.
2. Download your transactions as CSV, from the menu on the Transactions page or from **Settings → Data**.

**Amount** is negative for money leaving the account and positive for money arriving. **Merchant** becomes the payee, using the original statement text when there is no merchant. **Notes** becomes the memo, and **Category** is the other side.

Rows in the **Transfer** or **Credit Card Payment** category are paired: a row in one account is matched with a row for the opposite amount in another account, dated up to four days apart. Each pair becomes one transfer between the two accounts, dated on the side the money left. A row with no match stays against its category, which you can map to a clearing account.

What is lost: tags, owner, the original statement text when there is a merchant, and balance history.

## Copilot

1. In Copilot, export your transactions to CSV from the app's settings.

Ledgeline reads **date**, **name**, **amount**, **category**, **account**, **account mask**, **type**, **excluded** and **notes**. Accounts appear as the account name plus its mask (`Chase Checking 1234`), so two cards with the same name stay separate. Rows marked **excluded** are skipped, and the import says how many.

Copilot exports spending as positive amounts. Ledgeline checks this against the rows typed `income`: if those are mostly positive, it reads positive amounts as money in instead. Rows whose type contains `transfer` are paired across accounts in the same way as Monarch's. Pending rows are imported as pending (`!`).

What is lost: tags, parent categories, recurring markers and balance history.

## Why an import is refused

| Message | What to do |
|---|---|
| The transaction starting at row N does not balance | A row is missing from that transaction, often because the export was filtered by account. Export every account. |
| Row N has no transaction key | For Xero, add the Journal ID or Journal Number column. For GnuCash, the file starts partway through a transaction. Export it again. |
| The export holds more than one commodity | Export each currency separately, or convert to one currency first. |
| No date format reads every date | Some dates are unreadable or in mixed formats. Check the rows near the end of the file. |
| Row N has an unreadable amount | Fix the cell, or export again without spreadsheet formatting. |
