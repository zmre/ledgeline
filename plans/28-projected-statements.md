# 28 — Projected statements: the official reports, run forward

Replace the Projections tab's Net income and Net worth readings with the three
official statements — balance sheet, income statement, cash flow statement —
computed over the real journal up to today **plus** the scenario's forecast
transactions after it. Same sections, groups, subtotals, valuation and checks as
the Reports tab, because it is the same code. Keep the runway reading, renamed
**Runway**.

To get there, the engine stops summing a scenario into per-bucket totals and
instead **expands it into dated, balanced transactions**, the way
`hledger --forecast` does. That is what makes a scenario file mean the same
thing to Ledgeline and to hledger.

## The ask (Patrick, 2026-10-23)

> are we able to produce our various reports (like balance sheet, cash
> statement, p&l) with the projections data? so we could show a short ways
> back and then out into the future with those assumptions built in? i'd love
> to add our official reports to the projections page using the same groupings
> and sections and calculations, but looking forward. i'd want to keep the
> runway (and rename to just "runway" from "cash & runway") but replace the
> others with our official reports taking assumptions into account.

> we want to use projections both for individuals and for companies and we
> want the projections to be useful in all cases. runway is generally a concept
> for startup businesses. for individuals, it tends to be about whether or not
> they're saving enough for the future. … for now i think the major reports
> will capture more and have better consistency with what's in the reports
> section.

> remember that our projections need to be compatible fully with hledger so
> that hledger commands can also make use of them.

> Regarding the generation of future P prices -- I am expecting this to be a
> ledgeline only feature and for those to be basically in memory. Having to
> generate and save to disk and then modify and then maybe in the future
> conflict seems bad. Lets have stock appreciation stuff be captured by our
> growth tags, implemented with future price changes, but not persisted.

> Lookback default of 2 periods will be enough. But if we're mid period (like
> middle of october or if mid-month is the last updated transaction) then we
> will look back at the full periods so for monthly, september and august, and
> for october and forward we'll use the projections.

## Why this is a format change, not just a new view

The three statements are pure functions over `&[Transaction]`
(`balance_sheet_grouped` `reports/balance_sheet.rs:300`,
`income_statement_columns` `reports/income_statement.rs:920`,
`cash_flow_statement` `reports/cash_flow_statement.rs:381`). None reads a clock,
a source span or the journal's range, so a merged
`real ≤ as_of ++ forecast > as_of` transaction list goes through them
unchanged. The merged list is the whole seam.

What blocks it today is the projection model, not the reports:

1. **Projection lines are unbalanced virtual postings with an implied cash
   leg** (plan 22 §"What moves cash", the residual rule). The implied leg has no
   account. The cash flow statement drops virtual postings outright
   (`cash_flow_statement.rs:404`), the balance sheet would not balance, and —
   the hledger problem — `hledger bs --forecast` over today's scenario files
   shows salary in the P&L and never in any bank account.
2. **Asset appreciation is a net-worth-only delta** (plan 23 amendment 7) with
   no counter-account, so a balance sheet built from postings cannot see it.
3. **The engine never materializes postings** (`project()`,
   `projections.rs:1085-1297`); cash and net worth leave it as single totals.

## Decisions

Locked with Patrick 2026-10-23 (all eight):

1. **Every projected line names a counter-account, written in the file.** A
   rule becomes an ordinary balanced hledger periodic transaction:

   ```journal
   ~ monthly  projection
       revenues:salary   $-12000  ; growth: 3%/yr
       assets:checking
   ```

   When a line has none — an old scenario file, a `budget.journal` rule, a
   freshly seeded estimate — Ledgeline **picks a default** (Decision 7) and
   writes it on save. The point is cash: "we care about how cash changes over
   time", and a statement can only show that if the money lands somewhere. This
   supersedes plan 22's "there is no funding leg and none is wanted" and the
   residual rule.
2. **`growth:` stays a Ledgeline-only tag.** Expanding growth into stepped rules
   would be one rule per step for years or decades. hledger still reads the
   file; it shows the base amount without growth. This is the one accepted
   deviation, already stated in plan 22 Decision 7, and the docs keep saying
   so.
3. **Two kinds of appreciation, two mechanisms, both driven by `growth:`:**
   - **Priced holdings (stocks, funds):** an asset row's `growth:` on an
     account that holds priced commodities grows those commodities' **prices**.
     The engine builds future prices **in memory only**. They are never written
     to the scenario file, the journal or a prices file, so there is nothing
     to regenerate, edit or conflict with later. The statements' existing
     market valuation revalues the holding. No counter-account is needed: the
     balance sheet's valuation adjustment (`balance_sheet.rs:370`) already
     keeps A = L + E, as hledger's `--value=end` does. hledger, which never
     sees these prices, values the holding at its last real price. That is the
     same accepted deviation as Decision 2.
   - **Unpriced assets (a home, a private stake):** appreciation is a
     **posting** from the asset to a gains account the user picks, defaulting
     to **equity, not income**. That keeps plan 23 Decision 2 — paper gains do
     not touch net income or cash — unless the user deliberately points the
     gains at an income account.

