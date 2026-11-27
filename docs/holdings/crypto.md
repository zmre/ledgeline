---
title: Crypto
description: "Tracking bitcoin, ether, stablecoins and other coins on the Holdings tab: what works today, how to set it up, and the pitfalls."
order: 45
tags: [holdings, prices, crypto]
---

# Crypto

Ledgeline tracks coins the same way it tracks stocks. A coin is a commodity in
your journal, an account that holds one shows up on **Holdings → Stocks**, and
its price comes from Yahoo Finance. Charts, gains, the allocation pie and
Performance all work, provided you set up each coin as described below.

> [!WARNING]
> **Always give a coin a `yahoo:` tag.** Without one, Ledgeline looks the
> commodity up under its own symbol, and on Yahoo Finance `BTC`, `ETH`, `SOL`,
> `XRP` and `USDC` are *not* the coins. `BTC` is the Grayscale Bitcoin Mini
> Trust ETF, at about $27 a share. Update prices would write that price for your
> bitcoin and give no warning.

## Setting up a coin

Declare each coin once, with its Yahoo ticker and `type: crypto`:

```journal
commodity 1.00000000 BTC   ; type: crypto, yahoo: BTC-USD
commodity 1.000000 ETH     ; type: crypto, yahoo: ETH-USD
commodity 1.000000 SOL     ; type: crypto, yahoo: SOL-USD
commodity 1.00 USDC        ; type: crypto, yahoo: USDC-USD
```

- **`yahoo:`** is the Yahoo Finance ticker. Coins are quoted as a pair, for example
  `BTC-USD`. Use the pair in your journal's base currency: `BTC-USD` for a
  dollar ledger, `BTC-EUR` for a euro one. Ledgeline writes the fetched number
  in your base currency and does not convert it, so `BTC-EUR` in a dollar
  ledger would record euros as dollars.
- **`type: crypto`** puts the coin in the **Crypto** slice when the pie is
  divided by asset class or security type (see
  [Allocation by category](allocation.md)).
- **The number format** (`1.00000000`) sets how many decimals the coin's
  quantities show. Eight places is a satoshi.

Some tokens share a symbol with another token. Yahoo then adds a number to the
ticker, for example `UNI7083-USD` for Uniswap or `PEPE24478-USD` for Pepe. Search for the token on
finance.yahoo.com and copy the ticker exactly as shown. hledger needs a symbol with digits in
it quoted, so it is simpler to keep a plain symbol and put the full ticker in the
tag:

```journal
commodity 1.00 UNI   ; type: crypto, yahoo: UNI7083-USD
```

## The accounts

An account that holds a coin lands on **Stocks** without any tag, because a coin is a
non-currency commodity. Name your wallets and exchanges however you like:

```journal
account assets:crypto:coinbase   ; type: A
account assets:crypto:cold       ; type: A
```

Buying, selling and moving coins are ordinary transactions:

```journal
2026-03-02 Buy bitcoin
    assets:crypto:coinbase      0.05000000 BTC @ $61,200.00
    expenses:fees:crypto             $15.00
    assets:bank:checking        $-3,075.00

2026-03-09 Move to cold storage
    assets:crypto:cold          0.04990000 BTC
    expenses:fees:crypto        0.00010000 BTC
    assets:crypto:coinbase     -0.05000000 BTC
```

A move between two of your own wallets changes nothing on Holdings: Ledgeline
pools each coin across every account in scope. A network fee paid in the coin
reduces what you hold.

## Stablecoins

USDT, USDC and similar coins are priced like any other coin, so a depeg shows up.
They count as **Crypto** in the pie. To count one as cash instead, add
`assetclass: cash`:

```journal
commodity 1.00 USDC   ; type: crypto, assetclass: cash, yahoo: USDC-USD
```

The account stays on the Stocks tab either way. It holds a commodity, not your
base currency.

## Prices, weekends and today's quote

- **Update prices** and automatic updates fetch daily closes for coins like any
  other holding. Yahoo dates a coin's daily close at midnight UTC.
- **Weekends.** Automatic updates follow the stock-market calendar, so they do
  not fill in Saturday and Sunday closes. On the 1 week chart the weekend is
  drawn flat at Friday's price. The [delayed quote](index.md#the-value-over-time-chart)
  for today still applies on any day.
- **Small prices are rounded today.** A fetched price is rounded to your base
  currency's decimals. For a stablecoin that barely matters: USDT at $0.99874
  is written as `$1.00`. A token worth less than half a cent is written as
  `$0.00`, which values the holding at nothing, so fetched prices are not
  usable for such tokens yet. hledger itself keeps every decimal you write, so
  a price you enter by hand is exact:

  ```journal
  P 2026-11-26 PEPE $0.00000358
  ```

  Fetching still adds its rounded price for any date that has no price in the
  prices file, and the prices file is read after your own lines, so its
  `$0.00` would win. Until this is fixed, keep sub-cent tokens out of a
  ledger you update prices for, or delete the fetched lines for them.

## Not supported

- **Syncing wallets or exchanges.** Ledgeline reads your journal. It does not
  connect to a wallet, an exchange or a blockchain.
- **Tax lots.** Gains use average cost, as for stocks. There are no per-lot gains,
  wash-sale rules or tax reports.
- **DeFi positions, NFTs and staking pools** beyond what you book as
  ordinary transactions.

## What is planned

[Plan 48](https://github.com/zmre/ledgeline/blob/main/plans/48-crypto-holdings-research.md)
covers the gaps above:

- a coin tagged `type: crypto` is looked up as `SYMBOL-<base currency>` with no
  `yahoo:` tag needed;
- a warning when a well-known coin symbol is priced as something that is not a coin;
- small prices kept to their significant digits;
- weekend closes for coins;
- staking rewards and reinvested income counted as return rather than as new
  money in.
