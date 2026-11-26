---
title: Invoicer settings
description: "The Ledgeline Invoicer Settings page: these books' settings (company, new invoices, accounts, payments, editing, the dashboard, folders and wording) and the shared appearance, how Save works, and every setting under [invoicer] in ledgeline.toml with its default."
order: 50
tags: [invoicer]
---

# Invoicer settings

The **Settings** page holds everything you can change about how the invoicer works with your books. It has two tabs. **These books** holds the settings of the books you have open, and **Appearance** holds the theme and text size, which are shared with Ledgeline.

These books' settings are kept in `ledgeline.toml`, the file beside your main journal that Ledgeline also reads. You never have to open that file, but you can: [In the settings file](#in-the-settings-file) lists every setting as the file writes it.

## Opening Settings

Any of these opens the Settings page:

- **Settings** at the foot of the sidebar;
- <kbd>g</kbd> <kbd>,</kbd> from anywhere in the app;
- <kbd>Cmd</kbd>+<kbd>,</kbd> on macOS, <kbd>Ctrl</kbd>+<kbd>,</kbd> on Linux, which works from inside a field too;
- **Settings…** in the desktop app's menu: the app menu on macOS, the File menu on Linux.

On the page, <kbd>1</kbd> shows These books and <kbd>2</kbd> shows Appearance. With the focus on a tab, <kbd>←</kbd> and <kbd>→</kbd> move between them. Switching tabs keeps any changes you haven't saved yet.

## These books

These books is one form. Change as many settings as you like, then save them together.

### Save and Discard changes

As soon as something differs from what is saved, **Unsaved changes**, **Discard changes** and **Save** appear at the top right of the page. They stay in view as you scroll.

- **Save**, or <kbd>Cmd</kbd>+<kbd>S</kbd> (<kbd>Ctrl</kbd>+<kbd>S</kbd> on Linux), writes your changes. "Saved." confirms it.
- **Discard changes** puts every setting back as it is saved.

