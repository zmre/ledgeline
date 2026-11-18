# Accrual fixture (plan 45)

Synthetic books for revenue and expense recognition. Every name, number and
amount is invented. The clock is **2026-11-17**.

| File | What it is |
|---|---|
| `books.journal` | The main journal. Source transactions, posted recognition entries, invoices and payments. Includes `books-2025.journal`. |
| `books-2025.journal` | 2025's transactions: the opening balance and Customer X's prepayment with its December entry (placed here by date, as a per-year file should receive its own rows). |
| `recognition.journal` | The schedules: plain hledger `~` rules, **not** included by `books.journal`. |
| `ledgeline.toml` | `[books] closed-through = "2026-09-30"`. |
| `capture.sh` | Writes `expected/` from hledger itself. Never regenerated from Rust. |

Posted entries run through **2026-10-31**, so the November occurrences are due
on 2026-11-17 (and none after it).

## Cases

| Case | Where | What it pins |
|---|---|---|
| Point-in-time sale | 2026-03-10 Fabrikam Widgets | No schedule: revenue with a `customer:` tag only. |
| Annual prepay | 2025-12-17 Customer X Payment (`cx1217`) | Twelve $1,000 entries in advance, Dec 17 … Nov 17. Dec 2025 – Oct 2026 posted; **2026-11-17 due**. |
| Multi-line payment | 2026-08-01 Contoso Payment | A fee and a one-time item are never scheduled. Two deferred lines with the same terms share `ct0801` (one entry per month, a pair of postings per line); the 3-month line is `ct0802` (complete). |
| Invoice → payment | 2026-06-15 Tailspin Invoice 2001 (`tt2001`) | Recognition starts 2026-07-01 (`service-start`) whatever the payment. The payment on 2026-07-20 is $30 short: a fee proposal by invoice number. |
| Billed after delivery | 2026-09-05 Wingtip Invoice 2002 | An invoice straight to revenue; the 2026-09-25 payment matches exactly by invoice number: a clear settlement. |
| Renewal | Customer X (`cx1217`) | The latest schedule a renewal prefills from; a new schedule for Customer X is `contract: renewal`. |
| Anchored | 2026-05-17 Litware Payment (`lw0517`) | `proration: calendar`: a $483.87 stub dated 2026-05-17, $1,000 on the 1st, and the $516.13 remainder on 2027-05-01. |
| Credit-back | 2026-04-01 Adatum Payment (`ad0401`), 2026-10-15 Adatum Invoice 2003 (`ad1015`) | The upgrade credits $1,000 of unused deferred revenue (a debit with no `recognition:`) and starts an `expansion` schedule. Ending `ad0401` on 2026-10-31 leaves exactly the credit unrecognized. |
| Rounding | 2026-02-03 Customer Y Payment (`cy0203`) | $10,000 / 12: eleven $833.33 and an $833.37 true-up. |
| Prepaid D&O + E&O | `nw0301` (Northwind Insurance, 2026-03-01), `pw0615` (Proseware Assurance, 2026-06-15) | Two vendors, two start dates, `kpi: prepaid`, bills through payables: separate schedules, never one rolled-up entry. The D&O bill is paid exactly (`B-77`). |
| External invoice | 2026-11-02 Fourth Coffee Invoice 2004 | Receivable vs deferred revenue with `invoice:` only: **Needs a schedule**. |
| Closed period | Alpine Ski House (`as0710`) | The 2026-09-10 occurrence was never posted and is on or before `closed-through`: due in a closed period. |
| Duplicate | 2026-03-17 Customer X Recognized (copy) | A hand copy of a posted entry: the `recognition-duplicate` finding. |

## Check it with hledger

```sh
hledger -f books.journal bal                       # posted only
hledger -f books.journal -f recognition.journal reg tag:recognition \
  --forecast=tomorrow.. --today 2026-11-17         # the schedules ahead
hledger -s -f books.journal -f recognition.journal check
```
