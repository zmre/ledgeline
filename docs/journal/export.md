---
title: Exporting the journal
description: "Download the filtered journal as an Excel workbook laid out like QuickBooks Online's Journal report."
order: 40
tags: [journal, export, quickbooks]
---

# Exporting the journal

The **Export .xlsx** button in the journal's totals footer saves the transactions you are looking at as an Excel workbook. The layout copies QuickBooks Online's Journal report, so an accountant can read it without learning hledger.

## What gets exported

The export contains exactly what the table shows. The date range, account and search [filters](filtering.md) all apply. To export everything, choose **All time** and clear the other filters first. The button is hidden when no transactions match.

Each transaction is exported whole. An account filter picks which transactions go in, but every posting of a chosen transaction is written, so each group still balances.

Transactions are written oldest first, the way a journal reads. The table shows them newest first.

## Saving the file

The suggested file name follows the date range: `journal-2026-01-01-to-2026-03-31.xlsx` for a range, or `journal-all-dates.xlsx` for all time. In the desktop app a save dialog asks where to put it. If you run Ledgeline in a browser, the browser's download settings decide.

## The layout

The first three rows are a title band: the journal's name, the word "Journal", and the date range. Row 5 holds the column headers, and it stays frozen while you scroll.

Each transaction is a group of rows:

1. A marker row with the transaction's number alone in the first column.
2. One row per posting.
3. A `Total for` row with the group's debit and credit totals.

A `TOTAL` row with the grand totals closes the sheet, followed by a line saying when Ledgeline generated it.

<!-- screenshot: an exported workbook open in a spreadsheet app, showing one transaction group with its Total for row -->

### Columns

| Column | Contents |
| --- | --- |
| (unlabelled) | The group markers: the transaction number, `Total for` and `TOTAL`. |
| Transaction date | The posting's date, which is the transaction date unless the posting has its own. |
| Transaction type | Always `Journal Entry`. |
| Num | The transaction's code, if it has one. |
| Name | The transaction description. |
| Description | The posting's comment, or the transaction's comment when the posting has none. |
| Account Name | The hledger account. Virtual postings keep their `(account)` or `[account]` form. |
| Debit | Positive amounts. |
| Credit | Negative amounts, shown without the minus sign. |
| Quantity | Only when some posting has a cost. See below. |
| Item class, Customer full name, Vendor | Only when some exported transaction has a `class:`, `customer:` or `vendor:` tag. |

Debits and credits follow hledger's signs, so you don't need account types for them to come out right. A posting with more than one commodity gets one row per commodity.

### Costs and other commodities

A posting with a cost is written at its cost. Buying `10 AAPL @ $150.00` debits the brokerage `$1,500.00` and credits the bank `$1,500.00`, so the group balances the way an accountant expects. The Quantity column keeps the original `10 AAPL @ $150.00` text.

Amounts are rounded to the precision each commodity displays with, and totals are summed from those rounded amounts. A `Total for` row with a single commodity is a live `SUM` formula over the rows above it, as in QuickBooks, so it re-totals if someone edits a cell. A total that mixes commodities is written as text, such as `1,500.00 $, 10 AAPL`.

### Tags as QuickBooks columns

The three dimension columns come from hledger tags, read from the posting first and then from the transaction. These are the same tags the [QuickBooks import](../imports/quickbooks.md) writes, so a journal imported from QuickBooks exports with its classes, customers and vendors in place.

## Related pages

- [Filtering the journal](filtering.md)
- [Importing from QuickBooks](../imports/quickbooks.md)
- [Reports](../reports/index.md), each of which has its own export
