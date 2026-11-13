# 41 — Automatic price updates; a Banktivity-style budget table

Two independent work packages, requested by Patrick on 2026-11-13. They share
no files except `TODO.md`/docs, so they run in parallel worktrees.

---

## WP-A — Automatic price updates

### What the user asked for

1. The Holdings → Stocks **Update prices** button becomes a coloured button
   (`btn-primary`), not the grey default.
2. A setting turns **automatic price updates** on. It can be global or kept
   for one ledger.
3. With it on, shortly after launch, if any held symbol lacks a price for the
   **latest completed close**, fetch immediately.
4. Then **backfill weekly**: for each of the last N weeks (default 6,
   configurable), if a symbol has no `P` directive anywhere in that week, add
   that week's Friday close (or the week's last trading day when Friday was a
   holiday).
5. All of it runs in the background. If the app stays open for days, the
   check repeats periodically.
6. Never hammer Yahoo.

### Decisions

- **Runs in the engine (`ledgeline-server`), not the SPA.** Preferences
  already live server-side (`ui_prefs.rs`), the write pipeline is already
  there (`prices_api.rs`), and a WebView that's hidden or throttled must not
  stop the schedule. A `tokio` task is spawned at server start-up.
- **Settings** are two new `ui_prefs` specs:
  - `autoPrices` — flag, default `false`.
  - `autoPricesWeeks` — whole, `0..=52`, default `6`. `0` means latest close
    only, no backfill.
  Both are added to `PINNABLE`, so the Settings page's existing "keep for this
  ledger" mechanism (plan 39) gives the per-repo override. The ledger layer
  wins over the global one, as for every other key.
- **One Yahoo request per symbol per run, whatever N is.** The latest close
  and every backfill Friday come from the same daily chart request, with
  `period1` set to N weeks + 7 days back. More weeks cost no extra requests;
  they only make the response bigger.
- **Rate limits:**
  - Auto runs use concurrency 2 (the button keeps 5), with ≥ 250 ms between
    request starts.
  - A `429` or `999` from Yahoo stops the run. Auto runs are then
    suppressed for an exponential backoff (1 h, doubling, capped at 24 h),
    held in memory.
  - The periodic tick is hourly, and it is a **local** check: does the
    journal already have what we want? The network is touched only when
    something's missing.
  - Memo per (symbol, target close date): a symbol that's been asked for once
    is not asked again until the target date moves. This covers delisted
    tickers and exchange holidays (the fetch returns an older candle, which
    would otherwise look missing forever). The memo lives in memory only, so
    a relaunch retries once.
- **"Latest completed close."** The local check uses a US-centric heuristic.
  The target is today if it's a weekday and now is after 21:30 UTC (covers
  16:00 ET in both EST and EDT, plus margin), else the most recent weekday
  before today. The fetch itself is exchange-aware: a candle for the current
  session is dropped unless Yahoo's `meta.currentTradingPeriod.regular.end`
  has passed. **The manual button gets the same fix**, because today it can
  record an intraday price as the day's close.
- **Which symbols need a price for a date D** (added 2026-11-13). A
  commodity needs a price dated D only if it's held at some point in
  `[D, today]`. In other words, D ≤ its last-held date, where an open
  position counts as today. Examples:
  - Buy X on Sep 1 and still hold it: X gets backfilled even for weeks
    before Sep 1.
  - Sell all of Y on Sep 5: Y gets nothing after Sep 5's week.
  - Sell all of X yesterday: X doesn't get today's close.

  The last-held date comes from the same holdings engine the Stocks tab uses.
- **Prices file stays in date order** (added 2026-11-13). New `P` lines,
  from the manual button and from auto runs, go in chronological position
  after the last `P` line dated on or before them. They're not appended.
  Existing lines, comments and other directives are never reordered, so a
  file that's already out of order stays that way.
- **Weeks** are Monday–Sunday. "No other updates that week" means no `P`
  directive for that commodity dated in that week, in any included file.
- **No silent file creation.** If no file holds prices yet (`canCreateFile`),
  the auto run does nothing. The setting's help text says to click Update
  prices once to create `prices.journal`. Read-only journals: nothing runs.
- **Writes** reuse `plan_update`/`apply_update`, generalised from "one
  `FetchedPrice` per symbol" to "a list of (date, price) per symbol". They
  take the same write mutex, pin and reparse-and-verify, and dedupe through
  `line_exists`. Lines are appended in date order. A stale-pin conflict (the
  user edited mid-run) skips this run quietly; the next tick retries.
