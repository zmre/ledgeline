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

1. **the cash account the line's account was most often paired with** in your
   real transactions over the last twelve months. Salary that always landed in
   `assets:checking` projects into `assets:checking`;
2. otherwise the scenario's own default, a `; cash: <account>` line in the
   file's header;
3. otherwise the cash account with the largest balance;
4. otherwise the first account you have declared `type: C`, and failing that
   `assets:cash`.

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
- **Net income** reads revenue and expense postings only.
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

The same view in hledger, for a monthly projection starting 2026-10-01:

```console
$ hledger -f main.journal -f projection-x.journal bs --forecast=2026-10-01..2027-10-01 \
    -M -e 2027-10-01 expr:"NOT (date:2026-10-01.. AND NOT tag:generated)"
```

hledger differs from Ledgeline only by growth: it shows `growth:` amounts and
grown prices flat.
