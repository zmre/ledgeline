---
title: Entries and accounts
description: "Correcting and adding time in Ledgeline Invoicer: the Entries page by day, week or account, bulk changes and locked lines; and the Accounts page, which links time accounts to customers, marks them billable, and renames or archives them. With every time tag."
order: 20
tags: [invoicer, time]
---

# Entries and accounts

Track is for the clock. **Entries** is for everything else you do with time: adding an hour you forgot, fixing a wrong account, writing a day off. **Accounts** decides what each time account means for billing.

## Entries

**Time → Entries** (<kbd>g</kbd> <kbd>e</kbd>) lists every line of time in the period, from every file the roll-up includes, whoever wrote it. Three buttons choose the view:

| View           | Shows                                                                                       |
|----------------|---------------------------------------------------------------------------------------------|
| **By day**     | A timesheet: one heading per day with its total, the lines under it. The default.            |
| **Week**       | A grid of accounts by days, Monday to Sunday, with row, day and week totals.                |
| **By account** | One heading per account with its total.                                                     |

The columns are **Date**, **Person**, **Account**, **Hours**, **Note**, **Billing** and **File**. Billing says **Unbilled**, **Not billable**, **Invoice 108** (a link to it), **Bill …** or **Not billed**, with the reason if one was given. File gives the file and line the entry is on, so you can find it in an editor. Person and File drop out in a narrow window.

The filters above the list narrow it by period, person, customer, billable, and billing state, and **Search** matches notes and account names. They're kept in the address, so the [overview](dashboard.md) can link straight to a filtered list, and Back returns to the previous filter.

### Adding time

Press <kbd>a</kbd> or **Add time**. The row asks for a date (today unless you change it), an account, a duration and an optional note. The duration takes any of these:

| You type | Means           |
|----------|-----------------|
| `45m`    | 45 minutes      |
| `2h`     | 2 hours         |
| `1h30`, `1h 30m` | 1 hour 30 minutes |
| `1.5`    | 1.5 hours       |
| `0:45`   | 45 minutes      |
| `90s`    | 90 seconds      |

The line goes under its date in the month's tracker file, which is created if it's new. With the Person filter set, it goes in that person's file. Otherwise it goes in yours.

### Changing and deleting

**Edit** on a line, or <kbd>Enter</kbd> on the line under the cursor, opens the same fields in place. Save rewrites only that line. You can edit lines you wrote by hand in any timedot file, not only the tracker's. A timeclock session changes by its start and end.

**Delete** takes the line out of its file, after you confirm.

A line on a sent invoice, or on a contractor's bill, is **read-only**. Its Billing cell has a **read-only** badge, and Edit explains why. For an invoice it links to the invoice: void it to change the line (see [Releasing time](billing.md#releasing-time)). For a bill it offers **Void bill and release time…** (see [Voiding a bill](billing.md#voiding-a-bill)). A line on a draft invoice can still be changed.

### Changing many lines at once

Tick the box at the start of each line, or press <kbd>x</kbd> on the line under the cursor. <kbd>Shift</kbd>+click ticks a range. A bar shows how many lines and hours you picked, with these actions:

| Action                  | Does                                                                                         |
|-------------------------|----------------------------------------------------------------------------------------------|
| **Mark billable**       | Takes the `nobill:` tag off, so written-off time counts as unbilled again.                   |
| **Mark not billable…**  | Adds `nobill:`, with a reason if you give one. The time leaves To bill.                      |
| **Move to account…**    | Moves the lines to another account.                                                          |
| **Set person…**         | Sets who did the work. Leave it empty for no one.                                            |
| **Bill…**               | Bills the picked lines on an invoice draft. They must be one customer's unbilled time. See [Billing time](billing.md#billing-a-customer). <kbd>b</kbd> does the same. |
| **Clear**               | Unticks them all. <kbd>Esc</kbd> does the same.                                              |

### The weekly grid

In **Week**, every cell is a field. Type the day's new total for that account (`2h`, `1h30`, `0:45`) and press <kbd>Enter</kbd> or leave the cell. More time goes onto the day's open line, or a new one. Less comes off the open line.

Billed and clocked time is never changed from the grid. A cell holding some is marked, and a cut that would need it is refused, with the reason. To add a row for an account with no time that week yet, pick it under the grid. ← and → step through the weeks, and **This week** comes back.

### Warnings

Above the list, Entries warns about:

