# The balance sheet, and the `bsgroup:` / `bsterm:` tags

The balance sheet is three boxes — **Assets**, **Liabilities**, **Equity** — whose
lines are *groups* rather than raw accounts, valued into a single currency so
every line is one number. Under the boxes is a spreadsheet tie-out that proves the
statement balances.

This page covers how lines are chosen, how to control them with `bsgroup:`, how
`bsterm:` splits each box into current and non-current, what the equity lines
mean, and why the balance check has a tolerance.

Design notes and the reasoning behind each decision live in
[`plans/12-balance-sheet-redesign.md`](../plans/12-balance-sheet-redesign.md).

## Why groups instead of accounts

A chart of accounts is organised for bookkeeping; a balance sheet is organised for
reading. You want to know what you hold in *cash* and in *investments* — not how
much is in bank account A versus bank account B, nor how many shares of each
ticker. So each line is a group, collapsed by default. Expand one to see the
accounts behind it, at full depth.

There is no depth control on this tab. Depth stopped meaning anything useful once
groups became the reading: a group's roots are shown whatever their depth, and the
drill-down shows everything below them.

## The `bsgroup:` tag

Tag any account declaration with `bsgroup:` and it becomes a line on the balance
sheet:

```journal
account assets:property:house      ; type: A, bsgroup: Property
account assets:broker:ira          ; type: A, bsgroup: Long-term investments
account liabilities:mortgage       ; type: L, bsgroup: Long-term debt
account liabilities:card:visa      ; type: L, bsgroup: Short-term debt
account assets:invoices            ; type: A, bsgroup: Accounts receivable
```

The tag **inherits to sub-accounts**, exactly like `type:` does, so tagging
`assets:property` puts every account beneath it on the Property line unless one of
them overrides with its own `bsgroup:`.

Group names are free text — use whatever line items your books call for
(`Inventory`, `Deferred revenue`, `Paid-in capital`, `Intangible assets`,
`Accumulated depreciation`, …). Accounts that share a name share a line.

### Three gotchas, all inherited from hledger's syntax

**A comment on an `account` directive needs TWO spaces before the `;`.** An
account name may contain single spaces, so a single space cannot end one — with
only one, hledger reads the whole rest of the line as the account's NAME and
there is no comment left to carry your tags:

```journal
account assets:art ; type: A, bsgroup: Art    ; ✗ declares an account literally
                                              ;   named "assets:art ; type: A, bsgroup: Art"
account assets:art  ; type: A, bsgroup: Art   ; ✓ two spaces
```

This one is nasty because nothing complains. The journal parses, the declaration
is simply about an account no posting will ever mention, and the real
`assets:art` quietly has no type and no group. If a tag you definitely wrote
seems to have no effect, count the spaces first. (Only `account` is affected —
`commodity` and `payee` values cannot contain spaces, so one is enough there.)

**A tag value ends at the next comma.** This is how hledger parses all tags, and
it bites here because group names are prose:

```journal
account assets:art   ; type: A, bsgroup: Art, antiques     ; ✗ group is "Art"
account assets:art   ; type: A, bsgroup: Art and antiques  ; ✓ group is "Art and antiques"
```

**The tag name is the last word before the colon**, so don't put a space inside
it — `; balance sheet group: Cash` declares a tag called `group`, not `bsgroup`.

## Current vs non-current (`bsterm:`)

Tag an account `bsterm: noncurrent` and the Assets and Liabilities boxes split
into the standard subheadings, each with its own subtotal:

```journal
account assets:property:home  ; type: A, bsterm: noncurrent
account assets:vehicles:car   ; type: A, bsterm: noncurrent
account liabilities:mortgage  ; type: L, bsterm: noncurrent
```

```
ASSETS
  CURRENT
      Cash and cash equivalents      49,059.99
  Total current assets               49,059.99
  NON-CURRENT
      Investments                    10,552.63
      Property                      468,000.00
      Vehicles                       20,500.00
  Total non-current assets          499,052.63
  Total Assets                      548,112.62
```

