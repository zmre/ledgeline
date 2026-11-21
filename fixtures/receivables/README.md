# Receivables fixture (invoicer Phase 2a)

Synthetic books for the invoicer's books bridge: documents found by number,
N:1 settlement, voids, deposits, aging and cash-books **Apply payment**.
Every name, number and amount is invented. The clock is **2026-11-17**.

| File | What it is |
|---|---|
| `books.journal` | Accrual books, exactly as `the_invoicer_flow_on_accrual_books` (`crates/ledgeline-core/tests/accrual_library.rs`) leaves them, match ids aside. |
| `cash.journal` | Cash books, exactly as `the_invoicer_flow_on_cash_books` leaves them. |
| `capture.sh` | Writes `expected/` from hledger itself. Never regenerated from Rust. |

If the flow changes on purpose, regenerate the journals and then the captures:

```sh
LEDGELINE_RECEIVABLES_FIXTURE_WRITE=1 nix develop -c cargo test -p ledgeline-core --test accrual_library the_invoicer_flow
nix develop -c fixtures/receivables/capture.sh
```

## Cases (accrual)

| Invoice | What happens | At 2026-11-17 |
|---|---|---|
| 101 Contoso, $4,000 | Paid in two parts (09-20 $1,500, 10-05 $2,500), both joined to one `match:` link with `link_payments`: open after the first, complete after the second. | Paid, 34 days |
| 102 Fabrikam | Sent at $3,000, **re-booked** (delivery $1,200, a −$300 discount line) to $2,900, then paid exactly and linked 1:1 (plan 45's `link_settlement`). | Paid, 35 days |
| 103 Tailspin, $800 | **Voided** on 10-10: a reversal tagged `invoice:` and `void:`, linked with `match:`. | Void |
| 104 Customer X, $8,350 | A **deposit** invoice to `liabilities:customer-deposits`, paid 10-08. | Paid, 7 days |
| 105 Customer X | The **final** invoice (ARCHITECTURE §4.2): $7,200 + $9,500 less the $8,350 deposit (a negative line), `reference:` sanitized from `PO #PY75334, rev. 2`. The deposit liability nets to zero. | Open $8,350, current (due 12-10) |
| 106 Wingtip, $1,200 | Never paid. | Open, 78 days past due (61–90) |
| — Gamma, $250 | A receipt to receivables matched to nothing. | Unapplied |

Receivable balance $9,300 = $8,350 + $1,200 outstanding − $250 unapplied.
DSO over 90 days: $9,550 / $23,600 × 90 = **36.4**. Average days to pay
(101, 102, 104): **25.3**.

## Cases (cash)

| Invoice | Deposit | Applied |
|---|---|---|
| 201 Contoso, $3,000 ($1,800 + $1,200) | Stripe payout: $2,910 to the bank, $90 fee | Exact: the income side becomes the two rows. Undone (the text restored byte for byte) and applied again. |
| 202 Fabrikam, $2,000 ($1,500 + $500) | A $500 wire naming "INV 202" | Partial: $375 / $125, pro rata. |

The bank and fee lines are byte-identical to the import in both.

## Check it with hledger

```sh
hledger -f books.journal bal 'tag:invoice=^105$'           # one invoice's transactions
hledger -f books.journal bal tag:kpi=receivable not:tag:match  # what no link covers
hledger -f cash.journal bal 'tag:invoice=^201$'
```
