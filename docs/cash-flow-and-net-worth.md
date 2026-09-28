# Cash Flow and Net Worth, and their charts

Both tabs are period reports: one column per bucket (monthly, quarterly or yearly), the last `count` buckets ending with the one that holds the end date, accounts rolled up and clamped to the chosen depth. Cash Flow defaults to twelve months, Net Worth to five years. Each tab draws a chart above its table. The table is the chart's accessible twin: every number the chart draws is in the table too.

Each chart sits in a collapsible panel that is open by default. Whether it is open is remembered per browser.

## Each chart draws exactly its table

Both charts are built from the same displayed rows as the table below them, so the two cannot disagree:

- **The accounts are the table's accounts.** Each displayed row with no displayed child is a segment. A parent's figure in the table is the sum of the segments under it. When a parent has postings of its own, so that its figure differs from the sum of its children's, the difference is one more segment carrying the parent's name. Every figure in the table is therefore a segment, or a sum of segments.
- **The labels are the table's labels.** A segment is named with the text its row shows in the table: `checking` under `bank`, or `broker:taxable` for a chain the table collapses into one row. Only when two segments would read the same (a `cash` under two parents) do both use the full account name.
- **The net line is the table's Net row**, bucket for bucket.
- **Nothing is folded into "(other)".** Colours are assigned in table order. The first eight accounts get their own colours. Any after that are drawn in the muted grey, each still its own segment with its own legend entry and tooltip line, rather than reusing a colour and making two accounts look alike. An account keeps its colour in every bucket.
- **One currency is charted.** Amounts in other currencies cannot be drawn on the same axis, so the note under the chart names them ("Charted in $ only; EUR not shown"), and an account that holds only those currencies has no segment. The table still shows them. Net worth is valued into one commodity, so this mostly concerns Cash Flow.
- **An account whose sign changes** is drawn as two segments, one per sign, that share its colour and a single legend entry, so what is drawn above zero is exactly what was positive.

## Net Worth

Net worth at the end of each bucket, valued at market prices in one base commodity (the `valueIn` commodity, else the journal's base). Membership follows each account's *effective declared type*, and never its name: an account is in net worth when its type is Asset (Cash included) or Liability. See [balance-sheet.md](balance-sheet.md) for how types are resolved when a journal declares none.

### The chart

Each bucket is one stacked column. Asset accounts stack above zero, liability accounts below it. The net worth is a dashed line across the columns, with a dot and its value at each bucket when there are six or fewer. With more buckets, only the last point is labelled, since labels on every point would collide at phone width and the latest figure is the one you came for.

- **Which side an account goes on comes from the engine, not from the sign.** Each net-worth row carries a `kind` (`asset`, `liability` or `mixed`) that is decided by effective type. The legend lists the asset side and then the liability side, each in table order. Within a column, a segment is still drawn on the side its value falls. An overdrawn checking account is an asset-coloured segment *below* zero, and a card carrying a refund is a liability-coloured segment above it. That is the honest picture, and the tooltip and the table carry the account name either way. A `mixed` row is one where declared types nest a liability under an asset parent and the depth clamp folds them together. It is placed by the sign of its total.
- Hovering a column lists each segment's account and value for that period, followed by the net worth.

## Cash Flow

The per-bucket change in every *cash* account (effective type Cash, or the hledger name heuristic when a journal declares no types), with natural signs: money into cash is positive. Cash flow is **not** valued. A journal with cash in several currencies has one figure per currency. The chart draws the currency with the most figures and names the rest under the chart ("Charted in $ only; EUR not shown").

### The chart

Stacked areas: money into cash above zero, money out below it, and the net change as a dashed line. The legend is in table order. A transfer from checking to savings shows as one layer up and one down, and it cancels in the net.

Layers are filled solid, with a thin line of the page background between neighbours so adjacent accounts stay distinct.

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
```

The chart needs no request of its own: it draws the report the table shows.
