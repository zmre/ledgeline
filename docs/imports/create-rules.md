---
title: Creating a rules file
description: "Let Ledgeline draft an hledger CSV rules file from a statement that no existing rules file can read."
order: 20
tags: [imports]
---

# Creating a rules file

hledger needs a rules file to turn a CSV into transactions. When you drop a statement on [New Transactions](new-transactions.md) and none of your existing `*.rules` files fit it, Ledgeline offers to write one for you.

The draft is a starting point. Ledgeline guesses which column holds the date, the description and the amount, shows you every guess, and writes nothing until you press **Create rules file**.

## Start the wizard

Drop the statement. If the **Read it with** list says no rules file fits, press **Create rules file…**. Ledgeline reads the converted CSV and opens the drafting panel below the list.

The button only appears when nothing fits. To build a second rules file for data that an existing one already matches, copy that file in a terminal and edit it in [Edit Rules](rules-editor.md).

<!-- screenshot: Create rules panel with the column mapping table, guessed-column badges, and the "What the file will say" preview -->

## Fill in the draft

### File name

The name defaults to your CSV destination with `.rules` added, so `bank.csv` gets `bank.csv.rules`. hledger finds a rules file by that naming convention, so keep the pair together. The name is relative to your journal's folder, must end in `.rules`, and can't be hidden or step outside that folder.

### This file's account and fallback category

**This file's account** is the account the statement belongs to, such as `assets:bank:checking`. Every imported row has one posting there, and Ledgeline writes it as `account1`. You must fill it in, unless one of the columns is already mapped to `account1` because the statement names a different account on each row.

**Fallback category** is where a row goes when no rule matches it. It is written as `account2`. Something obvious like `expenses:unknown` makes the unsorted rows easy to find later.

### Currency

The currency is added in front of every amount. Fill it in when your file has bare numbers, so the imported amounts add up with the `$` amounts already in your journal. Leave it blank when the amounts already carry a symbol. hledger adds the prefix blindly, so `$` in front of `$-4.50` gives `$$-4.50`, which hledger treats as a different commodity.

### Which column is which

The mapping table lists every column with its header, an example value, and the hledger field Ledgeline picked. Columns it guessed with less confidence carry a badge, and the panel says how many need checking. Change a field name by typing in its box, or clear it to skip that column.

Every import needs a `date` and an amount. That is either `amount`, or `amount-in` and `amount-out` for banks that use two money columns. Any other name labels the column for later rules. A column named `cat` can be used as `comment category:%cat`.

Columns Ledgeline can't identify keep their own header as the name. An OFX file's `fitid` column stays available that way. Add `comment id:%fitid` later to make [re-downloads safe](new-transactions.md#re-downloading-a-statement).

## Warnings worth reading

hledger accepts many mistakes in a rules file without complaint, so Ledgeline lists the ones it can detect in your data:

- it couldn't recognise the date format, or the dates could be read two ways (`01/02/2026` is 2 January or 1 February);
- no column looks like a date, or none looks like an amount;
- the file has both a single amount column and in/out columns, and only one set was mapped;
- only one of the two money columns was found, which would give every row the same sign;
- a running-balance column was mapped to `balance`, which makes hledger check every row and fails the import if your history for that account is incomplete;
- the amounts disagree about the decimal mark, or carry more than one currency symbol;
- no currency could be found for bare numbers.

## Create the file

**What the file will say** shows the lines Ledgeline will write. Press **Create rules file** to save it beside your journal. Ledgeline then reads your statement again, and the new file should appear in the **Read it with** list ready to choose.

If the new file still doesn't match the statement, the list says so. Open it in [Edit Rules](rules-editor.md) and check the column mapping.

Rows that no rule matches land in your fallback category. Add `if` rules for them in [Edit Rules](rules-editor.md) once the file exists.
