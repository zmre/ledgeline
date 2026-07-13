# WP-10: Stock Holdings Tab

Read `plans/00-overview.md` first. Contracts referenced: WP-06 `PriceDb`/`valueAt` (`lib/reports/prices.ts`), WP-08 check engine (`lib/checks/engine.ts`), WP-04 `AccountTreeSelect`/urlSync.

## Scope

A third top-level route `/holdings` ("Holdings" in the navbar, between Journal and Reports). Layout mirrors the journal page: controls + insight section on top, details table below.

- **Top controls:** account scope chooser (include OR exclude mode) + as-of date (defaults to today on every fresh visit, never persisted to localStorage).
- **Insight section:** pie chart of current holdings by market value; stat tiles (market value, cost basis, unrealized gain $ and %); top-5 gainers and top-5 losers lists.
- **Details table:** one row per held stock — name, symbol, shares, basis, current price, price as-of date, market value, unrealized gain %.
- **Checks:** three new journal-wide rules in the problems drawer (missing basis, negative quantity, unpriced), plus inline scope-local warnings on the page.
- Pure engine in `lib/holdings/` (no Svelte/DOM — ports to Rust later, and directly enables the post-MVP "holdings over time" chart by mapping `computeHoldings` over a date series).

## Out of scope

Holdings-over-time charts (post-MVP; the engine signature is designed for it), lot-level/FIFO cost tracking (we use average cost), realized gains, xlsx export of the holdings table (can reuse WP-07's exporter later), chained price conversion (WP-06 TODO stands).

## Definitions & semantics

**Stock** = a commodity that is not a currency. `isCurrency()` matches ISO-4217 codes (three uppercase letters from a bundled list — EUR, USD, GBP, …) and symbol glyphs (`$`, `€`, `£`, `¥`, `US$`, `C$`, `A$`, `CHF`, …). Everything else with a nonzero balance in scope is a stock (AAPL, VTI, GLD, …).

**Scope** = accounts passing the include/exclude filter. Include mode with empty set = all accounts (same as journal). Matching uses `accountMatches` subtree semantics.

**Shares as of `asOf`** = per symbol, the sum of posting quantities in that commodity across in-scope postings of txns dated ≤ `asOf`. One pool per symbol across the whole scope (NOT per account): an in-scope→in-scope transfer nets to zero shares and zero basis impact, so cost-less transfers between own accounts stay harmless.

**Basis (average cost, in the valuation base commodity):**
- Buy (qty > 0) with cost annotation: basis += `@` per-unit × qty, or `@@` total.
- Sell (qty < 0): basis −= |qty| × current average cost (basis/shares before the sell).
- Buy with NO cost annotation: the pool is tainted — `basis: null` for that holding, flagged (see checks). Never guess a basis from price directives.
- Cost annotations in a commodity other than the base: valued via `PriceDb.lookupIn` at the txn date; if unconvertible, treat as a no-cost lot (taint).

**Price as of `asOf`** = `PriceDb.lookupIn(symbol, base, asOf)`; when no P directive exists, fall back to the latest cost annotation ≤ `asOf` seen in the journal (source `"cost"`, shown as "inferred" in the UI). Neither → unpriced: no market value, warning.

**Base commodity** = `PriceDb.baseCommodity() ?? "$"`.

**Row filtering:** only current holdings — shares > 0. Shares == 0 (fully sold) rows are dropped silently. Shares < 0 rows are dropped from table AND pie but reported (inline + problems drawer): negative quantity means the opening position was never entered.

**Name** = the value of a `name:` tag found on a posting holding the symbol (posting tags first, then its transaction's tags), scanning txns dated ≤ `asOf`; the latest occurrence wins. Fallback: the symbol itself.

## Interface contracts

```ts
// web/src/lib/holdings/types.ts  (pure TS)
export interface HoldingsScope {
    accounts: ReadonlySet<string>; // subtree roots, same invariant as JournalFilter
    mode: "include" | "exclude";   // include + empty set = everything
    asOf: ISODate;
}

export interface Holding {
    symbol: string;
    name: string;                       // name: tag, else symbol
    accounts: string[];                 // in-scope accounts currently holding shares
    shares: Dec;                        // > 0 by construction
    basis: Dec | null;                  // null = tainted (some lot lacks a cost)
    price: {qty: Dec; date: ISODate; source: "directive" | "cost"} | null;
    marketValue: Dec | null;            // shares × price, null when unpriced
    gain: Dec | null;                   // marketValue − basis, null when either is missing
    gainPct: number | null;             // display-boundary number; null when basis missing/zero
}

export interface HoldingsWarning {
    symbol: string;
    kind: "missing-basis" | "negative-shares" | "unpriced";
    message: string;
}

export interface HoldingsReport {
    asOf: ISODate;
    base: string;
    holdings: Holding[];                // shares > 0, sorted market value desc (unpriced last, by symbol)
    totals: {marketValue: Dec; basis: Dec | null; gain: Dec | null; gainPct: number | null};
    topGainers: Holding[];              // ≤ 5 by gainPct desc, gainPct != null only
    topLosers: Holding[];               // ≤ 5 by gainPct asc, gainPct != null only
    warnings: HoldingsWarning[];        // scope-local, rendered inline on the page
}

// web/src/lib/holdings/engine.ts
export function computeHoldings(txns: Transaction[], prices: PriceDirective[], scope: HoldingsScope): HoldingsReport;

// web/src/lib/holdings/commodities.ts
export function isCurrency(commodity: string): boolean;

// web/src/lib/stores/holdings.svelte.ts — runes store: scope $state
// (asOf initialized from localToday() on module load, never persisted),
// $derived HoldingsReport wired to journal.txns/journal.prices.
```

Totals note: `totals.basis`/`gain`/`gainPct` are null if ANY included holding is tainted or unpriced — a partial total silently understates, so refuse instead and let the inline warning explain (holdings that are merely unpriced are excluded from `totals.marketValue` too; the warning says so).

### Check rules (problems drawer, journal-wide at `today()`, unscoped)

Engine contract change (flagged per convention #9): `CheckRule.run(txns, ctx)` where `ctx: {prices: PriceDirective[]}` — update `engine.ts`, the five existing rules (ignore ctx), and WP-08 doc note. New rules in `ALL_RULES`:

- `stock-missing-basis` (warning) — a currently-held stock has ≥1 cost-less acquisition lot; `txnIndex` = the offending buy.
- `stock-negative` (warning) — net shares < 0 at today; `txnIndex` = the txn that took the pool negative; message says "opening position likely never entered".
- `stock-unpriced` (warning) — a currently-held stock has no P directive and no cost annotation to price it; `txnIndex` = latest txn touching the symbol.

## UI behavior

- **Nav:** "Holdings" link in `+layout.svelte`, `menu-active` on `/holdings`.
- **Scope chooser:** reuse `AccountTreeSelect` fed only with accounts that ever hold a stock commodity, plus an include/exclude mode toggle (daisyUI `join` buttons: "Only" / "All except"). Changing mode keeps the selection.
- **As-of:** date input, default `localToday()` on every visit; picking a date recomputes everything (pure derived, no refetch). URL-synced (`?asof=`, `?acct=`, `?mode=`) via the WP-04 replaceState pattern so reload/share works — but absent params always mean *today*, never a remembered date.
- **Pie** (`dataviz` skill BEFORE writing chart code; LayerChart like WP-05): slice per symbol by `toNumber(marketValue)`, top 9 + "other" bucket when more, legend with symbol + % share, tooltip with name + formatted value. Unpriced holdings are excluded from the pie (inline warning covers them).
- **Stat tiles** (daisyUI `stats`, style of WP-05 BigNumbers): Market value | Cost basis | Unrealized gain $ (with sign/color) | Unrealized gain % — em-dash when null.
- **Gainers/losers:** two compact lists (≤5 each) beside/below the pie: symbol, gain %, gain $; green/red per sign; hidden when fewer than 2 priced holdings.
- **Table:** columns Name | Symbol | Shares | Basis | Price (+ "inferred" badge when `source === "cost"`) | Price date | Market value | Gain % — right-aligned numerics via `formatDec`/`formatAmount` (2dp display cap), negatives `text-error`, sticky header, default sort market value desc, horizontal scroll at 375px. Em-dash for null cells.
- **Inline warnings:** one daisyUI `alert alert-warning` block above the table listing scope-local `warnings` (missing basis, negative dropped, unpriced) — mirrors drawer content but respects scope/asOf.
- **Empty state:** friendly "no stock holdings in scope" card when the report is empty.

## Fixture & test plan

`fixtures/sample.journal` changes (load the `hledger` skill first):

- `name: Apple Inc.` tag on the AAPL buys.
- New stock **VTI** (with `name: Vanguard Total Market`): 2 buys with `@` costs, 1 partial sell, P directives — exercises average-cost math.
- Fully-sold stock (buy then sell all) — must NOT appear in holdings.
- **GLD**: acquired WITHOUT a cost annotation → missing-basis warning, null basis row.
- Deliberate negative: small sell of a never-bought symbol → `stock-negative`, hidden row.
- A held stock with no P directive and no usable cost → `stock-unpriced` (GLD can double up here if it also has no P line).

Ripple effects (MUST be handled in the same commit): regenerate golden fixtures (`just golden`), update the pinned facts in `web/e2e/smoke.e2e.ts` (txn count, problem count, balance-sheet strings) and the fixture-facts comments in plans/09 — new stock rows change Total Assets commodity lines.

Tests:

- Unit (vitest, pure): average-cost basis incl. partial sell; transfer within scope nets zero; exclude-mode scoping; taint propagation to totals; price fallback to cost; asOf time travel (shares/price/name all respect it); top gainers/losers ordering; isCurrency table; the three check rules.
- Golden-ish: AAPL numbers cross-checked against `hledger -f fixtures/sample.journal bal assets:broker cur:AAPL -e <date+1>` and `-V` valuation.
- E2E: `/holdings` renders AAPL row with known shares/value at the pinned clock (2026-07-08); exclude an account and totals shrink; as-of set to 2025-01-01 changes shares; problems badge includes the new deliberate warnings.

## Key files created

`web/src/routes/holdings/+page.svelte`, `web/src/lib/holdings/{types,engine,commodities}.ts`, `web/src/lib/holdings/ui/{HoldingsPie,HoldingsStats,GainersLosers,HoldingsTable,ScopeBar}.svelte`, `web/src/lib/stores/holdings.svelte.ts`, plus edits to `+layout.svelte`, `lib/checks/{engine,rules}.ts`, `fixtures/sample.journal`, `web/e2e/`.

## Depends on / parallel

Depends on: shipped MVP (WP-02/04/05/06/08 contracts). Internally: engine + fixture first (lane A), UI second (lane B) — B can build against the `HoldingsReport` contract with a stub store.

## Definition of done

- Fixture: holdings table shows AAPL + VTI with basis/price/value matching hledger CLI valuation; GLD row shows null basis; sold and negative symbols absent; problems drawer gains the deliberate warnings
- As-of always opens at today; historical dates recompute shares, prices, names, warnings with no refetch
- Include AND exclude scoping affect pie, stats, gainers/losers, table, and inline warnings identically
- `just check` green, unit + golden + e2e green (with re-pinned constants), prettier clean, works at 375px and desktop, dark theme
- Commits (conventional, ≤50-char subjects): `feat: holdings engine and check rules` → `feat: holdings tab ui` → `test: holdings fixtures and e2e`
