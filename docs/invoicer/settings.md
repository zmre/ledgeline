---
title: Invoicer settings
description: "Every Ledgeline Invoicer setting under [invoicer] in ledgeline.toml, with its default: folders, accounts, numbering, file names, cash-books policies, recalculation, the dashboard, and the words the app recognizes."
order: 50
tags: [invoicer]
---

# Invoicer settings

The invoicer has no settings screen and no settings file of its own. Its settings go under `[invoicer]` in `ledgeline.toml`, the file beside your main journal that Ledgeline already reads. See [Settings files](../settings/settings-files.md) for how that file works. Every setting is optional: books without an `[invoicer]` section get the defaults below.

```toml
[invoicer]
company = "[[My Company]]"
revenue-account = "revenue:consulting"
deposit-account = "liabilities:customer-deposits"
number-start = 1001
```

`ledgeline-invoicer config` prints every setting as the app sees it, defaults included. A misspelled key is an error, not something silently ignored: every command stops and names the key, with the list of keys it expected.

The theme and text size are shared with Ledgeline and set there. Your books' basis and closed-through date are Ledgeline's `[books]` settings; see [Booking](booking.md#accrual-or-cash).

## Where files go

These decide where **new** files are written. The app finds existing files wherever they are; see [Files and formats](files.md#where-things-live). Paths are from the books folder and must stay inside it.

| Setting         | Default               | What it is                                                                     |
|-----------------|-----------------------|--------------------------------------------------------------------------------|
| `invoices-dir`  | `invoices`            | The folder for new invoices.                                                   |
| `customers-dir` | `customers`           | The folder for new customers.                                                  |
| `templates-dir` | `invoices/tpl`        | Where starter templates are copied and new templates and snippets are written. |
| `catalog`       | `invoices/catalog.md` | The catalog note new catalog items go in.                                      |
| `pdf-dir`       | `invoices/pdf`        | Where exported PDFs are kept. The app never reads notes from this folder.      |
| `company`       | none                  | Your company: the title of an organization note, or a link to it.              |

## Numbers and file names

| Setting        | Default                                        | What it is                                                                                                                   |
|----------------|------------------------------------------------|------------------------------------------------------------------------------------------------------------------------------|
| `number-start` | `1`                                            | The first invoice number, used while no invoice has one. A number (`1001`) or text (`"2026-0001"`).                          |
| `filename`     | `"{date} {customer} Invoice {invoice_num}.md"` | The name of a new invoice's file. `{date}`, `{customer}` and `{invoice_num}` are filled in. It must contain `{invoice_num}`. |

After the first invoice, the next number is one more than the highest number in your invoice files or your books. A number ending in digits counts up from those digits, so after `2026-0042` comes `2026-0043`. There is no counter file to keep in step.

## Accounts

Each account is found for you when it isn't set. Set it when your books have more than one candidate, or when you want another account.

| Setting              | When unset                                                                            | What it is                                                                               |
|----------------------|---------------------------------------------------------------------------------------|------------------------------------------------------------------------------------------|
| `receivable-account` | the books' one receivable account; with more than one, booking asks you to set this   | Where accrual books record what customers owe.                                           |
| `revenue-account`    | the books' one revenue account                                                        | Where a row books when neither the row, its catalog item nor the customer says.          |
| `deposit-account`    | the one account declared `kpi: deferred-revenue`, else `liabilities:deferred-revenue` | Where deposit invoices book. See [Deposits](booking.md#deposits).                        |
| `discount-account`   | none: discounts are shared over the rows they apply to                                | An account for every discount with a blank books cell, such as a contra-revenue account. |

The app recognizes accounts by their declared type or `kpi:` tag first and by name only as a fallback, as Ledgeline does. See [Declaring accounts](../settings/accounts.md).

## Cash-books policies

These matter only in cash books. Each has a sensible default; change it if your accountant does it the other way.

| Setting              | Default             | Other choice            | What it decides                                                                                                                                                                 |
|----------------------|---------------------|-------------------------|---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `cash-deposits`      | `income-on-receipt` | `liability-until-final` | Whether a deposit is income the day it arrives, or held in the deposit account until the final invoice is paid.                                                                 |
| `payment-allocation` | `match-rows`        | `pro-rata`              | How a payment that doesn't cover the whole invoice is split over its rows. `match-rows` moves each account toward its row's amount; `pro-rata` splits by the rows' proportions. |

## Recalculation

| Setting          | Default  | Other choice | What it decides                                                                                                                                                                                                                   |
|------------------|----------|--------------|-----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `recalc-on-save` | `drafts` | `never`      | Whether the app rewrites a draft's amounts after you change its rows in another editor. With `never`, the invoice page shows the mismatch and a **Recalculate…** button instead. A sent invoice is never recalculated on its own. |

## Dashboard

| Setting            | Default        | What it decides                                                                                              |
|--------------------|----------------|--------------------------------------------------------------------------------------------------------------|
| `aging-buckets`    | `[30, 60, 90]` | The aging ranges: the last day of each range after Current. One to eight numbers, each larger than the last. |
| `dso-days`         | `90`           | How many days days-sales-outstanding looks back over. At least 1.                                            |
| `paid-window-days` | `30`           | The period the Paid card starts on, 1 to 3660 days.                                                          |

See [Dashboard](dashboard.md#settings).

## Words

The app recognizes some things by their English words: table headers, total rows, the customer tag, contact labels. Each list is a setting under `[invoicer.words]`, so you can work in another language or your own terms. Setting a key replaces its whole list. The first word in a list is the one the app writes.

| Key              | Default                                               | Used for                                                       |
|------------------|-------------------------------------------------------|----------------------------------------------------------------|
| `description`    | `["Description", "Item", "Service", "Product"]`       | the description column's header                                |
| `quantity`       | `["Qty", "Quantity", "Hours", "Units"]`               | the quantity column                                            |
| `rate`           | `["Rate", "Price", "Unit price", "Unit cost"]`        | the rate column                                                |
| `amount`         | `["Amount", "Total", "Line total"]`                   | the amount column                                              |
| `date`           | `["Date", "Service date"]`                            | the date column                                                |
| `unit`           | `["Unit"]`                                            | the unit column                                                |
| `item`           | `["Code", "SKU"]`                                     | the catalog code column                                        |
| `subtotal`       | `["Subtotal"]`                                        | subtotal rows                                                  |
| `total`          | `["Total due", "Total", "Balance due", "Amount due"]` | total rows; the first is the final row of a new table          |
| `deposit`        | `["Less deposit"]`                                    | the label of a row that applies a deposit                      |
| `tax`            | `["Sales tax"]`                                       | the label of a new tax row                                     |
| `customer-tag`   | `"customer"`                                          | the tag that marks a contact as a customer (one word)          |
| `billing-labels` | `["billing", "work"]`                                 | which email, phone and address to bill, in order of preference |

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
