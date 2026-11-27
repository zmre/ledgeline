# 47 — Holdings charts: honest y-axis, up/down shading, holding filter, intraday

Decided with Patrick 2026-11-26. Branch `2026-11-26-holding-improvements`.

## Goal

The value-over-time charts on Holdings (Stocks and Other) should show change
without inventing drama, say at a glance whether the period was up or down,
narrow to one holding or a group, and — on short periods — answer "the market
is having a bad day, what does that do to me?".

We are not a stock research or monitoring app: no watch lists, no real-time
prices. Intraday is one delayed quote per symbol, held in memory.

## Decisions

| #  | Decision                                                                                                                                                                     |
| -- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| D1 | Y-axis is anchored on a horizon at least as long as the shown period: < 1 year (1wk, 1mo, 3mo, ytd, 12mo) → last 1 year; 5yr → last 5 years; all → all time.               |
| D2 | Anchor min/max skip leading points ≤ 0 (opening balances jump from $0). The shown window's own points join the min/max with the same skip, so the line is never clipped.     |
| D3 | Padding is a share of the anchor RANGE: 10 % above, 20 % below, floor 0. A flat range (max = min) pads by a share of the value instead (10 %/20 % of it).                     |
| D4 | Colour by the period's change, first non-null point vs last: up = green, down = red, unchanged = foreground ink. Line in that colour, area under it a vertical gradient (≈40 % opacity at the line → 0 at the bottom), Apple Stocks style. Only the subject line gets the fill; benchmark overlays stay dashed lines. |
| D5 | "Compare" is a primary button (`btn-primary btn-xs`, outline when nothing is ticked is not enough — it read as a label).                                                      |
| D6 | A "Holdings" multi-select sits in the scope bar beside Accounts. It narrows the CHART and the STAT TILES only; table, pie and gainers/losers keep every row (selected rows highlighted) so you can keep clicking. |
| D7 | Clicking a table row, a pie wedge (holding view) or a gainer/loser sets the filter to that one holding. Clicking a category wedge selects every holding with ANY share in that wedge (whole value). The dropdown lists exactly what is selected. |
| D8 | Comparisons still work under a filter: the benchmark line is seeded from the selected holdings' own flows.                                                                  |
| D9 | Other tab: same chart rules (D1–D4), and the same filter over its rows (accounts). Clicking a row filters to it.                                                             |
| D10 | Intraday on 1wk and 1mo only, and only when asOf is today. Everywhere on the page (table, stats, gains, chart's last point) uses the live quote then. Never written to the prices file. |
| D11 | Intraday quotes live in an engine memory cache, one per symbol: price, quote time. A symbol whose quote or close was fetched in the last 15 min is not asked again. Any 429 stops the round and backs off 15 min globally. The daily-close fetch (auto prices / Update prices) already gets `meta.regularMarketPrice` in its reply, so it fills the cache for free. |
| D12 | The period defaults to 12 months (was all time). A URL without `gain=` means 12mo; all time is written as `gain=all`. |

The filter and the intraday flag live in the URL like the rest of the scope
(`?hold=VTI,AAPL`), through `urlCodec.ts`.

## Wire contract

### Engine

- `GET /api/holdings`, `/api/holdings/series`, `/api/holdings/benchmarks`:
  new `symbols=VTI,AAPL` (comma list, empty/absent = all). The report and
  series count only those commodities; benchmarks seed from their flows. The
  table never sends it; the stat tiles' request does.
- `GET /api/holdings/other`, `/api/holdings/other/series`: new
  `rows=assets:house,assets:van` (Other rows are accounts).
- Series responses gain `anchor: {min: string, max: string} | null`
  (decimal strings, D1 + D2 applied, NO padding — padding is presentation)
  when the request carries `anchor=1yr|5yr|all`. Computed from an
  `auto`-interval series over the anchor horizon, same scope and filter,
  unioned with the shown window's points. `null` when nothing is > 0.
- `live=1` on `/api/holdings`, `/api/holdings/series`, `/api/holdings/other*`:
  value today's point with cached intraday quotes where the cache has one
  dated today (exchange date). Ignored unless asOf is today. No network in
  these GETs.
- `POST /api/prices/intraday` `{symbols?: string[]}` (default: every held
  stock symbol): refresh stale cache entries (D11), return
  `{quotes: [{symbol, price, time, currency}], skipped: [symbol], rateLimited: bool}`.
  Paced through the same `Pacer` / concurrency as auto prices.

### SPA

- `holdingsScope` gains `holdings: ReadonlySet<string>` (symbols on Stocks,
  accounts on Other; switching tab clears it).
- Page: on 1wk/1mo with asOf = today, after the normal load, POST intraday,
  then reload with `live=1`. Chart heading note gains "· live 2:35 PM
  (delayed)" while live.
- `PeriodLineChart` gains optional `trend: "up" | "down" | "flat"` styling
  for the hero series (colour + gradient area) and `yDomain` override; a pure
  helper `holdings/ui/yAxis.ts` turns `anchor` + shown values into the padded
  domain (D3) and decides the trend (D4).

## Work packages

1. **Engine filters + anchor** — `symbols`, `rows`, `anchor` (core holdings +
   reports_api + benchmarks_api), tests in `ledgeline-core/tests` and
   `ledgeline-server/tests`.
2. **Engine intraday** — cache, POST endpoint, `live=1`, fill from the close
   fetch, 15-min memo, 429 backoff; fake `PriceFeed` tests.
3. **Chart styling** — gradient/colour/domain in `PeriodLineChart` +
   `yAxis.ts`, Compare button.
4. **Filter UI** — scope-bar dropdown, URL, click wiring (table, pie incl.
   category wedges, gainers/losers, Other rows), stat-tile fetch.
5. **Intraday UI** — refresh call, `live=1` reloads, heading note.
6. Docs (`docs/` holdings page), `just test-integration` (API shape changed).
