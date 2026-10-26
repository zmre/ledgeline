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

---

**Fixes after Phase 5** (2026-10-24). Three bugs seen on the Projections tab
over `fixtures/sample.journal`, the e2e fixture. Amendment 20's first two
bullets are fixed here. Amendment 22 SUPERSEDES the "cash-typed" wording of
Decision 7 and amendment 4's definition of "paired with".

### 22. Runway reads total cash, valued per period (bug 1)

**Root cause.** `project()` valued the real opening cash into the target
commodity (`valued`, which also drops unpriced commodities) and then added the
generated postings raw. Over sample.journal the opening EUR became dollars,
while the seed's EUR food and travel lines subtracted raw EUR. The series held
`$51K + −19.06 EUR` in July, and the runway check ("any commodity negative")
fired in month one over a growing dollar balance. Per-account balances played
no part. Wise's −$215 (bug 2) is netted inside the `$` total.

**Fix.** Cash is carried RAW, per commodity, summed over every `is_cash`
account (opening plus generated postings). Every figure read off it (opening,
each period's close, the lookback history) is valued at that period's end by
`valued_keeping_unpriced` (now `pub(crate)`). A commodity a price reaches is
converted into the target. One that no price reaches is kept as itself, never
dropped. The runway is the first period whose valued total has any negative
component.

**How mixed commodities are judged:** after summing across accounts and
after valuation. EUR spending beyond the EUR balance is covered by dollars, so
it is not exhaustion. A negative commodity that cannot be valued cannot be
netted, so it fires on its own. One cash account overdrawn while the total is
positive is never a runway. This supersedes amendment 9's "generated amounts
are deliberately NOT valued" for the CASH series. The net-worth series is
unchanged and off the wire.

The "added without conversion" warning now names only commodities NO price
converts into the target. EUR has a `P` price in sample.journal, so its seed
now warns about nothing.

Tests: `eur_spending_against_eur_cash_is_no_false_runway` (the sample-journal
shape), `one_overdrawn_cash_account_is_not_a_runway_while_total_cash_is_positive`,
`a_priced_commodity_is_netted_into_the_total_before_it_is_judged`,
`an_unpriced_commodity_below_zero_is_a_runway_on_its_own`, and server
`the_sample_seed_raises_no_false_runway_and_infers_counters_per_commodity`.
The e2e now asserts "Cash never turns negative" over the fixture.

### 23. Default counter-account: per commodity, any balance-sheet account, one liability hop (bugs 2 and 3)

Patrick's decision was "infer the counter accounts based on most common
pairing in history". Decision 7 restricted that to cash-typed accounts, so
`expenses:depreciation` defaulted to checking and drained cash. Amendment 4
also paired by account only, so the `$` food line defaulted to the EUR-only
Wise account. `CounterDefaults` now works like this:

1. **Pairing, per (account, commodity).** Over `[cutoff − 12 months,
   cutoff)`, each real transaction's postings are netted per (account,
   commodity). For the line's account (its subtree pooled, as before) in the
   line's commodity, a **counterparty** is another account in the same
   transaction, same commodity, opposite sign. The counterparty must be an
   **asset or a liability**, cash or not.
2. **Weighting by amount, not by transaction count.** Each counterparty is
   credited `min(|its amount|, |the line account's amount|)`, and the
   largest total wins (ties lexical). Count-weighting was tried first: over
   sample.journal the brokerage's six small dividends outvoted its four large
   deposits from checking, so a contribution row was balanced against
   `income:dividends`. Amounts answer "where the money went".
3. **Equity, revenue and expense are not counterparties.** Equity is left out
   because an opening-balance transaction pairs every account with
   `equity:opening` (this broke the asset-contribution tests). Revenue and
   expense are left out because payroll withholding pairs
   `expenses:taxes` with `income:salary`. Projected that way, the tax never
   leaves cash while the gross salary lands in checking, which overstated
   sample.journal's build by about $973 a month (seen in the visual check).
   Such a line falls through to the fallbacks and comes out of cash.
