# The income statement, and the `issection:` / `isgroup:` tags

The P&L is a stack of boxes — **Revenue**, **Expenses** — whose lines are *groups*
rather than raw accounts, valued into a single currency so every line is one
number. Below the boxes sits net income, next to the same period a year's-worth
of days earlier.

Tag your accounts and the statement grows into the full GAAP multi-step form:
Cost of revenue, Gross profit, EBITDA, Operating income, Income before taxes. Tag
nothing and none of that appears. A personal journal never sees a line it has no
use for.

Design notes and the reasoning behind each decision live in
[`plans/13-income-statement-redesign.md`](../plans/13-income-statement-redesign.md).

## Why groups instead of accounts

The same reason as the [balance sheet](balance-sheet.md): a chart of accounts is
organised for bookkeeping, a statement is organised for reading. You want to know
what you spent on *housing*, not the split between `expenses:housing:rent` and
`expenses:housing:insurance`. Each line is a group, collapsed by default; expand
one to see the accounts behind it, at full depth.

It also fixes the thing that made the old report hard to scan. It used to print a
rolled-up `income` row, then every account under it, then a `Total Revenues`
footer holding that same figure a third time. Groups replace the ancestor rows
outright, and no number is printed twice.

There is no depth control on this tab, for the reason it left the balance sheet:
groups are the reading, and the accounts inside one are a drill-down.

## The two tags

**`issection:` picks the box.** It takes one of seven codes and nothing else:

| Code | Box |
|----------------|--------------------------------|
| `revenue` | Revenue |
| `cogs` | Cost of revenue |
| `opex` | Operating expenses (or just "Expenses" — see below) |
| `depreciation` | Depreciation & amortization |
| `interest` | Interest |
| `tax` | Income taxes |
| `other` | Other income & expense |

**`isgroup:` names the line inside that box.** It is free text, in any language:

```journal
account cogs                  ; type: X, issection: cogs
account cogs:hosting          ; type: X, isgroup: Cloud hosting
account cogs:payments         ; type: X, isgroup: Payment processing
account expenses:salaries     ; type: X, issection: opex
account expenses:depreciation  ; type: X, issection: depreciation
account expenses:interest     ; type: X, issection: interest
account expenses:taxes:income  ; type: X, issection: tax
account income:grants         ; type: R, issection: other
```

Both tags **inherit to sub-accounts**, exactly like `type:` does, so tagging
`cogs` covers everything beneath it unless a child overrides. Accounts that share
an `isgroup:` value share a line, even if they live in unrelated parts of the
tree.

An `issection:` value that isn't one of the seven codes is an **error**: the
report fails, naming the account, the value you wrote and the seven codes it
could have been. It is not silently ignored — a mistyped classification tag is
how an account goes missing from a report entirely, and a report that quietly
reads zero is worse than one that refuses to draw.

(Today that surfaces as a failed request on this tab rather than as an entry in
Problems, because Problems entries are anchored to a transaction and an `account`
directive has none. Routing it there instead is a known follow-up.)

### Two gotchas, both inherited from hledger's tag syntax

**A tag value ends at the next comma.** This bites `isgroup:` because group names
are prose:

```journal
account cogs:ops  ; type: X, isgroup: Servers, bandwidth     ; ✗ group is "Servers"
account cogs:ops  ; type: X, isgroup: Servers and bandwidth  ; ✓ the whole phrase
```

**The tag name is the last word before the colon**, so don't put a space inside
it — `; income section: cogs` declares a tag called `section`, not `issection`.

## What happens with no tags at all

Everything of declared type `R` lands in Revenue, everything of type `X` lands in
Expenses, and that is the whole statement: two boxes and a net income figure.

There is deliberately **no guessing** for `cogs`, `tax`, `interest` or
`depreciation`. Every rule that could produce them from an untagged journal would
have to match English account names, and when name-matching classification goes
wrong the symptom is a section that reads **zero**. A journal rooted at `cogs:`
with `type: X` lands in Expenses and reads correctly; splitting it out is one tag.

## How a line is chosen when there is no `isgroup:`

