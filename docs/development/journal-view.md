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
| `GET /api/journal` | Which journal is open, its account names with their types and terms, styles, and the snapshot `generation`. The SPA polls it every 5 seconds from the root layout, on every screen; a new generation means every view below is stale. |
| `GET /api/journal/rows?from&to&accounts&q&offset&limit&order&today&balance&balanceCommodity` | One window of the filtered journal, newest first (`order=asc` reverses it), at most 1,000 rows. Each row is the domain transaction the row components render, with its problem `flags`. Also returns `total`, the whole filtered count. With `balance=<account>` every row also carries `running` (see [Running balances](#running-balances)); without it the body is byte-for-byte what it was before plan 36. |
| `GET /api/journal/summary?<filter>` | The filtered count, the footer's visible net, each commodity's income/expenses/net with its style and categories, the commodities ranked by use, and the deepest account. |
| `GET /api/journal/chart?<filter>&mode&depth&commodity&category&interval&max` | The insights pie or line for one commodity, plus `groups`, the order colours are assigned from. |
| `GET /api/journal/running?account` | The pinned account's chip and commodity picker: every commodity its subtree posts with a posting count (most first; the first is the default), and per commodity the assertions in the subtree as `{tindex, date, ok}` in hledger's evaluation order. `400` when no posting is in the subtree. |
| `GET /api/journal/running/series?account&commodity&from&to&today&interval` | The pinned account's balance at the end of every period of the journal filter's date range, for the Balances tab's line (see [Running balances](#running-balances)). `400` when no posting is in the subtree, for a backwards range, or for more than 1,200 periods. |
| `GET /api/journal/position?index&<filter>` | Where the transaction with that `tindex` sits in the filtered view, or `null` when the filter hides it, plus its date. |
| `GET /api/transactions/{index}?today` | One transaction as a row: what the edit popup opens on. `GET /api/transactions` itself still answers `405`, because the SPA's "can I edit?" probe relies on that. |
| `GET /api/diagnostics?today&limit` | Every problem: the engine diagnostics, then the five local check rules. `count` and `maxSeverity` cover the whole list even when `limit` truncates it, so the badge asks for `limit=0`. |
| `GET /api/balances/cash?asOf` | The Balances view: cash and short-term liabilities for every commodity, as written. |

The filter means what it always did: the date range is inclusive on the transaction's primary date, a transaction matches the account selection when any posting is in any selected subtree, and the search text is trimmed, lowercased and matched as a substring of each transaction's search text. That text is built with the same display formatter the SPA uses (`display.rs`), so a query typed off the screen finds its row.

### Running balances

Plan 36. `rows?balance=<account>` pins an account. Every row then carries:

```jsonc
"running": {
  "change": RowAmount | null,   // the row's net in the subtree; null when it has no posting there in this commodity
  "balance": RowAmount,         // the subtree's balance after the row
  "touches": true,              // the row has a posting in the subtree in this commodity
  "postedOn": "2026-04-02",     // only when the row's last such posting is posting-dated to another day
  "assertions": [{"account", "op", "asserted": RowAmount, "calculated"?: RowAmount, "ok"}]
}
```

`RowAmount` is the shape posting amounts already use (`{commodity, qty: {m, p}, style}`). The figures are for one commodity: `balanceCommodity`, or the subtree's default (most postings, ties by name) when it's absent or the subtree never posts it, so a stale link still draws a column. An account with no posting in its subtree is a `400`.

The logic is `view/running.rs`. Postings are summed in `assertions::posting_order`, the order the assertion checker evaluates in (posting date, then read order), and the same order `hledger reg` lists them in. `crates/ledgeline-core/tests/running_balance.rs` holds it to `hledger reg -H` output captured into `fixtures/journal-view/errors/running/` by `scripts/gen-golden.sh`. The balance covers the whole journal and the subtree (virtual and generated postings included, costs ignored); filters only choose rows. A row with no posting in the subtree in the chosen commodity shows the balance carried from the last posting keyed at or before its (date, position). The table's tie-break is `tindex` and the stream's is position; they agree because tindexes are assigned in read order, `include`d files at their directive (pinned by a test).

An assertion mark's pass or fail comes from the checker's `AssertionFailure`s, never from comparing with the column: a plain `=` on a child asserts the child alone. A row shows the marks in the chosen commodity, plus any `==` zero-check failure in another commodity.

#### Balance over time

Plan 36 amendment A1. `running/series` answers from the same cached `RunningBalance` as the rows route, so pinning builds it once for both:

```jsonc
{
  "generation": "…", "account": "assets:bank", "commodity": "$",
  "style": WireStyle,            // every figure in the body displays in it
  "interval": "weekly",          // daily | weekly | monthly | quarterly | yearly
  "from": "2026-01-01", "to": "2026-03-01",   // the window, both ends resolved
  "points": [
    {"bucket": "2026-W05", "label": "W05 2026", "end": "2026-02-01",
     "balance": {"m": "540225", "p": 2},
     "tindex": 12,               // the newest transaction counted in the period; absent when none
     "failures": [{"tindex": 11, "date": "2026-01-31", "account": "assets:bank:checking", "op": "=",
                   "asserted": {…}, "calculated": {…}, "expected": {…}}]}   // absent when none
  ]
}
```

`commodity` resolves exactly as `balanceCommodity` does. An open `from` is the subtree's first posting and an open `to` is `today`; an open side never inverts the window. Without `interval` the server uses `holdings::auto_interval`, the Holdings value chart's rule (months from six of them, weeks from six weeks, days below that, quarters past ten years). Periods come from `reports::range_buckets`, and each point is the total after the last posting with an effective date on or before its clamped end, by binary search over the stream's entries. A failure is a failing mark in the commodity whose effective date falls in the period. Its `expected` is the point's balance shifted by `asserted − calculated`, so the gap drawn from the line is the "off by" even for a plain `=` on a child account. `running_balance.rs` holds every daily point to `hledger reg`'s running total and checks, by property test, that each point equals the Balance cell of the newest row on or before its end. The `ETag` is dated by `today`.

`today` is always the browser's local date. Two checks depend on it (future dates, and cash balances as of today), and the server's time zone must not change their answer. Routes that take it extend the `ETag` with the date.

## Caches

Each snapshot (one per load, edit or file change) carries a `ViewCache`. Everything in it is built at most once and dropped with the snapshot, so nothing can go stale:

- the display order, the account classes, the declared styles, the sign conventions, and every transaction's search text;
- the engine diagnostics, plus the local check results for the last four dates asked about;
- the last eight filter results, as positions into the journal;
- the posting evaluation order, built the first time an account is pinned (not prewarmed: most sessions never pin);
- the running balance and assertion marks of the last four pinned roots. One build covers every commodity, so the commodity picker costs nothing.

The engine diagnostics keep the raw `AssertionFailure`s from their one assertion check, and the running-balance marks reuse them; the journal is never checked twice.

A background thread warms the cache as soon as the snapshot exists, in the order the first screen needs it, so the first requests after a load or an edit find it ready. A request that arrives first simply builds the piece itself; both share one `OnceLock`.

`/transactions` is kept for hledger-web clients and is byte-for-byte what it was, but it is now serialized on first request rather than at every snapshot build.

## The SPA side

- `stores/journal.svelte.ts` holds the metadata from `/api/journal` and the `generation`.
- `stores/journalView.svelte.ts` follows (server, generation, filter). It holds the summary and a cache of 200-row blocks, fetches the blocks the table's window touches plus one either side, and keeps at most 50 blocks. A filter change drops everything. A generation change keeps the old rows on screen until their fresh copies arrive, so an inline edit doesn't flash the table.
- `TransactionTable.svelte` sizes its scroll area from `total`, draws a skeleton row (no `data-txn`) for any row whose block is still loading, and uses `lazyListCursor`, which tracks a position rather than a key.
- `stores/runningSeries.svelte.ts` holds the balance line. `InsightsPanel` loads it only while the panel is open on Balances with a pin set, keyed on the pin, the date range, `today` and the generation (not the account filter or search, which don't change it). `BalanceTrend.svelte` draws it through `PeriodLineChart`'s `overlay`, `extent`, `tooltipLines` and `onPeriodClick` extension points.
- `stores/problems.svelte.ts` fetches the count for the badge, and the full list only while the drawer is open. Rows bring their own flags.

## Goldens

Every computation was ported from TypeScript and is held to output captured from that TypeScript before it was deleted. The goldens are in `fixtures/journal-view/`, and the tests are `cash_balances_golden.rs`, `checks_golden.rs` and `journal_view_golden.rs` in `crates/ledgeline-core/tests/`. A failure means the port drifted. Don't regenerate the goldens from Rust.

Launch and scroll performance at 200k is in [Performance baseline](perf-baseline.md) (section 8).
