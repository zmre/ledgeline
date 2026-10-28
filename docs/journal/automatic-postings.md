---
title: Automatic postings
description: "How Ledgeline applies hledger's `= QUERY` auto-posting rules to your journal under --auto, shows the postings they add, and sends edits to the rule."
order: 25
tags: [journal, auto-postings, hledger-conf]
---

# Automatic postings

An automatic posting rule adds postings to every transaction that matches a query. hledger calls these *auto postings*. A rule is a line starting with `=` and a query, followed by indented postings:

```journal
; Sales tax collected on every sale.
= revenue:sales
    liabilities:tax:sales      *0.1
    assets:checking
```

With that rule, a sale of `$-333.33` to `revenue:sales` gains two postings directly below it: `$-33.333` to `liabilities:tax:sales`, and `$33.333` to `assets:checking`, the amountless posting that balances it.

## Turning rules on

hledger applies these rules only when it runs with `--auto`, and Ledgeline does the same. It reads the setting from the `hledger.conf` that applies to your journal. If that file has `--auto` in its general section, at the top before any `[section]` heading, the rules apply:

```
# hledger.conf beside the journal
--auto
```

There is no separate switch in Ledgeline, so the app and hledger in your terminal always agree. `--auto` inside a command section such as `[balance]` only affects that hledger command, so Ledgeline ignores it. Rules never apply when the file's first word replaces hledger's command, because then none of its options take effect.

Ledgeline re-reads the setting each time it reloads the journal. Change the file and the journal, reports and Problems drawer update.

Rules in projection scenarios work differently: they always apply to that scenario's forecasts. See [Projections](../projections/index.md).

## What the rules change

When `--auto` is on, Ledgeline uses the transactions with the rules' postings added for everything it shows: reports, the budget, holdings, insights and the Problems drawer. It follows hledger in these ways:

- Rules apply in file order, and a later rule can match postings an earlier rule added. A rule never matches its own postings.
- Rules in an `include`d file apply to the whole journal.
- `*0.1` multiplies the matched amount exactly, so `$-333.33` becomes `$-33.333`. Reports still round to the precision your journal writes, so `$-33.33` shows in a balance.
- A `tag:` query also matches tags a posting inherits from its account's `account` directive.
- [Balance assertions](problems.md#balance-assertion) are checked after the rules apply, as in hledger.
- In the [budget](../budget.md), rules add to the actual amounts and never to the goals.
- [Subscription detection](../reports/subscriptions.md) ignores the postings rules add, so a tax rule doesn't make a new subscription.

Your journal files never change. The added postings exist only while Ledgeline reads the journal.

## In the journal

A transaction a rule changed still shows its own accounts as `source → destination`, and its own amount. A muted **+2 auto** badge after the account chips counts the postings rules added. Hover it to see which rules. Click it to open the first of those rules in Settings.

The transaction dialog lists the added postings read-only, below the postings you can edit, each with a **⚙ from rule** link. Saving the dialog writes only your own postings.

You can't edit an added posting directly. To change it, change its rule. If an edit tries to reach one anyway, Ledgeline refuses it and names the rule.

## Settings › Automatic postings

This tab lists every rule in your journal and its included files. For each rule you see:

- Its file and line, such as `rules.journal:5`, its query, and its text as written.
- How many postings it added, while `--auto` is on.
- A lock if Ledgeline can't read its query, such as a complex `expr:`. A locked rule never applies here, so Ledgeline and hledger can disagree about it.
- Any problem it causes, listed below.

The tab also says whether `--auto` is on, and which config file says so. Change that in the config file.

## Problems a rule can cause

Ledgeline never refuses to open a journal because of a rule. It reports these in the [Problems drawer](problems.md) and on the rule's card:

| Problem | Severity | What hledger does |
| --- | --- | --- |
| The rule's postings don't balance, such as a fixed `$10` with no amountless posting to balance it | error, on the transaction | refuses the journal under `--auto` |
| The rule has an amountless posting and fires twice in one transaction, for example on two child accounts | error, on the transaction | refuses the journal under `--auto` ("there can't be more than one real posting with no amount") |
| The rule posts to an account nothing else in the journal declares or uses | warning, on the account | accepts it |

For the second case, Ledgeline balances each firing on its own. To make hledger agree, give the posting an explicit amount such as `*-0.1` in place of leaving it blank.

The third is usually a typo, like `expesnes:payroll-tax`. If the account is new on purpose, declare it with an `account` directive and the warning goes away.

## Related pages

- [Problems](problems.md)
- [The journal](index.md)
- [Settings](../settings/index.md)
