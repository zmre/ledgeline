# Holdings, and the `holdings:` tag

The Holdings tab has two sub-tabs:

- **Stocks** — securities, one row per commodity, with average-cost basis and
  unrealized gain.
- **Other** — everything else you own that is neither a security nor cash: a
  house, a car, a partnership interest, a receivable. One row per **account**,
  with its value, its cost, and how much it has changed.

This page covers which tab an account lands on, how to override that with
`holdings:`, what "change" means on each, and how a non-stock asset gets a value
in the first place.

Design notes and the reasoning behind each decision live in
[`plans/10-stock-holdings.md`](../plans/10-stock-holdings.md) and
[`plans/14-other-holdings.md`](../plans/14-other-holdings.md).

## Why two tabs and not one list

They are keyed differently, and no single table can be keyed both ways. A
security is a *commodity*: you hold 19.5 AAPL across three accounts, and the
interesting row is AAPL. A house is an *account*: `assets:property:home` is the
thing, and what it holds is an implementation detail. Averaging cost across lots
makes sense for the first and is meaningless for the second.

## Which tab an account lands on

Without any tag, the rule is mechanical:

| The account…                                              | Tab            |
|-----------------------------------------------------------|----------------|
| holds a non-currency commodity (`AAPL`, `VTI`, `BTC`)      | **Stocks**     |
| is `type:A`, not cash, and holds only currency             | **Other**      |
| is `type:C` (cash)                                         | neither        |
| is a liability, equity, revenue or expense account         | neither        |

"Not cash" is exact: hledger folds `type:C` into Asset for most purposes, but the
Other tab tests for `type:A` *specifically*, or every bank account would appear
on it.

## The `holdings:` tag

Tag an account declaration to override the mechanical rule:

```journal
account assets:property:home     ; type: A, holdings: other
account assets:crypto:cold       ; type: A, holdings: stocks
account assets:receivable:petty  ; type: A, holdings: none
```

| Value    | Meaning                                                                  |
|----------|--------------------------------------------------------------------------|
| `other`  | Other tab, whatever it holds. Removed from Stocks.                        |
| `stocks` | Stocks tab only. Never Other, even if it holds nothing but currency.      |
| `none`   | Neither tab. Still on the balance sheet — this hides clutter, not money.  |

The tag **inherits to sub-accounts**, exactly like `type:` and `bsgroup:`, so
tagging `assets:property` covers `assets:property:home:land` without restating it.

Unlike `bsgroup:`, this is a **closed vocabulary**: those three words and nothing
else. A misspelling is refused with a message naming the alternatives, rather
than being ignored — see [below](#a-misspelt-value-is-an-error).

### When you need `holdings: other`

Mostly for **an asset booked as its own commodity**, which is the only way a
dollar journal makes something revalue:

```journal
account assets:property:home  ; type: A, holdings: other, bsgroup: Property
commodity 1.0 HOME

P 2024-07-01 HOME $420,000.00
P 2026-06-30 HOME $468,000.00

2024-07-01 * Opening property position
    assets:property:home            1 HOME @ $420,000.00
    liabilities:mortgage        $-336,000.00
    equity:opening
```

`HOME` is not a currency, so without the tag the house would file itself under
Stocks and sit between your index funds. The `bsgroup:` tag beside it is a
separate concern — it fixes the same account's line on the balance sheet, which
otherwise groups it under Investments for the same "holds a non-base commodity"
reason.

## The two ways a non-stock asset changes value

Both work, and the Other tab shows both:

1. **The price moves.** Book the asset as its own commodity, as above, and write
   a `P` directive whenever you revalue it. Cost stays at what you paid; value
   follows the directives.
2. **The balance moves.** Book it in your own currency and write entries that
   adjust it — depreciation, improvements, a partner's capital contribution:

```journal
2026-06-30 * annual vehicle depreciation
    expenses:depreciation          $3,500.00
    assets:vehicles:car
```

A dollar-booked asset's cost and value are the same number by construction, so
its **all-time** change is exactly `$0.00`. That is the honest answer, not a
missing feature — nothing has revalued it, the balance simply went down. Switch
the window to Year-to-date or 12 months and the depreciation shows up as the loss
it is.

## What the columns mean

| Column     | Meaning                                                              |
|------------|----------------------------------------------------------------------|
| Value      | The account's balance at the as-of date, priced in the base currency. |
| Cost       | The same balance at cost (hledger's `-B`).                            |
| Change     | Value − reference (see below).                                        |
| Change %   | Change ÷ reference.                                                   |

The **reference** is chosen by the same window control the Stocks tab uses, and
means the same thing on both:

- **All time** → the reference is *cost*, so change is the gain over what you
  paid.
- **Year-to-date / 12 months** → the reference is the account's *value at the
  start of the window*. An asset you bought inside the window references zero, so
  the whole purchase reads as that window's change.

Totals sum only the rows that carry the input they need. An asset whose commodity
has no price route to the base currency contributes to no total and raises a
warning naming it, rather than being silently counted as zero.

## Valuation sources, and why the three reports differ

| Report            | Prices used                                                |
|-------------------|------------------------------------------------------------|
| Balance sheet     | Explicit `P` directives only (matches `hledger bs -V`)      |
| Other holdings    | Explicit `P` **plus** prices inferred from `@`/`@@` costs   |
| Stocks holdings   | `P` first, then a cost annotation as a per-symbol fallback  |

The Other tab infers from cost annotations for a specific reason: the common case
is a single `1 HOME @ $420,000.00` and no `P` directive at all. Reading only
explicit directives would report that house as unpriced — technically defensible,
practically useless. The balance sheet stays strict because it is claiming parity
with `hledger bs -V`.

## A misspelt value is an error

```
account 'assets:property:home' declares `holdings: hous`, which is not one of
stocks, other, none
```

The Holdings tab shows that sentence and a Retry button; fix the journal and
retry. This follows `issection:` rather than `type:`, and for the same reason: a
tag that decides *membership* must fail loudly, because the alternative is that
the account quietly returns to the tab you were trying to move it off, with
nothing on screen to say why. `type:` is lenient because journals are expected to
have types whether or not anyone declared them; `holdings:` exists only to change
something, so a `holdings:` that changes nothing is worth telling you about.

## Scope, dates and the account chooser

Both tabs share one scope bar: the account filter, the as-of date, and the change
window all apply to whichever tab is open.

The account chooser's options are **not** filtered by the current scope or date,
deliberately. An option that vanished the moment you deselected it could not be
reselected, and one that vanished when you travelled back a month would make a
scope impossible to compose. So it offers every account that could ever be a row,
whether or not it holds anything today.
