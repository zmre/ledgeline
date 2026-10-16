---
title: Filtering the journal
description: "Narrow the journal by date range, account and free-text search, and share a filtered view as a link."
order: 10
tags: [journal]
---

# Filtering the journal

The filter bar at the top of the Journal tab decides which transactions the table, the insights panel and the totals footer show. It has three controls: a date range, an account picker and a search box.

<!-- screenshot: filter bar with the date preset menu, account picker and search box circled, plus a row of active filter chips -->

A transaction has to pass all three filters to appear.

## Date range

The date button opens a list of presets:

- This month
- Last month
- Last 90 days
- Year to date
- This year
- Last year
- All time

The two date fields beside it set a custom range. Both ends are inclusive. Clearing a field leaves that end open, so an empty "from" means "since the beginning".

The journal opens on **Last 90 days**. A preset stays live: if you leave "Year to date" selected, it still means year to date tomorrow.

## Accounts

The account picker lists your accounts as a tree, with a search box at the top to find one by name. Press <kbd>a</kbd> to open it with the search box focused.

Checking an account selects it and everything under it. Checking `expenses:food` includes `expenses:food:groceries` and `expenses:food:restaurants`. A parent shows a partial check when only some of its children are selected.

A transaction matches when any of its postings hits a selected account. With nothing selected, every account matches.

Account filters also change the totals. The insights panel and the Visible Journal Total only count postings to the selected accounts. Filtering to `expenses:food` therefore shows what you spent on food, not the bank side of each purchase.

## Search

The search box matches text anywhere in a transaction:

- the description and code
- transaction and posting comments, including tags
- account names
- amounts as displayed, and commodity symbols

Matching is case-insensitive and finds the text inside longer words. Press <kbd>/</kbd> to jump to the search box. The table updates once you pause typing. <kbd>Esc</kbd> leaves the box and keeps the query.

## Active filter chips

When any filter differs from the default, a row of chips appears under the bar. There is one chip for the date range, one for each selected account and one for the search text. Click a chip's ✕ to remove that filter. **Reset all** returns to the last 90 days, all accounts and no search.

## Sharing a filtered view

The filters are written into the page URL as you change them:

| Parameter | Meaning |
| --- | --- |
| `preset` | A date preset: `thisMonth`, `lastMonth`, `last90`, `ytd`, `thisYear`, `lastYear` or `all`. |
| `from`, `to` | A custom range as `YYYY-MM-DD`. An empty value is an open end. |
| `acct` | Selected accounts, comma-separated. |
| `q` | The search text. |

For example, `?preset=ytd&acct=expenses:food` shows this year's food spending. A link with a preset keeps working as time moves. A link with explicit dates always shows the same range.

This is most useful when you run Ledgeline as a server and open it in a browser. You can bookmark a view or send it to someone who has access.

## Arriving from another page

Several pages open the journal already filtered. Pressing <kbd>Enter</kbd> on an account row in a [report](../reports/index.md), or on a holding in [Holdings](../holdings/index.md), shows that account's transactions over all dates. The [Problems](problems.md) drawer widens the date range just enough to include the transaction it is taking you to and leaves the other filters alone.

## Related pages

- [The journal](index.md)
- [Keyboard shortcuts](keyboard.md)
- [Exporting the journal](export.md), which exports exactly what the filters show