A subheading opens a band and its subtotal closes it, both a step to the left of
the group lines between them. The subheading carries no figure of its own — the
band's total is the subtotal, and printing it at both ends would invite you to
look for a difference between them. The subtotal takes a thin rule where the
section total below it takes a double rule and a fill, because one is a part and
the other is the whole.

**It is adaptive.** A journal that declares no `bsterm:` anywhere gets exactly
the balance sheet it got before this existed — no subheadings, no subtotals,
nothing to dismiss. The split appears the moment one account asks for it, which
is the same rule the income statement's GAAP ladder follows.

**Defaults, once it is on.** You tag the long-term things and leave the rest:

| Group                            | Default        |
|----------------------------------|----------------|
| `Investments` (the built-in)      | non-current    |
| everything else untagged          | current        |

So the brokerage above needed no tag. Only three accounts in that example carry
one.

`bsterm:` **inherits to sub-accounts** like every other tag here, and it is a
closed vocabulary — `current` and `noncurrent` (plus `short`/`long` spellings) —
refused by name when misspelt, because a term that quietly falls back files a
balance into the wrong subtotal and leaves the statement looking fine.

**Equity is never split.** The question the split asks — when does this become
cash, when does this come due — is not one you ask of capital.

### Three tags, three questions

It is a third tag rather than a value of `bsgroup:` because they answer different
questions, and one tag cannot answer two:

| Tag        | Question                          |
|------------|-----------------------------------|
| `type:`    | Which box?                        |
| `bsterm:`  | Which half of the box?            |
| `bsgroup:` | Which line within that half?      |

One consequence worth knowing: a group is keyed by (term, line), so a single
`bsgroup:` whose accounts straddle the halves prints as **two lines** under two
subheadings. That is not a defect — a receivable due this year and one due in
five really are two lines on a real statement.

## How a line is chosen when there is no tag

Untagged accounts still get sensible lines. Resolution stops at the first match:

| # | Rule | Line | `source` |
|---|--------------------------------------------------|--------------------------------|-------------|
| 1 | `bsgroup:` on the account itself | the tag's value | `tag` |
| 2 | `bsgroup:` on the nearest declared ancestor | the tag's value | `tag` |
| 3 | Effective account type is `C` (cash) | **Cash and cash equivalents** | `type` |
| 4 | An asset holding a non-base commodity | **Investments** | `commodity` |
| 5 | Otherwise: the account's second path segment | e.g. `assets:bank:…` → **Bank** | `segment` |

Step 5 is what makes an untagged journal read well: `assets:bank:chase` and
`assets:bank:ally` both land on one **Bank** line, which is the "not bank account A
vs. bank account B" behaviour you want, without configuring anything. A
single-segment account falls back to its root, so a bare `equity` posting still
gets a line.

Three conventional abbreviations get prettier labels at step 5 — `cc` → Credit
cards, `ar` → Accounts receivable, `ap` → Accounts payable.

### Why grouping never matches account *names*

Steps 3 and 4 are driven by the declared `type:` and by which commodities an
account actually holds. Step 5 is driven by position in the account tree. None of
them matches English words, because a chart of accounts may be in any language and
may use roots like `cogs:` — and when name-matching classification goes wrong, the
symptom is a report that reads **zero**, not one that merely looks odd.

The alias table above is the single exception, and it is deliberately cosmetic: it
runs *after* membership is decided and can only rename a line, never move an
account onto a different one.

### Ordering

Known lines come first in balance-sheet order — Cash and cash equivalents,
Accounts receivable, Investments, Credit cards, Accounts payable — then everything
else alphabetically. The two computed equity lines always sort last.

## Valuation

Lines are **valued at market in your base currency** by default, so a portfolio
reads as money rather than as a column of share counts. Prices come from the
journal's `P` directives only, matching `hledger bs -V`; costs are not inferred
into prices.

A commodity with no usable `P` directive on or before the report date is **not**
silently dropped. It stays on its line as a secondary figure and is named in a
warning banner above the report. If your stocks show as share counts, that is the
fix: add `P` directives.

`?value=cost` and `?value=none` are available on the API for the cost basis and
for raw, unvalued commodities.

## The equity box

Equity holds your declared equity accounts plus up to two computed lines, which is
what makes `assets = liabilities + equity` actually hold:

