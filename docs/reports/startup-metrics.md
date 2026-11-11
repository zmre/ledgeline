---
title: Startup metrics
description: "ARR, net new ARR, customers, NRR, burn multiple, runway and gross margin on the Insights tab: what each one means, the kpi:, segment: and customer: tags that turn them on, and the hledger commands that reproduce them."
order: 15
tags: [reports, account-tags]
---

# Startup metrics

When your journal holds the books of a subscription business, the [Insights](insights.md) tab adds a section headed **Startup metrics** under its six headline boxes: ARR, net new ARR, customers, net revenue retention, burn multiple and gross margin, plus an MRR bridge and a longer list of secondary figures. Runway replaces Net Worth in the headline row.

Every figure comes from postings already in your journal. You turn them on with a few hledger tags, and every one of them can be checked with an `hledger bal` command. A household journal has none of these tags, so it sees no change.

## What turns each part on

The section has three tiers. Each one needs a little more tagging than the one before.

| Tier | What you add | What appears |
| --- | --- | --- |
| Business | Any `kpi:` tag on an `account` directive | Runway in place of Net Worth, plus whichever of gross margin, Rule of 40, S&M, R&D and G&A as a share of revenue, deferred revenue, billings, DSO and DPO have the accounts they need |
| Recurring revenue | `kpi: recurring` on your subscription revenue accounts | ARR, MRR, usage, net new ARR and burn multiple |
| Customers | A `customer:` tag on at least 90% of recurring revenue | Customers, NRR and GRR, the MRR bridge, ARPA, ACV, logo churn, quick ratio, customer concentration, CAC and CAC payback |

