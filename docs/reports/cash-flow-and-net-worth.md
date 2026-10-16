---
title: Cash flow and net worth
description: "The Cash Flow and Net Worth period reports: From/To and Interval on Cash Flow, its Total column, End and Periods on Net Worth, and how their stacked charts match the table."
order: 40
tags: [reports]
---

# Cash Flow and Net Worth, and their charts

Both tabs are period reports: one column per period, with accounts rolled up and clamped to the chosen depth. Each tab draws a chart above its table, and every number the chart draws is in the table too.

The two tabs pick their columns differently. Cash Flow covers a date range, split by an interval, and ends with a Total column. Net Worth shows a number of periods ending at a date, with no Total, because its columns are balances on a date and adding them up means nothing.

## Cash Flow controls

| Control | What it sets | Default |
| --- | --- | --- |
| **From** | First day of the report, inclusive | The 1st of the month 11 months before today |
| **To** | Last day of the report, inclusive | Today |
| **Interval** | **Total**, **Monthly**, **Quarterly** or **Yearly** | Monthly |
| **Account depth** | How many levels of the account tree to show | 3 |

The defaults give twelve monthly columns, the current month included. The first and last columns are cut at From and To, so a range that starts on the 15th gives a half-month first column.

![The Cash Flow tab for January to June 2026: From and To boxed, Interval circled, and the Total column boxed on the right of the table](images/cash-flow.png)

### The Total column

The last column, **Total**, is the sum of the row's period columns: how much the account's cash changed over the whole range. The Total on the **Net** row is the grand total, the change across every cash account. A row with figures in more than one currency has one Total per currency.

### Interval = Total

Set Interval to **Total** and the table has one column, headed **Total**, covering From to To. A separate Total column would repeat it, so there isn't one. The chart has nothing to plot over time and says "Only one period in this range — widen it to see a trend." instead.

### Old links

Before Cash Flow had From and To it had **End** and **Periods**. A saved link such as `?tab=cf&end=2026-06-30&count=6&interval=monthly` still opens the report it showed: From becomes the first day of the oldest of the six periods, To becomes the End date, and the address bar is rewritten to the From/To form.

## Net Worth controls

Net Worth keeps the controls it had: **End**, **Interval** (Monthly, Quarterly or Yearly; no Total), **Periods** (up to 120) and **Account depth**. It shows that many periods, ending with the one that holds the End date, and defaults to five yearly columns. There is no Total column.

## The charts

Each chart sits in a collapsible panel that is open by default. Whether it is open is remembered per browser.

## Each chart draws exactly its table

Both charts are built from the same displayed rows as the table below them, so the two cannot disagree:

- **The accounts are the table's accounts.** Each displayed row with no displayed child is a segment. A parent's figure in the table is the sum of the segments under it. When a parent has postings of its own, so that its figure differs from the sum of its children's, the difference is one more segment carrying the parent's name. Every figure in the table is therefore a segment, or a sum of segments.
- **The labels are the table's labels.** A segment is named with the text its row shows in the table: `checking` under `bank`, or `broker:taxable` for a chain the table collapses into one row. Only when two segments would read the same (a `cash` under two parents) do both use the full account name.
- **The net line is the table's Net row**, bucket for bucket.
- **Nothing is folded into "(other)".** Colours go to the biggest accounts first, ranked by the largest amount each reaches in any bucket, so the accounts that dominate the chart are the ones you can tell apart. Every colour the theme defines (fourteen in the default theme) is used before any account is drawn in the muted grey, and a grey account is still its own segment with its own legend entry and tooltip line, rather than reusing a colour and making two accounts look alike. The legend stays in table order, and an account keeps its colour in every bucket.
- **One currency is charted.** Amounts in other currencies cannot be drawn on the same axis, so the note under the chart names them ("Charted in $ only; EUR not shown"), and an account that holds only those currencies has no segment. The table still shows them. Net worth is valued into one commodity, so this mostly concerns Cash Flow.
- **An account whose sign changes** is drawn as two segments, one per sign, that share its colour and a single legend entry, so what is drawn above zero is exactly what was positive.
- **Hovering names what is under the pointer.** Directly over a segment, the tooltip shows that one account and its value for the period, then the net. Anywhere else in the period (beside a bar, above or below the stack), it lists every account in that period in the order the column is drawn, top to bottom, so the list matches the stack beside it rather than the legend. On Cash Flow's areas, the segment is found at the pointer's exact position, between periods included, and the figures are the nearest period's.

