---
title: Billing time
description: "Billing tracked time in Ledgeline Invoicer: the To bill page and its suggested periods, billing time onto an invoice draft with rates, rounding and a timesheet table, releasing time when a draft is deleted or an invoice voided, and paying contractors for their time through accounts payable."
order: 40
tags: [invoicer, time, accrual]
---

# Billing time

Time on a billable account linked to a customer is **unbilled** until it goes on an invoice or is written off. This page covers turning it into an invoice draft, and paying a contractor for their time through accounts payable.

Billing only ever writes two things: a draft invoice, the same as one you write yourself, and an `invoice:` tag on each line of time it bills. Nothing reaches your journal until you send the invoice, as with any draft. See [Booking](../invoicer/booking.md).

## Unbilled time

A line of time is unbilled when all of these hold:

- its account is billable (`billable: yes` on it or its nearest parent that says);
- its account has a customer (`customer:` on it or a parent);
- it has no `invoice:` tag and no `nobill:` tag.

**To bill**, in the sidebar (<kbd>g</kbd> <kbd>u</kbd>), lists the time waiting to be billed or paid. Its sidebar link shows how many customers have unbilled time. The page has two tabs, each showing how many it lists: **To bill**, for your customers, and **To pay** (`/to-bill?tab=pay`), for your contractors. <kbd>1</kbd> and <kbd>2</kbd> switch between them. The **To bill** tab has these parts:

| Part | Shows |
|------|-------|
| The banner | A sum-up, such as *3 customers have unbilled time through October: Acme Corp 50m · Globex 3h · Initech Inc 40m*, with **Bill all…**. |
| A card per customer | Its unbilled hours, an estimate of what they'd bill, how often it's billed, a **Bill…** button for each suggested period, and **Everything unbilled** with its own **Bill…**. **See the entries** opens them in Entries. |
| **Needs a customer** | Billable time on an account that isn't linked to a customer, with **Link a customer on Accounts**. |
| **On an invoice that no longer exists** | Lines whose `invoice:` tag names an invoice that's in neither the files nor the books, with **Release** to make them unbilled again. |

The **To pay** tab lists each timekeeper's unpaid time, with **Create bill…**. See [Paying a contractor](#paying-a-contractor).

The page used to be **Time → Unbilled**. Its old address, `/time/unbilled`, still opens it.

How a customer's periods are picked depends on its note's `cadence:`:

| `cadence:`               | Suggested period                                                                                 |
|--------------------------|--------------------------------------------------------------------------------------------------|
| `monthly`                | The last complete month with unbilled time. Older months with unbilled time are their own suggestions. |
| `weekly`, `quarterly`    | The same, by week or quarter.                                                                    |
| none                     | Everything up to the end of last month.                                                          |

The period you're in now isn't suggested until it's over, so a month's time is billed once.

**Bill all…** (<kbd>Shift</kbd>+<kbd>B</kbd>, from either tab) opens the Bill dialog for each customer's main suggestion in turn, marked *1 of 3* and so on. **Bill time…** in the sidebar's **New** menu, there once time tracking is set up, opens To bill and starts it. **Skip** moves on to the next customer, and Cancel stops. At the end, the page lists the drafts it made, each a link to its editor.

## Billing a customer

**Bill…** on a customer's period opens the Bill dialog. You can also bill exactly the lines you pick in [Entries](entries-and-accounts.md#changing-many-lines-at-once): tick them and press **Bill…** or <kbd>b</kbd>. They must all be one customer's unbilled time.

Every change in the dialog updates its preview. <kbd>Cmd</kbd>+<kbd>Enter</kbd> (<kbd>Ctrl</kbd>+<kbd>Enter</kbd> on Linux and Windows) creates the draft from any field, and <kbd>Esc</kbd> cancels. Enter in a field doesn't create it.

**1. Rows.** Each group of time becomes one invoice row. Choose **By account** (the default), **By account and person**, or **By account and note**. Group by person when people bill at different rates, such as a senior and a junior on the same account.

**2. Item and rate.** Each row gets a catalog item, which gives its rate, unit and revenue account. The suggestion comes from the first of these that has one:

1. a `time_items` rule in the customer note that matches the account and person;
2. the account's `item:` tag, or its nearest parent's;
3. the item this customer is billed most often.

