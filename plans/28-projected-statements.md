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
