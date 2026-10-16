---
title: What moves cash
description: "The residual rule: how a projection line with no funding leg moves cash and net worth, including accruals and partial payments."
order: 20
tags: [projections]
---

# What moves cash: the residual rule

A projection line has no funding leg — it is an unbalanced virtual posting,
`(account) amount`, exactly as a budget goal is — so the cash effect has to be
implied. One rule, stated once:

> For each **group** — one `~` rule, or one dated event — sum **every** posting.
> The negation of that sum, the group's **residual**, is the implied cash leg,
> and it applies **in addition to** any cash postings the group already states.

Cash is an asset, so net worth takes that same implied leg, beside the group's
own asset and liability postings. Equity postings move neither series on their
own.

| the group | residual → implied leg | net cash | net worth |
|---|---|---|---|
| `(expenses:rent) $4200` | −4200 | −4200 | −4200 |
| …beside `(assets:checking) $-4200` | 0 | −4200, the stated leg | −4200 |
| `(expenses:legal) $45000` beside `(liabilities:payable) $-45000` | 0 | **0** | −45,000 |
| `(assets:cash) $2M` beside `(equity:preferred) $-2M` | 0 | +2,000,000 | +2,000,000 |
| `(expenses:rent) $4200` beside `(assets:cash) $-2000` | −2200 | −4200 | −4200 |

Row two is a double-count that a naive rule would make, and the residual avoids
it for a good reason rather than a special case: the group nets to zero.

**Row three is the one worth understanding.** It is an accrual — a bill incurred
in June and paid in August. The residual books **no cash in June**, because the
group already balances itself against a liability, so the runway is not charged
for money that has not left. Write the settlement as a separate dated event when
it happens:

```journal
~ 2027-06-15  legal fees, net 60
    (expenses:legal)          $45000
    (liabilities:payable)    $-45000

~ 2027-08-15  legal fees paid
    (liabilities:payable)     $45000
```

June moves net worth by −45,000 and cash by nothing; August moves cash by
−45,000 and net worth by nothing. That is the right answer, and it is one you
can express.

**Row five is a partial payment**, which is the case no boolean "did this fund
itself?" guard could express at all.

**Net income is untouched by any of this.** It reads revenue and expense
postings only.