4. **One liability hop.** If the winner is a liability, its settlement is the
   same vote over its own DEBIT postings (the payments that reduce it), in
   the same commodities. If that settlement is a cash account, the cash
   account is used. A projection does not project card payments, so spending
   left on the card would never draw cash down. If the card is settled from
   a non-cash account, or never, the card stays.
5. **Fallbacks, commodity-aware.** The order is unchanged: header `; cash:`,
   largest valued cash balance, first declared `type: C`, then any cash
   account. Each step is first restricted to cash accounts that HOLD one of
   the line's commodities: a non-zero balance at the cutoff, or a posting in
   it in the window. A header that holds nothing at all (a fresh account)
   still stands. Only when no cash account holds the commodity does the
   unfiltered chain run, ending at `assets:cash`.
6. **API.** `for_accounts(accounts)` became
   `for_lines(impl IntoIterator<Item = (&str, &Commodity)>)` and
   `for_account(account, commodity)`. Events contribute one pair per stated
   amount. A group that mixes commodities pools raw quantities across them;
   the seed never builds one (one group per account and commodity). The
   wire's `defaults.cash` is `cash_fallback()`, the header-less fallback in
   the valuation commodity. The legacy residual and the save-time upgrade
   still give a group ONE counter (amendments 3 and 5).

Over sample.journal the inferred defaults are now:

| line | counter |
|---|---|
| `$` food, housing, utilities, transport | `assets:bank:checking` (food via the visa hop) |
| `$` taxes | `assets:bank:checking` (no balance-sheet pairing; fallback) |
| `$` depreciation | `assets:vehicles:car:depreciation` |
| EUR food, EUR travel | `assets:bank:wise:eur` |
| `$` travel | `assets:bank:checking`, never Wise |
| salary / dividends | `assets:bank:checking` / `assets:broker:taxable:cash` |

Tests (in `projections/forecast.rs`): depreciation stays non-cash, both
inferred and expanded; card-paid food resolves through the card to checking;
an unsettled card is kept; salary goes to checking and withheld tax falls back
to cash; a `$` line never picks an EUR-only account, even when the header names
one; the no-history fallbacks. The e2e checks that depreciation's greyed
counter is the contra-asset. No golden pinned the old defaults: the existing
`rule:0` and event assertions still answer `assets:bank:checking`.

### 24. Still not done

Amendment 20's third bullet (`merged_transactions` clones the journal) and
amendment 21 are unchanged.

---

**Phase 6** (2026-10-24): hledger parity.

### 25. A rule with no `from` is phased from the cutoff (engine fix)

**Found by the parity test.** The expansion walked occurrences with the
budget report's phase (`budget::occurrences`): a bare `every N <unit>` rule
starts at the start of the unit period that contains the span start. That is
right for `hledger balance --budget` and is pinned by the budget golden. It is
NOT what `hledger --forecast` does. Driving `hledger print --forecast=S..`
(1.52) with `S` on and off every unit's boundary shows that a fixed-interval
rule with no `from` is phased from `S` itself:

| rule | `--forecast=2026-10-01..` | was (Ledgeline) |
|---|---|---|
| `~ weekly` | Thu 10-01, 10-08, … | Mon 10-05, 10-12, … |
| `~ every 2 weeks` | 10-01, 10-15, … | 10-12, 10-26, … |
| `~ yearly` | 2026-10-01, 2027-10-01 | 2027-01-01 |
| `~ monthly`, from 2026-10-05 | 10-05, 11-05, … | 11-01, 12-01, … |

Everything else already agreed: a rule with `from` (its own phase), the
anchored forms (`every 15th day of month`, `every tuesday`: a lower bound),
dated one-offs and the exclusive `to`.

