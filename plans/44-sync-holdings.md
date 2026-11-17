# 44 — Investment holdings from bank sync

Draft for Patrick's review, 2026-11-15. Builds on plan 43 (SimpleFIN sync,
branch `2026-11-14-simplefin`). Nothing here is built yet.

## Goal

SimpleFIN sends a snapshot of each investment account's positions. Use it to:

1. tell the user when the journal's shares disagree with the broker's;
2. price funds Yahoo can't (401(k) trusts, some money-market and mutual
   funds);
3. optionally, keep an account's positions in step when its transactions
   don't carry shares at all.

Without the journal ever silently drifting from what the user wrote.

## 1. What SimpleFIN sends

Each account may carry `holdings`; each holding has (from Patrick's real
Bridge data, names only):

| Field | Meaning (per the protocol; to be confirmed on real data) |
| --- | --- |
| `id` | The position's id at the provider. |
| `symbol` | Ticker. May be blank for cash or a plan-only fund (unconfirmed). |
| `description` | Fund name, e.g. "Vanguard Total Stock Market ETF". |
| `shares` | Quantity, decimal text. |
| `market_value` | Total value at the account's `balance-date`, in `currency`. |
| `cost_basis` | Total cost. |
| `purchase_price` | Per-share cost. |
| `currency` | Of the values. |
| `created` | Epoch; when the position record was created. Not a purchase date. |

What it **doesn't** send: lots, purchase dates, or trades as structured
data. A brokerage's transactions arrive as cash rows such as
`DIVIDEND VTI $12.34` or `YOU BOUGHT VTI -$2,200.00`, with no share count.

### What Ledgeline has today (survey, 2026-11-15)

- Holdings are computed from journal postings in a commodity, one
  average-cost pool per symbol; basis only from `@`/`@@`
  (`ledgeline-core/src/holdings/engine.rs`).
- Prices: `P` lines from Yahoo, written into the file with the most `P` lines
  (`prices_api.rs` `plan_update`/`apply_update`, `auto_target`).
- `assertions.rs` already checks `= 10 VTI` per commodity, as hledger does.
  Nothing writes assertions.
- Sync's balance check (`sync/engine.rs` `balance_check`) gives up on an
  account holding more than one commodity, i.e. every brokerage.
- No importer brings in investment trades except QIF.

## 2. Unknowns to settle on real data first

Each changes the design. Patrick runs this. It prints counts and yes/no
answers, no names or amounts, and costs 1 of the ~24 daily requests:

```sh
URL=$(security find-generic-password -s ledgeline.sync -a ledgeline.sync.<conn> -w)
curl -s "$URL/accounts?start-date=$(date -v-30d +%s)" | jq '[.accounts[]
  | select((.holdings // []) | length > 0)
  | ((.balance | tonumber) - ([.holdings[].market_value | tonumber] | add)) as $d
  | {
      holdings: (.holdings | length),
      blank_symbols: ([.holdings[] | select((.symbol // "") == "")] | length),
      fractional_shares: ([.holdings[] | select((.shares | tostring) | test("\\."))] | length),
      balance_vs_holdings: (if ($d < 1 and $d > -1) then "equal"
                            elif $d > 0 then "balance higher (cash outside holdings?)"
                            else "balance lower" end),
      transactions: (.transactions | length),
      trade_like_rows: ([.transactions[] | select(.description | test("BUY|BOUGHT|SELL|SOLD|REINVEST"; "i"))] | length),
      currencies: ([.holdings[].currency] | unique)
    }]'
```

Also from Patrick, in words: how the journal names these commodities today
(the ticker, a quoted fund name, something else), and whether any of these
accounts are already kept with share postings by hand or by QIF.

## 3. Design

### H1. Read holdings (core, no UI)

- `FeedAccount.holdings: Vec<FeedHolding>`, parsed in `simplefin.rs`. Decimal
  fields are validated like amounts; a bad one drops that holding with a
  read problem, never the account.
- The symbol is sanitized like any bank text. A blank symbol keeps the
  description as its display name.
- The holdings are kept in the sync state file (machine-local, like the last
  balance), not in the journal or `ledgeline.toml`.

### H2. Position check (read-only)

The safe, high-value part, and the only one that should ship before the
open questions below are answered.

- **Commodity mapping, per linked account**, stored on the link in
  `ledgeline.toml`:

  ```toml
  [[sync.account]]
  remoteId = "ACT-vanguard-brokerage"
  # …
  [sync.account.commodities]
  VTI = "VTI"
  "" = "$"          # the blank-symbol/cash position, if there is one
  "Plan Fund 2050" = "TDF2050"
  ```

  The mapping is filled at link time. Identity is used when the journal
  already has that commodity. An unmapped symbol shows as "Not in your
  journal yet: map it or add it." Mappings are editable in the account
  drawer.
- **Compare** each mapped holding's shares with the journal's balance of
  that commodity under the linked account's subtree at the snapshot's
  `balance-date`. Use the holdings engine's share count, not a raw sum, so
  splits are handled the same way.
