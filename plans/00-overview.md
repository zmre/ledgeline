# WP-00: Ledgeline Architecture Overview

Reference doc for all work packages. Every implementing agent reads this FIRST, then its own WP doc.

## What we're building

A modern, fast, responsive (mobile + desktop) web GUI for hledger. MVP is **read-only**: a pure static SvelteKit SPA that talks directly to the hledger-web JSON API from the browser. No app server of our own. Server URL is prompted on first run and stored in localStorage.

Two routes:

- `/` — journal view: filterable transaction list (defaults to current month), filter bar (dates, account tree multi-select, free-text search), collapsible insights panel (big numbers + pie/line charts + depth slider), pinned period totals
- `/reports` — balance sheet, income statement (P&L), cash flow, net worth as spreadsheet-style tables with xlsx export

Post-MVP (design for, don't build): transaction editing, Rust journal engine + API server, encrypted p2p-ish sync (rebellion-db), imports, AI.

## Confirmed stack

| Concern         | Choice                                                                      |
|-----------------|-----------------------------------------------------------------------------|
| Framework       | SvelteKit, **Svelte 5 runes**, `@sveltejs/adapter-static`, SPA mode          |
| Language        | TypeScript, strict mode, no `any` without cause                              |
| Pkg manager     | **Bun** (never npm)                                                          |
| Styling         | TailwindCSS v4 + daisyUI v5, **dark theme as default**, responsive-first     |
| Charts          | LayerChart (v2+, Svelte 5 compatible)                                        |
| xlsx export     | exceljs, lazy-loaded via dynamic import                                      |
| Tests           | vitest (unit), playwright (e2e)                                              |
| Dev env         | Nix flake at repo root (`nix develop`), tasks via `justfile`                 |

Run tools only through the dev shell: `nix develop path:. -c bun ...` or via direnv. Never install anything globally.

## Repo layout

```
ledgeline/
├── flake.nix, .envrc, justfile, README.md
├── plans/               # these docs
├── fixtures/            # sample.journal, golden/ (CLI-generated), api/ (raw API snapshots)
├── scripts/             # gen-golden.sh, snapshot-api.sh
└── web/                 # SvelteKit app — created by WP-01
    └── src/
        ├── routes/                  # +layout.svelte, +page.svelte, reports/+page.svelte
        └── lib/
            ├── api/                 # client.ts, types.raw.ts, normalize.ts
            ├── domain/              # types.ts, money.ts, accounts.ts, aggregate.ts
            ├── stores/              # settings.svelte.ts, journal.svelte.ts, filters.svelte.ts
            ├── journal/             # journal-view components (WP-03)
            ├── filters/             # filter-bar components (WP-04)
            ├── insights/            # insights panel + charts (WP-05)
            ├── reports/             # pure report engine (WP-06) + ui/ (WP-07)
            ├── checks/              # background checks (WP-08)
            └── export/              # xlsx.ts (WP-07)
```

Monorepo-ready: later phases add `crates/` (Rust). `fixtures/` lives at root because the future Rust engine tests against the same goldens.

## hledger-web JSON API (verified against hledger 1.52 source)

Launch (dev): `hledger-web -f FILE --serve-api --cors='*' --allow=view` — port 5000, binds 127.0.0.1, JSON API only, read-only, no auth.

| Route                            | Method | Returns                                             |
|----------------------------------|--------|-----------------------------------------------------|
| `/version`                       | GET    | version string                                      |
| `/accountnames`                  | GET    | flat array of all account names                     |
| `/transactions`                  | GET    | ALL transactions (full journal `jtxns`)             |
| `/prices`                        | GET    | market price (`P`) directives                       |
| `/commodities`                   | GET    | array of commodity symbols                          |
| `/accounts`                      | GET    | account tree with balances                          |
| `/accounttransactions/NAME`      | GET    | per-account register                                |
| `/add`                           | PUT    | add one transaction (201 on success) — post-MVP     |
| `/openapi`                       | GET    | OpenAPI 3.1 spec                                    |

**There are NO report endpoints.** All reports are computed client-side from `/transactions` (+ `/prices`).

### Wire shapes (hledger 1.52)

- **Transaction**: `tdate`, `tdate2`, `tdescription`, `tcode`, `tcomment`, `tindex`, `tstatus` (`"Unmarked" | "Pending" | "Cleared"`), `tpostings`, `ttags`, `tprecedingcomment`, `tsourcepos`
- **Posting**: `paccount`, `pamount` (array of Amounts = MixedAmount), `pstatus`, `pcomment`, `ptags`, `pdate`, `pdate2`, `pbalanceassertion`, `ptype` (regular/virtual/balanced-virtual), `poriginal`, `ptransaction_`
- **Amount**: `acommodity`, `aquantity: {floatingPoint, decimalPlaces, decimalMantissa}`, `astyle: {ascommodityside, ascommodityspaced, asprecision, asdecimalpoint, asdigitgroups}`

### Version drift table (CRITICAL)

The JSON is a dump of hledger's internal Haskell types and is **not a stable contract**.
(Corrected 2026-07-08 against a live 1.52 server — earlier draft had the columns backwards:)

| older hledger (pre-1.5x)  | hledger 1.52 (verified live)        |
|----------------------------|-------------------------------------|
| `aprice`                   | `acost` (+ `acostbasis`, may be null)|
| `asdecimalpoint`           | `asdecimalmark` (+ `asrounding`)    |
| `aismultiplier`            | (moved into cost representation)    |

Also: `/prices` on 1.52 returns `MarketPrice` records (`mpdate`/`mpfrom`/`mpto`/`mprate`, no amount style), not full `pd*` price directives. The normalizer tolerates both spellings of every drifted field.

Rule: **only `web/src/lib/api/normalize.ts` may know wire field names.** Raw types are permissive (drift-prone fields optional); the normalizer tolerates both shapes (`aprice ?? acost`) and emits our own frozen domain types. Nothing outside `api/` imports raw types.

## Architecture: one-directional data flow

```
hledger-web JSON
  → api/client.ts        (fetch, error taxonomy)
  → api/normalize.ts     (wire → domain, ONLY place that knows hledger JSON)
  → domain types         (stable, ours)
  → stores/journal.svelte.ts   ($state: txns, accounts, prices; polling refresh)
  → $derived views       (filters store → filteredTxns; insights; totals)
  → lib/reports/*        (pure functions, called with $derived from /reports)
```

## Non-negotiable conventions

1. **Money is exact.** `type Dec = { m: bigint; p: number }` built from `decimalMantissa`/`decimalPlaces`. NEVER use `floatingPoint` for accumulation. `toNumber()` only at chart/export display boundaries. Round only at format time. Multi-commodity totals are `MixedAmount = Map<string, Dec>`. Display precision is capped at 2 decimal places (`formatDec`/`MAX_DISPLAY_DECIMALS`, incl. xlsx number formats) — exact Decs keep full precision internally.
2. **Guard mantissa:** if `!Number.isSafeInteger(decimalMantissa)`, throw `ApiShapeError` / flag the record — never silently fall back to floats.
3. **Dates are ISO strings** (`"YYYY-MM-DD"`) end-to-end, compared lexically. NEVER `new Date('YYYY-MM-DD')` (parses UTC, shifts a day in negative-offset zones). "Today" is computed once from local `Date` parts, then pure string math (`lib/reports/periods.ts`).
4. **Runes state lives in `.svelte.ts` modules** (`$state`/`$derived.by`), not legacy `writable` stores, not component-local globals.
5. **Report engine is pure TS**: `lib/reports/` and `lib/domain/` import zero Svelte/DOM — they port to Rust later.
6. **Account matching:** selected account matches posting when `acct === sel || acct.startsWith(sel + ':')`.
7. **Depth clamping:** `name.split(':').slice(0, depth).join(':')`; depth 1 shows `assets`, `liabilities`, ...; depth 3 shows `assets:morganstanley:checking`.
8. **localStorage keys are versioned:** `ledgeline.settings.v1` etc.
9. **Interface contracts first.** Each WP doc states its exported types/signatures. Implement to the contract so parallel lanes don't block on each other. If you must change a contract, update the WP doc in the same commit and flag it.
10. **Style:** daisyUI components + Tailwind utilities; dark theme default; every view must work at 375px wide and at desktop widths. Prettier config: printWidth 160, tabWidth 4, trailingComma es5, bracketSpacing false, arrowParens always.

## Skills to load (for Claude subagents)

- Always: `ironcore-typescript-javascript`, `nix` (when touching flake/dev-env)
- WP-05 (charts) and any chart work: **`dataviz` skill before writing chart code**
- Anything touching journal files/fixtures or hledger CLI: `hledger` skill

## Execution schedule

```
WP-01 (scaffold, gate)
  → WP-02 (domain+api)  ∥  WP-09 fixtures authoring
    → WP-03 (journal view) ∥ WP-04 (filter bar) ∥ WP-05 (insights) ∥ WP-06 (report engine)
      → WP-07 (reports UI, after 06)
      → WP-08 (checks, after 02+03)
      → WP-09 golden tests (after 06) + e2e (after 03+04)
```

WP-03/04/05 read the filter/journal store contracts from the WP docs and may build against stubs; integration happens as lanes land.

## Risks / gotchas (all WPs)

1. **CORS:** users must launch hledger-web with `--cors`. Detect fetch failure in the setup modal and show the exact launch command to copy.
2. **Mixed content:** an https-hosted SPA cannot call `http://127.0.0.1:5000`. MVP is served locally over http (dev server or `vite preview`).
3. **API drift:** isolated in `normalize.ts` + versioned raw snapshots in `fixtures/api/`.
4. **Large journals:** normalize once (including per-txn lowercase search haystacks), virtualize the table, aggregate before charting. Target: smooth at 50k+ transactions.
5. **Polling churn:** compare a cheap fingerprint (txn count + last `tindex`) before swapping `$state`, or every `$derived` re-runs on every poll.
6. **hledger-web default mode exits after 2 min idle** (`--serve-browse`); always use `--serve-api` (+ `--serve` semantics) in scripts.

## Definition-of-done norms (all WPs)

- `just check` passes (svelte-check + tsc strict, zero errors)
- Unit tests for new pure logic; `just test` green
- No linter/formatter warnings; run prettier
- Works in dark theme at mobile (375px) and desktop widths
- Commit messages: conventional commits, subject ≤ 50 chars