4. **The gains account is a real, declared account, never a virtual one.**
   The user picks it on the asset row. The default is `equity:unrealized gains`.
   If the main journal does not declare it, the **scenario file** declares it
   (`account equity:unrealized gains  ; type: E`), so hledger and Ledgeline
   classify it the same way and the user's main journal is not touched.
   Acceptable because nothing includes a scenario file by default. A
   virtual `(account)` would be dropped by the cash flow statement, unbalance
   the balance sheet, and mean something different to hledger. Verified:
   hledger 1.52 accepts the same account declared in both files, including
   under `check --strict`.
5. **Projection starts at the first period that is not complete; there is
   no mixed period.** The cutoff is the start of the period containing the
   earlier of today and the day after the latest real transaction. Past
   periods before it are actuals; it and everything after are projected. Real
   transactions dated on or after the cutoff are left out of the projected
   statements.
   - Today 2026-10-23, monthly: August and September are actuals; October
     onward is projected, and October's real transactions so far are left out.
   - Latest real transaction 2026-10-15 while today is in November: October
     is incomplete, so it is projected too.
   - Latest real transaction 2026-10-31: the day after is 2026-11-01, so
     October is complete and is an actual.

   The opening position is the balance at the cutoff, from real transactions
   only. This keeps plan 22 Decision 9's point, that no column is a partial
   period, and replaces its "next whole bucket after today" rule. The hledger
   equivalent is in §"Verified against hledger 1.52".
6. **Lookback: 2 full periods, user-adjustable.** On the forecast's interval;
   0 is allowed. The statements mark where actuals end.
7. **Default counter-account: the most common pairing in history, then a
   fallback.** For a
   line's account, use the cash-typed account it was most often posted against
   over the seed's trailing window. Salary that always landed in
   `assets:checking` projects into `assets:checking`. Otherwise use the
   scenario-level default (a new header tag, `; cash: assets:checking`).
   Otherwise use the cash account with the largest balance at `as_of`.
   Editable per line in the table.
8. **Runway counts cash only** (`is_cash` accounts, as today). Selling
   stock on a schedule to fund spending is a later modelling question, not a
   reason to count investments as runway.

## Verified against hledger 1.52 (2026-10-23)

A main journal (checking, a house, 10 AAPL at $100) and a scenario file with
balanced `~` rules, a yearly house-appreciation rule to
`equity:unrealized gains`, and a future `P 2027-06-01 AAPL $150`:

- `hledger -f main.journal -f scenario.journal bs --forecast=2026-11-01..2027-07-01 -Q --value=end,$`
  showed checking growing $3,000/mo, the house stepping to $520,000, and the
  brokerage revaluing from $1,000 to $1,500 at the 2027-06-30 column.
- `is` and `cf` with `-M` agreed: $5,000 revenue, $2,000 rent, $3,000/mo into
  checking.
- An **explicit** `--forecast=PERIOD` generated rules from its own start even
  though a real transaction existed later (2026-12-20). The December column
  held both the real transaction and the forecast. That is why Decision 5
  pins the boundary: Ledgeline's cutoff and the `--forecast=` start must be
  the same day, or the two tools double-count differently.
- Declaring `equity:unrealized gains` in both files passed
  `check --strict`.
