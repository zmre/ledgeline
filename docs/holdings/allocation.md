---
title: Allocation by category
description: "The Divide by menu on the Stocks pie: asset class, sector, industry, security type, category and risk, from your commodity tags and Yahoo Finance."
order: 30
tags: [holdings, prices]
---

# Allocation by category

The pie on the Stocks tab has a **Divide by** menu. *By holding* (the default)
gives one slice per security. The other views split your holdings by what they
are:

| View              | What a slice is                                  | Example slices                         |
|-------------------|--------------------------------------------------|----------------------------------------|
| Asset class       | equity, bonds, cash, or anything you tag          | Equity, Bonds, Cash, Real estate        |
| Sector            | the business sector of the equity you own         | Technology, Healthcare, Energy          |
| Industry          | a stock's industry                                | Consumer Electronics, Semiconductors    |
| Security type     | what kind of security it is                       | Stock, ETF, Mutual fund                 |
| Category          | a fund's Morningstar category                     | Large Blend, Intermediate Core Bond     |
| Risk (Morningstar)| a fund's Morningstar risk rating                  | Low, Average, Above average             |

The menu lists only the views at least one holding on screen has data for.

## Funds are split, not lumped

A fund is divided in proportion to what it holds. A balanced fund that is 60%
stocks and 40% bonds puts 60% of its value in Equity and 40% in Bonds.

The Sector view does the same thing one level down. Yahoo reports a fund's
sector weights for its **stock** holdings only, so those weights split the
equity share and the rest of the fund is shown by asset class. For that
balanced fund, the pie shows its technology stocks under Technology and its
bonds as a "Bonds" slice. Spreading the sector weights over the whole fund
would count bonds as technology companies.

Industry, Category and Risk come from one field each and are not split. Yahoo
gives an industry for single stocks and not for funds. It gives a category and
risk rating for funds and not for single stocks.

## Where the data comes from

1. **Tags on your `commodity` directives.** These always win.
2. **Yahoo Finance**, for any view a commodity's tags leave open. Yahoo is
   asked under the commodity's [`yahoo:` tag](prices.md#the-yahoo-tag) when it has one,
   the same way a price update is.

Value nothing classifies goes into an **(unclassified)** slice, drawn in a faded
grey after every other slice. A private fund Yahoo doesn't know lands there,
and so does a fund's equity share when Yahoo has no sector weights for it. The
solid grey **(other)** slice is different: it folds together the smallest named
categories once there are more than the theme has chart colours (fourteen in
the default theme). The by-holding pie folds its smallest holdings the same way.

## The tags

Put them on the `commodity` directive line, like `name:` and `yahoo:`. Each tag
replaces its whole view for that commodity. It becomes one 100% slice, and
Yahoo's data for that view is ignored. The other views still come from Yahoo.

```journal
commodity 1.0000 VTSAX     ; category: Large Blend, risk: Average
commodity 1.0000 FXAIX     ; sector: Diversified
commodity 1.0000 CREF      ; assetclass: real estate, type: annuity
commodity 1.0000 BTC       ; type: crypto, yahoo: BTC-USD
commodity 1.0000 PRIVFUND  ; assetclass: private equity, sector: Healthcare, industry: Biotech, type: fund, category: Venture, risk: High
```

| Tag          | View          | Notes                                                                                       |
|--------------|---------------|---------------------------------------------------------------------------------------------|
| `assetclass` | Asset class   | `equity`/`stocks`, `bonds`/`fixed income`, `cash`/`money market`, `other` and `crypto` merge with Yahoo's labels. Anything else is shown as written. |
| `sector`     | Sector        | As written.                                                                                 |
| `industry`   | Industry      | As written.                                                                                 |
| `type`       | Security type | `stock`, `etf`, `mutualfund`/`fund`, `moneymarket`, `bond`, `crypto`, `currency` merge with Yahoo's labels. Anything else is shown as written. |
| `category`   | Category      | As written.                                                                                 |
| `risk`       | Risk          | As written. Yahoo's words are Low, Below average, Average, Above average and High.          |

`type:` is a tag journals already use for this (`type:mutualfund`), so the
Security type view reads what is already there.
A `type:` tag also sets the asset class when nothing else does: `type: stock` is
Equity, `type: bond` is Bonds, `type: moneymarket` is Cash and `type: crypto` is
Crypto. A `type: fund` implies nothing, because a fund can hold anything.

An `assetclass:` tag also reshapes the Sector view. `assetclass: bonds` on a
security puts its whole value in a "Bonds" slice there, whatever sector Yahoo
reports.

Labels that differ only in case are one slice, so `real estate` and
`Real Estate` land together.

A commodity with all six tags is never looked up on Yahoo.

## When Yahoo is unavailable

Yahoo's classification data sits behind an undocumented session handshake that
can stop working or rate-limit without notice. When it does, the pie uses your
tags alone, and a line above it says *Couldn't reach Yahoo — showing tag data
only*. Nothing else changes, and the next visit tries again.

## The cache: `commodity-profiles.json`

Ledgeline saves what Yahoo reports in `commodity-profiles.json`, in the same
folder as your main journal, so each security is looked up about once a month
and not on every visit. A ticker Yahoo doesn't know is asked about again after
a week, in case you have since added a `yahoo:` tag to fix the lookup.

- It holds only Yahoo's answers, not your tags, so a tag you edit takes effect
  at once.
- It is not a journal file. It is never `include`d, never read by hledger,
  never committed by the import's git step, and editing it does not trigger a
  reload.
- It is written only when Ledgeline may write to your journal's folder.
- **It is safe to delete.** Ledgeline rebuilds it on the next visit. A damaged
  file is treated as empty and replaced.

If your journal folder is a git repository, add `commodity-profiles.json` to
`.gitignore`, or it will show up as untracked.
