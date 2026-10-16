---
title: New transactions
description: "Drop a statement, pick the rules file that reads it, check the preview, and write the transactions into your journal."
order: 10
tags: [imports]
---

# New transactions

The New Transactions tab takes one statement file and walks it through to your journal. Nothing is written until you have seen exactly what hledger proposes and pressed **Write changes**.

## Drop a statement

Drag a file onto the dashed panel, or press **Choose file…** to pick one. Ledgeline takes one file at a time, because one import is one statement going into one journal. A file in a format it can't read, such as a PDF, is refused with a sentence saying why.

Ledgeline converts the file to a CSV and shows a short preview of the rows it read. It also reports anything it had to guess, such as the delimiter, the text encoding, or which sheet of a workbook it read. If the statement's own totals don't match its rows, it says so, which usually means a column was misread.

If the file is a QuickBooks Online Journal export, a different panel takes over. See [Importing from QuickBooks](quickbooks.md).

<!-- screenshot: New Transactions after a drop, showing the preview table and the "Read it with" candidate cards with scores -->

## Choose a rules file

Under **Read it with**, Ledgeline lists the `*.rules` files beside your journal that fit this data, best match first. Each card shows a score, the signals behind it (for example, whether the columns and dates line up), and a sample of the transactions that file would produce. Pick the one whose sample looks right.

The last option, **Don't import, just save the CSV**, writes the converted file and leaves your journal alone.

If no rules file fits, the list says so and offers **Create rules file…**. That drafts a new one from your data. See [Creating a rules file](create-rules.md).

## Where it goes

The **Where it goes** box has two fields.

- **CSV file** is where the converted CSV is saved, relative to the folder your journal is in. hledger keeps its import state, a `.latest` file, beside this CSV, so use the same name each time you import from the same account.
- **Import into** is the journal file the transactions are appended to. Ledgeline lists the journal files it can see and can write to. In a journal split across `include`d files, pick the file for the year or account you are importing.

Balances are always checked against the whole journal you opened, not only the file you are writing to. [Two journals](../development/imports-internals.md#two-journals--the-one-we-write--and-the-one-we-reckon-against) explains why that matters for split journals.

## Check the statement balance

The **Statement balance** box is optional. Type the closing balance from your statement and the account it belongs to. OFX files often include the balance, and then Ledgeline fills it in for you.

Ledgeline checks the balance against your journal plus the new transactions. It reports "The balance reconciles" or says how far off it is. Tick **Write it into the journal as a balance assertion** if you want hledger to re-check that balance every time it reads the file.

## Preview before writing

Press **Save and Import** to run a dry run. hledger works out what it would add and Ledgeline shows the result under "Nothing has been written yet". The panel shows:

- hledger's own status line, and the proposed transactions exactly as they will be appended;
- a warning if hledger would silently drop rows dated on or before the last import from this CSV;
- the balance check, if you gave a balance;
- any accounts your [account aliases](../settings/account-aliases.md) would rename, measured by running the import with and without them;
- a notice if a plain `hledger import` from a terminal would produce different account names, with a one-click fix that writes the matching `--alias` lines to `hledger.conf`;
- rows matched by id, described in [re-downloading a statement](#re-downloading-a-statement);
- a `ledgeline import …` command that repeats this import from a terminal.

If hledger rejects the import, the panel shows hledger's error output unedited. It usually names the row it failed on.

<!-- screenshot: dry-run panel with the proposed transactions, the balance verdict and the Write changes button circled -->

## Write the changes

Press **Write changes**. Ledgeline appends the text you just saw, unaltered, to the journal you chose. It then tells hledger to record the import so the same rows aren't proposed again. If that last step fails, Ledgeline restores the journal to how it was and reports the whole import as failed.

The result panel lists every file it wrote.

### The git safety net

If your journal is in a git repository, an import refuses to start while a file it will write has uncommitted changes. It names those files, so you can commit them first. Once the import is written, Ledgeline commits the CSV and the journal with a generated message. `git diff` and `git revert` then undo an import cleanly.

Ledgeline only stages the files the import wrote. Other work in progress, including files you have already staged, is left alone. If the commit itself fails, for example because of a pre-commit hook or a signing prompt, the import is still written and the result panel shows git's message.

### Re-sorting by date

hledger appends at the end of the file. If that leaves the journal out of date order, the result panel lists the transactions that would move and offers to re-sort the file. Re-sorting moves only transactions. Directives, `include` lines and comments stay where they are.

## Re-downloading a statement

hledger de-duplicates imports by date. It remembers the newest date it imported from a CSV and skips every row on or before it. That works for statements that only ever grow, but not when you download the same period twice. A card charge that first appeared as a pending hold and later settled would never be updated.

Ledgeline can match rows by the bank's own transaction id instead. Add this line to the rules file:

```
comment id:%fitid
```

OFX, QFX and QBO files carry a `FITID` for every transaction, and Ledgeline exposes it as the `fitid` column. With that line in place, every imported transaction carries an `id:` tag, and the dry run sorts each incoming row into one of four outcomes.

| Outcome | What happens |
| --- | --- |
| New | No transaction in the journal has this id. It is imported as usual. |
| Unchanged | The journal already has it, identically. It is skipped. |
| Status changed | Only the cleared status differs, such as a pending hold that has since cleared. Ledgeline updates the `*` or `!` mark on the existing transaction. |
| Conflicting | Something else differs, such as the amount. The row is reported with the differences and never imported or edited. |

The dry run lists the status changes and conflicts before you write anything. A conflict usually means you edited the transaction yourself, so Ledgeline leaves your version alone. Notes you add to a transaction's comment don't count as a difference.

Status changes only apply when the rules file sets a `status` field, and only to transactions in the file you are importing into. Rules files without an `id:` line keep hledger's date-based behaviour exactly. [Matching a re-download by row id](../development/imports-internals.md#matching-a-re-download-by-row-id) has the details.

## Doing it from a terminal

The dry-run panel shows a `ledgeline import` command for the import on screen. Run it from the folder that holds your journal to repeat the same import later with a new statement. `ledgeline import --help` lists the options.