Untagged accounts still group sensibly, by **dropping the prefix every account in
that box shares** and taking the next segment:

| Accounts in the box | Shared prefix | Lines |
|---|---|---|
| `income:salary`, `income:dividends` | `income` | Salary, Dividends |
| `expenses:food:groceries`, `expenses:housing:rent` | `expenses` | Food, Housing |
| `cogs:materials`, `expenses:rent` | *(none)* | Cogs, Expenses |

The shared prefix carries no information — if every expense is under `expenses:`,
saying so on every line is noise. Drop it and the next segment is the one you
actually chose as a category. When a box holds accounts from genuinely different
roots, nothing is shared, so the roots themselves become the lines.

This is the same rule the balance sheet uses; on a chart where everything sits
under one root it reduces to exactly the balance sheet's "second path segment"
behaviour, so the two statements group alike.

Like there, grouping never matches account *names* — it is position in the tree
and nothing else — and the same small cosmetic alias table may prettify a label
without ever moving an account to a different line.

## The subtotal ladder

Boxes appear in this order, and a box with nothing in it is omitted entirely.
Each subtotal appears only when the sections that give it meaning exist:

```
Revenue
Cost of revenue                          if any cogs
    Gross profit                         if any cogs
Operating expenses
    EBITDA                               if any depreciation
Depreciation & amortization              if any depreciation
    Operating income                     if the statement is multi-step
Other income & expense                   if any other
Interest                                 if any interest
    Income before taxes                  if any tax
Income taxes                             if any tax
                                Net income
```

EBITDA sits **above** D&A and Operating income **below** it, which is the order a
real statement uses and which makes every subtotal a running total of everything
printed above it. EBITDA is suppressed when there is no D&A box, because it would
then be the same number as Operating income — and printing one number twice is
the complaint this redesign exists to fix.

The `opex` box is titled **"Expenses"** on a simple statement and **"Operating
expenses"** once the statement goes multi-step. It is the same box either way;
only the label moves, so no account changes box when your journal grows its first
`cogs:` tag.

### Signs

Every box prints as a positive magnitude and the ladder does the subtracting —
except **Other income & expense**, which is genuinely mixed. A grant and a lawsuit
settlement can share it, so it is presented as a net contribution to income and is
allowed to print negative, in the parenthesised style a real statement uses.

## The comparison columns

**Prior period** is the immediately preceding window of the same length. For a
full calendar year that is simply the year before; for any other range it is an
honest apples-to-apples duration, with its dates in the column header. Each period
is valued at its own period end, matching `hledger is -V` run over that range —
so the prior column agrees with the report you actually ran last year, at the cost
of a currency move showing up as part of the change.

**% of revenue** is each line as a percentage of total revenue — the common-size
column. The denominator is total revenue, not net income. With no revenue in the
period there is no percentage, and the cell reads `—`.

## Valuation

Lines are **valued at market in your base currency** by default, so foreign
spending and dividends read as money rather than as a column of currencies. Prices
come from the journal's `P` directives only, matching `hledger is -V`.

A commodity with no usable `P` directive is **not** silently dropped: it stays on
its line as a secondary figure and is named in a warning banner above the report.

`?value=cost` and `?value=none` are available on the API for the cost basis and
for raw, unvalued commodities.

### Why net income here isn't Retained earnings there

The balance sheet's **Retained earnings** line is computed at *cost*; this
statement's net income is valued at *market*. On `fixtures/sample.journal` that is
`$42,998.91` against `$41,916.34`. Both are right. The difference is exactly what
the balance sheet's **Valuation adjustment** line absorbs, which is why that line
exists and why `assets = liabilities + equity` still ties out. Ask for
`?value=cost` and the two figures agree.

## The money-flow diagrams

Two Sankey diagrams sit in the statement: **Money in** immediately above the Revenue box, **Money out** immediately above the first cost box that prints. Each is a *decomposition* of the side of the statement below it, not a second statement.

**Money in** puts the revenue lines on the left and the accounts the money landed in on the right, so you can see that a salary arrived partly as cash in the bank and partly as tax withheld before you ever saw it. **Money out** swaps the columns: the accounts that funded the spending are on the left and the cost lines on the right, so you can see which card or account each category was actually paid from.