When the customer tier is hidden but customers were billed, the Customers box comes back counted from billings instead. See [Customers from billings](#customers-from-billings).

Gross margin also needs an account tagged `issection: cogs`, the tag the [income statement](income-statement.md) already reads. Segment breakdowns need recurring revenue and a dimension tag such as `segment:` with at least two values.

Each box appears only when it has a value. A box whose denominator would be zero or negative, or whose month doesn't exist yet, is left out rather than shown as zero.

Every box, every **More metrics** row, each MRR bridge category and the segment table's ARPA and NRR columns have an (i) icon. Hover over it, tab to it or tap it to see what the figure means, with any acronym spelled out and the formula used.

## Setting up

### The kpi tag

`kpi:` marks the role an account plays in the metrics. It goes on `account` directives, takes one value, and is inherited by subaccounts, the same as `type:` and `cfsection:`. A subaccount's own `kpi:` overrides its parent's.

| Value | Put it on | Meaning | Used for |
| --- | --- | --- | --- |
| `recurring` | revenue | Subscription revenue, as recognised. MRR is its monthly total. | MRR, ARR, growth, the bridge, NRR, customers, burn multiple |
| `usage` | revenue | Metered or overage revenue. | Shown beside MRR, never counted in ARR |
| `deferred-revenue` | liability | Money received or invoiced for service not yet delivered. | Billings, the deferred revenue balance |
| `sales-marketing` | expense | Sales and marketing spend. | CAC, CAC payback, S&M % of revenue |
| `rnd` | expense | Research and development: engineering, product, design, dev tooling. | R&D % of revenue |
| `ga` | expense | General and administrative. | G&A % of revenue |
| `receivable` | asset | Trade receivables. | DSO, and the payee fallback for customers |
| `payable` | liability | Trade payables. | DPO |

Revenue with no `kpi:` is non-recurring. It still counts in total revenue and gross margin, but never in MRR.

Values are matched without regard to case or surrounding spaces. A value outside the table is reported under [Problems](../journal/problems.md#unknown-account-tag-value) and otherwise ignored. A role on an account of the wrong type, such as `kpi: recurring` on an expense, is still read as you wrote it, and [Problems](../journal/problems.md#startup-metric-role-on-the-wrong-type) warns about it.

`kpi:` doesn't change any statement. The balance sheet, income statement and cash flow read the same as before. Cash comes from `type: C`, the operating, investing and financing split from `cfsection:`, and cost of goods sold from `issection: cogs`, so there is no `kpi:` value for those.

Receivables and payables have a fallback. If no account in the subtree declares `kpi:`, an asset account whose last name segment contains `receivable` counts as a receivable, and a liability whose last segment contains `payable` counts as a payable. A declared `kpi:` always wins, and nothing else is guessed from names.

You can set the tag from [Settings → Accounts](../settings/accounts.md), where it is the **Startup metric role** field.

### Dimensions

A dimension splits recurring revenue into segments. Ledgeline looks for four tag names: `segment:`, `product:`, `class:` (the one QuickBooks exports) and `region:`. Put them on accounts, on transactions or on single postings, whichever matches how your books are kept.

For each recurring-revenue posting, the value of a dimension is taken from the first of these that has one:

1. the posting's own tag
2. the transaction's tag
3. the nearest account, the posting's own account first and then each parent in turn

A posting with none of them goes in an `(unassigned)` row. A dimension is offered as a breakdown only when at least two different values carry recurring revenue in the span. When most of the revenue is unassigned, the breakdown is still offered, with the `(unassigned)` row visible so you can see how much is missing.

Ledgeline's account rule is "nearest wins", the same as for `type:`. hledger's `--pivot` agrees with it, and hledger's `tag:` query doesn't (see [the traps](#traps)).

### Customer tags

Per-customer figures need to know whose money each recurring posting is. Ledgeline decides that in this order:

1. A `customer-id:` tag on the posting. The name shown is its `customer:` tag, or the id when there isn't one. Use this when two customers share a name or a customer renames itself.
2. A `customer:` tag on the posting.
3. A `customer:` tag on the transaction.
4. The payee, which is the description up to the `|`. This fallback is used only when the payee isn't empty, the transaction has exactly one recurring posting, has no deferred-revenue leg, and has a receivable or cash leg, and the posting isn't a billing-clearing invoice (see [Deferred-revenue books](#deferred-revenue-books)). That keeps a month-end "Revenue recognition" entry from becoming one huge customer. Customers found this way are marked with an asterisk and a footnote saying they were inferred from payees.

Anything else is unattributed. Names are matched exactly after trimming spaces, so `Contoso Ltd` and `Contoso Ltd.` are two customers. Ledgeline doesn't merge them for you, but [Problems](../journal/problems.md#customer-name-variants) points them out.

Customer metrics need coverage: at least 90% of recurring revenue over the span's complete months must belong to a customer. Customers inferred from payees count. Below that, the customer boxes hide and one line under the section says why. There are three versions of it, each followed by a link to the section of this page that explains the fix:

| Reason | Line shown |
| --- | --- |
| No recurring posting has a customer | Customer metrics hidden: no recurring revenue has a `customer:` tag. |
| Some do, but under 90% | Customer metrics hidden: 62% of recurring revenue has a `customer:` tag (90% needed). |
| The books record billings on the recurring account | Customer metrics hidden: your `customer:` tags are on invoices, but the monthly revenue-recognition lines that make up MRR have none, so per-customer figures would be billings, not MRR. Tag the recognition lines to turn these on. |
| The books record billings on the recurring account, and customers were billed | Per-customer MRR figures hidden: your `customer:` tags are on invoices, but the monthly revenue-recognition lines that make up MRR have none, so per-customer figures would be billings, not MRR. Tag the recognition lines to turn these on. Customer counts are from billings. |

Whenever the Customers box is [counted from billings](#customers-from-billings), the line starts "Per-customer MRR figures hidden:" instead of "Customer metrics hidden:" and ends "Customer counts are from billings.", whatever the reason.

The fixes are under [Customer tags](#customer-tags) and [Deferred revenue books](#deferred-revenue-books).

### A journal that turns everything on

This is the smallest journal that shows every box. Two customers subscribe in January, one of them on an annual plan paid up front, and a third joins in February.

```journal
account assets:bank:operating             ; type: C
account assets:receivable                 ; type: A, kpi: receivable
account liabilities:payable               ; type: L, kpi: payable
account liabilities:deferred-revenue      ; type: L, kpi: deferred-revenue
account equity:funding                    ; type: E, cfsection: financing
account revenue:subscriptions             ; type: R, kpi: recurring
account revenue:subscriptions:smb         ; segment: smb
account revenue:subscriptions:enterprise  ; segment: enterprise
account revenue:usage                     ; type: R, kpi: usage
account revenue:services                  ; type: R
account expenses:hosting                  ; type: X, issection: cogs
account expenses:marketing                ; type: X, kpi: sales-marketing
account expenses:engineering              ; type: X, kpi: rnd
account expenses:admin                    ; type: X, kpi: ga

2026-01-02 Seed round
    assets:bank:operating                $500,000.00
    equity:funding

2026-01-05 Northwind | January subscription
    assets:receivable                      $1,000.00
    revenue:subscriptions:smb             $-1,000.00  ; customer: Northwind

2026-01-10 Globex | Annual plan, invoiced up front
    assets:receivable                     $24,000.00
    liabilities:deferred-revenue         $-24,000.00  ; customer: Globex

2026-01-31 Revenue recognition, January
    liabilities:deferred-revenue           $2,000.00
    revenue:subscriptions:enterprise      $-2,000.00  ; customer: Globex

2026-01-31 Northwind | January API overage
    assets:receivable                        $200.00
    revenue:usage                           $-200.00  ; customer: Northwind

2026-01-31 Cloudhost | January hosting
    expenses:hosting                         $600.00
    liabilities:payable

2026-01-31 Month-end costs
    expenses:marketing                     $8,000.00
    expenses:engineering                  $20,000.00
    expenses:admin                         $4,000.00
    assets:bank:operating

2026-02-04 Northwind | Payment
    assets:bank:operating                  $1,200.00
    assets:receivable

2026-02-05 Northwind | February subscription
    assets:receivable                      $1,000.00
    revenue:subscriptions:smb             $-1,000.00  ; customer: Northwind

2026-02-09 Globex | Payment
    assets:bank:operating                 $24,000.00
    assets:receivable

2026-02-12 Initech | February subscription
    assets:receivable                        $500.00
    revenue:subscriptions:smb               $-500.00  ; customer: Initech

2026-02-15 Cloudhost | Payment
    liabilities:payable                      $600.00
    assets:bank:operating

2026-02-20 Initech | Onboarding
    assets:receivable                      $3,000.00
    revenue:services                      $-3,000.00  ; customer: Initech

2026-02-28 Revenue recognition, February
    liabilities:deferred-revenue           $2,000.00
    revenue:subscriptions:enterprise      $-2,000.00  ; customer: Globex

2026-02-28 Month-end costs
    expenses:hosting                         $700.00
    expenses:marketing                     $9,000.00
    expenses:engineering                  $20,000.00
    expenses:admin                         $4,000.00
    assets:bank:operating
```

The details that matter:

- `kpi: recurring` sits on the parent `revenue:subscriptions`, and both leaves inherit it. Each leaf carries its own `segment:`.
- The annual plan goes to deferred revenue when it's invoiced, and a monthly entry recognises one twelfth of it. The `customer:` tag is on the recognition line, because that is the revenue.
- The overage goes to `revenue:usage` and the onboarding fee to `revenue:services`, so neither counts as MRR.
- The seed round is tagged as financing, so it doesn't count as negative burn.

With **Month over month** and End set to 2026-02-28, February is the current period and January the previous one:

| Box | Figure | Why |
| --- | --- | --- |
| ARR (2026-02 × 12) | $42,000.00 | February's MRR of $3,500.00 × 12. The $200.00 of usage was in January and the $3,000.00 onboarding fee isn't recurring. |
| Net new ARR | $6,000.00 | 12 × ($3,500.00 − $3,000.00) |
| Customers | 3 | Northwind, Globex and Initech, with one new logo |
| Burn multiple | 4.6× | Net burn adjusted for prepayments of $27,800.00 ÷ $6,000.00. Cash fell only $9,100.00 (from $468,000.00 to $458,900.00), because Globex paid January's annual invoice in February. The adjustment takes off the $2,000.00 of deferred revenue recognised and adds back the $20,700.00 of receivables collected: $9,100.00 − $2,000.00 + $20,700.00. |
| Gross margin | 89.2% | ($6,500.00 revenue − $700.00 hosting) ÷ $6,500.00 |

NRR stays hidden, because it needs a customer list from twelve months earlier.

## Months and periods

The section uses the same span and the same two halves as the rest of Insights. Inside each half it works in calendar months:

- A month is **complete** when it lies wholly inside the span and has finished: its last day is on or before both the span's end and today. A span that reaches into the future therefore measures the last month that has actually ended.
- **Point-in-time figures** (MRR, ARR, customers, cash, deferred revenue, receivables, payables, concentration) use the last complete calendar month in the half. A month that is only partly inside the span, or hasn't finished yet, is never used for them.
- **Flow figures** (the bridge, burn, spend, revenue, gross profit, billings) add up the complete months inside the half.
- **NRR and GRR** compare the half's last complete month with the same month a year earlier, even when that month is before the span starts.

A month is also complete only if it isn't after the **data horizon**: the last month with any `kpi: recurring` posting dated on or before the span's end. Books are usually closed a few weeks late, so the month you're in, or the one before it, may have no revenue recognition booked yet. Read as complete, it would show every customer churning at once. Instead the metrics stop at the horizon, and the section's heading says **Closed through 2026-06** (with the month) whenever the horizon is before the span's last month. Later months are marked `(open)` in the export, and nothing uses them. A journal with no recurring revenue has no horizon.

The presets all end on a month end, so every month is complete. A custom span that ends mid-month uses the month before it for point-in-time figures. A half that holds no complete month shows no point-in-time figures at all. Postings dated after the span's end are ignored, and a partial first month includes its postings from before the span's start.

Figures are in the base currency, valued with the same prices as the rest of Insights. Each month is valued at its own month end. Recurring revenue in a commodity with no price is left out of MRR, and a footnote names it.

## Each box

### ARR

**ARR = the latest complete month's recurring revenue × 12.** The box title names that month, as in **ARR (2026-06 × 12)**. Under the figure it shows MRR, which is the month's recurring revenue itself, and usage revenue for the same month. The change line compares ARR with the previous period's.

Usage revenue is shown beside ARR and never added to it, because metered revenue isn't contracted to repeat. Services and other revenue without `kpi: recurring` are left out entirely.

There is more than one definition of ARR in use:

- **Run-rate ARR**, which is what Ledgeline shows: recognised revenue for one month, annualised. It moves with every credit, proration and one-month discount.
- **Contracted ARR**: the annual value of every subscription active on a date, from a billing system. A customer who signs on the 20th counts in full on the 21st, whereas run-rate ARR shows ten days of revenue in that month and the full amount the month after.
- **Trailing twelve months of revenue**, which some people call ARR. It lags a growing business by about six months.

Your books hold recognised revenue, not contracts, so Ledgeline uses run-rate and labels it as such. If an investor asks for contracted ARR, it comes from your billing system.

### Net new ARR

**Net new ARR = 12 × (MRR at the end of the current period − MRR in the month before it started).** It is the ARR added over the period after churn and contraction.

The small line shows the compound monthly growth rate (CMGR) of MRR over the same months, in % per month: the steady monthly growth rate that would turn the starting MRR into the ending MRR. A period of 12 months with 5% CMGR grew by 80% over the year.

### Customers

**Customers = the number of customers with MRR above zero in the latest complete month.** Under the figure, a line such as `4 new · 1 churned` counts the customers gained and lost during the period. The small line is customer growth in percent, from the count in the month before the period started. An asterisk after the figure means some customers were [inferred from payees](#customer-tags).

This box needs the customer tier: at least 90% of recurring revenue attributed to a customer.

### Customers from billings

Some books put `customer:` on the invoices and not on the monthly recognition lines, so per-customer MRR can't be built (see [Deferred revenue books](#deferred-revenue-books)). Ledgeline can still count customers from what they were billed. When the customer tier is hidden and the books hold billings, the Customers box comes back as **Customers (from billings)**.

A **billing** is a credit to a `kpi: recurring` or `kpi: deferred-revenue` account, in a transaction with a receivable or cash leg, attributed to a customer by `customer-id:` or `customer:` on the posting or the transaction. Otherwise the payee names the customer (marked with an asterisk, as inferred), but only when the transaction holds exactly one such credit.

- **The count** is the customers billed in the twelve months up to and including the latest complete month. An annual customer billed once a year stays counted all year.
- **New** customers had their first-ever billing during the period. **Lapsed** customers went twelve months without a billing during it. The line under the figure reads like `3 new · 1 lapsed`.
- **The small line** is the change in the count from the month before the period started.

**More metrics** adds **ACV (from billings)**, billings over the last twelve months ÷ the customers billed in them, and **Logo churn (from billings)**, lapsed customers ÷ customers billed in the twelve months before the period.

NRR, GRR, the MRR bridge, ARPA, the ordinary ACV and logo churn, the quick ratio, customer concentration, CAC and CAC payback stay hidden: each needs MRR per customer. Customers from billings is never shown when the customer tier itself is available.

Until the journal reaches back twelve months before the latest complete month, the count can only see part of a year of billings. A line under the section then says: "Counts may be low until there are 12 months of history."

### NRR

**Net revenue retention = what last year's customers pay now, as a share of what they paid then.** Take every customer with MRR above zero in the same month a year before the latest complete month. Add up their MRR now, and divide by their MRR then:

```text
NRR = Σ MRR now of last year's customers  ÷  Σ their MRR a year ago  × 100
```

Customers who joined in the last twelve months are left out, so NRR measures only what happened to existing revenue. Above 100% means expansion outweighed churn and contraction. The box says "trailing 12 months" under the figure.

The small line shows **gross revenue retention** (GRR). It is the same calculation with each customer capped at what they paid a year ago, so expansion can't hide churn. GRR is never above 100% and never above NRR.

Definitions vary, and the differences can move the figure by several points:

- **The cohort.** Ledgeline uses a single month a year ago. Some companies use every customer active at any point in the year, or average twelve monthly figures.
- **Customers who left and came back.** A customer who paid a year ago, churned and returned counts at today's MRR. One who wasn't paying a year ago isn't in the cohort, even if they paid before that.
- **ARR or MRR.** Using ARR gives the same ratio, because both sides are multiplied by 12.

NRR hides until the journal reaches back twelve months before the latest complete month.

### Burn multiple

**Burn multiple = net burn ÷ net new ARR**, over the current period. It is how many dollars the business burned for each dollar of ARR it added. David Sacks, who proposed it, reads it roughly like this. The box prints the reading under the figure, and its colour follows the same bands:

| Burn multiple | Reading | Colour |
| --- | --- | --- |
| under 1 | Very efficient | green |
| 1 up to and including 2 | Good | neutral |
| over 2, up to and including 3 | Suspect | amber |
| over 3 | Bad | red |

**Net burn** is the cash the business used, not its loss:

```text
net burn = −(change in cash − financing cash flow)
         = −(operating cash flow + investing cash flow)
```

Cash is every `type: C` account. When you hold cash in more than one currency, the exchange-rate effect on cash counts as burn too. Financing is the financing section of the [cash flow statement](cash-flow.md), so a funding round or a loan doesn't count as negative burn and a loan repayment doesn't count as burn.

When the books have a `kpi: deferred-revenue` account or a receivable, the burn multiple uses net burn **adjusted for prepayments**, the same adjusted burn as [Runway](#runway):

```text
adjusted burn = net burn + change in deferred revenue − change in receivables
```

With annual prepayments, cash burn mostly measures when customers happened to pay: a quarter in which several renewals were paid up front looks almost free, and the next one looks expensive, though nothing about the business changed. The adjusted burn counts revenue when it's earned and costs when they're incurred, apart from payables. Sacks's original definition uses cash burn, and that is what Ledgeline uses for books without either account. The small line says which: `net burn $44,070.00 (adjusted for prepayments)`, or `net burn $43,720.00` on cash.

Some people use net loss from the income statement instead. That is close to the adjusted burn, but it spreads capital purchases out as depreciation and counts every cost when it's incurred, whereas the adjusted burn counts equipment when it's bought and costs when they're paid.

The box hides when net burn or net new ARR is zero or negative. A business that isn't burning, or whose ARR shrank, has no meaningful multiple.

### Gross margin

**Gross margin = (revenue − cost of goods sold) ÷ revenue**, over the current period. Revenue is all revenue, recurring or not. Cost of goods sold is every account the income statement puts in its COGS section, which means `issection: cogs` on it or a parent.

The small line is the change from the previous period, in percentage points. The box hides when no account is tagged `issection: cogs`.

### Runway

Runway is on the [Insights](insights.md#runway) page, because it takes Net Worth's slot in the headline row. In short, **runway = cash at the end of the latest complete month ÷ average burn over the last three complete months**. Negative cash reads as 0 months.

When the journal has a `kpi: deferred-revenue` account or a receivable, the burn is **adjusted for prepayments**: read as if every customer paid when they were served, so a big annual prepayment doesn't make one month look like a windfall.

```text
adjusted burn = net burn + change in deferred revenue − change in receivables
```

An annual prepayment raises cash and deferred revenue by the same amount and nets to zero. Each month's recognition lowers deferred revenue and counts as income. An unpaid invoice counts when it's issued, and its payment later nets to zero. Without either account, runway uses plain net burn.

The figure reads like **117 months**. On adjusted burn, the small line gives both burns, like `cash-out ~2036-03 · burn $3,693.33/mo adjusted for prepayments · $6,543.33/mo cash (3-mo avg)`; on cash burn, like `cash-out ~2031-12 · burn $7,000/mo (3-mo avg)`. When the burn the runway uses averages zero or less, the box says **Not burning** and shows cash instead. [Burn multiple](#burn-multiple) uses the same adjusted burn, summed over the period.

### More metrics

Under the boxes, **More metrics** opens a list of secondary figures. It starts collapsed and remembers whether you left it open. Each row appears only when it has a value.

| Metric | Definition | Needs |
| --- | --- | --- |
| ARPA | MRR from attributed customers ÷ customers, in the latest complete month | Customers |
| ACV | The average, over the period's new customers, of 12 × their first month's MRR | Customers |
| Logo churn | Customers lost during the period ÷ customers in the month before it | Customers |
| Quick ratio | (new + expansion + reactivation) ÷ (contraction + churn), over the period | Customers |
| Top customer share | The largest 1, 5 and 10 customers' share of MRR, shown as `top 1 · top 5 · top 10` | Customers |
| CAC | Sales and marketing spend ÷ new customers, over the period | Customers, `kpi: sales-marketing` |
| CAC payback | Months of gross profit from new and expansion MRR it takes to repay sales and marketing spend | Customers, `kpi: sales-marketing`, COGS |
| S&M % | Sales and marketing spend ÷ total revenue, over the period, shown as "x% of revenue" | `kpi: sales-marketing` |
| R&D % | Research and development spend ÷ total revenue, the same way | `kpi: rnd` |
| G&A % | General and administrative spend ÷ total revenue, the same way | `kpi: ga` |
| Rule of 40 | Revenue growth % + free cash flow margin % | Business |
| DSO | Receivables at month end ÷ revenue over the period × days in the period | A receivable |
| DPO | Payables at month end ÷ all expenses over the period × days in the period | A payable |
| Deferred revenue | The balance of deferred revenue at the end of the latest complete month | `kpi: deferred-revenue` |
| Billings | Revenue over the period + the change in deferred revenue over it | `kpi: deferred-revenue` |
| ACV (from billings) | Billings over the last 12 months ÷ the customers billed in them | [Customers from billings](#customers-from-billings) |
| Logo churn (from billings) | Customers who lapsed during the period ÷ customers billed in the 12 months before it | [Customers from billings](#customers-from-billings) |

A few of these have their own disputes:

- **ACV** usually means the annual value of a signed contract. Ledgeline has no contracts, so it annualises each new customer's first month. A customer whose first month is prorated reads low. Total contract value (TCV), bookings and remaining performance obligations aren't shown, because your books don't hold them.
- **CAC** here is blended: all sales and marketing spend divided by all new customers, with no lag between spend and signing. Spend that drives expansion is counted too.
- **Rule of 40** compares this period's revenue with the same number of months just before it. When those months had no revenue, it hides. It is meant to use annual growth, so read it on the **Year over year** comparison. On Quarter over quarter it adds one quarter's growth to the margin. Free cash flow is operating plus investing cash flow.

## The MRR bridge

The bridge explains how MRR got from the start of the current period to its end. It is built per customer, month by month, and each customer's change in a month falls into one category:

| Last month | This month | Category | Amount |
| --- | --- | --- | --- |
| 0 | above 0, first time ever | New | this month |
| 0 | above 0, paid before | Reactivation | this month |
| above 0 | 0 | Churn | last month |
| above 0 | higher | Expansion | the increase |
| above 0 | lower, above 0 | Contraction | the decrease |

The list shows starting MRR, then each category that isn't zero with its customer count and amount, then ending MRR. Click a category to see its five largest customers, by their total in that category over the period. Each customer's name links to the Journal, searching for that name across all dates. The search is a plain text match on the whole transaction, so it also finds transactions that mention the name without a `customer:` tag, and a short name such as `Acme` also finds `Acme Labs`.

A customer churns in the first month their MRR is zero or below. There is no grace period, so a customer who skips a month and comes back shows as churn followed by reactivation, not as nothing.

### The Other line

For the bridge, a customer's month is never counted below zero. A credit month where a refund is bigger than that month's charge counts as zero, so it reads as churn, and the next normal month reads as reactivation.

The real MRR still includes that negative amount, and it also includes unattributed revenue. The difference between the customer sums and the real MRR goes on an **Other** line, so the bridge always adds up:

```text
end MRR = start MRR + new + expansion − contraction − churn + reactivation + other
```

Other appears only when it isn't zero. A large Other usually means a lot of recurring revenue has no `customer:` tag, or a refund was booked to the recurring account.

### Segment moves

In a segment breakdown, each customer's revenue is split by the segment it came from in each month. A customer who moves from `smb` to `enterprise` shows as churn in `smb` and new in `enterprise` in the month of the move. In the total bridge the two cancel, leaving only the customer's real change in MRR.

### Segments

When a dimension has at least two values, the ARR, Customers and NRR boxes get a chevron. It opens an **MRR by segment** table under the row, with one line per value. The columns are the dimension's values, MRR, Δ (the change in MRR), Customers, ARPA and NRR. `(unassigned)` is always last. With more than one dimension, the **Break down by** picker chooses which one to show.

## Deferred revenue books

Many books bill a customer through revenue and then move it to deferred revenue:

```journal
2026-01-10 Globex | Invoice 1042
    assets:receivable                     $24,000.00
    revenue:sales                        $-24,000.00  ; customer: Globex

2026-01-10 Move to deferred
    revenue:sales                         $24,000.00
    liabilities:deferred-revenue         $-24,000.00

2026-01-31 Revenue recognition, January
    liabilities:deferred-revenue           $2,000.00
    revenue:sales                         $-2,000.00
```

MRR is still right: `revenue:sales` nets to the recognised $2,000.00, and the invoice and the move cancel. But the `customer:` tag is on the invoice, so the only customer figures Ledgeline could build would be billings, not MRR. Globex would show $24,000.00 of MRR in January and churn in February.

Ledgeline spots this pattern, looking only at the span's complete months:

- A **clearing account** is a recurring account that is debited, in one of those months, in a transaction that credits a deferred revenue account (the move to deferred).
- A **clearing posting** is a credit to a clearing account, in one of those months, in a transaction with no deferred revenue leg (the invoice). Recognition lines always have a deferred revenue leg, so they never are.
- Customer metrics hide when more than half of the recurring revenue attributed to customers in those months sits on clearing postings.

So books that recognise revenue from deferred revenue with `customer:` on the recognition lines keep their customer figures, even with the odd correcting move to deferred. An untagged clearing invoice is also never attributed to its payee. This reason takes precedence over the coverage ones, because in these books coverage often looks complete. It hides the customer metrics with a line saying your `customer:` tags are on invoices rather than on the recognition lines, and [Problems](../journal/problems.md#recurring-revenue-moved-to-deferred) lists the account. The invoices still count as billings, so the Customers box comes back [counted from billings](#customers-from-billings). Pure moves like the second transaction above score nothing in Insights' [Top Transactions](insights.md#top-transactions), and drop out.

To get customer figures from books like these:

1. **Tag the recognition lines.** Split each month's recognition entry by customer, with `customer:` on each revenue posting. That is where recurring revenue really is, and it is what the [example](#a-journal-that-turns-everything-on) does.
2. **Invoice straight to deferred revenue.** Post the invoice to `liabilities:deferred-revenue` and leave out the move, so revenue only ever holds recognised amounts.
3. **Use a leaf account per segment.** If revenue sits on the parent's own postings (`revenue:sales`) as well as on subaccounts, tag the leaves with `segment:` and stop posting to the parent.
4. **From QuickBooks, export with names.** The import writes each line's Name as `customer:` on revenue, deferred revenue and receivable lines. See [Importing from QuickBooks](../imports/quickbooks.md#customer-names-on-each-line).

## Check it with hledger

Every figure comes from an hledger query you can run yourself. These commands use the tags described above and were checked with hledger 1.52.

| Figure | Command |
| --- | --- |
| MRR by month | `hledger bal tag:kpi=recurring -M --invert` |
| MRR by segment | `hledger bal tag:kpi=recurring --pivot segment -M --invert` |
| MRR by QuickBooks class | `hledger bal tag:kpi=recurring --pivot class -M --invert` |
| MRR by customer and month | `hledger bal tag:kpi=recurring --pivot customer -M --invert -O csv` |
| A parent account's own postings | `hledger bal '^revenue:sales$' -M --invert` |
| Deferred revenue balance | `hledger bal tag:kpi=deferred-revenue -M -H --invert` |
| Billings | `hledger bal "expr:type:R or tag:kpi=deferred-revenue" -M --invert` |
| Cost of goods sold | `hledger bal tag:issection=cogs -M` |
| Cash at each month end | `hledger bal type:C -M -H` |
| Change in cash | `hledger cashflow -M` |

ARR is the MRR command's latest full month × 12. Billings is the total row of its command: revenue plus the growth in deferred revenue.

`hledger cashflow` shows the change in your cash accounts, not an operating, investing and financing split. For net burn, subtract the financing flows. If your financing accounts carry `cfsection: financing`, `hledger bal tag:cfsection=financing -M --invert` lists them. On the example journal, January's cash change of $468,000.00 less $500,000.00 of financing is $32,000.00 of net burn.

The customer CSV is the matrix behind the bridge, NRR and the customer count. Customer counts come from it too: count the non-zero cells in a month's column.

### Traps

- **Anchor account patterns.** `hledger bal revenue` matches any account with `revenue` anywhere in its name, which includes `liabilities:deferred-revenue`. Use `'^revenue'`, or `'^revenue:sales$'` for a parent's own postings without its subaccounts.
- **`tag:` and `--pivot` disagree about inherited tags.** hledger passes every ancestor's tags down to its subaccounts. With `segment: smb` on `revenue:sales` and `segment: enterprise` on `revenue:sales:enterprise`, `tag:segment=smb` matches both accounts. `--pivot segment` uses the subaccount's own value and puts it under `enterprise`. Ledgeline follows `--pivot`: the nearest account's value wins.
- **Payee tags don't reach postings.** A `customer:` tag on a `payee` directive is not seen by `tag:customer=` or `--pivot customer`, and Ledgeline doesn't read it either. Put the tag on the posting or the transaction.
- **A tag can follow free text.** In `; Recognised revenue, customer: Acme` the tag is `customer: Acme`. A tag's value ends at the next comma, so `customer: Acme, Inc.` is `Acme` plus a tag-less fragment. Leave commas out of customer names.
- **hledger can't count distinct values.** There is no query for "how many customers had revenue in March". Export the customer CSV and count the cells.
- **Postings with no value.** `--pivot` puts postings without the tag in a row with a blank name. That row is Ledgeline's `(unassigned)`.
- **`issection:` is Ledgeline's tag.** hledger reads it as a plain tag, so `tag:issection=cogs` finds the accounts, but `hledger is` doesn't give COGS its own section.
- **Payee inference is Ledgeline's.** Customers found from payees, and names from `customer-id:`, don't appear in hledger's `--pivot customer`. Their revenue is in the blank row there.

## Export

When the journal has recurring revenue, the Insights tab shows an **Export .xlsx** button. Without recurring revenue there is no button. It downloads `startup-metrics-<start>-to-<end>.xlsx` for the comparison span on screen. The workbook has up to five sheets:

| Sheet | Contents |
| --- | --- |
| Metrics | One column per month across the whole span, with rows for MRR, ARR, usage, customers, each bridge category, revenue, gross margin %, net burn, net burn adjusted for prepayments, cash, deferred revenue, S&M, R&D and G&A. A row appears only when the journal supports it. Partial months are marked in the header, and their ARR cell is left blank. |
| Periods | The current and previous period's figures side by side, including the runway basis and, when shown, the customers from billings. |
| By segment | For each dimension, one row per value: MRR now and before, customers, ARPA and NRR. Left out when no dimension is offered. |
| Customer MRR | Every customer's MRR in every month. Left out when there are no customer figures. |
| Definitions | One row per metric: its formula in words, the roles and accounts that fed it, the customer coverage and the span. |

Numbers are written as real numbers in the base currency, so they can be charted and summed in a spreadsheet.

## Pitfalls

**Invoice and deferral in different months.** If an invoice is posted to revenue on January 31 and moved to deferred revenue on February 1, January's MRR includes the whole invoice and February's goes negative. Date both on the same day, or invoice straight to deferred revenue.

**Prepayment booked straight to revenue.** A $24,000.00 annual payment posted to a recurring account reads as $24,000.00 of MRR in one month, then churn the next. Post it to deferred revenue and recognise $2,000.00 a month.

**Proration.** A customer who starts on the 20th has a partial first month. That month reads as a small new customer and the next as expansion, and ACV reads low. Run-rate ARR settles once a full month has passed.

**Credits and refunds.** A credit booked to the recurring account lowers MRR in its month. If it's bigger than the charge, the customer reads as churned that month and reactivated the next, and the bridge's Other line absorbs the negative remainder. If the credit belongs to an earlier month, date it there.

**Recognition date.** Revenue counts in the month of the posting's date. A `date:` tag on a posting moves it, exactly as on the income statement. A recognition entry dated the 1st of the next month lands in the wrong month.

**Foreign currency.** Recurring revenue in another currency is valued in the base currency with your prices. A rate change then moves MRR, and shows up as expansion or contraction for that customer. Revenue in a commodity with no price at all is left out of MRR and named in a footnote.

**Name variants.** `Contoso Ltd` in one month and `Contoso Ltd.` in the next are two customers: one churns and one is new. [Problems](../journal/problems.md#customer-name-variants) lists names that differ only by case, punctuation or a legal suffix. Pick one spelling, or give both a `customer-id:`.

## API

The section reads `GET /api/saas`, which takes the same `start` and `end` as `/api/insights`. `dims=` sets which dimension tags to consider, as a comma-separated list. It defaults to `segment,product,class,region`. `detail=customers` adds the per-customer matrix the export uses. `asOf=YYYY-MM-DD` is today's date for deciding which months have finished; it defaults to the server's current date (UTC), as on `/api/subscriptions`.
