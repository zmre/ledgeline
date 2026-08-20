# 14 — Other holdings (non-stock, non-cash assets)

Splits the Holdings tab into **Stocks** and **Other**, where Other shows every
tracked asset that is neither a security nor cash: a house, a car, a partnership
interest, a receivable. Driven by the README TODO "feat: non-stock and cash
holdings".

## The complaint being fixed

> "there's no nice way to see tracked assets that aren't stock or cash. they show
> up in the financial reports, which is good, but are otherwise less visible. an
> individual who owns a house and a car and maybe is invested in a partnership
> might expect to see those things under Holdings, but we only show stocks today."

## Why this cannot be a filter on the existing engine

The stock engine is **symbol-keyed** and its first filter drops every currency
amount (`holdings/engine.rs:764`). A house booked as `$150,000.00` produces no
pool, no symbol, and therefore no row — it is not hidden by a filter, it is
structurally invisible. Other holdings are **account-keyed**: the thing you own
is the account, and its value is that account's balance. Different key, different
engine. They share the scope, the tag vocabulary, the valuation base, and the
wire shape of the trend series — nothing else.

## Decisions (locked with Patrick 2026-08-19)

1. **Tag-driven override with a mechanical default.** An account holding a
   non-currency commodity is a stock by default; a `holdings:` tag overrides that
   in either direction. Declared, never name-matched — see below.
2. **Columns mirror the Stocks tab**: Value, Cost, Change, Change %, driven by
   the same All / YTD / 12mo window control that already sits in the scope bar.
3. **Table + trend chart**, no pie and no gainers/losers. Both say little about
   three illiquid assets, and a pie of "house, car" is a pie of two slices.

## Why `holdings:` is a code, not prose

Same reason as `type:` and `issection:`, recorded in [[account-type-not-name]]: a
classification that decides *membership* must never match English words, because
the failure mode is a tab that reads **empty** and a chart of accounts may be in
any language. `holdings:` is a closed vocabulary of three ASCII codes.

| Value    | Meaning                                                                 |
|----------|-------------------------------------------------------------------------|
| `other`  | Other tab, whatever it holds. Suppressed from Stocks.                    |
| `stocks` | Stocks tab only. Suppressed from Other, even if it holds only currency.  |
| `none`   | Neither tab. Still on the balance sheet — this hides clutter, not money. |

Absent → the default rule (§ Membership). An unknown value is **refused**
(`ReportError::UnknownHoldingsClass` → HTTP 400 naming the three codes),
following `issection:` rather than `type:`.

That was reversed during implementation. The first draft said "ignored, exactly
as an unknown `type:` is", and `reports/mod.rs:97-113` is the argument against
it: a closed vocabulary that decides *membership* must fail loudly, because a
silent fallback returns the account to the mechanical default and the user who
wrote `holdings: real-estate` to move their house finds it still on the Stocks
tab with nothing on screen to say why. `type:` is lenient because hledger
journals are expected to have types whether or not anyone declared them;
`holdings:` is pure opt-in, so the only reason to write one is to change
something, and a write that changes nothing is worth a sentence.

Inheritance is `type:`'s: the account's own tag, else the nearest declared
ancestor's. `holdings:` on `assets:property` covers `assets:property:house:land`
without restating it.

```journal
account assets:property:house   ; type: A, holdings: other
account assets:vehicles:van     ; type: A
account assets:partners:acme    ; type: A
account assets:broker:taxable   ; type: A
account assets:receivable:petty  ; type: A, holdings: none
```

`assets:property:house` may hold `1 HOUSE` priced by `P` directives and still
appear under Other, because the tag beats the commodity. That matters: booking an
asset as its own commodity is the only way a dollar journal makes a house
*revalue*, and revaluation is the whole point of "change in value over time".

## Membership

An account is an **other holding** at `as_of` when all three hold:

1. Its balance at `as_of` (as written, all commodities, subtree-exclusive) is
   non-zero.
2. `resolve_account_type(account, declared) == Some(AccountType::Asset)` —
   **exact match**. `is_account_type(.., Asset)` is wrong here: `Cash` folds into
   `Asset` (`account_types.rs:243`), so it would drag every `type:C` account in,
   which is precisely what the user asked to exclude. Liability, Equity, Revenue,
   Expense, Conversion, Gain and unresolvable are all out.
3. Its effective `holdings:` classification is `Other`, or is absent **and** the
   account holds no non-currency commodity at `as_of` (`is_currency`, the same
   predicate the stock engine uses).

The stock engine gains the mirror of rule 3: accounts classified `Other` or
`None` are dropped from its scope. Nothing may appear on both tabs — a house
counted twice is worse than a house counted nowhere.

**Rows are holdings, not accounts.** The first cut was flat — one row per
posting-bearing account — and the very first real chart it met broke it:

```journal
account assets:home:cost        ; type:A
account assets:home:unrealized  ; type:A
```