Each row says where its suggestion came from, such as *Set on the time account*. You can change its **Item**, **Rate**, **Books account** and **Description**, or tick **Leave off this bill**, and its time stays unbilled. **Remember for this account** writes `item:` onto the account in `time-accounts.journal`. Grouped by account and person, it reads **Remember for this person** and adds a `time_items` rule to the customer note.

**3. Rounding.** The dialog shows each row's hours as tracked, as billed, and the difference, and the totals: **Tracked**, **Billed** and **Rounding**. Rounding is set per customer (see [Rounding](#rounding)) and is never written into your time files.

**4. Timesheet.** Below the bill table, the invoice can carry a timesheet table of the work. **Timesheet under the bill** starts from the customer note's `time_detail:`, and the dialog shows the table as the invoice will:

| `time_detail:` | In the dialog                  | Adds                                     |
|----------------|--------------------------------|------------------------------------------|
| `daily`        | **A row per day and person**   | One row per day and person. The default.  |
| `entries`      | **A row per entry**            | One row per line of time.                 |
| `none`         | **No timesheet**               | Nothing.                                  |

The columns are **Date**, **Who**, **Work** and **Hours**. The notes on your time fill *Work*, and the hours are the actual hours, before rounding.

**5. The invoice.** **Date**, **Number**, **Terms (days)** and **Reference** default to the usual: today, the next number, the customer's terms, and none.

**6. Create draft.** This writes the draft and tags every line it bills with `invoice:` and the new invoice's number. The time leaves To bill at once, and the draft opens in its editor. Change it as you like, and send it when it's ready.

A November bill for the sample customer Acme Corp, grouped by account, makes this draft:

```markdown
| Code | Description          | Qty  | Unit | Rate          | Amount        | *Account*          |
|:-----|:---------------------|-----:|:-----|--------------:|--------------:|:-------------------|
| ARCH | Advisory, Nov 2026   | 4.10 | h    | $250          | $1,025.00     | revenue:consulting |
| TAX  | Tax return, Nov 2026 | 4.40 | h    | $200          | $880.00       | revenue:tax        |
|      |                      |      |      | **Total due** | **$1,905.00** |                    |

## Timesheet

| Date | Who | Work | Hours |
|:-----|:----|:-----|------:|
| 2026-11-02 | pat | gathered documents; email | 1.53 |
| 2026-11-03 | pat | email | 0.03 |
| 2026-11-04 | sam | second opinion on the K-1 | 2.50 |
| 2026-11-25 | pat | reviewed schedules C and E | 2.17 |
| 2026-11-26 | pat | call with Fred about the K-1; Tax return | 2.17 |
```

Each row's description is the account's last part and the period. The billed lines in the time file gain the tag after their note:

```timedot
2026-11-02 Mon  ; person: pat
    clients:acme:tax-return          90m  ; gathered documents, invoice: 102
    clients:acme:advisory             2m  ; email, invoice: 102
    internal:admin                   15m
```

The tags are written before the draft. If the draft can't be written, they're taken off again.

## Rounding

Set rounding in the customer's note. The defaults round nothing.

| Key                | Values                                  | Default | Means                                                       |
|--------------------|-----------------------------------------|---------|-------------------------------------------------------------|
| `time_increment`   | minutes, such as `6` or `15`            | none    | Round to this many minutes.                                  |
| `time_rounding`    | `up` or `nearest`                       | `up`    | Up to the next increment, or to the nearest one, half up.    |
| `time_round_scope` | `row`, `day` or `entry`                 | `row`   | What's rounded: each invoice row's total, each account's day within a row, or each line. |

```yaml
---
title: Acme Corp
type: organization
tags: [customer]
cadence: monthly
time_detail: daily
time_increment: 6
time_rounding: up
time_items:
  - account: clients:acme
    person: sam
    item: JR
---
```

Rounding each row is the default because rounding each small line adds up. Three two-minute emails rounded up to six minutes each bill 0.3 hours. Summed first, they bill 0.1. The American Bar Association's Formal Opinion 93-379 on billing makes the same point: bill the time spent, rounded only to a stated minimum increment.

The invoice row's quantity is the rounded hours. Your time files keep the actual minutes, and the timesheet table shows them.

## Rates

