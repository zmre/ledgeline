# 34 — Compare vs actuals: backtest a projection over the Actuals periods

A **Compare vs actuals** checkbox on the Projections tab. When it is ticked, every
reading stops projecting forward. Each one shows the Actuals periods twice:
first as they happened, then as the scenario on screen would have projected
them if the projection had been run on the first day of those periods. Two
columns at the end give the difference. You use it to see where, and by how
much, reality and the projection differed, in either direction. Coming in
above the projection is as useful to know as coming in below it.

The engine work is small. A backtest is an ordinary projection whose cutoff is
the first actual day instead of today, and plan 29's cache already keys real
sides by cutoff. Most of the work is joining two statements into one table and
adding Δ columns to the four shared statement views.

## The ask (Patrick, 2026-11-09)

> Compare projections: sometimes you want to know how accurate your projection
> was versus actuals and where it differed so you can improve your projection
> model. I want a way, for each report, to 'compare vs previous' as a checkbox.
>
> - if that is checked, then for the `actuals` previous periods, compare
>   actuals to what would have been projected if we did our projection at the
>   start of the period.
> - no need to project forward for this, but we do want to be able to do this
>   for each report type. lets say we have 3 actuals periods: show those three
>   as actuals, then a line, then show those three periods again as projected,
>   then a column showing differences (absolute and percent difference)

Settled with Patrick before this plan was written (marked **(P)** below):

1. **(P) Anchor once.** One projection, starting from the real balances on the
   first day of the first actual period, runs forward N periods (N = the
   Actuals control) with the **current** scenario on screen, unsaved edits
   included. Balance-sheet drift accumulates; that is intended. It should
   agree with what `hledger --forecast` produces for that window.
2. **(P) One Δ and one Δ% for the whole window.** Flows (income statement,
   cash flow) compare the window's **sum**. Balances (balance sheet, account
   balances) compare the **last** period.
3. **(P) No forward columns while comparing.** Layout:
   `[N actual] | [N as projected] | Δ | Δ%`.

Resolved with Patrick on review (2026-11-09):

4. **(P) Prices:** D3 as written. The projected side is valued at the prices
   known before `F` plus the scenario's grown prices.
5. **(P) Counter defaults:** today's inferred defaults, not ones re-inferred
   at `F` (D1).
6. **(P) Label:** "Compare vs actuals" (D4).
7. **(P) The backtest runs the scenario exactly as written** (D2). It never
   adjusts or re-seeds the scenario to suit the test window.
8. **(P) Wording:** a gap between actual and projected is a *difference*. It
   is never called a miss or an error, or labelled good or bad, in UI copy,
   docs or this plan. Beating the projection (more revenue, a stock above
   its growth rate) is as interesting as falling short of it.
9. **(P) Asset-row `opening:` overrides are ignored by the backtest.** Every
   account starts from the journal's real balance on `F`. An override states
   today's balance, so it is not a forecasting rule. The backtest anchors on
   real balances at `F` (D1), and the scenario's rules still run exactly as
   written (D2).

## What is true today

### Engine

- **The run.** `POST /api/projections/run` is `projections_api::run`
  (`crates/ledgeline-server/src/projections_api.rs:1846-1946`). It resolves a
  `Window` from `asOf`/`interval`/`count`/`depth`, bounds `lookback + count` by
  `MAX_BUCKETS` (`checked_lookback`, :1249; `MAX_BUCKETS = 1200`,
  `reports/periods.rs:355`), decodes the scenario, then on a `compute` thread:
  - `facts = cache.facts(journal)`;
  - `key = facts.key(window.end, interval, lookback, value_in)` (:1884);
  - `real = cache.side(journal, &facts, &key)`;
  - `project_cached` → `projected_statements` → `projected_accounts` when
    `accounts` is asked → `forecast_journal` when `journal` is asked.
- **`RunRequest`** (:1203-1233) is `deny_unknown_fields`. `accounts` and
  `journal` are opt-in flags. The SPA sends them only while their tab is open
  (`runBody`, `web/src/lib/projections/scenarioModel.ts:826-840`).
- **`WireProjection`** (:803-850) carries `buckets`, `lookback`, `actualsEnd`,
  `projectedFrom`, `cash {opening, values, history}` (`history` is the REAL
  closing cash at each lookback end, :631-634), `runway`, `assets`,
  `warnings`, the three statements over `lookback ++ buckets`, `defaults`, and
  `accounts`/`journal` (both `null` unless asked).
- **The cutoff and the window.** `JournalFacts::key`
  (`crates/ledgeline-core/src/projections/real_side.rs:143-157`) computes the
  cutoff with `forecast_cutoff(today, latest_real, interval)`: the start of the
  period holding the earlier of today and the day after the latest real
  transaction (plan 28 Decision 5). `ForecastWindow::from_cutoff`
  (`projections/forecast.rs:175-203`) puts `from` on the first lookback day
  and `end` on the last forecast day. `RealKey` (real_side.rs:162-172) is
  `{cutoff, interval, lookback, value_in}`.
- **The real side** (`RealSide`, real_side.rs:189-226) reads only real
  transactions dated **before its cutoff**. That covers the opening balances
  (`balances`, at the day before), every real price before the cutoff
  (`prices`, `db`), the explicit `P` directives the statements value with
  (`explicit`, `explicit_db`, `priced`), `CounterDefaults` (inferred over the
  trailing year before the cutoff, no header), the per-lookback-column seeds
  (`sheets`, `written`, `at_cost`, `cash_flow`, `before_from`), and the `late`
  postings: those dated on or after the cutoff on transactions dated before
  it, which are replayed beside the forecast. A real transaction dated on or
  after the cutoff never reaches any figure. This is exactly the leak guard a
  backtest needs.
- **The cache** (`crates/ledgeline-server/src/projection_cache.rs`) is an LRU
  of `SIDES = 4` real sides per snapshot (:42), keyed by `RealKey`.
- **`project_cached`** (real_side.rs:493-530) expands the scenario with
  `RealInputs {window, prices, db, target, balances, defaults, first_index}`
  all taken from the one real side, then rolls the series forward.
- **Occurrences.** The expansion walks `budget::forecast_occurrences`
  (`reports/budget.rs:259-275`). A bare `Every` rule (no `from`) is phased from
  the **span start**, as `hledger --forecast` does (plan 28 amendment 25), not
  from the unit boundary that `budget::occurrences` uses. Growth anchors are
  the row's `from`, else the cutoff. Disabled entries are dropped by
  `Scenario::active` (`projections.rs:629`) before any expansion. `=` rules
  apply to generated transactions only (`forecast.rs:1080-1112`).
- **Prices.** Grown prices are in-memory only (plan 28 Decision 3).
  `statement_prices` (real_side.rs:360-372) adds them only for commodities a
  real explicit `P` before the cutoff prices (plan 28 amendment 16).