- **UI feedback.** `GET /api/prices/status` gains `auto: {enabled, lastRun,
  lastOutcome, backoffUntil}`. The Holdings page shows a quiet "Prices
  auto-updated 2 h ago" line next to the button. The SPA's existing
  file-change watcher already reloads holdings after the engine writes.
- **Tests.** Fake `PriceFeed` (the existing seam, extended with a history
  call). The schedule logic is a pure function `plan_auto(now, journal,
  prefs, memo) -> AutoPlan`, unit-tested with fixed clocks: weekends,
  21:29 vs 21:31 UTC, holiday memo, a week already covered by a mid-week `P`,
  N=0, backoff. The integration test drives the tick with the fake feed
  against a fixture copy and checks the appended lines.

### Files

`yahoo.rs` (range fetch + completed-session filter), `prices_api.rs` (shared
apply), new `auto_prices.rs` (pure planner + task), `ui_prefs.rs` (+2 specs,
PINNABLE), `lib.rs` (spawn), `web/src/lib/settings/uiPrefs.ts`,
`settings.svelte.ts`, Settings page section, `UpdatePricesButton.svelte`,
docs page for Holdings.

---

## WP-B — Budget tab as a Banktivity-style table (desktop)

### What the user asked for

Use Banktivity's budget screen as the reference (screenshot in the request):

- Each rule sits on the same row as its progress, instead of in a separate
  list below.
- Scannable columns: **Budgeted | Actual | Remaining**.
- Gradient bars with ambers and oranges, instead of binary green/red.

### Decisions

- **Merged rows** (Patrick chose this over two side-by-side panels). On wide
  layouts (a container query, per plan 39: `@4xl` or wider) each section
  (Income, Expenses) renders as a grid:

  `Account | Budgeted | Actual + bar | Remaining`

  - A **Total** row heads each section (the section's `overall` bar).
  - **Budgeted** is the span goal, right-aligned and tabular. When exactly
    one editable goal backs the account, the amount is a button (primary
    colour, like Banktivity's blue) that opens the existing `GoalModal` for
    that goal. With several backing goals (e.g. monthly + yearly), the
    button opens a small popover listing them, each with Edit/Remove. With
    none editable, it's plain text and the lock tooltip.
  - **Actual** sits above the bar, left-aligned. **Remaining** is
    right-aligned with the engine's label word (`left`, `over`, `to go`,
    `ahead of pace`…), coloured by health.
  - The account name still opens the journal filtered to that account.
- **The goals editor shrinks** on wide layouts to the goals that have no row
  in the table: other periods, and accounts deeper or shallower than the
  current depth's leaf rows. It keeps "Add a budget goal". On narrow layouts
  today's stacked summary + editor stay exactly as they are.
- **Bar colour is a gradient fixed to the track, not to the fill.** The
  gradient spans the full track width, and the fill reveals it
  (`background-size` set to the track width). A bar at 40 % shows only
  green; at 85 % it reaches amber; at 100 % it ends in orange. Over budget
  turns the overflow red past the goal marker.
  - Expenses: `success → warning (≈75 %) → orange (100 %)`, with
    `error` beyond the goal marker.
  - Income: the opposite meaning, so a calm `info → success` ramp, with no
    red for earning more than the target. That keeps plan 20's "earned more
    than target must not read as overspend".
  - Pace still matters: when health is `behind`/`over`, the Remaining text
    goes `text-warning` or `text-error`. The pace tick stays.
  - Orange is a theme token. Add `--color-orange` to the theme layer
    (derived from `warning`/`error` with `color-mix` so all 20 themes get
    one) and use it everywhere. No hard-coded hex.
- **Sort, depth and period controls** are unchanged and drive the merged
  table.
- **Tests:** component tests for the merged row (columns, edit affordance
  per goal count, gradient style for expense vs income, narrow layout
  unchanged). The pure helper that maps summary rows to backing goals
  (`account` + span → goals from the listing) gets unit tests:
  multi-period, locked, other-period, depth mismatch.

### Files

`web/src/routes/budget/+page.svelte`, `web/src/lib/reports/ui/BudgetSummary.svelte`
(or a new `BudgetTable.svelte` used at wide widths), `web/src/lib/budget/ui/BudgetEditor.svelte`
(filter prop), new `web/src/lib/budget/rowGoals.ts`, `web/src/app.css` theme
token, docs page for Budget plus its screenshot.
