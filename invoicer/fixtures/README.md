# Invoicer fixtures

Four books folders for the invoicer (PLAN 0d, and `empty/` from Phase 4
feedback H22). Every name, number and amount
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
hold plan 45's invoices 2001–2004, which have no files: `check` reports
them as four orphan-transaction warnings. Numbering over files and books gives 2005; the bill `PA-9`
is a payable, in its own namespace, so it doesn't count.

## `cash/`

The receivables fixture's cash books (Apply payment already re-split 201 and
202), a deposit wire for 206, and `[books] basis = "cash"`. Cash books book
an invoice at payment only, so no invoice has a receivable posting, and a
deposit is income the day it arrives: 206's wire went to revenue, and 207's
deposit row books nothing.

| Invoice | Customer   | Case                                                |
|---------|------------|-----------------------------------------------------|
| 201     | Contoso    | Paid (Stripe payout, re-split)                      |
| 202     | Fabrikam   | Partly paid ($500 wire, pro rata)                   |
| 203     | Wingtip    | Sent, unpaid                                        |
| 204     | Customer X | Draft                                               |
| 205     | Tailspin   | Void                                                |
| 206     | Customer X | Deposit, paid by wire: income on receipt            |
| 207     | Customer X | Final, applying 206; unpaid ($7,000 left to pay)    |

## `empty/`

Books on the invoicer's first day: `books.journal` declares a few accounts and
books nothing; `ledgeline.toml` has `[invoicer]` (with `number-start = 1001`);
`company/My Company.md` is the company note. No invoices, customers, catalog
or snippets, so every list shows its empty state and the next number is
1001, "the first number". The e2e spec `invoicer/web/e2e/empty.e2e.ts` runs a
second server over a scratch copy of it; `session.rs` and the app's
`files_tests.rs` check it reads empty.

## `time/`

Time tracking (plan 46) beside small accrual books. **Its clock is
2026-11-26.** `time.journal` is the roll-up (`books.journal` does not
include it); `time-accounts.journal` links `clients:acme` to Acme Corp,
`clients:globex` to Globex and `clients:initech` to `Initech Inc` (that
customer's `ledger_customer`), with `item:` codes from
`invoices/catalog.md`. Tests: `invoicer-core/tests/time.rs`, the app's
`time_tests.rs`, and `cli.rs`'s `time_*`.

| File / note                   | Case                                                                                         |
|-------------------------------|----------------------------------------------------------------------------------------------|
| `time/pat/2026-10.timedot`    | Pat's October: 5 h on invoice 101 (sent and booked: locked), unbilled lines, an orphan account with no customer, an undeclared `internal:training` |
| `time/pat/2026-11.timedot`    | Pat's November: two 2-minute emails (rounding), travel (not billable), an archived account   |
| `time/sam/2026-11.timedot`    | A contractor (no `person:` tag: the folder says who), rebilled to Acme as `JR`               |
| `time/riley/2026-11.timedot`  | A contractor whose note has no rate or accounts: a bill asks for them                        |
| `time/imported/2026-09.timeclock` | One timeclock session                                                                     |
| `customers/Acme Corp.md`      | `cadence: monthly`, `time_detail: daily`, 6-minute increments rounded up, a `time_items` rule |
| `customers/Globex.md`         | No cadence, `time_detail: entries`                                                           |
| `people/Sam Lee.md`           | A timekeeper with every key, paid to `[[Lee Consulting LLC]]` (`pay_to`, `vendor:`)          |
| `people/Riley Chen.md`        | A timekeeper with none                                                                       |
| `invoices/… Invoice 101.md`   | Sent and booked (by `send`), so its time is locked                                           |

```sh
hledger -f time/time.journal bal --tree
hledger -f time/time.journal bal --pivot billable -p 2026-11
```

## Check the books with hledger

```sh
hledger -s -f accrual/books.journal -f accrual/recognition.journal check
hledger -f accrual/books.journal bal 'tag:invoice=^10[1-7]$' 'assets:accounts receivable'
hledger -s -f cash/books.journal check
hledger -s -f empty/books.journal check
hledger -s -f time/books.journal check
```
