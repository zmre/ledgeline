---
title: Files and formats
description: "The plain-text files Ledgeline Invoicer reads and writes: invoices, customers, your company, the catalog and snippets, field by field, and the bill table's rules."
order: 10
tags: [invoicer]
---

# Files and formats

Everything the invoicer knows is in plain files inside your books folder. This page describes them for anyone who edits them by hand or writes a script against them. You don't need it to use the app.

The files are markdown notes with a frontmatter block at the top, the same kind of notes [mbr](https://github.com/zmre/mbr-markdown-browser) shows. The app reads every change you make by hand. It never rewrites a file behind your back, except for the paid status (see [Booking](booking.md)) and the totals of a draft (see [Recalculation](#recalculation)).

## Where things live

The invoicer finds files by what they say, not by where they are. Any note in the books folder with `type: invoice` is an invoice, wherever it sits. The folders below only decide where **new** files go, and each one is a [setting](settings.md).

| What          | New files go in          | A file counts when it…                                                     |
|---------------|--------------------------|----------------------------------------------------------------------------|
| Invoices      | `invoices/`              | has `type: invoice`                                                        |
| Customers     | `customers/`             | has `type: person` or `type: organization` and the tag `customer`          |
| Your company  | anywhere                 | is the organization note the `company` setting names                       |
| Templates     | `invoices/tpl/`          | is an `.html` file that mentions `invoice_num` or extends the invoice base |
| Snippets      | `invoices/tpl/snippets/` | has `type: snippet`                                                        |
| Catalog       | `invoices/catalog.md`    | has `type: catalog`                                                        |
| Exported PDFs | `invoices/pdf/`          | is in that folder (never read back)                                        |

Hidden folders, `templates/` and `node_modules/` are skipped, and so is the PDF folder. That is why the template folder is `invoices/tpl/` and not `templates/`.

The app also keeps a folder of its own, `.ledgeline-invoicer/undo/`, in the books folder. It holds what it needs to undo a payment applied in cash books. Deleting it only loses those undos.

## An invoice

This is invoice 105 from the project's sample books:

```markdown
---
title: 2026-11-10 Customer X Invoice 105
type: invoice
template: /invoices/tpl/myco-default.html
status: sent
invoice_num: 105
date: 2026-11-10
terms: 30
due_date: 2026-12-10
reference: "PO #PY75334, rev. 2"
customer: "[[Customer X]]"
invoicee_name: Customer X
invoicee_email: accounts_receivable@customerx.com
invoicee_phone: +1 (303) 555-0142
invoicee_address: |
  1234 Main St. #555
  Springfield, CO 80000
invoicer: "[[My Company]]"
invoicer_name: My Company
invoicer_email: finance@mycompany.com
invoicer_phone: +1 (555) 010-2000
invoicer_address: |
  1234 Background Blvd.
  Anywhere, CA 90000
invoicer_logo: /company/logo.png
sent_date: 2026-11-10
sent_via: portal
paid: no
---

Hi Fred, this is the work we discussed, less the deposit you already paid.

| Description                 | Qty | Rate          | Amount      | *Account*                     |
|:----------------------------|----:|--------------:|------------:|:------------------------------|
| Senior architect hours      | 10  | $800          | $8,000      | revenue:consulting            |
| Friends and family discount |     | -10%          | -$800       |                               |
| Junior engineer             | 50  | $200          | $10,000     | revenue:consulting:delivery   |
| Friends and family discount |     | -5%           | -$500       |                               |
|                             |     | **Subtotal**  | **$16,700** |                               |
| Less deposit (Invoice 104)  |     |               | -$8,350     | liabilities:customer-deposits |
|                             |     | **Total due** | **$8,350**  |                               |

## Payment

Pay by ACH to account 123456789, routing 987654321, within 30 days.
```

Text above and below the bill table is ordinary markdown. Other tables, such as a timesheet, are left alone and printed as they are. The bill table is the first table with an Amount column.

### Frontmatter fields

Keys are lowercase with underscores, dates are `YYYY-MM-DD`, and amounts never appear in the frontmatter. That keeps a search like `grep -l 'status: sent'` reliable.

| Field                                           | Needed   | What it holds                                                                                                            |
|-------------------------------------------------|----------|--------------------------------------------------------------------------------------------------------------------------|
| `type`                                          | yes      | `invoice`.                                                                                                               |
| `invoice_num`                                   | yes      | The number, unique across the books. It needn't be a whole number: `2026-0042` works. No commas.                         |
| `date`                                          | yes      | The issue date, and the date the invoice is booked on in accrual books.                                                  |
| `status`                                        | yes      | `draft`, `sent` or `void`. What you intend; Send and Void change it.                                                     |
| `template`                                      | yes      | The template's path from the books folder, starting with `/`.                                                            |
| `customer`                                      | yes      | A link to the customer's note.                                                                                           |
| `invoicee_name`                                 | yes      | The customer's billing name as it was when the invoice was made.                                                         |
| `invoicee_email`                                | warning  | The billing email, copied the same way. Missing is a warning, not an error.                                              |
| `invoicee_address`                              | warning  | The billing address, copied the same way. Write it with `\|` so the lines stay apart.                                    |
| `invoicee_phone`                                | no       | Copied when the customer has a phone.                                                                                    |
| `invoicer`                                      | yes      | A link to your company's note.                                                                                           |
| `invoicer_name`, `_email`, `_address`, `_phone` | as above | Your company's details, copied the same way from its note.                                                               |
| `invoicer_logo`                                 | no       | Your logo's path, copied from the company note's `logo:`. See [Templates](templates.md#your-logo).                       |
| `terms`                                         | no       | Net days, a whole number. Copied from the customer.                                                                      |
| `due_date`                                      | no       | The date plus the terms, unless you set it yourself. "Overdue" is worked out from it, never stored.                      |
| `reference`                                     | no       | What the customer quotes back, such as a purchase order: `PO #PY75334`.                                                  |
| `deposit_for`                                   | no       | On a deposit invoice, what it is a deposit for.                                                                          |
| `paid`                                          | written  | `no`, `partial 2026-10-28 $1,500, 2026-11-19 $3,500`, or the paid-in-full date. Written from the books; none on void.    |
| `sent_date`                                     | written  | When it was first sent.                                                                                                  |
| `sent_via`                                      | no       | How it was delivered: `email`, `portal`, `mail`, or anything you like. Send sets it.                                     |
| `void_date`                                     | written  | When it was voided.                                                                                                      |
| `sent_hash`                                     | written  | A fingerprint of the last PDF exported, so the app can warn when the invoice changed after it was sent.                  |
| anything else                                   | no       | Your own fields. Templates see them, and the app keeps them exactly as written.                                          |

The app keeps the "written" fields up to date. If you change `paid:` by hand, the app writes back what the books say.

The default file name is `{date} {customer} Invoice {invoice_num}.md`, and the title matches it. The `filename` [setting](settings.md) changes the pattern.

## The bill table

### Columns

The invoicer recognizes columns by their header, in any order and any case. The books column must be last.

| Column      | Headers it recognizes                     | Holds                                                        |
|-------------|-------------------------------------------|--------------------------------------------------------------|
| Description | Description, Item, Service, Product       | Required. What the row is for.                               |
| Quantity    | Qty, Quantity, Hours, Units               | A number.                                                    |
| Rate        | Rate, Price, Unit price, Unit cost        | Money, or a percentage on a discount or tax row.             |
| Amount      | Amount, Total, Line total                 | Required. Money, written by the app.                         |
| Date        | Date, Service date                        | Optional.                                                    |
| Unit        | Unit                                      | Optional: `h`, `ea`, `mo`.                                   |
| Code        | Code, SKU                                 | Optional. A catalog item's code.                             |
| Books       | any header in italics, as the last column | Optional. The account the row books to. Hidden when printed. |

Each list of header words is a [setting](settings.md#words), so you can bill in another language. The first word in each list is the header new tables get.

### Row kinds

The app works out what each row is from what is in it.

| Row               | You write                                                        | Its amount                                                               |
|-------------------|------------------------------------------------------------------|--------------------------------------------------------------------------|
| Item              | a description, and usually a quantity and a rate                 | quantity times rate, or what you wrote when there is no quantity or rate |
| Percentage        | no quantity, and a rate like `-10%` or `8.25%`                   | that percentage of the nearest item or total row above it                |
| Fixed adjustment  | no quantity, no rate, and an amount with a sign: `-$500`, `+$12` | as written                                                               |
| Subtotal or total | a bold word such as **Subtotal** or **Total due**                | the sum of every row above it that isn't itself a total                  |
| Spacer            | nothing in any cell                                              | none                                                                     |

Position decides what a percentage applies to. A discount under an item discounts that item only. A discount under a Subtotal discounts everything above it. A tax row under the subtotal taxes the total after discounts. The last total row is the amount due, and it is the amount that gets booked.

An amount with no sign and no quantity or rate, such as `Shipping | | | $12`, is an item. Percentage signs are optional: `8.25` and `8.25%` read the same.

### The books column

The last column, with its header in italics, says where each row books. Customers never see it: every template hides it.

A cell holds an account name, or a deferral:

```text
revenue:consulting
liabilities:deferred-revenue → revenue:support, 12 months from 2026-12-01
```

A deferral books the row to the first account and moves it into the second a month at a time, over the months given. `->` works in place of `→`. Without `from`, the schedule starts on the invoice date. Add `calendar` at the end for calendar-month periods instead of periods from the start date. Ledgeline posts these entries; see [Revenue and expense recognition](../journal/accrual-recognition.md).

A blank cell uses the first of these that gives an account:

1. On a discount or tax row, the account of the row it applies to. A percentage of a subtotal is spread over the item rows above it in proportion to their amounts. The `discount-account` [setting](settings.md) sends discounts to one account instead.
2. The catalog item's account.
3. The customer's `revenue_account`.
4. The `revenue-account` setting.
5. The one revenue account in your books, if there is exactly one.

If none of them gives an account, the editor marks the row and the invoice can't be sent.

### Arithmetic

All arithmetic is exact decimal math, never floating point. Each computed amount is rounded to the currency's cents as it is computed, and totals add up the rounded amounts, so the table always adds up to the cent. Amounts are written the way your journal writes that currency: `$8,000.00` in a journal that writes dollars that way.

An invoice is in one currency.

### Recalculation

The Amount column is written into the file, so the invoice reads correctly in any markdown viewer. When you change a quantity or rate by hand in another editor, the written amounts go stale. What happens next depends on the invoice:

- **A draft** is recalculated by the app once your edit settles, if the app is open. The `recalc-on-save` [setting](settings.md) turns this off.
- **A sent invoice** is never rewritten on its own, because its amount is in the books. Its page says the totals don't match the rows and offers **Recalculate…**, which re-books it after you confirm.

From the command line, `ledgeline-invoicer recalc` does the same and `ledgeline-invoicer check` reports a table that doesn't add up without changing it. See [Command line](cli.md).

## A customer

A customer is a person or organization note with the tag `customer`. The contact fields are the ones mbr uses for any contact. Invoicing adds a few of its own.

```yaml
---
title: Customer X
type: organization
tags: [customer]
emails:
  - billing: accounts_receivable@customerx.com
  - work: hello@customerx.com
phones:
  - billing: "+1 (303) 555-0142"
addresses:
  - billing: |
      1234 Main St. #555
      Springfield, CO 80000
terms: 30
revenue_account: revenue:consulting
invoice_template: /invoices/tpl/myco-default.html
ledger_customer: Customer X
contracts:
  - "[[Customer X MSA 2026]]"
  - "/customers/contracts/Customer X SOW 3 renewal (signed).pdf"
open_pos:
  - reference: "PO #PY75334"
    amount: $40,000
    expires: 2027-06-30
billing_portal:
  name: Coupa
  url: https://customerx.coupahost.com/
  login: finance@mycompany.com
  supplier_id: SUP-44821
  requires_reference: true
  instructions: |
    Upload the PDF under Invoices → Create from PO, then attach the timesheet.
snippets: ["[[Net 30 ACH]]"]
---

Fred Smith is the champion.
```

| Field                           | What it does                                                                                                                                                                                                                |
|---------------------------------|-----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `emails`, `phones`, `addresses` | Labelled lists. The invoice uses the entry labelled `billing`, then `work`, then the first. An address may be text or a map with `street`, `city`, `region`, `postcode` and `country`.                                      |
| `company`                       | On a person: a link to the organization they work for. Invoices to the person bill the organization.                                                                                                                        |
| `terms`                         | Net days for new invoices.                                                                                                                                                                                                  |
| `revenue_account`               | The account rows book to when nothing more specific says otherwise.                                                                                                                                                         |
| `invoice_template`              | The template new invoices for this customer use.                                                                                                                                                                            |
| `ledger_customer`               | The customer's name in the books, the value of the `customer:` tag. Without it, the note's title. Set it before renaming a note, so the books keep one name.                                                                |
| `contracts`                     | Links to notes, or paths to files such as signed PDFs. The customer page lists them, and `check` reports paths that don't exist.                                                                                            |
| `open_pos`                      | Purchase orders: `reference`, `amount` and `expires`. The invoice editor suggests them for Reference, and the customer page shows how much of each is left.                                                                 |
| `billing_portal`                | A vendor portal the customer takes invoices through: `name`, `url`, `login` (the user name only, never a password), `supplier_id`, `requires_reference` (refuse to send an invoice without a reference) and `instructions`. |
| `snippets`                      | Snippets new invoices for this customer start with.                                                                                                                                                                         |

Closing a purchase order on the customer page removes it from `open_pos` and adds a line about it under a "Closed POs" heading at the end of the note.

## Your company

Your company is an organization note, named by the `company` [setting](settings.md). It uses the same contact fields as a customer, plus `logo:`, which names an image for the invoice header. See [Your logo](templates.md#your-logo).

## The catalog

The catalog is a note with `type: catalog` whose body is one table. It uses the bill table's columns and its books column:

```markdown
---
title: Catalog
type: catalog
---

| Code   | Description                   | Unit | Rate    | *Account*                                              |
|:-------|:------------------------------|:-----|--------:|:-------------------------------------------------------|
| ARCH   | Senior architect hours        | h    | $800    | revenue:consulting                                     |
| JR     | Junior engineer               | h    | $200    | revenue:consulting:delivery                            |
| SAAS-Y | Platform subscription, 1 year | ea   | $12,000 | liabilities:deferred-revenue → revenue:saas, 12 months |
```

Choosing a catalog item fills the row, account included. The account is written into the invoice, so changing the catalog later doesn't change invoices already written. Rows that aren't in the catalog are fine.

## Snippets

A snippet is a note with `type: snippet` and markdown in its body. `snippet_kind` sorts it as `payment`, `terms` or `note`.

```markdown
---
title: Net 30 ACH
type: snippet
snippet_kind: payment
---

## Payment

Pay by ACH to account 123456789, routing 987654321, within 30 days.
```

Inserting a snippet copies its text into the invoice. A sent invoice keeps the words it was sent with, even after you change the snippet.

## Words the app looks for

Some words are fixed because scripts and mbr rely on them: the frontmatter keys, the `type` values, the `draft`, `sent` and `void` statuses, the form of `paid:`, and the journal's tag names. The words the app recognizes in tables, the customer tag and the contact labels are [settings](settings.md#words).
