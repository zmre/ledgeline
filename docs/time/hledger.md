---
title: Time with hledger
description: "Reporting on Ledgeline Invoicer's time files with hledger directly: the account tree, billable time with --pivot, unbilled time, time by person and by customer, what went on an invoice, and time beside the books."
order: 60
tags: [invoicer, time]
---

# Time with hledger

Your time files are plain hledger, so every hledger report works on them. These recipes run from your books folder, on the roll-up, `time.journal`. The output shown is from the [sample time books](https://github.com/zmre/ledgeline/tree/main/invoicer/fixtures/time) in the repository.

hledger reads each line of time as a virtual posting with no commodity, in hours. Timeclock sessions come out in the commodity `h`, so a report that covers both shows two totals, such as `29.98` and `1.50h`.

## Where the time went

The account tree, rolled up at each level, for a month:

```console
$ hledger -f time.journal bal --tree -p 2026-11
               21.73  clients
                8.23    acme
                3.56      advisory
                3.67      tax-return
                1.00      travel
               13.50    globex:audit
                1.17  internal
                1.00    admin
                0.17    old
--------------------
               22.90
```

Add `--depth 1` for the top level alone, `-W` or `-M` for a column per week or month, and `-S` to sort by hours.

## Billable time

Use `--pivot billable`, which groups the time by each account's own `billable:` value, or its nearest parent's:

```console
$ hledger -f time.journal bal --pivot billable -p 2026-11
                2.17  no
               20.73  yes
--------------------
               22.90
```

> [!WARNING]
> Don't use `tag:billable=yes` for this. hledger gives an account every tag of its parents too, and a query matches any of them. With `billable: yes` on `clients` and `billable: no` on `clients:acme:travel`, travel has both, so `tag:billable=yes` counts it as billable. `billable: no` even matches a bare `tag:billable`.

```console
$ hledger -f time.journal bal tag:billable=yes -p 2026-11
                3.56  clients:acme:advisory
                3.67  clients:acme:tax-return
                1.00  clients:acme:travel
               13.50  clients:globex:audit
--------------------
               21.73
```

The travel hour shouldn't be there. To keep the account names and still leave out an override, also exclude the `no`:

```sh
hledger -f time.journal bal tag:billable=yes not:tag:billable=no -p 2026-11
```

That works while no account says `billable: yes` under a parent that says `no`. The invoicer itself always takes the nearest value, and its Accounts page shows where each account's value comes from.

## Unbilled time

Billable time for a customer, not on an invoice and not written off:

```console
$ hledger -f time.journal bal -p 2026-11 tag:billable=yes not:tag:billable=no tag:customer not:tag:invoice not:tag:nobill
                3.56  clients:acme:advisory
                3.67  clients:acme:tax-return
               13.50  clients:globex:audit
--------------------
               20.73
```

For one customer, as the roll-up's header suggests, list the lines with `reg`. `tag:customer=` takes a regular expression, so `Acme` matches `Acme Corp`:

```sh
hledger -f time.journal reg not:tag:invoice tag:customer=Acme -p 2026-11
```

## By customer and by person

```sh
hledger -f time.journal bal --pivot customer -S -p 2026-11
hledger -f time.journal bal --pivot person -p 2026-11
hledger -f time.journal reg tag:person=pat -p 2026-11
```

hledger knows a person only from a `person:` tag. The tracker writes one on every date line, but a contractor's own timesheet may have none. The invoicer knows whose it is from its folder (`time/sam/…`), and hledger shows it as no one:

```console
$ hledger -f time.journal bal --pivot person -p 2026-11
               14.50
                8.40  pat
--------------------
               22.90
```

To fix that for hledger, use **Set person…** on those lines in [Entries](entries-and-accounts.md#changing-many-lines-at-once), which writes the tag.

## What went on an invoice

Billed time carries the same `invoice:` tag as the invoice's booking in your books. Read both files and one query shows the invoice and the hours behind it:

```console
$ hledger -f books.journal -f time.journal reg tag:invoice=101
2026-10-06 Tue                  (cl:acme:tax-return)          3.00          3.00
2026-10-20 Tue                  (cl:acme:tax-return)          2.00          5.00
2026-10-31 Acme Corp | Invoi..  ..ccounts receivable     $1,000.00          5.00
                                                                       $1,000.00
                                revenue:tax             $-1,000.00          5.00
```

A contractor's bill works the same way with its reference: `reg 'tag:bill=sam timesheet 2026-11'`.

## Time beside the books

Your main journal doesn't include the time on purpose. If you'd rather it did, add `include time.journal` to it. Ledgeline keeps the hours out of its money reports either way, but hledger shows them in every report.

Since every line of time is a virtual posting, `--real` (`-R`) leaves it out, and `real:0` keeps only it:

```sh
hledger -f books.journal bal -R            # money only
hledger -f books.journal bal real:0         # hours only
```

Without the include, give hledger both files when you want both: `hledger -f books.journal -f time.journal …`.

## Strict mode

`hledger -s` checks that every account and commodity is declared. The roll-up declares `commodity ""` for timedot hours, and `time-accounts.journal` declares the accounts. Two things can still fail the check:

- **Timeclock files.** Their hours are in the commodity `h`. Setup declares it when the tracker writes timeclock, and the invoicer adds `commodity h` when it first writes a timeclock file the roll-up doesn't include yet. A roll-up of your own may not declare it: `ledgeline-invoicer time setup --adopt FILE` lists the `commodity` lines it lacks.
- **An account that has time but isn't declared.** Declare it on the [Accounts](entries-and-accounts.md#undeclared-accounts) page, or add an `account` line by hand.

```sh
hledger -f time.journal -s check
```
