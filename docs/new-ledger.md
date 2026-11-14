---
title: Starting a new ledger
description: "File → New: start a new set of hledger books from a personal, business or empty chart of accounts, or from another app's export. What the wizard asks, the files it writes, and the sample data."
order: 5
tags: [account-tags]
---

# Starting a new ledger

**File → New…** (<kbd>Cmd</kbd>+<kbd>N</kbd> on macOS, <kbd>Ctrl</kbd>+<kbd>N</kbd> on Linux) opens the New ledger wizard. It sets up a new set of books in a folder you choose, then opens them. The first time you start Ledgeline, with no journal to reopen, it opens on this wizard.

Nothing is written until the last step. **Cancel** on the first step goes back to the journal you had open.

The wizard needs the desktop app, because it uses the system folder picker. A browser connected to `ledgeline --server` shows the wizard but can't create anything.

## Start fresh

1. **Chart.** Pick a starting chart of accounts:
   - **Personal**: bank accounts, cash, investments, a credit card, loans, income, and everyday spending.
   - **Business**: bank accounts, receivables, inventory, fixed assets, payables, sales tax, payroll, owner's equity, revenue, and common business expenses.
   - **Empty**: no accounts. Add your own in [Settings → Accounts](settings/accounts.md), or let an import bring them in.
2. **Currency.** Pick the base currency, or **Other…** for any currency code. The preview shows how amounts will look. Tick the decimal comma box to write `1.000,00` instead of `1,000.00`.
3. **Accounts.** Review the chart. Rename an account in place, or remove one you won't use. Subaccounts take their parent's type. You can add accounts and change tags later in Settings → Accounts.
4. **Folder.** Press **Choose folder…** and pick an empty folder, or make a new one in the picker. A folder that holds only hidden files, such as `.DS_Store` or `.git`, counts as empty. Give the ledger a name; it starts as the folder's name. Then press **Create ledger**.

Ledgeline opens the new journal and adds it to **File → Open Recent**.

## Start from another app

Pick the app you're coming from, such as Quicken, Banktivity, Moneydance, GnuCash, Xero, QuickBooks Online, YNAB, Monarch or Copilot, or a bank statement. The wizard says where that app keeps its export. Then choose the exported file.

The currency and folder steps are the same as above. Ledgeline creates an empty ledger, then opens the [Imports](imports/index.md) screen with your file already loaded. There you map the export's accounts and categories to hledger accounts, then choose what to write. Since the new ledger has no accounts yet, Ledgeline suggests a name for each one from the export itself (a bank register becomes `assets:bank:…`, a spending category `expenses:…`), so most of the work is checking its suggestions. See [Importing a QIF file](imports/qif.md), [Importing from other apps](imports/other-apps.md) and [Importing from QuickBooks](imports/quickbooks.md) for what each export brings over.

A bank statement (OFX, QFX or CSV) goes through the ordinary statement import instead, with a rules file. If Ledgeline can't read the file at all, the wizard says so. It still creates the ledger, and you can import the file later.

## Try with sample data

The first step has a **Try with sample data** link. It opens a copy of the sample journal used throughout this manual: a household with bank accounts, a brokerage account, a mortgage, and a few years of transactions. The copy lives in Ledgeline's cache folder, and each click starts from a fresh copy, so feel free to change it. Its dates are moved forward by whole months so that its newest transactions fall in the current month, and the journal's default view (the last 90 days) opens on data.

## The files the wizard writes

Two files, both plain hledger journals:

`main.journal` is the file Ledgeline opens. Its first line is the ledger's name, which Ledgeline shows at the top of the window. It sets the base currency and includes the chart of accounts. New transactions go at the end of it.

```journal
; Acme Books
;
; Created by Ledgeline's New ledger wizard. The chart of accounts is in
; accounts.journal; transactions go at the end of this file.

decimal-mark .
commodity $1,000.00
D $1,000.00

include accounts.journal

; Transactions
```

`accounts.journal` declares every account with its `type:`, and a few accounts with the report tags Ledgeline reads:

```journal
account assets  ; type: A
account assets:bank  ; type: C, bsgroup: Cash and cash equivalents
account assets:bank:checking
account assets:receivable  ; bsgroup: Accounts receivable, kpi: receivable
account liabilities:loans  ; bsterm: noncurrent, bsgroup: Long-term debt
account transfers  ; type: A, clearing:
account revenue  ; type: R, issection: revenue
account expenses:cost-of-goods-sold  ; issection: cogs
```

Every top-level account has a declared type, so reports classify the accounts correctly even if you rename them into another language. See [Accounts](settings/accounts.md#account-types).

The `transfers` account is a clearing account for moving money between your own accounts. See [Transfer matching](journal/transfer-matching.md). The business chart's `kpi:` tags turn on the [startup metrics](reports/startup-metrics.md) on Insights. Remove them if you don't want those figures.

The wizard never writes over a file. It refuses a folder that holds anything other than hidden files. It doesn't create `ledgeline.toml` either; see [Settings files](settings/settings-files.md).

Both files work with the `hledger` command: `hledger -f main.journal check` passes on a new ledger.
