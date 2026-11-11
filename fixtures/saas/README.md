# Startup-metrics fixtures

The fixtures for `plans/37-startup-metrics.md`. Tests:
`crates/ledgeline-core/tests/saas_metrics.rs` (engine),
`crates/ledgeline-core/tests/saas_diagnostics.rs` (Problems checks) and
`crates/ledgeline-server/tests/report_endpoints.rs` (`/api/saas`).

| File | What it is |
|---|---|
| `startup.journal` | A small SaaS business, 2025-01 through 2026-06, in the recommended structure. **Generated** by `generate.py`; don't edit it by hand. |
| `generate.py` | Writes `startup.journal`. Deterministic. It exists only to write the file. |
| `clearing.journal` | The invoice → revenue → deferred pattern. Hand-written. |
| `payees.journal` | Customers inferred from payees. Hand-written. |
| `capture.sh` | Captures hledger 1.52's own output into `expected/`. It is also the docs' "Check it with hledger" command list. |
| `expected/*.csv` | hledger's output. **Never regenerate from Rust.** Re-run `capture.sh` after `generate.py`, and commit both. |

All three journals pass `hledger -f X check -s`.

## startup.journal

### Accounts

- Cash `assets:bank:operating` (`type: C`). Receivable `assets:receivable`
  (`kpi: receivable`). Payable `liabilities:payable` (`kpi: payable`).
  Deferred `liabilities:deferred-revenue` (`kpi: deferred-revenue`).
- Recurring revenue is `revenue:subscriptions` (`type: R, kpi: recurring`)
  with three leaves, each with `segment:` and `product:`. No `segment:` sits
  on a parent, so hledger's pivot and Ledgeline's nearest-wins rule can't
  differ.
  - `smb`: segment smb, product core.
  - `mid`: segment mid, product core, and `class: direct`.
  - `enterprise`: segment enterprise, product platform.
- Contoso's lines carry a posting-level `class: partner`, which overrides the
  mid leaf's `class: direct`. hledger's `--pivot class` agrees
  (`expected/class-mrr.csv`).
- `revenue:usage` (`kpi: usage`), `revenue:services` (non-recurring),
  `expenses:cogs:hosting` (`issection: cogs`), `expenses:sales-marketing`,
  `expenses:rnd`, `expenses:ga` (each with its `kpi:`), and
  `equity:funding` (`cfsection: financing`).

### Customers

Monthly customers are invoiced on the 1st through receivables (`customer:` on
the revenue line), and each invoice is collected 30 days later. June 2026's
invoices are still outstanding at the end.

| Customer | Segment | Story | MRR by month |
|---|---|---|---|
| Bluebird Analytics | mid | Monthly payer. Starts in month 1, expands in month 7. Also has $100/mo usage from month 7. | 500 (M1–M6), 800 (M7–M18) |
| Northwind Logistics | enterprise | Annual prepay. $12,000 invoiced to deferred revenue in M3 and recognised at $1,000/mo with `customer:`. Renews at $15,600 in M15: expansion at renewal. | 1,000 (M3–M14), 1,300 (M15–M18) |
| Pinecrest Dental | smb | Churns after month 9. | 200 (M1–M9) |
| Harbor Yoga | smb | Contracts in month 11. | 300 (M1–M10), 150 (M11–M18) |
| Lumen Labs | smb | Starts in month 6, zero from month 9, back in month 12. | 250 (M6–M8), 250 (M12–M18) |
| Granite Health | mid | Mid-market logo, joins in month 10 with a one-off $5,000 services fee that isn't MRR. | 1,500 (M10–M18) |
| Orbit Freight | smb → enterprise | Changes segment in month 13, upgrading as it moves. | 400 smb (M1–M12), 900 enterprise (M13–M18) |
| Contoso Ltd | mid | Name variants: spelled `Contoso Ltd.` in month 2 only. Both keys are real rows in the matrix and the Problems panel reports `customer-name-variants`. | 600 (all months, under `Contoso Ltd.` in M2) |
| Fjord Studio | smb | Credit month. A $500 service credit in M8 is larger than the month's $350 charge, so C = −150 that month. | 350 every month, −150 in M8 |
| (unattributed) | smb + mid | "Marketplace payout" with two untagged recurring legs, so the payee fallback doesn't apply. | 100 (M4, M16) |

M1 = 2025-01, M18 = 2026-06.

The other figures:
- Usage is Bluebird's $100/mo from M7.
- Hosting (COGS) is 20% of the month's MRR. It is billed on the 20th and paid
  15 days later.
- S&M is $2,000/mo for M1–M9 and $3,000/mo for M10–M18.
- R&D is $4,000/mo and G&A is $1,500/mo. Both are paid from the bank on the
  15th.
