# Cash Flow and Net Worth, and their charts

Both tabs are period reports: one column per bucket (monthly, quarterly or yearly), the last `count` buckets ending with the one that holds the end date, accounts rolled up and clamped to the chosen depth. Cash Flow defaults to twelve months, Net Worth to five years. Each tab draws a chart above its table. The table is the chart's accessible twin: every number the chart draws is in the table too.

Each chart sits in a collapsible panel that is open by default. Whether it is open, and which Cash Flow breakdown it shows, is remembered per browser.

## Net Worth

Net worth at the end of each bucket, valued at market prices in one base commodity (the `valueIn` commodity, else the journal's base). Membership follows each account's *effective declared type*, and never its name: an account is in net worth when its type is Asset (Cash included) or Liability. See [balance-sheet.md](balance-sheet.md) for how types are resolved when a journal declares none.

### The chart

Each bucket is one stacked column. Asset accounts stack above zero, liability accounts below it. The net worth is a dashed line across the columns, with a dot and its value at each bucket when there are six or fewer. With more buckets, only the last point is labelled, since labels on every point would collide at phone width and the latest figure is the one you came for.

- **The stack never double counts.** The table lists a parent and its children together, but the chart stacks only the *leaves* at the chosen depth. When a parent also holds postings of its own, those show as a separate `account (own)` segment. So the segments in a column always add up to the depth-1 totals.
- **Which side an account goes on comes from the engine, not from the sign.** Each net-worth row carries a `kind` (`asset`, `liability` or `mixed`) that is decided by effective type. Within a column, a segment is still drawn on the side its value falls. An overdrawn checking account is an asset-coloured segment *below* zero, and a card carrying a refund is a liability-coloured segment above it. That is the honest picture, and the tooltip and the table carry the account name either way. A `mixed` row is one where declared types nest a liability under an asset parent and the depth clamp folds them together. It is placed by the sign of its total.
- **The tail is folded on each side.** Around six accounts keep their names, and at least two are kept on each side, so a large asset base cannot push every liability into the tail. The rest of each side becomes one `(other)` segment in a muted grey. A tail of just one account is never folded, because "(other)" would only hide its name.
- **Colour follows the account.** Categorical colours are assigned once per chart, assets first and then liabilities, so an account keeps its colour in every column.
- Hovering a column lists each segment's account and value for that period, followed by the net worth.

## Cash Flow

The per-bucket change in every *cash* account (effective type Cash, or the hledger name heuristic when a journal declares no types), with natural signs: money into cash is positive. Cash flow is **not** valued. A journal with cash in several currencies has one figure per currency. The chart draws the currency with the most figures and names the rest under the chart ("Charted in $ only; EUR not shown").

### The chart

Stacked areas: money into cash above zero, money out below it, and the net change as a dashed line. A toggle switches between two breakdowns of *the same* movement. The net line is identical in both.

**By source** (the default) answers *why* cash moved. Each bucket's movement is attributed to the accounts on the other side of each transaction: salary above the axis, rent, groceries and the card payment below it, and a stock purchase below it as `assets:broker`. This needs its own request, which is made only while this view is open.

**By account** answers *where* it moved. It uses the table's own rows (the cash accounts). A transfer from checking to savings shows as one layer up and one down, and it cancels in the net.

An account can be positive one month and negative the next. It is drawn as two layers, one per sign, that share its colour and a single legend entry. Stacking an account whose sign flips as one layer would trace the layer's edge on the wrong side of the axis.

### How a transaction's cash is attributed to its sources

For each transaction that touches cash, for each bucket (a cash posting's own `date:` decides the bucket, as in the table), and for each commodity:

1. **D** is the sum of the transaction's cash postings. If D is zero, as in a transfer between two cash accounts, nothing is attributed: cash moving between cash accounts is internal.
2. The **weights** are the amounts on the non-cash postings in the same commodity. When there are none (buying shares with cash, where the counterparty is in `AAPL`), the non-cash postings' amounts *at cost* are used instead.
3. Each counterparty receives `D × weight / Σ weights`. In the ordinary balanced transaction that is exactly the negation of what it was posted, with no division involved: a `$100.00` grocery bill is `−$100.00` of groceries, and a paycheck split between checking and savings, with taxes withheld, credits salary with the gross and debits taxes with the withholding. Otherwise the shares are rounded to the cash amount's precision, and the largest weight absorbs the rounding remainder.
4. When nothing can explain the movement, the whole of D goes to a row named `(unattributed)`. This happens with a currency exchange between two cash accounts, or with transfer legs dated into different buckets.

As a result, **per bucket, the sources add up to the cash flow total exactly**. The engine tests assert this on `fixtures/sample.journal` at every depth and interval.

Counterparties roll up and clamp to the chosen depth like any report row. Note that at depth 1 an `assets` source means *non-cash* assets, because cash accounts are never sources.

## Empty charts

Neither chart draws a broken plot. A window with nothing in it says so, and a window of a single bucket says to widen the range, because one column or one point is a number rather than a trend, and the table already shows it.

## API

```
GET /api/reports/networth
      ?end=YYYY-MM-DD&interval=monthly|quarterly|yearly&count=N&depth=N&valueIn=$
```

Each row carries `"kind": "asset" | "liability" | "mixed"`. The key is omitted, not `null`, on every other period report, so their bytes did not change.

```
GET /api/reports/cashflow
      ?end=YYYY-MM-DD&interval=monthly|quarterly|yearly&count=N&depth=N

GET /api/reports/cashflow/sources
      ?end=…&interval=…&count=…&depth=…   (exactly the cash flow's query)
```

`/sources` returns the same shape as the cash flow (`buckets`, `rows`, `totals`), with counterparty accounts as the rows and totals equal to the cash flow's, bucket for bucket. It is a separate route, rather than a field on the cash flow, because it is a second pass over every posting and only the collapsible chart reads it.
