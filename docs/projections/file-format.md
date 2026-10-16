---
title: The scenario file format
description: "A projection is an ordinary hledger journal of ~ rules plus four tags: projection, growth, line and opening."
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

~ monthly  projection
    (revenues:salary)      $-12000  ; growth: 3%/yr
    (revenues:consulting)   $-2500
    (expenses:rent)          $4200  ; growth: 2%/yr
    (expenses:software)       $900

; A STEP: payroll jumps in April and grows from the new base. Two bounded rules
; for one account, linked by the `line:` tag so the editor shows one row.
~ monthly to 2027-04-01  projection
    (expenses:payroll)      $50000  ; line: payroll

~ monthly from 2027-04-01  projection
    (expenses:payroll)     $120000  ; line: payroll, growth: 5%/yr

; A ONE-OFF on the balance sheet: cash and equity, no net-income effect.
~ 2027-03-01  Series A
    (assets:cash)         $2000000
    (equity:preferred)   $-2000000

; A ONE-OFF on the P&L.
~ 2027-06-15  legal fees
    (expenses:legal)         $45000

; ASSET ROWS: a balance, a rate, and what you put into it. The amount is the
; CONTRIBUTION — `$0` when the balance just compounds — and `growth:` is what
; marks the row as a stock rather than a flow.
~ monthly  investing
    (assets:brokerage)           $0  ; growth: 7%/yr
    (assets:savings)          $2000  ; growth: 4%/yr
```

Every line of it is hledger's own syntax. Nothing here is a Ledgeline format.

## The tag vocabulary

Four tags. One lives in a comment at the top of the file, and the other three
go on postings.

| tag | where | means |
|---|---|---|
| `projection: <name>` | a comment at the top of the file | the scenario's display name |
| `growth: <rate>/<unit>` | a posting comment | the amount grows by `rate` per completed `unit`; on an asset-typed account it ALSO marks the row as a balance |
| `line: <id>` | a posting comment | two bounded rules are two segments of ONE row |
| `opening: <number>` | an asset row's posting comment | override the journal's balance for that account |

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
and [Asset rows](asset-rows.md#opening--restates-the-balance--and-says-so) says exactly
what the engine does with it.

**`line:` is what makes a step change one row.** A step is a permanent change to
a recurring line: payroll jumps from `$50k/mo` to `$120k/mo` in April and stays
there, growing from the new base. That is two bounded `~` rules for the same
account, and the shared `line:` tag is what tells the editor to show them as one
row with a "from" on it rather than two unrelated lines. Without the tag they
are still two perfectly good rules; they just appear separately.

## Growth is stepwise, not smoothly compounded

A line marked `+3%/yr` holds flat for twelve months and then bumps 3%. That is
how a rent increase or a salary review actually lands. The stepped figure is a
real number you could have typed, so it is rounded back to the amount's own
display precision **at every step** — which is why a `$1,875.00` line is written
with its cents even when the figure is round.

Growth anniversaries are counted from the line's `from` date when it has one,
and from the start of the projection otherwise. That is what makes the second
segment of a step change grow from its **new** base.

## What hledger does, and does not, do with this

`hledger` reads these files. Both of these work:

```console
$ hledger -f projection-series-a.journal balance --budget -M
$ hledger -f projection-series-a.journal print --forecast=2026-10-01..2029-10-01
```

**What hledger will not do is apply `growth:`.** hledger has no arithmetic in
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
```