- The $500,000 seed round lands on 2025-01-02.

### MRR (`expected/mrr.csv`)

| Month | MRR | Month | MRR | Month | MRR |
|---|---:|---|---:|---|---:|
| 2025-01 | 2,350 | 2025-07 | 3,900 | 2026-01 | 5,550 |
| 2025-02 | 2,350 | 2025-08 | 3,400 | 2026-02 | 5,550 |
| 2025-03 | 3,350 | 2025-09 | 3,650 | 2026-03 | 5,850 |
| 2025-04 | 3,450 | 2025-10 | 4,950 | 2026-04 | 5,950 |
| 2025-05 | 3,350 | 2025-11 | 4,800 | 2026-05 | 5,850 |
| 2025-06 | 3,600 | 2025-12 | 5,050 | 2026-06 | 5,850 |

### The bridge, per month (total level)

The bridge uses clamped values, max(C, 0). Other = ΔMRR − (new + expansion −
contraction − churn + reactivation).

| Month | Bridge |
|---|---|
| 2025-01 | new 2,350 (6 logos: Bluebird, Pinecrest, Harbor, Orbit, Contoso Ltd, Fjord) |
| 2025-02 | new 600 (Contoso Ltd.), churn 600 (Contoso Ltd) |
| 2025-03 | new 1,000 (Northwind), reactivation 600 (Contoso Ltd), churn 600 (Contoso Ltd.) |
| 2025-04 | other +100 (unattributed) |
| 2025-05 | other −100 |
| 2025-06 | new 250 (Lumen) |
| 2025-07 | expansion 300 (Bluebird) |
| 2025-08 | churn 350 (Fjord: clamped to 0), other −150 (Fjord's negative month) |
| 2025-09 | reactivation 350 (Fjord), churn 250 (Lumen), other +150 |
| 2025-10 | new 1,500 (Granite), churn 200 (Pinecrest) |
| 2025-11 | contraction 150 (Harbor) |
| 2025-12 | reactivation 250 (Lumen) |
| 2026-01 | expansion 500 (Orbit). Per segment it is churn 400 in smb and new 900 in enterprise. |
| 2026-02 | none |
| 2026-03 | expansion 300 (Northwind's renewal) |
| 2026-04 | other +100 (unattributed) |
| 2026-05 | other −100 |
| 2026-06 | none |

### Period metrics: the Year-over-year span

Span 2024-07-01 … 2026-06-30 (the YoY preset with the clock at 2026-07-08).
- Previous half: 2024-07 … 2025-06, so *m* = 2025-06 and *m − k* = 2024-06,
  before the journal starts.
- Current half: 2025-07 … 2026-06, so *m* = 2026-06 and *m − k* = 2025-06.
- *k* = 12 in both halves, and each half's F is 365 days.

| Metric | Current | Previous |
|---|---:|---:|
| MRR / ARR | 5,850 / 70,200 | 3,600 / 43,200 |
| Usage (at *m*) | 100 | 0 |
| MRR growth | 62.5% | none (MRR(*m − k*) = 0) |
| CMGR | (5850/3600)^(1/12) − 1 = 4.12886…% | none |
| Net new ARR | 27,000 | 43,200 |
| Bridge | new 1,500 (1), expansion 1,100, contraction 150, churn 800 (3), reactivation 600 (2), other 0 | new 4,200 (9), churn 1,200 (2), reactivation 600 (1), other 0 |
| Customers | 8 (growth 0%) | 8 (growth none) |
| Logo churn | 3 / 8 = 37.5% | none |
| ARPA | 731.25 | 450.00 |
| ACV | 18,000.00 (Granite) | 5,600.00 (50,400 / 9) |
| NRR / GRR | 4,350 / 3,600 = 120.833…% ; 3,250 / 3,600 = 90.277…% | none (2024-06 is before the journal) |
| Quick ratio | 3,200 / 950 = 3.368… | 4,800 / 1,200 = 4 |
| Top-1 / top-5 / top-10 share | 1,500 / 5,850 = 25.64% ; 5,100 / 5,850 ; 100% | 1,000 / 3,600 ; 2,850 / 3,600 ; 100% |
| Revenue | 66,550 (60,350 MRR + 1,200 usage + 5,000 services) | 18,450 |
| Gross profit / margin | 54,480 / 81.863…% | 14,760 / 80% |
| Net burn (Σ over F) | 43,720 | 24,120 |
| Net burn, 3-mo avg (cash) | 19,630 / 3 = 6,543.33 | 19,730 / 3 = 6,576.67 |
| Adjusted burn, 3-mo avg (A1.1) | 11,080 / 3 = 3,693.33 | 14,130 / 3 = 4,710.00 |
| Cash | 432,160 | 475,880 |
| Runway (basis: adjusted) | 432,160 × 3 / 11,080 = 117.01 months; cash-out 2036-03-31 | 475,880 × 3 / 14,130 = 101.04 months; cash-out 2033-12-01 |
| Net burn Σ over F, adjusted (A3) | 44,070 | 29,520 |
| Burn multiple (basis: adjusted, A3) | 44,070 / 27,000 = 1.632… | 29,520 / 43,200 = 0.683… |
| Rule of 40 | (66,550 / 18,450 − 1) × 100 − 43,720 / 66,550 × 100 = 195.0… | none (no revenue before the half) |
| S&M / R&D / G&A | 33,000 / 48,000 / 18,000 | 12,000 / 24,000 / 9,000 |
| CAC | 33,000.00 (1 new logo) | 1,333.33 |
| CAC payback | 33,000 / (2,600 / 12 × 0.81863) = 186.05 months | 42.86 months |
| Deferred revenue | 10,400 | 8,000 |
| Billings | 66,550 + 10,400 − 8,000 = 68,950 | 18,450 + 8,000 − 0 = 26,450 |
| DSO | 4,650 / 66,550 × 365 = 25.50 days | 2,600 / 18,450 × 365 = 51.44 days |
| DPO | 1,170 / 111,070 × 365 = 3.84 days | 720 / 48,690 × 365 = 5.40 days |

Adjusted burn per month (A1.1: NetBurn + Δdeferred − ΔAR, from
`expected/cash.csv`, `expected/deferred.csv` and the receivable balance):

| | M1 | M2 | M3 | M4 | M5 | M6 | M7 | M8 | M9 | M10 | M11 | M12 | M13 | M14 | M15 | M16 | M17 | M18 |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| Cash | 5150 | 7970 | −8730 | 8070 | 3490 | 8170 | 2620 | 5780 | 8180 | −2570 | 9490 | 1410 | 4860 | 9610 | −15290 | 9570 | 390 | 9670 |
| Adjusted | 5150 | 5620 | 4620 | 4720 | 4840 | 4570 | 4220 | 4780 | 4430 | −820 | 4590 | 4310 | 3860 | 3960 | 3660 | 3620 | 3740 | 3720 |

- The prepayments land in M3 and M15. Raw burn swings to −8,730 and
  −15,290 there; adjusted burn stays in line.
- M10 is negative because Granite's $5,000 services fee counts when it is
  invoiced.
- Burn multiple uses adjusted burn too (A3); raw net burn is still reported.

Visibility:
- recurring, customers, business and cogs are all available.
- The dimensions offered are `segment`, `product` and `class`.
- Coverage is (78,800 − 200) / 78,800 = 99.75%.
- No customers are inferred.

Segments at the current *m*:
- segment: mid 2,900, enterprise 2,200, smb 750. smb's NRR is 750 / 1,500 = 50%, because Orbit left smb.
- product: core 3,650, platform 2,200.
- class: direct 2,300, partner 600, (unassigned) 2,950.

## clearing.journal

Each annual invoice is booked to `revenue:sales` with `customer:`, moved to
deferred revenue the same day, and recognised monthly without a customer. SMB
revenue sits on the parent's own postings and enterprise on a leaf. `segment:`
is declared on both the parent and the leaf, which pins the nearest-wins rule.

Expect:
- MRR is 2,100 in each of 2025-01 … 2025-03.
- Customers from billings (A1.2), over 2025-01-01 … 2025-02-28 (halves are
  January and February). The billings are the two invoices: Alder $1,200 and
  Birchway $24,000, both in January.

  | | Current (m = 2025-02) | Previous (m = 2025-01) |
  |---|---|---|
  | Customers | 2 | 2 |
  | Growth | 0% | none |
  | New logos | 0 | 2 |
  | Lapsed logos | 0 | 0 |
  | Logo churn | 0% | none |
  | ACV | 25,200 / 2 = 12,600 | 12,600 |
  | Window short | yes | yes |
  | Inferred | no | no |
- `customers_hidden_reason = billing-clearing`.
- One `recurring-billing-clearing` Problems finding per recurring account.

## payees.journal

The recurring account is tagged, but no posting carries `customer:`. Each
invoice has one recurring posting and a receivable leg. The receivable is
inferred from its name, with no `kpi:`. A month-end recognition entry has a
deferred-revenue leg.

Expect:
- Customers are available through the payee fallback, with Aspen Dental and
  Cedar Studio inferred.
- `inferred_customers = true`.
- The recognition entry's $50/mo is unattributed.
- MRR is 850, 950 and 550.
