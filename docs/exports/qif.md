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
| Asset holding securities | `Invst` account (see below) |
| Cash (`C`) | `Bank` account |
| Other asset (`A`) | `Oth A` account |
| Liability (`L`) | `CCard` or `Oth L` account |
| Revenue (`R`, `G`) | income category |
| Expense (`X`) | expense category |
| Equity (`E`, `V`) | an Opening Balance record, or the "Ledgeline Adjustments" account |

A liability becomes a credit card (`CCard`) when, in more than half of the transactions that touch it, it is the only account on the money side and the other side is an expense. Card purchases look like that. Loan payments come from another account, so a loan stays `Oth L`. Transactions with an equity posting, such as the opening balance, don't count either way.

To choose the QIF type yourself, add a `qif:` tag to the account declaration. It accepts `Bank`, `Cash`, `CCard`, `Invst`, `Oth A` and `Oth L`:

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

## Investments

An account holding securities becomes a QIF investment (`Invst`) account. These are the accounts the Holdings page lists on its Stocks tab: an asset account holding a commodity the journal doesn't use as money. An account tagged `holdings: other` or `holdings: none` is left out, and so is one with a `qif:` tag naming another type. Like every other choice here, this goes by account type, not name.

### Which accounts form one investment account

A brokerage is often booked as one account per security plus a cash account under one parent:

```journal
account assets:broker:taxable:cash    ; type: C
account assets:broker:taxable:aapl    ; type: A
account assets:broker:taxable:vti     ; type: A
```

Ledgeline exports these as one investment account, `assets:broker:taxable`, and its cash is that account's cash. A parent is used this way when everything posted under it is a security account or a plain money account, there is at most one money account, and it is not a top-level account such as `assets`. Otherwise each security account is its own investment account, with any money posted to it as its cash. To force a parent to be one investment account, tag it `qif: Invst`.

### Securities and prices

Each file starts with a security list (`!Type:Security`), one entry per security its records use. The symbol (`S`) is the hledger commodity. The name (`N`) is the `name:` tag on the `commodity` directive, else the first `name:` tag on a posting of that commodity, else the symbol. Records name the security (`Y`) by that same name. If two securities share a name, each gets its symbol added, as in `Tech (AAPL)`. The type (`T`) is `Mutual Fund` when the `commodity` directive has `type: mutualfund`, otherwise `Stock`.

A security goes in the file of the currency it was bought or sold in. `P` directives that price it in that currency are written at the end of that file as a price list, one entry per price, in Quicken's format:

```
!Type:Prices
"AAPL",228.00,"9/30/24"
^
```

The date is month/day/two-digit year, without leading zeros.

### How transactions become investment records

| In the journal | QIF action |
| --- | --- |
| Units bought with the account's own cash | `Buy` |
| Units bought with money from another account | `BuyX`, with `L[account]` and the amount in `$` |
| Units sold, money kept in the account | `Sell` |
| Units sold, money sent to another account | `SellX` |
| Income, with a zero-unit posting naming the security (`assets:broker:aapl  0 AAPL`) | `Div`, or `DivX` when paid to another account |
| Units bought with income alone | `ReinvDiv` |
| Units in or out against equity, or with no money at all | `ShrsIn` / `ShrsOut` |
| Units moved from one investment account to another | `ShrsOut` in one, `ShrsIn` in the other |
| Money in or out with no security | `XIn` / `XOut`, or `MiscInc` / `MiscExp` with the category in `L` |

The quantity (`Q`) and price (`I`) are the ones in the journal: `10 AAPL @ $220.00` is `Q10` and `I220.00`. A sale is written at the price on its negative posting. A price is left out when it is a total (`@@`) or when units in one transaction were bought at different prices; the total (`T`) still holds the amount. An expense posting in a buy or sell, such as a broker fee, becomes the commission (`O`). Investment records have no number field, so a transaction code is not carried on them.

A sale with an income posting (a realized gain or loss) was booked at cost, so its `@` price is the cost, not the sale price. Ledgeline writes the cash received as the total, leaves out the price, and drops the gain posting. The app works the gain out from its own lots. `Div` and `ReinvDiv` have no field for the income account, so the app uses its own dividend category.

Income paid into a brokerage's cash account with no security posting, such as the sample journal's dividends, has no security to name. It is written as `MiscInc` with its income account as the category.

A transaction that moves a security but fits none of these is written the old way: the units become the cash they cost. The report counts each one and gives the reason. Examples are two securities in one transaction, or units bought for money with no written cost.

An investment account has no Opening Balance record. Opening cash in one is an `XIn` from Ledgeline Adjustments, and opening shares are `ShrsIn`.

Still not carried: lots and specific-lot identification, cost basis beyond each record's own amount, stock splits, and the prices of anything that isn't a security, such as a house booked as its own commodity.

## Currency exchanges

A cost on a bank account is a currency exchange, so Ledgeline keeps both currencies. The euros go into the euro account in the EUR file, and the dollars leave the checking account in the dollar file. Each side balances against Ledgeline Adjustments. Two commodities in one transaction with no written cost are handled the same way. Exchange rates (`P` directives between two currencies) have no QIF form and are counted as not exported.

## What the report counts

The report has two parts.

Not exported. QIF has no place for these:

- balance assertions
- market prices (`P` directives) of anything that is not a security, and security prices in a currency with no file for that security
- exchange rates (`P` directives between currencies)
- realized gain postings on sales
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
- security transactions written as investment records, and security prices written to the price list
- dividends written as `Div` or `ReinvDiv`, which name no income account
- security transactions written as cash at cost, each with its reason
- other postings with a cost written as cash
- currency conversions
- posting comments moved into the transaction memo
- renamed accounts, untyped accounts, and the liabilities exported as credit cards

A line appears only when its count is above zero.