**Fix.** `budget::forecast_occurrences` substitutes the span start as the
phase of an `Every` rule with no `from`, then calls `occurrences` unchanged.
The expansion's two walks (flows, asset contributions) use it, and the budget
report keeps `occurrences`. Growth anchors are unchanged (the row's `from`,
else the cutoff). Because the cutoff is always a period start, `~ monthly`
still lands on the 1st in a monthly, quarterly or yearly projection. In a
weekly projection, whose cutoff is a Monday, it lands on that Monday's day of
the month, as hledger's does.

**Golden moved deliberately.** `projection_parity.rs` case 4 has a bare
`~ weekly` line under a quarterly cutoff of Thursday 2026-10-01. Q4 2026 now
has 14 occurrences instead of 13, and the later quarters shift with their
Thursday counts. Its figures were recaptured and the module docs say why.
Nothing else in `just engine-test` moved. Unit test
`a_forecast_phases_a_bare_interval_from_its_own_start` pins the hledger dates
without the binary.

### 26. What the parity suite covers

`tests/projection_files.rs`, the `parity_*` tests (opt-in,
`LEDGELINE_HLEDGER_PROJECTION_CHECK=1`; `just hledger-checks` already runs this
file). Fixtures are in `fixtures/projections-parity/`. `main.journal` declares
every account and commodity. Its latest transaction is a real 2026-10-05 one,
after the 2026-10-01 cutoff, and EUR is repriced inside the lookback. The
scenarios are `flows` (explicit counters; weekly, every 2 weeks, quarterly,
yearly, bounded `from`/`to`, a dated rule), `events` (an accrual to a
liability, its settlement, an equity raise), `legacy` (unbalanced pre-plan-28
rules), `assets` (an asset row with `growth:` and no rate, and a gains
account), `step` and `commodities` (EUR lines, and one rule mixing EUR and `$`
against one counter).

For each scenario and cadence (monthly, and quarterly or weekly where they
differ), the file goes through the server's save path
(`upgrade_legacy_counters` at a monthly cutoff, then `write_scenario`) and the
next load is expanded. Then:

- `hledger -f main -f scenario check --strict ordereddates` passes.
- `hledger print --forecast=<cutoff>..<end+1> tag:generated` equals the
  expansion's postings as (date, account, commodity, amount) multisets.
- `bs`, `bse` (equity rows), `is` and `cf` run in the documented form with
  `-O csv --layout=bare`; `bs`, `bse` and `is` add `--value=end,$`. They must
  equal, per account, per commodity and per column, `balance_sheet_columns` /
  `income_statement_columns` (market, the forecast's target) and the cash
  flow statement's per-cash-account change over `merged_transactions` and
  `merged_prices`. These are the calls `projected_statements` makes.
  hledger's `cf` is a list of cash-account changes, not our indirect
  statement, so only the per-account change is compared; ours must also
  `reconcile`.
- A test removes the `expr:` and shows hledger then counts the real $77 from
  October. The command does real work in this fixture.

**With growth** (`growth.journal`), the test asserts the two accepted
deviations and nothing else. hledger books rent at its base while ours grows,
and cash differs by exactly the extra rent. hledger's house is flat with no
`equity:unrealized gains` row, while ours rises and the gains account books
the difference. hledger values AAPL at its last real `P` while ours revalues
it. Every other account agrees per column, and the scenario's own gains
declaration passes `check --strict`.

**One representational difference, not a numeric one.** A legacy balanced
`(account)` pair stays virtual in hledger, which still counts it in every
report run without `-R`. The expansion books it as a regular posting, because
the cash flow statement drops virtual ones. The generated-posting comparison
strips the marker.

**Docs.** `cash.md`'s command lacked `-b <lookback start>`. Both docs now give
the full form, explain each part, add `--value=end,$` for other commodities,
and say what `cf` compares. `file-format.md` gained "When a rule fires"
(amendment 25).

---

## Status: built

All six phases shipped on `2026-10-23-projection-fixes`:

- **Phase 1:** `forecast_transactions` expands a scenario into balanced, dated,
  `generated-transaction`-tagged transactions per (group, date). Decision 5
  sets the cutoff. Decision 7, amended by 23, infers default counters per
  commodity. Legacy residuals are posted to the default counter. Unpriced
  asset rows post gains, and priced holdings grow in-memory prices. The
  per-bucket accumulators are gone.
- **Phase 2:** the format carries counter-accounts as the amountless posting,
  plus `gains:`, `; cash:` and gains declarations. Old files load, and are
  upgraded only on save.
- **Phase 3:** `balance_sheet_columns` (one pass) and `pivot_balance_sheets`.
- **Phase 4:** `POST /api/projections/run` returns the three statements over
  `lookback ++ buckets`, plus `actualsEnd`, `projectedFrom`, runway and
  defaults.
- **Phase 5:** the tabs are Runway, Balance sheet, Income statement and Cash
  flow, with an Actuals control, an Actual/Projected divider, and counter,
  gains and cash columns in the table.
- **Fixes:** runway reads total cash valued per period (22). Counters are
  inferred per commodity, against any balance-sheet account, with one
  liability hop (23).
- **Phase 6:** per-account, per-period hledger parity for scenarios without
  growth, the growth deviations pinned, `check --strict`, and the forecast
  phase fix (25, 26).

**Still out:**

- The Reports tab's periodic balance sheet (Phase 3's last sentence;
  amendment 21).
