# WP-04: Filter Bar

Read `plans/00-overview.md` first, then WP-02 contracts (accounts tree, domain types).

## Scope

The filter model + store and the top filter bar UI: date range with quick presets, account-tree multi-select with search, free-text search input, and URL query-param sync. This store is THE contract WP-03/05 consume — publish exactly this shape.

## Out of scope

Applying filters to data (WP-03's derived views), insights (WP-05).

## Interface contracts

### `web/src/lib/stores/filters.svelte.ts`

```ts
export interface JournalFilter {
    from: ISODate | null;              // inclusive
    to: ISODate | null;                // inclusive
    accounts: ReadonlySet<string>;     // selected account names (empty = all)
    query: string;                     // free text, matched against txn.haystack lowercased
    preset?: DatePreset | null;        // which preset produced from/to; null = hand-picked range (added 2026-07-08)
}
export const filters: {
    readonly value: JournalFilter;     // $state backed
    setRange(from: ISODate | null, to: ISODate | null): void;
    applyPreset(p: DatePreset): void;
    toggleAccount(name: string): void; // selecting a parent implies its subtree via accountMatches — do NOT add children individually
    clearAccounts(): void;
    setQuery(q: string): void;
    reset(): void;                     // back to default: current month, no accounts, empty query
};
export type DatePreset = "thisMonth" | "lastMonth" | "last90" | "ytd" | "thisYear" | "lastYear" | "all";
export function presetRange(p: DatePreset, today: ISODate): { from: ISODate | null; to: ISODate | null }; // pure, string math, unit-tested
```

Default on first load: `thisMonth`.

### `web/src/lib/filters/urlSync.ts`

```ts
export function startUrlSync(): () => void;  // filters → ?preset=|?from=&to= + &acct=a,b&q= via debounced replaceState (~250ms); parse URL → store once at startup. Store is source of truth; URL is a projection (no history entries, no loops).
```

URL date semantics (updated 2026-07-08): preset-produced ranges are stored as
the PRESET NAME (`?preset=ytd`) and recomputed against the current day on
restore, so a kept-open or bookmarked "year to date" never pins stale dates.
Hand-picked ranges are still written as an explicit `from`/`to` pair.

## Components (`web/src/lib/filters/`)

- **`FilterBar.svelte`** — horizontal bar on desktop; on mobile collapses to a compact row with a daisyUI drawer/collapse for the account tree. Shows active-filter chips (removable) when filters differ from default.
- **`DateRangePicker.svelte`** — from/to native `<input type="date">` (good mobile UX) + preset buttons/dropdown (`DatePreset` list). All range math via `presetRange`.
- **`AccountTreeSelect.svelte`** — dropdown/popover with search input at top; tree from `buildAccountTree(journal.accountNames)`; checkbox per node; checking a parent selects the subtree (store just the parent name); indeterminate state when only descendants selected; search filters tree to matching nodes with ancestors kept visible. Virtualize only if account count demands it (>1k).
- **`SearchInput.svelte`** — debounced (~150ms) text input with clear button; hint text "description, amount, account, comment…".

## Key files created

`web/src/lib/stores/filters.svelte.ts`, `web/src/lib/filters/{FilterBar,DateRangePicker,AccountTreeSelect,SearchInput}.svelte`, `web/src/lib/filters/urlSync.ts`, tests: `presetRange` edge cases (month ends, year boundary, leap day), urlSync round-trip, tree selection semantics

## Depends on / parallel

Depends on: WP-02. Parallel with: WP-03, WP-05, WP-06.

## Definition of done

- Presets produce correct ranges (unit-tested with fixed `today`, no Date parsing of ISO strings)
- Account tree: search finds deep accounts; parent selection behaves as specified; chips reflect state
- URL reflects filters after edits; loading a URL with params restores state; no history spam
- Bar usable at 375px (drawer) and desktop; `just check` + `just test` green
- Commit: `feat: filter bar with date presets, account tree, search`
