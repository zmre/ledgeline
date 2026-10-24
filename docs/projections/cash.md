---
title: What moves cash
description: "Every projection line names the account its money lands in or comes from. How Ledgeline picks one when a line does not, and how accruals and partial payments work."
order: 20
tags: [projections]
---

# What moves cash: the counter-account

Every projection line is balanced against a **counter-account**: the account
its money lands in or comes from. Salary lands in checking. Rent comes out of
checking. A legal bill you will pay in 60 days goes to `liabilities:payable`.
In the file, the counter-account is the rule's last posting, written with no
amount, so hledger fills it in:

```journal
~ monthly  projection
    revenues:salary   $-12000  ; growth: 3%/yr
    expenses:rent       $4200
    assets:checking
```

Each month that rule is one balanced transaction: salary in, rent out, and
`assets:checking` up by the difference. Ledgeline expands a scenario into dated
transactions like this, the way `hledger --forecast` does, and every number on
the Projections tab is read off them. Cash is the balance of your cash
accounts; net worth is your assets less your liabilities.

## When a line names no counter-account

Older scenario files, `budget.journal` rules and freshly seeded estimates have
no counter-account. Ledgeline picks a default for them, in this order:

1. **the account your history paired it with.** Ledgeline looks at your real
   transactions over the last twelve months that post the line's account in
   the line's commodity, and asks where the other side of the money went: the
   asset or liability account with the most money paired with it. Salary that
   always landed in `assets:checking` projects into `assets:checking`.
   Depreciation booked against `assets:car:depreciation` keeps booking there,
   so it never touches cash.
   - **A credit card is followed to the account that pays it.** Groceries paid
     on `liabilities:visa` would never reduce cash in a projection, because
     the projection does not project the card payments. So when the pairing is
     a liability, and that liability is mostly paid down from a cash account,
     the cash account is the default. One hop only.
   - **Commodities are kept apart.** A `$` line is only paired with an account
     that took `$`; a EUR line with one that took EUR. Dollar groceries never
     land in your euro account.
   - **Equity, revenue and expense accounts are not pairings.** An opening
     balance says nothing about where next month's money goes, and tax
     withheld from a payslip pairs the tax with the salary rather than with
     where the money sits. A line with no other pairing falls through to the
     steps below.
2. otherwise the scenario's own default, a `; cash: <account>` line in the
   file's header;
3. otherwise the cash account with the largest balance;
4. otherwise the first account you have declared `type: C`, and failing that
   `assets:cash`.

Steps 2 to 4 prefer a cash account that holds the line's commodity, so a EUR
line with no history goes to your euro account even if your dollar account is
larger. A `; cash:` header naming an account that only ever held another
commodity is passed over for that line. Only when no cash account holds the
commodity does the order run without that preference.

A cash account is one whose type is Cash: declared `type: C`, or inferred the
way hledger infers it. Account names are never guessed at.

The default only applies to the money a rule leaves **unbalanced**. A rule
that already states its own cash leg balances, and nothing is added:

| the rule | what is posted | net cash |
|---|---|---|
| `(expenses:rent) $4200` | rent, and −4200 to the default cash account | −4200 |
| …beside `(assets:checking) $-4200` | as written; it balances | −4200 |
| `(expenses:rent) $4200` beside `(assets:cash) $-2000` | as written, and the remaining −2200 to the default | −4200 |

When you **save**, Ledgeline writes the default it used into the file, so the
file becomes plain balanced hledger and the numbers do not move. It never
writes to `budget.journal`.

## In the table

Every Income, Expenses and One-off row has a **Counter-account** column. It
starts empty, with the default Ledgeline would use greyed in it. Type an
account to override it; clear the box to go back to the default. An event has
one counter-account, on its first posting: it takes whatever the event's
postings leave unbalanced.

The scenario's own default, the `; cash:` header, is the **Default cash
account** box above the table. It starts greyed with the account Ledgeline
falls back to (steps 3 and 4 above).

A rule has exactly one counter-account. When you give one line of a
multi-line rule a different counter-account from the rest, Ledgeline moves that
line into a rule of its own, so the file can still be written as hledger reads
it. No number changes; the file just has one more `~` rule.

## Accruals and partial payments

To keep a bill off your cash until you pay it, give it a liability as its
counter-account:

```journal
~ 2027-06-15  legal fees, net 60
    expenses:legal          $45000
    liabilities:payable

~ 2027-08-15  legal fees paid
    liabilities:payable     $45000
    assets:checking
```

June moves net worth by −45,000 and cash by nothing. August moves cash by
−45,000 and net worth by nothing.

A **partial payment** states the cash part and lets the counter-account take
the rest:

```journal
~ 2027-06-15  rent, half on account
    expenses:rent            $4200
    assets:checking         $-2000
    liabilities:payable
```

An **equity raise** moves cash and net worth and leaves net income alone,
because neither account is revenue or expense:

```journal
~ 2027-03-01  Series A
    assets:checking       $2000000
    equity:preferred
```

## What counts

- **Runway counts cash only**: the balance of your cash-typed accounts. Stock
  you could sell is not runway.
- **Runway reads total cash, never one account.** Every cash account is
  summed, commodity by commodity, and each period's total is valued into your
  main commodity at that period's end. The runway is the first period where
  that total closes below zero. One account going overdrawn while the rest
  cover it is not running out of cash, and neither is spending more euros
  than your euro account holds while your dollars cover it. A commodity no
  price converts cannot be netted against anything, so it is judged on its
  own: if it closes below zero, that is a runway.
- **Net income** is the income statement's: revenue and expense postings
  only.
- **Appreciation** on an asset row is booked to a gains account
  (`equity:unrealized gains` by default), so it moves net worth and not net
  income or cash. See [Asset rows](asset-rows.md).

## Where the projection starts

The projection starts at the first period that is not yet complete: the start
of the period containing the earlier of today and the day after your latest
real transaction. Everything before it is actuals; it and everything after are
projected, and real transactions dated inside it are left out. If today is
23 October and you have entered October's transactions, October is still
projected, because it is not over. If your latest transaction is 15 October
and today is in November, October is projected too. If your latest transaction
is 31 October, October is complete.

The same view in hledger, for a monthly projection starting 2026-10-01 with
the default two months of actuals before it:

```console
$ hledger -f main.journal -f projection-x.journal bs --forecast=2026-10-01..2027-10-01 \
    -M -b 2026-08-01 -e 2027-10-01 expr:"NOT (date:2026-10-01.. AND NOT tag:generated)"
```

`--forecast=` starts on the cutoff and `-b` on the first actual period. Both
end dates are exclusive, so they are the day after the last projected day. The
`expr:` leaves out real transactions dated on or after the cutoff and keeps the
forecast. Use `-Q`, `-Y` or `-W` instead of `-M` for other intervals, and `is`
or `cf` instead of `bs` for the other statements. If you hold other
commodities, add `--value=end,$` to value each column at its own end, as the
tab does.

Per account and per period, hledger then shows the same figures as the tab.
Two things differ, both because of growth: hledger shows `growth:` amounts at
their base, and it values a holding whose price a growth rate raises at its
last real price. hledger's `cf` lists the change in each cash account. The
tab's cash flow statement starts from net income instead, but its cash
balances agree with hledger's account by account.
