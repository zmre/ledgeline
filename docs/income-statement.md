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
account expenses:depreciation ; type: X, issection: depreciation
account expenses:interest     ; type: X, issection: interest
account expenses:taxes:income ; type: X, issection: tax
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

## Export

The XLSX export mirrors the screen: filled section headers, bold group rows,
indented accounts, ruled subtotals, then net income — with the prior-period and
percentage columns alongside. Because every line is valued into one commodity, the
amount cells hold real numbers with a number format rather than text, so the
workbook is arithmetic you can build on. Groups are written expanded regardless of
what is collapsed on screen.

## API

```
GET /api/reports/incomestatement/grouped
      ?from=YYYY-MM-DD        (default: Jan 1 of the current year)
      &to=YYYY-MM-DD          (default: today; both ends inclusive)
      &value=market|cost|none (default: market)
      &valueIn=$              (default: the journal's base commodity)
      &compare=previous|none  (default: previous)
```

The older `GET /api/reports/incomestatement` returns the flat, unvalued,
`hledger is`-shaped report and is unchanged — it backs the hledger parity golden.