- anyone with more than 24 hours on one day;
- lines hledger reads differently from the invoicer, such as an amount with one space before it, which hledger reads as zero. [Formats](formats.md#lines-hledger-misreads) lists them.

### Keys

| Keys                        | Does                                       |
|-----------------------------|--------------------------------------------|
| <kbd>a</kbd>                | Add time                                   |
| <kbd>j</kbd> / <kbd>k</kbd> | Move the cursor down / up                  |
| <kbd>Enter</kbd>            | Edit the line under the cursor             |
| <kbd>x</kbd>                | Tick the line under the cursor             |
| <kbd>b</kbd>                | Bill the ticked lines                      |
| <kbd>Esc</kbd>              | Untick every line                          |
| <kbd>/</kbd>                | Search                                     |

## Accounts

**Time accounts**, in the sidebar's Library (<kbd>g</kbd> <kbd>a</kbd>), shows `time-accounts.journal` as a tree. Its columns are **Account**, **Customer**, **Billable**, **Item**, **This month** and **Unbilled**. A value an account inherits from a parent says where it comes from, such as "from clients:acme". <kbd>[</kbd> and <kbd>]</kbd> step the month the hours are for.

Time accounts don't have to sit under one root. Any account that has time in a file the roll-up includes is a time account, so a tree of your own such as `projects:…` or `kunden:…` works as it is.

### Changing an account

**Edit**, or <kbd>Enter</kbd>, opens the account's settings:

- **Customer** links the account and everything under it to an Invoicer customer. Start typing to pick one, or choose **New customer "…"…** to create it. Leave it empty to use the parent's.
- **Billable**: **Yes**, **No**, or not set, which follows the parent. With nothing set anywhere above, the account isn't billable.
- **Default item** is the catalog item its time bills as: the rate and the revenue account. **Billing time** suggests it.
- **Archived (hidden from pickers)** keeps the account out of Track's search and slots. Its past time still reports.

Save writes the tags onto the account's line in `time-accounts.journal`. If an account names a customer with no Invoicer customer note, the row shows **Create customer**.

**Add under** (<kbd>a</kbd>) adds a child account, and **Add account** (<kbd>A</kbd>) adds a top-level one.

**Rename** (<kbd>r</kbd>) renames the account and every account under it. It shows each line it will change, in the accounts file and in every time file, as **Where**, **Before** and **After**, and changes nothing until you confirm. Billed lines keep their `invoice:` and `bill:` tags.

### Undeclared accounts

An account that has time but isn't declared in `time-accounts.journal` appears under **Not declared yet**, and in the tree with an **undeclared** badge. **Declare** adds it to the file, so you can give it a customer or make it billable.

### Keys

| Keys                        | Does                                          |
|-----------------------------|-----------------------------------------------|
| <kbd>j</kbd> / <kbd>k</kbd> | Move the cursor down / up                     |
| <kbd>→</kbd>                | Unfold the account, or step into it            |
| <kbd>←</kbd>                | Fold it, or step out to its parent             |
| <kbd>Enter</kbd>            | Edit the account                              |
| <kbd>a</kbd>                | Add an account under this one                 |
| <kbd>A</kbd>                | Add a top-level account                       |
| <kbd>r</kbd>                | Rename the account                            |
| <kbd>[</kbd> / <kbd>]</kbd> | Previous / next month                         |

## The accounts file

`time-accounts.journal` is ordinary hledger account directives with tags. hledger ignores an indented `account` line, so the tree is laid out by order and blank lines instead: one block per top-level account, parents before children, the tags lined up in one column.

```journal
; Time accounts. Tags: customer (links to an Invoicer customer), billable
; (yes/no, nearest ancestor wins), item (catalog code to bill as),
; archived (hide from pickers). Edit freely.

account clients                                   ; billable: yes
account clients:acme                              ; customer: Acme Corp, item: ARCH
account clients:acme:advisory
account clients:acme:tax-return                   ; item: TAX
account clients:acme:travel                       ; billable: no

account clients:globex                            ; customer: Globex, item: AUD
account clients:globex:audit

account internal                                  ; billable: no
account internal:admin
account internal:old                              ; archived: yes
```

Here, `clients:acme:tax-return` bills Acme Corp as item `TAX`, `clients:acme:advisory` bills Acme as `ARCH`, and travel to Acme isn't billable.

An account's tag comes from the account itself if it has one, else from its nearest parent that does. So the `billable: no` on travel beats the `billable: yes` on `clients`. hledger's `tag:billable=yes` query doesn't work that way. [Time with hledger](hledger.md#billable-time) explains the difference.

## The tags

| Tag        | Goes on                 | Value                                   | Means                                                                                      |
|------------|-------------------------|-----------------------------------------|--------------------------------------------------------------------------------------------|
| `customer` | a time account          | the customer's name in the books        | Its time, and its children's, bills this Invoicer customer.                                 |
| `billable` | a time account          | `yes` or `no`                           | Whether its time is billable. `no` when nothing in the chain says.                          |
| `item`     | a time account          | a catalog code                          | The item its time bills as, suggested when you bill.                                       |
| `archived` | a time account          | `yes`                                   | Hidden from Track's search and slots.                                                       |
| `person`   | a date line, or a line  | a short name                            | Who did the work. A line's own tag beats its date line's.                                   |
| `invoice`  | a line                  | an invoice number                       | Billed on that invoice. The same tag as in the books, so one query finds both sides.         |
| `bill`     | a line                  | a bill reference                        | Paid to the person who did it, through that accounts payable bill.                         |
| `nobill`   | a line                  | a reason, or nothing                    | Written off: never to be billed.                                                           |

The customer's name is the one the books use: the customer note's `ledger_customer`, or its title when it has none. [Customer files](../invoicer/files.md#a-customer) explains it.

Any other text in a line's comment, beside its tags, is the line's **note**.

Each tag name can be changed for your books, for example to write `rechnung:` instead of `invoice:`. See [Formats](formats.md#settings).
