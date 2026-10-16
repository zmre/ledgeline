---
title: Subscriptions
description: "How Ledgeline finds recurring monthly and annual charges, and how to add or remove one with a subscription: tag."
order: 50
tags: [reports]
---

# Subscriptions

The Subscriptions tab lists the charges that recur every month or every year, with what each costs per year and when the next one is due. Nothing in a plain-text journal marks a charge as a subscription, so Ledgeline infers them from the pattern of your spending.

<!-- screenshot: Subscriptions tab with both boxes, the per-month and per-year totals circled -->

## Reading the tab

The tab has no controls. It always scans the 24 months ending today, and the line at the top states that window.

There are two boxes, Annual Subscriptions and Monthly Subscriptions. Each header shows the box's total per month and per year. Rows are sorted by yearly cost, so the charge that would save the most if you cancelled it comes first.

Each row shows:

- the payee,
- the date of the next expected charge,
- the typical charge, which is the median of the matched charges,
- for monthly rows, the yearly cost underneath.

A **tagged** badge marks a row that is there because of a `subscription:true` tag rather than detection.

Click a row to open the journal filtered to that payee over all dates, so you can see the actual charges.

## How detection works

The detector reads expense transactions in the 24-month window and works in four steps.

1. It groups charges by payee. The payee is the part of the description before `|`, following hledger's `payee | note` convention. Only the expense side of each transaction counts, valued in the base currency.
2. Within each payee, it clusters charges whose amounts are within 15% of each other. This separates a steady $9.99 iCloud charge from the one-off app purchases billed by the same merchant.
3. It checks each cluster's timing. Every charge must land within 4 days of the cluster's usual day of the month. For monthly, the median gap between charges must be 26 to 35 days. For annual, it must be 330 to 400 days, and the charges must fall in the same month each year, give or take one.
4. It requires enough repeats: at least 5 charges for monthly and 2 for annual. An annual pattern must also account for at least 80% of that payee's charges, so a supermarket you visit every week doesn't produce a false annual subscription.

Several more rules keep the list honest:

- Accounts are classified by declared `type:` first, so costs booked outside an `expenses:` root still count. See [Settings → Accounts](../settings/accounts.md).
- A transaction with a revenue leg is skipped. Without this, a paycheck with payroll-tax withholding would look like a large monthly subscription.
- Anything whose description contains `mortgage` is left out. It recurs reliably, but it is debt service, not something you can cancel.
- Two charges on the same day at the same price count once.

## When a subscription drops off

A charge that stops appearing is treated as cancelled, but only once the account that pays it has newer activity. A monthly charge gets 3 months of grace and an annual charge gets 15. The grace is measured against the latest transaction on the paying account, not against today. If you haven't imported a card's statements for a while, its subscriptions stay on the list.

## What it won't catch

Variable bills don't form a tight amount cluster, so utilities, groceries and usage-based charges are usually missing. Quarterly, weekly and biweekly charges are left out by the timing check. A subscription younger than 5 months won't appear in the monthly box yet.

## Overriding the detector

Tag a transaction to overrule detection. The tag can sit on the transaction or on one of its postings.

| Tag | Effect |
| --- | --- |
| `subscription:true`, `subscription:yes` or `subscription:monthly` | Puts the payee on the monthly list, whatever the amounts do |
| `subscription:annual` or `subscription:yearly` | Puts the payee on the annual list |
| `subscription:false` or `subscription:no` | Removes the matching charge cluster from the list |

```journal
2026-03-14 City Power | electric bill  ; subscription:true
    expenses:utilities:electric    $87.12
    assets:bank:checking
```

A `subscription:false` affects only the cluster of charges it sits in. Marking a one-off purchase from Apple doesn't hide the iCloud subscription billed by Apple too. A tagged transaction is also exempt from the `mortgage` exclusion.

> [!TIP]
> hledger reads a tag's value up to the next comma. If the comment already has a tag, separate them with a comma, as in `; category:infra, subscription:true`. Without the comma, the new tag becomes part of the previous tag's value.

The detection code is in [`crates/ledgeline-core/src/reports/subscriptions.rs`](https://github.com/zmre/ledgeline/blob/main/crates/ledgeline-core/src/reports/subscriptions.rs).
