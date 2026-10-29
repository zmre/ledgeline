---
title: Accounts
description: "The Accounts report: every account per period, assets and liabilities as balances with a Δ, everything else as each period's movement with a Total."
order: 47
tags: [reports, accounts]
---

# Accounts

Every account in your journal, one row each, one column per period. It's the
plain ledger the statements are built from, before groups, sections and
valuation shape it.

Accounts are grouped by type, and each group reads the way that kind of
account is read:

| Section | Each period's column | The last column |
| --- | --- | --- |
| Assets, Liabilities | the **balance** at the end of the period, everything before it included | **Δ**: how much the balance changed from the start of the first period to the end of the last |
| Equity, Revenue, Expenses | what **moved** in the period | **Total**: the sum of the periods |

Every figure in a Δ column is marked "Δ", so a quick read never takes a change
for a balance. Each section has a total row, and there is no grand total,
because adding balances to movements means nothing. Accounts with no type
declared and none that can be inferred are listed under **Other accounts** and
read as movements.

Amounts are shown as written, one line per commodity, and are not valued into
one currency. Signs are hledger's: revenue and liabilities are negative.

## Controls

Accounts uses the same window as [Cash flow](cash-flow.md): **From**, **To**
and **Interval** (Total, Monthly, Quarterly or Yearly), and the two tabs share
the range, so switching between them keeps it. The **depth** slider rolls the
tree up; it never changes a section's totals. **Show empty accounts** adds every
account the journal declares or has ever posted to, as zero rows where nothing
moved.

## The hledger commands

The asset and liability columns are `hledger balance -H -M` (each period's
closing balance). The other columns, and every row's last column, are
`hledger balance -M -T`. Both use the same `--tree --depth` and no `--value`.

The same report over a projection is the [Accounts reading on
Projections](../projections/index.md#accounts).