- **Builders the join can reuse:**
  - `pivot_balance_sheets(&[BalanceSheetReport])`
    (`reports/balance_sheet.rs:1194`) joins any list of single-date sheets
    into one column tree, with `{}` where a column lacks a node;
  - `income_statement_columns_from(direct, prices, opts, …)`
    (`reports/income_statement.rs:1035`) finishes per-window direct totals,
    but takes ONE `PriceDb` and derives its windows from `opts.from/to`;
  - `CfAccumulator` (`reports/cash_flow_statement.rs:445`, `extend` :487,
    `finish` :577) resolves operating/investing at `finish` from its own
    `investments` set;
  - `account_report_from(direct, opening, opts)`
    (`reports/account_report.rs:188`) reads assets and liabilities as
    balances and every other section as changes.
- **hledger parity** (`crates/ledgeline-core/tests/projection_files.rs`) has
  `Parity` (:1346), `Parity::over(Some(main_text), …)` to run over an edited
  main journal, `documented()` (:1468, the exact command), and
  `PARITY_TODAY = "2026-10-20"` (:1311). `fixtures/projections-parity/main.journal`
  reprices EUR on 2026-09-15 (:91), inside the default two-month lookback
  (Aug–Sep 2026).

### SPA

- **Controls bar** (`web/src/routes/projections/+page.svelte:476-526`):
  Interval, Periods, Actuals (`lookback`, 0–`MAX_COUNT` = 120), depth slider.
- **`projectionWindow`** (:262-269) is what the run is keyed on. It carries
  `accounts` and `journal` only while their tab is open. `ProjectionWindow` and
  `sameProjectionQuery` are in `scenarioStore.svelte.ts:47-60` and `:90-100`.
- **URL codec** (`web/src/lib/projections/params.ts`): `tab`, `interval`,
  `count`, `lookback`, `depth`, `empty`, `show`, all written in full.
- **Readings** (`projections/ui/ProjectionReports.svelte`):
  - The runway chart (:183-200) plots `[...cash.history, ...cash.values]` with
    `projectedFrom` at the history length. `PeriodSeries.dashed` exists
    (`components/periodSeries.ts:16`).
  - The statements (:206-225) render through `BalanceSheetView`,
    `IncomeStatementView`, `CashFlowStatementView` and `AccountReportView`,
    each with `projectedFrom`.
  - `ProjectedHeadRow.svelte` prints the Actual / Projected band, and
    `projectedColumns.ts` holds `isProjected`, `projectedEdge` and
    `projectedSpans`. When `projectedFrom` is null nothing changes, which is
    how the Reports tab stays as it was.
- **Column keys are each-block keys.** `BalanceSheetView.svelte:229,249`,
  `IncomeStatementView.svelte:239`, `CashFlowStatementView.svelte:137` and
  `AccountReportView.svelte:110` key `{#each}` on `column.key`/`period.key`.
  Svelte 5 throws on a duplicate key, so a joined report cannot repeat bucket
  keys.
- **The % precedent.** The income statement's Total-mode "% change" column is
  computed client-side from engine figures: `pctChange(current, prior, base)`
  (`reports/ui/incomeStatementRows.ts:159`) is `(now − was) ÷ |was|`, exact
  BigInt, one decimal, half away from zero, `null` (shown `—`) for a zero or
  absent `was` or no base commodity. `fmtPct` (:191) formats it.
- **Money math**: `maAdd`, `maNeg`, `maIsZero` (`domain/money.ts`), all exact.
- **Decoder**: `decodeProjection` (`api/nativeDecode.ts:2799`) runs the
  statements through the Reports decoders. The key sweep's `PROJECTION_RUN`
  literal (`nativeDecode.test.ts:1928`) mirrors `WireProjection` field for
  field, and `TOLERATED` must never grow.
- **Export** (`projections/export.ts:116-197`) reuses the Reports workbook
  builders, with forecast columns suffixed " (projected)" (`markProjected`,
  :32).

## Decisions

### D1. The backtest is a projection at an earlier cutoff

The backtest's key is the main key moved back:

```text
main      RealKey {cutoff: C, interval, lookback: N, value_in}
backtest  RealKey {cutoff: F, interval, lookback: 0, value_in}
          F = ForecastWindow::from_cutoff(C, interval, N, 0).from
backtest window = from_cutoff(F, interval, 0, N): exactly the N lookback spans
```

`RealSide::build(journal, facts, backtest_key)` then does what the backtest
needs with no new code:

- openings are the real balances the day before `F`;
- prices are those dated before `F`;
- real transactions dated on or after `F` are left out;
- late postings are replayed.

Real activity inside the window cannot leak into the projected side, because
the real side never reads it. The cache holds the backtest side under its own
key, so an edit costs the warm path twice and never rescans the journal.