- Net-worth series conversion. `project()`'s net-worth series still adds
  generated amounts unvalued (amendment 9). It is off the wire, and the
  statements value per column.
- Mixed-commodity groups. A group mixing commodities pools raw quantities
  when inferring its one default counter (amendment 23, item 6). The seed
  never builds one.
- `merged_transactions` clones the journal on every run, and this has not
  been measured against the plan 26 baseline (amendments 20 and 24).
- Docs screenshots were not regenerated (amendment 21).

## Follow-ups (2026-10-26)

### What-if table: descriptions, order, line tag, seed depth

Four changes asked for after using the table. The decisions, and why:

**1. Descriptions are visible and editable, per RULE.** A `~` rule's header
text (`ScenarioLine.note`) used to reach the screen only as the "estimated"
badge's `title`. Each row's account cell now has a speech-bubble icon
(`NoteButton.svelte`). Hovering or focusing it shows the text in a portalled
tooltip, and the same text is the button's `aria-describedby`, through an
always-present `sr-only` copy. Clicking it opens a portalled editor. Enter
saves, Escape and an outside click cancel (`keys/dismissible`, which also puts
focus back on the icon). The icon is on asset rows and on one-off lines too,
and on every segment of a step, because each segment is a rule with its own
header. Events keep their existing Description column.

- A rule has ONE header, and the writer takes it from the rule's first line.
  So `withNote` sets the note on every line of the `group`. The tooltip and the
  editor say "Shared by the N lines of this rule" whenever N > 1.
- The editor refuses what `serialize::rule_header` refuses (`noteProblem`): a
  line break or control character, `;` or `#`, more than 256 bytes. It says so
  under the box and keeps Save disabled, so the save doesn't fail later.
- **Stale, not rewritten.** A note that starts with the seed's
  `unbudgeted — average over ` on a row whose typed `entry` differs from its
  amount gets a warning dot and a tooltip sentence (`noteIsStale`). Nothing
  rewrites user text. The flag lasts for the session only: after a reload the
  edited amount *is* the amount, and nothing records what the average was.
- `chunk_print` now trims a note, as `rule_header` writes it, so a trailing
  space can't fail the round-trip check.

**2. Order is the file's order.** Row menus gain Move up and Move down, offered
only where there is somewhere to go. Event menus get them too. A drag handle
(`⠿`, `aria-hidden`, not focusable) is the mouse path. The menu items are the
keyboard path, and a drag is just a sequence of the same single moves
(`moveRowTo` loops `moveRow`), so the two can't produce different orders.
Drags never cross sections or zones.

