# 36 amendment A1: a balance-over-time line in the Balances tab

Plan 36 gives the journal a pinned account with a running-balance column. This
amendment shows the same series as a chart. **While an account is pinned, the
Balances tab's cash pie becomes a line of that account's balance over time.**
The column gives exact figures row by row; the chart shows the shape of the
balance, so a dip, a doubled step or a drift away from the statement figures
stands out before you start reading rows.

Build this after plan 36 phases A–C (and E) are in and verified. It reuses
their engine, pin state and assertion data.

## The ask (Patrick, 2026-11-09)

> What if we also change the "balances" insights pie chart to a line graph when
> clicking on a specific account? So we could see the running balance in the
> journal entries, but can also see the balance over time in a chart.

## Decisions

1. **The pin decides it.** If an account is pinned, the Balances tab shows its
   line; with no pin, it shows the pie. Clicking a Balances row already pins
   (plan 36), so clicking an account turns the pie into its line, and ✕ on the
   pin chip brings the pie back. There's no separate chart toggle.
2. **Any pinned account gets a line, not only cash.** Pinning a brokerage or
   mortgage account from a report shows its line in this tab too. The heading
   names the account and commodity, since the list beside it may not include
   it. Pinning **doesn't switch tabs**: if you're on the P&L tab, you stay
   there.
3. **The chart follows the journal's date range.** The Balances list
   deliberately ignores the filter bar (plan 19, decision 1), and it still does.
   The chart is different: it describes the rows you're looking at, so
   **Narrow to gap** zooms the chart and the table together. "All time" shows
   the whole history. An open start runs from the subtree's first posting, an
   open end to today. As in the column, the **values** always include
   everything before the range; the range only sets the window.
4. **Same commodity as the column.** The chart follows the pin chip's commodity
   picker; it has no picker of its own.
5. **Values as written, zero in frame.** Liabilities plot negative, matching the
   Balance column. `includeZero` is on, as `PeriodLineChart` says a balance
   chart needs.
6. **End-of-period values on even periods.** Reuse `PeriodLineChart` and its
   conventions (x is the period index, integer ticks, points only at ≤31
   periods). The interval is chosen from the window's length: days up to ~90
   days, weeks up to ~2 years, months beyond that. **Reuse the interval rule
   the P&L line already uses** in `view/series.rs` rather than writing a second
   one. Each point is the balance at the end of its period.
7. **Assertions on the chart.** A period containing a **failing** assertion gets
   a distinct marker (a shape, not just a colour), and a second point at the
   *asserted* figure joined by a hairline. That shows the gap between the books
   and the statement as a vertical distance. Passing assertions aren't drawn;
   they'd crowd the chart and the column already shows them. The tooltip gives
   the date, balance, and for a failure, asserted / actual / difference.
8. **Clicking a period scrolls the table** to the newest row in that period,
   using the position route as the Problems drawer does. If the filter hides
   that row, nothing happens.
9. **The chart takes the pie's space and responsive rule.** It is hidden below
   `lg`, like the pie, so the transaction table keeps its height on small
   windows. It gets a little more width than the pie (`w-80` against `w-56`),
   because a line needs it more than a circle does; check it at 1280px.
10. **The pinned account's row is highlighted** in the Balances list when it
    appears there, so the row you clicked is visibly the one charted.

## Engine

`RunningBalance` (plan 36, `view/running.rs`) already holds each commodity's
ordered entries with the total after each. Add:

- `RunningBalance::series(commodity, from, to, interval) -> Vec<SeriesPoint>`:
  for each period end, the total after the last entry dated on or before that
  end, found by binary search over the entries already built, so there is no
  second pass over the journal. `SeriesPoint { label, end, balance,
  failures: Vec<AssertionMark> }`, where `failures` are the failing assertions
  inside the period, from the same `assertions()` plan 36 builds.
- The interval choice comes from `series.rs`'s existing logic, made `pub(crate)`
  if it isn't already.

Tests:

- Period ends land on the correct entry, including same-day ties and
  posting-dated legs (the ordering from plan 36).
- An open start is clamped to the first posting, an open end to `today`.
- The interval switches at the thresholds.
- **The series agrees with the column:** for every period end, the point equals
  the Balance column of the newest row on or before that date.
- An empty subtree in the window gives a flat line at the carried balance, not
  an empty series. A window before the first posting is all zero.

## Server

`GET /api/journal/running/series?account&commodity&from&to&today&interval`:

```jsonc
{
  "generation": "…",
  "account": "assets:elevations:money-market",
  "commodity": "$",
  "style": { … },
  "interval": "week",
  "points": [
    {"label": "Mar 2", "end": "2026-03-02", "balance": {"m": "1240000", "p": 2}},
    {"label": "Mar 9", "end": "2026-03-09", "balance": {…},
     "failures": [{"tindex": 812, "date": "2026-03-07", "asserted": {…}, "calculated": {…}}]}
  ]
}
```

`interval` is optional (the server picks). `commodity` falls back to the
default like plan 36's `balanceCommodity`; an unknown account is `400`. It uses
the same `ViewCache.running` entry as the rows route, so pinning builds the
data once for both. ETag is dated by `today`, gzip and the report pool as for
the sibling routes. Add it to the routes table in
`docs/development/journal-view.md`, and run `just test-integration`.

## SPA

- `stores/runningSeries.svelte.ts` (or a resource on the existing
  `journalView` store, whichever matches the pattern plan 36 used for
  `running`) follows (server, generation, pin, date range). It fetches **only
  while the Balances tab is active and a pin is set**, the same rule
  `cashBalances` follows.
- `insights/BalanceTrend.svelte`: a thin adapter over `PeriodLineChart` (as
  `holdings/ui/HoldingsTrend.svelte` is), with one series in the foreground
  colour, `includeZero`, the failure markers and asserted-point hairlines from
  decision 7, and clicking a period from decision 8. If `PeriodLineChart` can't
  take extra markers, add a small, general extension point (a points-overlay
  snippet) rather than forking the chart.
- `insights/BalancesView.svelte`: `{#if filters.value.pin}` → `BalanceTrend`,
  else the pie. The pinned row is highlighted. The heading above the chart
  reads "assets:elevations:money-market · $ · Mar 1 – Nov 9" so it's clear
  which account, commodity and window are shown.
- The loading state keeps the previous line on screen (as `journalView` does
  across generations) instead of flashing the pie.

Tests: decoder (good + malformed), `BalanceTrend` rendering (a line,
failure-marker count, tooltip text), `BalancesView` switching pie ↔ line on pin
and unpin, the fetch firing only while the tab is active, and clicking a period
calling the scroll. Add an integration suite for the route against
`fixtures/sample.journal`, and an unrun e2e spec: pin from Balances → the line
appears → ✕ → the pie returns.

## Docs

- `docs/journal/running-balances.md`: a "Balance over time" section, plus a
  screenshot from the plan 36 docs spec showing the column and the line
  together.
- The Balances section of the insights docs (wherever the pie is described)
  gets one line: pinning an account replaces the pie with that account's
  balance line.

## Open questions

- With a very long all-time window at monthly intervals, an error inside one
  month (say, a duplicate reversed a week later) won't show, because only month
  ends are plotted. Possible fix: draw each period's minimum and maximum as a
  faint band. Leave it until it causes a problem in real use.
- Should the P&L tab do the same thing (pinning swaps its pie for the pinned
  account's line)? Not for now: P&L is about flows over a period, and a balance
  line there would mix the two ideas.