**(P) One deviation from "the run at `asOf = F`": counter defaults.** A counter-less
line takes the **main** run's inferred default (today's cutoff, the scenario's
`; cash:` header applied), not a default re-inferred at `F`. Those are the
accounts the editor shows greyed, and the ones a save would write
(`upgrade_legacy_counters` at today's cutoff, plan 28 amendment 7). So the
backtest of what you see is the backtest of what you would save, and the
hledger parity over a saved file holds. When every line names its counter (any
file saved since plan 28), the backtest equals `POST /run {asOf: F,
lookback: 0, count: N}` exactly. The live integration test pins that.

### D2. The scenario exactly as written, run from `F` (P)

**Principle.** The scenario under test is whatever the user has on screen,
whether hand-written or seeded, with unsaved edits included. The backtest
never adjusts it to suit the test window. It does not re-seed unbudgeted
averages from the months before `F`, re-infer anything the file states, or
skip entries that look like hindsight. A scenario tuned to the window would
no longer be the scenario being tested, and the comparison would mean nothing.
Every row of the table below follows from this rule.

**The one exception, and why it is consistent: `opening:` overrides.** An
asset row's `opening:` tag states what the account holds *today*. It is a
correction to the starting point, not a rule about what happens over time. A
backtest's starting point is fixed by D1: the real balances on `F`. So the
backtest ignores the override and starts that account from the journal's
balance on `F`. The row's rules (contribution, `growth:`, gains account) still
run as written. Nothing in the scenario is edited: the expansion reads the
override as absent (`RealInputs.backtest`), and the file, the editor and the
forward run keep it. One knock-on effect: a priced row with an override grows
through prices in the backtest, because plan 28 amendment 10's "no `opening:`
override" condition now holds there. This also helps hledger parity, because
hledger never reads `opening:` and always starts from the real balance.


| Entry                                         | In the backtest                                                                                         |
|-----------------------------------------------|---------------------------------------------------------------------------------------------------------|
| Rule with `from` before `F`                   | Fires from its `from`'s phase; occurrences inside the window count                                      |
| Rule with `from` inside the window            | Fires from its `from`                                                                                   |
| Rule with `from` after the window             | Never fires                                                                                             |
| Rule with `to` before `F`                     | Never fires                                                                                             |
| Bare rule (`~ every 2 weeks`, no `from`)      | Phased from `F` (`forecast_occurrences`), so it can land on other days than in the forward run          |
| Dated one-off / event inside the window       | Fires, even though the forward run (cutoff after it) never shows it                                     |
| Event before `F` or after the window          | Never fires                                                                                             |
| Step change (`to D` / `from D` pair)          | Whichever segment covers each date, as written                                                          |
| Disabled entry (plan 31 C7)                   | Left out (`Scenario::active`), as in the forward run                                                    |
| `=` auto rule                                 | Applied to the backtest's generated transactions only, as in the forward run                            |
| `growth:` on a flow or unpriced asset row     | Anchored at the row's `from`, else `F`: a fresh projection made on `F`                                  |
| `growth:` on a priced holding                 | Prices grow from the latest real price **before `F`**                                                   |
| Asset row `opening:` override                 | Ignored: the account starts from its real balance on `F`; its rules still run (see above)               |

hledger can reproduce all of this except the two deviations plan 28 already
accepts: `growth:` amounts and grown prices. hledger shows both flat. D8 gives
the command and its one extra caveat about prices.

### D3. Valuation: each side at the prices it would have had (P)

Both blocks are valued at market into the projection's target commodity
(`Forecast::target`: `valueIn`, else the price table's base), each column at its
own end, as the statements are today:

- **Actual block:** real explicit `P` directives up to the column end. These
  are exactly today's lookback columns.
- **As-projected block:** real explicit `P` directives dated **before `F`**,
  plus the scenario's grown prices for commodities those price. This is what
  `statement_prices` of the backtest side gives, unchanged.

So a stock that rose faster or slower than its `growth:` rate shows up as a Δ
on the balance sheet. That difference between the growth assumption and the
market is one of the things this feature exists to show. The **Accounts** reading is unvalued (plan 28,
"Valuation: none"), so it shows quantity deviations with price taken out. The
docs say so. If the two blocks resolve different targets (a journal whose `D`
or base commodity first appears inside the window), the run warns and values
the projected block into the actual block's target.

### D4. One checkbox for every reading, in the shared controls bar (P: label)

**Compare vs actuals** is a checkbox after **Actuals** in the controls bar.
It is mirrored to the URL as `compare=0|1` and written in full, like every other
param. It applies to every tab. Per-tab checkboxes were rejected for three
reasons:

- the window and anchor are shared by every reading;
- the runs are shared (one request answers all tabs);
- a per-tab switch would leave the strip mixing forward-looking and backward-
  looking readings with nothing on screen to say which is which.

This is the same reason Interval, Periods and Actuals are shared.

- **Disabled at Actuals = 0.** The box is greyed, and a hint beside it reads
  "Set Actuals to 1 or more to compare". The URL keeps `compare=1`, so raising
  Actuals brings the comparison back. The effective flag is
  `compare && lookback > 0`.
- **Periods is dimmed while comparing** (`aria-disabled`, title "Not used while
  comparing"). Its value is kept, and the forward run still uses it for the
  editor's placeholders and warnings (D6).
- No keyboard shortcut in this plan. Digits 1–6 still switch readings.

### D5. Δ and Δ%: definitions

Per row, over the two blocks of N columns:

- **Window figure.** `flow`: the sum of the block's N values. `balance`: the
  block's last value. `opening`: the block's first value (cash flow beginning
  cash only).
- **Δ = actual − projected**, per commodity (`maAdd(actual, maNeg(projected))`),
  exact. A positive Δ means reality came in above the projection in the
  figure's own displayed sign:
  - an income statement cost row with Δ > 0 means you spent more than
    projected;
  - a revenue row with Δ > 0 means you earned more;
  - Net income with Δ > 0 means net income came in above the projection.
  The Accounts reading is in natural signs, so there revenue more than
  projected reads Δ < 0. The docs give both cases.
- **Δ% = Δ ÷ |projected| × 100**, one decimal, rounded half away from zero, in
  the base commodity. It is `pctChange(actual, projected, base)` from
  `incomeStatementRows.ts`, moved to a shared module and reused, not re-written.
  The denominator is the **magnitude**, so the sign of Δ% always matches Δ, and
  sign flips read sensibly: projected −$100 with actual +$50 gives
  Δ = +$150, **+150.0%**.
- **Δ% is `—` (null)** when:
  - the projected figure is zero or absent;
  - the projected figure **rounds to zero at the base commodity's display
    precision** (`$0.004` reads `$0.00`, and a ratio against a figure the
    reader sees as zero is noise). This tightens `pctChange`, which only checks
    for exactly zero. The IS Total-mode column keeps its current behaviour,
    because the new check is a separate argument;
  - either figure holds a commodity other than base (an unpriced holding). A
    ratio of the convertible part alone would mislead;
  - the report has no base commodity.
- **Display cap.** |Δ%| ≥ 1000 prints `>999%` or `<-999%`. The exact figure is
  in the cell's `title` and in the workbook.
- **No colour by direction.** A difference above the projection is as worth
  reading as one below it, and what a sign means depends on the row. The
  cells use the same neutral ink as the Total column they replace. UI copy
  says "difference", "above" and "below", never "miss", "error", "good" or
  "bad".
- **Where it is computed: the client**, in one pure function
  `compareCells(values, split, reading, base, precision)` in
  `web/src/lib/reports/ui/compareColumns.ts`. It sits beside
  `projectedColumns.ts`, and all four views and the export use it. Reasons:
  - It follows the split the IS % column already uses: the engine sends exact
    figures and the client forms the ratio.
  - Every input is on the row, so a Δ can never disagree with the cells beside
    it.
  - None of the four shared wire types (and none of their native goldens)
    changes.
  This leans against plan 26's "logic in Rust". The function is about 40 lines
  over exact decimals, and it goes on plan 26's port list (Phase 4 task).
  Joining the two blocks is real logic (which rows line up, chain compression,
  the income statement's ladder), so the join is done in Rust.

### D6. Wire: a `compare` flag and a `compare` block on the same run

Not a sibling endpoint. A backtest needs the same decoded scenario, facts and
defaults as the forward run, and a second request per edit would decode and
validate the scenario twice and double the debounce traffic. The pattern is
the one plan 28 used for `accounts` and `journal`.

**Request:** `compare: bool`, `#[serde(default)]`.
- `compare` with `lookback = 0` returns `400` ("compare needs at least one
  actual period").
- `compare` with `2 × lookback > MAX_BUCKETS` returns `400`, naming both
  numbers.
- The SPA sends the key **only when true**, like `journal`. A new SPA against
  an older engine (which is `deny_unknown_fields`) still works with the box
  unticked. Desktop builds ship both halves together.

**Response:** `compare: WireComparison | null`. The key is always present
(plan 22 amendment 14), and it is `null` unless the request asked.

```text
compare {
  from              F, the first day of the window (the backtest's cutoff)
  actualsEnd        C, the forward run's cutoff (the window ends the day before)
  columns           N; each statement below has 2N columns: actual, then as projected
  cash {opening,    real cash the day before F
        actual[N],  = the forward run's cash.history
        projected[N]}  the backtest's closing cash per period
  balanceSheet      WireBalanceSheetColumns, 2N columns
  incomeStatement   WireIncomeStatementReport (interval mode), 2N buckets
  cashFlow          WireCashFlow, 2N buckets
  accounts          WireAccountReport over 2N periods | null (when `accounts` asked)
  journal           WireForecastJournal: the BACKTEST's generated txns | null (when `journal` asked)
  warnings          backtest warnings the forward run's list does not already hold
}
```

- **The forward run is unchanged and still computed.** The editor needs its
  `defaults`, `assets` and `warnings` whatever the tab shows. With `compare`,
  the top-level `accounts` and `journal` are `null` and the compare block
  carries them. They are the only readings a comparing page shows, so the
  forward ones are not built.
- **Column keys in the projected block are prefixed `projected:`**
  (`projected:2026-08`). Labels and `from`/`to` are unchanged. This keeps the
  views' keyed `{#each}` legal. Phase 3 checks that no decoder or consumer
  parses a statement column key as a date (the drill links read `from`/`to`).
- **Each joined statement is a valid report of its type.** Its Total-like
  fields (`Amounts.current`, `CfColumns.total`, `AccountReportRow.summary`)
  follow the type's own rule over all 2N columns, so every invariant a decoder
  or test checks still holds. Those fields mean nothing across two blocks, so
  no view and no workbook shows them in compare mode. Δ and Δ% take their
  place.
- **`just test-integration` is required** for Phase 3 (`test:unit` skips the
  live-engine tests).

### D7. Warnings specific to the backtest

These go in `compare.warnings` and show in the page's warnings alert with the
prefix "Comparison:":

- Untouched seed lines (`source: unbudgeted`, entry unchanged): "N rows are
  averages of the last 12 months, which include the periods being compared,
  so they will look more accurate than a real forecast would have been." This
  is a lookahead bias the user cannot otherwise see. The SPA can build this
  one from the scenario, so it is a client-side line, not an engine warning
  (Phase 4). It is only a warning. Per D2 the scenario is never re-seeded or
  adjusted.
- Target mismatch (D3).

**No line for ignored `opening:` overrides.** Ignoring them (D2) is a fixed
rule, not a situation the run detects, so a line would appear on every
comparison of every scenario that has one and become noise. Nothing is wrong
when it fires. The docs state the rule, and the editor still shows the
override where it is used: in the forward run.

### D8. The hledger command for the projected block

The forward command with `F` in place of the cutoff and `C` as the end:

```sh
hledger -f <main> -f <scenario> bs|is|cf|bal|print --forecast=<F>..<C> --auto \
  -b <F> -e <C> -<interval> \
  expr:"NOT (date:<F>.. AND NOT tag:generated)"
```

The actual block is the Reports-tab command over the same range:
`hledger -f <main> bs|is|cf -b <F> -e <C> -<interval>`.

**One caveat beyond growth.** hledger has no way to leave `P` directives out by
date. With `--value=end,$`, a real `P` dated inside the window values the
projected block at that real price, and Ledgeline does not use it (D3). So:

- unvalued readings (`bal`, `print`, `-B`) always agree;
- valued readings agree when no real `P` is dated in `[F, C)`. Otherwise they
  agree against a copy of the main journal with those `P` lines removed, which
  is what "as the journal stood on F" means.

The docs say this and show how to make that copy.

### D9. Journal tab: in scope, as the backtest's generated transactions

While comparing, the Journal reading lists the **backtest's** generated
transactions under the heading "Generated as if projected on 1 Aug 2026"
(`hledger print --forecast=F..C tag:generated`). This costs one call to the
existing `forecast_journal` and answers "what did my model assume happened".
It is not compared row by row with real transactions. Matching generated to
real transactions is a reconciliation problem, and it is out of scope.

### D10. Performance: only while ticked, and cached

- **Only while ticked.** `projectionWindow` gains `compare`, so the backtest
  runs only while the box is ticked and Actuals > 0, the way Accounts and
  Journal run only while their tab is open.
- **Cold** (first run after ticking, or after a journal change): one more
  `RealSide::build`, one journal scan. Plan 29 measured cold at
  68 ms / 319 ms for 50k / 200k transactions, including `JournalFacts`, which
  is shared. Estimate +50 ms at 50k and +250 ms at 200k, once.
- **Warm** (every edit): a second expansion over N periods, statements over N
  columns, an N-column actual block from the main side's seeds, and the joins.
  The current warm run is 5 ms for 26 columns, flat in history. Estimate
  +1–3 ms at the default N = 2, and growing with N, not with history.
- **Cache:** the steady state holds two sides (forward and backtest). `SIDES =
  4` covers two configurations, for example flipping the interval and back.
  It stays at 4. Each side costs about 9 MB at 50k and 16–25 MB at 200k (plan
  29 §Memory).
- **Benches:** `run_cold_compare` and `run_warm_compare` beside plan 29's ids.
  Target: warm compare at most 2× warm at 50k, and flat in history length.

## Scope

**In:** the backtest key and expansion; the joined statements for BS, IS, CF and
Accounts; the wire block; the checkbox and URL param; Δ/Δ% in the four shared
views; the runway overlay; the Journal reading; xlsx for every reading; hledger
parity for the window; docs; benches.

**Out:**
- rolling re-anchoring (one projection per period);
- comparing two scenarios;
- matching generated transactions to real ones;
- attributing a Δ to a scenario line ("salary was off by $500"). The Accounts
  reading gets close, and line attribution is a follow-up if wanted;
- comparing on the Reports tab (budget vs actual already exists there);
- a keyboard shortcut for the checkbox.

---

## Phase 1 — Core: backtest and joined statements (PR 1)

New module `crates/ledgeline-core/src/projections/backtest.rs`. It is inside
`projections`, so it can read `RealSide`'s private seeds through
`pub(super)` accessors. Nothing is added to `RealSide`'s public API except what
is listed here.

```rust
impl RealKey {
    /// D1: the key of the projection made on the first actual day. `None` at lookback 0.
    pub fn backtest(&self) -> Result<Option<RealKey>, ReportError>;
}
/// D1: expand `scenario` against `backtest` (window = the N lookback spans of `main`),
/// with `main`'s counter defaults (header applied). `scenario` is already `active()`.
pub fn project_backtest(scenario: &Scenario, main: &RealSide, backtest: &RealSide,
                        depth: usize) -> Result<Projection, ReportError>;
pub struct ComparedStatements {
    pub actual: Vec<BucketSpan>,          // N
    pub projected: Vec<BucketSpan>,       // the same N spans, keys prefixed `projected:`
    pub balance_sheet: Vec<BalanceSheetReport>,   // 2N, ready for pivot_balance_sheets
    pub income_statement: IncomeStatementReport<Columns>,  // 2N buckets
    pub cash_flow: CashFlowStatement,                     // 2N buckets
}
pub fn compared_statements(main: &RealSide, backtest: &RealSide, facts: &JournalFacts,
                           forecast: &Forecast /* the backtest's */, opts: &StatementOpts)
    -> Result<ComparedStatements, ReportError>;
pub fn compared_accounts(main: &RealSide, backtest: &RealSide, facts: &JournalFacts,
                         forecast: &Forecast, opts: &AccountsOpts) -> Result<AccountReport, ReportError>;
/// The full-scan oracle, for tests and benches only.
pub fn reference_compared_statements(journal: &Journal, facts: &JournalFacts, main_key: &RealKey,
                                     forecast: &Forecast, opts: &StatementOpts)
    -> Result<ComparedStatements, ReportError>;
pub fn reference_compared_accounts(..) -> Result<AccountReport, ReportError>;
pub const PROJECTED_KEY_PREFIX: &str = "projected:";
```

Tasks:

1. **`RealKey::backtest`.** `F = ForecastWindow::from_cutoff(cutoff, interval,
   lookback, 0)?.from`. It returns `None` at lookback 0. Pure, no clock.
2. **`project_backtest`.** `forecast::expand_from` with `RealInputs` from
   `backtest` except `defaults`, which is `main`'s. The scenario's `; cash:`
   header is applied inside the expansion, as it is today
   (`for_lines_under`, `forecast.rs:290-327`), so nothing changes there. Then `roll_forward`
   against the backtest's openings. Make `project_cached` and
   `project_backtest` share one private `run_with(scenario, real, defaults,
   count, depth)` (DRY). Add `backtest: bool` to `RealInputs`. When it is set,
   the expansion's asset path reads every row's `opening` as `None` (D2's
   exception). The scenario value is not cloned or edited.
3. **The actual block from `main`'s seeds**, with no forecast:
   - BS: `balance_sheet_columns_seeded` over `main.sheets` with an empty tail,
     ends = the lookback ends, `main.explicit_db`;
   - IS: `main.written`/`at_cost`;
   - CF: a clone of `main.cash_flow` finished over the lookback spans;
   - Accounts: `main.written[..N]` + `main.before_from`.
   Invariant (tested): equal by `Debug` to the first N columns of today's
   `projected_statements` / `projected_accounts`.
4. **The projected block** is `projected_statements`/`projected_accounts`
   over `backtest` + the backtest forecast. The window is exactly N columns,
   so it reuses the same code with no change.
5. **Balance sheet join:** `actual_reports ++ projected_reports` →
   `pivot_balance_sheets` (unchanged) in the server.
6. **Income statement join: one build over 2N windows, not a post-hoc
   merge.** A post-hoc zero-fill is wrong for the GAAP ladder. If the scenario
   books COGS that the actual window never had, the actual block has no COGS
   box and no Gross profit line, and a zero-filled Gross profit would read $0
   instead of revenue. So generalise `income_statement_columns_from` into an
   inner `income_statement_windows(direct: Vec<Totals>, windows:
   &[BucketSpan], prices: &[&PriceDb], …)`:
   - explicit windows instead of `from/to/interval`;
   - one price table per window: `main.explicit_db` for the actual block,
     `backtest.statement_prices(forecast)` for the projected one.
   `income_statement_columns_from` becomes a one-line caller with one table
   repeated, so the Reports tab is untouched. The ladder, `multiStep` and
   section presence are then computed over both blocks at once, by the
   existing code.
7. **Cash flow join: post-hoc, by key.** One accumulator over 2N columns would
   carry cash forward from the actual block into the projected one, and the
   projected block must start from real cash on `F`. Finish both accumulators
   with the **union** of their `investments` sets (`finish_with(tags, opts,
   investments)`), so an account sits in the same section in both blocks.
   Then `join_cash_flows(a, b)`:
   - sections by kind (operating always; investing and financing when either
     block has them);
   - lines by `(name, source)`, rows by account, cash lines and rows likewise,
     in the builder's own name order;
   - absent nodes are zero-filled. That is correct here, because an absent CF
     line is a zero line;
   - `total` recomputed by the `CfColumns` rule over 2N; `reconciles` is the
     AND of both blocks.
8. **Accounts join: post-hoc, by `(section, account)`.** Zero-fill is correct:
   a row is omitted only when its subtree is zero in every column. Section
   `totals` are concatenated, and `summary` is recomputed by the report's own
   rule over 2N. `periods`/`buckets` use the prefixed keys.
9. **References.** For the actual block, the full-scan `reference_statements`
   of the forward run, truncated to N columns. For the projected block,
   `reference_statements` over the backtest forecast. Both go through the same
   joins.

Tests (`projections/backtest.rs` unit tests, and
`tests/projection_backtest.rs`):

- `the_backtest_key_is_the_first_lookback_day` (monthly, quarterly, yearly,
  weekly; lookback 0 → `None`).
- `real_transactions_inside_the_window_never_reach_the_projected_block`: add
  a real transaction in the window. The actual block moves and the projected
  block is byte-identical by `Debug`.
- `a_late_posting_before_the_window_is_replayed_into_the_projected_block`.
- `the_actual_block_is_the_forward_runs_lookback_columns` (all four readings,
  by `Debug`).
- `the_projected_block_is_a_projection_made_on_the_first_day`. With explicit
  counters, `project_backtest` equals `project_cached` on a side built at
  `key(today = F, lookback 0)`.
- `a_counterless_line_takes_todays_default_not_one_inferred_at_the_window_start`.
  Use a journal whose most-paired account changes between `F` and `C`.
- D2 table, one test per row: `from` before, inside and after the window; `to`
  before `F`; bare `every 2 weeks` phased from `F`; an event inside, before
  and after; a step pair; a disabled rule, event and `=` rule; an auto rule;
  growth anchored at `F`.
- `an_opening_override_is_ignored_and_the_account_starts_from_its_real_balance`:
  a row with `opening:` and `growth:`. The backtest opens at the journal
  balance on `F` and books no override gains transaction, while its growth
  steps still post. The forward run over the same scenario still applies the
  override. A priced row with an override grows through prices in the
  backtest, and no warning is emitted.
- `grown_prices_start_from_the_last_real_price_before_the_window` and
  `a_real_price_inside_the_window_values_only_the_actual_block`.
- `the_income_statement_ladder_is_computed_over_both_blocks`: COGS only in the
  projected block, and actual Gross profit = actual revenue.
- `cash_flow_sections_agree_across_blocks_under_the_union_investments_set`.
- `projected_column_keys_are_prefixed_and_unique`.
- **Equivalence** (extends `tests/projection_cache.rs`): `compared_*` equals
  `reference_compared_*` by `Debug` over every parity fixture, sample.journal,
  the edge journal, intervals monthly/quarterly/yearly, lookback 1/2/7, depth
  1/2/unclamped, market/cost/none, plus 24 proptest cases.

Gates: `just engine-test`, `just engine-check`.

## Phase 2 — hledger parity for the backtest window (PR 1 or PR 2)

Extend `crates/ledgeline-core/tests/projection_files.rs` with
`Parity::backtest(file, cadence)`. It is `Parity::over` with the forecast
expanded at the backtest key: `today = F`, lookback 0, count N (the cadence's
lookback). It goes through the same save path, so counters are explicit and D1's
defaults question cannot arise. Plus `documented_backtest()`:

```sh
hledger -f <main> -f <scenario> <cmd> --forecast=<F>..<C> --auto \
  -b <F> -e <C> -<interval> expr:"NOT (date:<F>.. AND NOT tag:generated)"
```

New opt-in tests (`LEDGELINE_HLEDGER_PROJECTION_CHECK=1 cargo test -p
ledgeline-core --test projection_files`, already run by `just hledger-checks`):

- `backtest_parity_*` over `flows`, `events`, `legacy`, `assets`, `step`,
  `commodities`, `auto`, `periods`, monthly and quarterly:
  - `print … tag:generated` equals the backtest forecast's postings (date,
    account, commodity, amount multisets) and its order (the Journal reading);
  - `bal` (`-T`, `-H`, `--tree --no-elide --depth N`) equals the projected
    block of `compared_accounts`, per account, commodity and column;
  - `bs`, `bse`, `is` with `--value=end,$` over **`main` with every `P` dated
    on or after `F` removed** (written by the test through `Parity::over`)
    equal the projected block of `compared_statements`. `cf` is compared per
    cash account, as plan 28 amendment 26 does.
- `backtest_parity_actual_block_is_the_reports_command`: `hledger -f main bs|is
  -b F -e C -M --value=end,$` equals the actual block.
- `backtest_parity_an_in_window_price_is_the_only_valued_difference`: against
  the untouched `main.journal`, hledger's valued `bs` differs from ours exactly
  by the EUR holdings revalued at the 2026-09-15 `P` in the September column,
  and in nothing else. This pins D8's caveat as fact.
- `backtest_parity_with_growth_differs_only_by_the_accepted_deviations`: the
  `growth.journal` assertions, re-run on the window.
- `backtest_parity_needs_the_expr`: without the `expr:`, hledger counts the
  real August and September transactions on the projected side. This shows the
  command does real work.

## Phase 3 — Server and wire (PR 2)

`crates/ledgeline-server/src/projections_api.rs`:

- `RunRequest.compare: bool` (`#[serde(default)]`) and the two `400`s (D6).
- In `run`, when `compare`:
  - `main_key.backtest()?`, `cache.side(journal, &facts, &bkey)`;
  - `project_backtest(&scenario, &real, &backtest_side, window.depth)`;
  - `compared_statements(…)` with the same `StatementOpts`;
  - `compared_accounts` when `accounts` is asked;
  - `forecast_journal(&backtest_projection, …)` when `journal` is asked;
  - the forward `accounts`/`journal` are skipped.
- `WireComparison` and `WireCompareCash` structs. Statements go through the
  existing `wire_statements` (D6), with
  `pivot_balance_sheets(&[actual.., projected..])` and the joined spans.
  `warnings`: backtest warnings minus any already in the forward run's list,
  plus D3's target mismatch.
- Doc comment on `run` updated.

Tests (`crates/ledgeline-server/tests/projection_endpoints.rs`):

- `compare` is `null` unless asked; it is never absent.
- Lookback 0 with compare returns `400`, and `2N > MAX_BUCKETS` returns `400`,
  each naming its numbers.
- 2N columns everywhere; `compare.columns == N`; the projected keys are
  prefixed and all keys are unique.
- The actual block equals the forward run's first N columns in all four
  readings, and `cash.actual == cash.history`.
- The projected block equals `run {asOf: F, lookback: 0, count: N}` (explicit
  counters).
- Adding a real transaction in the window moves only the actual block.
- With compare, top-level `accounts`/`journal` are `null` and
  `compare.accounts`/`journal` are set when asked.
- Cache: a compare run adds exactly one side; a second edit hits both.

TypeScript (same PR, since the wire changes):

- `projections/types.ts`: `Comparison`, `CompareCash`, and
  `Projection.compare: Comparison | null`.
- `api/nativeDecode.ts`: `decodeComparison`. It uses the Reports decoders
  for the four statements and rejects a statement whose column count is not
  `2 × columns`, and `cash.actual`/`projected` whose length is not `columns`.
  `decodeProjection` demands the key (`decodeNullable`).
- `nativeDecode.test.ts`: `PROJECTION_RUN` gains a **non-null** `compare`, so
  the sweep reaches every key. `TOLERATED` does not grow.
- `api/native.ts`: `RunProjectionBody.compare?: true`.
  `scenarioModel.runBody` sends `compare: true` only when asked.
- `api/nativeLive.integration.test.ts`: a compare run decodes. Its projected
  block equals a run with `asOf = compare.from, lookback 0, count N`, and its
  actual block equals the forward run's lookback columns.

Bench (`crates/ledgeline-core/benches/engine.rs`): `run_cold_compare` and
`run_warm_compare` at 5k/50k/200k, N = 2 and N = 12. Results go into this plan.

Gates: `just engine-test`, `just test`, `just check`, `just lint`,
**`just test-integration`**.

## Phase 4 — The tab (PR 3)

**Params and the run**
- `params.ts`: `compare: boolean`, default `false`, `compare=0|1` written in
  full and parsed with `oneOf`. `compareActive(params) = compare && lookback > 0`.
  Update the scheme comment.
- `scenarioStore.svelte.ts`: `ProjectionWindow.compare`, and
  `sameProjectionQuery` compares it. `+page.svelte`: `projectionWindow` sends
  `compare: compareActive(params)`.

**The control** (`+page.svelte` controls bar, after Actuals):
- `<label class="label cursor-pointer gap-2"><input type="checkbox"
  class="checkbox checkbox-sm" …> Compare vs actuals</label>`, with
  `aria-describedby` pointing at a one-line hint: "Re-runs this scenario from
  <F> and sets it beside what happened". `F` is the label of the first actual
  column from the held projection, and the hint is omitted before one lands.
- Disabled at Actuals = 0, with the hint "Set Actuals to 1 or more to
  compare".
- Periods is `aria-disabled` and dimmed while comparing.
- While the compare run is pending, `projection.compare` is `null` and each
  reading shows "Running the comparison…" (the Accounts/Journal pattern).

**The shared pieces** (`web/src/lib/reports/ui/`):
- `compareColumns.ts`: `WindowReading = "flow" | "balance" | "opening"`,
  `compareCells(values, split, reading, base, precision) → {actual, projected,
  delta, pct}` and `fmtComparePct` (the `>999%` cap). `pctChange` moves to
  `reports/ui/percent.ts` with an optional `precision` argument (D5),
  re-exported from `incomeStatementRows.ts` so the IS's callers are unchanged.
- `ProjectedHeadRow.svelte` gains `projectedLabel = "Projected"` and
  `compareLabel: string | null`. In compare mode the band reads **Actual |
  As projected from 1 Jul 2026 | Difference**, and the third span covers the
  Δ and Δ% headings.
- Each projected-block header cell gets an `sr-only` " as projected", because
  the visible labels repeat.

**The four views** (`BalanceSheetView`, `IncomeStatementView`,
`CashFlowStatementView`, `AccountReportView`) each take an optional
`compareFrom: number | null` (default `null`):
- They render `projectedFrom = compareFrom` for the divider and band.
- They drop the Total/summary column and add two figure columns, Δ and Δ%, on
  every row, group, subtotal, section total and bottom line.
- The reading per figure:
  - BS: `balance` everywhere. The tie-out row's Δ cells stay blank, and the
    verdict stays per column;
  - IS: `flow`;
  - CF: `flow`, except beginning cash (`opening`) and ending cash (`balance`);
  - Accounts: the section's `reading` (`balance` for assets and liabilities,
    `flow` for `change`).
- `FigureColgroup`/`statementMinWidth` get the two extra columns. Collapse,
  the cursor and chain compression need no change (they act on rows).
- **With `compareFrom = null` the views render exactly as today.** The
  existing Reports-tab and projection component tests stay unchanged as the
  proof.

**ProjectionReports.svelte**:
- **Runway** in compare mode: `PeriodLineChart` over the N actual labels with
  two series: `{name: "Actual", values: cash.actual}` and `{name: "As
  projected from <F>", values: cash.projected, dashed: true}`. Two series get a
  legend, so identity is never carried by the dash alone. `includeZero` is
  kept, and there is no `markAt`.
  - The sentence changes to: "Cash at the end of Sep 2026 was **$31,850**,
    $1,850 (5.5%) less than projected on 1 Jul 2026."
  - "More" or "less" comes from the sign of Δ, and the figures come from
    `compareCells(…, "balance")`.
  - The uncharted-commodity note is kept.
- **BS / IS / CF / Accounts**: pass `report = compare.<statement>`,
  `compareFrom = compare.columns`.
- **Journal**: `ForecastJournalView` over `compare.journal`, with a heading
  prop: "Generated as if projected on 1 Jul 2026".
- **Warnings**: `compare.warnings` with a "Comparison: " prefix, joined into
  the page's alert. The seed lookahead warning (D7) is built here from the
  scenario (`source === "unbudgeted"` and not stale-edited).

**Export** (`projections/export.ts`, `export/xlsx.ts`):
- The four statement builders take an optional `compare: {split, reading}`.
  Projected-block headings get " (as projected)". The Total column is replaced
  by **Δ** and **Δ %**:
  - Δ is a money cell;
  - Δ % is a numeric fraction with format `0.0%`, uncapped; an empty cell when
    null.
  - Values come from `compareCells`.
- `buildRunwayWorkbook` gains a compare layout: Period | Actual | As projected
  | Δ | Δ %.
- The filename gains `-compare` before the span. The params line reads "N
  actual periods from F, compared with the scenario projected on F".
- Journal: the backtest's entries, with "as projected on F" in the period line.

**Vitest**
- `params.test.ts`: round trip, default, `compare=1&lookback=0` →
  `compareActive` false, a junk value → default.
- `compareColumns.test.ts`:
  - flow sum, balance last, opening first;
  - Δ sign; `+150.0%` on a sign flip;
  - zero, absent and rounds-to-zero denominators → null;
  - an unpriced second commodity → Δ% null and Δ still per commodity;
  - base null → null;
  - the cap and the exact title;
  - half-away rounding at ±x.x5.
- `percent.test.ts`: the moved `pctChange` keeps every existing IS
  expectation, and the precision argument is opt-in.
- `scenarioStore`/`scenarioModel`: `sameProjectionQuery` includes compare;
  `runBody` omits it when false.
- View component tests, one per view, over a 2×2-column fixture built from the
  goldens:
  - compare headers, band and sr-only text;
  - no Total;
  - Δ/Δ% on a row, a group, a subtotal (IS Gross profit) and the bottom line;
  - BS tie-out cells blank;
  - CF beginning Δ = 0;
  - Accounts balance vs change readings.
- `ProjectionReports.svelte.test.ts`: the checkbox's disabled state at
  Actuals 0; the runway's two series, legend, dashed flag and sentence; the
  pending placeholder; the Journal heading; the Comparison warnings.
- `export.test.ts`: column labels, Δ % numeric cells, the runway compare
  sheet, the filename.

**e2e** (`web/e2e/projections.e2e.ts`, specified here; agents do not run
Playwright):
1. Open Projections over `fixtures/sample.journal` with `lookback=3`, then
   tick **Compare vs actuals**. The URL gains `compare=1`.
2. Income statement:
   - 6 period headings, then Δ and Δ%;
   - the band reads Actual / As projected from … / Difference;
   - no Total heading;
   - the Net income row has a Δ cell and a Δ% cell.
3. Balance sheet: the Net worth Δ equals the last actual column minus the last
   projected column (read both cells and compare to the text).
4. Runway: the legend shows "Actual" and "As projected from …", and the
   sentence starts "Cash at the end of".
5. Reload: still comparing. Set Actuals to 0: the checkbox is disabled, the hint
   shows, and the IS shows forward Projected columns again. Set Actuals to 2:
   comparing again, with 4 period columns.
6. Journal tab heading "Generated as if projected on …".
7. The export button's download name contains `-compare-`.

Gates: `just test`, `just check`, `just lint`, `just test-integration`,
`just e2e` (run by Patrick or CI).

## Phase 5 — Docs (PR 3 or follow-up)

- New `docs/projections/compare.md` ("Comparing with actuals"), linked from
  `index.md`'s page list. It covers:
  - what the checkbox does and the anchor (D1);
  - the D2 table in prose;
  - D3 valuation: "the prices you would have had", and the Accounts tab for
    quantities only;
  - D5's Δ and Δ%, with a worked example of each sign case and the `—` and
    `>999%` rules;
  - D7's warnings, and why seeded averages flatter a backtest;
  - that `opening:` overrides are ignored, and why: they state today's
    balance, and a comparison starts from the real balance on `F`;
  - D8's commands, the price caveat, and how to make the `P`-truncated copy
    (`grep -v` on `P` lines dated on or after `F` is enough for a single-file
    journal);
  - growth and grown prices shown flat by hledger.
- `cash.md`: one paragraph pointing to the backtest command beside the forward
  one.
- Screenshots: add `compare-is.png` and `compare-runway.png` through
  `web/e2e/projections.docs.ts`.
- mbr: no `[!INFO]` callouts. Check that `docs/.mbr/config.toml` picks up the
  new page, and run the site locally (memory: ledgeline-docs-site-mbr).
- `plans/26-platform-readiness.md`: add `compareCells`/`pctChange` to the list
  of client logic to port.

---

## Mockups

All figures are in $. The window is Jul–Sep 2026, today is 2026-10-20, and N = 3.
Numbers are illustrative but internally consistent.

### Controls bar

```
 Interval [Monthly ▾]   Periods [ 24 ] (dimmed)   Actuals [ 3 ]   [✓] Compare vs actuals   Depth ━━●━━ 2
                                                                  Re-runs this scenario from 1 Jul 2026
                                                                  and sets it beside what happened
```

### Income statement, comparing

```
                        ACTUAL                          │ AS PROJECTED FROM 1 JUL 2026      │   DIFFERENCE
                    Jul 2026   Aug 2026   Sep 2026      │  Jul 2026   Aug 2026   Sep 2026   │          Δ       Δ%
 Revenue                                                │                                   │
   Salary          12,000.00  12,000.00  12,500.00      │ 12,000.00  12,000.00  12,000.00   │     500.00     1.4%
   Dividends          140.00       0.00     160.00      │    100.00     100.00     100.00   │       0.00     0.0%
 Total revenue     12,140.00  12,000.00  12,660.00      │ 12,100.00  12,100.00  12,100.00   │     500.00     1.4%
 Expenses                                               │                                   │
   Housing          4,200.00   4,200.00   4,200.00      │  4,200.00   4,200.00   4,200.00   │       0.00     0.0%
   Food               912.40   1,104.10     986.75      │    900.00     900.00     900.00   │     303.25    11.2%
   Travel             640.00   1,310.00       0.00      │    500.00     500.00     500.00   │     450.00    30.0%
   Insurance            0.00       0.00   1,200.00      │      0.00       0.00       0.00   │   1,200.00        —
 Total expenses     5,752.40   6,614.10   6,386.75      │  5,600.00   5,600.00   5,600.00   │   1,953.25    11.6%
 ═══════════════════════════════════════════════════════╪═══════════════════════════════════╪═══════════════════
 Net income         6,387.60   5,385.90   6,273.25      │  6,500.00   6,500.00   6,500.00   │  -1,453.25    -7.5%
```

Flows compare the window sums. Food: 3,003.25 − 2,700.00 = 303.25, and
303.25 ÷ 2,700.00 = 11.2%. Insurance was never projected, so its Δ% is `—`.
Net income: 18,046.75 − 19,500.00 = −1,453.25, which is −7.5%.

### Balance sheet, comparing

```
                        ACTUAL                          │ AS PROJECTED FROM 1 JUL 2026      │   DIFFERENCE
                    Jul 31     Aug 31     Sep 30        │  Jul 31     Aug 31     Sep 30     │          Δ       Δ%
 Assets                                                 │                                   │
   Checking        23,400.00  27,100.00  31,850.00      │ 23,900.00  28,800.00  33,700.00   │  -1,850.00    -5.5%
   Brokerage       10,400.00  10,900.00  11,600.00      │ 10,060.00  10,120.00  10,180.00   │   1,420.00    13.9%
 Total assets      33,800.00  38,000.00  43,450.00      │ 33,960.00  38,920.00  43,880.00   │    -430.00    -1.0%
 Liabilities                                            │                                   │
   Visa               850.00   1,200.00     640.00      │      0.00       0.00       0.00   │     640.00        —
 Total liabilities    850.00   1,200.00     640.00      │      0.00       0.00       0.00   │     640.00        —
 ═══════════════════════════════════════════════════════╪═══════════════════════════════════╪═══════════════════
 Net worth         32,950.00  36,800.00  42,810.00      │ 33,960.00  38,920.00  43,880.00   │  -1,070.00    -2.4%
 Equity  …                                              │                                   │
 Balances?                ✓          ✓          ✓       │         ✓          ✓          ✓   │
```

Balances compare the last column, Sep 30. Brokerage is valued at the real AAPL
price on the actual side and at the `growth: 7%/yr` price grown from June's last
`P` on the projected side (D3). The +13.9% is the difference between the stock's actual price and the
`growth:` assumption. The
projection assumed the card was paid off each month, so the Visa row has no
denominator.

### Runway, comparing

```
 Cash at the end of Sep 2026 was $31,850, $1,850 (5.5%) less than projected on 1 Jul 2026.

 Cash
 $34k ┤                                  ┄┄○  As projected from 1 Jul 2026
      │                    ┄┄┄┄┄┄○┄┄┄┄┄┄┄
 $30k ┤          ┄┄┄┄┄┄┄┄┄           ●     ●  Actual
      │  ○┄┄┄┄┄┄          ●──────────
 $25k ┤  ●────────────────
      └──────────────────┬───────────────────┬──
         Jul 2026        Aug 2026            Sep 2026
```

## Sequencing

1. **PR 1:** Phases 1 and 2 (core plus parity). There is no wire change, and
   the forward run is byte-identical.
2. **PR 2:** Phase 3 (server, wire, decoder, live integration, benches).
   `just test-integration` is required.
3. **PR 3:** Phase 4 (UI, export, e2e spec) and Phase 5 (docs).

Worktree agents: create worktrees from HEAD and warm `target/` with
`/bin/cp -Rc` (memory: worktree-agents-branch-from-main). Use `rg` or
`grep -a` on TS files (`model.ts` and others hold NUL bytes). Run
`just pre-push` after each merge.

## Risks

- **The income statement window refactor touches the Reports tab's builder.**
  Mitigation: `income_statement_columns_from` becomes a thin caller with one
  price table repeated. Every IS golden, `report_endpoints.rs` and the plan 29
  equivalence suite must pass unchanged.
- **Joined totals are meaningless but present** (D6). A consumer that shows
  them would print a sum across two blocks. Mitigation: the views and the
  export are the only consumers, and their compare-mode tests assert that no
  Total heading or cell exists.
- **Prefixed column keys.** Code that turns a column key into a date would
  break on `projected:2026-08`. Mitigation: Phase 3 checks every consumer of
  statement column keys with `rg -a`, and the decoder test feeds prefixed
  keys through every view.
- **Valued hledger parity needs a `P`-truncated journal** (D8). Users who run
  the documented command without the truncation see a different number when
  prices moved in the window. Mitigation: the docs say so, with the copy
  recipe, and a parity test pins the difference.
- **Counter defaults differ from "a projection made on F"** for unsaved
  counter-less lines (D1). Mitigation: chosen deliberately so the backtest
  matches the editor and the save. Saved files are unaffected.
- **Moving `pctChange`** could shift the IS % column. Mitigation: it is
  re-exported unchanged, the precision check is opt-in, and the existing
  tests stay where they are.
- **Cold cost on 200k journals** (about +250 ms the first time the box is
  ticked). Accepted. It is paid once per journal snapshot and key, and the
  "Running the comparison…" placeholder covers it.

## Open questions for Patrick

None. Every question was settled on review (2026-11-09; items 4–9 at the top).

## Definition of done

- The Compare vs actuals box drives every reading as in Phase 4. The forward
  run is unchanged when it is off.
- Phase 1's tests and the equivalence suite pass. The backtest parity suite
  passes under `LEDGELINE_HLEDGER_PROJECTION_CHECK=1`.
- Gates: `just engine-test`, `just engine-check`, `just hledger-checks`,
  `just test`, `just check`, `just lint`, `just test-integration`, `just e2e`.
- Bench numbers are recorded below. Docs and screenshots are published.

## Status

Not started.