- **Leaving out real transactions after the cutoff (Decision 5).**
  `expr:"date:..CUTOFF OR tag:generated"` is refused ("using date: in OR
  expressions is not supported"). The NOT/AND form works:
  `expr:"NOT (date:CUTOFF.. AND NOT tag:generated)"`. It dropped a real $1
  December transaction and kept December's forecast. Exporting actuals first
  (`hledger print -e CUTOFF > actuals.journal`, then
  `-f actuals.journal -f scenario`) gave the same figures.

The docs will give the exact equivalent command for any projection:

```sh
hledger -f <main> -f <scenario> bs|is|cf --forecast=<cutoff>..<end> \
  -b <lookback start> -e <end> -<interval> \
  expr:"NOT (date:<cutoff>.. AND NOT tag:generated)"
```

hledger will differ only by growth: amounts (Decision 2) and future prices
(Decision 3), both of which it shows flat.

## Scope

In: forecast expansion into transactions; counter-accounts (model, file,
inference, table); asset gains accounts and commodity price trends; a
multi-column balance sheet; the run endpoint returning the three statements
plus runway; the tab's new readings; docs; hledger parity tests.

Out, and unchanged from plans 22/23: liability amortisation; tax; Monte Carlo;
comparing scenarios; savings-adequacy views for individuals (Patrick: "we'll
come back to things we might want to show or model for that"); writing growth
out as stepped rules.

---

## Phase 1 — Forecast expansion (core)

`crates/ledgeline-core/src/projections.rs`:

- `forecast_transactions(scenario, opts, types, history) -> Forecast { txns:
  Vec<Transaction>, prices: Vec<PriceDirective>, warnings }`. It is pure, with
  no clock. For each line it walks `budget::occurrences` (the walk the budget
  report and `place_flow` already share), applies stepwise growth
  (`growth_steps`/`grow_once`, unchanged), and emits one balanced transaction
  per occurrence: the line's posting plus its counter-account posting.
  Generated transactions carry `generated-transaction`/`tag:generated`
  semantics, as hledger's do, so a reader can filter them.
- **Asset rows:**
  - A contribution is an ordinary leg against its counter-account.
  - Each growth step on an unpriced asset becomes a posting from the asset to
    its gains account (Decision 3).
  - A priced asset's row grows through `prices`, not postings.
- **Price growth (Decision 3):** an asset row with `growth:` on an account
  holding priced commodities produces stepwise in-memory prices for those
  commodities, starting from each one's latest price at the cutoff, and
  returned in `Forecast.prices`. They are never serialized. Two rows that
  would grow the same commodity at different rates are a warning, and the
  first row's rate is used: a price belongs to the commodity, not to the
  account.
- **Re-derive the existing outputs from the expansion.** The cash series,
  net-worth series and runway come from `account_totals` over
  `real ++ forecast`, not from the per-bucket accumulators. The accumulators
  and the residual rule are then deleted, so there is one source of truth.
- **The cutoff (Decision 5)** is computed in core from `today` (passed in, as
  every report's date is) and the latest real transaction date. Real
  transactions on or after it are filtered out of `real`.
- **Legacy input:** a group whose postings do not balance (every file written
  before this plan, and `budget.journal`) gets its residual assigned to the
  default counter-account (Decision 7) at load. That keeps today's numbers
  while the counter-leg becomes explicit.

Tests:
- A parity golden: for balanced scenarios, the new cash, net-worth and runway
  figures equal the current engine's.
- Plan 22's residual table rows (accrual, partial payment, equity raise) are
  restated as explicit-counter-account cases.
- Growth and contribution timing from plan 23 amendment 6 are unchanged.

## Phase 2 — The file and the model

- `ScenarioLine` gains `counter: Option<AccountName>`, and asset rows gain
  `gains: Option<AccountName>`.
- The scenario header gains `; cash: <account>`, the default counter-account.
- The serializer writes the counter posting as an amountless second posting
  (hledger infers it) and declares a gains account the main journal lacks
  (Decision 4).
- Old files load (Phase 1's legacy rule) and are upgraded only when the user
  saves. Plan 22 Decision 5 still holds: never write `budget.journal`.
- `docs/projections/file-format.md` and `cash.md` are rewritten:
  - the residual rule goes
  - counter-accounts, gains accounts and price trends come in
  - the exact hledger command is given

## Phase 3 — A multi-column balance sheet (reports)

`balance_sheet_grouped` is single-date today. Add a columns variant that
computes cumulative totals in one pass over the transactions and values each
column at its own end, as `income_statement_columns` does. Calling the
single-date function once per column is O(columns × txns) at 150k
transactions, so it is the fallback, not the plan; measure against the plan 26
perf baseline. The Reports tab gets a periodic balance sheet from this too.

## Phase 4 — Server

The projection run endpoint (`projections_api.rs`) gains:
- `lookback`, a count of past full buckets, default 2
- the three statements, built from `real ++ forecast` with the same
  `CfTags`/groups/sections/terms the Reports handlers build
  (`reports_api.rs:2606`, `:2713`, `:2951`)
- `actualsEnd`, the cutoff
- the runway

Net income and net worth leave the wire. One expansion serves every reading.

## Phase 5 — The tab

- Tabs become **Runway**, **Balance sheet**, **Income statement**,
  **Cash flow**, with shared interval, count and lookback controls. Digits
  1–4 switch them, as on every other tab strip.
- The three statements render through the existing `BalanceSheetView`,
  `IncomeStatementView` and `CashFlowStatementView`. They gain one prop: the
  column index where projection starts, drawn as a divider with "Projected"
  over the forecast columns.
- Runway keeps its sentence and chart.
- The asset-contribution breakdown under Net worth goes. The balance sheet
  shows each asset account's balance per column, which is what it summarized.
- The what-if table gains a counter-account column for flows and a gains
  account for asset rows (account combobox, default shown greyed until
  overridden, as the asset Balance column does today).

## Phase 6 — hledger parity

Extend the interop test at `crates/ledgeline-core/tests/projection_files.rs:725-906`.
For fixture scenarios **without growth**, run
`hledger bs|is|cf --forecast=…` over `main + scenario` and assert per-account,
per-period agreement with Ledgeline's statements.

With growth, assert the documented difference: hledger shows base amounts.
This test is what keeps "fully compatible with hledger" true after this plan
lands.

## Already shipped ahead of this plan (2026-10-23)

- "Cash & runway" is now labelled **Runway**. The URL value stays `cash`, so
  existing links keep working.
- Digits 1–3 switch the current projection readings, listed under a
  **Projections** group in `?`. Phase 5 extends them to 1–4.
- <kbd>g</kbd> <kbd>p</kbd> goes to Projections. Both are documented in
  `docs/journal/keyboard.md`.

## Open questions

None blocking. Decisions 1–8 were settled with Patrick on 2026-10-23.

---

## Contract amendments made during implementation

**Phases 1 and 2 only** (2026-10-24). Everything below was settled while
building the expansion, the file format and the wire. Phases 3 to 6 should
trust these over the prose above.

### 1. The API Phase 4 builds on

All in `crates/ledgeline-core/src/projections/forecast.rs`, re-exported from
`ledgeline_core::projections`:

```rust
pub struct History<'a> { pub txns: &'a [Transaction], pub prices: &'a [PriceDirective] }
pub struct ForecastOpts<'a> {
    pub today: &'a str, pub interval: Interval, pub count: usize,
    pub lookback: usize, pub value_in: Option<Commodity>,
}
pub struct ForecastWindow {
    pub cutoff: String,          // first projected day == Phase 4's `actualsEnd`
    pub from: String,            // first day of the first lookback period
    pub end: String,             // last day of the last forecast period
    pub interval: Interval,
    pub lookback: Vec<String>,   // full actual periods before the cutoff
    pub buckets: Vec<String>,    // forecast periods; the first starts at `cutoff`
}
pub struct Forecast {
    pub window: ForecastWindow,
    pub txns: Vec<Transaction>,        // generated, date-ascending, balanced
    pub prices: Vec<PriceDirective>,   // in-memory only, dated at growth steps
    pub assets: Vec<AssetRow>,
    pub warnings: Vec<String>,
    pub target: Option<Commodity>,
}
pub fn forecast_transactions(scenario: &Scenario, opts: &ForecastOpts, types: &AccountTypes,
                             history: &History) -> Result<Forecast, ReportError>;
pub fn forecast_cutoff(today: &str, latest_real: Option<&str>, interval: Interval) -> Result<String, ReportError>;
pub fn latest_real_date(txns: &[Transaction]) -> Option<&str>;
pub fn merged_transactions(history: &[Transaction], forecast: &Forecast) -> Vec<Transaction>;
pub fn merged_prices(history: &[PriceDirective], forecast: &Forecast) -> Vec<PriceDirective>;
pub fn is_generated(txn: &Transaction) -> bool;
pub struct CounterDefaults { .. }   // ::new(history, types, cutoff, header, value_in)
                                     // .for_account(a) / .for_accounts(iter)
pub fn upgrade_legacy_counters(scenario: &Scenario, defaults: &CounterDefaults) -> Scenario;
pub const DEFAULT_LOOKBACK: usize = 2;
pub const DEFAULT_GAINS_ACCOUNT: &str = "equity:unrealized gains";
pub const GENERATED_TAG: &str = "generated-transaction";
```

Phase 4's statements are `real ++ forecast` = `merged_transactions` and
`merged_prices`. Both drop real data dated on or after the cutoff. `Forecast`
carries no real transactions, so the server does not clone the journal unless
it needs the merged list.

Generated transactions are marked as hledger marks them: tags
`generated-transaction: ~ <period>` and `_generated-transaction`, plus the same
text in `comment`. Their `Tindex` continues from the journal's highest, and
their `source_file` is empty.

### 2. One transaction per GROUP and DATE, not per line

The plan says "one balanced transaction per occurrence: the line's posting plus
its counter-account posting". The expansion builds one transaction per
`(group, date)` instead: every line of one `~` rule that fires that day, its
explicit counter postings (pooled per counter account), and the legacy
residual. That is what hledger generates from one rule. It is also the only
place the legacy residual can be computed, because the residual belongs to the
group. Each line is still balanced against its own counter.

Asset growth steps are separate transactions (`<account> growth`, period
marker `~ growth`): the asset up by the step, the gains account down by it.

### 3. The legacy rule is per occurrence, pooled per group

For lines with `counter: None`, the expansion sums their postings per group per
date. If the sum is non-zero, it is posted, negated, to
`CounterDefaults::for_accounts(<those lines' accounts>)`. The pairing counts of
the group's accounts are pooled, so a one-line group answers exactly as
Decision 7 states. Groups get one default because a `~` block can hold only
one amountless posting (amendment 5). Per occurrence rather than per base
amount, because a rule balanced at its base amounts can drift out of balance
once growth applies, and the old engine's residual was per bucket.

The parity golden (`tests/projection_parity.rs`) pins this: five scenarios,
including unbalanced legacy rules with growth, the accrual / partial / raise
events, unpriced asset rows with contributions, a step change and weekly
growth. Cash, net worth, net income, runway and per-row asset attribution all
equal the old engine's figures exactly. The old figures were captured from
`33439a7` before the accumulators were deleted.

### 4. Decision 7, filled in

- **Window:** transactions dated in `[cutoff − 12 months, cutoff)`.
- **"Paired with":** each real transaction that has a cash-typed posting counts
  once for every (other account, cash account) pair in it. A line account
  pools its descendants (`revenues:salary` sees `revenues:salary:base`). A cash
  account inside the line's own subtree never counts as a pairing.
- **Ties** go to the lexically first account at every step.
- **Largest balance** is valued into the valuation commodity the day before the
  cutoff.
- **Two steps the plan did not have:** if the journal has no cash balance at all,
  the first account declared `type: C`, and failing that `assets:cash`.
  hledger's own name heuristic reads `assets:cash` as Cash, so the runway still
  sees the money in an empty or untyped journal. This is why the old unit
  tests over an empty journal still pass unchanged.

### 5. The file format: the amountless posting IS the counter

- A rule's counter-account is its **regular posting with no amount**. The
  journal parser infers that posting's amount during balancing and recorded
  nothing about it, so `PeriodicTransaction` gained
  `pub elided: Option<usize>`: the index of the regular posting whose amount
  was elided, recorded before balancing. `lines_from_rule` skips that posting
  and gives its account to every regular line of the block. **This changes the
  seed:** a `budget.journal` rule written `expenses:food $500` /
  `assets:checking` now seeds ONE line, with `counter: assets:checking`. It
  used to seed two lines, the second being the inferred `$-500` "stated cash
  leg". The numbers are identical.
- **Virtual lines keep no counter.** In a block that has a counter, a
  `(account) amount` line stays `counter: None`, because hledger leaves
  unbalanced virtual postings out of balancing. The writer mirrors this: lines
  with a counter are written plain, lines without are written `(account)`, and
  the counter row comes last. A pre-plan-28 block with no counters is written
  byte-for-byte as before.
- **Two different counters in one group are refused**
  (`SerializeError::Invalid`, "every line of one rule must share its
  counter-account; split them into separate rules"), as two recurrences in one
  rule already are. Phase 5's table must give a row whose counter differs from
  its group's a group of its own.
- **Events** gained `ScenarioEvent::counter`: the block's amountless posting,
  which takes whatever the event's postings do not balance. The plan named only
  `ScenarioLine::counter`, but every pre-plan-28 event is unbalanced too.
- **`gains:`** is a posting tag on asset rows (`gains: equity:home gains`;
  spaces are fine, commas are refused). It is refused on a flow line.
- **`; cash: <account>`** is a header line (`Scenario::cash`). The header span
  and `parse_header` recognise it.
- `chunk_print`, the round-trip comparison, now includes each line's counter
  and gains and each event's counter, so `confirm_round_trip` proves them.

### 6. The gains declaration (Decision 4) and its one-time byte change

The writer declares `account <gains>  ; type: <letter>` for each gains account
that a row with a **non-zero** rate books to and that neither the main
journal's declared types nor the file itself declares. The letter is the
account's resolved type, Equity when it resolves to nothing. The declaration
goes below the header, set off by a blank line, so the next read's header span
ends before it and a second save is byte-identical.

Consequence: the **first** save of a pre-plan-28 file with a growing asset row
adds that declaration even if nothing else changed. Two tests pinned "an
unchanged save rewrites no byte" over such files. Both now assert the one
inserted declaration and then byte-identity on the second save:
`a_growth_tag_on_an_asset_account_reads_as_an_asset_row` and
`the_committed_asset_fixture_reads_and_rewrites_unchanged`.
`fixtures/asset-growth-scenario.journal` itself was NOT edited, because it also
feeds the TypeScript-captured `journal_meta` golden.

### 7. The save-time upgrade lives in the server's save handler

`projections_api::save` now runs `upgrade_legacy_counters` before writing. It
uses defaults built with `today_utc()` and a monthly cutoff. Only a group in
which **no** line names a counter and whose base amounts do not sum to zero is
touched, and every line in it gets the group's default. The same goes for an
unbalanced event with no counter. Balanced groups, mixed groups and anything
already countered are left alone. `the_upgrade_moves_no_number` pins that the
upgraded scenario expands to the same account totals.

### 8. `ProjectionOpts.as_of` became `today`, plus `lookback`

The start semantics changed deliberately (Decision 5). `Projection::start` is
now the cutoff. `Projection` gained `lookback: Vec<String>`, and
`BalanceSeries` gained `history: Vec<MixedAmount>`: the REAL closing balances at
the end of each lookback period. `net_worth.history` comes from
`net_worth_priced`; `cash.history` comes from a one-pass closing-balance scan.
The server passes `DEFAULT_LOOKBACK` but does not serialize either field yet,
so the wire is unchanged for them.

Opening balances are real figures the day before the cutoff. Every real read
(`account_balances_at`, `net_worth_priced`, counter inference) is bounded
there, by posting date, so the journal is never cloned to filter it. Real `P`
directives and inferred cost prices dated on or after the cutoff are dropped
too, consistent with "no mixed period".

### 9. How the legacy series are derived, and why not by valuing each column

`project()` reads cash, net worth and net income off `forecast.txns`, one
`account_totals` call per bucket over the date-sorted slice, beside the real
openings:

- cash = valued real cash opening + raw generated postings to `is_cash`
  accounts;
- net worth = `net_worth_priced` opening + raw generated postings to asset and
  liability accounts + the revaluation of every REAL asset/liability holding
  in a commodity with forecast prices (valued at the bucket end, less valued at
  the cutoff);
- net income = generated revenue and expense postings, cash-flow oriented, as
  before.

Generated amounts are deliberately NOT valued per column. `value_at` drops
unpriced commodities, so valuing would silently delete a EUR scenario line
that the old engine added as EUR with a warning. That warning ("added without
conversion") is unchanged and still true. Phase 4's statements value per
column as the Reports tab does. That is right there, and it differs from this
legacy series only for multi-commodity scenarios.

### 10. Asset rows: when a row is "priced", and what grows

A row grows through **prices** when all of these hold: there is a valuation
commodity, the row has no `opening:` override, and its subtree holds a
commodity other than that one with a direct price (explicit or inferred) the
day before the cutoff. Then:

- each such commodity gets stepwise `PriceDirective`s at the row's growth
  boundaries (anchor = the row's `from`, else the cutoff), starting from that
  latest price and rounded at every step to
  `max(price places, price display precision, 4)`. The floor of 4 places keeps a
  1%/yr step on a $5 price from rounding to nothing;
- the row's **base-commodity** part (cash held in the brokerage, plus
  contributions) still compounds through gains postings, as an unpriced row
  does;
- `AssetRow::growth` = the gains steps + the priced holdings revalued from the
  cutoff to the end of the span;
- the net-worth series revalues **every** real holding of a grown commodity,
  wherever it sits, because a price belongs to the commodity. So the series can
  move more than the sum of `AssetRow::growth` when one commodity is held
  outside the row's account.

An `opening:` override makes the row compound as one balance (the old
behaviour). Its difference from the journal is now a gains transaction dated
the cutoff (asset vs gains account) rather than a net-worth-only adjustment.
The warning text is unchanged.

**One behaviour change:** a growing asset row on a **cash-typed** account (a
savings account declared `type: C`) now moves cash, because its gains posting
lands in a cash account. Before, appreciation never reached cash. This is
interest in the account, so the runway counts it. No existing test covered
the case.

### 11. The wire (Phase 2 only)

- `WireScenarioLine` gained `counter` and `gains`, `WireScenarioEvent` gained
  `counter`, and `WireScenario` gained `cash`. All are strings or `null`, never
  absent (plan 22 amendment 14). Inbound they are optional, and blank means
  none. `gains` on a flow line is a `400`.
- `fixtures/native/v1/projections-seed.json` and `projections-asset.json` were
  regenerated by replaying through the in-process oneshot (no port bound).
  Every other golden is byte-identical.
- TypeScript: `ScenarioLine.counter/gains`, `ScenarioEvent.counter` and
  `Scenario.cash` were added to `projections/types.ts`, the raw types and
  decoders in `nativeDecode.ts`, `WireScenario*In` in `native.ts`, and
  `scenarioToWire` / `cloneScenario` / `blankLine` / `blankEvent` /
  `emptyScenario`. They carry the values through untouched, so a counter stated
  in a file survives a UI save. **Nothing renders them yet; that is Phase 5.**
  `TOLERATED` did not grow.
- `WireProjection` is unchanged.

### 12. Tests that changed meaning, deliberately

Every test that read "as-of 2026-09-20, projection starts October" now reads
"today 2026-10-01 over a journal ending 2026-09-30" (the fixture journals moved
from 2026-01-10 / 2026-01-01 to 2026-09-30). Yearly tests over a journal use
`at_year_end` with today 2027-01-01. Tests whose as-of was a period's last day
moved to the next day. `the_projection_starts_at_the_next_whole_bucket` became
`the_projection_starts_at_the_first_incomplete_period`, with the plan's three
worked examples. In `projection_endpoints.rs`, `sample.journal` ends
2026-07-03, so at `asOf=2026-07-08` July is now projected and the buckets,
runway index and totals positions moved by one month. Three save tests now
expect the upgraded, balanced form of the legacy plan file.

New tests: the cutoff examples, the window, counter inference (pairing,
subtree pooling, trailing window, header, largest balance, declared and
fallback), the legacy residual, balanced groups, explicit counters, plan 22's
table restated with explicit counters (accrual, partial, raise, card), gains
postings, price trends with the conflict warning and net-worth revaluation,
the upgrade (what it touches, and that it moves no number), serializer round
trips of counter/gains/cash and the refusals, the lookback, the parity golden,
and a gated hledger interop test proving the balanced format forecasts into
checking and passes `hledger check`.

### 13. Not done here

Phases 3 to 6. In particular, there is no multi-column balance sheet, no
statements on the wire, and no counter or gains columns in the table. The
hledger interop test is a single check, not Phase 6's per-account,
per-period parity. The docs (`docs/projections/cash.md`, `file-format.md`,
`asset-rows.md`, `index.md`) were rewritten for counter-accounts, gains
accounts, price trends, the cutoff and the exact hledger command.

---

**Phases 4 and 5** (2026-10-24). Phase 3 (`balance_sheet_columns`) landed in
`c62e4fc` without amendments of its own.

### 14. The run wire

`POST /api/projections/run` takes `lookback` (default `DEFAULT_LOOKBACK`, `0`
allowed; `lookback + count` above `MAX_BUCKETS` is a `400` naming both) and
answers:

```text
buckets, lookback          forecast keys; actual keys before them
actualsEnd                 the cutoff (was `start`, renamed: same value)
projectedFrom              index of the first forecast column in the statements
cash {opening, values}     forecast periods only, as before
runway, assets, warnings   as before
balanceSheet               WireBalanceSheetColumns (below)
incomeStatement            WireIncomeStatementReport, interval mode
cashFlow                   WireCashFlow
defaults {cash, gains, lines: {group: account}, events: {id: account}}
```

`netIncome` and `netWorth` are gone. The statements cover `lookback ++
buckets`; `cash`/`runway` cover `buckets` only, so the runway still counts from
the cutoff. `cash.history` (amendment 8) is still not serialized.

**The balance sheet is PIVOTED on the server.** `balance_sheet_columns`
returns independent reports; `reports::pivot_balance_sheets` joins them into
`BalanceSheetColumns`: one tree with the single-date report's keys at every
level (`sections[].groups[].rows[]`, `subsections[]`), every figure an array
with one entry per column (`total`, `own`, `inclusive`, `netWorth`, `check`,
`balanced`), and `{}` where a column lacks the node. Groups are keyed by
(section, term, name), ordered by the single-date sort key (now one shared
`group_order`). Chosen over an array of reports because the join is logic and
belongs in Rust (plan 26), and because a client-side join cannot reproduce
chain compression per column. `columns` carries the same `{key, label, from,
to}` spans the income statement and cash flow statement do, so the three line
up column for column.

The run is still not in the `fixtures/native/v1` manifest (it is a POST with a
body; the harness replays GET URIs). It is pinned by
`projection_endpoints.rs`, by `nativeLive.integration.test.ts` against a live
engine, and its two reused types by their own goldens.

### 15. How the statements are built

`projected_statements` in `projections_api.rs`: `merged_transactions` and
`merged_prices` over the window `[from, end]`; `balance_sheet_columns` at each
`range_buckets` span's `to` (depth `None`, as the SPA asks the Reports tab);
`income_statement_columns` with `compare: false`; `cash_flow_statement` with
interval columns and the request's `depth`. The declarations come from
`reports_api::StatementShape::of(journal)` (declared types, `bsgroup:`,
`bsterm:`, `issection:`, `isgroup:`), which the Reports handlers now use too;
the cash flow tags are `CfTags::from_journal`, the call the Reports handler
makes.

**One deviation:** valuation is market, into the projection's own target
(`Forecast::target`: `valueIn`, else the price table's base). The Reports tab
resolves `valueIn` through `resolve_grouped_value_in` (journal `D` default,
admission test). Using the forecast's target keeps the runway and the
statements in one commodity; it agrees with the Reports tab whenever `D` is
the base commodity, as in every fixture.

### 16. Grown prices only for commodities a real `P` prices

`merged_prices` now drops a forecast price whose commodity no real explicit
directive (before the cutoff) prices. The expansion grows prices from the
latest explicit OR inferred price (amendment 10), but the statements value
through explicit `P` only. Without the filter, a commodity priced only by its
cost (sample.journal's GLD) became priced in forecast columns only: a jump the
size of the whole holding at the first growth step, which is not growth. With
it, such a commodity reads as share counts and `meta.unpriced` in every column,
as on the Reports tab. The legacy net-worth series (`project`) is unchanged.

### 17. `Projection` keeps its `Forecast`; defaults on the wire

`Projection.forecast` holds the expansion, so the server builds the statements
from the same generated transactions the cash series came from. `Forecast`
gained `defaults: ForecastDefaults`: per group of counter-less lines and per
counter-less event, `CounterDefaults::for_accounts` over their accounts;
`cash`, the header-less fallback (Decision 7 steps 3–4); `gains`,
`DEFAULT_GAINS_ACCOUNT`. These are what the table greys.

### 18. The tab

- Tabs and URL codes: **Runway** `cash` (unchanged), **Balance sheet** `bs`,
  **Income statement** `is`, **Cash flow** `cf` — the Reports tab's codes.
  Runway is first and the default: it is the reading only this tab has. Digits
  1–4 follow strip order. `tab=net` and `tab=worth` fall back to Runway.
- A new **Actuals** number input (`lookback`, URL `lookback=`, 0–120) beside
  Interval and Periods. The depth slider now rolls up the cash flow
  statement's lines.
- The statements render through `BalanceSheetView`, `IncomeStatementView` and
  `CashFlowStatementView`, each with an optional `projectedFrom`. A shared
  `ProjectedHeadRow` prints an **Actual / Projected** band over the columns,
  and `projectedEdge` rules the first projected column. Null changes nothing,
  so the Reports tab is unchanged.
- `BalanceSheetView` takes a `BalanceSheetReport` or a `BalanceSheetColumns`.
  A single report becomes one column labelled "Amount" (`asColumns`), so there
  is one template. The display walk is generic over the figure type
  (`columnSectionRows`); in columns a parent folds into its child only when it
  has nothing of its own in EVERY column. The tie-out keeps one verdict cell;
  the warning names the first column that does not balance. Unconverted
  commodities stack one per line in columns, so the boxes stay aligned.
- The runway chart's labels are the balance sheet's forecast column labels.
  The asset breakdown, the net-income chart and the net-worth chart are gone.

### 19. The table: counter, gains and cash

- **Counter-account** column on Income, Expenses, dated `~` lines and events
  (an event's on its first posting). **Gains account** column on asset rows.
  **Default cash account** box above the sections (`Scenario.cash`). Each is an
  `AccountInput` (which gained a `label` prop) that is empty with the engine's
  default as placeholder, from the last good projection; `default` before one
  lands. Clearing a box means "the default" (`null`).
- **Not shown:** an asset row's own counter-account (its contribution's). It is
  carried through untouched and defaults as before. The ask named counter for
  flows and events and gains for asset rows; adding a ninth column to the asset
  table was left for when contributions need it.
- **One counter per rule.** `withCounter` sets one line's counter and, when any
  rule-mate's counter would then differ, gives the line a fresh `group`, so the
  serializer's refusal (amendment 5) is never reached. Clearing counts as
  differing: a counter-less line beside a countered one is written virtual,
  which hledger leaves out of balancing. A split re-keys the row, so focus
  leaves it after Enter. Splitting each line of a rule to the same new counter
  one by one yields one rule per line, not one rule; the numbers are the same.
- `scenarioToWire` trims `counter`, `gains` and `cash`; blank is `null`.

### 20. Seen while verifying, not fixed here

- **The runway can fire on a non-charted commodity.** It is the first bucket
  where ANY commodity of the cash series is negative. Over sample.journal's
  seed the EUR lines are added unvalued (amendment 9), the opening EUR was
  valued into `$`, so EUR goes negative in month one and the sentence reads
  "Cash turns negative in Jul 2026" over a $51K balance. This predates Phases 4
  and 5.
- **Decision 7 pairs by account, not commodity.** The seed's `$` food line
  defaults to `assets:bank:wise:eur`, the account food was most often paid
  from, so the forecast posts dollars into the EUR account.
- `merged_transactions` clones the real journal per run. Not measured against
  the plan 26 baseline.

### 21. Not done

Phase 6. The Reports tab's periodic balance sheet (Phase 3's last sentence).
The docs screenshots were not regenerated; `projections.docs.ts` waits on the
Runway reading instead of the removed Net income one.
