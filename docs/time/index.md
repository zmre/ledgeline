---
title: Time tracking
description: "Time tracking in Ledgeline Invoicer: hours kept in plain hledger timedot and timeclock files beside your books, a ten-slot tracker, a dashboard, and billing time to invoices or paying contractors for it."
order: 85
tags: [invoicer, time]
---

# Time tracking

Ledgeline Invoicer can track the hours you work and bill them. The hours go into plain-text [timedot](https://hledger.org/dev/hledger.html#timedot) and [timeclock](https://hledger.org/dev/hledger.html#timeclock) files beside your books, in the formats hledger reads. You can edit them in any text editor, keep them in git, and report on them with `hledger` without the app.

Time is not money, so the hours stay out of your journal. They sit under their own tree of time accounts, such as `clients:acme:tax-return` and `internal:admin`. Tags on those accounts link them to your Invoicer customers and say whether their time is billable. Tags on each entry record which invoice it went on.

## Where it lives

Time tracking is the **Time** group in the Invoicer's sidebar, with four pages:

| Page      | What it is for                                                                                                     | Keys                      |
|-----------|--------------------------------------------------------------------------------------------------------------------|---------------------------|
| Dashboard | Where your time went: totals, billable share, hours over time, the account tree and top lists. See [Dashboard](dashboard.md). | <kbd>g</kbd> <kbd>w</kbd> |
| Entries   | Every line of time, by day, by week or by account, to add, change and correct. See [Entries and accounts](entries-and-accounts.md). | <kbd>g</kbd> <kbd>e</kbd> |
| Track     | Ten slots, one key each, to start and stop the clock. See [Tracking time](tracking.md).                          | <kbd>g</kbd> <kbd>r</kbd> |
| Accounts  | The time account tree: which customer each bills, whether it's billable, what it bills as. See [Entries and accounts](entries-and-accounts.md#accounts). | <kbd>g</kbd> <kbd>a</kbd> |

Track is <kbd>g</kbd> <kbd>r</kbd> (r for recording) because <kbd>g</kbd> <kbd>t</kbd> already goes to Templates.

Above the sidebar's links, on every screen, a light shows what's being timed: a green ▶ with the account and the running time, or a grey ■ with the last account and the key that resumes it. Click it to stop or resume.

## Setting up

The first time you open Track, it offers to set up time tracking. Nothing is written until you press a button.

**Start fresh** creates three files beside your main journal. It asks for one thing first, your name in the time files:

- It suggests a short name from this computer's account: the first word of your full name in lower case, or your login name if there's no full name.
- Change it, or clear it. With no name, your files go straight into `time/`, and the date lines carry no `person:` tag. If you're the only one tracking time in these books, you don't need a name.
- The screen lists the files it will create, and **Show what it will hold** shows each one's text before you press **Set up**.

Setup also seeds the accounts file with a `clients:<customer>` account for each of your Invoicer customers, tagged with the customer and `billable: yes`, and an `internal:admin` account tagged `billable: no`. Rename or delete them afterwards.

**Use existing…** is for time files you already keep. It lists the `.timedot` and `.timeclock` files it finds under your books folder, or takes a path you type. Point it at a roll-up journal of your own, or at a single time file, which then counts as the whole time book. It creates nothing else. **Check** says whether the file can be used, and **Use this file** records it as `file` under `[time]` in your books' `ledgeline.toml`.

> [!NOTE]
> The invoicer doesn't create a `ledgeline.toml` for you. If your books don't have one, **Use existing…** shows the two lines to put in a new `ledgeline.toml` beside your journal. Adopting a file named `time.journal`, the default, needs no setting at all.

You can change your name later at the foot of the Track page (**Change**). That only changes what new entries are written with. See [Settings](formats.md#settings) for where it's kept.

## The files

A fresh setup gives you this layout:

```text
books/
├── books.journal               your main journal: doesn't include the time files
├── time.journal                the roll-up: directives and includes
├── time-accounts.journal       the time account tree, with its tags
└── time/
    ├── pat/
    │   ├── 2026-10.timedot     your tracker files, one a month
    │   └── 2026-11.timedot
    └── sam/
        └── 2026-11.timedot     a contractor's timesheet
```

**`time.journal`** is the roll-up. It's written once at setup and then left to you:

```journal
; Time tracking roll-up. Not included by books.journal on purpose:
; hours are not money. To report on time:
;   hledger -f time.journal bal --tree -p thismonth
;   hledger -f time.journal bal --pivot billable -p lastmonth
;   hledger -f time.journal reg not:tag:invoice tag:customer=Acme
; To see time beside the books, include this file from books.journal.

commodity ""                      ; time has no commodity (needed for -s)

include time-accounts.journal
include time/**/*.timedot
include time/**/*.timeclock
```

Whatever the roll-up includes is your time book. The invoicer doesn't look for files anywhere else, so a time file in another folder counts once the roll-up includes it.

**`time-accounts.journal`** declares the time accounts and their tags. [Entries and accounts](entries-and-accounts.md#accounts) explains the tags.

**The tracker files** are where Track and Entries write your time, one file a month: `time/<your name>/YYYY-MM.timedot`, or `time/YYYY-MM.timedot` with no name. [Formats](formats.md) shows what's in them and how to change the names and places.

The invoicer follows changes to all of these files. Edit one by hand and every time page updates.

## Why your journal doesn't include it

Your main journal doesn't include `time.journal`, for two reasons. Hours aren't money, so they don't belong in the balance sheet. And hledger turns each line of time into a virtual posting with no commodity, which would sit in every `hledger bal` you run on the books.

If you want time beside the books, include the roll-up from your journal, as the hledger manual shows:

```journal
include time.journal
```

Ledgeline reads the time files then, but keeps their hours apart from your transactions, so its Journal, reports, Budget and Insights show only money. `hledger` itself mixes them; [Time with hledger](hledger.md#time-beside-the-books) shows how to keep them apart there.

## In this section

- [Tracking time](tracking.md): the Track page, the slots, the light, keys on each platform, and the `time` command line for scripts and launchers.
- [Entries and accounts](entries-and-accounts.md): adding and correcting time, the weekly grid, and managing the account tree and its tags.
- [Dashboard](dashboard.md): where your time went, with the hledger command behind every figure.
- [Billing time](billing.md): unbilled time, billing it on an invoice draft, rounding, and paying contractors for theirs.
- [Formats](formats.md): timedot and timeclock as the invoicer writes and reads them, hledger's pitfalls, and the settings for names, places and languages.
- [Time with hledger](hledger.md): reporting on time with `hledger` directly.
