---
title: The scenario file format
description: "A projection is an ordinary, balanced hledger journal of ~ rules plus a few tags: projection, cash, growth, line, opening and gains."
order: 10
tags: [projections]
---

# The scenario file format

This is the whole of it:

```journal
; Ledgeline projection
; projection: Series A with a hiring ramp
; created: 2026-09-18
; updated: 2026-09-20
; cash: assets:checking

account equity:unrealized gains  ; type: E

; Every rule ends with its COUNTER-ACCOUNT, written with no amount: where the
; money lands or comes from. hledger fills the amount in, so each rule is an
; ordinary balanced transaction.
~ monthly  projection
    revenues:salary      $-12000  ; growth: 3%/yr
    revenues:consulting   $-2500
    expenses:rent          $4200  ; growth: 2%/yr
    expenses:software       $900
    assets:checking

; A STEP: payroll jumps in April and grows from the new base. Two bounded rules
; for one account, linked by the `line:` tag so the list shows them as one step change.
~ monthly to 2027-04-01  projection
    expenses:payroll      $50000  ; line: payroll
    assets:checking

~ monthly from 2027-04-01  projection
    expenses:payroll     $120000  ; line: payroll, growth: 5%/yr
    assets:checking

; A ONE-OFF on the balance sheet: cash and equity, no net-income effect.
~ 2027-03-01  Series A
    assets:checking     $2000000
    equity:preferred

; A ONE-OFF on the P&L, accrued: no cash moves until it is paid.
~ 2027-06-15  legal fees
    expenses:legal        $45000
    liabilities:payable

; ASSET ROWS: a balance, a rate, and what you put into it. The amount is the
; CONTRIBUTION — `$0` when the balance just compounds — and `growth:` is what
; marks the row as a stock rather than a flow. Appreciation is booked to the
; row's gains account.
~ monthly  investing
    assets:house              $0  ; growth: 4%/yr, gains: equity:unrealized gains
    assets:savings         $2000  ; growth: 4%/yr
    assets:checking

; A TRANSFER balances on its own, so it needs no counter-account.
~ monthly  to savings
    assets:savings          $300
    assets:checking         $-300
```

Every line of it is hledger's own syntax. Nothing here is a Ledgeline format.

Files written before counter-accounts existed use unbalanced virtual postings,
`(expenses:rent) $4200`, with no counter-account. They still load and project
the same numbers, and Ledgeline writes the counter-account it used the next
time you save. [What moves cash](cash.md) explains how it picks one.

## The tag vocabulary

One header tag names the scenario, one names its default cash account, and the
rest go on postings.

| tag | where | means |
|---|---|---|
| `projection: <name>` | a comment at the top of the file | the scenario's display name |
| `cash: <account>` | a comment at the top of the file | the counter-account for a line your history says nothing about |
| `growth: <rate>/<unit>` | a posting comment | the amount grows by `rate` per completed `unit`; on an asset-typed account it ALSO marks the row as a balance |
| `line: <id>` | a posting comment | two bounded rules are two steps of ONE change; written only for a step change |
| `opening: <number>` | an asset row's posting comment | override the journal's balance for that account |
| `gains: <account>` | an asset row's posting comment | where appreciation is booked; `equity:unrealized gains` when absent |

**The gains account is a real, declared account.** If your main journal does
not declare it, Ledgeline declares it in the scenario file
(`account equity:unrealized gains  ; type: E`), so hledger and Ledgeline
classify it the same way and your main journal is not touched.

**`; projection: <name>` is the display name and the only marker.** A file
without it still loads — any journal with budget-like entries can be used as a
projection — and takes its name from its filename. `; created:` and
`; updated:` are the dates Ledgeline stamps when it writes the file; you can
delete them and nothing breaks.

**`growth:` reads `3%/yr`, `2.5%/mo`, `-1%/wk` and `0.03/year`.** The percent
sign is optional and the unit spelling is generous, because this is a tag a
human types into a journal by hand. Anything it cannot read makes the line
**flat** — never a failed file.

