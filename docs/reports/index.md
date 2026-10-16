---
title: Reports
description: "The Reports page: its six tabs, the shared controls, links you can share, and XLSX export."
order: 30
tags: [reports, export]
---

# Reports

Reports holds six tabs. Ledgeline computes every figure in its Rust engine with exact decimal math, so the totals match what hledger prints for the same journal.

| Tab | Shortcut | What it answers | Page |
| --- | --- | --- | --- |
| Insights | <kbd>1</kbd> | How did this period compare with the one before it? | [Insights](insights.md) |
| Balance Sheet | <kbd>2</kbd> | What do I own and owe on a given date? | [The balance sheet](balance-sheet.md) |
| P&L | <kbd>3</kbd> | What came in and went out over a date range? | [The income statement](income-statement.md) |
| Cash Flow | <kbd>4</kbd> | How did my cash accounts change, period by period? | [Cash Flow and Net Worth](cash-flow-and-net-worth.md) |
| Net Worth | <kbd>5</kbd> | How have assets and liabilities moved over time? | [Cash Flow and Net Worth](cash-flow-and-net-worth.md) |
| Subscriptions | <kbd>6</kbd> | Which charges recur every month or year? | [Subscriptions](subscriptions.md) |

Insights is the tab that opens first. The digit shortcuts work only while the Reports page is showing, and <kbd>?</kbd> lists them with the rest of the page's keys. See [Keyboard shortcuts](../journal/keyboard.md).

<!-- screenshot: Reports page with the tab strip, the controls bar and the Export .xlsx button circled -->

## The controls bar

The grey bar under the tabs shows only the controls the current tab reads. Settings carry over when you switch tabs, so a date you set on one tab is still there when you come back.

- **As of** sets the date for the Balance Sheet. The date is inclusive.
- **From** and **To** set the date range for the P&L. Both ends are inclusive.
- **End**, **Interval** and **Periods** set the columns for Cash Flow and Net Worth. End picks the date whose period is the last column, Interval picks monthly, quarterly or yearly periods, and Periods sets how many columns to show (up to 120).
- **Depth** limits how many levels of the account tree Cash Flow and Net Worth show.

The defaults come from today's date. The Balance Sheet starts at today, the P&L at the current calendar year, Cash Flow at the last 12 months, and Net Worth at the last 5 years.

Insights and Subscriptions don't use this bar. Insights has its own comparison picker, and Subscriptions always scans the last 24 months.

The Balance Sheet and P&L have no depth control. They group accounts into lines, and you expand a line to see the accounts inside it. Their pages explain how the grouping works.

## Valuation and missing prices

The Balance Sheet, P&L and Net Worth value holdings in your base currency using the journal's `P` price directives and the prices implied by `@` and `@@` costs. If a commodity has no price, a yellow banner above the report names it ("Some holdings are not valued"), and that commodity is left out of the valued totals.

To fix it, add a `P` directive for the commodity, or fetch prices from the Holdings page. See [Holdings](../holdings/index.md).

## Sharing a report

The URL records the tab and every control the tab uses. Copy the address and anyone with the same journal sees the same report, even after the today-based defaults have moved on.

An old bookmark that points at the Budget tab, which used to live here, forwards to the [Budget](../budget.md) page with its dates and depth carried over.

## Exporting to Excel

The **Export .xlsx** button sits to the right of the tabs on the Balance Sheet, P&L, Cash Flow and Net Worth tabs. Insights and Subscriptions have no export.

The workbook holds the report you are looking at, with a title and a line stating the parameters:

| Tab | Parameters line | File name |
| --- | --- | --- |
| Balance Sheet | `as of 2026-07-08` | `balance-sheet-2026-07-08.xlsx` |
| P&L | `2026-01-01 to 2026-12-31` | `income-statement-2026-01-01-to-2026-12-31.xlsx` |
| Cash Flow | the period span and depth | `cash-flow-<end>.xlsx` |
| Net Worth | the period span and depth | `net-worth-<end>.xlsx` |

The button appears only once the report on screen matches the current controls. That stops a file named for one report from holding another report's figures while a refetch is in flight.

The Balance Sheet and P&L exports use a conventional statement layout meant for sharing. The in-app view rolls lines up and lets you drill in. The export code is in [`web/src/lib/export/xlsx.ts`](https://github.com/zmre/ledgeline/blob/main/web/src/lib/export/xlsx.ts).

## Where the figures come from

Each tab calls an engine endpoint under `/api/reports/` (or `/api/insights` and `/api/subscriptions`). The tab pages describe how each report classifies accounts. Most of them read the declared `type:` on your `account` directives first and fall back to guessing from the account name. [Settings → Accounts](../settings/accounts.md) shows those types and lets you set them.
