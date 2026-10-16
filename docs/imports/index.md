---
title: Imports
description: "Bring bank, card and QuickBooks statements into your hledger journal, and manage the CSV rules files that read them."
order: 20
tags: [imports]
---

# Imports

The Imports screen turns a statement you downloaded into transactions in your journal. It uses hledger's own `hledger import` and your existing `*.rules` files, so an import done here gives the same result as one run from a terminal.

Imports has two tabs. Press <kbd>1</kbd> or <kbd>2</kbd> to switch between them.

- [New Transactions](new-transactions.md) is where you drop a statement file, choose the rules file that reads it, preview the proposed transactions and write them.
- [Edit Rules](rules-editor.md) lists the `*.rules` files beside your journal and lets you edit their settings, column mapping, default accounts and `if` rules.

Account aliases used to be a third tab here. They now live under [Settings → Account Aliases](../settings/account-aliases.md), and an old bookmark to the Imports tab is forwarded there.

<!-- screenshot: Imports screen with the New Transactions tab selected and the empty drop target -->

## What you need first

Imports runs hledger, so Ledgeline needs hledger 1.40 or newer. If it can't find one, or finds one that is too old, the New Transactions tab shows a banner where you can type the path to the binary. The rest of the tab stays hidden until hledger works.

Ledgeline also needs a journal file it can write to. If it was started without one, New Transactions explains that there is nowhere for an import to go, and Edit Rules still works read-only.

## Supported formats

Ledgeline converts whatever you drop into one CSV, then hands that CSV to hledger with the rules file you choose.

| Format | Extensions | Notes |
| --- | --- | --- |
| Delimited text | `.csv`, `.tsv`, `.ssv` | Ledgeline guesses the delimiter and the text encoding and tells you when it guessed. |
| OFX family | `.ofx`, `.qfx`, `.qbo` | Each transaction's `FITID` becomes a `fitid` column, which makes re-downloads safe. See [re-downloads](new-transactions.md#re-downloading-a-statement). |
| Spreadsheets | `.xls`, `.xlsx`, `.xlsm`, `.xlsb`, `.ods` | Ledgeline reads one sheet and says which. |
| QuickBooks Online Journal report | `.xlsx` | Detected from the file's contents and handled by its own panel. See [Importing from QuickBooks](quickbooks.md). |

PDF statements are refused. The numbers in a PDF aren't reliably where they appear to be, so Ledgeline asks you to download CSV, OFX/QFX or a spreadsheet from your bank instead. The research behind that decision is in [PDF extraction](../development/pdf-extraction.md).

## What Ledgeline will not do

There is no raw text editor for rules files or journals. The Edit Rules tab covers the settings, the column mapping, the two default accounts and an ordered list of `if` rules. Anything it can't model safely is shown read-only and saved byte for byte. For everything else, open the file in your editor.

Ledgeline also never runs a rules file's `source ... | COMMAND` line, because that would let a file run arbitrary programs. [Security](../development/imports-internals.md#security) in the developer notes explains the full set of guards.

## Pages in this section

- [New transactions](new-transactions.md): drop a statement, check the preview, write it.
- [Creating a rules file](create-rules.md): let Ledgeline draft a rules file for a statement that has none.
- [Editing rules](rules-editor.md): what the rules editor can change and what it leaves alone.
- [Importing from QuickBooks](quickbooks.md): bring in a QuickBooks Online Journal export and map its accounts.

How the import pipeline works internally, and why, is in [Imports internals](../development/imports-internals.md).
