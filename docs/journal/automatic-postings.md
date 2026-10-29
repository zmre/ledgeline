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
- An amountless posting balances the rule's other postings at cost. A rule matching `1 EUR @ $1.10` balances with `$-1.10`, not `-1 EUR`.
- A `tag:` query also matches tags a posting inherits from its account's `account` directive.
- [Balance assertions](problems.md#balance-assertion) are checked after the rules apply, as in hledger. An assertion written on a rule's own posting, such as `assets:savings  $10 = $500`, is checked on every posting the rule adds, and a failure points at the rule's line.
- In the [budget](../budget.md), rules add to the actual amounts and never to the goals.
- [Subscription detection](../reports/subscriptions.md) ignores the postings rules add, so a tax rule doesn't make a new subscription.

Applying the rules never changes your journal files. The added postings exist only while Ledgeline reads the journal. The rules themselves are text in your journal, and you can edit them in [Settings › Automatic postings](#editing-a-rule).

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

The tab also says whether `--auto` is on, and which config file says so. Change that on the [hledger config](../settings/hledger-config.md) tab or in the file.

### Editing a rule

**Edit** on a rule's card opens it in a form: the account it matches, the postings it adds, and the account that balances them. Type percentages and amounts as positive numbers meaning "this account grows by". Ledgeline works out each sign from the accounts' types, the same way the [projections rule editor](../projections/index.md) does. For example, 10% sales tax on a sale (a negative revenue amount) is written `*0.1` to the liability. A form edit is saved in a standard shape:

```journal
= ^revenue:sales(:|$)
    liabilities:tax:sales  *0.08
    assets:checking        *-0.08
```

The `^…(:|$)` query matches the account and its subaccounts, but not a sibling such as `revenue:sales2`. The balancing posting gets an explicit amount, because hledger refuses a blank one that fires twice in one transaction.

Numbers are written the way your journal reads them. In a journal with `decimal-mark ,`, or a comma-style `D` or `commodity` directive, 8% is written `*0,08` and a fixed amount `1500,50 EUR`. Written with a `.`, hledger would read those as 800% and 150050 EUR.

Leave **forecasts only** unticked for a rule about your recorded transactions. Ticked, the query gains `tag:_generated-transaction`, so hledger applies the rule only to `--forecast` transactions. Ledgeline doesn't forecast your main journal, so such a rule never adds anything here.

Some rules can't be shown as a form: a `tag:` or `desc:` query with no account, virtual postings, posting comments, balance assertions. These open as text instead, and so does **Edit as text** on any rule. What you type is written exactly as typed.

**+ Add a rule** appends a new rule to the end of your main journal file. hledger applies a rule to every transaction, wherever the rule sits in the file. **Delete** asks once more, then removes the rule and the blank line above it.

Each save rewrites only that rule's lines. The rest of the file, including your comments and alignment, is left byte for byte as it was. Nothing is written if the file changed on disk since the list was read, or if the journal would no longer read. A rule in a file Ledgeline can't write, such as a symlink, has no Edit button.

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
