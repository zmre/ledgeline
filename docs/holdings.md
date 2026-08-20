# Holdings, and the `holdings:` / `valuation:` tags

The Holdings tab has two sub-tabs:

- **Stocks** — securities, one row per commodity, with average-cost basis and
  unrealized gain.
- **Other** — everything else you own that is neither a security nor cash: a
  house, a car, a partnership interest, a receivable. One row per **holding**,
  with its value, its cost, and how much it has changed. A holding may be one
  account or a small subtree of them.

This page covers which tab an account lands on and how `holdings:` overrides
that, how several accounts become one holding, how `valuation:` separates what
you paid from what it is now worth, and what "change" means on each tab.

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

> **Two spaces before the `;`.** A comment on an `account` directive needs at
> least two spaces in front of it, or hledger reads the whole line as the account
> NAME and none of these tags exist. Nothing warns you. See
> [docs/balance-sheet.md](balance-sheet.md#three-gotchas-all-inherited-from-hledgers-syntax).

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

## One asset, several accounts

A very common way to carry an asset's cost/market split is in the account tree
rather than in commodity costs:

```journal
account assets:home             ; type: A, name: Family home
account assets:home:cost        ; type: A
account assets:home:unrealized  ; type: A, valuation: unrealized
```

Those three accounts are **one house**, and the Other tab reports them as one
row. Two rules decide where a row begins:

1. **An explicit `holdings:` tag wins.** The nearest tagged ancestor owns the
   row. Tag the umbrella when the shape below it is unusual.
2. **Otherwise a purely-container parent is rolled up**: one with no postings of
   its own whose posting-bearing descendants are *all* direct children, of which
   there are at least two.

Rule 2 is deliberately shallow — applied once, never repeated — which is what
keeps two funds apart:

```
assets:partnerships                 <- its child is not a leaf, so NOT a row
  :angel-continuity                 <- children are leaves => ONE row
    :contributed
    :unrealized
  :vintage-2021                     <- its own row
    :contributed
    :unrealized
```

Two consequences worth knowing:

- **The row's name comes from the root.** Put `name:` on `assets:home`, not on
  `assets:home:cost`.
- **A lone child is not rolled up.** `assets:partnerships:second-fund:contributed`
  with no sibling stays its own row, because merging it would only trade a
  specific name for a vaguer one. If you would rather it read as
  `…:second-fund`, tag that account `holdings: other` and rule 1 takes over.

## The `valuation:` tag

Within a holding, `valuation:` says what an account CONTRIBUTES:

| Value          | Meaning                                                       |
|----------------|---------------------------------------------------------------|
| `cost`         | Money actually put in. The default — you rarely write it.      |
| `unrealized`   | A mark-to-market or NAV adjustment.                            |
| `depreciation` | Accumulated depreciation. Same behaviour, honest name.         |
| `adjustment`   | Anything else that moves value without moving basis.           |

The last three are one role under three names. They behave identically —
counted in Value, excluded from Cost — and exist separately because a car's
accumulated depreciation is not "unrealized" and a house's mark is not
"depreciation"; forcing one word on both would make one of them read as a
mistake in the journal that declares it.

It inherits down the tree like every other tag here, and it is a closed
vocabulary: an unrecognised value is refused by name rather than ignored.

This is a **separate tag from `holdings:` on purpose**, and it is the same split
the rest of the app makes twice over: `type:` decides which statement section an
account is in and `bsgroup:` decides its line within that section; `issection:`
decides the box and `isgroup:` the line inside it. Membership and role are never
the same tag. `holdings:` says which *tab*; `valuation:` says what an account
*means* once it is on one.

Without the tag, a mark is just another dollar balance — it lands in Cost as
well as in Value, so the holding reports a change of exactly `$0.00`. That is
the honest answer for a journal that never declared the account to be an
adjustment, and it is precisely the situation the tag exists to fix:

```
                    Value        Cost       Change   Change %
untagged       620,000.00  620,000.00        $0.00        —
tagged         620,000.00  500,000.00  120,000.00     +24.0%
```

A holding whose accounts are *all* marks has no cost side at all. Its Cost and
Change read as em-dashes — unknown, rather than a zero basis and an infinite
gain — and it drops out of those two totals while still counting in Value.

## Two things that are easy to get wrong

### Book the purchase price, not the down payment

If a holding is leveraged, put the **whole purchase price** in the cost account
and leave the loan as an ordinary liability:

```journal
2019-06-01 * buy the house
    assets:home:cost           $700,000.00   ; the PRICE
    liabilities:mortgage      $-550,000.00
    assets:bank:checking      $-150,000.00
```

It is tempting to book only the cash you put in, since that is what left your
pocket. Don't: everything the lender funded then has nowhere to live but the
adjustment account, so Change stops meaning appreciation and starts meaning
"appreciation plus the mortgage". On the house above, marked to $750,000:

```
booked at the price:        cost $700,000  change  +$50,000   +7.1%   <- appreciation
booked at the down payment: cost $150,000  change +$600,000  +400%   <- leverage
```

The balance sheet is unaffected either way — the mortgage is a liability in both
— so the error is invisible everywhere except here.

Note the Other tab reports **gross value**, not your equity in it. A house worth
$750,000 with $530,000 still owed reads $750,000, and the debt against it appears
under Liabilities as usual.

### Depreciating assets need a contra-asset account

Posting depreciation straight at the asset moves cost and value together, so the
loss reads as `$0.00`. Split it, which is textbook bookkeeping anyway:

```journal
account assets:vehicles:car               ; type: A, name: Honda CR-V
account assets:vehicles:car:cost          ; type: A
account assets:vehicles:car:depreciation  ; type: A, valuation: depreciation

2026-06-30 * annual depreciation
    expenses:depreciation                    $3,500.00
    assets:vehicles:car:depreciation
```

The two roll into one row (they are a container parent's only children), so you
get `Honda CR-V — value $20,500.00, cost $28,000.00, change -$7,500.00, -26.8%`
instead of a $20,500.00 asset that has apparently never lost a penny.

## The two ways a non-stock asset changes value

Both work, and the Other tab shows both:

1. **The price moves.** Book the asset as its own commodity, as above, and write
   a `P` directive whenever you revalue it. Cost stays at what you paid; value
   follows the directives.
2. **A mark account moves.** Keep cost and market in sibling accounts and tag the
   adjustment `valuation: unrealized`, as above. This is the approach that needs
   no commodity, and the one most funds and property holdings use.
3. **The balance moves.** Book it in your own currency and write entries that
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
