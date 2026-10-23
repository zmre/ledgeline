# 27 — Cash flow statement (indirect method, tagged sections and groups)

Replace the Cash Flow tab's per-cash-account view with a real statement of
cash flows: net income, adjusted by the change in every non-cash balance-sheet
account, sorted into operating, investing and financing, and reconciled to the
actual change in cash. Sections and lines come from account tags, the same way
`issection:`/`isgroup:` shape the P&L and `bsterm:`/`bsgroup:` the balance
sheet.

## The ask (Patrick, 2026-10-22)

> It's right for [cash flow] to focus on cash accounts, but we do sometimes
> have accounts that we want to track in cash flow to see deltas over time.
> Accounts receivable and accounts payable are examples. … I think we need
> account tags like we use for the balance sheet and p&l but for cash flow so
> we can include other items and group them.
>
> I may also want to roll up all the cash accounts in cash flow so we want
> sections and groups.

Two reference layouts for the same company. A hand-built spreadsheet:

```text
Cash Flow from Operations
    Net Income
    ∆ in AR
    ∆ in Other Curr Assets
    ∆ in AP
    ∆ in Other Curr Liabilities
    ∆ in Deferred Revenue
    ∆ in Long Term Liabilities
        Total Cash Flow Operations
Cash Flow from Investing
    Depreciation Exp
    CapEx
        Total Cash Flow Investing
Cash Flow from Financing
    ∆ in Equity
        Total Cash Flow Financing
Net Change in Cash
Beg Cash Balance
End Cash Balance
```

And QuickBooks Online's Statement of Cash Flows, which lists one line per
account under Operating ("Adjustments to reconcile Net Income…": AR,
Prepayments, Accum Depr, AP, each credit card, Payroll Wages Payable,
Deferred Revenue, Ramp Card) and Financing (Convertible Note – Accrued
Interest), then NET CASH INCREASE FOR PERIOD.

## Decisions taken (2026-10-22)

1. **Replace, not toggle.** The tab always shows the statement. Today's
   per-cash-account detail survives as the expandable cash block at the
   bottom (Beginning cash / Ending cash, by cash group).
2. **Untagged accounts get GAAP defaults; tags override.** Every non-cash
   balance-sheet account lands in some section, so the statement always
   reconciles to cash.
3. **Depreciation defaults to Operating** (an add-back), as QuickBooks and
   GAAP place it. Detected structurally from the existing
   `valuation: depreciation` tag, never from the account name. The
   spreadsheet's Investing placement is one tag.
4. **Closing entries are detected and skipped,** so Δ equity never counts net
   income twice.
5. **Cash accounts take groups too,** so several bank accounts can roll up into
   one line in the cash block.

## The arithmetic, and why it always reconciles

Every transaction sums to zero. Partition accounts into **cash**, **P&L**
(revenue/expense) and **other balance-sheet**. For any window, in hledger's
signs:

```text
Δcash = −Δ(P&L) − Σ Δ(other) = net income + Σ (−Δ other account)
```

So each non-cash balance-sheet account contributes **−Δ** of its own balance:
an increase in AR (debit, +) reads negative (cash tied up), an increase in AP
or deferred revenue (credit, −) reads positive. Summing every line gives
exactly the change in cash, per commodity, with no valuation step to drift.

Two consequences:

