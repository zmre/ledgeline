# Invoicer fixtures

Two books folders for the invoicer (PLAN 0d). Every name, number and amount
is invented. The clock is **2026-11-17**.
`invoicer-core/tests/fixtures.rs` checks that both read with no problems and
that every booked invoice's total equals its receivable posting.
`ledgeline-invoicer/tests/cli.rs` runs the CLI on a copy of each, under
`target/`, and snapshots its JSON. Tests never write to these folders.

## `accrual/`

A copy of `fixtures/accrual` (plan 45's books, `recognition.journal`
included), with the receivables fixture's invoices booked at the end of
`books.journal`, plus the markdown side: `customers/`, `company/`,
`invoices/`, `invoices/tpl/` and `invoices/catalog.md`. `ledgeline.toml` keeps
`[books] closed-through` and adds `[invoicer]`.

| Invoice | Customer   | Case                                                     | Books at the clock                  |
|---------|------------|----------------------------------------------------------|-------------------------------------|
| 101     | Contoso    | Paid in two parts (`paid: 2026-10-05`)                   | Settled by two payments on one link |
| 102     | Fabrikam   | Re-booked; a fixed discount to `revenue:discounts`       | Paid exactly                        |
| 103     | Tailspin   | Void (`void_date: 2026-10-10`)                           | Reversed, linked to the original    |
| 104     | Customer X | Deposit invoice to `liabilities:customer-deposits`       | Paid                                |
| 105     | Customer X | Final invoice applying 104; a supporting timesheet table | Open, $8,350                        |
| 106     | Wingtip    | Sent, never paid                                         | Open, overdue                       |
| 107     | Contoso    | Partly paid (`paid: partial 2026-11-05 $1,000`)          | Open N:1 link, $2,000 left          |
| 108     | Customer X | Draft with a deferred row and a custom field             | Not booked (I2)                     |

Customer X has a billing portal that requires a `reference`, contracts as a
wikilink and a file path, open POs and a default snippet. Fred Smith is a
person at Customer X, so invoices to him bill Customer X. The books also
hold plan 45's invoices 2001–2004, which have no files: Phase 2's `check`
reports them. Numbering over files and books gives 2005; the bill `PA-9`
is a payable, in its own namespace, so it doesn't count.

## `cash/`

The receivables fixture's cash books (Apply payment already re-split 201 and
202), a deposit wire for 206, and `[books] basis = "cash"`. Cash books book
an invoice at payment only, so no invoice has a receivable posting.

| Invoice | Customer   | Case                                   |
|---------|------------|----------------------------------------|
| 201     | Contoso    | Paid (Stripe payout, re-split)         |
| 202     | Fabrikam   | Partly paid ($500 wire, pro rata)      |
| 203     | Wingtip    | Sent, unpaid                           |
| 204     | Customer X | Draft                                  |
| 205     | Tailspin   | Void                                   |
| 206     | Customer X | Deposit, paid by wire to the liability |
| 207     | Customer X | Final, applying 206; unpaid            |

## Check the books with hledger

```sh
hledger -s -f accrual/books.journal -f accrual/recognition.journal check
hledger -f accrual/books.journal bal 'tag:invoice=^10[1-7]$' 'assets:accounts receivable'
hledger -s -f cash/books.journal check
```
