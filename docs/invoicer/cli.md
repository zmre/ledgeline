---
title: Invoicer command line
description: "Reference for the ledgeline-invoicer command: every subcommand and option, how it finds your books, JSON output, and exit codes."
order: 70
tags: [invoicer]
---

# Command line

`ledgeline-invoicer` is the app and a command-line tool in one. With no command it opens the app's window. With a command it does that one job and exits, which makes it easy to use from scripts. Everything the command line does goes through the same code as the app, and writes the same files.

```text
ledgeline-invoicer [OPTIONS] [JOURNAL] [COMMAND]
```

## Finding your books

Every command needs your books' main journal. It looks, in order, at:

1. `JOURNAL` before the command, or `-f JOURNAL` anywhere on the line;
2. the `LEDGELINE_FIXTURE` environment variable;
3. the journal you opened last in Ledgeline or the invoicer.

In the last two cases it prints which journal it used on the error output. It never reads `LEDGER_FILE`, so a shell set up for hledger doesn't send it to the wrong books.

```sh
ledgeline-invoicer ~/books/books.journal list
ledgeline-invoicer list -f ~/books/books.journal
```

## Options for every command

| Option                 | Does                                                                                                             |
|------------------------|------------------------------------------------------------------------------------------------------------------|
| `-f`, `--file JOURNAL` | The books' main journal. The same as `JOURNAL`, but it can go after the command.                                 |
| `--json`               | Print one JSON value instead of text. See [JSON output](#json-output).                                           |
| `--today DATE`         | Treat `DATE` (`YYYY-MM-DD`) as today: the default date of a new invoice, and what "overdue" is measured against. |
| `-h`, `--help`         | Help for the tool or a command: `ledgeline-invoicer send --help`.                                                |
| `-V`, `--version`      | The version.                                                                                                     |

## Naming an invoice

A command that works on one invoice takes its number (`105`) or its file. A file can be a path from the current folder, or a path from the books folder with or without `.md`.

## Commands

| Command         | Does                                                                           | Writes                   |
|-----------------|--------------------------------------------------------------------------------|--------------------------|
| `new`           | Create a draft invoice                                                         | an invoice file          |
| `list`          | List invoices, newest first                                                    | nothing                  |
| `show`          | Show one invoice: its fields, its computed table, its problems                 | nothing                  |
| `set`           | Set or remove fields in an invoice                                             | the invoice file         |
| `recalc`        | Rewrite bill-table amounts that don't add up                                   | invoice files            |
| `check`         | Check every invoice, customer and the catalog against each other and the books | nothing                  |
| `next-number`   | Print the next invoice number                                                  | nothing                  |
| `config`        | Print the settings the app is using                                            | nothing                  |
| `customers`     | List customers, or show one                                                    | nothing                  |
| `send`          | Book an invoice and mark it sent                                               | the journal and the file |
| `void`          | Reverse an invoice's booking and mark it void                                  | the journal and the file |
| `rebook`        | Re-book a sent invoice after an edit                                           | the journal              |
| `apply-payment` | Cash books: apply a deposit to an invoice, or undo it                          | the journal and the file |
| `reconcile`     | Update each sent invoice's paid status from the books                          | invoice files            |
| `pdf`           | Export an invoice's PDF                                                        | a PDF                    |
| `receivables`   | Open invoices, aging, DSO and average days to pay                              | nothing                  |
| `recognize`     | Post the recognition entries due                                               | the journal              |
| `time`          | Time tracking: the tracker, entries, billing time and paying for it. See [Tracking time](../time/tracking.md#the-command-line) and [Billing time](../time/billing.md#from-the-command-line). | time files, drafts, the journal |

### new

Create a draft. The books aren't touched.

```sh
ledgeline-invoicer new --customer Contoso \
  --row 'Strategy workshop|1|$2,500' \
  --row 'Discount||-10%' \
  --from-catalog WKSHP:2 \
  --dry-run
```

| Option                      | Does                                                                                                                                              |
|-----------------------------|---------------------------------------------------------------------------------------------------------------------------------------------------|
| `--customer NAME`           | Required. The customer's note title, another name it goes by, or a link to it.                                                                    |
| `--template PATH`           | The template, as a path from the books folder: `/invoices/tpl/classic.html`. Default: the customer's, else the first in your books, else Classic. |
| `--number N`                | The number. Default: the next one.                                                                                                                |
| `--date DATE`               | The issue date. Default: today.                                                                                                                   |
| `--terms DAYS`              | Net days. Default: the customer's.                                                                                                                |
| `--reference TEXT`          | What the customer quotes back, such as `"PO #105"`.                                                                                               |
| `--row SPEC`                | A row: description, quantity, rate and account, separated by `\|`. Repeatable. See below.                                                         |
| `--from-catalog CODE[:QTY]` | A row from the catalog. Repeatable.                                                                                                               |
| `--field KEY=VALUE`         | A field of your own. `KEY:=JSON` writes any value. Repeatable.                                                                                    |
| `--dry-run`                 | Print the draft instead of writing it.                                                                                                            |

Rows keep the order you give them, across `--row` and `--from-catalog`. In `--row`:

- Fields at the end can be left off: `'Consulting|10|$150'` uses the default account.
- With no quantity, a percentage is a discount or tax on the row above: `'Discount||-10%'`.
- With no quantity, a money rate is the row's amount: `'Less deposit||-$5,000|liabilities:customer-deposits'`.
- A total word alone is a total row: `'Subtotal'`.
- `\|` is a literal `|` in a description.

If the template is a starter your books don't have yet, `new` copies it in.

### list

```sh
ledgeline-invoicer list --overdue
```

| Option            | Lists                                               |
|-------------------|-----------------------------------------------------|
| `--status STATUS` | Only `draft`, `sent` or `void`.                     |
| `--customer NAME` | Only this customer's: a title, link or billed name. |
| `--type TYPE`     | Only `invoice`, `estimate` or `credit-note`.        |
| `--open`          | Only open invoices: sent and not paid in full.      |
| `--overdue`       | Only overdue invoices: open and past due.           |

Open and overdue come from the books, so they are right even before the paid status in the files has been refreshed.

### show

```sh
ledgeline-invoicer show 105
```

The invoice's fields, its bill table as the app computes it (with any amount that disagrees), the text around the table, and every problem found with it, the books included.

### set

Set or remove fields, one key at a time. Everything else in the file stays exactly as it was, comments and all.

```sh
ledgeline-invoicer set 108 reference="PO #PY75334" support_tier=gold
ledgeline-invoicer set 108 terms=45
ledgeline-invoicer set 108 --unset support_tier
```

| Form          | Writes                                                                                                    |
|---------------|-----------------------------------------------------------------------------------------------------------|
| `key=value`   | Text. A whole number is written as a number, so `terms=45` is a number but `invoice_num=0042` stays text. |
| `key:=JSON`   | Any value: `tags:='["rush"]'`.                                                                            |
| `--unset KEY` | Removes the field. Repeatable.                                                                            |

`set` refuses three fields. `status` changes only through `send` and `void`. `paid` is written from the books by `reconcile`. `invoice_num` can change only on a draft, and only to a number not in use.

Setting `terms` also sets `due_date` to the invoice date plus the new terms, unless you give `due_date` in the same command. Changing `date` alone doesn't move `due_date`; set it too.

Setting `template` to a starter your books don't have copies the starter in.

### recalc

```sh
ledgeline-invoicer recalc --dry-run
ledgeline-invoicer recalc 105
```

With no invoices named, rewrites every draft whose amounts don't match its rows, and reports sent and void invoices that don't match without touching them. Name a sent invoice to rewrite it, then `rebook` it. `--dry-run` reports without writing.

### check

```sh
ledgeline-invoicer check
ledgeline-invoicer check --strict
```

Checks every invoice, customer and the catalog, and compares each invoice with its booking. Each problem is printed with its file and line and, where there is one, the command that fixes it. [Booking](booking.md#checking-that-the-books-and-the-files-agree) lists the book problems it finds. Exits 1 on errors; `--strict` exits 1 on warnings too.

### next-number

```sh
ledgeline-invoicer next-number
```

Prints the number the next invoice gets: one more than the highest in your files and your books, or the `number-start` setting when there are none.

### config

Prints the books' `[books]` settings and every `[invoicer]` setting as the app uses them, defaults included, in the same form as `ledgeline.toml`: `default-terms` and `default-template` too, once set. Under them, as comments, it prints the two payment-matching settings in use, `settlementLookbackDays` and `settlementLeadDays`. Those are shared with Ledgeline, so they come from `ui-prefs.toml`, or from `[ui]` in these books' `ledgeline.toml` when the books keep their own. See [In the settings file](settings.md#in-the-settings-file).

```sh
ledgeline-invoicer config
```

The Settings page in the app changes the same settings; `config` shows what it saved.

### customers

```sh
ledgeline-invoicer customers list
ledgeline-invoicer customers show "Customer X"
```

`list` prints every customer with their kind, terms, billing email and portal, how many invoices they have and how many are open. `show` adds the contact details, accounts, template, contracts (and whether each file exists), portal, open purchase orders with what is left of each, snippets and invoices.

#### Adding customers

`customers new` and `customers import-csv` create customer notes in your customers folder, exactly as **Add customer** in the app does: the same details make the same file.

```sh
ledgeline-invoicer customers new "Northwind Traders" --email ap@northwind.example --terms 30
```

| Flag                                                                                                                              | Writes                                                                             |
|-----------------------------------------------------------------------------------------------------------------------------------|------------------------------------------------------------------------------------|
| `NAME` (required)                                                                                                                 | The customer's name: what invoices link to, and the books' customer                |
| `--kind person\|organization`                                                                                                     | Person or organization (default organization)                                      |
| `--email`, `--phone`, `--address`                                                                                                 | The billing email, phone and address. Put line breaks in an address with `$'…\n…'` |
| `--company ORG`                                                                                                                   | A person's organization                                                            |
| `--terms DAYS`                                                                                                                    | Net days                                                                           |
| `--revenue-account ACCOUNT`                                                                                                       | The revenue account for this customer's lines                                      |
| `--invoice-template PATH`                                                                                                         | The customer's invoice template, such as `/invoices/tpl/x.html`                    |
| `--ledger-customer NAME`                                                                                                          | The name the books use, when it differs                                            |
| `--contract LINK\|PATH`                                                                                                           | A contract; repeat for more                                                        |
| `--portal-name`, `--portal-url`, `--portal-login`, `--portal-supplier-id`, `--portal-requires-reference`, `--portal-instructions` | The billing portal. The login is a username, never a password                      |

A name any note already has is refused, whatever its case, and an existing file is never overwritten. `--dry-run` prints the note without writing it.

```sh
ledgeline-invoicer customers import-csv customers.csv --dry-run
ledgeline-invoicer customers import-csv customers.csv
```

The first row names the columns and each row after it is one customer:

```text
name,kind,email,address,terms
"Smith, Jones & Co",organization,ap@sj.example,"1 Main St
Springfield, CO",30
Zoë Müller,person,zoe@example.de,,14
```

The columns are `name` (required), `kind`, `email`, `phone`, `address`, `company`, `terms`, `revenue_account`, `invoice_template`, `ledger_customer`, `contracts` (several separated by `;`), `portal_name`, `portal_url`, `portal_login`, `portal_supplier_id`, `portal_requires_reference` (yes or no) and `portal_instructions`. They mean what the `customers new` options of the same name mean. Headers match whatever their case, spacing, `-` or `_`, so `Revenue Account` works. Any other header is refused, and the message lists the ones allowed. Blank cells and blank rows are skipped.

Save the file from your spreadsheet as CSV UTF-8. Quoted cells may hold commas and line breaks. Files are limited to 4 MiB and 10,000 rows.

**All or nothing.** Every row is checked before anything is written. If a row doesn't read, repeats another row's customer, or names a customer that already exists, nothing is written and every such row is listed with its line number. To add only the new customers from a list you imported before, use `--skip-existing`: rows naming an existing customer are skipped and reported. `--dry-run` checks every row and lists what would be created.

### send

```sh
ledgeline-invoicer send 108 --dry-run
ledgeline-invoicer send 108 --via email
```

| Option      | Does                                                                      |
|-------------|---------------------------------------------------------------------------|
| `--via HOW` | Record how it was delivered: `email`, `portal`, `mail`, or your own word. |
| `--dry-run` | Show what would be booked, and write nothing.                             |

In accrual books, books the invoice, then marks the file sent. In cash books, only marks the file sent. See [Booking](booking.md#sending).

### void

```sh
ledgeline-invoicer void 106 --date 2026-11-20
```

Reverses the booking on `--date` (default: today) and marks the file void with that date. A draft is only marked void.

`--time release` (the default) takes the invoice's `invoice:` tags off its tracked time, so the time is unbilled again. `--time nobill` writes the time off instead. See [Releasing time](../time/billing.md#releasing-time).

### rebook

```sh
ledgeline-invoicer rebook 105 --dry-run
ledgeline-invoicer rebook 105
```

Replaces a sent invoice's booking with one that matches the file. Does nothing when they already agree. `--dry-run` shows what is booked and what would be. Refused for an invoice in a closed period.

### apply-payment

Cash books only.

```sh
ledgeline-invoicer apply-payment 207
ledgeline-invoicer apply-payment 207 --txn REF
ledgeline-invoicer apply-payment 207 --undo
ledgeline-invoicer apply-payment 207 --lookback-days 365
```

| Option                 | Does                                                                                                                                   |
|------------------------|----------------------------------------------------------------------------------------------------------------------------------------|
| (none)                 | List the deposits in the books that might pay the invoice, best first, each with its reference.                                        |
| `--txn REF`            | Apply that deposit. Its first six or more characters are enough.                                                                       |
| `--undo`               | Undo a payment applied to this invoice, restoring the deposit exactly as it was. With `--txn`, the one to undo when there are several. |
| `--lookback-days DAYS` | How many days back to look for deposits, 0 to 3650. 0 looks back forever.                                                              |
| `--lead-days DAYS`     | How many days before the invoice a payment may come, 0 to 60.                                                                          |

Without the last two, `apply-payment` uses the same [payment-matching settings](settings.md#payment-matching-shared-with-ledgeline) as the app and Ledgeline: these books' own, else the shared ones, else 180 days and 4 days. A deposit short of the invoice is always offered as a part payment. A flag changes them for that one run and saves nothing.

Applying writes an undo record under `.ledgeline-invoicer/undo/` in the books folder and refreshes the invoice's paid status. See [Getting paid](booking.md#cash-books).

### reconcile

```sh
ledgeline-invoicer reconcile --dry-run
ledgeline-invoicer reconcile 105 107
```

Updates the paid status of every sent invoice, or the ones named, from the books, and reports where the books and the files disagree. A file that changed while it ran is left for the next run. `--dry-run` reports without writing. Exits 1 on errors and 2 when a file couldn't be written. The app does this by itself whenever your books change.

### pdf

```sh
ledgeline-invoicer pdf 105
ledgeline-invoicer pdf 105 -o ~/Desktop/invoice-105.pdf
```

Exports the PDF through the invoice's template. A sent invoice's PDF is kept in the PDF folder (`invoices/pdf/` by default), named like the invoice; an earlier export of the same invoice is kept with its date added to the name. A draft's PDF goes to a temporary file.

`-o PATH` writes a copy to `PATH` instead, and keeps nothing in the PDF folder. The folder must exist.

`pdf` prints through the system's web view, so it needs the desktop build and a desktop session. A build without the window (`--no-default-features`) refuses it.

### receivables

```sh
ledgeline-invoicer receivables --aging
```

| Option            | Does                                                                         |
|-------------------|------------------------------------------------------------------------------|
| `--aging`         | Add the aging table to the text. `--json` always includes it.                |
| `--dso-days DAYS` | The days sales outstanding period. Default: the `dso-days` setting, else 90. |

In accrual books, lists every receivable still owed, including any without an invoice file, with days late, aging, DSO and average days to pay. In cash books it lists sent invoices less the payments applied. Aging uses the fixed ranges Current, 1–30, 31–60, 61–90 and 91+.

### recognize

```sh
ledgeline-invoicer recognize --dry-run
ledgeline-invoicer recognize --through 2026-10-31
```

| Option             | Does                                                                                     |
|--------------------|------------------------------------------------------------------------------------------|
| `--through DATE`   | Post the entries dated on or before `DATE`. Default and latest: today.                   |
| `--include-closed` | Also post the entries due in closed periods, as one adjustment per schedule dated today. |
| `--dry-run`        | List what is due, and write nothing.                                                     |

Posts the scheduled revenue and expense entries that Ledgeline's **Post through** would. See [Revenue and expense recognition](../journal/accrual-recognition.md).

## Running the app

With no command, `ledgeline-invoicer` opens the window on the journal it finds. These options apply only then:

| Option                  | Does                                                                      |
|-------------------------|---------------------------------------------------------------------------|
| `-s`, `--server`        | Run without a window, serving the app on a local port (5199 by default).  |
| `--host HOST`           | The address to listen on. Default: `127.0.0.1`.                           |
| `--port PORT`           | The port. Default: 5199 with `--server`, any free port for the window.    |
| `--allow-origin ORIGIN` | For development only: let a page served from `ORIGIN` call the app's API. |

## JSON output

With `--json`, every command prints exactly one JSON value on standard output and nothing else there. Paths are from the books folder. Amounts in the books commands are plain decimals, such as `"8350.00"`, next to their currency. Field names are only ever added, never renamed, so a script keeps working across versions.

A failure prints an object with `error`, and for the books commands a `code`:

```json
{"error": "`paid` can't be set: paid is a mirror of the books, written only by the reconciler"}
```

The codes are `not-found`, `ambiguous`, `invalid`, `booking`, `status`, `setup`, `refused`, `external-change`, `stale`, `books`, `markdown-not-written`, `markdown`, `frontmatter`, `storage`, `overflow` and, from `pdf`, `needs-gui`.

## Exit codes

| Code | Means                                                                                                                  |
|------|------------------------------------------------------------------------------------------------------------------------|
| 0    | Done.                                                                                                                  |
| 1    | `check` or `reconcile` found errors (with `check --strict`, warnings too).                                             |
| 2    | The command couldn't run: a bad option, books that don't open, an invoice that isn't there, a refused or failed write. |

## When the books change underneath

Every write to the journal checks that the journal and the file are still what it read. If Ledgeline or an editor changed them in between, nothing is written; the command reads them again and tries once more, and says so on the error output.