### How a posting is attributed

A statement line is one side of a transaction; the other side is where the money came from or went to. With two postings that is unambiguous. With more, each statement posting is allocated across the postings on the **opposite side of the ledger in its own transaction**, in proportion to their size. A paycheck:

```journal
2026-01-27 * Acme Corp | January salary
    income:salary             $-5,660.00
    expenses:taxes:federal     $1,150.00
    expenses:taxes:state         $310.00
    assets:bank:checking       $4,200.00
```

draws Salary to Taxes: Federal at `$1,150.00`, to Taxes: State at `$310.00` and to Bank: Checking at `$4,200.00` in **Money in**, and Salary to Taxes at `$1,460.00` in **Money out**. The withheld tax was funded by gross pay and not by the cash account, and both diagrams say so.

Allocating the *statement* side rather than the account side is what makes each statement posting split exactly (integer mantissa arithmetic, last share takes the remainder) and what makes market valuation harmless: a transaction balances at cost, not at market, so a valued transaction's debits and credits may differ, and allocating a known statement amount across proportions is indifferent to that.

**Other income & expense is in neither diagram.** It is the one box the statement lets print negative, because a grant and a lawsuit settlement can share it, so it has no single direction to flow in.

### When the picture is not the whole story

Links whose net over the window is zero or negative are not drawn. A category refunded more than it was charged has no width, and a Sankey cannot render a negative one; the same goes for a statement posting with no counterparty to attribute it to.

Whatever that removes shows up as a line under the diagram reading **`Showing $X of $Y`**: `$X` is what the ribbons carry, `$Y` is the statement figure they decompose. On an ordinary journal the two agree and the line is absent entirely. It is never hidden, because the gap is the only place that missing money can be seen.

### Reading the picture

Colour identifies the **account**, and an account keeps its colour in *both* diagrams: `assets:bank:checking` is the same blue wherever it appears. The palette has eight slots; accounts past the eighth fold into one grey `(other)` bar, with their links merged. Statement lines never fold and never take a colour, because folding them would hide exactly the spending categories the diagram exists to show; the panel grows taller instead. Every bar carries its own label and figure, and a legend under each diagram names every account, so identity is never colour alone.

Both panels are collapsible and each remembers its own state across reloads. A collapsed panel is not merely hidden: the diagrams are a separate endpoint and a second pass over every posting in the window, so with both panels shut nothing is fetched at all. Expanding one fetches immediately.

## Export

The XLSX export mirrors the screen: filled section headers, bold group rows,
indented accounts, ruled subtotals, then net income — with the prior-period and
percentage columns alongside. Because every line is valued into one commodity, the
amount cells hold real numbers with a number format rather than text, so the
workbook is arithmetic you can build on. Groups are written expanded regardless of
what is collapsed on screen.

The two diagrams are deliberately **not** in the workbook. A picture of the flows is not arithmetic anyone can build on, and the numbers behind it are already in the boxes it decomposes.

## API

```
GET /api/reports/incomestatement/grouped
      ?from=YYYY-MM-DD        (default: Jan 1 of the current year)
      &to=YYYY-MM-DD          (default: today; both ends inclusive)
      &value=market|cost|none (default: market)
      &valueIn=$              (default: the journal's base commodity)
      &compare=previous|none  (default: previous)
```

The older `GET /api/reports/incomestatement` returns the flat, unvalued, `hledger is`-shaped report and is unchanged, backing the hledger parity golden.

```
GET /api/reports/incomestatement/flows
      ?from=YYYY-MM-DD        (default: Jan 1 of the current year)
      &to=YYYY-MM-DD          (default: today; both ends inclusive)
      &valueIn=$              (default: the journal's base commodity, else the
                               single commodity the window is written in)
```

No `value=` and no `compare=`: a link's width is one number, so the widths are always market-valued, and neither diagram has a comparison column. The response carries both graphs, each with its `nodes`, its `links`, the `total` they carry and the `sectionTotal` they decompose. `base` is `null` when the journal has several commodities and nothing prices them against each other, and both graphs are then empty.