## What Net Worth measures

Net worth at the end of each bucket, valued at market prices in one base commodity (the `valueIn` commodity, else the journal's base). Membership follows each account's *effective declared type*, and never its name: an account is in net worth when its type is Asset (Cash included) or Liability. See [balance-sheet.md](balance-sheet.md) for how types are resolved when a journal declares none.

### The chart

Each bucket is one stacked column. Asset accounts stack above zero, liability accounts below it. The net worth is a dashed line across the columns, with a dot and its value at each bucket when there are six or fewer. With more buckets, only the last point is labelled, since labels on every point would collide at phone width and the latest figure is the one you came for.

- **Which side an account goes on comes from the engine, not from the sign.** Each net-worth row carries a `kind` (`asset`, `liability` or `mixed`) that is decided by effective type. The legend lists the asset side and then the liability side, each in table order. Within a column, a segment is still drawn on the side its value falls. An overdrawn checking account is an asset-coloured segment *below* zero, and a card carrying a refund is a liability-coloured segment above it. That is the honest picture, and the tooltip and the table carry the account name either way. A `mixed` row is one where declared types nest a liability under an asset parent and the depth clamp folds them together. It is placed by the sign of its total.

## What Cash Flow measures

The per-bucket change in every *cash* account (effective type Cash, or the hledger name heuristic when a journal declares no types), with natural signs: money into cash is positive. Cash flow is **not** valued. A journal with cash in several currencies has one figure per currency. The chart draws the currency with the most figures and names the rest under the chart ("Charted in $ only; EUR not shown").

### The chart

Stacked areas: money into cash above zero, money out below it, and the net change as a dashed line. The legend is in table order. A transfer from checking to savings shows as one layer up and one down, and it cancels in the net.

Layers are filled solid, with a thin line of the page background between neighbours so adjacent accounts stay distinct.

## Empty charts

Neither chart draws a broken plot. A window with nothing in it says so, and a window of a single bucket says to widen the range, because one column or one point is a number rather than a trend, and the table already shows it.

## Export

The **Export .xlsx** button writes the table as it stands, at the current depth.

- **Cash Flow** has a column per period and then Total, or the single Total column when Interval is Total. The title line reads like `2026-01-01 to 2026-06-30, monthly, depth 3`, and the file is `cash-flow-2026-01-01-to-2026-06-30-monthly.xlsx`.
- **Net Worth** has a column per period and no Total. The title line names the periods and the depth, and the file is `net-worth-<end>.xlsx`.

## API

```
GET /api/reports/cashflow
      ?from=YYYY-MM-DD        (no default; switches to the range form)
      &to=YYYY-MM-DD          (default: today; both ends inclusive)
      &interval=total|monthly|quarterly|yearly
                              (default: monthly; daily and weekly also work)
      &depth=N                (default: 2)
```

`interval=total` returns one bucket whose key is `FROM..TO`.

Without `from`, the endpoint answers the older form, `?end=YYYY-MM-DD&interval=monthly|quarterly|yearly&count=N&depth=N`: the last `count` buckets ending with the one that holds `end`. Old callers keep working.

Both forms add a `total` to every row (the sum of its buckets) and a `grandTotal` to the report (the Net row's Total).

These requests get a `400`:

- `from` together with `end` or `count`.
- `to` without `from`.
- `interval=total` without `from`.
- `from` after `to`.
- A range that needs more than 1,200 columns.

```
GET /api/reports/networth
      ?end=YYYY-MM-DD&interval=monthly|quarterly|yearly&count=N&depth=N&valueIn=$
```

Each row carries `"kind": "asset" | "liability" | "mixed"`. The key is omitted, not `null`, on every other period report, so their bytes did not change. Net worth has no `total` and no `grandTotal`.

The charts need no request of their own: each draws the report its table shows.
