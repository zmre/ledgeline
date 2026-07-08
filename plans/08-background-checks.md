# WP-08: Background Checks & Live Updates

Read `plans/00-overview.md` first, then WP-02/WP-03 contracts (journal store, domain types).

## Scope

LSP-style background attention flags plus the polling loop that keeps data live: periodic refetch with cheap change detection, pure check rules over the journal, row-level flags in the table, a problems badge in the navbar, and a problems drawer listing flagged records.

## Out of scope

Fixing/editing records (post-MVP). Server push (API has no websockets; polling only).

## Interface contracts

### `web/src/lib/checks/engine.ts` + `rules.ts` (pure)

```ts
export type Severity = "error" | "warning" | "info";
export interface Problem { txnIndex: number; rule: string; severity: Severity; message: string }
export interface CheckRule { id: string; run(txns: Transaction[]): Problem[] }
export function runChecks(txns: Transaction[], rules?: CheckRule[]): Problem[];  // defaults to ALL_RULES
export const ALL_RULES: CheckRule[];
```

MVP rules (`rules.ts`):

- `unbalanced` (error): postings don't sum to zero per commodity (respect elided amounts — a single amountless posting absorbs the remainder and is fine; two+ amountless postings or nonzero residue = problem)
- `pending` (warning): `status === "pending"`
- `uncategorized` (warning): any posting to `*:unknown`, `*:uncategorized`, or a bare top-level `expenses`/`income` with no subaccount
- `missing-description` (info): empty description
- `future-date` (info): `txn.date > today()`

Rules are pure and unit-tested; adding a rule = one object in `ALL_RULES`.

### Polling (extend `journal.svelte.ts` per its WP-03 seam)

```ts
export function startPolling(intervalMs?: number): () => void;  // default 30_000
```

- Pause when `document.visibilityState === "hidden"`; resume + immediate refresh on visible
- Cheap fingerprint before swapping state: `txns.length` + max `tindex` + last txn date from the fresh fetch vs current; if unchanged, discard (avoids re-running every `$derived` with 10k fresh objects)
- On fetch error: keep stale data, set `journal.status = "error"`, surface a reconnect affordance (links back to ServerSetupModal)

### Store

`web/src/lib/stores/problems.svelte.ts`: `$derived.by` over `journal.txns` → `runChecks` result, plus `byTxn: Map<number, Problem[]>` for row lookup. Debounce/idle-schedule if check time exceeds a few ms on large journals (`requestIdleCallback`).

## UI

- **`ProblemsBadge.svelte`** — navbar indicator (daisyUI `badge` / `indicator`): count colored by max severity; click opens drawer.
- **`ProblemsDrawer.svelte`** — daisyUI `drawer` listing problems grouped by rule; clicking one scrolls the journal table to that txn (needs a `scrollToTxn(index)` export from WP-03's table) and pulses the row.
- **Row flags** — WP-03's `TransactionRow` shows a small severity dot when `byTxn` has entries; tooltip lists messages.
- Connection status dot in navbar (green ready / yellow loading / red error) fed by `journal.status`.

## Key files created

`web/src/lib/checks/{engine,rules}.ts` (+ tests), `web/src/lib/stores/problems.svelte.ts`, `web/src/lib/checks/{ProblemsBadge,ProblemsDrawer}.svelte`, polling additions to `journal.svelte.ts`, row-flag wiring in WP-03 components

## Depends on / parallel

Depends on: WP-02, WP-03. Parallel with: WP-07.

## Definition of done

- Rule unit tests incl. elided-amount balance cases; fixture journal's deliberate problem records (WP-09) all flagged with correct severities
- Edit the fixture journal while `just serve-api` + `just dev` run → change appears within one poll interval; no UI churn when nothing changed (verify via a render count or derived-log during dev)
- Badge/drawer/row flags work in dark theme, mobile + desktop; `just check` + `just test` green
- Commit: `feat: background checks, problems drawer, live polling`
