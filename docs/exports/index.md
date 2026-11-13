---
title: Exports
description: "Export your hledger journal as QIF or as a posting-row CSV, to move it into another app or keep a copy outside Ledgeline."
order: 25
tags: [exports]
---

# Exports

Ledgeline can write your journal in two formats other apps read. Both export the journal that is open, including every file it `include`s.

- [QIF](qif.md) is for personal-finance apps such as Banktivity, Quicken, Moneydance and GnuCash. It carries accounts, categories, splits and transfers, but it is single-entry and has no currency field, so some things change on the way out. Ledgeline reports each change.
- [CSV (posting rows)](csv.md) writes one row per posting in the same columns as `hledger print -O csv`. It keeps every posting of every transaction, so no transaction loses a leg.

In the desktop app, use **File → Export**. On the command line, use `ledgeline export`:

```sh
ledgeline export --qif out/ -j main.journal
ledgeline export --csv books.csv -j main.journal
```

Neither command overwrites a file that already exists. Add `--force` to replace it.

Both exports apply your `=` auto-posting rules first, as `hledger print --auto` does. Neither format can hold the rules, and without their postings the exported totals would not match your reports.

To export the transactions you are looking at as an Excel workbook, see [Exporting the journal](../journal/export.md).
