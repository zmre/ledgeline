---
title: Journal view internals
description: "How the Journal tab reads a large journal a window at a time: the engine routes, their caches, and how the SPA keeps them in step."
order: 45
tags: [development]
---

# Journal view internals

This page is for maintainers. Until plan 26, the SPA downloaded every transaction (`/transactions`, hundreds of megabytes at 200k transactions) and computed the table, search, totals, charts, cash balances and problem checks itself. Now the engine computes them and the SPA asks for exactly what it draws. The logic is in `crates/ledgeline-core/src/view/`; the routes are in `crates/ledgeline-server/src/journal_api.rs`.

## The routes

Every route below is token-guarded and gzip'd like the rest of `/api`, carries the snapshot's `ETag` (a `304` costs nothing), and runs its CPU work on the report pool. Exact decimals travel as `{m: "<mantissa>", p: places}`, the mantissa a string so no digit is lost in JavaScript.

| Route | Answers |
|---|---|
| `GET /api/journal` | Which journal is open, its account names with their types and terms, styles, and the snapshot `generation`. The SPA polls it every 30 seconds; a new generation means every view below is stale. |
| `GET /api/journal/rows?from&to&accounts&q&offset&limit&order&today` | One window of the filtered journal, newest first (`order=asc` reverses it), at most 1,000 rows. Each row is the domain transaction the row components render, with its problem `flags`. Also returns `total`, the whole filtered count. |
| `GET /api/journal/summary?<filter>` | The filtered count, the footer's visible net, each commodity's income/expenses/net with its style and categories, the commodities ranked by use, and the deepest account. |
| `GET /api/journal/chart?<filter>&mode&depth&commodity&category&interval&max` | The insights pie or line for one commodity, plus `groups`, the order colours are assigned from. |
| `GET /api/journal/position?index&<filter>` | Where the transaction with that `tindex` sits in the filtered view, or `null` when the filter hides it, plus its date. |
| `GET /api/transactions/{index}?today` | One transaction as a row: what the edit popup opens on. `GET /api/transactions` itself still answers `405`, because the SPA's "can I edit?" probe relies on that. |
| `GET /api/diagnostics?today&limit` | Every problem: the engine diagnostics, then the five local check rules. `count` and `maxSeverity` cover the whole list even when `limit` truncates it, so the badge asks for `limit=0`. |
| `GET /api/balances/cash?asOf` | The Balances view: cash and short-term liabilities for every commodity, as written. |

The filter means what it always did: the date range is inclusive on the transaction's primary date, a transaction matches the account selection when any posting is in any selected subtree, and the search text is trimmed, lowercased and matched as a substring of each transaction's search text. That text is built with the same display formatter the SPA uses (`display.rs`), so a query typed off the screen finds its row.

`today` is always the browser's local date. Two checks depend on it (future dates, and cash balances as of today), and the server's time zone must not change their answer. Routes that take it extend the `ETag` with the date.

## Caches

Each snapshot (one per load, edit or file change) carries a `ViewCache`. Everything in it is built at most once and dropped with the snapshot, so nothing can go stale:

- the display order, the account classes, the declared styles, the sign conventions, and every transaction's search text;
- the engine diagnostics, plus the local check results for the last four dates asked about;
- the last eight filter results, as positions into the journal.

A background thread warms the cache as soon as the snapshot exists, in the order the first screen needs it, so the first requests after a load or an edit find it ready. A request that arrives first simply builds the piece itself; both share one `OnceLock`.

`/transactions` is kept for hledger-web clients and is byte-for-byte what it was, but it is now serialized on first request rather than at every snapshot build.

## The SPA side

- `stores/journal.svelte.ts` holds the metadata from `/api/journal` and the `generation`.
- `stores/journalView.svelte.ts` follows (server, generation, filter). It holds the summary and a cache of 200-row blocks, fetches the blocks the table's window touches plus one either side, and keeps at most 50 blocks. A filter change drops everything. A generation change keeps the old rows on screen until their fresh copies arrive, so an inline edit doesn't flash the table.
- `TransactionTable.svelte` sizes its scroll area from `total`, draws a skeleton row (no `data-txn`) for any row whose block is still loading, and uses `lazyListCursor`, which tracks a position rather than a key.
- `stores/problems.svelte.ts` fetches the count for the badge, and the full list only while the drawer is open. Rows bring their own flags.

## Goldens

Every computation was ported from TypeScript and is held to output captured from that TypeScript before it was deleted. The goldens are in `fixtures/journal-view/`, and the tests are `cash_balances_golden.rs`, `checks_golden.rs` and `journal_view_golden.rs` in `crates/ledgeline-core/tests/`. A failure means the port drifted. Don't regenerate the goldens from Rust.

Launch and scroll performance at 200k is in [Performance baseline](perf-baseline.md) (section 8).
