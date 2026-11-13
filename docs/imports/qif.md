---
title: Importing a QIF file
description: Bring a whole set of books from Banktivity, Quicken, Moneydance or GnuCash into your hledger journal through a QIF or QMTF export.
order: 50
tags: [imports, qif, banktivity, quicken]
---

# Importing a QIF file

QIF (Quicken Interchange Format) is the one export that carries a whole set of personal-finance books: every account, the category list, splits, transfers between accounts, cleared status and classes. Banktivity, Quicken for Windows, Moneydance and GnuCash all write it. Quicken for Mac writes QMTF, a variant of it, which Ledgeline reads the same way.

Ledgeline reads the file itself and writes double-entry transactions straight into your journal, through the same panel as a [QuickBooks import](quickbooks.md). No rules file or CSV is involved.

> [!NOTE]
> **Exporting QIF**
>
> - **Banktivity:** File → Export → QIF. Select every account and the whole date range. Banktivity writes one file per currency; import each one.
> - **Quicken for Windows:** File → File Export → QIF File. Choose **All Accounts** and include transactions, the account list and the category list.
> - **Quicken for Mac:** File → Export → Quicken Transfer Format (`.qmtf`).
> - **Moneydance** and **GnuCash:** File → Export → QIF.

## Import the file

Open **Imports → New Transactions** and drop the `.qif` or `.qmtf` file, or press **Choose file…**. Ledgeline recognises QIF by its extension or by its first line (`!Type:…`, `!Account` or `!Option:…`), so a file renamed to `.txt` still works.

The panel shows how many transactions and postings it read and a sample of them. QIF dates have no declared order, so Ledgeline reads every date in the file and picks month/day or day/month, whichever fits all of them. If both fit, because no day in the file is above 12, it warns you and names the order it chose. Check a few sample dates. If they are wrong, export a wider date range.

Two-digit years are read as 1969–2068, and Quicken's `1/ 5'98` style is understood.

## Map the accounts and categories

Every name the file uses appears in the **Map QIF accounts** table until your journal maps it. Each row starts with a suggestion: an existing account with the same last name segment if your journal has exactly one, otherwise a new name from the file's own account type, such as `assets:bank:Checking` for a bank register, `liabilities:credit cards:Visa` for a credit card, or `expenses:Groceries` for a spending category. Change any you disagree with, clear a row to leave it for later, and press **Map accounts**. Each mapping is saved as a plain `alias` line, exactly as for [QuickBooks](quickbooks.md#map-the-quickbooks-accounts), and you can review it later in [Settings → Account Aliases](../settings/account-aliases.md).

Declare the accounts you want first in [Settings → Accounts](../settings/accounts.md), with their types, so the reports classify them.

The names you will see:

| In the file | Name in the table |
| --- | --- |
| An account (`!Account`, or the register a transaction is in) | Its name, such as `Checking` |
| A transfer, `L[Savings]` | The other account's name, `Savings`, so one mapping covers the account and every transfer into it |
| A category, `LFood:Groceries` | The category path, `Food:Groceries` |
| A category spelled exactly like an account | The name with ` (category)` added, so the two can be mapped apart |
| No category | `Uncategorized` |
| An opening balance | `Equity:Opening Balances` |
| A security held in an investment account | `Brokerage:ACME`, the account and the security |
| Investment income | `Investment Income:Dividends`, `:Interest`, `:Capital Gains` or `:Miscellaneous` |

A mapping on a parent covers its children, by hledger's own alias rule: mapping `Food` to `expenses:food` turns `Food:Groceries` into `expenses:food:Groceries`. Map the child separately if you want a different name.

A file exported from a single account, with no account list, takes its account name from its `Opening Balance` line, or failing that from the file name.

**Import** stays disabled until every name is mapped. [Review Unknowns and Review All](quickbooks.md#review-before-importing) work as they do for QuickBooks.

## What gets written

Each QIF transaction becomes one hledger transaction:

- the payee as its description, or the memo when there is no payee;
- the check number (`N`) as its code, `(1041)`;
- a posting to the account it was in, carrying the transaction's memo;
- one posting per split line, each with its own memo, or one posting to the category or transfer account;
- the class (`LFood/Household`) as a `class:` tag on that posting;
- an `id:` tag, described under [Importing again](#importing-again);
- amounts in your journal's default commodity, as for QuickBooks.

```journal
2026-01-05 * (1041) Harbor Market  ; id: qif-3f1c0a9d2b7e4410
    assets:bank:checking          $-142.37  ; Weekly shop and a hardware run
    expenses:food:Groceries         $96.12  ; Produce and pantry
    expenses:household:Supplies     $31.25  ; Light bulbs
    expenses:food:Snacks            $15.00
