---
title: Comparing against a benchmark
description: "Draw what your portfolio would be worth had the same cash flows gone into an index fund, with dividends included."
order: 40
tags: [holdings, prices]
---

# Comparing against a benchmark

The Stocks tab's *Value over time* chart can draw what your portfolio would be
worth had the same money gone into an index fund instead. Open **Compare** in
the chart's heading and tick any of:

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
it. Your choices are remembered in this browser. None are ticked by default.

## What the line means

It is **the same cash flows, on the same dates, into that fund**. Every buy in
the current scope is money that could have bought the fund that day; every sale
or return of capital is money taken out of it that day. Moves between your own
accounts and stock splits are not cash flows and are ignored — exactly the rules
the windowed gain already uses to tell a contribution from a gain.

The two lines **start together** at the chart's left edge: the benchmark account
is seeded with your portfolio's actual market value on the first point, then
only the flows after that are replayed. So over 12 months the question is "given
what I had a year ago and what I have put in and taken out since, would I be
ahead in the fund?". A fund younger than the window (VXUS began trading in 2011)
starts at the first point it has a price for.

A withdrawal larger than the simulated account holds empties it rather than
driving it below zero. A share movement with neither a cost nor a market price
on its date cannot be valued, so it is left out of the simulation.

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
journal redraws the portfolio line and re-requests the benchmark lines, because
each one starts from the portfolio's value.

Fetched history is kept in **`benchmarks.prices.journal`, beside your main
journal**, as ordinary hledger price directives:

```journal
; ledgeline-benchmark SPY from 2025-09-18 through 2026-09-27
P 2025-09-29 SPY 656.5962 USD
P 2025-09-30 SPY 659.0695 USD
```

It is deliberately **not included** in your journal and must not be: these are
adjusted closes, not market prices, and would misvalue a real holding of the
same fund. Ledgeline never adds an `include` for it, never commits it with the
git safety net, and the live-reload watcher ignores it. When a chart needs days
the file doesn't have yet, that fund's whole history for the chart is
downloaded again in one request. That keeps every cached day on the same
dividend adjustment, because Yahoo re-bases the adjusted history each time a
fund pays a dividend.

It is **safe to delete** at any time. The next comparison simply downloads the
history again. A read-only session (one that cannot edit the journal) writes no
file at all: it keeps the fetched history in memory until the server stops.