- **The statement is unvalued** (cost/native amounts per commodity, like
  today's Cash Flow). In a single-currency journal its Net income equals the
  P&L tab's. With priced commodities the P&L tab is market-valued, so the two
  can differ, and the docs say so.
- **Skipping a transaction that touches no cash cannot break reconciliation:**
  it adds zero to both sides. That is what makes closing-entry detection safe.

## Tags

Both inherit to sub-accounts like every other account tag (nearest declared
ancestor wins).

### `cfsection:` — which section an account's change lands in

Closed vocabulary (coded, never matched against English, per
`account-type-not-name`):

| Value       | Meaning                                                              |
|-------------|----------------------------------------------------------------------|
| `operating` | Cash flow from operating activities                                  |
| `investing` | Cash flow from investing activities                                  |
| `financing` | Cash flow from financing activities                                  |
| `cash`      | Counts as cash: shown in the cash block, not as an adjustment        |

`cash` exists so an account can count as cash or cash-equivalent (a
treasury-bill account, say) without changing its hledger `type:`, which
other hledger tooling reads.

There is deliberately **no `none`/exclude value**: excluding a balance-sheet
account would break the reconciliation guarantee in silence.

### `cfgroup:` — which line an account prints on

Free text, like `isgroup:`/`bsgroup:`. Accounts sharing a value share a line,
even across unrelated parts of the tree. On cash accounts it names the
cash-block line ("Mercury", "Operating cash").

### Resolution, first match wins

1. Revenue/expense accounts (effective type R/X/G) are **never** lines: they
   are Net income. A `cfsection:`/`cfgroup:` on one is ignored and reported.
2. `cfsection:` on the account or its nearest declared ancestor.
3. Effective type Cash → `cash`.
4. `valuation: depreciation` (account or ancestor) → `operating`.
5. Effective type Asset or Liability with `bsterm: noncurrent` (declared or
   inherited) → `investing` for assets, `financing` for liabilities.
6. Any other Asset or Liability (current, or no `bsterm:` at all) →
   `operating`.
7. Equity (including Conversion) → `financing`.
8. No type at all → `operating`, and an `account-tag` diagnostic suggests a
   `type:` (this is the "reads in the wrong place in silence" case).

Checked against the IronCore journal as it stands, the defaults alone give
QuickBooks' layout: AR, prepayments, accumulated depreciation, AP, both cards,
payroll payable, deferred revenue → Operating; the convertible note's accrued
interest (under `liabilities:long-term`, `bsterm: noncurrent`) → Financing;
fixed assets → Investing.

### Line names without `cfgroup:`

One line per account, rolled up to the report's **Account depth**, named by
its full account path. That is QuickBooks' per-account listing. Adding
`cfgroup:` collapses lines into the spreadsheet's "∆ in AR" style.

## Closing-entry detection

A transaction is a closing entry when **all** of its postings are to P&L
accounts or equity accounts, with at least one of each. Detected structurally
(types, never names), so `equity:retained-earnings` needs no tag. Such
transactions are left out of every window: Net income and Δ equity both. An
opening-balance transaction touches cash or other assets, so it is never
mistaken for one.

## Report shape

```text
Operating activities
    Net income
    <operating lines…>
    Net cash from operating activities
Investing activities
    <investing lines…>
    Net cash from investing activities
Financing activities
    <financing lines…>
    Net cash from financing activities
Net change in cash
Beginning cash          ▸ expandable by cash group / account
Ending cash             ▸ expandable by cash group / account
```

- **Columns:** the existing From/To and Interval controls. Total gives one
  column; Monthly/Quarterly/Yearly give one per bucket plus Total (plan 24's
  rules, reusing the P&L's `Columns` shape and `range_buckets`).
  Beginning/Ending cash are point-in-time: the Total column's Beginning is the
  first bucket's, its Ending the last's, never a sum.
- **Adaptive, no empty boxes:** a section with no non-zero line in any column
  is omitted, as on the P&L. Net income always shows.
- **Lines display contributions:** −Δ, so every column sums down to Net change
  in cash.
- **Multi-currency:** one figure per commodity per cell, as today.
- **Reconciliation:** Net change in cash is computed BOTH ways (sum of
  sections, and Ending − Beginning cash). They are equal by construction. If
  they ever disagree the report says so with a banner rather than hiding it.
  This is a bug detector, not an expected state.

### Chart

Today's chart stacks cash accounts. Proposed replacement: per period, stacked
bars for Operating / Investing / Financing (positive up, negative down) with
a Net change in cash marker. **Open question below.**

## Work packages

### A — Engine (`ledgeline-core`)

1. `reports/cash_flow.rs` rewritten as `cash_flow_statement(txns, opts,
   interval, declared types, cf sections, cf groups, bs terms, valuation
   roles)`. Pure; no I/O. The old `cash_flow`/`cash_flow_range` and the
   Trailing window are deleted (the legacy `end`/`count` URL already rewrites
   to From/To, per plan 24).
2. `CF_SECTION_TAG`/`CF_GROUP_TAG`, `parse_cf_section_tag`,
   `account_cf_sections_from`, reusing `nearest_declared` and
   `declared_groups`.
3. Closing-entry predicate, shared and unit-tested on its own.
4. `wire.rs`: the `cfsection:` vocabulary joins `CLOSED_TAGS`, and new
   findings for "cfsection/cfgroup on a revenue/expense account is ignored"
   and "no type, defaulted to operating".
5. Tests:
   - Property test (proptest): over random balanced journals, the sum of
     sections equals Ending − Beginning cash, per commodity, for every
     interval.
   - Fixture `fixtures/reports/cf-statement.journal`: AR, prepayments, AP, a
     card, deferred revenue, fixed asset + accumulated depreciation, a note
     payable, equity raise, a closing entry, two cash groups. Golden
     figures hand-checked against `hledger bal` deltas, the command quoted
     beside each assertion, as the P&L suite does.
   - Defaults table above, one test per rule.
   - Closing entry: skipped, and Δ equity excludes it.

### B — Server (`ledgeline-server`)

1. `/api/reports/cashflow` returns the new statement (`WireCashFlow…`); the
   reports API stops building `PeriodReport` for `cf`.
2. Endpoint tests mirroring `report_endpoints.rs`' P&L ones.

### C — Web

1. Decoder + types for the statement; `CashFlowStatementView.svelte`
   (sections, subtotals, expandable groups, interval columns, Total), built
   like `IncomeStatementView`.
2. Replace `CashFlowChart` per the chart decision.
3. Accounts editor: `cfsection` (select of the four codes) and `cfgroup`
   (free text) in `accounts/model.ts`' known tags.
4. xlsx export for the new shape (`cash-flow-…xlsx`, sections and
   subtotals, as the P&L export does).
5. Component tests; the e2e smoke's Cash Flow assertions updated to the
   statement's figures against `fixtures/sample.journal`.

### D — Docs

1. Rewrite the Cash Flow half of `docs/reports/cash-flow-and-net-worth.md`
   (likely its own page): the statement, the defaults table, the two tags,
   closing entries, unvalued figures.
2. `docs/settings/accounts.md` and `docs/journal/problems.md`: the new tags
   and findings.
3. Screenshots via `docs-screenshots`.

Order: A → B → C → D; A and the closing-entry predicate are where the risk
is, and the property test is the gate for moving on.

## Answers (Patrick, 2026-10-22)

1. **Chart:** stacked Operating / Investing / Financing per period with a Net
   change in cash marker.
2. **Default lines:** one per account, at Account depth.
3. **Beginning/Ending cash:** detail kept, shown expanded by default (it is
   today's per-cash-account view, so it stays visible).

The `type:A` typos below were fixed in the journal the same day.

## Journal fixes this surfaced (not code)

In the IronCore `accounts.journal`:

- `liabilities:capital-one` and `liabilities:long-term` are declared
  `type:A`. Under rule 5 the long-term note would land in **Investing**, not
  Financing, until they are `type:L`.
- `assets:fixed:computer-office-equipment:accum-depreciation` still carries
  `issection: other` (now ignored and flagged by Problems). Its
  `valuation: depreciation` tag is what puts it in Operating here.
- `assets:rd-tax-credit-receivable` carries `issection: tax`, ignored for the
  same reason.

## Status (2026-10-22): built

All four work packages shipped. Where the build departed from the plan, and
why:

- **Cash stays in the currency held; an "Effect of exchange rates on cash"
  line was added.** The plan said "everything at cost". That made a euro
  account's balance read in dollars and disagree with hledger. Non-cash lines
  are at cost; cash postings are as held; for a cash posting with a cost the
  difference (as-held − at-cost) is the exchange line, which keeps the
  identity exact. Found by the `cf-monthly` hledger golden on the sample
  journal's May currency exchange.
- **Investments default to Investing.** Rule 5 gained the balance sheet's
  Investments signal: with no `bsterm:`, an asset that has held a commodity
  other than the base one (from the journal's prices) is non-current.
  Without it, share purchases in a personal journal landed in Operating.
- **The old `cash_flow`/`cash_flow_range` functions are kept**, unused by the
  server. They are public, correct, and carry a large invariant and golden
  suite; removing them is separate cleanup.
- **No diagnostic for an untyped account defaulting to Operating.** Problems
  findings are anchored to `account` declarations, and an untyped account is
  usually undeclared. Left out rather than half-done.
- **Interval = Total heads its one column "Amount"**, as the P&L does.
- **The legacy `end`/`count` query is converted to the range it covers**
  server-side, rather than removed.

Checked against the IronCore journal for 2025: reconciles, and the untagged
defaults reproduce QuickBooks' Statement of Cash Flows layout.