The rate comes from the catalog item, and is fixed on the invoice row once the draft exists. Changing the catalog later doesn't change invoices already written, as with any invoice.

For different rates per person on one account, group by account and person and give each row its own item. The `time_items` rule in the example above bills Sam's time on Acme as `JR`, a junior rate, wherever the bill is grouped by person. A rule's `account` covers everything under it.

## Releasing time

Time on an invoice stays tagged as long as the invoice needs it:

- **Deleting a draft** takes its `invoice:` tags off. **Delete draft…** in the invoice editor says how much, such as *Its 8.4 hours of time go back to To bill.*
- **Voiding a sent invoice** asks what happens to its time. **Release it: back to To bill, to bill again** is the default. **Mark it not billed (written off)** replaces each `invoice: 108` with `nobill: invoice 108 void`.
- **Editing and re-booking** a sent invoice leaves the time tags alone. The invoice decides what was billed, not the time file.

A line on a sent invoice is read-only in Entries until the invoice is voided.

From the command line, `void` takes `--time release` (the default) or `--time nobill`:

```sh
ledgeline-invoicer void 108 --time nobill
```

`time release 108` (and **Release** on the To bill page) is for tags left behind by an invoice that no longer exists, such as one whose file you deleted by hand. `--nobill` writes the time off instead. It's refused while invoice 108 is still a draft or a sent invoice that isn't void, since the same hours would then be billed twice. Delete the draft or void the invoice instead, which releases the time itself.

## Writing time off

To keep time from ever being billed, mark it **not billable** in Entries (**Mark not billable…**, with an optional reason). That adds `nobill:` to each line, and the time leaves To bill. **Mark billable** takes it off again. Time on an account that isn't billable at all never shows as unbilled, so this is for single lines on billable accounts.

## Paying a contractor

Some of the time in your books may be a contractor's, paid by the hour. Their time becomes a bill you owe, booked to accounts payable, rather than an invoice to a customer. The same time can still be billed to your customer: the `invoice:` and `bill:` tags are separate.

### Their time cards

A contractor's time is any time with their `person:` name: a `.timedot` file they send you, dropped into `time/<name>/`; time they track with the invoicer on a shared books folder; or lines you add in Entries with their name as the Person.

Each contractor needs a **timekeeper note**: a `type: person` or `type: organization` note tagged `timekeeper`, beside your customer notes. It says whose time it is and who gets paid. Its frontmatter can hold the defaults for their bills:

```yaml
---
title: Sam Lee
type: person
tags: [timekeeper]
time_person: sam                      # the person: name on their time; default: from the title
pay_rate: $85                         # hourly rate
vendor: Sam Lee                       # the vendor: tag in the books; default: the title
expense_account: expenses:contractors
payable_account: liabilities:accounts payable
pay_terms: 15                         # days until it's due
pay_to: "[[Lee Consulting LLC]]"      # pay another note's vendor, such as their company
---
```

Without `time_person`, the note's title gives the name the same way setup suggests yours: the first word, in lower case, so `Sam Lee` matches `sam`. Use an organization note for an agency, or set `pay_to` to pay the contractor's company.

Everything but the title and the tag is optional. `payable_account` can be left out when your books declare one payable account.

### Making the bill

The **To pay** tab, on the To bill page, lists each timekeeper's time that isn't on a bill yet, billable or not. **Create bill…** on a person, or on one of their periods, opens the Pay dialog. It shows who is paid, the lines with their hours and amounts, and the total with its due date.

- Anything the note doesn't give is an empty field, and the first takes focus: **Hourly rate**, **Expense account**, and **Payable account** if your books have more than one. **Terms (days)**, **Date** and **Reference** can be changed too.
- **Save to Sam Lee's note**, ticked by default, writes your answers into the note's frontmatter, so the next bill doesn't ask. Untick it to use them for this bill only.
- With no note for the person, **Whose note** asks you to **Pick a note** or **Create note…** (a person or an organization) first. A bill always has someone to pay.
- **A line per account** is the default. **One line in total** makes a single line.
- Time with no person can't be paid. Set who did it in Entries first.

<kbd>Cmd</kbd>+<kbd>Enter</kbd> (<kbd>Ctrl</kbd>+<kbd>Enter</kbd>) creates the bill, and <kbd>Esc</kbd> cancels.

