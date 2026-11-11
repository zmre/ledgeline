# 37 — Startup metrics on Insights

Insights gains a row of startup metrics when the journal can support them:
ARR, MRR growth, net new ARR, customers, NRR, burn multiple, runway, gross
margin, and a set of secondary metrics behind them. Each metric shows up only
when the journal has the data it needs. Nothing changes for a household
journal.

The data comes from three opt-in, hledger-native layers. Each turns on more
boxes:

1. A **`kpi:` account tag** marks roles that no existing tag captures
   (recurring revenue, deferred revenue, sales & marketing, …).
2. **Dimension tags** (`segment:`, `product:`, QuickBooks' `class:`, `region:`)
   on accounts or postings give per-segment breakdowns.
3. A **`customer:` posting tag** on recognised-revenue lines gives
   per-customer metrics: counts, the MRR bridge, NRR/GRR, ARPA, ACV,
   concentration.

Everything Ledgeline shows can be reproduced with `hledger bal … --pivot …`,
and the docs list the commands.

## The ask (Patrick, 2026-11-10)

> I want to have some statistic boxes on the reports insights tab that only
> show up when applicable and i particularly want to capture metrics important
> for startups such as ARR/MRR, ACV, TCV (maybe), new logo vs. expansion mrr,
> NRR, growth vs. burn multiple, number of customers, customer growth.
>
> But we need to be flexible in case someone wants to break these out by
> SMB/Mid/Enterprise or by product line and I'm not sure the best way to track
> such things without making a mess of the books or the account table with a
> zillion tags. I'd like to be as hledger native as possible.

A brainstorm compared the options: a counting commodity, customer
subaccounts, payee inference, invoice spreading and a sidecar export. This
plan is its outcome.

## Decisions (agreed with Patrick 2026-11-10)

1. **The tag key is `kpi:`.** One key, a closed set of values, one value per
   account, inherited by subaccounts. Same shape as `cfsection:`/`holdings:`.
2. **`kpi:` only adds what no existing tag says.** Gross margin comes from
   `issection: cogs` through the income statement's own resolver. Cash comes
   from `type: C`, operating/investing/financing from `cfsection:`, and
   current/long-term from `bsterm:`. There is no `kpi: cogs`.
3. **Same dashboard.** The metrics are a section of the Insights tab, not a new
   tab.
4. **Hide existing boxes that don't apply.**
   - **Biggest Movers** hides when it has no rows.
   - **Investment Performance** hides when there are no holdings in either
     period.
   - **Net Worth** hides when it's negative at the end of the current period,
     or when the journal is a business (any account declares `kpi:`).
   - Runway takes Net Worth's slot when it's available.
5. **ARR = the latest complete month's recurring revenue × 12.** It is
   labelled as such. Usage revenue is shown beside it, not in it.
6. **No TCV, bookings or RPO.** These are contract facts the books don't hold.
7. **Recognised revenue is the source of truth.** Customer attribution only
   distributes recognised MRR. Ledgeline never invents MRR from invoices.
8. **Customer metrics require coverage.** They show only when ≥ 90% of
   recurring revenue in the span is attributed to a customer. Otherwise they
   hide, with the coverage figure and the reason.
9. **Churn is the first zero month.** A customer whose MRR goes from > 0 to ≤ 0
   is churned that month. A later return is *reactivation*, a separate bridge
   line, so a credit month shows up as churn plus reactivation rather than
   disappearing. A configurable grace period is deferred (`TODO.md`).
10. **`kpi:` doesn't change any statement.** Its readers are Insights, the
    Problems panel, Top Transactions' skip rule and the accounts editor.
    Projections may read it later (deferred).
11. **QuickBooks per-line Name becomes `customer:`.** It applies on revenue,
    deferred-revenue and receivable lines that don't already carry a customer
    column value. Then nobody has to search-and-replace after each import.
12. **Export: yes.** An xlsx workbook, monthly across the whole span.

## Vocabulary

### `kpi:` values

| Value              | Expected type | Meaning                                                                                     | Feeds                                                  |
|--------------------|---------------|---------------------------------------------------------------------------------------------|--------------------------------------------------------|
| `recurring`        | Revenue       | Subscription revenue as recognised. MRR is its monthly total.                               | MRR, ARR, growth, bridge, NRR/GRR, customers, burn multiple |
| `usage`            | Revenue       | Metered or overage revenue. Shown beside MRR, excluded from ARR.                             | Usage box line                                         |
| `deferred-revenue` | Liability     | Contract liability.                                                                         | Billings, deferred balance, Top Transactions skip rule |
| `sales-marketing`  | Expense       | Sales and marketing spend.                                                                  | CAC, CAC payback, S&M % of revenue                     |
| `rnd`              | Expense       | Research & development: engineering, product, design, dev tooling.                          | R&D % of revenue                                       |
| `ga`               | Expense       | General & administrative.                                                                   | G&A % of revenue                                       |
| `receivable`       | Asset         | Trade receivables. Overrides name inference.                                                | DSO, payee fallback                                    |
| `payable`          | Liability     | Trade payables. Overrides name inference.                                                   | DPO                                                    |

Revenue with no `kpi:` is non-recurring. The values are matched
case-insensitively and trimmed, like `cfsection:`. An unknown value is a
Problems finding and is otherwise ignored.

**Name inference for receivable/payable.** With no `kpi:` anywhere in the
subtree, an Asset-typed account whose last segment contains `receivable` is
treated as a receivable. A Liability-typed account whose last segment
contains `payable` is treated as a payable. The match is ASCII
case-insensitive. This is the documented fallback; the declared tag wins.
Nothing else is inferred from names.

### Dimension keys

The default keys are `segment`, `product`, `class` and `region`, configurable
through the endpoint's `dims=` parameter.

For a recurring-revenue posting, the value of key *k* is decided in this
order:

1. The posting's own tag *k*.
2. The transaction's tag *k*.
3. The **nearest** declared account tag *k* on the posting's account or an
   ancestor. Nearest wins, like `type:`. (hledger's `--pivot` agrees; its
   `tag:` query doesn't. The docs say so.)
4. Otherwise no value. The posting lands in an `(unassigned)` row for that
   key.

A key is offered as a breakdown only if at least two distinct values carry
recurring revenue in the span. A key whose recurring revenue is more than 50%
unassigned is still offered, with the `(unassigned)` row visible.

### Customer identity

For each recurring-revenue posting:

1. A `customer-id:` posting tag is the key, if present. The display name comes
   from `customer:`, or is the id itself.
2. Otherwise the `customer:` posting tag is both key and name, trimmed.
3. Otherwise, the transaction's `customer:` tag.
4. **Otherwise, the payee fallback.** The payee is the description before `|`,
   trimmed. It's used only when all of the following hold, so a month-end
   recognition entry ("Revenue recognition") never becomes one giant
   customer:
   - the transaction has exactly one recurring posting,
   - it has no deferred-revenue leg, and
   - it has a receivable or cash leg.

   Postings attributed this way are flagged `inferred`. The UI says "some
   customers inferred from payees" when they make up any share of the
   current month.
5. Otherwise the posting is **unattributed**.

Matching is exact after trimming. Names that differ only by case,
punctuation or a legal suffix are a Problems finding, not merged silently.

## Semantics

### Months and periods

Insights splits `[start, end]` into a previous and a current half. The
startup metrics work in **calendar months**:

- **Month series.** Every calendar month that intersects `[start, end]`. A
  month that's only partly inside the span is flagged `partial`, and its MRR
  is computed but never used as a "latest complete month".
- **Point-in-time metrics** (MRR, ARR, customers, deferred revenue, AR, AP,
  cash, concentration) use the **last complete calendar month ending on or
  before the half's end**. A half with no complete month (only possible with
  custom dates) has `None` for these.
- **Flow metrics** (bridge sums, burn, S&M, billings, revenue, gross profit)
  sum the complete months inside the half.
- **Trailing metrics** (NRR, GRR) compare the half's last complete month *m*
  with *m − 12*. That month is computed by the engine even when it's before
  `start`, because the engine reads the whole journal. If *m − 12* is before
  the journal's first transaction, the value is `None`.
- **Net burn** for runway is the average of the last three complete months up
  to *m* (fewer if the journal is younger). It's labelled "3-mo avg".

### MRR and valuation

`MRR(m)` = −(sum of postings in month *m* to accounts whose resolved role is
`recurring`), valued in the base commodity the way Insights' Revenue box
values revenue: the same `PriceDb` and the same "at cost" rules. Amounts in a
commodity that can't be valued go into `unvalued` (shown as a footnote) and
are left out of MRR.

`Usage(m)` is the same for `usage`.

Postings are selected and dated **exactly as the income statement selects
them**: posting date (`date:` tag wins), with the same virtual-posting and
auto-posting treatment. Read the income statement's code and reuse its
filter. Don't define a second one. A test pins that, for any period, Σ MRR +
Σ usage + non-recurring revenue = the income statement's revenue total.

### The customer matrix

`C[c][m]` = MRR attributed to customer *c* in month *m*, for every month from
the journal's first recurring posting to the span's end. The bridge, NRR,
coverage and concentration all come from this one matrix.

Each month also has `unattributed(m)` (recurring revenue with no customer),
and `C` is never clamped. For the bridge only, a customer's month value is
`max(C[c][m], 0)`. The difference between the clamped sum and `MRR(m)` goes
into a reconciling **Other** line, so this identity always holds exactly:

```
MRR(m) = MRR(m−1) + new + expansion − contraction − churn + reactivation + other
```

### Bridge classification (month *m*, per customer, using clamped values *p* = m−1, *q* = m)

| Condition                                         | Category      | Amount  |
|---------------------------------------------------|---------------|---------|
| *p* = 0, *q* > 0, never positive before *m*       | new           | *q*     |
| *p* = 0, *q* > 0, positive in some month < *m*    | reactivation  | *q*     |
| *p* > 0, *q* = 0                                  | churn         | *p*     |
| *p* > 0, *q* > *p*                                | expansion     | *q − p* |
| *p* > 0, 0 < *q* < *p*                            | contraction   | *p − q* |
| otherwise                                         | none          | 0       |

`other(m) = MRR(m) − MRR(m−1) − (new + expansion − contraction − churn + reactivation)`.

Counts per month: `new_logos`, `churned_logos`, `reactivated_logos`.
`customers(m)` = |{c : C[c][m] > 0}|.

The bridge is computed **per customer across all segments first**. For a
per-segment bridge, the customer's value is split by the segment its revenue
came from in that month. A customer that moved segment therefore shows up as
churn in one segment and new in the other. At the total level it nets to the
customer's real change. The docs explain this.

### Period metrics (computed for both the current and previous half)

Here *m* is the half's last complete month, *F* the set of complete months in
the half, and *k* = |F|. A metric is `None` whenever its tier is missing, its
denominator is ≤ 0, or a needed month doesn't exist.

| Metric                 | Formula                                                                                              | Tier |
|------------------------|------------------------------------------------------------------------------------------------------|------|
| `mrr`                  | MRR(m)                                                                                               | B    |
| `arr`                  | 12 × MRR(m)                                                                                          | B    |
| `usage`                | Usage(m)                                                                                             | B    |
| `mrr_growth_pct`       | (MRR(m) / MRR(m − k) − 1) × 100, where *m − k* is the month before the half                          | B    |
| `cmgr_pct`             | ((MRR(m) / MRR(m − k))^(1/k) − 1) × 100                                                              | B    |
| `net_new_arr`          | 12 × (MRR(m) − MRR(m − k))                                                                           | B    |
| `bridge`               | Sums of each category over *F* (amounts and counts)                                                  | C    |
| `customers`            | customers(m)                                                                                         | C    |
| `customer_growth_pct`  | customers(m) / customers(m − k) − 1                                                                  | C    |
| `logo_churn_pct`       | Σ churned_logos over F / customers(m − k)                                                            | C    |
| `arpa`                 | MRR(m) / customers(m). Attributed MRR only: Σ_c C[c][m]⁺ / customers(m)                              | C    |
| `acv`                  | Mean over new logos in *F* of 12 × their first positive month's MRR                                  | C    |
| `nrr_pct`              | Σ_{c ∈ S} C[c][m]⁺ / Σ_{c ∈ S} C[c][m−12]⁺ × 100, where S = {c : C[c][m−12] > 0}                      | C    |
| `grr_pct`              | Σ_{c ∈ S} min(C[c][m]⁺, C[c][m−12]⁺) / Σ_{c ∈ S} C[c][m−12]⁺ × 100                                     | C    |
| `quick_ratio`          | (new + expansion + reactivation) / (contraction + churn) over F                                      | C    |
| `top1_pct`, `top5_pct`, `top10_pct` | Largest 1/5/10 customers' C[c][m]⁺ / MRR(m)                                             | C    |
| `revenue`              | Total revenue over F (same figure as Insights' Revenue box restricted to F)                          | A    |
| `gross_profit`, `gross_margin_pct` | Revenue − COGS over F, via `section_resolver`. Shown only if any account resolves to `Cogs` | A    |
| `net_burn_avg3`        | Mean of NetBurn over the last ≤ 3 complete months ending at *m*, where NetBurn(month) = −(net change of operating + investing cash flow) | A |
| `cash`                 | Cash accounts' balance at the end of *m* (`cash_predicate`)                                          | A    |
| `runway_months`, `cash_out_date` | cash / net_burn_avg3, and the end of *m* plus that many months (to the day). Only when net_burn_avg3 > 0 | A |
| `burn_multiple`        | Σ NetBurn over F / net_new_arr. Only when both > 0                                                    | A+B  |
| `rule_of_40`           | revenue growth % (revenue over F vs over the previous half's F) + FCF margin %, where FCF = operating + investing cash flow over F | A |
| `sm`, `rnd`, `ga`      | Spend over F                                                                                         | B    |
| `sm_pct`, `rnd_pct`, `ga_pct` | Spend / revenue                                                                               | B    |
| `cac`                  | sm / Σ new_logos over F                                                                              | B+C  |
| `cac_payback_months`   | sm / ((new + expansion) over F / k × gross_margin_pct / 100)                                          | B+C  |
| `deferred_revenue`     | Balance of `deferred-revenue` accounts at end of *m* (positive = owed)                                | B    |
| `billings`             | revenue over F + (deferred(m) − deferred(m − k))                                                      | B    |
| `dso_days`             | AR(m) / revenue over F × (days in F)                                                                  | B/infer |
| `dpo_days`             | AP(m) / (COGS + operating expenses over F) × (days in F)                                              | B/infer |

`NetBurn(month) = −(Δcash(month) − Financing(month))`. Here Δcash is the
change in cash accounts' balance (`cash_predicate`), and Financing is the
financing section's total from `cash_flow_statement` (its `cf_role_resolver`)
for that month. Because operating + investing + financing = Δcash in the
statement, this equals −(operating + investing). Financing (equity raises,
loans) is excluded, so a funding round doesn't look like negative burn.
Reuse `cash_flow_statement` with a monthly interval over the span. Don't
write a second classifier. A test pins the identity against the statement's
own operating + investing totals.

### Visibility (what the API reports, and the UI obeys)

```
available.recurring  = some account resolves to kpi: recurring AND MRR(m) > 0 for some complete month in the span
available.customers  = available.recurring AND coverage ≥ 0.90
                       coverage = attributed recurring revenue / |recurring revenue| over the complete months in the span
available.business   = any account declaration carries a kpi: tag (valid or not)
available.cogs       = any account resolves to IsSectionKind::Cogs
available.dimensions = keys with ≥ 2 distinct values (see above)
```

When `recurring` is true but `customers` is false, the response carries
`customers_hidden_reason`:

- `"no-customer-tags"`: coverage is 0.
- `"low-coverage"`: 0 < coverage < 0.90. The response also carries
  `coverage_pct`.
- `"billing-clearing"`: a recurring account is debited against deferred
  revenue (the invoice → revenue → deferred pattern), and most attributed
  revenue sits on those clearing postings. The UI text says that per-customer
  figures there would be billings, not MRR, and links to the docs section.

### Top Transactions skip rule (Insights Box 10)

A transaction is also skipped from Top Transactions when **every** leg is
either revenue-typed or `kpi: deferred-revenue`, **and** its revenue legs net
to zero in the base commodity. That's a pure "move to deferred". A combined
entry that also recognises revenue isn't skipped: it really is revenue.
Existing rule `is_bookkeeping_only` is unchanged. The new rule is a sibling
function with its own tests.

## Engine (`crates/ledgeline-core/src/reports/`)

### `kpi.rs` (new)

Modelled on `cash_flow_statement.rs`'s `cfsection:` set:

- `pub const KPI_TAG: &str = "kpi";`
- `pub enum KpiRole { Recurring, Usage, DeferredRevenue, SalesMarketing, Rnd, Ga, Receivable, Payable }`
  with `code()` (wire spelling as in the table) and `ALL`.
- `pub fn parse_kpi_tag(value: &str) -> Option<KpiRole>`, trimmed and ASCII
  case-insensitive.
- `pub fn declared_kpi_roles(accounts: &[AccountDeclaration]) -> BTreeMap<String, KpiRole>`.
  Skips empty or unreadable values.
- `pub fn kpi_resolver<'a>(declared: &'a BTreeMap<String, KpiRole>, types: &'a AccountTypes) -> impl Fn(&str) -> Option<KpiRole> + 'a`.
  Nearest declared via `nearest_declared`, then receivable/payable name
  inference as specified.
- `pub fn expected_type(role) -> AccountType` (root type) for the Problems
  check.
- `pub const DIMENSION_KEYS_DEFAULT: [&str; 4] = ["segment", "product", "class", "region"];`
- `pub fn declared_dimension_tags(accounts, keys) -> BTreeMap<String, BTreeMap<String, String>>`
  (account → key → value) and a nearest-wins lookup.

### `saas.rs` (new)

```rust
pub struct SaasOpts<'a> {
    pub start: &'a str,                 // same span as Insights
    pub end: &'a str,
    pub dims: &'a [String],             // dimension keys to consider
    pub coverage_min: Dec,              // 0.90
    pub top_n: usize,                   // bridge customer lists, default 5
    pub with_customer_matrix: bool,     // export detail
}

pub fn saas_metrics(journal: &Journal, opts: &SaasOpts) -> Result<SaasReport, ReportError>;
```

`SaasReport`:

- `period: InsightsPeriod`: the same split as `insights()`. Factor the split
  into a shared function, and don't change its behaviour.
- `base: String`
- `available: SaasAvailability { recurring, customers, business, cogs, dimensions: Vec<String>, customers_hidden_reason: Option<HiddenReason>, coverage_pct: Option<f64>, inferred_customers: bool }`
- `months: Vec<SaasMonth>`: span months, each with
  `{ month: "YYYY-MM", partial, mrr, usage, unattributed, customers: Option<u32>, bridge: Option<Bridge>, revenue, cogs: Option<Dec>, net_burn, cash_end, sm, rnd, ga, deferred_end: Option<Dec>, ar_end: Option<Dec>, ap_end: Option<Dec> }`
- `current: SaasPeriodMetrics`, `previous: SaasPeriodMetrics`: the table
  above. All `Option` except where the tier is A.
- `segments: Vec<SegmentBreakdown { key, rows: Vec<SegmentRow { value, mrr_curr, mrr_prev, customers_curr: Option<u32>, arpa: Option<Dec>, nrr_pct: Option<f64>, bridge: Option<Bridge> }> }>`
- `bridge_customers: Option<BridgeCustomers { new, expansion, contraction, churn, reactivation: Vec<CustomerMove { customer, name, segment: Option<String>, amount: Dec, inferred: bool }> }>`:
  the current half, top `top_n` by amount.
- `top_customers: Vec<CustomerShare { customer, name, mrr, pct }>` at *m*
  (top 10).
- `customer_matrix: Option<Vec<CustomerRow { customer, name, segment, values: Vec<Dec> }>>`
  (aligned to `months`), only when `with_customer_matrix`.
- `unvalued: MixedAmount`

Exact `Dec` everywhere. Percentages are `f64` computed at the end, the same
way `MetricDelta::pct` is. Shared helpers come from `periods.rs`
(`add_months`, `bucket_key`, `days_between`).

**One pass.** Walk the transactions once, in posting-date order. For each
posting, resolve its account's type, role, section, cash flag and dimensions
through caches keyed by account name, the way `account_totals` caches.
Accumulate per-month totals and the customer matrix. Derive everything else
from those arrays. The target is < 20 ms on the 200k-transaction perf corpus;
record the figure in `docs/development/perf-baseline.md`.

Register the module in `reports/mod.rs` and re-export the public types.

### Changes to `insights.rs`

- Factor `split_period(start, end) -> Result<InsightsPeriod, ReportError>` out
  of `insights()`, unchanged, for `saas_metrics` to share.
- Add `is_deferral_move(txn, types, kpi)` and apply it in `top_transactions`
  as specified.
- `InsightsReport` gains `has_holdings: bool` (any holding in either half). The
  SPA uses it to hide Investment Performance. Movers already come back empty
  when there are none.

## Problems checks (`crates/ledgeline-core/src/wire.rs`)

| Rule id                        | Severity | Fires when                                                                                                       |
|--------------------------------|----------|------------------------------------------------------------------------------------------------------------------|
| (closed tag) `kpi`             | warning  | `kpi:` has an unknown value. Add a `ClosedTag` entry, so the existing message lists the accepted values.          |
| `kpi-type-mismatch`            | warning  | The account's effective root type ≠ `expected_type(role)`, e.g. `kpi: recurring` on an expense account.           |
| `recurring-billing-clearing`   | info     | A transaction debits a `recurring` account and credits a `deferred-revenue` account (a "move to deferred"). Reported once per account, with the count of such transactions and the first one's index. |
| `customer-name-variants`       | warning  | Two or more distinct `customer:` values normalise to the same key: lowercase, strip punctuation and whitespace, drop a trailing `inc`, `llc`, `ltd`, `corp`, `co`, `gmbh`, `plc`, `sa`, `bv`. Reported once per group, listing the variants and one transaction for each. |

Add each rule to `DIAGNOSTIC_RULES`, and give each a section in
`docs/journal/problems.md`. Tests go in `tests/tag_diagnostics.rs`
(closed tag, mismatch) and a new `tests/saas_diagnostics.rs` (clearing,
variants), including negative cases:

- an account with no `kpi:` never fires,
- a receivable *inferred* from its name never fires the mismatch check,
- `Acme` vs `Acme Labs` doesn't fire.

## QuickBooks import (per-line Name)

- `qb_journal.rs`: `QbPosting` gains `name: Option<String>`, read from every
  row's Name cell (`posting()`). `QbTransaction.name` keeps reading the head
  row, unchanged, for the payee.
- `crates/ledgeline-server/src/qb_journal_api.rs` `posting_comment`: when the
  posting has no `customer` and has a `name`, **and** the mapped account
  resolves (against the target journal's declarations) to Revenue type,
  `kpi: deferred-revenue`, or a receivable, write `customer: <name>`, made safe
  with `journal_safe`. Otherwise write nothing new. Vendors and employees on
  expense lines stay as they are today.
- `merge_same_account_postings` must not merge two lines with different
  customers. Check whether it already compares tags, and add the condition if
  it doesn't.
- Fixture: extend `ledgeline-export.gen.ts` with a journal entry whose
  revenue and deferred lines carry a per-line Name and no Customer column
  value. Regenerate *only* that xlsx (see the fixture-generator-churn note:
  don't commit unrelated regenerated binaries). Tests in `tests/qb_journal.rs`
  (the name is parsed per line), `qb_journal_endpoints.rs` (the tag is written
  only on the right lines) and `qb_journal_hledger_check.rs` (the output
  passes `hledger check`).

## Server (`crates/ledgeline-server/src/reports_api.rs`)

`GET /api/saas?start=&end=&dims=&detail=customers`

- `start`/`end` work as `/api/insights` does: the same defaults
  (`default_insights_start`), the same parsing and the same errors.
- `dims`: CSV of keys via `parse_csv(.., DIMENSION_KEYS_DEFAULT)`.
- `detail=customers` sets `with_customer_matrix`. Any other value is 400.
- Wire structs `WireSaasReport` and friends follow the existing camelCase,
  `Dec`-as-amount conventions. `None` is omitted (`skip_serializing_if`), so a
  household journal's response is small:
  `{period, base, available:{recurring:false, business:false, …}, months:[…]}`.
  Even then `months` carries the A-tier fields (revenue, burn, cash), since
  runway applies to any business.
- Register the route next to `/api/insights` in `lib.rs`.
- `WireInsightsReport` gains `hasHoldings`. Update
  `fixtures/native/v1/insights.json` with `just snapshot-native`, and add
  `/api/saas?start=2025-07-01&end=2026-07-08` to `requests.tsv`. sample.journal
  has no `kpi:`, which pins the "nothing available" shape.

Tests in `tests/report_endpoints.rs`:

- The startup fixture's headline figures over the default and a custom span.
- Bad dates, a bad `detail` and an empty `dims` give the same error surface as
  insights (extend the bad-date table).
- `detail=customers` adds the matrix, and its absence omits it.
- `available` on sample.journal is all false, with `months` present.

## Fixtures

`fixtures/saas/`, with a README describing each customer's story and the
expected figures it pins:

- **`startup.journal`**: the "clean" recommended structure, 2025-01 through
  2026-06 (18 months). It's generated by `fixtures/saas/generate.py`
  (deterministic, committed beside it, header comment "generated, do not
  edit"). The script exists only to write the file; the journal is the
  fixture. It contains:
  - Declarations:
    - Cash: `assets:bank:operating ; type: C`.
    - Receivable: `assets:receivable ; type: A, kpi: receivable`.
    - Payable: `liabilities:payable ; kpi: payable`.
    - Deferred: `liabilities:deferred-revenue ; type: L, kpi: deferred-revenue`.
    - Recurring revenue, three leaves:
      - `revenue:subscriptions:smb ; segment: smb`
      - `revenue:subscriptions:mid ; segment: mid`
      - `revenue:subscriptions:enterprise ; segment: enterprise`
      
      The parent `revenue:subscriptions` declares `type: R, kpi: recurring`.
      Two product lines go through a `product:` account tag on the leaves.
      One leaf instead uses posting-level `class:` on some lines, to test
      precedence.
    - Usage: `revenue:usage ; kpi: usage`.
    - Services: `revenue:services` (non-recurring).
    - COGS: `expenses:cogs:hosting ; issection: cogs`.
    - Opex: `expenses:sales-marketing`, `expenses:rnd` and `expenses:ga`,
      each with its `kpi:`.
    - `equity:funding` with `cfsection: financing`.
  - Customers, with invented names, each exercising one path:
    - **Monthly payer.** Starts in month 1 and expands in month 7.
    - **Annual prepay.** Invoiced to deferred revenue in month 3 and
      recognised monthly with `customer:`. Renews at a higher price in
      month 15 (expansion at renewal).
    - **Churns.** SMB, churns after month 9.
    - **Contracts.** SMB, contracts in month 11.
    - **Reactivates.** Starts in month 6, a zero month 9, back in month 12.
    - **Mid-market logo.** Joins in month 10 with a one-off services fee in
      the same month (it must not count as MRR).
    - **Changes segment.** Moves SMB → enterprise in month 13 (segment move:
      nets correctly at total level, churn/new per segment).
    - **Name variants.** Spelled `Contoso Ltd` in one month and `Contoso Ltd.`
      in another, to fire the variant check. It doesn't affect the expected
      metrics in a way that hides it: the README lists both keys.
    - **Credit month.** A refund credited to the recurring account for one
      month, larger than that month's charge (negative C, tests the clamp and
      the `other` line).
    - **Unattributed.** A small untagged recurring posting in two months, to
      keep coverage below 100% but above 90%.
  - Monthly COGS ≈ 20% of revenue. S&M, R&D and G&A each month. A funding
    round in month 1. AR invoices collected 30 days later, so DSO is
    meaningful. Payables paid 15 days later.
- **`clearing.journal`**: the invoice → `revenue:sales` → deferred pattern.
  SMB revenue sits on the parent's own postings and enterprise on a leaf.
  `customer:` is only on the invoices. Expect: MRR correct,
  `customers_hidden_reason = billing-clearing`, and the clearing Problems
  finding.
- **`payees.journal`**: recurring accounts tagged, no `customer:` tags, one
  invoice transaction per customer through `assets:receivable`, plus one
  month-end recognition entry that must *not* become a customer. Expect:
  customers available through payee inference, `inferred_customers = true`,
  and the recognition entry unattributed.
- **`expected/`**: hledger 1.52's own output, captured by
  `fixtures/saas/capture.sh` and committed. Never regenerate from Rust.
  - `customer-mrr.csv`: `hledger -f startup.journal bal tag:kpi=recurring --pivot customer -M --invert -O csv -b 2025-01 -e 2026-07`
  - `mrr.csv`: same, without the pivot, `--depth 1` style total row.
  - `segment-mrr.csv`: `--pivot segment`.
  - `deferred.csv`: `bal tag:kpi=deferred-revenue -M -H --invert -O csv`.
  - `billings.csv`: `bal "expr:type:R or tag:kpi=deferred-revenue" -M --invert -O csv`.
  - `cogs.csv`: `bal tag:issection=cogs -M -O csv`.
  - `cash.csv`: `bal type:C -M -H -O csv`.

  The capture script is also the docs' command list, so the docs and the
  tests can't drift apart.

## Engine tests

`crates/ledgeline-core/tests/saas_metrics.rs` (integration, fixtures):

- **hledger parity.**
  - The engine's customer matrix equals `customer-mrr.csv` cell for cell.
    Parse the CSV and compare `Dec`s, mapping hledger's blank or `0` to
    zero.
  - The per-month MRR, segment MRR (pivot), deferred balances, billings,
    COGS and cash equal the corresponding CSV.
  - This parity covers both the default and the posting-tag precedence.
    hledger's pivot uses the posting's own tag first. Where hledger and our
    nearest-wins rule could differ (parent and child both declaring
    `segment:`), the fixture avoids parent-level `segment:`, and a separate
    unit test pins our rule.
- **Hand-computed expectations.** The README lists, per customer and month,
  the bridge category. The test asserts the bridge for every month and the
  current/previous period metrics: ARR, net new ARR, customers, NRR, GRR,
  logo churn, ARPA, ACV, quick ratio, top-1 share, gross margin, burn
  multiple, runway, DSO, DPO and billings, with exact `Dec` where the value
  is decimal, and `f64` within 1e-9 for percentages.
- **Invariants**, over the fixture and a proptest generator (random customers
  × months × amounts including negatives and zero months, random
  attribution):
  - the bridge identity holds in every month;
  - Σ segments' MRR = total MRR;
  - Σ_c C[c][m] + unattributed(m) = MRR(m);
  - customers(m) = |{C > 0}|;
  - 0 ≤ GRR ≤ 100;
  - GRR ≤ NRR;
  - with no expansion, NRR = GRR.
  
  Add the proptest regressions file the way `rules.proptest-regressions` is
  kept.
- **Visibility.**
  - sample.journal: nothing available.
  - startup: everything.
  - clearing: recurring, but not customers (reason billing-clearing).
  - payees: customers through inference.
  - startup with all `customer:` tags stripped: `no-customer-tags`.
  - startup with 20% stripped: `low-coverage`, with the right `coverage_pct`.
- **Edges.**
  - A span shorter than one complete month gives `None` point metrics, no
    panic.
  - A journal starting inside the span gives NRR `None`.
  - A unit test for net burn > 0 and net new ARR ≤ 0 gives burn multiple
    `None`, and another for net burn ≤ 0 gives runway `None`.
  - Multi-currency recurring revenue with and without a price gives a valued
    amount or `unvalued`.
  - Posting `date:` tags move revenue across months.
  - Virtual and auto postings follow the income statement (pinned by the revenue-sum identity).
- **Top Transactions.** A pure move-to-deferred is skipped. A combined
  recognise+move entry is kept. A transaction with a deferred leg plus a cash
  leg is kept.
- **`kpi.rs` unit tests.**
  - Parse every value, including case and whitespace.
  - Unknown values are rejected.
  - Nearest-wins inheritance, with a child overriding the parent.
  - Name inference only fires when no `kpi:` is declared in the subtree, and
    never for a non-asset "receivable".

Also extend `insights_arithmetic.rs`:

- `has_holdings` is false on a journal without holdings, and true on
  sample.journal.
- The refactored `split_period` behaves exactly as before. The existing
  tests already cover this; add one for the odd-day split through the new
  function.

## SPA (`web/src/lib/`)

### Wire and data

- `reports/saasTypes.ts`: TS mirrors of the wire types. It's a pure types file
  under `reports/`, so it obeys the purity rule (relative imports only).
- `api/native.ts`: `saas(query: {start?, end?, dims?, detail?})`.
- `api/nativeDecode.ts`: `RawSaasReport` plus `decodeSaasReport`, strict like
  `decodeInsightsReport`. A decode test uses a captured fixture response.
  Add the request to `fixtures/native/v1/` so the golden decode test covers
  it.
- `stores/saas.svelte.ts`: `createResource` like `insights.svelte.ts`, keyed
  on the same `{start, end}`. The dashboard fetches both in parallel, and
  each section renders when its data arrives.
- `insightsTypes.ts` and the decoder gain `hasHoldings`.
- Add a live test in `nativeLive.integration.test.ts` that calls `/api/saas`
  and `/api/insights` against the live sample engine, which also adds the
  first live coverage for insights.

### Components (`reports/ui/insights/`)

- `StartupMetrics.svelte`: the section, rendered under the six headline boxes
  when `available.recurring || available.business`.
  - **Headline row** (`StatBox`, same grid). Each box only when its value
    exists:
    - **ARR** (big), with "MRR $x · usage $y" extras and a delta line vs the
      previous half.
    - **Net new ARR**, with CMGR in the small line.
    - **Customers**, with new/churned counts in the small line.
    - **NRR**, with GRR in the small line and a "trailing 12 months" label.
    - **Burn multiple**, with net burn in the small line. The colour band
      follows Sacks: < 1 green, 1–2 neutral, > 2 amber, > 3 red. The
      thresholds live in one const with a comment citing the source.
    - **Gross margin**, with the delta in percentage points.
  - **Secondary row**: a compact definition list, collapsed under "More
    metrics" by default, with the open state remembered in `localStorage`
    like other disclosures. Contents: ARPA, ACV, logo churn, quick ratio,
    top-customer share, CAC, CAC payback, S&M/R&D/G&A %, Rule of 40, DSO,
    DPO, deferred revenue, billings. Each hides when `None`.
  - **MRR bridge** list (`MrrBridge.svelte`). Start MRR → each non-zero
    category (count and amount) → Other (only when non-zero, with a tooltip:
    "unattributed revenue, credits") → end MRR, for the current half.
    Clicking a category expands its top customers from `bridge_customers`.
    Each customer links to the journal filtered to `tag:customer=<name>`,
    using the existing journal-link pattern from Subscriptions.
  - **Segment disclosure.** When `available.dimensions` isn't empty, the ARR,
    Customers and NRR boxes get a chevron that opens a `SegmentTable.svelte`
    under the row. A key picker appears when there's more than one key. The
    rows come straight from `segments`, the columns being MRR, Δ, customers,
    ARPA and NRR. `(unassigned)` is shown last.
  - **Hidden-customers note.** When `customers_hidden_reason` is set, one
    muted line explains it, linking to the docs anchor. Examples:
    - "Customer metrics hidden: 62% of recurring revenue has a `customer:`
      tag (90% needed)."
    - Billing clearing (reworded after review): "Customer metrics hidden:
      your `customer:` tags are on invoices, but the monthly
      revenue-recognition lines that make up MRR have none, so per-customer
      figures would be billings, not MRR. Tag the recognition lines to turn
      these on." See amendment A1.2 for the prefix and suffix used when
      customer counts from billings are shown.
  - **Inferred note.** When `inferred_customers` is set, an asterisk and
    footnote, as Biggest Movers does.
- `InsightsDashboard.svelte`, hide rules (decision 4):
  - Net Worth hides when `saas.available.business` or
    `report.netWorth.current` (base) < 0.
  - Investment Performance hides when `!report.hasHoldings`.
  - Biggest Movers hides when `report.movers.length === 0`.
  - **Runway** (`StatBox` testid `insights-box-runway`) appears in Net
    Worth's slot when Net Worth is hidden and `runwayMonths` exists. The big
    value is "14 months"; the small line is "cash-out ~2027-08 · burn $x/mo
    (3-mo avg)". When the business isn't burning, show "Not burning" with
    cash in the small line rather than hiding the box.
  - Hidden boxes leave no gap: the grid reflows.
- `format.ts`: `fmtMonths`, `fmtPctPoints`, `fmtRatio` (one decimal plus
  "×"), with tests.

### Settings → Accounts

`accounts/model.ts` `SPECIAL_TAGS` gains `kpi` (closed, label "Startup
metric role"), with options for each value and short help ("Marks the
account's role in startup metrics on Insights. Doesn't change any statement."),
and the doc comment names `parse_kpi_tag`. Tests extend `model.test.ts`: the
round trip, and an unknown value going to Other tags.

### Export

- `export/xlsx.ts`: `buildSaasWorkbook(report: SaasReport, styles)`, with
  sheets:
  1. **Metrics**: rows MRR, ARR, usage, customers, new, expansion,
     contraction, churn, reactivation, other, revenue, gross margin %, net
     burn, cash, deferred revenue, S&M, R&D, G&A. One column per month.
     Partial months are marked in the header.
  2. **Periods**: the current/previous period metrics side by side, with
     labels.
  3. **By segment**: per key, rows × (MRR current, MRR previous, customers,
     ARPA, NRR).
  4. **Customer MRR**: the matrix (requires `detail=customers`).
  5. **Definitions**: one row per metric, giving the formula in words, the
     roles and accounts that fed it (from `available`), the coverage %, and
     the span.

  Numbers are real numbers in the base currency, like the other exports.
- `routes/reports/+page.svelte`: the Insights tab shows the Export button only
  when `saas.available.recurring`. `runExport` fetches `detail=customers`
  for the current span, then builds and downloads
  `startup-metrics-<from>-to-<to>.xlsx`.
- Tests in `xlsx.test.ts`: the sheet names, a known cell from the fixture
  response, and that a missing matrix leaves the sheet out rather than
  writing an empty one.

### Vitest

- `StartupMetrics.svelte.test.ts`:
  - each box renders when its value exists and is absent when `None`;
  - the hidden-reason text, one case per reason;
  - the inferred footnote;
  - the segment table toggles;
  - the bridge expands and its links are right.
- `InsightsDashboard.svelte.test.ts` (create if missing), the hide rules:
  - business hides Net Worth and shows Runway;
  - negative net worth hides it;
  - no holdings hides Investment Performance;
  - no movers hides Biggest Movers;
  - a household journal keeps all of today's boxes (regression).
- Decoder and format tests as above.

### E2E (run by the lead, never by agents)

- `playwright.config.ts`: a new engine on **port 5094** serving
  `fixtures/saas/startup.journal` (read-only, served in place).
- `e2e/startupMetrics.e2e.ts`, with the clock pinned:
  - the headline boxes show the README's expected figures;
  - Net Worth is hidden and Runway shown;
  - the segment table opens with three segments;
  - the bridge's churn row expands to the churned customer and its link
    lands on the journal filtered to that customer;
  - "More metrics" opens;
  - the export downloads a workbook with five sheets.
- `e2e/insights.e2e.ts`: assert that sample.journal shows **no** startup
  section and still shows Net Worth and Investment Performance.

## Docs

- **`docs/reports/startup-metrics.md`** (new; `order: 15`;
  `tags: [reports, account-tags]`; description quoted):
  1. What it shows and when. The tiers, in plain words.
  2. **Setting up**: `kpi:` (a table of values), dimensions, `customer:`, with
     a minimal journal that turns everything on.
  3. **Each box**: definition, formula, what hides it, and the common
     disputes (ARR vs run-rate, NRR cohort, burn multiple on cash).
  4. **The MRR bridge**, including the Other line, reactivation, and segment
     moves.
  5. **Deferred-revenue books**: the invoice → revenue → deferred pattern,
     why customer figures hide, and how to fix it (a leaf per segment,
     `customer:` on recognition lines, QuickBooks per-line Name).
  6. **Check it with hledger**: the command table, taken from
     `fixtures/saas/capture.sh`, plus the traps (anchor `'^revenue'`; `tag:`
     vs `--pivot` inheritance; no "count distinct"; `issection` is
     Ledgeline's tag).
  7. **Export.**
  8. **Pitfalls**: invoice and deferral in different months, prepay booked
     straight to revenue, proration, credits, recognition date, FX, name
     variants.
- `docs/reports/insights.md`: a section on the hide rules and Runway, linking
  to the new page. Update its frontmatter description ("ten boxes" is no
  longer always true).
- `docs/reports/index.md`: a row in the tab table.
- `docs/settings/accounts.md`: a `kpi:` row in the field table.
- `docs/journal/problems.md`: four new checks in the table, plus sections.
- QuickBooks import page (`docs/imports/…`): per-line Name → `customer:`.
- `docs/development/perf-baseline.md`: the saas engine timing.
- A screenshot of the startup section for the new page via the docs-screenshot
  harness (`playwright.docs.config.ts`), if it can serve the startup fixture.
  Otherwise note it in TODO.md.
- Build check: `mbr -b docs` (or the `docs.yml` command) passes with no
  broken links.

## Deferred (`TODO.md`)

- Counting commodities for tier D (headcount, units) and revenue per FTE.
- Inventory metrics (`kpi: inventory`, turns, DIO, GMROI) on top of Holdings
  lots.
- A churn grace period (*N* zero months).
- The "spread invoices over term" fallback for books that can't tag
  recognition lines.
- Magic number. Cohort retention chart.
- The default projection treating recurring revenue as run-rate.
- AR aging.

## Execution

| Step | Owner                         | Scope                                                                 | Gate                                                                                         |
|------|-------------------------------|-----------------------------------------------------------------------|----------------------------------------------------------------------------------------------|
| 1    | Engineer A (this worktree)    | Fixtures + capture, `kpi.rs`, `saas.rs`, insights changes, engine tests, Problems checks, QB per-line Name (core + server), `/api/saas` + wire + endpoint tests + native golden | `just engine-check`, `just engine-test`, the new tests named above all pass                  |
| 2a   | Engineer B (after 1)          | SPA: types, decoder, store, components, hide rules, settings, export, vitest, live test, e2e spec written but not run | `just check`, `just test`, `just lint`, `just test-integration`                              |
| 2b   | Writer C (parallel with 2a, docs/ only) | Docs listed above, except screenshots                       | `mbr -b` build is clean                                                                      |
| 3    | Lead                          | Run e2e (ports free), screenshots, perf figure, full pre-push suite, TODO.md, review of the whole diff | Everything green                                                                             |

Agents don't run Playwright. Steps 2a and 2b touch disjoint trees (`web/` vs
`docs/`). The lead resolves the one shared seam: label text in the UI vs
the docs.

## Implementation notes (step 1)

Engineer A, 2026-11-10. Where the plan was ambiguous or didn't work as
written, these are the choices made. The wire shape below is final, and the SPA
builds against it.

### `GET /api/saas` wire shape

Query: `start`, `end` (as `/api/insights`), `dims` (CSV, default
`segment,product,class,region`), `detail` (`customers` or absent), and
`asOf` (`YYYY-MM-DD`, default the server's UTC today via `today_utc`, as on
`/api/subscriptions`; bad or empty values are a 400 naming `asOf`, exactly
like `start`/`end`).
- `detail` with any other value is a 400: `unknown detail 'x' (expected customers)`.
- An explicit `dims` that names no key (`dims=`, `dims=,`) is a 400:
  `dims must name at least one dimension key (e.g. segment,product)`.
- Bad dates give insights' messages.

Serialisation rules:
- `Dec` is `{mantissa: string, places: number}`, as elsewhere on the native
  wire. Values valued through a price can carry more than 2 places, and
  `x × 12` is normalised (ARR `70200`, places 0).
- Percentages are numbers in percent units (62.5 means 62.5%). This includes
  `customerGrowthPct` and `logoChurnPct`.
- Ratios are plain numbers.
- Every `Option` is omitted when `None`. Empty `segments`/`topCustomers` and an
  empty `unvalued` are omitted too.

```text
WireSaasReport {
  period: { start, mid, end, prevStart, prevEnd, currStart, currEnd }   // same as /api/insights
  base: string
  available: {
    recurring: bool, customers: bool, business: bool, cogs: bool,
    dimensions: string[],                       // keys offered, in request order
    customersHiddenReason?: "no-customer-tags" | "low-coverage" | "billing-clearing",
    coveragePct?: number,                       // 0–100, present whenever recurring is true
    inferredCustomers: bool
  }
  months: [{                                    // every calendar month intersecting [start, end]
    month: "YYYY-MM", partial: bool,
    mrr: Dec, usage: Dec, unattributed: Dec,
    customers?: number, bridge?: Bridge,        // only when available.customers
    revenue: Dec, cogs?: Dec,                   // cogs only when available.cogs
    netBurn: Dec, cashEnd: Dec,
    sm: Dec, rnd: Dec, ga: Dec,
    deferredEnd?: Dec, arEnd?: Dec, apEnd?: Dec // only when that role resolves somewhere
  }]
  current: PeriodMetrics
  previous: PeriodMetrics
  segments?: [{ key, rows: [{ value, mrrCurr: Dec, mrrPrev: Dec, customersCurr?: number,
                              arpa?: Dec, nrrPct?: number, bridge?: Bridge }] }]
                                                // rows by mrrCurr desc, "(unassigned)" last
  bridgeCustomers?: { new, expansion, contraction, churn, reactivation:
                      [{ customer, name, segment?: string, amount: Dec, inferred: bool }] }
  topCustomers?: [{ customer, name, mrr: Dec, pct: number }]   // ≤ 10, at current.month
  customerMatrix?: [{ customer, name, segment?: string, values: Dec[] }]  // aligned to months
  unvalued?: { [commodity]: Dec }
}
Bridge { new, expansion, contraction, churn, reactivation, other: Dec,
         newLogos, churnedLogos, reactivatedLogos: number }
PeriodMetrics {
  month?: "YYYY-MM",                            // the half's last complete month m
  months: number,                               // k
  mrr?, arr?, usage?: Dec, mrrGrowthPct?, cmgrPct?: number, netNewArr?: Dec,
  bridge?: Bridge, customers?: number, customerGrowthPct?, logoChurnPct?: number,
  arpa?, acv?: Dec, nrrPct?, grrPct?, quickRatio?, top1Pct?, top5Pct?, top10Pct?: number,
  revenue: Dec, grossProfit?: Dec, grossMarginPct?: number,
  netBurn: Dec,                                 // Σ NetBurn over F (added; feeds burn multiple)
  netBurnAvg3?: Dec, cash?: Dec, runwayMonths?: number, cashOutDate?: "YYYY-MM-DD",
  burnMultiple?, ruleOf40?: number,
  sm?, rnd?, ga?: Dec, smPct?, rndPct?, gaPct?: number,
  cac?: Dec, cacPaybackMonths?: number,
  deferredRevenue?, billings?: Dec, dsoDays?, dpoDays?: number
}
```

On a household journal (sample.journal) the response is `period`, `base`,
`available` (all false, `dimensions: []`), `months` (tier-A fields only; mrr,
usage, sm and so on are zero), and `current`/`previous`. Those last two are
always present, because they carry the tier-A revenue, net burn, cash and
runway that the Runway box needs on any journal.

`/api/insights` gains `hasHoldings: bool`.

### Engine choices

- **Top Transactions skip rule.** The plan says "its revenue legs net to zero".
  A pure move to deferred *debits* revenue by the amount it defers, so that
  reading never fires. Implemented as: every leg is revenue-typed or
  `kpi: deferred-revenue`, at least one leg is deferred, and the revenue legs
  recognise nothing (they net to zero or to a debit, never a credit). This
  skips the pure move and keeps the combined recognise+move entry and anything
  with a cash leg, the three cases the plan names. The skip happens in
  `is_deferral_move` in `insights.rs`.
- **Payee fallback** additionally requires a non-empty payee. A description
  like `| Invoice` names no one.
- **Coverage** = (Σ MRR − Σ unattributed) / Σ MRR over the span's complete
  months, clamped to 0–1. Inferred postings count as attributed.
- **Hidden-reason precedence**: billing-clearing, then no-customer-tags
  (coverage 0), then low-coverage.
- **billing-clearing** (revised after review; see "Review fixes" below): only
  the span's complete months count. A *clearing account* is a recurring
  account debited in a transaction that credits deferred revenue. A *clearing
  posting* is a credit to such an account in a transaction with no
  deferred-revenue leg. The reason fires when more than 50% of attributed
  recurring revenue sits on clearing postings.
- **Tier gating**:
  - B: mrr/arr/usage, growth, cmgr, net new ARR and burn multiple need
    `available.recurring`.
  - sm/rnd/ga need their role to resolve somewhere. deferredRevenue and
    billings need the deferred role. DSO needs a receivable (inferred names
    count), and DPO needs a payable.
  - C (bridge, customers, NRR/GRR, ARPA, ACV, quick ratio, top-N, CAC, CAC
    payback) needs `available.customers`.
  - Dimensions are offered only when `recurring` is true.
- **Tier A in the period struct**: `revenue` and `netBurn` are always present
  (zero when the half has no complete month). `cash` is `None` when the half
  has no complete month.
- **Months and partial months**: postings dated after `end` are ignored. A
  partial *first* month includes its postings before `start`: the engine reads
  the whole journal and buckets by calendar month. Partial months are never a
  period's *m*.
- **NetBurn** comes from `cash_flow_statement` (monthly, from the walked range's
  first month to `end`) as −(netChange − financing.total). It therefore equals
  −(operating + investing + exchange-rate effect). It is valued in base at the
  month end. A test pins NetBurn = −(operating + investing) on the fixture.
- **Cash** (point balance) uses `AccountTypes::is_cash`, the memoised
  `cash_predicate`.
- **DPO** denominator is all Expense-typed spend over F (COGS + every other
  expense).
- **Rule of 40** compares revenue over F with the *k* months immediately before
  F. For the current half of an even split, that is exactly the previous half's
  F. It is `None` when that prior revenue is ≤ 0, which is the case for the
  previous YoY half on the fixture.
- **Runway** is cash⁺ × n / Σ burn over the last n ≤ 3 complete months.
  - It uses the exact sum, not the rounded average.
  - Negative cash reads as 0 months.
  - `cashOutDate` is the last day of month m + ⌊r⌋, plus round(frac × days in
    the following month) days.
  - It is omitted beyond 1,200 months.
- **Money ÷ count** (ARPA, ACV, CAC, 3-month burn) uses `Dec::div_int` at the
  input's scale (at least cents), rounding half away from zero.
- **Valuation**: each month's sums are valued at that month's end with the
  Insights price set (`all_prices`: inferred + explicit `P`).
  - Unpriced recurring and usage revenue goes into `unvalued`, with the
    revenue sign positive.
  - Other unpriced amounts are dropped silently, as Insights' boxes drop them.
- **Bridge customers** sums each customer's amount per category over the current
  half's complete months, top `top_n` (5) by amount. `segment` is the first
  requested dimension key's value in the move month (for churn, the month the
  revenue left from).
- **customerMatrix** appears only with `detail=customers` *and*
  `available.customers`. Each row's `segment` is the first dimension key's value
  in the customer's latest positive month.
- **Walk cap**: the engine walks at most 1,200 months back from `end`, the same
  bucket cap `range_buckets` uses. Older postings still count toward balances.
- `split_period` is `pub` and re-exported, and `effective_date` (the
  posting-date rule) is factored out of `account_totals` and shared.

### Problems checks (`wire.rs`)

- **`kpi` closed tag**: reported under the existing `account-tag` rule.
- **`kpi-type-mismatch`**:
  - Severity warning. Emitted by `journal_to_tag_diagnostics` and anchored to
    the account.
  - Fires only on an account that *declares* a readable `kpi:` itself. An
    inherited, inferred or untyped role never fires.
- **`recurring-billing-clearing`**:
  - Severity `info` (new `DIAGNOSTIC_INFO`).
  - One finding per recurring account, anchored to the first such
    transaction's `txnIndex`. The account is named in the message, because the
    drawer drops `account` when `txnIndex` is set.
  - Emitted by the new `journal_to_saas_diagnostics`, which runs after the tag
    pass in both `journal_to_all_diagnostics` and `view::checks`.
- **`customer-name-variants`**:
  - Severity warning, anchored to the transaction.
  - Normalisation: lowercase, split on non-alphanumerics, drop one trailing
    legal-suffix word (only if another word remains), then join. So "Acme Co"
    becomes acme while "Costco" stays costco.
  - hledger ends a tag value at a comma, so `Acme, Inc.` is read as `Acme`.
- `DIAGNOSTIC_RULES` grows from 8 to 11.
- **SPA**: `ProblemsDrawer.svelte` `RULE_LABELS` needs labels for
  `kpi-type-mismatch`, `recurring-billing-clearing` and
  `customer-name-variants`.

### QuickBooks per-line Name

- `QbPosting.name` is `#[serde(default)]`.
- `posting_comment` writes `customer: <name>` when all of these hold:
  - the line has no Customer value,
  - it has a Name, and
  - the alias-mapped account resolves, against the target journal's
    declarations, to Revenue (Gain included), `kpi: deferred-revenue`, or a
    receivable (declared or name-inferred).
- `merge_same_account_postings` already compares the rendered comment, so
  lines for different customers never merge. A test pins this.
- Fixture txn 15 in `ledgeline-export.gen.ts`/`.xlsx`: only that workbook was
  regenerated, and it is tracked (not gitignored).
- Note: a salary line aliased to a revenue account also gets `customer:` (the
  rule as written).

### Not done in step 1

- The perf figure is not recorded: `docs/` belongs to step 2b, and the timing
  belongs to the lead. A `reports/saas` bench now sits beside
  `reports/insights` in `benches/engine.rs`. Run it with `just perf-corpus` +
  `cargo bench --bench engine -- reports/saas`.
- The SPA's `nativeDecode.test.ts` needs a `saas` decoder now that
  `fixtures/native/v1/saas.json` exists.

### Review fixes (step 1, round 2)

- **Billing clearing** (`saas.rs`, `ClearingRule`):
  - A pre-pass finds the recurring accounts debited in a transaction with a
    deferred-revenue credit, inside the span's complete months.
  - A clearing posting is a CREDIT to one of those accounts, in those months,
    in a transaction with no deferred-revenue leg. Recognition lines never
    are.
  - The reason fires when Σ attributed recurring revenue on clearing postings
    is more than 50% of Σ attributed recurring revenue in the span's complete
    months.
  - The payee fallback never attributes a clearing posting.
  - Tests:
    - well-booked books with one correction inside and one outside the span
      keep customers;
    - `clearing.journal` with `customer:` moved onto the recognition lines
      shows customers, with no "Invoice" payee customer;
    - `clearing.journal` as committed is still `billing-clearing`.
- **As of**:
  - `SaasOpts.as_of: Option<&str>`. A month is complete only when its last day
    is on or before min(span end, as_of).
  - Months after as_of are flagged `partial`.
  - Server param `asOf` as above. `fixtures/native/v1/requests.tsv` pins
    `asOf=2026-07-08`; `saas.json` is byte-identical.
  - The e2e (span ends 2026-06-30) is unaffected.
- **Inferred per month**: inferred customers are tracked per (customer, month).
  - `CustomerMove.inferred` is true when the month a move is measured on is
    payee-inferred (q's month, or p's for churn), in any month the move
    covers.
  - `available.inferredCustomers` is any inferred share in the current half's
    *m*.
- **`customer-name-variants`**: a tag set (posting or transaction) carrying
  `customer-id:` is excluded from name grouping entirely.
- **QuickBooks**:
  - Every tag value written by `posting_comment` (class, customer, vendor,
    per-line Name) goes through `tag_value`: journal-safe, commas become
    spaces, spaces are collapsed. So `Acme, Inc.` becomes `Acme Inc.`; the memo
    keeps its commas.
  - The fixture's per-line Name is now `Globex, Ltd.`, written and read back
    by hledger as `Globex Ltd.`.
  - Re-importing an export committed by an earlier version reports the
    recognition entry as changed (postings 4 → 6). `updateChanged` splits it
    once, and a tag-only difference is not a change. This is pinned by a test
    and documented in `docs/imports/quickbooks.md`.

## Amendment A1 (Patrick, 2026-11-10): adjusted burn, customers from billings, Top Transactions by P&L impact

Feedback after the first build:
1. Runway is naive when big annual prepayments make cash in lumpy.
2. The reference books tag `customer:` on invoices only. Customer metrics are hidden there (correctly), but counts from billings are wanted as well.
3. The same invoice → receivable → payment → deferral flow fills Top Transactions with near-duplicates.

Decided:

### A1.1 Burn adjusted for prepayments and receivables

`NetBurnAdj(month) = NetBurn(month) + Δdeferred(month) − ΔAR(month)`

- Δdeferred is the change in `kpi: deferred-revenue` balances, positive when the liability grows.
- ΔAR is the change in receivable balances (declared `kpi: receivable`, or the documented name fallback).

The adjustment reads burn as if every customer paid when served:
- An annual prepayment raises cash and deferred revenue by the same amount, and nets to 0.
- Each month's recognition lowers deferred revenue, and counts as income.
- An unpaid invoice counts when it's issued; its later payment nets to 0.

Rules:
- `netBurnAdjusted` per month, and `netBurnAdjustedAvg3` per period, exist only when a deferred-revenue or receivable account resolves.
- When they exist, **runway and cash-out date use `netBurnAdjustedAvg3`**. The period carries `runwayBasis: "adjusted" | "cash"`.
- Raw `netBurnAvg3` stays in the response, and the Runway box's small line shows both ("burn $x/mo adjusted for prepayments · $y/mo cash, 3-mo avg").
- Burn multiple keeps raw net burn, which is the standard definition. The docs say so.

Tests:
- A fixture month with a large prepayment: raw burn swings, adjusted burn doesn't.
- The identity `Σ NetBurnAdj = Σ NetBurn + (deferred_end − deferred_start) − (AR_end − AR_start)` over any range.
- Runway basis selection.

### A1.2 Customers from billings

A **billing** is a customer-attributed credit to a recurring or deferred-revenue account, in a transaction with a receivable or cash leg. Attribution is by `customer:`, `customer-id:` or the payee fallback (flagged inferred).

`BilledActive(m)` = customers with a billing in months m−11 … m.

When the per-customer MRR metrics are hidden **and** billings exist, the period gains `billingCustomers`:
- `customers`: |BilledActive(m)|.
- `customerGrowthPct`: vs the month before the half.
- `newLogos`: customers whose first-ever billing falls in F.
- `lapsedLogos`: customers active at the start of a month in F and not at its end, i.e. 12 months since their last billing.
- `logoChurnPct`: lapsed ÷ BilledActive at the month before the half.
- `acv`: trailing-12-month billings at m ÷ |BilledActive(m)|.
- `windowShort`: true when the journal starts less than 12 months before m.

`available.billingCustomers` is true in that case. NRR, GRR, the bridge, ARPA and concentration stay hidden.

UI:
- The Customers box shows the billing count, titled **"Customers (from billings)"**, with extras "N new · N lapsed" and its own help text.
- More metrics shows **"ACV (from billings)"** and logo churn.
- The hidden note becomes "Per-customer MRR figures hidden: … Customer counts are from billings."
- A `windowShort` footnote says the count may be low until there is 12 months of history.

Tests:
- `clearing.journal` (billings only) gives the expected counts.
- A customer lapses after 12 months without a billing.
- Payee-inferred billings set the inferred flag.
- Hidden when nothing is billed.
- Never shown when the MRR customer metrics are available.

### A1.3 Top Transactions ranked by income and expense impact

- A transaction's score is the larger of the debit-side and credit-side sums of its **revenue- and expense-typed** postings, valued in the base commodity.
- Revenue postings that are **debits** in a transaction with a deferred-revenue **credit** are moves to deferred, and are left out.
- Transactions scoring 0 drop out. These are payments, transfers and pure deferral moves.
- Closing entries are skipped too (`is_closing_entry`).
- The displayed amount is the score.

Expected:
- A card purchase shows its expense.
- A paycheck shows its gross revenue, not revenue plus withholding.
- A payment from a customer drops out.
- A combined recognise-and-move entry shows only what it recognises.
- The invoice shows its revenue.

Tests: one per case above, plus the existing Top Transactions tests updated where the figure changes. If sample.journal's golden/e2e figures move, capture the change and explain it.

## Implementation notes (A1)

Engineer A, 2026-11-10. Final wire additions to `GET /api/saas`, all
camelCase; `None` is omitted.

- `available.billingCustomers: bool`
- `months[].netBurnAdjusted?: Dec`. Present when a deferred-revenue or
  receivable account resolves, including a receivable inferred from its name.
- `current` / `previous`:
  - `netBurnAdjustedAvg3?: Dec`
  - `runwayBasis?: "adjusted" | "cash"`. Present whenever the half has a
    complete month.
  - `billingCustomers?: { customers: number, customerGrowthPct?: number,
    newLogos: number, lapsedLogos: number, logoChurnPct?: number, acv?: Dec,
    windowShort: bool, inferred: bool }`

Choices:
- **A1.1 adjusted burn**:
  - Computed from month-end balances valued in base: Δdeferred is the change
    in the owed-positive deferred balance, ΔAR the change in the receivable
    balance.
  - When the adjusted burn exists, `runwayMonths` and `cashOutDate` use the
    exact 3-month sum of adjusted burn (same months and count as
    `netBurnAvg3`). Otherwise they use cash burn.
  - `burnMultiple` still uses raw net burn.
- **A1.2 billings**:
  - A billing is a credit to a `recurring` or `deferred-revenue` account in a
    transaction with a receivable or cash leg.
  - Attribution order: posting `customer-id:`, posting `customer:`,
    transaction `customer-id:`, transaction `customer:`. Otherwise the payee
    (flagged inferred), but only when the transaction holds exactly one such
    credit.
  - `available.billingCustomers` = `recurring` && !`customers` && any billing
    exists. Each half then carries `billingCustomers`.
  - `lapsedLogos` counts, for each complete month j of the half, the customers
    active at j−1 and not at j. `acv` uses trailing-12-month billings, and
    `inferred` covers the active customers' window.
  - `available.inferredCustomers` is also set when the current half's billing
    count includes payee-inferred billings.
- **A1.3 Top Transactions**:
  - `pnl_impact` in `insights.rs` scores each transaction: the larger of the
    debit and credit sums of its revenue- and expense-typed postings, each
    amount valued in base at its posting date with the Insights price set.
  - Revenue debits in a transaction with a deferred-revenue credit are left
    out.
  - Closing entries and the old `is_bookkeeping_only` are skipped; a score of
    zero drops out.
  - `is_deferral_move` (step 1) is removed. It skipped the combined
    recognise-and-move entry that A1.3 says must show what it recognises, and
    the new scoring already drops pure moves.
  - The old unit test "keep a plain transfer" is inverted: transfers drop out.
- **sample.journal Top Transactions**:
  - Unchanged on the native span (2025-07-01…2026-07-08) and on the YoY span
    (2024-07-01…2026-06-30). Both are five $5,660.00 salaries.
  - On a 2026-05-01…2026-06-30 span (current June), the old #4 "Fidelity |
    sell TSLA" ($630) and #5 "Chase | visa autopay" ($300) drop out. They are
    replaced by "Hotel Laimer Hof | lodging, Munich" ($239.82) and "Safeway |
    weekly groceries" ($184.15). The top 3 are unchanged: June salary $5,660,
    vehicle depreciation $3,500, rent $1,875.
- **Native goldens**: `saas.json` gains only `available.billingCustomers:
  false` and `runwayBasis: "cash"` on both halves. `insights.json` is
  unchanged.

## Implementation notes (A2: data horizon)

Engineer A, 2026-11-10.

**Bug.** A span ending after the books' last month-end close read the unclosed
months as total churn. For example, startup.journal over 2024-11-01 …
2026-10-31 as of 2026-11-10 picked m = 2026-10 with MRR 0.

**Fix.** The horizon is the last calendar month holding any `kpi: recurring`
posting dated on or before the span's end (`recurring_horizon` in
`saas.rs`).
- A month is complete only when its last day is on or before min(span end,
  asOf) *and* it is not after the horizon. In practice the effective as-of
  becomes min(asOf, last day of the horizon month).
- Every period metric (point, flow, trailing, the bridge sums, burn averages,
  runway) uses those same complete months.
- Months after the horizon that have started by min(asOf, end) come back with
  `partial: true` and `open: true`. Their A-tier fields (cash, burn) are still
  filled in, but nothing uses them.
- A month that starts after asOf is only `partial`.
- With no recurring revenue at all there is no horizon, and the behaviour is
  as before.

Wire additions to `/api/saas`:
- `available.closedThrough?: "YYYY-MM"`: the horizon, present whenever
  `recurring` is true.
- `months[].open?: true`: omitted when false.

Tests:
- startup.journal over 2024-11-01 … 2026-10-31 as of 2026-11-10 gives
  m = 2026-06, closedThrough 2026-06, and 2026-07…10 open. MRR, ARR,
  customers, NRR, GRR, top-1 share, cash, deferred revenue and runway equal
  the YoY span's. Logo churn is 0%.
- A journal whose last recognition is in 2026-03 stops there, with 04…06
  open.
- A journal with no recurring revenue has no horizon and m = 2026-10.
- The step-1 as-of tests now use books extended through 2026-12.

The native goldens are unchanged (sample.journal has no recurring revenue).

## Implementation notes (A3: burn multiple basis)

Engineer A, 2026-11-10. Decision (Patrick): the burn multiple uses ADJUSTED
burn whenever it exists.
- `burnMultiple` = Σ NetBurnAdj over the half's complete months ÷ netNewArr,
  when `netBurnAdjusted` exists (a deferred-revenue or receivable account
  resolves). Otherwise it is Σ raw NetBurn ÷ netNewArr.
- It is `None` when either the chosen burn or netNewArr is ≤ 0.
- This reverses A1.1's "burn multiple keeps raw net burn".

Wire additions to `current` / `previous` on `/api/saas`:
- `netBurnAdjusted?: Dec`: Σ adjusted net burn over the half.
- `burnMultipleBasis?: "adjusted" | "cash"`: present whenever `burnMultiple`
  is. It is the same enum as `runwayBasis` (`RunwayBasis` in Rust).
- Raw `netBurn` stays.

On startup.journal over the YoY span (2024-07-01 … 2026-06-30, asOf
2026-07-08), with basis adjusted:

| Half | Burn multiple | Was (raw) |
|---|---|---|
| Current | 44,070 / 27,000 = 1.6322 | 1.6193 |
| Previous | 29,520 / 43,200 = 0.6833 | 0.5583 |

The native goldens are unchanged: sample.journal has no `burnMultiple`.
