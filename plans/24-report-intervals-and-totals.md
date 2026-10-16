# 24 — Report intervals and Total columns (QuickBooks-style P&L and Cash Flow)

Bring the P&L and Cash Flow reports closer to how QuickBooks lays them out:
an Interval control on both, a right-hand Total column whenever there is more
than one column, and a "% change" in place of "% of revenue".

## The ask (Patrick, 2026-10-16)

> For P&L, add an Interval dropdown like we have with cash flow. Allow for
> Interval to have an option for "Total" which will just show the totals for the
> selected period, which is what we have now so make that the default. Get rid
> of the "% of revenue" column and replace with a %change (between the periods)
> instead. If someone chooses monthly, don't do the comparison to the previous
> period or % change stuff at all. If someone chooses Monthly, add a Total
> column on the right that sums up all the months (or whatever interval).
>
> For Cash Flow report, add a Total column that sums up everything to its left.
> Under the Interval drop down, allow selection of just Total but note that this
> means we'll have to change to using a start and end date and interval
> specifies monthly/weekly/whatever, but the Periods chooser goes away.

Plus one small UI item: when there are no problems, hide the problems
triangle in the header.

## Decisions

1. **One interval vocabulary for both reports.** `ReportInterval` gains
   `"total"`. The UI options are Total, Monthly, Quarterly, Yearly, in that order.
   Net Worth does not get Total, because its columns are point-in-time
   snapshots and summing them means nothing. The Interval select takes an
   options list per tab.
2. **P&L with Interval = Total (the default) behaves as it does today:** one
   window `[from, to]` compared with the prior window of the same length.
   The "% of revenue" column becomes **% change** =
   `(current − prior) / |prior|`. When prior is zero there is no percentage and
   the cell shows "—". This uses exact BigInt arithmetic with the existing
   `divRoundHalfAway`. The net-income panel shows % change too.
3. **P&L with Interval ≠ Total:** one column per bucket across `[from, to]`.
   The first bucket is clamped to `from` and the last to `to`. A **Total**
   column comes last. There is no prior comparison and no % column, and the
   request sends `compare=none`. **Total = sum of the bucket columns**, as in
   QuickBooks. Under market valuation each bucket is valued at its own end,
   so the Total can differ from a single whole-range valuation. That is
   intended, and the docs say so. Section and group membership is still
   resolved over the union of all windows, as it is today.
4. **Backend shape for P&L intervals:** add an `interval` query param to
   `/api/reports/incomestatement/grouped`, with `total` as the default so
   existing callers are unchanged. The current two-window `Amounts`
   (current/prior) stays as it is for Total mode. Interval mode returns a
   separate per-bucket shape (buckets + `Vec<MixedAmount>` per row/group/
   subtotal/section/net + `total`), built by generalising `Windows` to N
   windows rather than copying it. The engineer chooses the exact type. The
   constraint is one summing path and the purity rule: `reports/` has no
   clock and no I/O.
5. **Cash Flow moves to From/To.** The controls are From, To, Interval (Total/
   Monthly/Quarterly/Yearly) and Depth. The Periods chooser goes away for cf
   but stays for Net Worth.
   - Defaults: `to` = today, `from` = the first day of the month 11 months
     earlier. That gives the same 12 monthly columns as today.
   - Backend `/api/reports/cashflow` accepts `from`/`to`. Buckets cover
     `[from, to]`, with the first clamped to `from` and the last to `to`.
     `interval=total` gives a single bucket. The legacy `end`/`count` params
     keep working when `from` is absent, so old URLs and API callers don't break.
   - SPA URL back-compat: `?tab=cf&end=…&count=…` without `from` is translated
     to an equivalent from/to.
6. **Cash Flow Total column:** each row gets a total (sum of its buckets), and
   the Net row gets a grand total. These are computed in Rust and added to
   the wire as an optional `total` per row plus `grandTotal`, present only
   for cash flow. The shared `ReportTable`/xlsx `addPeriod` path renders a
   Total column only when the report carries totals, so Net Worth is
   unchanged. When there is exactly one bucket (Interval = Total) the Total
   column would just repeat it, so the table shows only the one column,
   headed "Total". The chart keeps its existing "widen the range" empty state.
7. **XLSX follows the screen.** P&L Total mode exports with a "% change"
   header. P&L interval mode exports N bucket columns plus Total and no %.
   Cash Flow exports a Total column. Export titles and filenames show the
   from/to range and the interval.
8. **Problems icon:** `ProblemsBadge` renders nothing when
   `problems.count === 0`. The keyboard shortcut that opens the drawer still
   works, and the drawer's "No problems found" text stays.

## Work split

- **Engineer A (reports):** decisions 1–7 end to end, covering Rust core,
  server, wire golden (`just snapshot-native`), decoders, params/URL codec,
  controls, views, xlsx, and unit, component and e2e tests. P&L and cf share
  the interval plumbing, so one agent does both, in sequence, to avoid
  conflicts in `params.ts`, `ReportControls.svelte`, `xlsx.ts` and
  `reports_api.rs`. It does **not** edit `docs/`; it hands off a list of doc
  changes instead.
- **Iris:** decision 8 (small, separate files).

## Done when

`just pre-push` passes (fmt, clippy `-D warnings`, cargo test, vitest, check,
lint), `just e2e` passes, and the wire goldens are regenerated and reviewed.