- **Retained earnings** — all revenues minus all expenses through the report date,
  at cost. The same quantity the income statement calls net income.
- **Valuation adjustment** — the sheet on the display basis minus the same
  accounts at cost. At `value=cost` it is zero and disappears; at market it is the
  unbooked revaluation.

The second line is *not* called "unrealized gains" on purpose. The same
subtraction also absorbs currency revaluation and unpriced holdings, so it can
legitimately carry several commodities at once — and "933,25 EUR of unrealized
gains" is not a sentence a balance sheet can say.

## The balance check

Under the boxes, `Liabilities + Equity` is set against `Total Assets`. When they
agree the statement is marked balanced; when they don't you get a warning with the
exact residual, because that means the journal itself has a problem.

It catches two things:

1. a transaction whose postings don't sum to zero at cost, and
2. an account whose type can't be resolved — it lands in no section at all, the
   failure mode where a report quietly reads zero.

### Why there is a tolerance

The check has a tolerance, and it is not a fudge. A priced posting is worth more
decimal places than the cash paying for it can be written to:

```journal
26.2690 VTI @ $289.7713   =  $7,612.00227970    settled with   $-7,612.00
```

That entry is *legal* — hledger 1.52 accepts it without complaint — yet it leaves
`$0.00227970` behind. Over a real journal that dust accumulates, and holding the
sum to exact zero would have meant flagging journals that no ledger could ever
satisfy. There is no way to write $0.0022797 into a bank account.

So: a residual counts as dust when, **for every commodity, it is strictly smaller
than that commodity's tolerance**, and the tolerance is the wider of two things:

```
tolerance = max(one unit of the precision you write that commodity at, 0.01)
```

The first term is why it is not simply a cent: share quantities aren't
denominated in cents, so a book that writes `5 GLD` in whole shares gets a whole
share of tolerance, and not every currency has two decimal places.

The second term is a **one-cent floor**, and it is a deliberate decision rather
than a derivation — the balance sheet ignores imbalances below one cent, full
stop. Without it the tolerance was a function of the most finely written posting
anywhere in your journal, which has nothing to do with how large a rounding
residue can get: one line of brokerage interest written as `$0.0327` dropped the
threshold to `$0.0001` and the dust above was flagged all over again. The floor
only ever *loosens* the rule — a commodity written in whole units keeps its
tolerance of one unit.

### What the floor still catches, and what it gives up

An account whose type can't be resolved contributes its **whole balance** to the
residual, so the "report reads zero" failure is untouched unless the account is
genuinely worth less than a cent. A transaction that doesn't balance is caught
too whenever it is worth catching — being out by `$10.00` is a thousand times the
floor.

What the floor gives up is this report noticing a *sub-cent* transaction
imbalance in a journal that writes finer than cents. That one isn't lost: every
entry is separately checked against hledger's own much tighter rule — half a unit
at the precision that entry was written at — and shows up in Problems as an
unbalanced transaction. The balance sheet is the second line of defence there,
never the only one.

One caveat worth stating: the floor is one hundredth *of the commodity*, not of a
dollar. On a book denominated in something with a high unit value, 0.01 units is
not a negligible sum. The exact residual is always reported, whatever the
verdict, so the number is there to look at.

## Export

The XLSX export mirrors the screen: filled section headers, bold group rows,
indented accounts, ruled subtotals, the tie-out and its verdict, then net worth.
Because every line is valued into one commodity, the amount column holds real
numbers with a number format rather than text, so the workbook is arithmetic you
can build on. Anything unpriced goes in its own column rather than being dropped.
Groups are written expanded regardless of what is collapsed on screen — an
exported statement shouldn't depend on which disclosures happened to be open.

## API

```
GET /api/reports/balancesheet/grouped
      ?asOf=YYYY-MM-DD        (default: today)
      &depth=N                (optional; omit for full depth, 0 for totals only)
      &value=market|cost|none (default: market)
      &valueIn=$              (default: the journal's base commodity)
```

The older `GET /api/reports/balancesheet` returns the flat, unvalued,
`hledger bs`-shaped report and is unchanged — it backs the hledger parity golden.