- **A move has to be one a file can state.** `moveRow` first gathers each rule's
  lines together (`gatherRules`, which is exactly the order the writer emits).
  Two rows that are postings of one rule swap their postings. Otherwise the
  row's rules move past the neighbour's rules, so a row that shares a budget
  rule with others carries them along. A file can't put a rule between two
  postings of another rule. Step segments are one row and always move
  together. A move touches no `section` hold and no account, so THE
  SECTION-STABILITY RULE holds. A held, half-typed row moved through its menu
  stays in its section (component test).
- **The writer** (`serialize::assign_to`): the blocks the chunks claim are
  SLOTS, and the chunks fill them in scenario order. Rules and events fill
  separate slot sets, so a file that interleaves them keeps that
  interleaving. Bytes between blocks stay where they were. A deleted block
  still removes its own slot. A chunk that moved into another block's slot is
  always re-rendered. Only an in-place, unchanged chunk keeps its bytes.
- `confirm_round_trip` no longer compares claimed-then-appended prints. It
  compares rules then events, each in the order the file states them, which
  is the scenario's order on one side and the file's on the other.

**3. `line:` is written only for a step change.** `line_tag_values`: an id on
more than one line gets a tag, and no other id does. The value keeps an id that
is already readable (`payroll`). An id that is a machine handle gets a slug of
its account's leaf instead, de-duplicated with `-2`, `-3`. A machine handle is
anything with a colon (`rule:K:N`, `gap:…`) or the table's
`line-N`/`group-N`/`event-N`. This answers the caveat the investigation found.
Untagged rows read back as `rule:K:N`, so a written tag must never be able to
equal one after blocks move. **No written tag contains a colon.** What a block
says now has two readings (`Tags`). `Literal` is what the file states: a
tagged posting's id is its tag. It is used for existing blocks and for the
read-back. `Assigned` is what this save writes. A block whose literal tags
differ from the assigned ones is rewritten. That is how a legacy
`line: gap:expenses:food:$` on a single row is dropped on the next save, and
why the two segments of a step can never end up with different tags. Old files
with `gap:` tags still load: the tag is read as the id, as before. An id that
would not survive the tag grammar (`a,b`) is no longer a refusal. A single row
never writes it, and a step writes a readable one instead.

**4. The seed uses the page's depth.** `scenarioStore`'s seed resource is keyed
by depth, and the page passes its slider's value on first seed. **Moving the
slider does not re-seed by itself.** A seed is a starting point, and once rows
exist they sit at whatever level the user put them. The page offers
**Re-seed at depth N** only when the scenario on screen is an untouched seed
(`seedDepth !== null`, not dirty, no file open) and the slider has moved off
the depth it was seeded at. In any other state it would throw away work or a
file. Mix-and-match needed no engine change, and is now pinned:
`a_line_on_a_parent_account_is_its_own_row_beside_its_children` runs
`expenses:food` $300 beside `expenses:food:groceries` $200. The income
statement's Food group shows them as two rows (depth 2 and 3, $300 and $200),
the group totals $500, and net income is −$500. The undeclared parent is
classified as an expense from its declared root.
`the_seed_names_its_categories_at_the_requested_depth` pins the seed route at
depths 2 and 3.

**Not done:** dated lines (single-date `~` rules shown among the one-offs) have
no move controls. They are listed after the events in journal order and aren't
a list of their own. Docs screenshots were not regenerated.

### Accounts and Journal tabs