Two rows for one house, sorted apart by value, and each one's cost equal to its
own balance so BOTH reported a change of zero. The tab's entire subject read as
nothing. Roll-up is therefore not a nicety; see § Roll-up below.

## Roll-up: which accounts form one row

Decided with Patrick 2026-08-19, after the flat version met a real chart.

1. **An explicit `holdings:` tag wins** — the nearest tagged ancestor-or-self owns
   the row.
2. **Otherwise roll up to a purely-container parent**: no postings of its own,
   its posting-bearing descendants all direct children, at least two of them.

Rule 2 is applied ONCE, never iterated, and both of its clauses earn their keep:

- *Shallow* is what keeps `assets:partnerships:angel-continuity` a row while
  leaving `assets:partnerships` alone — the fund's children are leaves, the
  portfolio's child is not.
- *At least two* is what stops a lone `assets:vehicles:car` being relabelled
  `assets:vehicles`, which would trade a named row for an unnamed one and gain
  nothing, since there is nothing to merge.

The tag override exists for the case rule 2 declines: a single-child fund reads
as `…:second-fund:contributed` until you tag `…:second-fund`.

## The `valuation:` tag, and why it is not `holdings:`

`holdings:` decides which TAB; `valuation:` decides what an account MEANS once it
is on one. Closed vocabulary, `cost` (the default) and `unrealized`, refused by
name when misspelt.

Keeping them apart was Patrick's call and it is the split this codebase already
makes twice: `type:` picks the statement section and `bsgroup:` the line within
it; `issection:` picks the box and `isgroup:` the line inside it. Membership and
role are never one tag. Overloading `holdings:` would have made "move this to the
Other tab" and "this is a paper gain" the same sentence.

**No inference.** An untagged mark counts as money in, so cost equals value and
the change reads `$0.00`. The rejected alternative — treat value arriving from a
revenue account as a mark — reads a reinvested distribution as a paper gain when
it genuinely raises basis, and a wrong Cost column is worse than an honest zero.

## Valuation

Prices come from explicit `P` directives **plus** costs inferred from `@`/`@@`
annotations — `infer_market_prices`, the same source `net_worth` uses and for the
same reason: an account whose only price evidence is the annotation on its own
purchase should still show that value. This deliberately differs from the balance
sheet (explicit `P` only, matching `hledger bs -V`) and from the stock engine
(directive first, cost annotation as a per-symbol fallback). Stated here because
the three reports disagreeing silently would be a bug.

| Field        | Definition                                                                    |
|--------------|-------------------------------------------------------------------------------|
| `value`      | The WHOLE subtree at `as_of`, market-valued in `base`. `None` if any held commodity is unpriceable. |
| `cost`       | The subtree's non-`unrealized` accounts at cost (`-B`), valued into `base`.    |
| `reference`  | `cost` when `gain_since` is `None`; else `value` recomputed at `gain_since`.    |
| `change`     | `value − reference`.                                                            |
| `change_pct` | `change / reference × 100`; `None` when `reference` is missing or zero.          |

The `reference` rule is the stock engine's rule verbatim (`types.rs:30-40`), so
the window control means the same thing on both tabs. For a dollar-booked van,
all-time change is honestly `$0.00` — cost is value. That is the correct answer,
not a missing feature.

Under a bounded window the reference has **three** cases, and the last two are
easy to collapse into one by accident (the first implementation here did):

| At the window start the account was… | Reference   |
|---------------------------------------|-------------|
| not held                              | `0`         |
| held and priceable                    | its value   |
| held but **unpriceable**              | `None`      |

The third must propagate null. Treating it as zero reports the asset's entire
current value as that window's change — a fabricated figure, and a plausible
enough one to go unnoticed. Pinned by
`an_asset_unpriced_at_the_window_start_has_an_unknown_change`, with
`an_asset_not_held_at_the_window_start_references_zero` guarding the case it
must not be confused with.

Totals are the engine's, summed over rows that carry the needed input, and never
recomputed in the UI. Unpriced rows contribute to no total and raise a warning,
exactly as on the Stocks tab.

## Interface contracts

```rust
// crates/ledgeline-core/src/holdings/classify.rs
pub enum HoldingsClass { Stocks, Other, None }

/// Nearest declared `holdings:` tag (own, then ancestors); `None` = untagged.
pub fn declared_holdings_classes(accounts: &[AccountDeclaration]) -> BTreeMap<String, HoldingsClass>;
pub fn parse_holdings_tag(value: &str) -> Option<HoldingsClass>;
pub fn resolve_holdings_class(account: &str, declared: &BTreeMap<String, HoldingsClass>) -> Option<HoldingsClass>;

// crates/ledgeline-core/src/holdings/other.rs
pub struct OtherHolding {
    pub account: String,          // full path
    pub name: String,             // nearest declared `name:`, else the last segment
    pub commodities: MixedAmount, // as written — lets the UI show "1 HOUSE"
    pub value: Option<Dec>,
    pub cost: Option<Dec>,
    pub change: Option<Dec>,
    pub change_pct: Option<f64>,
}

pub struct OtherHoldingsTotals { pub value: Dec, pub cost: Option<Dec>,
                                 pub change: Option<Dec>, pub change_pct: Option<f64> }

pub struct OtherHoldingsReport {
    pub as_of: String,
    pub base: String,
    pub holdings: Vec<OtherHolding>,   // value desc, unpriced last, then by account
    pub totals: OtherHoldingsTotals,
    pub warnings: Vec<OtherHoldingsWarning>,
}

pub fn other_holdings(txns, prices, accounts, scope: &HoldingsScope)
    -> Result<OtherHoldingsReport, ReportError>;

/// Reuses `HoldingsPoint`/`HoldingsSeries` unchanged, so `HoldingsTrend.svelte`
/// renders the Other trend with no new chart code: `market_value` is the summed
/// row values at each bucket end, `basis` the summed costs.
pub fn other_holdings_series(txns, prices, accounts, scope, interval, count)
    -> Result<HoldingsSeries, ReportError>;
```

