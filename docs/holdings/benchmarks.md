---
title: Comparing against a benchmark
description: "Draw an index fund's total return beside your own on the Performance chart, with dividends included."
order: 40
tags: [holdings, prices]
---

# Comparing against a benchmark

The Stocks tab's chart can draw an index fund's return beside your own. Open
**Compare** in the chart's heading and tick any of the funds below. Comparisons
are drawn on the **Performance** view only (see
[Value or Performance](index.md#the-value-over-time-chart)). Pressing Compare
while the chart shows Value switches it to Performance and opens the list.
Switching back to Value hides the benchmark lines; your ticks are kept.

| Box                    | Fund |
|------------------------|------|
| S&P 500 (SPY)          | SPDR S&P 500 ETF |
| Dow Jones (DIA)        | SPDR Dow Jones Industrial Average ETF |
| Nasdaq-100 (QQQ)       | Invesco QQQ |
| US total market (VTI)  | Vanguard Total Stock Market ETF |
| US bonds (BND)         | Vanguard Total Bond Market ETF |
| International (VXUS)   | Vanguard Total International Stock ETF |
| Small cap (IWM)        | iShares Russell 2000 ETF |
| Gold (GLD)             | SPDR Gold Shares |

Each ticked benchmark is a dashed line in its own colour — always the same
colour, whichever others are showing — and the legend under the chart names
it. Ledgeline saves your choices with your other view settings (see [Settings files](../settings/settings-files.md)). None are ticked by default.

When the chart is narrowed to some holdings (see
[the Holdings menu](index.md#the-value-over-time-chart)), your line is the
return of those holdings alone. That makes "how did my tech stocks do against
the Nasdaq-100" a fair comparison.

## What the line means

Each benchmark line is **the fund's total return since the chart's first
point**, in percent, with dividends reinvested. Your own line is your
time-weighted return over the same points, which buying and selling do not
move. All the lines start at 0 % at the chart's left edge, so at any point the
question is "had I held this fund instead, over the same stretch, would I be
ahead?". A fund younger than the window (VXUS began trading in 2011) starts at
the first point it has a price for.

Your line leaves dividends out, whether paid as cash or reinvested (see
[Dividends are not in Performance yet](index.md#the-value-over-time-chart)),
while the fund's line reinvests its dividends. A holding that pays dividends
therefore reads slightly behind a fund that returned the same.

The comparison needs holdings valued in US dollars (`$` or `USD`), because the
funds are priced in dollars. In any other base currency the box explains that
instead of drawing a line.

## Dividends are included

Prices are Yahoo Finance's **adjusted close**, which folds dividends (and
splits) back into the price. A line from plain closing prices would leave out a
bond fund's entire income and understate a stock fund by a couple of percent a
year. So the benchmark line is a total return, as if every distribution had
been reinvested.

## Where the prices come from, and the cache file

Your journal is not touched. The chart draws your portfolio from local data
first; the ticked benchmarks are then fetched together in the background by the
Ledgeline server, in one request (the page itself never contacts Yahoo), and
their lines appear when it arrives. Ticking another box fetches just that one.
If Yahoo cannot be reached for a benchmark, its box says so, with a retry, and
the rest of the chart is unaffected. Refreshing, updating prices or editing the
journal redraws the portfolio line and requests the benchmark lines again with
it. Nothing is fetched while the chart shows Value.

Fetched history is kept in **`benchmarks.prices.journal`, beside your main
journal**, as ordinary hledger price directives:

```journal
; ledgeline-benchmark SPY from 2025-09-18 through 2026-09-27
P 2025-09-29 SPY 656.5962 USD
P 2025-09-30 SPY 659.0695 USD
```

It is deliberately **not included** in your journal and must not be: these are
adjusted closes, not market prices, and would misvalue a real holding of the
same fund. Ledgeline never adds an `include` for it, and the live-reload
watcher ignores it. When a chart needs days
the file doesn't have yet, that fund's whole history for the chart is
downloaded again in one request. That keeps every cached day on the same
dividend adjustment, because Yahoo re-bases the adjusted history each time a
fund pays a dividend.

It is **safe to delete** at any time. The next comparison simply downloads the
history again. A read-only session (one that cannot edit the journal) writes no
file at all: it keeps the fetched history in memory until the server stops.
