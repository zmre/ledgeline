# 20 — Reading a budget bar right: pace, the gaps below it, and an order you can scan

Three changes to the Budget tab that share two files and therefore share a plan:
the colour of a bar stops lying about income, every bar is judged against where
it *should* be by now rather than against the whole period's goal, an
expandable section says what the budget does not cover, and the goals can be
sorted. Driven by the TODO entries "budget: fix revenue display", "budget gaps"
and "budget sorting".

## The ask

> "budget: fix revenue display: There's something wrong when a budget shows you
> red for earning more money than expected. We need to treat
> revenues/income/sales/whatever differently from expenses in terms of how we
> present current state to the user. If revenue is under budget (and not
> on-track -- so if we're looking at annual budget but we're only on month six,
> then on-track would be half of annual budget) then the whole line should be
> red (and the label showing how much, too). If we're "over budget," meaning
> we've made more than we budgeted, then the whole bar should be green (with the
> target white line showing appropriately and the label in green too)."

> "budget gaps: show unbudgeted income and unbudgeted expenses to give the user
> some idea of how encompassing their budget is versus historical income and
> expenses and maybe hint about the higher-level categories with the biggest
> dollar amounts not in the budget. It may be fine to ignore these (I personally
> set budgets for some things, like clothing, but not for other things, like
> insurance) so this section should be expandable and default to being collapsed"

> "budget sorting: the budget goals are currently sorted however they are in the
> budget file which is whatever order they happened to be added in. So I have
> revenues:salary at the top and revenues:dividends at the bottom, which makes
> it hard to scan. The user should be able to sort the budget goals by account
> name or by dollar amount (ascending or descending)."

## Decisions (locked with Patrick 2026-09-18)

1. **Pace applies to both sides, symmetrically.** A bar is healthy when the
   fill is on the right side of the *pace* mark — at or above it for revenue, at
   or below it for expenses — and unhealthy otherwise. One sentence covers
   income and expenses, which is the only way a two-section screen stays
   legible. The alternative considered and rejected was fixing only the inverted
   income colours: it leaves a half-year view of an annual expense goal reading
   as a large, reassuring underspend, which is the same class of lie in the
   other direction.
