> **Superseded:** describes the original read-only SPA build; the app is now a Rust single-binary engine + GUI — see `docs/development.md` and the repo README.

# WP-03: Journal View

Read `plans/00-overview.md` first, then WP-02 contracts (domain types, aggregate, settings).

## Scope

The main `/` route: journal store, virtualized transaction table with configurable columns, and the pinned totals footer. Consumes the filter store contract from WP-04 (build against a stub with the exact exported shape if WP-04 hasn't landed).

## Out of scope

Filter bar UI (WP-04), insights panel (WP-05), background checks/polling (WP-08 — but leave the seam: a `refresh()` on the store).

## Interface contracts

### `web/src/lib/stores/journal.svelte.ts`

```ts
export const journal: {
    txns: Transaction[];               // $state, normalized, frozen
    accountNames: string[];
    prices: PriceDirective[];
    status: "idle" | "loading" | "ready" | "error";
    error: string | null;
    fetchedAt: number | null;
    refresh(): Promise<void>;          // full refetch: /transactions, /accountnames, /prices
};
```

`refresh()` is called on startup (once `settings.serverUrl` is set) and by WP-08's poller. Swap `$state` only after successful normalize; keep old data visible on error with an error toast.

### Filtered view (lives here, consumes filters store)

```ts
// in journal.svelte.ts or a small derived module:
export function getFilteredTxns(): Transaction[];   // $derived.by over journal.txns + filters (date range, selected accounts, query vs haystack)
export function getFilteredTotals(): MixedAmount;   // sum of postings in selected accounts within filtered txns
```

Filter semantics: date range inclusive on both ends against `txn.date`; account selection matches a txn if ANY posting matches ANY selected account (`accountMatches`); empty selection = all accounts; query is case-insensitive substring against `txn.haystack`. Totals sum only postings whose account matches the selection (all postings when selection empty).

## Components (`web/src/lib/journal/`)

- **`TransactionTable.svelte`** — virtualized list (simple windowing over a scroll container or `@tanstack/virtual-core`; must stay smooth at 50k rows). daisyUI `table` styling, sticky header. Sorted by date desc, then `index` desc.
- **`TransactionRow.svelte`** — renders configured columns. Defaults: Date, Status, Description, Accounts, Amount.
- **`StatusBadge.svelte`** — cleared `*` / pending `!` / unmarked, as compact daisyUI badges.
- **`AccountsCell.svelte`** — from→to arrow chips: within a txn, postings with net-negative MixedAmount are sources, net-positive are destinations; render `source → dest`. Degrade to a plain wrapped account list for N-way splits (>2 distinct sides). Truncate long account names with tooltip on hover.
- **`AmountCell.svelte`** — `formatAmount` per commodity, right-aligned, one line per commodity, negative styled (e.g. `text-error`).
- **`CommentIndicator.svelte`** — small corner icon when txn or posting comments exist; hover (desktop) / tap (mobile) shows comment text in a daisyUI tooltip/popover.
- **`TotalsFooter.svelte`** — pinned bottom bar: MixedAmount totals from `getFilteredTotals()`, txn count, period label.
- Column config UI: small dropdown (gear icon) toggling columns; persists via `settings.columns`.

Mobile: at narrow widths collapse to a card-per-transaction layout (date + description + amount prominent, accounts below) rather than a squeezed table.

## Route wiring

`src/routes/+page.svelte`: mounts FilterBar (WP-04 slot/placeholder), InsightsPanel (WP-05 placeholder), TransactionTable, TotalsFooter. On mount: if `settings.serverUrl` set → `journal.refresh()`.

## Key files created

`web/src/lib/stores/journal.svelte.ts`, `web/src/lib/journal/{TransactionTable,TransactionRow,StatusBadge,AccountsCell,AmountCell,CommentIndicator,TotalsFooter}.svelte`, updated `src/routes/+page.svelte`, unit tests for filter/totals derivations and the source/destination heuristic (pure helpers extracted to `web/src/lib/journal/rowModel.ts`)

## Depends on / parallel

Depends on: WP-02. Parallel with: WP-04, WP-05, WP-06.

## Definition of done

- Against `just serve-api` fixture: table renders all fixture txns, default current-month filter applied, totals footer matches a hand-checked fixture value
- Column toggles persist across reload; comment indicator shows fixture comments; arrows correct for simple 2-posting txns and degrade for splits
- Smooth scroll with a synthetic 50k-txn dataset (add a vitest/bench or manual note)
- `just check` + `just test` green; mobile card layout verified at 375px
- Commit: `feat: journal view with virtualized transaction table`