## HTTP

```
GET /api/holdings/other?asOf=&accounts=&mode=&gainSince=&valueIn=
GET /api/holdings/other/series?asOf=&accounts=&mode=&interval=&count=&valueIn=
```

Same query parsing as `/api/holdings` (`holdings_scope`, `parse_mode`,
`resolve_value_in`), same `compute(...)` blocking wrapper. Wire types live beside
the existing `Wire*` block in `reports_api.rs` and use `WireDec`/`WireMixed`;
nulls are kept, not omitted. The series response reuses `WireHoldingsSeries`
byte-for-byte. Both URIs are added to `fixtures/native/v1/requests.tsv`, which is
what makes the Rust golden and the TS decode test cover them.

## UI

- `web/src/lib/holdings/params.ts` — pure, node-testable: `HoldingsTab =
  "stocks" | "other"`, `TAB_ORDER`, `TAB_LABELS`, `isTab`. Mirrors
  `lib/imports/params.ts`.
- `HoldingsTabs.svelte` — a copy of `ReportTabs.svelte`: `role="tablist"`,
  `tabs tabs-border`, `aria-selected`, digit bindings. So the e2e convention
  (click by role, assert `aria-selected`) works unchanged.
- The tab is **not** part of `HoldingsScope`. Scope is the resource's refetch key
  (`stores/holdings.svelte.ts:89`); putting the tab there would refetch the stock
  report on every tab click. It is separate state, carried in the same query
  string by extending `holdings/ui/urlCodec.ts` — one writer, no second
  `searchMirror`.
- Other is fetched **lazily**, the first time its tab is opened. A user who never
  clicks it pays nothing.
- `KeyGroup` gains `"Holdings"` (`lib/keys/types.ts:7,9`). `HoldingsTable`'s
  bindings currently borrow `"Journal"`; they move too, or the help drawer files
  one holdings feature under two headings.

## Known sharp edge — `group_rank` has one rank space for two sections

Noticed while pinning the balance-sheet tests against this epic's fixture
accounts. Not a bug today; recorded so it is a decision rather than a surprise.

`group_rank` (`reports/account_groups.rs`) sorts a group by its position in
`BUILT_IN_ORDER`, and returns `BUILT_IN_ORDER.len()` — one value, the same one —
for everything not in that list. So:

- **All custom groups tie.** Every tag-sourced and segment-sourced group gets the
  identical rank and is ordered purely by the name tiebreak. `Property` before
  `Vehicles` is alphabetical accident, not intent. There is currently no way to
  say "show Property above Vehicles" short of renaming one.
- **Assets and liabilities share that one rank space.** `BUILT_IN_ORDER` mixes
  them — `Cash and cash equivalents`, `Accounts receivable`, `Investments` are
  asset groups; `Credit cards` and `Accounts payable` are liability groups — and
  the ranks run across all five. An ASSET group a user names `Accounts payable`
  via `bsgroup:` would therefore be hoisted to rank 4 and sort above `Property`
  inside the Assets section, as though it were the liability built-in it merely
  shares a name with.

Nothing is visibly wrong right now, because groups are ranked only within a
section and no section mixes assets and liabilities — the liability entries are
unreachable while sorting Assets and vice versa. It is a latent trap, not a live
defect: it needs a user to pick a colliding name before anything looks odd, and
even then the number is right and only the row order is strange.

If it is ever worth fixing, the shape is to make the rank a
`(section, position)` pair rather than a bare index into one flat list, so the
built-in order is scoped to the section it describes. Deliberately not done here
— this epic had no reason to touch group ordering, and the fix wants its own
test.

## Definition of done

- `just check` and `just test` green; `cargo test` green; `cargo clippy -D warnings` clean.
- Unit tests for the classifier, the membership rule, the reference/window math,
  and the params codec round-trip.
- Integration test over a fixture carrying a house, a van and a partnership,
  including a commodity-booked house forced to Other by the tag.
- The `holdings:` tag documented in `docs/` next to `bsgroup:`/`issection:`.
- Works dark, at 375px and desktop.
