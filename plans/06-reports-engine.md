# WP-06: Report Engine (Pure TS)

Read `plans/00-overview.md` first, then WP-02 contracts (domain types, money, aggregate).

## Scope

Pure-TypeScript report computation: period math, balance sheet, income statement, cash flow, net worth with market prices. **Zero Svelte/DOM imports** — this module ports to Rust in a later phase. Everything unit-testable against hledger-CLI golden files (WP-09).

## Out of scope

Any UI (WP-07), xlsx export (WP-07), budget report (post-MVP stretch — leave a TODO section in types).

## Interface contracts

### `web/src/lib/reports/types.ts`

```ts
export interface ReportRow { account: string; depth: number; own: MixedAmount; inclusive: MixedAmount }
export interface Section { title: string; rows: ReportRow[]; total: MixedAmount }
export interface SectionedReport { asOf?: ISODate; from?: ISODate; to?: ISODate; sections: Section[]; grandTotal: MixedAmount }  // bs, is
export interface ReportMeta { unpriced: string[] }  // EXTENSION (WP-06): commodities skipped during valuation (sorted, deduped)
export interface PeriodReport { buckets: string[]; rows: { account: string; depth: number; values: MixedAmount[] }[]; totals: MixedAmount[]; meta?: ReportMeta }  // cf, net worth
```

### `web/src/lib/reports/periods.ts` (pure string date math — NEVER `new Date('YYYY-MM-DD')`)

```ts
export type Interval = "daily" | "weekly" | "monthly" | "quarterly" | "yearly";
export function bucketKey(date: ISODate, interval: Interval): string;        // "2026-07", "2026-Q3", "2026-W28", ...
export function bucketLabel(key: string): string;
export function bucketStart(key: string): ISODate;                          // first date in bucket — EXTENSION (WP-06), companion to bucketEnd
export function bucketEnd(key: string): ISODate;                            // last date in bucket
export function lastNBuckets(end: ISODate, interval: Interval, n: number): string[]; // oldest→newest
export function today(): ISODate;                                            // from local Date parts, ONLY Date usage allowed
export function compareISO(a: ISODate, b: ISODate): -1 | 0 | 1;              // lexical
```

### Report functions

```ts
// balanceSheet.ts — assets + liabilities (equity optional flag post-MVP); balances as of date
export function balanceSheet(txns: Transaction[], opts: { asOf: ISODate; depth: number }): SectionedReport;

// incomeStatement.ts — revenues + expenses over a range; net = revenues - expenses
export function incomeStatement(txns: Transaction[], opts: { from: ISODate; to: ISODate; depth: number }): SectionedReport;

// cashFlow.ts — changes in asset (cash-like) accounts per bucket, last N buckets ending at `end`
export function cashFlow(txns: Transaction[], opts: { end: ISODate; interval: "monthly" | "quarterly" | "yearly"; count: number; depth: number }): PeriodReport;
export function isCashLike(account: string): boolean;  // EXTENSION (WP-06): hledger's Cash-type name heuristic; post-MVP: declared account types from /accounts

// netWorth.ts — assets + liabilities valued at market prices per bucket end
// EXTENSION (WP-06): optional valueIn = valuation target; defaults to prices.baseCommodity(); when null (no directives) balances stay unvalued/mixed.
export function netWorth(txns: Transaction[], prices: PriceDb, opts: { end: ISODate; interval: Interval; count: number; valueIn?: string }): PeriodReport;

// prices.ts — direct conversions only (chain conversion = documented post-MVP TODO)
export interface PriceDb {
    lookup(commodity: string, asOf: ISODate): Amount | null;                          // latest P directive ≤ asOf, any target
    lookupIn(commodity: string, target: string, asOf: ISODate): Amount | null;        // EXTENSION (WP-06): latest ≤ asOf priced directly in `target`
    baseCommodity(): string | null;                                                   // EXTENSION (WP-06): most frequent price commodity (tie → lexical); null if no directives
}
export function buildPriceDb(directives: PriceDirective[]): PriceDb;
export interface ValuationMeta { unpriced: string[] }                                 // EXTENSION (WP-06): valueAt out-param
export function valueAt(ma: MixedAmount, target: string, db: PriceDb, asOf: ISODate, meta?: ValuationMeta): Dec; // convert each commodity via db (mul), identity if same; unpriced commodities are SKIPPED (never guessed) and reported via the optional `meta` out-param; reports surface them as PeriodReport.meta.unpriced
```

Date semantics: `asOf`/`from`/`to`/`end` are all INCLUSIVE; hledger's `-e DATE` is exclusive, so goldens generated with `-e D` compare against `asOf`/`to` = day before D. Sectioned-report presentation matches hledger: liabilities (bs) and revenues (is) rows/totals are sign-flipped positive; bs grandTotal = assets − liabilities(displayed), is grandTotal = revenues(displayed) − expenses. PeriodReport values keep natural signs (net worth liabilities negative).

Section membership via `categorize` (WP-02). Rows: `accountTotals` → `rollUp` → `atDepth`, sorted by account name; `own` = clamped-name direct total, `inclusive` = rolled-up. Sign conventions match hledger reports (assets positive, liabilities typically negative internally — present sign-flipped totals the way `hledger bs` does; verify against goldens).

## Key files created

`web/src/lib/reports/{types,periods,balanceSheet,incomeStatement,cashFlow,netWorth,prices}.ts` + colocated `*.test.ts` (fixture-driven tests expanded in WP-09)

## Depends on / parallel

Depends on: WP-02 only. Parallel with: WP-03, WP-04, WP-05 — **the big parallel lane**.

## Definition of done

- Unit tests green for: bucket math (month/quarter/year/week boundaries, leap years), each report on small inline fixtures
- Golden comparison (with WP-09 fixtures): balance sheet and income statement outputs match `hledger bs/is -O json` derived pairs (compare mantissa/places, not floats) at depths 1–3
- No Svelte/DOM imports anywhere in `lib/reports/` (add an eslint restriction or a test asserting it)
- `just check` + `just test` green
- Commit: `feat: pure-ts report engine (bs/is/cf/net-worth)`
