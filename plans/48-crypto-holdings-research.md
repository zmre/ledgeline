# 48 — Crypto on Holdings

Research 2026-11-26 (live API probes 2026-11-27 02:30 UTC); decisions
recorded with Patrick the same day. Ready to build on its own branch. What
already works is documented in `docs/holdings/crypto.md` (shipped with plan
47); this plan closes the gaps that page lists. Branch at time of writing: `2026-11-26-holding-improvements`.

## Summary

- **Yahoo can price crypto, with no new provider.** `BTC-USD`, `ETH-USD`,
  `USDT-USD`, `USDC-USD` and `SOL-USD` all answer on the chart endpoint Ledgeline
  already uses, with daily candles at 00:00 UTC, weekends included, history back
  to 2014-09-17 for BTC, and `meta.instrumentType: "CRYPTOCURRENCY"`.
- **Ledgeline already half-supports crypto.** `commodity BTC ; yahoo: BTC-USD`
  works today, and `type: crypto` already produces a Crypto slice on the pie.
- **The real problem is silent mispricing.** Without the tag, `BTC` is looked up
  as itself, and on Yahoo `BTC` is the *Grayscale Bitcoin Mini Trust ETF*
  ($26.59, not $61,656). The same happens with `ETH`, `XRP`, `SOL` and `USDC`.
- **The second problem is rounding.** Fetched prices are rounded to the base
  currency's precision, so USDT at $0.99874 is written as `$1.00` and PEPE at
  $0.00000358 as `$0.00`.
- **Recommended approach:** stay Yahoo-only. Default the ticker to `SYM-<base>`
  only for commodities the user marked `type: crypto`, warn (never block) when an
  untagged well-known coin symbol resolves to a non-crypto instrument, round
  prices by significant digits, and let crypto prices land on weekends too.
