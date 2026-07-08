# WP-02: Domain Types, Money, API Client, Settings

Read `plans/00-overview.md` first (especially the API tables and money/date conventions).

## Scope

The data foundation everyone else builds on: stable domain types, exact money math, the hledger-web API client, the wire-format normalizer, the settings store with first-run server-URL setup, and account tree/aggregation utilities.

## Out of scope

Journal UI, filters UI, reports. No polling loop (WP-08); `journal.svelte.ts` itself is WP-03.

## Interface contracts (implement exactly; consumers code against these)

### `web/src/lib/domain/types.ts`

```ts
export type ISODate = string; // "YYYY-MM-DD", compare lexically
export type TxnStatus = "unmarked" | "pending" | "cleared";
export interface AmountStyle { side: "L" | "R"; spaced: boolean; precision: number; decimalPoint: string; digitGroups: [string, number[]] | null }
export interface Amount { commodity: string; qty: Dec; style: AmountStyle; cost?: { commodity: string; qty: Dec; per: boolean } }
export interface Posting { account: string; amounts: Amount[]; status: TxnStatus; comment: string; tags: [string, string][]; date?: ISODate }
export interface Transaction {
    index: number;            // hledger tindex — stable id within a fetch
    date: ISODate; date2?: ISODate;
    status: TxnStatus; description: string; code: string; comment: string;
    tags: [string, string][]; postings: Posting[];
    haystack: string;         // precomputed lowercase search text (desc+comments+accounts+amounts+commodities)
}
export interface PriceDirective { date: ISODate; commodity: string; price: Amount }
```

### `web/src/lib/domain/money.ts`

```ts
export interface Dec { m: bigint; p: number }                    // mantissa, decimal places
export function dec(m: bigint | number, p: number): Dec;
export function add(a: Dec, b: Dec): Dec;                        // rescales lower-p operand; never rounds
export function sub(a: Dec, b: Dec): Dec;
export function neg(a: Dec): Dec;
export function mul(a: Dec, b: Dec): Dec;                        // p = a.p + b.p (price conversion only)
export function cmp(a: Dec, b: Dec): -1 | 0 | 1;
export function isZero(a: Dec): boolean;
export function toNumber(a: Dec): number;                        // DISPLAY ONLY (charts/export)
export type MixedAmount = Map<string, Dec>;                      // commodity → qty
export function maAdd(a: MixedAmount, b: MixedAmount): MixedAmount; // drops zero entries
export function maNeg(a: MixedAmount): MixedAmount;
export function maIsZero(a: MixedAmount): boolean;
export function formatDec(d: Dec, style: AmountStyle): string;   // rounding happens HERE only
export function formatAmount(a: Amount): string;                 // honors side/spacing/precision/groups
```

### `web/src/lib/domain/accounts.ts`

```ts
export interface AccountNode { name: string; fullName: string; children: AccountNode[] }
export function buildAccountTree(names: string[]): AccountNode[];     // from /accountnames
export function clampAccount(name: string, depth: number): string;    // split(':').slice(0,depth).join(':')
export function accountMatches(selected: string, account: string): boolean; // === or startsWith(sel+':')
export type RootCategory = "asset" | "liability" | "equity" | "revenue" | "expense" | "other";
export function categorize(account: string): RootCategory;            // hledger-convention name matching (assets*, liabilities*, equity*, revenues|income*, expenses*)
```

### `web/src/lib/domain/aggregate.ts`

```ts
export interface PostingFilter { from?: ISODate; to?: ISODate; accounts?: string[]; status?: TxnStatus }
export function accountTotals(txns: Transaction[], f?: PostingFilter): Map<string, MixedAmount>; // one pass, full names
export function rollUp(totals: Map<string, MixedAmount>): Map<string, MixedAmount>;               // adds each into all ancestors (inclusive balances)
export function atDepth(rolled: Map<string, MixedAmount>, depth: number): Map<string, MixedAmount>; // keys with ≤ depth segments
```

### `web/src/lib/api/client.ts`

```ts
export class ApiUnreachableError extends Error {}   // network/CORS — setup modal shows launch command
export class ApiShapeError extends Error {}         // unexpected JSON
export class HledgerApi {
    constructor(baseUrl: string);
    version(): Promise<string>;
    transactions(): Promise<unknown>;               // raw; normalize separately
    accountNames(): Promise<string[]>;
    prices(): Promise<unknown>;
    commodities(): Promise<string[]>;
}
```

### `web/src/lib/api/normalize.ts`

```ts
export function normalizeTransactions(raw: unknown): Transaction[];
export function normalizePrices(raw: unknown): PriceDirective[];
```

The ONLY file that knows wire field names. Tolerates 1.52 and 2.0-preview (`aprice ?? acost`; `tstatus` string → lowercase enum). Builds Dec from `decimalMantissa`/`decimalPlaces` with `Number.isSafeInteger` guard (throw `ApiShapeError` naming the transaction). Computes `haystack` here (once). Freeze objects (`Object.freeze`) — domain data is immutable.

### `web/src/lib/api/types.raw.ts`

Permissive interfaces mirroring hledger JSON; all drift-prone fields optional (`aprice?`, `acost?`, `aismultiplier?`, `acostbasis?`). Not exported outside `lib/api/`.

### `web/src/lib/stores/settings.svelte.ts`

```ts
export const settings: {
    serverUrl: string | null;          // $state, localStorage "ledgeline.settings.v1"
    columns: ColumnConfig;             // journal table column toggles (defaults per WP-03)
    insightsOpen: boolean;
    setServerUrl(url: string): Promise<void>;  // verifies GET /version before persisting
};
```

### `web/src/lib/components/ServerSetupModal.svelte`

Rendered by `+layout.svelte` whenever `settings.serverUrl === null` (or a connection error flags reconfiguration). Input for URL (default `http://127.0.0.1:5000`), Verify button hitting `/version`, on `ApiUnreachableError` show copyable launch command: `hledger-web -f YOUR.journal --serve-api --cors='*' --allow=view`. daisyUI `modal`, mobile-friendly.

## Key files created

`web/src/lib/domain/{types,money,accounts,aggregate}.ts`, `web/src/lib/api/{types.raw,client,normalize}.ts`, `web/src/lib/stores/settings.svelte.ts`, `web/src/lib/components/ServerSetupModal.svelte`, unit tests beside each pure module (`money.test.ts`, `aggregate.test.ts`, `normalize.test.ts`)

## Depends on / parallel

Depends on: WP-01. Parallel with: WP-09 fixture authoring (use `fixtures/api/v1.52/transactions.json` snapshot in normalize tests once it exists; hand-roll minimal raw JSON literals until then).

## Definition of done

- `just check` + `just test` green; money tests cover align/add/sub round-trips, mul precision, formatting per style (side L/R, spacing, digit groups incl. comma-decimal), safe-integer guard
- `normalizeTransactions` handles a 1.52-shaped sample AND a 2.0-shaped sample (aprice vs acost) in tests
- With `just serve-api` running (needs WP-09 fixture; else a scratch journal), the setup modal verifies and persists the URL, layout shows connected state
- Commit: `feat: domain types, exact money math, hledger api client`
