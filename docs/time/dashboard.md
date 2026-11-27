---
title: Time overview
description: "The time overview in Ledgeline Invoicer: total and billable hours, working days, hours over time, the account tree and top lists, each with the hledger command that reproduces it."
order: 30
tags: [invoicer, time]
---

# Time overview

The overview shows where your time went, whether you bill it or not. Open it with **Time** in the sidebar, or <kbd>g</kbd> <kbd>w</kbd>.

**Time** has two tabs: **Overview**, this page, and [Entries](entries-and-accounts.md#entries) (<kbd>g</kbd> <kbd>e</kbd>), the lines behind it. Both share one filter bar, and switching tabs keeps the filters.

Every figure links to the entries behind it, with the same filters. Every figure also has an info tip with the `hledger` command that prints the same number, so you can check it, or take it into a script.

## Filters

The bar at the top sets what every figure counts, and on Entries, which lines are listed:

| Filter       | Choices                                                                                          |
|--------------|--------------------------------------------------------------------------------------------------|
| **Period**   | This week, Last week, This month, Last month, This quarter, This year, Last year, Last 12 months, All time, or Custom… with a From and To date. ‹ and › step to the period before or after. |
| **Search**   | Text in the note or the account name.                                                            |
| **Person**   | Everyone, one person, or No one named (time with no person). Hidden when only one person has time. |
| **Customer** | Any customer, or one.                                                                            |
| **Billable** | Either, Billable, or Not billable.                                                               |
| **Billing**  | Any, Unbilled, On an invoice, Not billed (written off), Not billable, or Invoiced or written off. |
| **Account**  | An account and everything under it, such as `clients:acme`.                                      |

Each tab adds its own view options at the end of the period row. The overview's are:

| Option                     | Does                                                                                    |
|----------------------------|-----------------------------------------------------------------------------------------|
| **Compare with previous**  | On by default. Each figure shows its change against the period before, of the same length. Off for All time. |
| **Depth**                  | How many levels of the account tree to show.                                            |

All time runs from the first month with any time to today.

The filters are kept in the address. A link, a reload and Back all land on the same view. **Clear filters** resets them, and keeps the period. Switching tabs keeps the filters but not the other tab's view options.

| Keys                        | Does                         |
|-----------------------------|------------------------------|
| <kbd>[</kbd> / <kbd>]</kbd> | Previous / next period       |
| <kbd>/</kbd>                | Go to Search                 |

## The page

**The tiles** across the top:

- **Total hours**.
- **Billable hours**, and their share of the total.
- **Average per working day**, over the working days in the period, Monday to Friday.
- **Days tracked**: working days with any time, plus any weekend days.
- **Unbilled hours**: billable time for a customer that isn't on an invoice yet or written off. The tile appears once a billable account has a customer.

**Hours over time** is a stacked bar chart, one bar per day, week or month. The page picks the bar size from the period's length (days up to a month, weeks up to about six months, months beyond), and the **Day**, **Week** and **Month** buttons change it. **Split by** colours each bar by top-level account, customer, person, or billable. With a weekly target set (`weekly-target` in [Settings](formats.md#settings)), **Target** draws it as a line. **Table** under the chart lists the same numbers.

**The account tree** shows **Hours**, **Share** of the total, **Billable** hours and the **Change** at each level, rolled up from the levels below, to the depth in the filter bar. Unfold or fold any branch on its own, and **Reset to depth N** goes back. It lists only accounts with time in the period, so a large chart of accounts shows just what you worked on. A line under the table counts what it left out, including accounts that had time only in the previous period; their hours still count in their parents' **Change**.

**The top lists** show the ten biggest of each, the rest summed as **Other**, each with its change:

- **Top accounts**, at the filter's depth;
- **Top customers**;
- **Top people**, when more than one person has time;
- **Most frequent notes**.

## The same figures with hledger

The tips give commands like these. Run them from your books folder, where `time.journal` is. The period becomes `-p 2026-11` for a whole month, and `-b FROM -e DAY-AFTER` otherwise, since hledger's end date isn't included.

| Figure                         | Command                                                                                        |
|--------------------------------|------------------------------------------------------------------------------------------------|
| Total hours                    | `hledger -f time.journal bal -p 2026-11 --depth 0`                                              |
| Billable hours                 | `hledger -f time.journal bal -p 2026-11 --pivot billable`                                       |
| Average per day, days tracked  | `hledger -f time.journal bal -p 2026-11 -D --depth 0`                                           |
| Unbilled hours                 | `hledger -f time.journal bal -p 2026-11 --pivot billable '^yes$' tag:customer not:tag:invoice not:tag:nobill` |
| Hours over time, by top-level account | `hledger -f time.journal bal -p 2026-11 -W --depth 1`                                     |
| Hours over time, by customer   | `hledger -f time.journal bal -p 2026-11 -W --pivot customer`                                    |
| Account tree                   | `hledger -f time.journal bal -p 2026-11 --tree --depth 2`                                       |
| Top accounts                   | `hledger -f time.journal bal -p 2026-11 --depth 2 -S`                                           |
| Top customers                  | `hledger -f time.journal bal -p 2026-11 --pivot customer -S`                                    |
| Top people                     | `hledger -f time.journal bal -p 2026-11 --pivot person -S`                                      |
| Most frequent notes            | `hledger -f time.journal print -p 2026-11`                                                     |

The filters add query terms: `^clients:acme(:|$)` for an account, `'tag:customer=Acme Corp'` for a customer, `tag:person=pat` for a person, `tag:invoice` for time on an invoice and `tag:nobill` for time written off. The other billing states and Search have no single hledger query, so the command leaves them out, and its tip says so.

On the [sample time books](https://github.com/zmre/ledgeline/tree/main/invoicer/fixtures/time), the billable split for November reads:

```console
$ hledger -f time.journal bal -p 2026-11 --pivot billable
                2.17  no
               20.73  yes
--------------------
               22.90
```

### Where hledger's figure can differ

hledger and the overview agree to within a few hundredths of an hour. Where they can differ, the tip says so.

- **Rounding.** hledger 1.52 rounds every line to 0.01 hour, so a `2m` line counts 0.03 and two of them 0.06. The overview adds the exact minutes. [Formats](formats.md#lines-hledger-misreads) has more.
- **Billable, with a tree.** The billable filter becomes `--pivot billable` where it can, which gets the nearest parent's value right. Where the command already pivots on something else or needs account names, it falls back to `tag:billable=yes`, which also counts a `billable: no` account under a billable one. See [Billable time](hledger.md#billable-time).
- **People.** hledger knows a person only from a `person:` tag. The invoicer also knows the person from the folder a file is in (`time/sam/…`). A contractor's timesheet without `person:` tags counts in the overview's Top people but shows as no one in `--pivot person`.
- **Notes.** hledger can't group by a note, so the notes command lists every entry with its note instead.