```

If a transaction's split lines don't add up to its total, the difference goes to `Uncategorized` with a memo saying so, rather than the file being refused.

### Cleared status

`C*` and `Cc` (cleared) are written as hledger's `*`. So are `CX` and `CR` (reconciled), because hledger has no separate reconciled state. `C!` is written as pending, `!`.

### Transfers written twice

A transfer between two accounts that are both in the file appears in both registers; Banktivity always writes it that way. Ledgeline keeps one copy. A plain transfer is dropped when the other account's register has a transaction on the same date moving the same amount back to it: the other copy of the transfer, or a split line `S[Account]`. When both copies are plain transfers, the one earlier in the file is kept. Two identical transfers on the same day stay two, because each copy is matched once.

A transfer whose two sides have different dates in the file is not recognised as one, and both are written.

### Opening balances

Quicken and Banktivity start each account with an `Opening Balance` transaction whose transfer names the account itself (`L[Checking]` in the Checking register). Ledgeline writes its other side to `Equity:Opening Balances`, which you map like any other name.

### Investments

QIF investment records are imported as the money they move, not as share lots. Each one is valued at its total:

| Action | Written as |
| --- | --- |
| `Buy` | Cash out of the account, into `Account:Security` |
| `Sell` | Back out of `Account:Security` into cash, at the sale proceeds; no cost basis or gain is worked out |
| `Div`, `IntInc`, `CGLong`, `CGShort`, `MiscInc` | Cash in, from `Investment Income:…` |
| `ReinvDiv`, `ReinvInt`, `ReinvLg`, `ReinvSh` | Into `Account:Security`, from `Investment Income:…` |
| `ShrsIn`, `ShrsOut` | Into or out of `Account:Security`, against `Equity:Opening Balances` |
| `XIn`, `XOut`, `Cash` | Cash in or out, against the `L` account |
| `MargInt`, `MiscExp` | Cash out, to the `L` category or `Investment Expenses` |

An action ending in `X` (`BuyX`, `SellX`, `DivX` …) moves the cash through the `L` account instead of the investment account's own cash. The shares, price and commission are kept in the memo, as in `40 ACME Index Fund @ 50.00, commission 4.95`. An action that moves no money, such as `StkSplit`, or one Ledgeline does not know, is written as a zero-amount transaction with the action in its memo, so nothing in the file disappears unseen.

If you track holdings by share count, edit those transactions afterwards to use the security as a commodity.

## Importing again

QIF has no transaction ids, so Ledgeline makes one for each transaction from its date, account, amount and payee, plus a count that tells identical transactions on the same day apart. The id is written as an `id:` tag starting `qif-`. Importing the same file again, or a later export that overlaps it, skips the transactions already in your journal. The comparison works exactly as described for [QuickBooks](quickbooks.md#importing-again), including **Update transactions that changed in the source app** (`--update-changed`).

A transfer's id ignores which side it was read from, so importing one account's file and then the other account's file doesn't write the transfer twice.

If you change a transaction's date, amount or payee in the source app and export again, it gets a new id and is written again as a new transaction. Delete the old copy by hand.

## What QIF can't carry

- **Currencies.** QIF has no currency field. Every amount is written in your journal's default commodity. For a second currency, import its file into a journal whose default is that currency, or edit the amounts afterwards.
- **Budgets, scheduled transactions, attachments and reconciliation history.** Banktivity and Quicken don't export them.
- **Share lots and prices.** See [Investments](#investments).
- **Memorized transactions, securities and price lists** (`!Type:Memorized`, `!Type:Security`, `!Type:Prices`) are skipped.

## From a terminal

```sh
ledgeline import -i Banktivity.qif -j main.journal
```

The file is recognised as QIF from its name or contents. As with QuickBooks, `-o`, `-r` and the balance options are refused, every name has to be mapped first, and `--dry-run` and `--update-changed` work.

The design notes are in [plan 39](https://github.com/zmre/ledgeline/blob/main/plans/39-onboarding-offboarding.md).