2. **Pace is prorated by days elapsed in the report's own span, not by
   buckets.** `summarizeBudget` already sums a goal across every bucket in the
   span, so a `~ yearly` rule (which lands entirely in January's bucket) and a
   `~ monthly` rule × 12 both come out as the whole year's figure. Prorating
   that sum by elapsed days is correct for both, and it is the arithmetic the
   ask describes ("month six … would be half"). Counting completed buckets
   instead would step the mark once a month and make the same bar flip colour
   on the 1st for reasons that have nothing to do with money.
3. **A span that has already ended has a pace of 100%.** Looking at last year,
   `elapsed = 1`, the pace mark sits exactly on the goal mark, and every bar
   means what it means today. Nothing about historical review changes.
4. **The fill is one colour.** Today it is green up to the goal marker and red
   past it, which is what makes "earned more than target" render as a red
   overspend. The fill becomes a single colour decided by the pace test — the
   ask's "the whole line should be red" / "the whole bar should be green" — with
   the goal marker kept as the high-contrast line and the pace mark drawn as a
   second, quieter tick.
5. **Gaps use the same span as the bars, at the page's existing depth.** The
   numbers on one screen have to tie to each other. A trailing-12-month gap list
   under a one-month bar chart is two different questions stacked vertically.
6. **"Unbudgeted" means what the engine already means by it.** An account is
   unbudgeted exactly when `remap_account` sends it to `UNBUDGETED` — no second
   definition, no name matching. The gaps list is that same remap with the
   collapse-to-one-row step removed.
7. **The gaps list is filtered by resolved account type, to revenue and expense
   only.** Today's single `<unbudgeted>` row is dominated by the cash legs of
   every unbudgeted transaction and nets to a large negative number that answers
   nothing. Per [[account-type-not-name]], membership is decided by
   `resolve_account_type`, never by the account's name.
8. **Sorting is one control governing both the bars and the goals editor.** The
   complaint is about scanning, and the two halves of the page are the same
   subject read two ways — they must not disagree about order. Amounts are
   compared *annualised*, so a weekly goal and a yearly one sort against each
   other sensibly.
9. **The pure modules never read the clock.** `budgetSummary.ts` lives under
   `web/src/lib/reports/` and is guarded by `purity.test.ts`; `periods.rs`
   deliberately does not port `today()`. `asOf` is therefore a parameter passed
   down from the component, exactly as the subscriptions panel already passes
   the browser's local `today()`.

## Scope

In: the bar colour/label model, the pace mark, a gaps engine function and its
endpoint, a collapsible gaps section, and a sort control wired through both
halves of the page and the URL.

Out of scope, on purpose: the budget report's existing wire shape (`GET
/api/budget` is byte-for-byte pinned by `fixtures/native/v1/` and does not need
to change for any of this); the goal editor's write path; multi-commodity bars,
which already degrade to "no geometry, text only" and keep doing so.

---

## Phase 1 — Pace, and the colour that follows from it

### The bug, precisely

`web/src/lib/reports/ui/BudgetSummary.svelte:73`:

```ts
const state: BudgetBar["state"] = remValue !== null && remValue > 0 ? "under" : remValue !== null && remValue < 0 ? "over" : "onplan";
```

with `remainder = |goal| − |actual|` (`:69`) and `stateText` mapping
`under → text-success`, `over → text-error` (`:121`). For a revenue line that
reads: earned more than target ⇒ `remainder < 0` ⇒ `over` ⇒ red, labelled
"$10,000 over". Earned less ⇒ green, labelled "to go". Exactly inverted.

The second defect has no single line. `summarizeBudget`
(`web/src/lib/reports/budgetSummary.ts:29`) sums the goal over every bucket in
the span while the actual can only run to today, so in June a year-to-date view
reports every expense line as roughly half-spent and every income line as
roughly half-earned. Fixing the colours without fixing this would make every
income line red for eleven months of the year.

### TS: `budgetSummary.ts`

Add, keeping every existing export intact (`barGeometry` is exercised directly
by `budgetSummary.test.ts`):

```ts
/** Fraction of the report span that has elapsed as of `asOf`, clamped to [0,1].
 *  1 for a span that has already ended — a finished period is fully paced. */
export function elapsedFraction(from: ISODate, to: ISODate, asOf: ISODate): number;

/** The goal prorated to `fraction` — where a line should be by now. */
export function paceAmount(goal: MixedAmount, fraction: number): MixedAmount;

export type BudgetHealth = "healthy" | "behind" | "over";

/** Healthy iff the actual is on the right side of pace: at/above for revenue,
 *  at/below for expense. `over` is the expense-only third state: past the GOAL,
 *  not merely past pace, which is the one an envelope user must not miss. */
export function budgetHealth(actual: number, pace: number, goal: number, income: boolean): BudgetHealth;
```

`elapsedFraction` is `(asOf − from + 1) / (to − from + 1)` over the inclusive
span. It needs a day difference, and **`web/src/lib/reports/periods.ts` does not
have one** — it stops at `monthsBetween:173` and `compareISO:186`, while the
Rust twin has had `days_between` since `periods.rs:376`. Port it, mirroring the
Rust implementation exactly (both are Hinnant civil-day arithmetic over ISO
strings, deliberately clock-free), with its own unit test across a leap day and
a year boundary. It is a sibling module, so the purity rule is satisfied.

Rewrite `BarGeometry` to carry the pace mark and drop the two-colour split:

```ts
export interface BarGeometry {
    /** 0..100 — width of the fill. One colour; `health` says which. */
    fillPct: number;
    /** 0..100 — position of the goal marker (the high-contrast line). */
    markerPct: number;
    /** 0..100 — position of the pace mark, or null when pace === goal. */
    pacePct: number | null;
    /** spent / budget as a fraction, or null when there is no positive budget. */
    ratio: number | null;
}
```

Keep the `scaleMax = max(spent, budget × 1.25)` rule from
`barGeometry:120` — it is what guarantees the goal marker is always on screen
with headroom, and nothing about pace changes that argument.

### UI: `BudgetSummary.svelte`

- Take a new `asOf: ISODate` prop, passed from `routes/budget/+page.svelte`
  (which already computes `span`).
- `toBar` computes `pace` from the section's own `from`/`to` and classifies with
  `budgetHealth`, using the `income` flag it already derives at `:64`.
- Fill colour: `bg-success` when `healthy`, `bg-error` otherwise. The label takes
  the same colour.
- The pace tick renders only when `pacePct !== null` and differs from
  `markerPct` by more than a hair, styled quieter than the goal line
  (`bg-base-content/35`, half height) and carrying a `title` naming it, because
  an unlabelled second tick on a bar is a puzzle.
- Label vocabulary, replacing the current three strings:

  | Type    | Situation                    | Label                  | Colour |
  |---------|------------------------------|------------------------|--------|
  | Expense | at or under pace             | `$X left`              | green  |
  | Expense | past pace, still under goal  | `$X ahead of pace`     | red    |
  | Expense | past goal                    | `$X over`              | red    |
  | Revenue | at or past goal              | `$X above target`      | green  |
  | Revenue | at or past pace, under goal  | `$X to go`             | green  |
  | Revenue | short of pace                | `$X behind pace`       | red    |

  "on plan" survives for the exact-zero case in either column.
- The section header bar (`overall`, `:99`) is classified the same way from the
  section totals, so the summary line and the rows it summarises cannot
  disagree.

### Tests

- `budgetSummary.test.ts`: `elapsedFraction` at the span start, mid-span, past
  the end, and for a single-day span; `budgetHealth` across all six rows of the
  table above plus both zero-boundary cases; `barGeometry` keeping its headroom
  property and placing the pace mark.
- New `BudgetSummary.svelte.test.ts` (component): mount with a frozen
  `today()` and a revenue line at 130% of target — assert the label text and
  that the fill carries the success class, which is the regression this whole
  phase exists for. Assert the mirror case for an expense at 130%.

---

## Phase 2 — The gaps below the bars

### Rust: `crates/ledgeline-core/src/reports/budget.rs`

A new public function beside `budget_report`, reusing its `budgeted` set and
`remap_account` verbatim:

```rust
/// One unbudgeted category: an account with activity that no selected rule
/// budgets, clipped to `depth` and split by resolved type.
pub struct GapRow {
    pub account: String,
    pub depth: usize,
    pub total: MixedAmount,
}

pub struct BudgetGaps {
    pub revenue: Vec<GapRow>,
    pub expense: Vec<GapRow>,
    /// Inclusive span the figures cover, echoed so the UI cannot mislabel them.
    pub from: String,
    pub to: String,
}

pub fn budget_gaps(
    txns: &[Transaction],
    rules: &[PeriodicTransaction],
    declared: &BTreeMap<String, AccountType>,
    opts: &BudgetOpts,
) -> Result<BudgetGaps, ReportError>;
```

Mechanics, in order:

1. Build `budgeted` exactly as `budget_report:211` does — from *all* rules,
   not the `DESCPAT`-selected subset, because the question is "does my budget
   mention this", not "is it in the filtered view".
2. One pass over postings inside `[bucket_start(buckets[0]), opts.end]`. Skip
   any posting whose account does not remap to `UNBUDGETED`; those are covered.
3. Resolve the effective type with `resolve_account_type(account, declared)` and
   keep only `Revenue` and `Expense`. This is the step that removes the cash
   legs, and it is the step most likely to be got wrong later — see
   [[account-type-not-name]]: the failure mode is a section that reads zero, not
   one that reads wrong.
4. Clip to `opts.depth` with the existing `clip`, accumulate, `drop_zeros`.
5. Sort each list by descending magnitude of the primary commodity, ties broken
   by account name so the order is total and the golden is stable. Revenue
   magnitudes are compared on the absolute value, since they are credit-normal.

Rows are *not* rolled up: every row is a distinct unbudgeted subtree clipped to
the same depth, so they sum to the section total without double counting.

### Wire: `GET /api/budget/gaps?end=&interval=&count=&depth=`

- `crates/ledgeline-server/src/reports_api.rs`: a `GapsQuery` beside
  `BudgetQuery:1951`, resolved through the shared `Window::resolve:1843` so the
  bars and the gaps cannot be given different windows. `WireGapRow` /
  `WireBudgetGaps` with `wire_mixed:110`, handler wrapped in `compute:2044`.
- `crates/ledgeline-server/src/lib.rs`: register beside the budget report at
  `:630` — **above the `route_layer` at `:800`**, or it ships unauthenticated
  (the file says so at `:655`).
- Golden: append `budget-gaps` to `fixtures/native/v1/requests.tsv` with pinned
  dates, run `just snapshot-native`, review and commit the JSON.

Deliberately a separate endpoint rather than a new field on `GET /api/budget`:
the budget report is refetched on every control change and on every goal save
(`budgetStore.afterWrite`), and a collapsed section should not make any of that
slower. Plan 22 calls `budget_gaps` in-process, Rust to Rust, over a different
window — which is the other reason it is a function first and an endpoint
second.

### SPA

- `web/src/lib/api/native.ts`: `budgetGaps(query)` returning `Promise<unknown>`,
  doc-commented with its decoder's name, as every sibling is.
- `web/src/lib/api/nativeDecode.ts`: `RawGapRow`/`RawBudgetGaps` + `decodeBudgetGaps`
  under its own banner. `decodeMixed` throws on an absent value — do not default
  it to zero (DRY-3).
- `web/src/lib/budget/budgetStore.svelte.ts`: a fourth `createResource`, `gaps`,
  loaded **only when the section is open** — the disclosure state gates the
  fetch, which is the whole point of defaulting it collapsed.
- `web/src/lib/budget/ui/BudgetGaps.svelte`: a daisyUI `collapse`, closed by
  default, open state persisted in `settings.svelte.ts` beside `insightsOpen:33`
  (one flag; the two lists inside share it). Header states the count and the
  total so the collapsed row is still informative — "12 categories, $41,208 not
  budgeted". Each row links into the journal with `openJournal` for the same
  span, exactly as a bar does.

### Tests

- `budget.rs` unit tests: an unbudgeted expense subtree appears; a budgeted one
  does not; the cash leg of an unbudgeted transaction is excluded by type; a
  non-English/`cogs:`-rooted chart declared `; type: X` classifies correctly
  (the regression guard that the golden fixtures structurally cannot catch —
  add the case to `fixtures/account-types/non-english.journal`'s test).
- `crates/ledgeline-server/tests/budget_endpoints.rs`: the body, a 400 on a bad
  interval, and that the route 401s without the token.
- `nativeDecode.test.ts`: decode the new golden; `without(obj, "total")` proves
  the decoder notices an absent field rather than defaulting it.

---

## Phase 3 — Sorting

### TS: `web/src/lib/budget/sort.ts` (new, not purity-guarded)

```ts
export type BudgetSortKey = "account" | "amount";
export type SortDir = "asc" | "desc";

/** Goal magnitude normalised to a year, so periods compare. */
export function annualised(amount: MixedAmount, period: string): MixedAmount;

export function sortBudgetLines<T>(items: readonly T[], key: BudgetSortKey, dir: SortDir, of: (t: T) => {account: string; amount: MixedAmount; period: string}): T[];
```

Annualisation factors: daily × 365, weekly × 52, monthly × 12, quarterly × 4,
yearly × 1. An unrecognised period (a `~ every 2 weeks` rule, after plan 21)
sorts last within its direction rather than guessing a factor — the editor
already shows such rules read-only and this keeps that honesty.

Amount sorting on the **bars** uses the bar's own goal, which is already the
span total and needs no annualisation; sorting in the **editor** uses the goal's
`entry` magnitude and does. Both sort on the primary commodity's magnitude;
multi-commodity goals sort last.

### UI

- `web/src/lib/budget/params.ts`: add `sort: BudgetSortKey` and `dir: SortDir`
  to `BudgetParams:82`, `defaultBudgetParams:95` (`account` / `asc` — which is
  what the bars already do and what the editor does not), and both halves of the
  URL codec at `:101`/`:112`, validating against the unions the way `depth` is
  validated.
- A small `join` control in the existing controls bar in
  `routes/budget/+page.svelte:247`, beside the depth slider.
- `BudgetSummary.svelte` sorts `section.bars`; `BudgetEditor.svelte` sorts
  `Group.rows` within each period group (`:groups`) and `otherRows`. The
  grouping by period is not disturbed — it is the organising idea and the sort
  operates inside it.

### Tests

- `web/src/lib/budget/sort.test.ts`: both keys, both directions, the
  annualisation table, the multi-commodity and unknown-period tails, and that
  the sort is stable for equal keys.
- `budget/params.test.ts`: the two new params round-trip and a malformed value
  falls back.

---

## Sequencing

```
Phase 1 (pace/colour) ──┐
Phase 3 (sorting)  ─────┼──> all three touch BudgetSummary.svelte
Phase 2 (gaps)     ─────┘    and routes/budget/+page.svelte
```

Run **sequentially, one agent**. All three phases edit
`BudgetSummary.svelte` and `routes/budget/+page.svelte`; three agents in
parallel would spend more time reconciling than working. Phase 1 first — it is
the actual bug.

This plan has no dependency on plans 21 or 22 and can land before either.

## Where the code is

| Path | Purpose |
|---|---|
| `crates/ledgeline-core/src/reports/budget.rs` | `budget_gaps` beside `budget_report`; reuses `budgeted`, `remap_account`, `clip` |
| `crates/ledgeline-server/src/reports_api.rs` | `GapsQuery`, `WireBudgetGaps`, handler |
| `crates/ledgeline-server/src/lib.rs` | route registration, above `route_layer` |
| `web/src/lib/reports/budgetSummary.ts` | `elapsedFraction`, `paceAmount`, `budgetHealth`, reshaped `BarGeometry` |
| `web/src/lib/reports/ui/BudgetSummary.svelte` | one-colour fill, pace tick, new label vocabulary, sort |
| `web/src/lib/budget/sort.ts` | the sort comparator and annualisation |
| `web/src/lib/budget/params.ts` | `sort`/`dir` in the URL |
| `web/src/lib/budget/ui/BudgetGaps.svelte` | the collapsed section |
| `web/src/lib/budget/ui/BudgetEditor.svelte` | sort within each period group |
| `web/src/lib/budget/budgetStore.svelte.ts` | the `gaps` resource, gated on the disclosure |
| `docs/budget.md` | § "Reading the bars" rewritten for pace; a new § for gaps |

## Testing

| Level | Covers |
|---|---|
| `budget.rs` unit tests | the gap remap, the type filter, the depth clip, the sort order |
| `tests/account_types.rs` | a `cogs:`-rooted, non-English chart still produces gap rows |
| `tests/budget_endpoints.rs` | the gaps body, a 400, and the token guard |
| `budgetSummary.test.ts` | `elapsedFraction`, all six health rows, bar geometry with a pace mark |
| `BudgetSummary.svelte.test.ts` | a revenue line at 130% of target is green and says "above target" |
| `budget/sort.test.ts` | both keys, both directions, annualisation, the unsortable tails |
| `budget/params.test.ts` | `sort`/`dir` round-trip and fallback |
| `nativeDecode.test.ts` | the gaps golden decodes; an absent `total` throws |
| `e2e/budget.e2e.ts` | the gaps section is present and starts collapsed; opening it fetches |

Deliberately not tested: the exact pixel position of the pace tick. jsdom has no
layout engine (`web/README.md:64-78`), and the geometry it derives from is
already covered as arithmetic in `budgetSummary.test.ts`.

## Definition of done

- `just engine-check`, `just engine-test`, `just check`, `just test`, `just lint`,
  `just e2e` green.
- `just snapshot-native` re-run and the new golden committed.
- New behaviour has a test that failed before the change landed.
- `docs/budget.md` amended; the three TODO entries deleted.
- Any contract in this doc that changed during implementation is amended here in
  the same commit, per `plans/00-overview.md` convention #9.
