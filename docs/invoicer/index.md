---
title: Invoicer
description: "Ledgeline Invoicer: invoices kept as plain-text notes beside your hledger books, booked into the same journal Ledgeline reads, with PDF export, a receivables dashboard and a command line."
order: 80
tags: [invoicer]
---

# Ledgeline Invoicer

Ledgeline Invoicer writes your invoices, turns them into PDFs, and books them into the hledger journal that Ledgeline reads. It is a separate app from the same project. It opens the same books folder and uses the same theme.

Every invoice is a markdown file in your books folder, and so is every customer. You can open them in any text editor, keep them in git, or search them with `grep`. The money side lives in the journal as ordinary hledger transactions, so `hledger` and Ledgeline report on your invoices without any extra step.

## What it does

- Writes invoices with a bill table that does its own arithmetic: quantities, rates, discounts, subtotals, sales tax and deposits.
- Keeps drafts out of the books. An invoice touches the journal only when you send it.
- Books a sent invoice the way your books work: in accrual books, a receivable and revenue on the invoice date; in cash books, revenue when the payment arrives.
- Notes on each invoice whether it has been paid, partly paid or not, by reading the payments in your books.
- Exports a PDF through a template you can change, and keeps a copy of every final PDF it exports.
- Shows what you are owed on a [dashboard](dashboard.md): what is overdue, how old it is, and what came in recently.
- Tracks your time in hledger timedot files and bills it. See [Time tracking](../time/index.md).
- Does almost everything from the [command line](cli.md) too, so you can [script it](scripting.md).

## Getting it

**macOS.** Download `LedgelineInvoicer-<version>-arm64.dmg` from the [latest GitHub release](https://github.com/zmre/ledgeline/releases/latest), open it and drag **Ledgeline Invoicer** to Applications. It is signed and notarized. It runs on Apple silicon Macs only; there is no Intel build.

**Nix (macOS and Linux).** Installing Ledgeline from the flake installs the invoicer too, as the app and the `ledgeline-invoicer` command:

```sh
nix profile install --accept-flake-config github:zmre/ledgeline
```

To run it without installing:

```sh
nix run --accept-flake-config github:zmre/ledgeline#invoicerWithSpa -- ~/books/books.journal
```

The invoicer isn't on crates.io. Without Nix, build it from the [Ledgeline repository](https://github.com/zmre/ledgeline) inside its development shell:

```sh
just build-invoicer                                   # the app's screens
cargo run -p ledgeline-invoicer -- ~/books/books.journal
```

It doesn't need hledger installed: it reads and writes the journal itself.

## Opening your books

Give the app your books' main journal, the same file you open in Ledgeline. The folder that journal is in is your books folder, and everything the invoicer reads or writes stays inside it.

With no journal named, the app reopens the books you used last in either app. **File → Open…** picks another journal and **File → Open Recent** lists the ones you've had open. The list is shared with Ledgeline.

The invoicer shares its theme and text size with Ledgeline: change them in either app's **Settings → Appearance**. A books folder pinned to a theme opens in that theme in both apps.

## Getting around

The sidebar has six sections, and a **Time** group below them:

| Section   | What it holds                                                                                     |
|-----------|---------------------------------------------------------------------------------------------------|
| Dashboard | What you are owed, what is overdue, what was paid recently. See [Dashboard](dashboard.md).        |
| Invoices  | Every invoice, with filters for status, customer, dates and how late it is.                       |
| Customers | The people and organizations you bill, what each owes, and their open purchase orders.            |
| Catalog   | Products and services you bill often, with their rate and the account each one books to.          |
| Snippets  | Reusable text such as payment instructions and terms, copied into an invoice when you insert one. |
| Templates | How invoices look on screen and on paper. See [Templates](templates.md).                          |
| Time      | Time tracking: Dashboard, Entries, Track, Accounts and Unbilled. See [Time tracking](../time/index.md).     |

**Settings**, at the foot of the sidebar, holds these books' settings and the appearance. <kbd>g</kbd> <kbd>,</kbd> or <kbd>Cmd</kbd>+<kbd>,</kbd> opens it too. See [Settings](settings.md).

The **New** button at the top of the sidebar starts a new invoice. Its menu also offers a new customer, catalog item, snippet or template. Press <kbd>?</kbd> anywhere to see the [keyboard shortcuts](keyboard.md) for the page you are on.

Web and email links in notes and invoices open in your browser and mail app.

## An invoice's life

1. **Draft.** You pick a customer and a template, fill the bill table, and save. The app copies the customer's billing name, address, email and payment terms into the invoice, so later changes to the customer don't change it. Nothing is booked.
2. **Sent.** **Send…** marks the invoice sent and books it. You can export the PDF at the same time. If the customer takes invoices through a vendor portal, the send step shows the portal's instructions and a button to open it.
3. **Paid.** You record the payment the way you always do in Ledgeline, by importing or syncing your bank. The invoicer sees the payment in the books and updates the invoice to show it paid, partly paid or unpaid.
4. **Void**, if it was a mistake. **Void…** reverses the booking on a date you choose and marks the invoice void. The file stays.

A sent invoice can still be edited. Saving it re-books it to match, unless its date falls in a period you have closed in Ledgeline. [Booking](booking.md) explains each step in detail, in both accrual and cash books.

## In this section

- [Files and formats](files.md): the invoice, customer, catalog and snippet files, field by field, and the bill table's rules.
- [Booking](booking.md): what lands in the journal and when, deposits, voids, and how the paid status is kept up to date.
- [Templates](templates.md): changing how invoices look, for people who don't write HTML.
- [Dashboard](dashboard.md): the receivables dashboard and the settings that shape it.
- [Settings](settings.md): the Settings page, and every invoicer setting in `ledgeline.toml` with its default.
- [Keyboard shortcuts](keyboard.md): every key, page by page.
- [Command line](cli.md): every command and option.
- [Scripting recipes](scripting.md): `grep`, `hledger` and the command line working on your invoices.
- [Time tracking](../time/index.md): tracking hours and billing them, in its own section.
