---
title: Exporting to QIF
description: "Write the journal as QIF files for Banktivity, Quicken and other personal-finance apps, and read the report of what QIF could not carry."
order: 10
tags: [exports, qif]
---

# Exporting to QIF

QIF (Quicken Interchange Format) is the format most personal-finance apps import. Banktivity, Quicken, Moneydance and GnuCash all read it. Banktivity can also export it, which makes QIF the one format that goes both ways between Ledgeline and Banktivity.

QIF is single-entry. Each transaction belongs to one account's register, and its other postings become categories or transfers. Ledgeline translates your double-entry journal into that shape and prints what it had to drop or change.

## Running the export

In the desktop app, choose **File → Export → QIF…** and pick a folder. If an earlier export is already there, Ledgeline asks before replacing it. When it finishes, a dialog shows the report.

From the command line:

```sh
ledgeline export --qif out/ -j main.journal
```

The report goes to standard output. Ledgeline creates the folder if it is missing and refuses to replace existing files unless you add `--force`.

## One file per currency

QIF has no currency field. Ledgeline writes one file per commodity and names each after the journal and the commodity, for example `main-dollar.qif` and `main-EUR.qif`. The journal's `D` default commodity comes first. Import every file, and tell the target app which currency each one is in. Banktivity does the same when it exports more than one currency.

## How accounts are mapped

Ledgeline decides by each account's type (its `type:` tag, or the type inherited from a parent), never by its name. A chart of accounts in Spanish or German exports the same way as an English one.

| Account type | In QIF |
| --- | --- |
| Cash (`C`) | `Bank` account |
| Other asset (`A`) | `Oth A` account |
| Liability (`L`) | `CCard` or `Oth L` account |
| Revenue (`R`, `G`) | income category |
| Expense (`X`) | expense category |
| Equity (`E`, `V`) | an Opening Balance record, or the "Ledgeline Adjustments" account |

A liability becomes a credit card (`CCard`) when, in more than half of the transactions that touch it, it is the only account on the money side and the other side is an expense. Card purchases look like that. Loan payments come from another account, so a loan stays `Oth L`. Transactions with an equity posting, such as the opening balance, don't count either way.

To choose the QIF type yourself, add a `qif:` tag to the account declaration. It accepts `Bank`, `Cash`, `CCard`, `Oth A` and `Oth L`:

```journal
account assets:wallet   ; type: C, qif: Cash
```

An account with no type at all is exported as an expense category, and the report lists it.

Account and category names are the full hledger names, such as `expenses:food:groceries`. QIF uses `:` for subcategories too, so the hierarchy carries over. QIF reserves `/` and `[ ]`, so Ledgeline replaces them with `-` and `( )` and lists every renamed account.

## How transactions are written

Each transaction is written once, in the register of its first bank-like account in name order. Its other postings become split lines: a category for income and expenses, or `[Account]` for a transfer to another account. A transaction with one other posting gets a plain category line instead of splits.

A transfer appears in one account only. The importing app creates the other side. This avoids the duplicated transfers that Banktivity's own QIF export is known for. If your app does not create the other side, use `--mirror-transfers` on the command line to write each transfer in both accounts, the way Quicken exports.

An equity transaction that sets up an account (an opening balance) becomes a QIF `Opening Balance` record in that account. That is how Quicken-family apps set a starting balance. Ledgeline only does this for an account's first transaction in each file.

Some transactions have no bank-like account, such as moving an amount from one expense to another. Others have an equity posting that is not an opening balance, such as an owner's draw. QIF has nowhere to put these, so Ledgeline writes them through an `Oth A` account called **Ledgeline Adjustments**. Its balance is the net of everything sent there.

The fields are filled like this:

| QIF field | From |
| --- | --- |
| `D` date | the transaction date, as `MM/DD/YYYY` |
| `N` number | the transaction code, such as `(1001)` |
| `P` payee | the description |
| `M` memo | the transaction comment, without its tags |
| `E` split memo | the posting comment, without its tags |
| `/Class` | the `class:` tag on the posting or the transaction |
| `C` status | see below |

Status maps one to one, so a later import can restore it. An unmarked transaction has no `C` line. Pending (`!`) becomes cleared (`C*`), and cleared (`*`) becomes reconciled (`CX`).

## Investments and currency exchanges

QIF investment accounts need actions such as Buy and Sell that hledger postings don't record. Ledgeline does not write them. A posting with a cost (`@` or `@@`) in an investment account is written as the cash it cost, so buying 10 shares at $150 moves $1,500 into the brokerage account. The share count, lots and prices are lost, and the report counts these postings.

A cost on a bank account is a currency exchange, so Ledgeline keeps both currencies. The euros go into the euro account in the EUR file, and the dollars leave the checking account in the dollar file. Each side balances against Ledgeline Adjustments. Two commodities in one transaction with no written cost are handled the same way.

## What the report counts

The report has two parts.

Not exported. QIF has no place for these:

- balance assertions
- market prices (`P` directives)
- periodic transactions, which covers budgets and forecasts
- account tags such as `kpi:` and `cfgroup:`
- transaction and posting tags other than `class:`
- secondary dates and posting dates
- unbalanced virtual postings, written `(account)`

Changed to fit QIF:

- extra files for extra currencies
- auto-posting rules, whose postings are written as ordinary splits
- transactions written through Ledgeline Adjustments
- Opening Balance records
- postings with a cost written as cash
- currency conversions
- posting comments moved into the transaction memo
- renamed accounts, untyped accounts, and the liabilities exported as credit cards

A line appears only when its count is above zero.