Save writes only the settings you changed. Some saves ask a question first: [creating the settings file](#when-the-app-writes-the-settings-file), or confirming a [change that affects new bookings](#changes-that-affect-new-bookings).

Save stays off while a setting has a problem, such as an aging range smaller than the one before it. The setting says what is wrong, in red, under its control. If you leave the page with unsaved changes, the app asks "Your settings have unsaved changes. Save them first?", with **Save**, **Discard** and **Cancel**.

### What each row shows

Each setting has a plain label and one sentence of help on the left, and its control on the right. In a narrow window, or at a large text size, the control goes under the label. Beside or under the control:

- **Default** means these books don't set it, so the invoicer uses its built-in value.
- **Found for you: …** appears on an account the books don't set. It names the account the invoicer found in your books and is using.
- **Use the default** appears on a setting the books do set. Choose it, then Save, and the setting goes back to its default. The app takes it out of the settings file.
- **Changed** marks a setting you have changed but not saved yet.
- **Affects new bookings** marks a setting that changes how new invoices and payments book. Saving a change to one asks first.

From the widest windows down, a list of the sections sits on the left; in a narrow window it is a row of links above the form. A dot beside a section means it holds unsaved changes.

### Company

| Setting      | What it does                                                            |
|--------------|-------------------------------------------------------------------------|
| Your company | The organization note whose name, address and logo go on your invoices. |

Choose the note by its title. Its details, such as the address and the logo, are in the note itself; see [Your company](files.md#your-company).

### New invoices

| Setting               | What it does                                                                                                     |
|-----------------------|------------------------------------------------------------------------------------------------------------------|
| New invoice file name | How a new invoice's file is named. It must include the number, so no two invoices share a file.                  |
| First invoice number  | The number the first invoice gets. After that, each invoice gets one more than the highest so far.               |
| Default payment terms | Days until a new invoice is due, when neither the invoice nor its customer says. Leave it empty for no due date. |
| Default template      | The look a new invoice starts with, when its customer has no template of its own.                                |

The file name is built from pieces. **Add Date**, **Add Customer** and **Add Number** add a piece that is filled in for each invoice, the × on a piece removes it, and you type any text in between. Under it, an example shows what the next invoice's file would be called, such as "2026-11-25 Acme Invoice 1001.md". The Number piece can't be left out.

Once your books have invoices, First invoice number shows the next number and where it comes from. The setting matters only before the first invoice.

Default payment terms apply to new invoices only. An invoice's own terms come first, then its customer's, then this setting. Invoices you already have keep their terms.

Default template lists your templates and the starters. A starter is copied into your books the first time an invoice uses it. Leave it at **The first template for invoices** to keep the invoicer's usual choice. See [Choosing a template](templates.md#choosing-a-template).

### Accounts

| Setting            | What it does                                                                                                       |
|--------------------|--------------------------------------------------------------------------------------------------------------------|
| Receivable account | Where accrual books record what customers owe you.                                                                 |
| Revenue account    | Where a row's amount books when neither the row, its catalog item nor its customer names an account.               |
| Deposit account    | Where deposit invoices book until their final invoice uses them up.                                                |
| Discount account   | An account for every discount that doesn't name one. Without it, a discount is shared over the rows it applies to. |

Each account field suggests your books' accounts as you type. An account you leave unset is found for you, and the row says which one: [Account settings](#account-settings) says how. Every account here affects new bookings.

### Payments

| Setting                           | What it does                                                                                                   |
|-----------------------------------|----------------------------------------------------------------------------------------------------------------|
| Deposit payments                  | In cash books, when a deposit counts as income.                                                                |
| Splitting a payment over rows     | In cash books, how a payment that doesn't cover the whole invoice is shared among its rows.                    |
| How far back to look for payments | How many days after an invoice a payment may arrive and still be offered as its payment. 0 looks back forever. |
| Short-payment tolerance           | A payment this much short of its invoice still counts as paid, with the difference booked as a fee.            |
| Payment before invoice            | How many days before an invoice's date a payment may arrive and still count for it.                            |

Deposit payments has two choices:

- **Count a deposit as income when it arrives**, the default;
- **Hold a deposit as owed until its final invoice**.

Splitting a payment over rows has two choices:

- **Move each account toward its row's amount**, the default;
- **Split each payment by the rows' proportions**.

Both are cash-books settings and both affect new bookings. In accrual books the page folds them into one line: "These books use accrual; the cash-books choices don't apply." [Booking](booking.md#cash-books) explains what each choice books.

The last three are the [payment-matching settings](#payment-matching-shared-with-ledgeline), shared with Ledgeline.

### Editing

| Setting                                | What it does                                                                               |
|----------------------------------------|--------------------------------------------------------------------------------------------|
| Recalculate drafts after outside edits | What happens when a draft's rows change in another editor and its totals no longer add up. |

The choices are **Recalculate a draft's totals**, the default, and **Leave them, and show the difference**. With the second, the invoice page shows the mismatch and a **Recalculate…** button. A sent invoice is never recalculated on its own.

This section also shows two rows that are [coming later](#coming-later): sending into a closed period, and editing a sent invoice.

### Dashboard

| Setting                       | What it does                                                                                     |
|-------------------------------|--------------------------------------------------------------------------------------------------|
| Aging ranges                  | The last day of each range after Current: one to eight numbers, each larger than the one before. |
| Days-sales-outstanding window | How many days back days-sales-outstanding is measured over.                                      |
| Paid window                   | The period the dashboard's Paid card starts on, in days.                                         |

Type the aging ranges as numbers separated by commas: `15, 45` gives Current, 1–15, 16–45 and 46+. See [Dashboard](dashboard.md#settings).

### Files and folders

| Setting          | What it does                                                                    |
|------------------|---------------------------------------------------------------------------------|
| Invoices folder  | Where new invoices are written. Existing ones are found wherever they are.      |
| Customers folder | Where new customers are written.                                                |
| Templates folder | Where new templates and snippets are written, and starter templates are copied. |
| Catalog note     | The note new catalog items go in.                                               |
| PDF folder       | Where exported PDFs are kept. Notes in it are never read as invoices.           |

Each is a folder or note inside your books folder, written from the books folder, such as `invoices/pdf`.

### Books

| Setting         | What it does                                                                                 |
|-----------------|----------------------------------------------------------------------------------------------|
| Accrual or cash | Whether these books count income when you invoice it, or when you're paid. Set in Ledgeline. |
| Closed through  | Entries on or before this day are kept as they are. Set in Ledgeline.                        |

These two belong to Ledgeline, so the invoicer only shows them. **Open in Ledgeline** opens these books there, on macOS with Ledgeline installed. See [Accrual or cash](booking.md#accrual-or-cash).

### Wording

Wording is folded away at the end, marked "(advanced)". Its settings are the words the app recognizes in tables, total rows, tags and contact labels. The first word in each list is the one the app writes.

| Setting                | What it does                                                    |
|------------------------|-----------------------------------------------------------------|
| Description column     | Headers that mark a table's description column.                 |
| Quantity column        | Headers that mark the quantity column.                          |
| Rate column            | Headers that mark the rate column.                              |
| Amount column          | Headers that mark the amount column.                            |
| Date column            | Headers that mark the date column.                              |
| Unit column            | Headers that mark the unit column.                              |
| Item code column       | Headers that mark the column of catalog codes.                  |
| Subtotal rows          | Labels of subtotal rows.                                        |
| Total rows             | Labels of total rows; the first ends a new invoice's table.     |
| Deposit rows           | Labels of a row that applies a deposit.                         |
| Tax rows               | Labels of a new tax row.                                        |
| Customer tag           | The one tag that marks a contact note as a customer.            |
| Billing contact labels | Which email, phone and address to bill, in order of preference. |

Each list shows its words as chips. Type a word and press <kbd>Enter</kbd> or <kbd>,</kbd> to add it, and press × on a chip to remove it. <kbd>Backspace</kbd> in an empty field removes the last word. A list needs at least one word. See [Words](#words) for the defaults and an example in German.

## When the app writes the settings file

The app reads these books' settings from `ledgeline.toml` and writes it only when you press Save.

- Opening the Settings page, or any other page, writes nothing.
- Save writes only the settings you changed. A setting the file doesn't have is added only once you change it and save.
- **Use the default** takes the setting out of the file, with any comment lines directly above it.
- Your comments, the order of the settings, Ledgeline's own settings and anything else in the file stay as they are.

If your books have no `ledgeline.toml` yet, the page says so, and every setting shows its default. The first Save asks: "These books have no settings file yet. Create it?" **Create it and save** creates the file beside your main journal, with a short comment at the top saying what it is, and saves your changes into it. **Cancel** creates nothing and keeps your changes on the page.

## Changes that affect new bookings

The accounts, Deposit payments and Splitting a payment over rows change what new bookings do. When you save a change to any of them, the app first lists each change with what it will do, and asks you to confirm. For example:

> Splitting a payment over rows: Payments you apply from now on are split pro rata. Payments already in your books keep the split they were booked with.

Each change applies from now on only. Invoices and payments already in your books keep how they were booked. When a change leaves sent invoices as they were, the confirmation says how many, and that re-booking one moves it. **Save these changes** saves them. **Cancel** keeps them unsaved on the page.

### Deposit payments while a deposit is waiting

In cash books, the app won't save a change to Deposit payments while a deposit you have received is still waiting for its final invoice. Changing it then would count that deposit as income twice, or leave the deposit account below zero. The app says "This change can't be saved yet" and lists the deposits that are waiting, each with its invoice, customer, date and the amount still waiting. Choose **Undo this change** to put the setting back, or **Keep editing**. Once the final invoices have used up those deposits, you can change the setting.

## When the settings can't be changed here

The page turns read-only when it can't safely write the settings file:

- the file is a symbolic link;
- it is larger than 64 KiB;
- it isn't valid TOML;
- its invoicer settings have a problem, such as a misspelled setting or a value out of range.

The page then says "These settings can't be changed here.", explains why in plain words, lists any problems by setting, and turns every control off. **Open the settings file** opens it so you can fix it. When the books themselves couldn't open because of the problem, the page still opens and says so. Once you fix the file, the page reloads on its own.

## When the file changes outside the app

The page follows the settings file. When the invoicer's settings in it change outside the app, because you edited the file in another editor or pulled a change with git, the page reloads:

- With no unsaved changes, it shows the new values. Nothing asks you anything.
- With unsaved changes, the page keeps them and says the settings file changed on disk while you were editing it. **Reload theirs (discard mine)** shows the file as it now is. **Keep mine (save over theirs)** saves your changes over it. Until you choose, Save is refused.

A change to Ledgeline's own settings in the same file, such as closing the books through a date, never conflicts with your unsaved changes here.

## Payment matching, shared with Ledgeline

How far back to look for payments, Short-payment tolerance and Payment before invoice decide which payments **Apply payment…** offers for an invoice. Ledgeline's [invoice matching](../journal/invoice-matching.md) uses the same three settings, so a change in either app changes them in both. In Ledgeline they are under **Settings → Books**, as **Invoice matching**.

| Setting                           | Default | Range                                |
|-----------------------------------|---------|--------------------------------------|
| How far back to look for payments | 180     | 0 to 3650 days; 0 looks back forever |
| Short-payment tolerance           | 3%      | 0% to 100%                           |
| Payment before invoice            | 4       | 0 to 60 days                         |

They save with the rest of the form, when you press Save. They apply to every set of books you open, unless you tick **Use for these books only** under one of them. That keeps its value for the books you have open: the app saves it into their `ledgeline.toml` at once, creating the file if needed, and from then on changes to it in either app stay with these books. Untick it to share the value again. The box isn't offered when [view settings are kept in the browser](../settings/settings-files.md#keeping-view-settings-in-the-browser).

## Appearance

The Appearance tab sets the [theme](../settings/themes.md) and the text size. They are shared with Ledgeline: a change here changes it there too. Each applies the moment you choose it. There is no Save.

- **Theme.** Click a theme's card to use it. On a card with **Auto**, **Light** and **Dark**, Auto follows your system's light or dark setting.
- **Text size.** Choose from 70% to 200%, or use **−**, **+** and **Reset**. All text grows or shrinks together, and the layout around it follows. <kbd>Cmd</kbd>+<kbd>+</kbd>, <kbd>Cmd</kbd>+<kbd>−</kbd> and <kbd>Cmd</kbd>+<kbd>0</kbd> (<kbd>Ctrl</kbd> on Linux) do the same from anywhere in the app, as does the desktop app's View menu.

Tick **Use for these books only** beside either one to keep it for the books you have open, as with the payment-matching settings. "These books have their own theme." then shows under Theme.

## Coming later

Some choices are planned but not built yet. The page shows them turned off, marked **Coming later**, so you can see what is on the way:

| Where                         | What happens now                              | Coming later                                                            |
|-------------------------------|-----------------------------------------------|-------------------------------------------------------------------------|
| Splitting a payment over rows | The two choices above                         | Pay the oldest row first                                                |
| Deposit payments              | The two choices above                         | Keep a deposit as a deferral, counted as income as its work is invoiced |
| Sending into a closed period  | Refused: change the date to an open day first | Book it on the first open day                                           |
| Editing a sent invoice        | Its booking is replaced where it stands       | Reverse it and book it again                                            |

## In the settings file

The Settings page is the easy way to change these. This section is for reading or editing `ledgeline.toml` yourself, and for scripts.

The invoicer's settings go under `[invoicer]` in `ledgeline.toml`, the file beside your main journal. See [Settings files](../settings/settings-files.md) for how that file works. Every setting is optional: books without an `[invoicer]` table get the defaults below.

```toml
[invoicer]
company = "[[My Company]]"
revenue-account = "revenue:consulting"
deposit-account = "liabilities:customer-deposits"
number-start = 1001
default-terms = 30
```

`ledgeline-invoicer config` prints every setting as the app sees it, defaults included, and the payment-matching settings in use. A misspelled setting is an error, not something silently ignored: every command stops and names it, with the list it expected.

Your books' basis and closed-through date are Ledgeline's `[books]` settings; see [Booking](booking.md#accrual-or-cash). The theme and text size are view settings shared with Ledgeline; see [Themes](../settings/themes.md).

### Folders and your company

These decide where **new** files are written. The app finds existing files wherever they are; see [Files and formats](files.md#where-things-live). Paths are from the books folder and must stay inside it.

| Setting         | On the page      | Default               | What it is                                                                     |
|-----------------|------------------|-----------------------|--------------------------------------------------------------------------------|
| `invoices-dir`  | Invoices folder  | `invoices`            | The folder for new invoices.                                                   |
| `customers-dir` | Customers folder | `customers`           | The folder for new customers.                                                  |
| `templates-dir` | Templates folder | `invoices/tpl`        | Where starter templates are copied and new templates and snippets are written. |
| `catalog`       | Catalog note     | `invoices/catalog.md` | The catalog note new catalog items go in.                                      |
| `pdf-dir`       | PDF folder       | `invoices/pdf`        | Where exported PDFs are kept. The app never reads notes from this folder.      |
| `company`       | Your company     | none                  | Your company: the title of an organization note, or a link to it.              |

### Numbering, file names and defaults

| Setting            | On the page           | Default                                        | What it is                                                                                                                                                |
|--------------------|-----------------------|------------------------------------------------|-----------------------------------------------------------------------------------------------------------------------------------------------------------|
| `number-start`     | First invoice number  | `1`                                            | The first invoice number, used while no invoice has one. A number (`1001`) or text (`"2026-0001"`).                                                       |
| `filename`         | New invoice file name | `"{date} {customer} Invoice {invoice_num}.md"` | The name of a new invoice's file. `{date}`, `{customer}` and `{invoice_num}` are filled in. It must contain `{invoice_num}`.                              |
| `default-terms`    | Default payment terms | none                                           | Net days for a new invoice whose customer has no `terms`, 0 to 3650. Without it, such an invoice has no due date.                                         |
| `default-template` | Default template      | the first template for invoices                | The template a new invoice starts with when its customer has no `invoice_template`, as an invoice's `template:` names it: `"/invoices/tpl/classic.html"`. |

After the first invoice, the next number is one more than the highest number in your invoice files or your books. A number ending in digits counts up from those digits, so after `2026-0042` comes `2026-0043`. There is no counter file to keep in step.

If `default-template` names a template that isn't in your books, or one that isn't for invoices, a new invoice gets the first template for invoices instead, and the app says why.

### Account settings

Each account is found for you when it isn't set. Set it when your books have more than one candidate, or when you want another account.

| Setting              | On the page        | When unset                                                                            | What it is                                                                               |
|----------------------|--------------------|---------------------------------------------------------------------------------------|------------------------------------------------------------------------------------------|
| `receivable-account` | Receivable account | the books' one receivable account; with more than one, booking asks you to set this   | Where accrual books record what customers owe.                                           |
| `revenue-account`    | Revenue account    | the books' one revenue account                                                        | Where a row books when neither the row, its catalog item nor the customer says.          |
| `deposit-account`    | Deposit account    | the one account declared `kpi: deferred-revenue`, else `liabilities:deferred-revenue` | Where deposit invoices book. See [Deposits](booking.md#deposits).                        |
| `discount-account`   | Discount account   | none: discounts are shared over the rows they apply to                                | An account for every discount with a blank books cell, such as a contra-revenue account. |

The app recognizes accounts by their declared type or `kpi:` tag first and by name only as a fallback, as Ledgeline does. See [Declaring accounts](../settings/accounts.md).

### Cash-books policies

These matter only in cash books. Each has a sensible default; change it if your accountant does it the other way.

| Setting              | On the page                   | Default             | Other choice            | What it decides                                                                                                                                                                 |
|----------------------|-------------------------------|---------------------|-------------------------|---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `cash-deposits`      | Deposit payments              | `income-on-receipt` | `liability-until-final` | Whether a deposit is income the day it arrives, or held in the deposit account until the final invoice is paid.                                                                 |
| `payment-allocation` | Splitting a payment over rows | `match-rows`        | `pro-rata`              | How a payment that doesn't cover the whole invoice is split over its rows. `match-rows` moves each account toward its row's amount; `pro-rata` splits by the rows' proportions. |

### Recalculation

| Setting          | On the page                            | Default  | Other choice | What it decides                                                                                                                                                                                                                   |
|------------------|----------------------------------------|----------|--------------|-----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `recalc-on-save` | Recalculate drafts after outside edits | `drafts` | `never`      | Whether the app rewrites a draft's amounts after you change its rows in another editor. With `never`, the invoice page shows the mismatch and a **Recalculate…** button instead. A sent invoice is never recalculated on its own. |

### Dashboard settings

| Setting            | On the page                   | Default        | What it decides                                                                                              |
|--------------------|-------------------------------|----------------|--------------------------------------------------------------------------------------------------------------|
| `aging-buckets`    | Aging ranges                  | `[30, 60, 90]` | The aging ranges: the last day of each range after Current. One to eight numbers, each larger than the last. |
| `dso-days`         | Days-sales-outstanding window | `90`           | How many days days-sales-outstanding looks back over, 1 to 3660.                                             |
| `paid-window-days` | Paid window                   | `30`           | The period the Paid card starts on, 1 to 3660 days.                                                          |

See [Dashboard](dashboard.md#settings).

### Payment-matching keys

The three payment-matching settings aren't under `[invoicer]`. They are view settings shared with Ledgeline, kept in `ui-prefs.toml`, or under `[ui]` in `ledgeline.toml` for books that keep their own. See [Settings files](../settings/settings-files.md#the-keys).

| Setting                  | On the page                       | Default | Allowed values                                    |
|--------------------------|-----------------------------------|---------|---------------------------------------------------|
| `settlementLookbackDays` | How far back to look for payments | `180`   | 0 to 3650 days. 0 means no limit                  |
| `settlementToleranceBps` | Short-payment tolerance           | `300`   | 0 to 10000, in hundredths of a percent: 300 is 3% |
| `settlementLeadDays`     | Payment before invoice            | `4`     | 0 to 60 days                                      |

```toml
[ui]
# These books are paid slowly: look back a year.
settlementLookbackDays = 365
```

### Words

The app recognizes some things by their English words: table headers, total rows, the customer tag, contact labels. Each list is a setting under `[invoicer.words]`, so you can work in another language or your own terms. Setting a key replaces its whole list. The first word in a list is the one the app writes.

| Key              | On the page            | Default                                               | Used for                                                       |
|------------------|------------------------|-------------------------------------------------------|----------------------------------------------------------------|
| `description`    | Description column     | `["Description", "Item", "Service", "Product"]`       | the description column's header                                |
| `quantity`       | Quantity column        | `["Qty", "Quantity", "Hours", "Units"]`               | the quantity column                                            |
| `rate`           | Rate column            | `["Rate", "Price", "Unit price", "Unit cost"]`        | the rate column                                                |
| `amount`         | Amount column          | `["Amount", "Total", "Line total"]`                   | the amount column                                              |
| `date`           | Date column            | `["Date", "Service date"]`                            | the date column                                                |
| `unit`           | Unit column            | `["Unit"]`                                            | the unit column                                                |
| `item`           | Item code column       | `["Code", "SKU"]`                                     | the catalog code column                                        |
| `subtotal`       | Subtotal rows          | `["Subtotal"]`                                        | subtotal rows                                                  |
| `total`          | Total rows             | `["Total due", "Total", "Balance due", "Amount due"]` | total rows; the first is the final row of a new table          |
| `deposit`        | Deposit rows           | `["Less deposit"]`                                    | the label of a row that applies a deposit                      |
| `tax`            | Tax rows               | `["Sales tax"]`                                       | the label of a new tax row                                     |
| `customer-tag`   | Customer tag           | `"customer"`                                          | the tag that marks a contact as a customer (one word)          |
| `billing-labels` | Billing contact labels | `["billing", "work"]`                                 | which email, phone and address to bill, in order of preference |

For example, to bill in German:

```toml
[invoicer.words]
description = ["Beschreibung"]
quantity    = ["Menge"]
rate        = ["Preis"]
amount      = ["Betrag"]
subtotal    = ["Zwischensumme"]
total       = ["Gesamtbetrag"]
```

The file format itself stays in English: frontmatter keys, `type` values, the `draft`, `sent` and `void` statuses, and the journal's tag names. Scripts and mbr rely on them.