- **New problem code `positions`** (warning, on the account), for example:
  "Vanguard says 152.31 VTI; your journal has 150 VTI on 2026-11-14." The
  remedies:
  - **Show the difference**: a per-symbol table;
  - **Add an assertion** (below);
  - **Dismiss until it changes**.
- **The balance check learns brokerages.** When the account holds
  securities, compare the bank's balance with the journal's cash plus Σ
  journal shares × the bank's own per-share price (`market_value ÷ shares`).
  That keeps "Ledger differs from bank" meaningful there, instead of giving
  up as it does today.
- **Assertions on request, never automatically.** A **Add a balance
  assertion** button writes `= 152.31 VTI` on a zero-amount posting dated
  `balance-date`. It goes into the account's sync target file, through the
  same import mutex and pinned writer as sync. It uses hledger syntax the
  checker already reads, so a later mismatch shows on the Problems page too.
  This is plan 43 Phase 4's "auto balance assertions", made explicit.

### H3. Prices from holdings

- `market_value ÷ shares` on `balance-date` is a price per mapped symbol.
- Write it as a `P` line **only when the journal has no `P` for that symbol
  on that date**. Yahoo's close, when present, wins. Use the existing prices
  writer and file choice (`plan_update`/`apply_update`/`auto_target`),
  tagged so its origin is visible:

  ```journal
  P 2026-11-14 TDF2050 $31.47  ; source:simplefin
  ```

- When: with the sync's own write. On **Sync now**, and in the background
  only when `autoSync` is on (decision 1 of plan 43). A per-link switch,
  `prices = false`, turns it off.
- The Holdings page's price source column reads "bank (SimpleFIN)" for
  these.

### H4. Keeping positions in step (opt-in, per account) — needs decisions

For accounts whose transactions never carry shares: most brokerages over
SimpleFIN, and 401(k)s, which often send no transactions at all. Three
candidate behaviours, which aren't exclusive:

- **(a) Assertions only (H2).** The user fixes the journal by hand.
  Safest; most work for the user.
- **(b) Position-update transactions.** When the shares differ, sync
  proposes one transaction per snapshot:

  ```journal
  2026-11-14 Vanguard position update  ; id:<short>-pos-2026-11-14, source:simplefin
      assets:invest:vanguard:VTI    2.31 VTI @ $220.12
      assets:invest:vanguard:cash
  ```

  The offset account is chosen per link:
  - the account's cash, if the buy's cash row was categorized to a
    clearing account;
  - an income account, for a 401(k) contribution;
  - `equity:adjustments`.

  It is held for review like a possible duplicate, never written unseen.
  The cost is the snapshot price, so basis is approximate and the drawer
  says so.
- **(c) Route trade cash through a clearing account.** The sync rules file
  sends `BUY|BOUGHT|SOLD` rows to `assets:invest:<acct>:pending-trades`,
  and (b)'s update offsets against it. The clearing account then nets to
  zero when the two agree, which makes mismatches visible. It is the most
  faithful option, and it depends on what real trade rows look like (§2).

**Recommendation:** ship H1–H3. Then decide H4 from the §2 data. If trade
rows are recognizable, (b) + (c); for 401(k)s with no transactions, (b)
against an income account the user picks.

### Not doing

- **Lots or tax-lot basis from `cost_basis`.** It's a total, not lots.
  Show it beside the journal's basis in the drawer ("Vanguard: $30,000 ·
  your journal: $29,880") and never write it.
- **Trades parsed from descriptions.** Banks phrase them differently, and
  a regex that books shares would be wrong quietly.

## 4. GUI

- **Account drawer, Holdings section** (investment links only):
  - a symbol → commodity mapping table with a per-row status (✓ matches ·
    differs by N · not in journal);
  - the cost-basis comparison;
  - the "Add a balance assertion" button;
  - the per-link price switch.
- **Accounts tab row:** "3 positions · all match" or "1 position differs"
  beside the balance line.
- **Holdings page:** the price source shows "bank (SimpleFIN)". A
  position that differs gets a small ⚠ linking to the drawer.
- **Docs:** `docs/imports/bank-sync.md` gets "Investment accounts";
  `docs/holdings/prices.md` mentions the bank as a price source.

## 5. Testing

- Core:
  - holdings parsing (good, bad decimals, blank symbol), with the demo
    fixture's `holdings` extended;
  - the position comparison against `fixtures/reports/` journals with
    splits;
  - the brokerage balance check.
- Server:
  - the fake bridge gets a brokerage account with 3 holdings: one
    matching, one differing, one with no symbol;
  - endpoint tests for the `positions` problem;
  - the assertion write (byte-exact);
  - `P` writes that respect an existing Yahoo `P`.
- e2e and screenshots on the fake bridge, as in plan 43.
- Real-bank QA by Patrick: run the §2 shape check, then the drawer's mapping
  on his accounts.

## 6. Open questions for Patrick

1. Is H1–H3 first, H4 later, the right order?
2. Bank prices: write them only when Yahoo has none for that date
   (recommended), or always prefer the bank's for that account?
3. H4: which of (a), (b) and (c)? This can wait for the §2 data.
4. Assertions: button only (recommended), or an opt-in per link that
   writes one each sync?
