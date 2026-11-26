---
title: Dashboard
description: "The Ledgeline Invoicer dashboard: drafts, what is outstanding and what was paid recently, the aging of what you are owed, recently updated invoices, days sales outstanding, average days to pay, and the settings that change them."
order: 40
tags: [invoicer]
---

# Dashboard

The dashboard is the app's home page. It answers three questions: what is waiting to be sent, what you are owed, and what came in recently. Every figure on it is a link to the invoices behind it.

What you are owed comes from your books, the same figures Ledgeline shows. In accrual books that means every receivable still open, including ones booked without an invoice file, such as an opening balance. In cash books it means sent invoices less the payments applied to them.

## Getting paid

The top row follows money through three steps:

| Card        | Shows                                                                 | Opens                                          |
|-------------|-----------------------------------------------------------------------|------------------------------------------------|
| Drafts      | The total of your drafts, how many, and the date of the oldest.       | the invoice list, drafts only                  |
| Outstanding | Everything still owed, how many items, and how much of it is overdue. | the invoice list, open invoices only           |
| Paid        | Money received in the period, and how many invoices it paid into.     | the invoice list, invoices paid in that period |

The **Paid in the** menu picks the period: the last 7, 30, 90 or 365 days, plus the number of days in your Paid window [setting](settings.md#dashboard) if it is another. The period ends today and counts both ends, so the last 30 days on 22 November starts on 24 October. Your choice is kept in the page's address, so Back returns to the previous one.

"Paid" counts money received, on the day it was received:

- In accrual books, every payment linked to a receivable, including receivables with no invoice file. Those are counted separately and noted under the card.
- In cash books, the payments applied to invoices, before any fee booked separately. A $2,910 payout with a $90 card fee counts as $3,000.
- A partial payment counts its invoice, which is still open.
- Credits, write-offs and voids are not money received.

## Outstanding

The outstanding panel draws what you are owed as one bar on a scale: the overdue part in red, the part not yet due in yellow. The scale runs to the total, or to at least 10,000 in your currency, so a small balance looks small. Hover over a part of the bar, or Tab to it, to see its amount, its count and its share. <kbd>Esc</kbd> hides the tooltip.

The large figure beside the bar is the total outstanding. Next to it, the aging list breaks the total down by how late it is:

| Row     | Holds                          |
|---------|--------------------------------|
| Current | not yet due                    |
| 1–30    | 1 to 30 days past the due date |
| 31–60   | 31 to 60 days past due         |
| 61–90   | 61 to 90 days past due         |
| 91+     | 91 days or more past due       |

Each row opens the invoices in it. The Aging ranges [setting](settings.md#dashboard) changes the ranges.

If some of what is owed was booked without an invoice file in these books, a line under the bar says how much and how many items. The invoice list can only show invoices that have files, so those items appear on the dashboard but not in the list.

With more than one currency, each gets its own panel.

## Recently updated

Six cards for the invoices that changed most recently, newest first. Each card shows the number, the customer, what happened last and when ("Paid 2026-11-05"), the amount, and a coloured band with the status in words: paid, open, partly paid, overdue, draft or void. A void invoice's amount is struck through.

"What happened last" is the latest of: the invoice's date, the last time its file was saved, the day it was sent, the last payment, and the day it was voided. Nothing after today counts.

## The tiles

| Tile                | Shows                                                                                                                                                                             |
|---------------------|-----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| Oldest overdue      | How many days the oldest overdue item is past due, with its number, customer, amount and due date. It opens that invoice, or the customer's overdue invoices when it has no file. |
| DSO                 | Days sales outstanding: what is owed today divided by what you billed over the last 90 days, times 90. Voids count in neither. The `dso-days` setting changes the 90.             |
| Average days to pay | The average number of days from an invoice's date to the payment that completed it, over every invoice ever paid in full. Voids are left out.                                     |
| Recognition due     | Shown when your books have a recognition schedule: how many scheduled revenue and expense entries are due through today, and how many more fall in a closed period.               |

**Open in Ledgeline** on the recognition tile opens these books in Ledgeline, where **Journal → Recognition** posts the entries. The button needs Ledgeline installed and is only on macOS. Elsewhere the tile tells you where to go. See [Revenue and expense recognition](../journal/accrual-recognition.md).

## Settings

Change these in **Settings → These books → Dashboard**, then Save. See [Settings](settings.md#dashboard). They are kept under `[invoicer]` in the books' `ledgeline.toml`:

| Setting            | Default        | Changes                                                                                                                                                                 |
|--------------------|----------------|-------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `aging-buckets`    | `[30, 60, 90]` | The aging ranges. Each number is the last day of a range after Current. `[15, 45]` gives Current, 1–15, 16–45 and 46+. One to eight numbers, each larger than the last. |
| `dso-days`         | `90`           | The number of days DSO looks back over.                                                                                                                                 |
| `paid-window-days` | `30`           | The period the Paid card starts on, 1 to 3660 days. It is also added to the menu's choices.                                                                             |

```toml
[invoicer]
aging-buckets = [15, 30, 60]
dso-days = 30
paid-window-days = 90
```

The aging ranges apply to the dashboard only. `ledgeline-invoicer receivables` and Ledgeline keep the fixed ranges in the table above.

## Keys

| Keys                                   | Does                                                             |
|----------------------------------------|------------------------------------------------------------------|
| <kbd>1</kbd> <kbd>2</kbd> <kbd>3</kbd> | Open Drafts, Outstanding, Paid                                   |
| <kbd>w</kbd> / <kbd>W</kbd>            | A longer / shorter paid period                                   |
| <kbd>j</kbd> / <kbd>k</kbd>            | Next / previous recently updated card; <kbd>Enter</kbd> opens it |

All the shortcuts are on [Keyboard shortcuts](keyboard.md).

## The same figures from the command line

`ledgeline-invoicer receivables --aging` prints the open items, the aging, DSO and average days to pay. Add `--json` for a script. See [Command line](cli.md#receivables).
