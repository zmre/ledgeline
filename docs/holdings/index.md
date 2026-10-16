---
title: Holdings
description: "The Holdings tab: stocks and other assets, which tab an account lands on, the columns, and the period control."
order: 40
tags: [holdings]
---

# Holdings

The Holdings tab has two sub-tabs:

- **Stocks** shows securities, one row per commodity, with average-cost basis and
  unrealized gain.
- **Other** shows everything else you own that is neither a security nor cash: a
  house, a car, a partnership interest, a receivable. It has one row per holding,
  with its value, its cost, and how much it has changed. A holding may be one
  account or a small subtree of them.

<!-- screenshot: Holdings Stocks tab, full page, with the Period control and the Update prices button circled -->

This section has five pages:

- this one: which tab an account lands on, what the columns mean, and the period control;
- [The holdings and valuation tags](tags.md): moving an account between tabs, rolling several accounts into one holding, and separating cost from value;
- [Prices](prices.md): where valuations come from and the Update prices button;
- [Allocation by category](allocation.md): the Divide by menu on the Stocks pie;
- [Comparing against a benchmark](benchmarks.md): the index-fund lines on the Value over time chart.

Design notes and the reasoning behind each decision live in
[`plans/10-stock-holdings.md`](https://github.com/zmre/ledgeline/blob/main/plans/10-stock-holdings.md) and
[`plans/14-other-holdings.md`](https://github.com/zmre/ledgeline/blob/main/plans/14-other-holdings.md).

## Why two tabs and not one list

They are keyed differently, and no single table can be keyed both ways. A
security is a *commodity*: you hold 19.5 AAPL across three accounts, and the
interesting row is AAPL. A house is an *account*: `assets:property:home` is the
thing, and what it holds is an implementation detail. Averaging cost across lots
makes sense for the first and is meaningless for the second.

## Which tab an account lands on

Without any tag, the rule is mechanical:

| The account…                                              | Tab            |
|-----------------------------------------------------------|----------------|
| holds a non-currency commodity (`AAPL`, `VTI`, `BTC`)      | **Stocks**     |
| is `type:A`, not cash, and holds only currency             | **Other**      |
| is `type:C` (cash)                                         | neither        |
| is a liability, equity, revenue or expense account         | neither        |

"Not cash" is exact: hledger folds `type:C` into Asset for most purposes, but the
Other tab tests for `type:A` *specifically*, or every bank account would appear
on it.

"Currency" means a symbol like `$`, `€` or `£`, or a three-letter ISO code
that your journal uses as money: one that is posted to a cash, liability,
income or expense account, or that a cost is written in (`@ 5 CHF`). Some codes
are also tickers. `BND` is both the Brunei dollar and Vanguard's bond fund.
Bought and held in a brokerage account, never spent or banked, `BND` is the
fund, so it lands on Stocks.

To override this rule, use the [`holdings:` tag](tags.md#the-holdings--tag).

## What the columns mean

| Column     | Meaning                                                              |
|------------|----------------------------------------------------------------------|
| Value      | The account's balance at the as-of date, priced in the base currency. |
| Cost       | The same balance at cost (hledger's `-B`).                            |
| Change     | Value − reference (see below).                                        |
| Change %   | Change ÷ reference.                                                   |

The **reference** is chosen by the same window control the Stocks tab uses, and
means the same thing on both:

- **All time** → the reference is *cost*, so change is the gain over what you
  paid.
- **Any bounded window** (1 week, 1 month, 3 months, year to date, 12 months,
  5 years) → the reference is the account's *value at the
  start of the window*. An asset you bought inside the window references zero, so
  the whole purchase reads as that window's change.

Totals sum only the rows that carry the input they need. An asset whose commodity
has no price route to the base currency contributes to no total and raises a
warning naming it, rather than being silently counted as zero.

## Scope, dates and the account chooser

Both tabs share one scope bar: the account filter, the as-of date, and the
period all apply to whichever tab is open.

### The period

The **Period** control sets two things at once, on both tabs: the window the
gain (Stocks) or change (Other) figures are measured over, and the span of the
*Value over time* chart. A chart of the last twelve months beside a one-week
gain would be two answers to the same question, so there is one control.

Every window ends at the as-of date. Gain and change columns carry a suffix
naming it — `Gain (1wk)`, `Change (5yr)` — so a windowed number is never read as
an all-time one.

| Period        | Gain measured from                     | Chart                                   |
|---------------|----------------------------------------|-----------------------------------------|
| 1 week        | seven days before as-of                | daily, 8 points                          |
| 1 month       | one calendar month before (31st → 28th/29th) | daily                              |
| 3 months      | three calendar months before           | weekly                                   |
| Year to date  | December 31 of the previous year       | daily → weekly → monthly as the year goes on |
| 12 months     | one year before                        | monthly: 13 points                       |
| 5 years       | five years before                      | monthly: 61 points                       |
| All time      | cost (the gain over what you paid)     | from the scope's first holding activity  |

For every bounded window the chart's **first point is the gain's reference**:
it is taken on the date the gain is measured from, not at the end of that
date's month or week, and shows exactly the value the gain subtracts. The
points after it fall at the end of each day, week or month, and the last one
is the as-of date. So the twelve-month chart for an as-of of 28 September 2026
starts on 28 September 2025, then runs through the twelve month-ends to 28
September 2026. The benchmark comparison is seeded at that same first point.

A windowed gain is measured against the value at the *end* of its start date,
and counts only money moved in or out after it. That is why year to date starts
from December 31: its closing value is what the year opened with, and anything
you bought or sold on January 1 belongs to this year.

For **year to date** and **all time** the engine picks the interval so the line
has enough points to read without crowding: months once there are at least six
of them, weeks (or days) before that, and quarters or years past ten years of
history. "All time" starts at the first transaction that moved a holding in the
current scope *on that tab* — so the Other tab's chart starts at the house, not
at the first share you bought.

Daily and weekly points are labelled by date on the chart ("Sep 28"); a weekly
point is the value at the end of its week. The period is part of the page URL
(`?gain=1wk`, `?gain=5yr`, …), so a shared link opens on the same window.

The account chooser's options are **not** filtered by the current scope or date,
deliberately. An option that vanished the moment you deselected it could not be
reselected, and one that vanished when you travelled back a month would make a
scope impossible to compose. So it offers every account that could ever be a row,
whether or not it holds anything today.
