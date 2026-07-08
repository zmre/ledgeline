# WP-05: Insights Panel & Charts

Read `plans/00-overview.md` first, then WP-02 contracts. **Load the `dataviz` skill before writing any chart code.**

## Scope

The expandable/collapsible insights box at the top of the journal view: big numbers for the filtered period, a chart widget with pie/line modes, interval selector, and an account-depth slider.

## Out of scope

Reports (WP-06/07). Filter logic (consumes WP-04's store; stub with the documented shape if needed).

## Behavior

- Collapsed/expanded state persists in `settings.insightsOpen`.
- **Big numbers:** Income, Expenses, Net for the current filter period — computed from postings under `revenue`/`expense` root categories (`categorize` from WP-02), sign-adjusted so income displays positive. Large daisyUI `stat` components; primary commodity prominent, additional commodities listed small.
- **Chart widget**, two modes:
  - **Pie:** period totals per account at the selected depth (`accountTotals` → `rollUp` → `atDepth`), one slice per account, respecting current filters. Top N slices + "other" bucket to keep it readable.
  - **Line:** series over time; interval selectable `daily | weekly | monthly`; one line per selected-depth account (cap series count, e.g. top 6 by magnitude + "other"). X buckets from `periods.ts` (WP-06) — if WP-06 hasn't landed, implement `bucketKey(date, interval)` locally in `series.ts` with pure string math and reconcile later.
- **Depth slider:** 1..maxDepth(visible accounts); affects both modes. Depth 1 = `assets`/`liabilities`/`expenses`/`equity`/`revenues`.
- All chart values via `toNumber()` at the last moment (display boundary); tooltips show `formatAmount` strings.
- Multi-commodity: chart one commodity at a time (dropdown when >1 present, default the most-used); never sum across commodities.

## Interface contracts

### `web/src/lib/insights/series.ts` (pure, unit-tested)

```ts
export type AccountSelection = ReadonlySet<string> | undefined;    // filter bar's subtree roots; empty/undefined = all
export interface PieDatum { account: string; value: number; formatted: string }
export interface LineSeries { account: string; points: { bucket: string; value: number }[] }
export function pieData(txns: Transaction[], opts: { depth: number; commodity: string; maxSlices?: number; accounts?: AccountSelection; conventionTxns?: Transaction[] }): PieDatum[];
export function lineData(txns: Transaction[], opts: { depth: number; commodity: string; interval: "daily" | "weekly" | "monthly"; maxSeries?: number; accounts?: AccountSelection; conventionTxns?: Transaction[] }): LineSeries[];
export function bigNumbers(txns: Transaction[], commodity: string, accounts?: AccountSelection, conventionTxns?: Transaction[]): { income: Dec; expenses: Dec; net: Dec };
export function commoditiesInUse(txns: Transaction[], accounts?: AccountSelection): string[];   // sorted by frequency
export type SignFactor = 1 | -1;
export interface SignConventions { revenue: SignFactor; expense: SignFactor }
export function signConventions(txns: Transaction[], commodity: string): SignConventions; // magnitude-weighted dominant sign per category
```

Posting-level filtering (added 2026-07-08): insights receive TXN-filtered data, but a
matching txn still carries its other legs (asset/liability side). All series
functions therefore take the filter's `accounts` selection and skip non-matching
postings — filtering to `expenses` must not chart the checking-account legs.

Display signs (added 2026-07-08): each of revenue/expenses displays with the
sign that makes its dominant money flow positive — income positive when money
came in, expenses positive when money was spent — whatever raw signs the
journal records (hledger standard, bank-sign CSV imports, or fully inverted).
`signConventions` detects the convention per commodity, magnitude-weighted
(immune to count skew), over `conventionTxns` = the WHOLE journal (passed down
from `journal.txns` as the panel's `allTxns` prop) so it never flip-flops
between filter periods; a refund-dominated filtered period under a standard
journal still nets negative. `net = income − expenses`.

## Components (`web/src/lib/insights/`)

- **`InsightsPanel.svelte`** — daisyUI `collapse`; header row always visible with net number even when collapsed.
- **`BigNumbers.svelte`** — three `stat`s; stack vertically on mobile.
- **`ChartWidget.svelte`** — LayerChart pie/line; mode toggle (daisyUI `join` buttons), interval select (line mode only), commodity select when needed. Responsive width; legible in dark theme (follow dataviz skill for palette/contrast; colors must be distinguishable and consistent between pie and line for the same account).
- **`DepthSlider.svelte`** — daisyUI `range` with tick labels.

## Key files created

`web/src/lib/insights/{InsightsPanel,BigNumbers,ChartWidget,DepthSlider}.svelte`, `web/src/lib/insights/series.ts` (+ `series.test.ts`), wired into `src/routes/+page.svelte`

## Depends on / parallel

Depends on: WP-02 (+ reads WP-04 filter store contract). Parallel with: WP-03, WP-04, WP-06.

## Definition of done

- `series.ts` unit tests: depth clamping, top-N + other bucketing, interval bucketing across month/year boundaries, sign conventions for income/expense
- Against fixture: big numbers match hand-checked `hledger is -p ...` values for the fixture month; pie slices sum to period total
- Mode/interval/depth changes are instant (derived, no refetch); collapse state persists
- Charts render correctly in dark theme, mobile + desktop; `just check` + `just test` green
- Commit: `feat: insights panel with pie/line charts and depth slider`