Patrick: "with hledger, I can see how all accounts are changing over time …
(optionally with empty accounts not mentioned so i can see what i've missed),
and I can see the forward projected journal". Two more tabs, **Accounts**
(`tab=accounts`) and **Journal** (`tab=journal`), digits 5 and 6.

**Core.**
- `reports::account_periods` (new module `account_periods.rs`) is
  `hledger balance -M` over `[from, to]` with a `BalanceMode`: `Change` (each
  period's movement, rows carry a Total) or `Historical` (closing balance,
  everything before `from` included, no Total). It returns the existing
  `PeriodReport`. The per-period pass is the cash flow report's own, extracted
  as `cash_flow::bucket_totals(txns, ranges, include)`, and the roll-up and depth
  clamp are its `pivot`, so no aggregation is duplicated. `known: Option<&[String]>`
  adds every named account, clamped with its ancestors, as all-zero rows (`-E
  --declared`). Without it, rows whose whole subtree is zero in every column are
  dropped (hledger's default). A zero parent with a live child stays.
- `Forecast.origins: Vec<ForecastOrigin>`, index-aligned with `txns`:
  `Rule(group)`, `Event(id)`, `Growth(group)`. It rides alongside because a
  `Transaction` has no slot for it, and a tag would leak into every report over
  the merged list. The date sort sorts the pairs.

**Wire: the run response, behind request flags.** Not a sibling endpoint: both
readings need the same expansion and the same merged list, and a second route
would expand the scenario twice per edit. `RunRequest` gained
`accounts: {mode: "change"|"balance", empty}` and `journal: bool`. The answer
gained `accounts: WirePeriodReport | null` and `journal: {total, entries:
[{origin: {kind, key}, txn: Row}], truncated} | null`. Each is computed only
when asked. The SPA asks only while the matching tab is open, so the four
original tabs' bodies are unchanged. The journal's `txn` is the journal view's
own `Row` (`RowContext::row`), so the client decodes it with `decodeRow` and the
generated tag (`generated-transaction: ~ monthly`) arrives as hledger prints
it. `MAX_JOURNAL_ENTRIES = 5000` caps it. Without a cap, the count is bounded only
by lines × occurrences, and the columns already obey `MAX_BUCKETS`. Past the cap,
the earliest entries are kept and `truncated` says so. `projected_statements`
now takes the `Merged {txns, prices}` built once per run, shared with Accounts.

**Valuation: none.** Accounts shows amounts as written and in natural signs,
like `hledger balance` without `-V`. The statements already give the valued
view. A per-account raw reading is the point of this tab, and valuing a change
column has no single right answer (hledger `-V` on change reports values each
period's change at its end). Consequence: grown prices (Decision 3) do not show
here, only on the balance sheet.

**Known accounts for "Show empty accounts":** declared (`account` directives),
every account posted to in the journal (before or after the cutoff), and every
account the forecast books to.

**The tab.**
- `ProjectedAccountsView.svelte` uses the statements' fixed template
  (`FigureColgroup`, `statementMinWidth`, `ProjectedHeadRow`, `projectedEdge`)
  and the Reports period tables' `compressPeriodRows`.
- A Change|Balance button pair and a Show empty accounts checkbox sit above it.
  They are in the URL as `accounts=change|balance&empty=0|1` (default change, 0)
  and are kept while other tabs are open.
- `ForecastJournalView.svelte` renders `print`-style blocks: a header with
  date, description and an origin badge (the rule's `~ period`, `event` or
  `growth`), then one line per posting.
- The journal tab's `TransactionTable` is NOT reused: it is bound to the
  windowed journal store and to editing, and its fixed-pitch virtualisation
  cannot represent variable-height blocks. `AmountCell` and `AccountLabel` are
  reused. The list is paginated at 100.
- Switching onto either tab re-runs the projection (the query differs). Until
  it lands, the held projection has `null` there and the tab shows a "Reading…"
  placeholder.

**Tests.**
- Core: `account_periods` (change, historical, depth, empty, zero parent, net
  zero) and `every_generated_transaction_names_its_origin`.
- Server (`projection_endpoints.rs`): null unless asked; change vs balance with
  `balance[i] = balance[i−1] + change[i]` on every row; depth; empty inclusion;
  bad mode is a `400`; journal entries in print shape with origins, agreeing
  with the Accounts columns; growth origin.
- Web: decoder (and the rename sweep), params, store query identity, `runBody`
  flags, component tests (tabs, digits 1–6, band, toggles, paging, cap), live
  integration, and two e2e specs.

**Not done.** No hledger parity test for `balance`/`print` was added to the
gated `parity_*` suite. Both readings sit on the merged list and expansion
those tests already pin. No keyboard row cursor on the Accounts table.