**`opening:` is the only tag that is not optional decoration.** It belongs to an
asset row and nothing else, it carries a bare number in the row's own commodity
(hledger ends a tag's value at a comma, so `$200,000` would read back as `$200`),
and [Asset rows](asset-rows.md#the-opening-tag-restates-the-balance-and-says-so) says exactly
what the engine does with it.

**`line:` is what links the two steps of a step change.** A step is a permanent
change to a recurring line: payroll jumps from `$50k/mo` to `$120k/mo` in April
and stays there, growing from the new base. That is two bounded `~` rules for
the same account. The shared `line:` tag tells the list they belong together,
so each card shows `step 1 of 2` or `step 2 of 2` and links to the other.
Without the tag they are still two good rules. They just aren't linked.

**Ledgeline writes `line:` only for a step change.** A rule with one step
needs no tag, so a save writes none, and drops the `line: gap:…` tags older
versions put on every seeded line. A step gets a short id made from its account's
leaf (`expenses:payroll` becomes `payroll`, and a second one `payroll-2`), or
keeps the id you wrote yourself. An id Ledgeline writes never contains a colon.
Untagged lines get positional ids when the file loads, and a written id can never
look like one of those, so reordering the rules can't join two unrelated lines.

## Amounts are signed as hledger writes them

A rule's amounts are the file's amounts, with their signs. Revenue is
negative, as in any hledger journal: `revenues:salary  $-12000`. An expense is
positive. A minus moves money out of an account, so a transfer from checking to
savings is `$300` on savings and `$-300` on checking. The list shows each
amount with this sign, and the editor takes it the same way. Type `-12000` for
salary and `4200` for rent. Ledgeline never flips a sign for you, even when you
change a posting's account.

The `=` rule form is the one exception. Its percentages follow [their own
rule](#editing-rules-on-the-projections-page).

## A rule that balances needs no counter-account

If a rule's postings already sum to zero, there is nothing left over for a
counter-account to take, so the rule has none:

```journal
~ monthly  to savings
    assets:savings      $300
    assets:checking    $-300

; Deferred revenue: a retainer billed in January is earned month by month.
~ monthly from 2027-01-01  retainer earned
    liabilities:deferred-revenue  $2000
    revenues:consulting          $-2000
```

The list draws no `→` line for these, and the editor's Counter-account box says
"None needed". Ledgeline writes them as plain postings, and adds no
counter-account unless you type one. If an edit makes the postings stop
balancing, the rule needs a counter-account again, and the default comes back.

## Descriptions and order

**A rule's description is the text after its period**, separated by two spaces:
`~ monthly  unbudgeted — average over 2025-11-01 to 2026-10-22`. The list shows
it on the card's first line, after the period, and the editor has a
**Description** box for it. A description is one line and can't contain `;` or
`#`, because hledger would read the rest of the line as a comment.

![A seeded rule's card: the period, the description "unbudgeted — average over 2025-08-01 to 2026-07-08" and an estimated badge, then the expenses:housing posting and its default counter-account](images/description.png)

A description belongs to the whole rule, so a rule with several postings has
one description. A seeded rule's description says what the estimate was
averaged over. If you change that rule's amount, a small dot appears after the
description, and hovering over it says the description may be out of date.
Ledgeline never rewrites a description itself.

**The list's order is the file's order.** Dragging a card by its grip, Move up
and Move down in its menu, and <kbd>J</kbd> / <kbd>K</kbd> move the whole entry
in the file, with the comment above it. A rule's postings are reordered inside
the editor, with ↑ and ↓. The two steps of a step change move separately,
because hledger doesn't care which comes first. File text, such as an `account`
directive, never moves. Other entries can move past it.

Moving an entry doesn't rewrite it. A save writes an entry you didn't edit with
its own bytes, wherever it now sits.

## Comments and section headings

Comments in a scenario file are entries in the list, in the place the file has
them. Ledgeline reads a run of comment lines (lines starting with `;`, `#` or
`*`, with no blank line inside) as one of three things:

- **A section heading**, when the run has a divider line: a line of three or
  more of `-`, `=`, `_`, `~`, `*`, `#` or `;`. The list shows its title in
  capitals with a rule after it. A heading never attaches to the rule below it.
- **A comment on the entry below**, when the run sits directly above a rule,
  a one-off or an automatic posting, with no blank line between. The card shows
  it in italics, and it moves, duplicates, turns off and deletes with its
  entry. The editor's **Comment above this entry** box edits it.
- **A note** otherwise: a comment with a blank line after it. It stays where
  you put it.

```journal
;;;;;;;;;;;;;;;;;;;;;;
;; Spending

; The lease renews in January at the new rate.
~ monthly to 2027-01-01  rent
    expenses:rent      $1875  ; line: rent
    assets:checking

; Assumes no new car before 2028.

~ 2027-04-15  tax bill
    expenses:taxes     $6000
    assets:checking
```

Here `;; Spending` is a section heading, "The lease renews…" belongs to the
rent rule, and "Assumes no new car…" is a note.

**+ Add → Section heading** writes a new heading as a line of 22 semicolons and
then `;; Title`, with no closing line. A heading already in the file keeps its
own framing when you edit its title. A note is written `; ` before each line.
Ledgeline puts a blank line after a heading or a note, so the next read doesn't
attach it to the entry below.

A `comment` … `end comment` block is a note too. Ledgeline shows it as written,
and never reads anything inside it as a rule.

**Comments inside a rule survive edits.** A comment on a posting
(`expenses:rent  $4200  ; owner: alex`), a comment after the period on the `~`
line, and indented comment lines between postings stay when you edit the rule.
The editor has a comment box for each posting and a **Header comment** box for
the `~` line. Ledgeline writes its own tags (`growth:`, `opening:`, `gains:`,
`line:`) first, then your text after a comma, so hledger still reads your own
tags as tags. One comment is lost. A comment on the counter-account line is
dropped when the rule is rewritten.

## Turning an entry off

**Turn off** (or <kbd>t</kbd>, or the editor's **Enabled** switch) comments the
entry out. Ledgeline puts `; ` in front of every line of the entry and leaves
the comment above it as it is:

```journal
; With a big new customer: the "with" alternative.
; ~ monthly from 2027-01-01  consulting, with Acme
;     revenues:consulting   $-6000
;     assets:checking
```

hledger skips it, and so does Ledgeline. A turned-off entry changes no number on
the page. The list still shows it, dimmed, struck through and badged `off`.
**Turn on** removes the `; ` again and gives back the exact lines you had.

Ledgeline reads a commented-out block as a turned-off entry only when all of
these hold:

- its first line, after the comment character and at most one space, starts
  with `~` or `=`;
- every line of it starts with the same comment prefix, so a block that mixes
  `;` and `#` is not one;
- it has at least one posting line;
- the text, with the prefix removed, is exactly one hledger rule.

Anything else stays a comment. `; ~ monthly is how often we pay` has no posting,
so it is a note. A rule inside `comment` … `end comment` is part of that note,
too.

## Duplicate as alternative

To compare a plan with and without something, keep both versions in the file
and turn one off. Say you want to see the runway with a big new customer and
without one:

1. Open the menu on the consulting rule and choose **Duplicate as
   alternative** (or press <kbd>C</kbd>).
2. The editor opens on a copy, placed directly below the original. Change the
   amount to the bigger figure (`-6000`: income is negative), and put "with
   Acme" in the description.
3. Press **Done**. The copy is added and the original is turned off, in one
   change, so the readings now show the "with" version.

To switch back, turn the copy off and the original on. Each is one press of
<kbd>t</kbd>. **Cancel** in step 2 leaves no copy, and the original stays on.

**Duplicate** (<kbd>c</kbd>) does the same without turning anything off. Use it
to start a new rule from an existing one, for example to give one posting a
different counter-account. Copies never share a `line:` id with the original,
so a copy is never mistaken for a step.

## Growth is stepwise, not smoothly compounded

A line marked `+3%/yr` holds flat for twelve months and then bumps 3%. That is
how a rent increase or a salary review actually lands. The stepped figure is a
real number you could have typed, so it is rounded back to the amount's own
display precision **at every step** — which is why a `$1,875.00` line is written
with its cents even when the figure is round.

Growth anniversaries are counted from the line's `from` date when it has one,
and from the start of the projection otherwise. That is what makes the second
segment of a step change grow from its **new** base.

## When a rule fires

On the days `hledger --forecast` fires it, from the first projected day:

- A rule with a `from` keeps its own rhythm: `~ monthly from 2027-01-15` fires
  on the 15th of every month.
- A rule with **no** `from` starts on the first projected day. `~ weekly` fires
  on that day's weekday, not on Mondays, and `~ yearly` on that day's
  anniversary, not on January 1st. A monthly, quarterly or yearly projection
  starts on the 1st of a month, so `~ monthly` lands on the 1st. A weekly
  projection starts on a Monday, so there `~ monthly` lands on that Monday's
  day of the month.
- `to` is exclusive: `~ monthly to 2027-04-01` last fires in March.
- `every 15th day of month` and `every tuesday` fire on that day.
- `every weekday`, `every mon,wed,fri` and `every 1st Apr` fire on those
  days. `monthly in 2027q1` and `monthly 2027-01..2027-04` are bounded. The
  full list of forms is in [Budget](../budget.md).
- A dated rule, `~ 2027-03-01` or `~ 2027q2`, fires once.

`hledger balance --budget` places a bare `~ weekly` goal on Mondays instead.
The Budget tab follows that report, and the Projections tab follows
`--forecast`.

## Amounts follow your main journal's commodity styles

A projection file reads its amounts under the main journal's `commodity`
directives, as though it were `include`d. With `commodity $1,000.00` in the
main journal, a hand-typed `$11,344` in a projection file is eleven thousand
dollars. A `commodity` or `decimal-mark` directive in the projection file
itself takes precedence.

hledger does not lend a `commodity` directive across sibling files, so
`hledger -f main.journal -f projection.journal` would read that same
`$11,344` as `$11.344`. When you save, Ledgeline therefore adds the main
journal's directive to the projection file (`commodity $1,000.00`) for each
commodity the file uses but doesn't declare. After that, hledger reads the
file the same way however it's opened.

## Automatic postings (`=` rules)

A projection file can hold hledger's auto-posting rules, so a cost can follow a
revenue or a wage line as a percentage:

```journal
= revenue:sales tag:_generated-transaction
    expenses:people-costs:sales-commission  *-0.03
    assets:mercury:checking

= expenses:people-costs:wages-salaries tag:_generated-transaction
    expenses:people-costs:payroll-tax  *0.1
    assets:mercury:checking
```

Wherever a rule's query matches a posting of a projected transaction, its
postings are added right below that posting. `*0.1` is 10% of the matched
amount, and the posting with no amount balances the others. Ledgeline applies
a projection file's rules to every projected transaction, always, whatever your
`hledger.conf` says about `--auto`. It never applies them to recorded
transactions.

- **`tag:_generated-transaction`** in the query is what tells *hledger* to do
  the same. Without it, `hledger --auto` would also apply the rule to your
  recorded transactions. Ledgeline still keeps the rule to the projection, and
  its warnings say so.
- **A rule on an account also fires for that account's children.**
  `expenses:people-costs:wages-salaries` matches `…:wages-salaries:george`, and
  each child posting gets its own 10%. hledger's queries match anywhere in the
  name, so it also matches a sibling such as `…:wages-salaries2`. To keep to
  the subtree, write `^expenses:people-costs:wages-salaries(:|$)`.
- **If one projected transaction has several matching postings** (two people
  in the same `~` rule), the amountless posting is added once per match.
  Ledgeline balances each one, but `hledger --auto` refuses the file. Write
  that posting with the opposite multiplier, `assets:mercury:checking  *-0.1`,
  and hledger reads it the same way. Putting each person in a `~` rule of
  their own works too. The Projections page warns when this applies.
- A bare number, such as `2`, is written with no commodity, as hledger 1.52
  writes it.
- Queries Ledgeline does not evaluate yet (`expr:`, `type:`, `date2:`) keep the
  rule in the file but it never fires, and the page says why.

### Editing rules on the Projections page

The editor for an automatic posting is a form: *when a posting to
this account (or one of its subaccounts) is projected, add these postings*.
Each posting is a percentage of the matched amount or a fixed amount, and a
counter-account (the scenario's cash account by default) takes the balance.
A line under the rule shows what it comes to on an amount the scenario already
posts there, such as "≈ $1,200.00 to expenses:people-costs:sales-commission
on each $40,000.00 to revenue:sales".

**In this form, type positive figures.** This is unlike a `~` rule, whose
amounts you type with the file's signs. A percentage here means "this account
grows by this share of the matched amount", and Ledgeline works out the sign from the two
accounts' types. Asset and expense accounts grow with a positive amount;
liability, equity and revenue accounts grow with a negative one. So 3%
commission on a sale (revenue into an expense) is written `*-0.03`, 10%
payroll tax on wages (an expense into an expense) is `*0.1`, and 10% of wages
owed as a tax liability (an expense into a liability) is `*-0.1`. A fixed amount
takes its own account's sign: `$50` into an expense, `$-50` into a liability.

An account's type comes from an `account` directive, in the main journal or in
the scenario file itself (`account hmrc:paye  ; type: L`; where both declare
one, the scenario file's wins). An undeclared account is typed from its name
(`liabilities:…`, `expenses:…`). If the name gives no type either, as with
`hmrc:paye`, the account is treated as growing with a positive amount. In that
case the percentage is written exactly as you type it, so declare the account's
type if it is really a liability.

A rule edited here is saved in this form:

```journal
= ^revenue:sales(:|$) tag:_generated-transaction  ; sales commission
    expenses:people-costs:sales-commission  *-0.03
    assets:mercury:checking                 *0.03
```

- The query matches the account and its subtree only, never a sibling such
  as `revenue:sales2`, and it is limited to forecasts in hledger as well. Any
  extra query terms you add (**+ query terms**, for example `desc:acme` or
  `amt:>1000`) follow it.
- The counter-account gets the **opposite multiplier** rather than no amount,
  so hledger loads the file even when a rule fires twice in one transaction.
  For that reason a counter-account balances percentages, or fixed amounts in
  one commodity, but not a mix of the two.
- Only the rule you edited is rewritten. The others keep their text, comments
  and layout exactly. A deleted rule's block is removed, and a new rule goes
  where you added it in the list. Order matters, because hledger applies rules in
  file order, and a later rule sees the postings an earlier one added.
- A hand-written rule in the style above (a plain account name, an amountless
  last posting, with or without the tag) opens as a form too. Once you edit it
  it is saved in the form shown. An untagged rule shows **projections only
  here** and a button that adds the tag.
- A rule the form cannot show is listed as its text with a lock: virtual
  `(account)` postings, `*N CUR`, `%account`, posting comments, or an
  unsupported query. It is still applied and saved byte for byte. Edit it in
  the file.
- A rule that posts to an account no `account` directive declares and no
  transaction uses, such as `expesnses:people-costs:payroll-tax`, gets a
  warning naming the account. hledger accepts it, so Ledgeline does too, but it
  is almost always a typo.

Generated postings are marked **auto** in the Journal tab. Save As copies every
rule to the new file.

To check a projection with hledger, add `--auto` to the commands below.

## What hledger does, and does not, do with this

`hledger` reads these files. Both of these work:

```console
$ hledger -f projection-series-a.journal balance --budget -M
$ hledger -f projection-series-a.journal print --forecast=2026-10-01..2029-10-01
```

**What hledger will not do is apply `growth:`**, and it never sees the future
prices Ledgeline uses to grow a stock holding (see [Asset rows](asset-rows.md)).
Both are shown flat by hledger. hledger has no arithmetic in
amounts — `$1200 * 1.03` is a parse error — so a journal file cannot state a
growing amount. `growth:` is a comment to hledger, and a `balance --budget`
report over a projection file shows the **base** amount in every period, in year
five exactly as in year one. Only Ledgeline expands it.

That was a deliberate trade. The alternative is writing the file out expanded,
one bounded rule per growth step, which plain hledger would read correctly and
which no human could then edit. The compact form won, and this paragraph is the
price of it. (Nothing is lost permanently: the tag is already there, so an
expanded export is possible later without a format change.)

> **Never round-trip a scenario through `hledger print`.** It drops `~` rules
> entirely, so the file would come back with every scenario line gone and
> nothing to say it had happened. Ledgeline writes these files itself.

## Reading a projection file by hand

Everything the tab shows, you can get from `hledger` — minus growth.

```console
# The goals, as written, month by month.
$ hledger -f projection-series-a.journal balance --budget -M -b 2027-01-01 -e 2027-04-01

# The same rules read as future transactions.
$ hledger -f projection-series-a.journal register --forecast=2027-01-01..2028-01-01

# Both files at once — what you plan, beside what you did.
$ hledger -f main.journal -f projection-series-a.journal balance --budget -M

# Every asset row in a file, by the tag that marks one.
$ hledger -f projection-series-a.journal print --forecast=2027-01-01..2027-02-01 tag:growth

# The balance sheet the tab projects, from a cutoff of 2026-10-01: actuals before
# it, forecast from it, and no real transaction after it counted twice.
$ hledger -f main.journal -f projection-series-a.journal bs \
    --forecast=2026-10-01..2027-10-01 -M -b 2026-08-01 -e 2027-10-01 \
    expr:"NOT (date:2026-10-01.. AND NOT tag:generated)"

# The same window as an income statement, other commodities valued as the tab
# values them.
$ hledger -f main.journal -f projection-series-a.journal is \
    --forecast=2026-10-01..2027-10-01 -M -b 2026-08-01 -e 2027-10-01 \
    expr:"NOT (date:2026-10-01.. AND NOT tag:generated)" --value=end,$
```

These agree with the tab account by account and period by period, except where
`growth:` applies. [What moves cash](cash.md#where-the-projection-starts) explains
each part of the command.