**Create bill** books the bill in your journal, owed to the vendor, and tags each line of time with `bill:` and the bill's reference:

```journal
2026-11-26 Lee Consulting | sam timesheet 2026-11  ; reference: sam timesheet 2026-11, due: 2026-12-11
    liabilities:accounts payable  $-977.50  ; vendor: Lee Consulting
    expenses:contractors          $212.50  ; vendor: Lee Consulting, hours: 2.50, time: clients:acme:advisory
    expenses:contractors          $722.50  ; vendor: Lee Consulting, hours: 8.50, time: clients:globex:audit
    expenses:contractors          $42.50  ; vendor: Lee Consulting, hours: 0.50, time: internal:admin
```

```timedot
2026-11-04 Wed
    clients:acme:advisory          150m  ; second opinion on the K-1, bill: sam timesheet 2026-11
    clients:globex:audit             4h  ; fieldwork, bill: sam timesheet 2026-11
```

Record the payment the way you record any bill you pay.

### Voiding a bill

A line on a bill is read-only in Entries. Its **Void bill and release time…** writes a reversing entry for the bill in the books, so nothing is owed on it, and takes the `bill:` tag off every line on it. The time is unpaid again, and can be changed and billed anew.

A bill that's paid, with a payment linked to it, can't be voided this way. Unlink the payment first. From the command line, `time release --bill REF` does the same, and `--nobill` writes the time off instead, as `nobill: bill <reference> void`:

```sh
ledgeline-invoicer time release --bill "sam timesheet 2026-11"
```

## The tags billing uses

| Tag       | Written by                             | Value                         | Means                                         |
|-----------|----------------------------------------|-------------------------------|-----------------------------------------------|
| `invoice` | **Create draft**                       | the invoice number            | Billed on that invoice.                        |
| `bill`    | **Create bill**                        | the bill's reference          | Paid to the person through that bill.          |
| `nobill`  | **Mark not billable…**, a void         | a reason, or nothing          | Written off, never to be billed.               |

`invoice:` is the same tag the books use on the invoice's booking. So `hledger reg tag:invoice=101`, run over your books and time together, shows the invoice and the hours on it. See [Time with hledger](hledger.md#what-went-on-an-invoice).

## From the command line

| Command                                   | Does                                                                                  |
|-------------------------------------------|---------------------------------------------------------------------------------------|
| `time unbilled`                           | Each customer's unbilled hours, with the period to bill, and the time that needs a customer |
| `time bill CUSTOMER`                      | Bill a customer's time onto a draft                                                   |
| `time unpaid`                             | Each timekeeper's unpaid hours; `--everyone` for everyone's                           |
| `time pay PERSON`                         | Book a contractor's bill                                                              |
| `time release INVOICE`                    | Take `invoice:` off time whose invoice no longer exists; `--nobill` writes it off instead |
| `time release --bill REF`                 | Void a contractor's bill and release its time; `--nobill` writes it off instead       |

`time bill` takes `--from` and `--to` (default: the suggested period), `--group account|account-person|account-note`, `--item GROUP=CODE` and `--rate GROUP=$250` for a group (repeat them), `--remember`, `--detail none|daily|entries`, `--date`, `--reference`, and `--dry-run` to show the bill without writing it.

`time pay` takes `--from`, `--to`, `--rate`, `--expense-account`, `--payable-account`, `--note` for a note when none is linked, `--total` for one line, `--no-save` to leave the note alone, `--date`, and `--dry-run`. When something's missing it exits with code 2 and lists it:

```console
$ ledgeline-invoicer time unbilled
Acme Corp                   8h 04m  bill 2026-10-01 to 2026-10-31: 50m
Globex                     16h 30m  bill 2026-09-29 to 2026-10-31: 3h
Initech Inc                    40m  bill 2026-10-20 to 2026-10-31: 40m
needs a customer: clients:orphan 15m
$ ledgeline-invoicer time bill "Acme Corp" --dry-run
Acme Corp 2026-10-01 to 2026-10-31: 50m actual, 54m billed, $225.00
  Advisory, Oct 2026                         0.90 h      $250  $225.00
$ ledgeline-invoicer time pay riley --dry-run
riley timesheet 2026-11 3h for Riley Chen: ?
  needs rate: riley's hourly rate
  needs expense_account: the expense account the time is booked to
```
