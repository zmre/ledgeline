---
title: Exporting to CSV (posting rows)
description: "Write the journal as one CSV row per posting, in the same columns and format as hledger print -O csv."
order: 20
tags: [exports, csv]
---

# Exporting to CSV (posting rows)

This export writes one CSV row per posting. Rows from the same transaction share a `txnidx` number, so a transaction with any number of postings comes back whole. Ledgeline's own grouped-CSV import reads this format, and so does any tool that reads `hledger print -O csv`.

## Running the export

In the desktop app, choose **File → Export → CSV (posting rows)…** and pick where to save it.

From the command line:

```sh
ledgeline export --csv books.csv -j main.journal
ledgeline export --csv - -j main.journal > books.csv
```

`-` writes the CSV to standard output and the summary to standard error. Ledgeline refuses to replace an existing file unless you add `--force`.

## The format

The output is byte for byte what the bundled hledger 1.52 writes for `hledger print --auto -O csv`. Ledgeline's test suite checks this against hledger on every journal in its fixtures. Ledgeline writes the file itself and does not run hledger.

The columns, in order:

```
txnidx,date,date2,status,code,description,comment,account,amount,commodity,credit,debit,posting-status,posting-comment
```

hledger 1.52 puts `credit` before `debit`. hledger 1.99.4 swapped the two. Ledgeline matches the version it bundles, so read the columns by header name if you can.

- Every field is in double quotes. A `"` inside a field is doubled.
- Rows are in date order, with ties in file order. `txnidx` is the transaction's position in the files, so it is not always ascending.
- `amount` is the number without its commodity symbol or thousands separators, at the precision you wrote it, with the commodity's decimal mark. `commodity` holds the symbol.
- `credit` holds the size of a negative amount and `debit` the size of a positive one. Zero is a debit of `0`.
- A virtual posting's account keeps its brackets, as `(account)` or `[account]`.
- Comments keep their tags. A comment over several lines stays several lines inside the quoted field.
- A posting that balances two commodities gets one row per commodity.

## What is lost

Very little, but some things have no column:

- costs (`@` and `@@`), so a share purchase shows the share count in one row and the cash in another
- balance assertions
- market prices, periodic transactions and budgets, and auto-posting rules (their postings are included)
- account declarations and their tags

Auto-posting rules are applied before the export, as `hledger print --auto` does. If you import the CSV into a journal that still has the rules, they apply a second time. Remove the rules, or import into a fresh folder.