- **Effort:** about 4.5–5.5 days in six work packages (see [Build plan](#build-plan)).
  A second price provider is not planned (C1).
- **Risk:** low if crypto support stays opt-in. Yahoo's unofficial API and its
  ToS are the same exposure Ledgeline already accepts for stocks.
- **Timing:** not in the plan 47 PR. Do it as its own PR (plan 48) soon after.
  The collision guard and the rounding fix go first, because they are
  correctness bugs that can already hit any user who holds crypto.

## Decisions

Patrick answered the open questions on 2026-11-26. These override anything
the research sections below say.

| #  | Decision |
| -- | -------- |
| C1 | Yahoo only. No second provider. Revisit Coinbase only if Yahoo drops `-USD` pairs. |
| C2 | Ticker resolution: `yahoo: X` always wins; else a commodity tagged `type: crypto` (or `assetclass: crypto`) is looked up as `<SYMBOL>-<base ISO>`; else the bare symbol (unchanged). Never add a suffix to an untagged symbol. |
| C3 | Non-USD ledgers use Yahoo's native pair (`BTC-EUR` for a `€` base), not `BTC-USD` converted through the journal's FX prices. |
| C4 | Collision guard WARNS, never blocks: an untagged well-known coin symbol (built-in list) that resolves to a non-`CRYPTOCURRENCY` instrument is still priced, and the Update prices summary / Holdings warning names what it was priced as and how to fix it. A `type: crypto` commodity whose ticker answers with a non-crypto `instrumentType` is REFUSED (not written) and reported. |
| C5 | `type: crypto` does NOT imply `holdings: stocks`; a coin account already lands on Stocks because a coin is a non-currency commodity. |
| C6 | Income-funded acquisitions are RETURN, not new money: a staking reward or any buy whose counter-leg is an `income` account (a reinvested dividend too) is not a flow in plan 47's Performance/flow maths, and a coin fee paid to an `expenses` account is a loss, not a flow. Sane default per the "accounting flows configurable" principle; a setting only if someone asks. Patrick had no view, so this is the conventional investment-performance treatment. It also fixes plan 47 A1f for reinvested dividends on stocks. |
| C7 | Classification: use `meta.instrumentType` from the chart response as a fallback. Re-test quoteSummary for `BTC-USD` during the build; if it has no sector/category, show coins as **Crypto** for category and sector too, so the pie never shows them as Unclassified. |
| C8 | Fetched prices round to significant digits (about 6), never below the base currency's own decimals. Fixes stablecoins (`$0.99874`), sub-cent tokens and penny stocks. |
| C9 | Coins get weekend closes in automatic updates (daily target = yesterday UTC every day; weekly fill ends on Sunday). |
| C10 | Documentation is a first-class deliverable: every WP updates `docs/holdings/crypto.md` (and `prices.md`/`allocation.md` where it touches them) in the same commit, and the page's "Not supported"/pitfall notes shrink as WPs land. |

## Build plan

Branch from main after plan 47 merges. Order matters: WP1 and WP2 fix wrong
numbers users can already get. Each WP is one commit with its tests and its
docs change. Read the research sections below for the evidence behind each.

### WP1 — Ticker resolution and collision guard (M, 1–1.5 days)

- Where: the symbol → ticker mapping shared by `prices_api.rs`, `auto_prices.rs`
  and `intraday.rs` (find the `yahoo:` tag lookup; keep one function). Base ISO
  from the holdings base: `$`/`USD`/`US$` → `USD`, `€` → `EUR`, `£` → `GBP`, an
  ISO code as itself; an unmappable base means no suffix (fall back to the bare
  symbol and say so in the summary).
- Parse `meta.instrumentType` and `meta.longName` in `yahoo.rs` `ChartMeta`
  (both appear in every chart reply; see Findings §1).
- Guard list (C4): BTC, ETH, USDT, USDC, SOL, XRP, BNB, DOGE, ADA, TRX, AVAX,
  LINK, DOT, LTC, BCH, XLM, SHIB, MATIC/POL, TON, UNI. Warning text names what
  was priced ("BTC priced as Grayscale Bitcoin Mini Trust ETF") and the fix
  (`type: crypto` or `yahoo: BTC-USD`). Surface it in the Update prices summary
  and as a Holdings warning (existing warnings list).
- The C4 refusal applies to Update prices, auto prices and intraday quotes alike.
- Accept: a fixture `commodity BTC ; type: crypto` fetches `BTC-USD`; an untagged
  `BTC` answered with the recorded ETF body is written AND warned; a `type: crypto`
  answered with an ETF body is refused; `€` base asks `BTC-EUR`.

### WP2 — Significant-digit rounding (S, 0.5 day)

- Where: `prices_api.rs` `apply_update` (`MAX_FETCHED_QUOTE_PLACES`,
  `round_to_places`, `infer_style`). Round to `max(base precision, places for 6
  significant digits)`; raise the 6-place cap for prices under 1 (cap total
  places at something like 12 to bound Dec growth); strip float noise as today.
- The written `P` line's style must not change how the base currency displays
  elsewhere (check `infer_style` and the journal's commodity style for `$`).
- Accept: 0.99874 → `$0.99874`; 3.58e-06 → `$0.00000358`; 61656.01 → `$61,656.01`;
  a penny stock 0.0412 → `$0.0412`; existing two-decimal stocks unchanged.

### WP3 — Weekend closes for coins (S–M, 0.5–1 day)

- Where: `auto_prices.rs` target-date logic. A symbol is a coin when tagged
  `type: crypto`/`assetclass: crypto` or its last fetch reported
  `CRYPTOCURRENCY` (remember it beside the existing memo). Daily target =
  yesterday UTC on any weekday; weekly history fill takes the week's last day.
- Accept: clock-pinned tests with a stock and a coin in one plan on a Saturday,
  Sunday and Monday.

### WP4 — Flows: income-funded buys and in-kind fees, for ALL holdings (S–M, 1 day)

Not crypto-specific: this applies to every holding on both tabs. Its most common
case is a REINVESTED STOCK DIVIDEND, which plan 47 counts as new money in, so
Performance leaves dividends out while the Compare lines (total return) include
them. Patrick deferred that fix from plan 47 to here (2026-11-26).


- Where: `ledgeline-core/src/holdings/performance.rs` and the flow recording in
  `engine.rs` (`HoldingsFlows`). A posting pair whose counter-leg is an income
  account is not a flow (C6); a quantity leaving to an expense account is not a
  flow. Benchmarks seed from the same flows, so check `benchmark.rs` too.
- Accept: a reinvested stock/fund dividend and a staking reward raise
  `returnPct`; an in-kind fee (coin or share) lowers it; none produces a flow
  marker; benchmark seeding ignores them too; the Other tab is unchanged (its
  flows already exclude income and equity counter-legs — add a test proving it). Update plan 47's A1f note and
  `docs/holdings/index.md` (cash dividends are still the remaining limit).

### WP5 — Classification fallback (S, 0.5 day)

- Where: `commodity_profile.rs` (already maps `CRYPTOCURRENCY` → Crypto). Use
  the remembered `instrumentType` when quoteSummary is unavailable; per C7,
  fill sector and category with "Crypto" for coins when Yahoo gives none.
- Accept: an untagged-but-`yahoo: BTC-USD` coin lands in the Crypto slice in
  every Divide-by view without a quoteSummary response.

### WP6 — Fixtures, precision, docs (M, 1 day)

- Crypto fixture ledger (see "Testing without owning crypto"; mind the fixture
  gitignore traps), recorded Yahoo bodies (`BTC-USD`, `USDT-USD`, `BTC` the ETF,
  `UNI-USD` not found, a null-close day, a `BTC-EUR`).
- Verify the Holdings quantity column honours `commodity 1.00000000 BTC`, and an
  18-place ETH × 10-place price does not overflow (`Dec` is i128; checked ops
  error rather than wrap).
- Rewrite `docs/holdings/crypto.md` for the finished feature: drop the warning
  about untagged symbols down to a note, remove the rounding and weekend
  pitfalls, keep "Not supported". Add a screenshot via `just docs-screenshots`.
- Run `just test-integration` (wire shape) and the e2e suite.

Total about 4.5–5.5 days.

## Findings

### 1. Yahoo's chart endpoint and crypto

These were probed live with `curl` against
`https://query1.finance.yahoo.com/v8/finance/chart/<T>?interval=1d&range=…` on
2026-11-27 02:33 UTC.

| Ticker     | `instrumentType` | Exchange | Name (`longName`)                 | Price       | `priceHint` |
|------------|------------------|----------|-----------------------------------|-------------|-------------|
| `BTC-USD`  | CRYPTOCURRENCY   | CCC      | Bitcoin USD                       | 61,656.01   | 2           |
| `ETH-USD`  | CRYPTOCURRENCY   | CCC      | Ethereum USD                      | 1,733.00    | 2           |
| `USDT-USD` | CRYPTOCURRENCY   | CCC      | Tether USDt USD                   | 0.99874     | 5           |
| `USDC-USD` | CRYPTOCURRENCY   | CCC      | USDC USD                          | 0.9998625   | 6           |
| `SOL-USD`  | CRYPTOCURRENCY   | CCC      | Solana USD                        | 72.76       | 2           |
| `BTC-EUR`  | CRYPTOCURRENCY   | CCC      | Bitcoin EUR (currency EUR)        | 56,176.34   | —           |
| `BTC`      | **ETF**          | NYSEArca | Grayscale Bitcoin Mini Trust ETF  | **26.59**   | 2           |
| `ETH`      | **ETF**          | NYSEArca | Grayscale Ethereum Mini Trust ETF | **16.10**   | —           |
| `XRP`      | **ETF**          | NYSEArca | Bitwise XRP ETF                   | **10.52**   | —           |
| `SOL`      | **EQUITY**       | NYSE     | (no name, no price)               | —           | —           |
| `USDC`     | **EQUITY**       | OTCPK    | USDATA Corp.                      | **0.0012**  | —           |
| `USDT`, `DOGE`, `ADA` | —     | —        | "No data found"                   | —           | —           |

**Symbol convention.** `<COIN>-<FIAT>`: `BTC-USD`, `BTC-EUR`, `BTC-GBP`. `CCC`
is Yahoo's exchange code for its crypto feed. Yahoo's quote pages credit the data
to "Coinbase and CoinMarketCap" (see Sources).

**Duplicate token symbols.** Yahoo disambiguates them with a numeric id spliced
into the base symbol: `UNI-USD` is "not found", and Uniswap is `UNI7083-USD`.
Its search endpoint (`query2…/v1/finance/search?q=PEPE`) returns
`PEPE24478-USD` (Pepe), `PEPECOIN-USD`, `PEP-USD` (Pepecoin), `PEPE32982-USD`
and others. So a mechanical `SYM-USD` works for the large coins and is
unreliable for the long tail. In the worst case it quietly picks a different
token.

**Daily candles and "close".** `exchangeTimezoneName: "UTC"`, `gmtoffset: 0`.
Every daily timestamp is exactly 00:00 UTC. A 3-month `BTC-USD` series had 93
consecutive candles with all seven weekdays and no gaps. Each candle covers one
UTC calendar day, so its close is the last price before the next 00:00 UTC. The
convention matches CoinMarketCap's daily close: a CoinMarketCap article's
Wednesday close of $60,378 matches Yahoo's 60,378.57. I found no Yahoo document
that states the cutoff, so treat it as observed behaviour, not documented.

**`currentTradingPeriod`.** For crypto, `regular` is `start = 00:00 UTC today`
and `end = 23:59:00 UTC today`, with `pre` and `post` both zero-length. The
existing rule in `yahoo::parse_chart_closes` ("drop today's candle until
`now >= regular.end`") therefore works unchanged: today's forming candle is
dropped and yesterday's UTC close is kept. One edge: `end` is 23:59:00, not
24:00, so a fetch in the last minute of the UTC day records a near-final price
as that day's close. The error is at most one minute of movement, which is
acceptable.

**Null closes happen.** The `BTC-USD` (and ETH, USDT, USDC, SOL) candle for
2026-11-26 had `close: null` in every response. Coinbase's candle for the same
day was 60,495.57. Ledgeline already skips null closes and falls back to the
previous real one (`skips_a_null_close_and_falls_back_to_the_prior_real_one`).
A missing day costs one day's accuracy. It does not write a wrong price.

**Intraday.** `interval=5m&range=1d` returns candles (32 at 02:33 UTC), and
`meta.regularMarketPrice` and `regularMarketTime` are present and current at
any hour, including weekends. Plan 47's intraday cache needs no change: it
dates a quote with `gmtoffset` (0, so UTC), and `reports_api::today_utc()` is
UTC too.

**`adjclose`.** Present and identical to `close`. Crypto has no splits or
dividends, so benchmark code that reads adjusted closes works unchanged.

**History depth.** `BTC-USD` daily data starts 2014-09-17
(`meta.firstTradeDate`). This is deeper than any free alternative below.

**quoteSummary / profile.** **Unverified live**: the crumb endpoint answered
"Too Many Requests" on both attempts. Ledgeline's own parser test already models
a crypto answer as `quoteType: CRYPTOCURRENCY` with no `assetProfile` or
`fundProfile`, so there is no sector, industry or category, and I would expect
that to hold. It does not matter much: the chart response, which Ledgeline
fetches anyway for prices, carries `instrumentType: "CRYPTOCURRENCY"` and
`longName`, so crypto can be classified without the crumb handshake.

**Precision.** `priceHint` is 2 for BTC, ETH and SOL, 5 for USDT and DOGE, 6 for
USDC, and only **5 for PEPE and SHIB**, whose prices are about 3.6e-06. So
`priceHint` alone is not a safe rounding rule.

### 2. Alternatives

Probed 2026-11-27 without keys from a US IP, plus each provider's docs.

- **CoinGecko.** The keyless `api.coingecko.com/api/v3/simple/price` works
  (HTTP 200). Keyless `market_chart` with `days=400` is refused with "Public API
  users are limited to querying historical data within the past 365 days". The
  free *Demo* plan needs a key (`x-cg-demo-api-key`) and allows 100 calls/min
  and 10,000 calls/month, 1 year of daily history, and **attribution
  required**. The keyless public tier is IP-rate-limited with no published
  number. Commercial use on Demo is not stated. Ids are slugs (`bitcoin`,
  `tether`), not symbols, so every coin needs a mapping. Portfolio Performance
  hit the 365-day limit and added a paid-key option (issue #3896).
- **CoinMarketCap.** The free Basic plan needs a key: 15,000 credits/month,
  50 req/min, about 1 year of daily history. Its terms allow integrated use but
  not redistribution. Each user would need their own key.
- **Coinbase Exchange.** `api.exchange.coinbase.com/products/BTC-USD/candles?granularity=86400`
  needs no key. It returns at most 300 candles per request, with buckets
  aligned at 00:00 UTC (observed), and allows 10 req/s per IP, burst 15. It
  only covers pairs Coinbase lists. `api.coinbase.com/v2/prices/BTC-USD/spot`
  also works keyless.
- **Kraken.** `api.kraken.com/0/public/OHLC?pair=XBTUSD&interval=1440` needs
  no key. It returns **only the 720 most recent** entries; older data cannot be
  fetched. Kraken uses `XBT` for bitcoin and odd pair names (`XXBTZUSD`).
- **CryptoCompare / CoinDesk Data.** **Dead for free use.** The free tier was
  retired 2026-05-21, and `min-api.cryptocompare.com/data/v2/histoday` now
  answers 401 "API key required". Empower's crypto tracking was reported to use
  CryptoCompare. That report predates the retirement.
- **Binance.** `api.binance.com` answers **HTTP 451** "restricted location"
  from the US. Unusable as a default.

None of these is worth adding beside Yahoo now. Yahoo has the deepest history,
no key, one symbol scheme, and code paths that already exist. Coinbase is the
only plausible fallback: keyless, stable, documented, UTC-aligned. It would add
a second ticker mapping and a second set of failure modes for one asset class.
Keep it as the fallback if Yahoo drops crypto.

### 3. How hledger users write crypto

- **Symbols.** Usually the ticker as a bare commodity: `BTC`, `ETH`, `ADA`
  (hledger's own investments tutorial writes `2000 ADA @ $0.02`). `₿` works as
  a prefix symbol: hledger 1.52 parsed `₿0.001` (verified locally). A symbol
  containing digits must be quoted (`"UNI7083"`) per the hledger manual.
  `USDT` and `USDC` are bare four-letter symbols.
- **Precision.** `commodity 1.00000000 BTC` (8 places, satoshis) is the natural
  declaration. ETH's 18 places (wei) are possible: hledger stores up to 255
  significant digits. In practice people use far fewer (**unverified** as a
  convention). hledger keeps a `P` price's full precision in valuation:
  `P … PEPE $0.0000045123` and `P … USDT $0.99874` valued exactly (verified on
  1.52).
- **Lots and cost basis.** hledger 1.52 does not compute gains from lots.
  Users write `@` costs and, if they want lots, per-lot subaccounts. hledger
  1.99.x / 2.0 previews add `lots:` commodity tags, FIFO/average methods and a
  `holdings` command. Ledgeline uses average cost, which is fine for display.
  Tax lots are out of scope.
- **Fees in the coin.** The hledger tutorial says fees are usually simply
  expensed (`expenses:fees  0.0001 BTC`), or capitalised by disposing and
  re-acquiring at basis. ~~Open: how Ledgeline's average-cost pool treats a
  coin leaving to an expense account with no `@`.~~ **Resolved by WP4:** a
  sale at zero. The quantity and its pro-rata share of the basis leave; it is
  not a flow, so it lowers Performance. (Before WP4 the wallet-move spelling
  also tainted the basis: the arriving bare leg read as a cost-less buy.)
- **Staking rewards.** Usually income at fair value on receipt
  (`assets:crypto  0.01 ETH @ $3000` / `income:staking`). ~~Open: whether
  Ledgeline counts that as money in (a flow) or as gain.~~ **Resolved by
  WP4 (C6):** gain. It is not a flow, so it raises Performance and a windowed
  gain; the `@` fair value is still basis, so the all-time gain (value −
  basis) leaves it out, as the tax view does.
- **Transfers between wallets and exchanges** are just two asset postings of the
  same commodity. Ledgeline pools per commodity across accounts in scope, so
  nothing special is needed.

**Collision with Ledgeline's currency detection.** None for the common cases.
`commodities::is_iso_code` is the active ISO-4217 list, which has no `BTC` and no
`XBT`, and `USDT` and `USDC` fail the three-letter check. So BTC is a "stock"
even if the journal pays fees in it to an expense account. ₿ is not in
`is_glyph`, so it is a stock too, but it needs a `yahoo:` tag because Yahoo
cannot look up `₿`. Docs already show `account assets:crypto:cold ; type: A,
holdings: stocks` and `commodity BTC ; type: crypto, yahoo: BTC-USD`.

**Stablecoins.** Economically near-cash, but they can depeg and carry
issuer risk. Ghostfolio puts all crypto under asset class `LIQUIDITY`, subclass
`CRYPTOCURRENCY`. Recommendation: price stablecoins like any coin (Yahoo has
them), keep them on Stocks, and default the asset class to Crypto. A user who
wants them counted as cash writes `assetclass: cash` on the commodity, which
already works. Do not hard-code a peg at $1.00, because the price exists and is
cheap to fetch.

### 4. What other apps do

| App                    | How crypto is modelled                                                                                                   | Verified? |
|------------------------|--------------------------------------------------------------------------------------------------------------------------|-----------|
| Yahoo Finance          | A security with `quoteType` CRYPTOCURRENCY, ticker `BTC-USD`; sits in the same portfolio lists as stocks.                | yes (API) |
| Ghostfolio             | Asset class LIQUIDITY, subclass CRYPTOCURRENCY. Data from Yahoo (internal `BTCUSD` → `BTC-USD`) or CoinGecko. Yahoo search is filtered to base-currency pairs and a known-coin list. | yes (source) |
| Portfolio Performance  | An ordinary security with a CoinGecko quote feed; free history capped at 365 days, paid key optional.                    | yes (issue #3896) |
| Empower                | Manual "Other Assets" holdings, priced automatically (reportedly via CryptoCompare); allocation bucket unclear.          | partly    |
| Apple Stocks           | Shows `BTC-USD`-style symbols as quotes. No portfolio model.                                                              | unverified |

The consensus is a security in the normal holdings list, with a separate asset
class or subclass for allocation. That is Ledgeline's existing model.

## Options compared

| Option                                   | Keys     | History free          | Symbol scheme       | New code   | Silent-mispricing risk | Verdict        |
|------------------------------------------|----------|-----------------------|---------------------|------------|------------------------|----------------|
| A. Do nothing (tag `yahoo: BTC-USD`)     | none     | 2014→ (Yahoo)         | user writes ticker  | none       | **high** (`BTC` = ETF)  | not enough     |
| B. Yahoo + opt-in defaults + guard       | none     | 2014→ (Yahoo)         | `SYM-<base>` if `type: crypto` | ~3–4 days | low                | **recommended** |
| C. B + Coinbase fallback                 | none     | paged, 300/request    | `BTC-USD` (same)    | +2 days    | low                    | later, if Yahoo breaks |
| D. CoinGecko as crypto source            | demo key | 365 days              | slug ids            | +3 days    | low (ids are unique)    | no             |
| E. CoinMarketCap                         | key      | ~1 year               | numeric ids         | +3 days    | low                    | no             |
| F. Kraken / Binance / CoinDesk           | none / — | 720 days / blocked / dead | exchange pairs | +2 days    | medium                 | no             |

## Design sketch for Ledgeline

1. **Ticker resolution** (extends `prices_api`'s mapping):
   1. `yahoo: X` → `X`. Unchanged, and always wins.
   2. else, commodity tagged `type: crypto` (or `assetclass: crypto`) →
      `<SYM>-<base ISO>`, where base ISO is `USD` for `$`, `USD` or `US$`, and
      the ISO code for `€` → `EUR`, etc.
   3. else → the symbol itself. Unchanged.

   No blind `-USD` suffix on untagged symbols. `BTC` could be a deliberate
   holding of the Grayscale ETF, and Ledgeline cannot know.
2. **Collision guard.** Keep a small built-in list of the top coin symbols (BTC,
   ETH, USDT, USDC, SOL, XRP, BNB, DOGE, ADA, TRX, AVAX, LINK, DOT, LTC, …).
   When an untagged commodity on that list resolves to a non-CRYPTOCURRENCY
   instrument, still write the price, because it may be the ETF, but report it
   per symbol: *"BTC priced as Grayscale Bitcoin Mini Trust ETF (NYSEArca). If
   you hold bitcoin, add `type: crypto` or `yahoo: BTC-USD` to `commodity BTC`."*
   The reverse check needs no list: a commodity tagged `type: crypto` whose
   ticker answers with a non-crypto `instrumentType` is refused, not written.
3. **Show what was priced.** The Update prices summary already reports per
   symbol. Add Yahoo's `longName` from the chart `meta` ("Uniswap USD") so a
   wrong long-tail token is visible.
4. **Significant-digit rounding.** Round a fetched price to
   `max(style.precision, digits needed for ~6 significant figures)`, and raise
   or remove the 6-place `MAX_FETCHED_QUOTE_PLACES` cap for prices below $1.
   With this rule USDT → `$0.99874` and PEPE → `$0.00000358000`. This also fixes
   penny stocks.
5. **24/7 auto prices.** For crypto-classified symbols (tag, or a remembered
   `instrumentType` from an earlier fetch), the "latest completed close" is
   yesterday UTC on every day of the week, not the US 21:30-UTC weekday rule,
   and the weekly-history fill takes the week's last day (Sunday) rather than
   Friday. Without this, a 1-week chart draws Saturday and Sunday flat at
   Friday's price, though intraday hides that for today's point.
6. **Classification.** Use the chart `meta.instrumentType` (free with every
   price fetch) as a fallback source for the security type, so crypto lands in
   the Crypto slice even when the quoteSummary crumb is rate-limited. The
   existing mapping `CRYPTOCURRENCY → Crypto` in `commodity_profile.rs` does
   the rest.
7. **Quantity display.** Honour the commodity's declared precision (8 for BTC)
   in the Holdings quantity column. **Unverified** whether it already does.
   Check in WP5.

## Work packages

Superseded by [Build plan](#build-plan) above.

## Risks

- **Silent mispricing.** This is the main risk, and it exists today: `BTC`,
  `ETH`, `XRP` and `USDC` resolve to unrelated securities. WP1 is the
  mitigation. Long-tail tokens (`UNI7083-USD`) still need an explicit `yahoo:`
  tag, and the docs should say so.
- **Rounding.** This also exists today: stablecoins written as `$1.00`, and
  micro-priced tokens written as `$0.00`, which values the holding at zero.
  WP2 fixes it.
- **Yahoo instability and ToS.** The chart endpoint is unofficial, and Yahoo's
  terms forbid automated collection without permission. This is the exposure
  Ledgeline already accepts for stocks, and crypto adds requests, not a new
  category of risk. The quoteSummary crumb was rate-limited during this
  research, which is a reminder to keep crypto independent of it (design
  point 6).
- **Data gaps.** Null daily closes occur (2026-11-26). The existing
  null-skipping handles them. Note in docs that a missing day takes the prior
  close.
- **Decimal overflow.** `Dec` is `i128` with a scale. An 18-place ETH quantity
  multiplied by a 10-place price carries 28 places and leaves about 1e10 of
  headroom. Operations are checked, so the failure mode is an error, not a
  wrong number, but it needs a test (WP5).
- **Scope creep.** Tax lots, DeFi positions, wallet sync and on-chain balances
  are out of scope. Say so in the docs.

## Testing without owning crypto

- **Fixture ledger** (`fixtures/crypto/` or added to an existing holdings
  fixture). It should contain:
  - `commodity 1.00000000 BTC ; type: crypto`
  - an 18-place ETH, USDT and USDC, a `"PEPE24478"`-style quoted symbol with a
    `yahoo:` tag
  - an untagged `BTC` in a second ledger, to exercise the guard
  - a fee paid in BTC, a staking reward, and a transfer between two wallets
  - weekend-dated `P` lines

  Mind the fixture gitignore traps (see memory).
- **Recorded Yahoo responses.** Save the chart JSON for `BTC-USD`, `USDT-USD`,
  `BTC` (the ETF), `UNI-USD` (not found) and a null-close day as test bodies.
  The existing `PriceFeed` and `Transport` seams take canned bodies, the way
  `yahoo.rs` and `yahoo_profile.rs` tests already do. The probe outputs from
  this research are a starting point. Strip anything volatile.
- **Clock-pinned `auto_prices` tests.** Saturday and Sunday targets for crypto
  versus Friday targets for stocks, side by side in one plan.
- **Rounding tests** at 0.99874, 3.58e-06 and 61656.01.
- No live-network tests in CI. A manual smoke check with a throwaway ledger is
  enough before release.

## Recommendation

**Not in this PR.** Plan 47 is about charts, the branch already carries
substantial engine and web changes, and every crypto change lands in plan-41
territory (price writing, auto-price scheduling, ticker mapping). Folding it in
would widen review and regression surface for a feature the author cannot
exercise with real holdings.

**Do it next, as its own PR (plan 48), WP1 and WP2 first.** Those two are
correctness fixes that already matter to anyone holding crypto. The rounding
fix also helps any sub-$1 security. WP3–WP5 follow in the same PR. Revisit a
second provider (Coinbase) only if Yahoo stops serving `-USD` pairs.

## Open questions (answered)

1. Block or warn on an untagged `BTC` that resolves to an ETF? **Warn** (C4).
2. How should a fee paid in the coin and a staking reward count? Patrick had
   no view; decided as C6 (income-funded = return, fee = loss).
3. Native `BTC-EUR` pair or `BTC-USD` through FX? **Native pair** (C3).
4. Should `type: crypto` imply `holdings: stocks`? **No** (C5).
5. Does quoteSummary give coins a sector or category? Still unverified; re-test
   during WP5, and show "Crypto" when it gives nothing (C7).

## Sources

Live probes (run 2026-11-27 ~02:30 UTC, no keys, US IP):

- Yahoo chart: `https://query1.finance.yahoo.com/v8/finance/chart/{BTC-USD,ETH-USD,USDT-USD,USDC-USD,SOL-USD,BTC,ETH,SOL,USDC,USDT,XRP,DOGE,ADA,UNI-USD,UNI7083-USD,BTC-EUR,BTC-GBP,PEPE24478-USD,SHIB-USD}`
- Yahoo search: `https://query2.finance.yahoo.com/v1/finance/search?q=UNI` and `?q=PEPE`
- Yahoo quoteSummary crumb: `https://query2.finance.yahoo.com/v1/test/getcrumb` (answered "Too Many Requests")
- CoinGecko: `https://api.coingecko.com/api/v3/simple/price?ids=bitcoin,tether&vs_currencies=usd` (200), `…/coins/bitcoin/market_chart?days=400` (401, 365-day limit)
- Coinbase: `https://api.coinbase.com/v2/prices/BTC-USD/spot`, `https://api.exchange.coinbase.com/products/BTC-USD/candles?granularity=86400`
- Kraken: `https://api.kraken.com/0/public/OHLC?pair=XBTUSD&interval=1440`
- Binance: `https://api.binance.com/api/v3/klines?symbol=BTCUSDT&interval=1d&limit=2` (451)
- CryptoCompare: `https://min-api.cryptocompare.com/data/v2/histoday?fsym=BTC&tsym=USD&limit=2` (401)
- hledger 1.52 locally: `P` precision preserved in `bal -V`; `₿0.001` parses.

Documents (read 2026-11-26/27):

- Yahoo Terms of Service, §2(d)(9), §2(e): https://legal.yahoo.com/us/en/yahoo/terms/otos/index.html
- Yahoo BTC-USD quote page (data credit, CCC exchange): https://finance.yahoo.com/quote/BTC-USD/latest-news/ ; https://finance.yahoo.com/chart/BTC-USD/
- CoinMarketCap daily-close article (matching Yahoo's close): https://coinmarketcap.com/academy/article/bitcoin-price-holds-june-close-wall-street
- CoinGecko API introduction: https://docs.coingecko.com/reference/introduction
- CoinGecko rate limits: https://docs.coingecko.com/docs/common-errors-rate-limit
- CoinGecko pricing (Demo: 10k calls/month, 100/min, 1-year history, attribution): https://www.coingecko.com/en/api/pricing
- CoinMarketCap API pricing (Basic plan): https://coinmarketcap.com/api/pricing/
- Coinbase Exchange candles: https://docs.cdp.coinbase.com/exchange/reference/exchangerestapi_getproductcandles
- Coinbase Exchange rate limits: https://docs.cdp.coinbase.com/exchange/rest-api/rate-limits
- Kraken OHLC (720 entries): https://docs.kraken.com/api/docs/rest-api/get-ohlc-data/
- CoinDesk Data free-tier retirement (2026-05-21): https://data.coindesk.com/blogs/changes-to-coindesk-data-indices-api-free-tier-access
- rotki issue on CryptoCompare key requirement: https://github.com/rotki/rotki/issues/12215
- hledger manual (commodity quoting, 255 significant digits): https://hledger.org/manual.html
- hledger "Track investments" (ADA example, fees, in-kind transfer fee): https://hledger.org/investments.html
- hledger lots spec: https://hledger.org/SPEC-lots.html ; releases: https://github.com/hledgerorg/hledger/releases
- PTA forum, cryptocurrency thread: https://forum.plaintextaccounting.org/t/cryptocurrency/37
- Ghostfolio schema (AssetClass/AssetSubClass/DataSource): https://github.com/ghostfolio/ghostfolio/blob/main/prisma/schema.prisma
- Ghostfolio Yahoo data enhancer (CRYPTOCURRENCY → LIQUIDITY): https://github.com/ghostfolio/ghostfolio/blob/main/apps/api/src/services/data-provider/data-enhancer/yahoo-finance/yahoo-finance.service.ts
- Portfolio Performance CoinGecko 365-day issue: https://github.com/portfolio-performance/portfolio/issues/3896
- Empower crypto tracking (secondary review): https://wallethacks.com/personal-capital-review/

Ledgeline code read: `crates/ledgeline-server/src/{yahoo.rs, yahoo_profile.rs,
prices_api.rs, auto_prices.rs, intraday.rs, commodity_profile.rs}`,
`crates/ledgeline-core/src/holdings/{commodities.rs, live.rs}`,
`docs/holdings/{index,prices,tags,allocation}.md`, `plans/47-holdings-charts.md`.
