# 22 — Projections: a what-if you can edit, and the runway it implies

A new top-level **Projections** tab. The top half is an editable what-if table
seeded from your budget plus whatever the budget does not cover; the bottom half
is the same table read forward in time as net income, cash and net worth, with
the question that actually matters — when does the cash run out — answered on
its face. Scenarios are ordinary hledger journal files that nothing includes, so
they can be edited, diffed, committed and read by hledger itself. Driven by the
TODO entry "projections".

## The ask

> "projections: Seeing if you're above/below budget doesn't tell you much.
> Ultimately we need to be able to project into the future to understand the
> what-ifs and maybe live play with numbers to see how things change.
>   * New Projections top level tab
>   * Two sections: top and bottom
>     * Top section is the "what-if" section that starts with budget, plus a
>       bucket for any spending or income from the past 12 months that wasn't
>       budgeted as a line item.
>       * New lines can be added, too.
>       * For each line (revenue or expense), there's a column allowing for a
>         projected increase or decrease over some period. This would be
>         reflected as a percent change per week/month/year.
>       * There's also an ability to add one-time line items on particular
>         dates, which could allow projecting an investment, inheritance, bonus,
>         or whatever.
>       * Any projection can be saved to disk in a file not included from main
>         but standalone. You can also select and load an existing saved
>         projection.
>         * User should give it a name. File should be in the same format as a
>           budget file using comments and tags to add extra information.
>           Comments should include last updated date and created date. Name can
>           be anything and will govern display and file name.
>         * File names should be `projection-name-given-by-user.journal` and can
>           be in any subfolder under the repo so a user could organize them as
>           desired. Really any journal file with budget-like entries could be
>           used. We should default to showing the last loaded and if there isn't
>           one, default to using the active budget file.
>     * Bottom section needs to show reports (in tabs) over time. Similar to how
>       we show the cash flow over time right now where there are controls to set
>       intervals (monthly, quarterly, annually) and pick a number of periods.
>       * Given the projection's income and expense information, what net income
>         is being projected going forward?
>       * Sum up inflows, outflows, and net income using red/green to indicate
>         health.
>       * We should be able to project net worth forward, too … more at a summary
>         level.
>       * This would be useful as a sort of napkin-level runway check … For a
>         startup, but also for an individual, the goal would be to understand
>         runway and when/if cash balances turn negative"

On the shape of a non-recurring item, clarifying the sentence that broke off in
`TODO.md`:

> "I was imagining an investment coming in and then a hiring spree resulting in
> payroll jumping up, but then staying up and perhaps increasing at some
> percentage from that going forward. As opposed to a one-time purchase style
> expense."

## Decisions (locked with Patrick 2026-09-18)

1. **Two kinds of non-recurring item, and they are different things.** A
   **one-off** is a single dated amount (a purchase, a bonus, an inheritance).
   A **step** is a permanent change to a recurring line on a date — payroll
   jumps from $50k/mo to $120k/mo in April and *stays there*, growing from the
   new base. The step is the one the ask is really about, and it is the one a
   naive "one-time line item" feature would not give you.
2. **Whether an event hits net income is decided by the accounts it posts to,
   never by a tag.** A $2M raise posting to `assets:cash` and
   `equity:preferred` moves cash and net worth and leaves net income alone; a
   $45k legal bill posting to `expenses:legal` hits all three. The engine reads
   this off `resolve_account_type`, which is the rule everywhere else in the
   codebase ([[account-type-not-name]]) and means the format needs no new
   vocabulary for it.
3. **The projection math runs in the Rust engine; the SPA POSTs the whole
   scenario, saved or not.** Exact `Dec` money lives in one place and the wire
   already carries it losslessly. Live editing stays live because the round trip
   is to localhost. The rejected alternative — arithmetic in TypeScript — would
   duplicate the money math into a module the purity rule says gets ported back
   to Rust later anyway.
4. **Growth is stepwise, not smoothly compounded.** A line marked `+3%/yr` holds
   flat for twelve months and then bumps 3%. That is how a rent increase or a
   salary review actually lands, and it has a second, large benefit: the amount
   is constant within each growth unit, so a scenario is expressible as a
   handful of bounded `~` rules rather than one rule per bucket.
5. **Saving is always Save As.** The Projections tab never writes to
   `budget.journal` or to anything reachable by `include` from the main journal.
   Playing with a what-if cannot damage a plan of record.
6. **Growth lives in a posting tag; bounds live in the rule header.** Posting
   comments and tags already survive Ledgeline's parser (`parse.rs:1300`);
   bounded and single-date rule headers do not, which is what
   [`21-periodic-rule-parser.md`](21-periodic-rule-parser.md) fixes. A step
   change is therefore two bounded rules for the same account, linked by a
   shared `line:` tag.
7. **Growth is expanded by the engine, not written out expanded.** hledger has
   no arithmetic in amounts — `$1200 * 1.03` is a parse error — so a file
   cannot state a growing amount. The choice is between a compact file whose
   `growth:` tag only Ledgeline understands, and a file expanded into one
   bounded rule per step which plain hledger reads correctly. **Compact wins**,
   and `docs/projections.md` says plainly that `hledger --forecast` over a
   projection file shows base amounts without growth. The expanded form is
   recoverable later without a format change, because the tag is already there.
8. **Cash and net worth are projected at summary level only.** Opening balances
   come from the real journal as of today; each period adds the projected flows.
   Liabilities are held flat unless a projection line posts to one. No
   amortisation model — a mortgage payment cannot be split into principal and
   interest from anything the journal says, and guessing the split would
   silently invent a net-worth curve.
9. **The projection starts at the next whole bucket.** Opening balances are as
   of today; the first projected period is the next complete one (October, for a
   September today). A partial first period would make the first bar shorter
   than the rest for reasons that have nothing to do with the scenario. The tab
   states the start date.
10. **Last-loaded is remembered in `localStorage`, not `prefs.json`.**
    `WirePrefs` is `deny_unknown_fields` and `PUT /api/prefs` replaces the whole
    object with no PATCH (`import_api.rs:5615-5629`), so any older client that
    saved preferences would silently clear the field. `settings.svelte.ts`
    already holds per-browser view state and is the right home.

## The file format

A scenario is a journal file. This is the whole format:

```journal
; Ledgeline projection
; projection: Series A with a hiring ramp
; created: 2026-09-18
; updated: 2026-09-18

~ monthly  projection
    (revenues:salary)      $-12000  ; growth: 3%/yr
    (revenues:consulting)   $-2500
    (expenses:rent)          $4200  ; growth: 2%/yr
    (expenses:software)       $900

; A STEP: payroll jumps in April and grows from the new base. Two bounded rules
; for one account, linked by the `line:` tag so the editor shows one row.
~ monthly to 2027-04-01  projection
    (expenses:payroll)      $50000  ; line: payroll

~ monthly from 2027-04-01  projection
    (expenses:payroll)     $120000  ; line: payroll, growth: 5%/yr

; A ONE-OFF on the balance sheet: cash and equity, no net-income effect.
~ 2027-03-01  Series A
    (assets:cash)         $2000000
    (equity:preferred)   $-2000000

; A ONE-OFF on the P&L.
~ 2027-06-15  legal fees
    (expenses:legal)         $45000
```

Notes that are load-bearing:

- **`; projection: <name>` is the display name and the only marker.** A file
  without it still loads — "really any journal file with budget-like entries
  could be used" — and takes its name from its filename.
- **`to` is exclusive**, as hledger writes it. The April rule starts on the day
  the March rule stops.
- **Amounts are unbalanced virtual postings**, `(account) amount`, exactly as
  budget goals are. There is no funding leg and none is wanted; §"What moves
  cash" says how the cash effect is derived.
- **hledger reads this file.** `hledger -f projection-x.journal balance
  --budget -M` and `... print --forecast=2026-10-01..2029-10-01` both work. What
  hledger will not do is apply `growth:`.
- **Never round-trip a scenario through `hledger print`** — it drops `~` rules
  entirely. Ledgeline writes these files itself, through the span-splice
  machinery in `periodic.rs:576-596`, which touches only the bytes it must.

### What moves cash

A projection line has no funding leg, so the cash effect is implied. One rule,
stated once and tested:

> For each **group** — one `~` rule, or one dated event — sum **every** posting.
> The negation of that sum, the group's **residual**, is the implied cash leg,
> and it applies **in addition to** any cash postings the group already states.

Cash is an asset, so **net worth takes that same implied leg**, beside the
group's own asset and liability postings. Equity postings move neither series on
their own, which is what `net_worth.rs:179-192` already means by net worth.

A residual rather than a guard, because a guard has to answer "did this group
fund itself?" as a boolean, and can only answer for the fundings it recognises:

| group | residual → implied leg | net cash | net worth |
|---|---|---|---|
| `(expenses:rent) $4200` | −4200 | −4200 | −4200 |
| …beside `(assets:checking) $-4200` | 0 | −4200, the stated leg | −4200 |
| `(expenses:legal) $45000` beside `(liabilities:payable) $-45000` | 0 | **0** | −45,000 |
| `(assets:cash) $2M` beside `(equity:preferred) $-2M` | 0 | +2,000,000 | +2,000,000 |
| `(expenses:rent) $4200` beside `(assets:cash) $-2000` | −2200 | −4200 | −4200 |

Row two is the double-count a guard existed to stop, and the residual stops it
for a better reason: the group nets to zero. Row three is an **accrual**, and it
is what a cash-posting guard gets wrong — a liability is not cash, so the guard
would not fire and the bill would charge cash it has not yet cost. Row five is a
**partial** payment, which no boolean guard can express at all. Checked against
`hledger 1.52` over the accrual and its later settlement (`cf -M`, `bse -M`,
`is -M` all agree).

Cash-likeness is still the predicate the cash-flow report already uses
(`cash_flow.rs:19` / `cash_predicate`), so the two reports cannot disagree about
what counts as cash — but it now decides only which **stated** postings are cash,
never whether an implied leg exists.

**Net income is untouched by any of this.** It reads revenue and expense
postings only; amendment 3 states its orientation.

## Scope

In: the scenario model and its journal serializer, the projection engine, a run
endpoint, a seed endpoint, discovery/load/save of scenario files, the route, the
editable table, three report tabs with charts, and `docs/projections.md`.

Out of scope, on purpose, and named here so nobody adds them quietly:
retirement and tax modelling; asset-class growth (stocks, home value) — an
asset's balance is held flat; liability amortisation; per-account balance
projection; Monte Carlo or any distribution over outcomes; comparing two
scenarios side by side. Each is a plan of its own and several are explicitly
deferred in the ask.

**Depends on [`21-periodic-rule-parser.md`](21-periodic-rule-parser.md)
landing first** — bounded (`~ monthly from … to …`) and single-date
(`~ 2027-03-01`) rules are the format, and today they fail the whole journal.
Phase 1 also uses `budget_gaps` from
[`20-budget-pacing-gaps-and-sorting.md`](20-budget-pacing-gaps-and-sorting.md);
whichever plan lands first owns that function and the other reuses it — it must
not be written twice.

---

## Phase 1 — The engine and the wire

No UI, no file I/O. Everything here is testable from `cargo test`.

### Rust: `crates/ledgeline-core/src/projections.rs` (new module)

```rust
pub struct Scenario {
    pub name: String,
    pub created: Option<String>,
    pub updated: Option<String>,
    pub lines: Vec<ScenarioLine>,
    pub events: Vec<ScenarioEvent>,
}

/// One recurring row of the what-if table. A step change is TWO segments of one
/// logical line, sharing `id`.
pub struct ScenarioLine {
    pub id: String,
    pub account: AccountName,
    pub amount: Amount,
    pub period: PeriodSpec,           // from plan 21
    pub growth: Option<Growth>,
    pub note: String,
}

pub struct Growth { pub rate: Dec, pub unit: GrowthUnit }   // GrowthUnit = Week|Month|Year

/// A dated one-off: its postings say whether it touches the P&L, the balance
/// sheet, or both.
pub struct ScenarioEvent { pub date: String, pub description: String, pub postings: Vec<Posting> }
```

```rust
pub struct ProjectionOpts<'a> {
    /// Opening balances are taken as of this date; the first bucket is the next
    /// whole one after it.
    pub as_of: &'a str,
    pub interval: Interval,
    pub count: usize,
    pub depth: usize,
    pub declared: &'a BTreeMap<String, AccountType>,
    pub value_in: Option<Commodity>,
}

pub struct Projection {
    pub buckets: Vec<String>,
    /// Rows = projected revenue/expense accounts; totals = net income per bucket.
    pub net_income: PeriodReport,
    pub cash: BalanceSeries,
    pub net_worth: BalanceSeries,
    /// First bucket index whose closing cash is negative, if any.
    pub runway: Option<Runway>,
    pub warnings: Vec<String>,
}

pub struct BalanceSeries { pub opening: MixedAmount, pub values: Vec<MixedAmount> }
pub struct Runway { pub bucket: usize, pub date: String, pub periods: usize }

pub fn project(scenario: &Scenario, txns: &[Transaction], prices: &Prices, opts: &ProjectionOpts) -> Result<Projection, ReportError>;
```

Mechanics:

1. **Forward buckets.** `periods.rs` is trailing-only today —
   `last_n_buckets:322` is the whole vocabulary. Add its mirror:
   `next_n_buckets(start, interval, n) -> Result<Vec<String>>`, ~8 lines over
   the existing `next_bucket:343`, with the same `MAX_BUCKETS` cap. Mirror it in
   `web/src/lib/reports/periods.ts` (`lastNBuckets:147` / `nextBucket:167`),
   which has the same gap.
2. **Occurrences.** Reuse the forward walk at `reports/budget.rs:160-174`,
   intersected with each line's `PeriodSpec` bounds — which is the same code
   plan 21 teaches about bounds, so it should be lifted to a shared helper
   rather than copied.
3. **Growth.** At occurrence date `d`, `n` = whole growth units completed
   between the line's effective start and `d`; the amount is
   `base × (1 + rate)^n`, rounded to the amount's own `AmountStyle.precision`
   **at each step**, not at the end. Stepwise growth means the stepped figure is
   a real number a user could type, so rounding there is not a loss.
   `Dec` already has what this needs — `mul:125` and `rounded:199`, the latter
   rounding half-even via `rounded_half_even:339`. What is missing is the
   `MixedAmount` level: add
   `ma_scale(&self, factor: Dec) -> Result<MixedAmount, DecError>` to
   `mixed_amount.rs` beside `ma_add:112`.
4. **Opening balances.** Cash: as-of balances of cash-like asset accounts at
   `as_of`, computed the way `net_worth.rs:202-232` carries its `running` prefix
   sum. Net worth: `net_worth_priced` (`net_worth.rs:128`, already `pub(super)`
   for exactly this kind of reuse) at `as_of`.
5. **Roll forward** per bucket, applying §"What moves cash" and the net-worth
   rule.
6. **Warnings** for anything the projection silently could not do: an
   `Unsupported` period spec, a multi-commodity scenario where the opening
   balance is in another commodity, a growth rate on a line with no interval.
   A projection that quietly drops a line is worse than one that says so.

### Wire

| Route | Purpose |
|---|---|
| `POST /api/projections/run` | body `{scenario, asOf?, interval, count, depth, valueIn?}` → `WireProjection`. The scenario travels in the body precisely so unsaved edits project. |
| `GET /api/projections/seed?from=&to=` | → `WireScenario` built from the loaded journal's `~` rules plus `budget_gaps` over the trailing 12 months (one monthly line per unbudgeted category at its 12-month average). |

- `crates/ledgeline-server/src/projections_api.rs` (new). `WireProjection`
  embeds `WirePeriodReport` (`reports_api.rs:680`) for `netIncome` so
  `ReportTable` renders it with no changes, and money goes out through
  `wire_mixed:110` as everywhere else.
- Window params resolved through the shared `Window::resolve`
  (`reports_api.rs:1843`), so an interval the cash-flow report rejects is
  rejected here identically.
- Register in `lib.rs` **above the `route_layer` at `:800`** — below it, the
  route ships with no bearer token (`lib.rs:655`).
- `POST` with a body is new territory for a *report*; bound the body size and
  the line/event counts explicitly and 400 past them, rather than letting a
  scenario with 10,000 lines occupy a `compute` slot (`reports_api.rs:2036`).
- Goldens: append `projections-seed` to `fixtures/native/v1/requests.tsv` with
  pinned dates and `just snapshot-native`. `POST /api/projections/run` cannot
  join that manifest (it replays URIs only); pin it instead with a committed
  request/response pair under `fixtures/projections/` asserted by
  `crates/ledgeline-server/tests/projection_endpoints.rs`.

### Tests

- `projections.rs` unit tests: a flat monthly line over 12 buckets; stepwise
  growth bumping exactly on the anniversary and not before; a two-segment step
  line; a P&L one-off changing net income and cash but not equity; a
  balance-sheet one-off changing cash and net worth but not net income; the
  double-count guard when a rule states its own cash leg; runway detection and
  its absence; a scenario whose lines are all `Unsupported` producing warnings
  and a zero projection rather than an error.
- `periods.rs`: `next_n_buckets` across a year boundary, a leap year, and
  `n == 0`.
- `mixed_amount.rs`: `ma_scale` exactness, and that scaling an empty amount
  stays empty.
- `tests/projection_endpoints.rs`: the run body, the seed body, a 400 on an
  oversized scenario, and the token guard.

---

## Phase 2 — The tab

### Route and shell

- `web/src/routes/projections/+page.svelte` and a nav item in
  `+layout.svelte:89-102`. `routes/budget/` is the worked example of a
  standalone feature route with its own params codec and its own store.
- `web/src/lib/projections/params.ts`: `{interval, count, depth, tab}` mirrored
  to the URL via `searchMirror` (`url/searchSync.ts:50`), parsed once on mount.
  Defaults: **monthly × 24** — two years is the horizon a runway question is
  usually asked over, and it is still inside `MAX_COUNT`.
- `web/src/lib/projections/scenarioStore.svelte.ts`: the scenario is `$state`
  owned by the store; the projection is a `createResource`
  (`stores/resource.svelte.ts:55`) keyed on `{scenario, window}`. The resource's
  "the payload and the query it answers are one value" invariant is what stops a
  stale projection rendering against an edited table.
- Recompute is **debounced 250ms** on any scenario edit — the same debounce the
  URL mirror uses, and for the same reason.

### The what-if table

Sections mirroring the budget's: **Income**, **Expenses**, **One-off events**.
Columns:

| Column | Notes |
|---|---|
| Account | the existing `AccountInput` combobox — segment-aware, already portalled |
| Amount | magnitude, never signed; the sign flip for revenue happens in one place, as it does for budget goals (`budget_api.rs:1012`) |
| Per | weekly / monthly / quarterly / yearly |
| Growth | a rate and a unit (`+3 % / yr`); blank means flat |
| From / To | optional bounds; setting one on an existing line offers "make this a step" |
| — | row menu: add a step, duplicate, delete |

An event row is a date, a description, and one or more account/amount pairs.

Seeding: on first load with nothing remembered, `GET /api/projections/seed`
fills the table from the budget plus the unbudgeted categories. The unbudgeted
rows arrive flagged so the UI can say where they came from — they are an
estimate from history, not something the user wrote.

### The bottom half

Three tabs — **Net income**, **Cash & runway**, **Net worth** — over shared
interval/count controls, styled on `ReportControls.svelte`.

- **Net income**: a `PeriodReport`, so `ReportTable` renders the table
  unchanged, above a chart with inflows above the axis, outflows below, and net
  as a line. Red/green by sign, per the ask.
- **Cash & runway**: the cash series as a line with a zero rule, the crossing
  point marked and stated in words above the chart ("cash turns negative in
  March 2028 — 18 months"). When it never crosses, the same slot says so and
  gives the average monthly build instead.
- **Net worth**: the single summary line, opening balance labelled.

Charts need the `dataviz` skill loaded before any chart code is written
(`plans/00-overview.md:123-127`). The app has **no bar or area chart today** —
`HoldingsTrend.svelte` is the only line chart and is bespoke. Generalise it into
`web/src/lib/components/PeriodLineChart.svelte` taking `{labels, series[],
formatValue, formatAxis}`, keeping its conventions (x is the bucket *index* with
a string axis formatter; explicit integer `xTicks` computed to ~6 labels;
`points` only when ≤31 buckets) and port `HoldingsTrend` onto it in the same
commit, or the codebase gains a second chart idiom.

The inflow/outflow bar chart is **the same component the TODO wants over the
Cash Flow report** ("stacked area line graph over the Cash Flow report with a
dotted line over the top showing the Net"). Build it as
`PeriodFlowChart.svelte` with a props shape that report can adopt, and say so in
its header comment.

### Tests

- `projections/params.test.ts`: the URL codec round trip and fallbacks.
- `projections/scenarioModel.test.ts`: adding a step splits a line into two
  bounded segments and merges back; deleting one segment of a step; the
  revenue sign flip.
- `ProjectionsTable.svelte.test.ts`: a seeded scenario renders its unbudgeted
  rows flagged; editing an amount marks the scenario dirty.
- `PeriodFlowChart.svelte.test.ts`: the series handed to the chart, not its
  geometry — jsdom has no layout engine.

---

## Phase 3 — Files

### Discovery

`crates/ledgeline-core/src/projections/discovery.rs`, modelled closely on
`crates/ledgeline-core/src/rules/discovery.rs` — same root
(`parse::include_root_for:888`), same containment test (`parse::confine:980`),
same refusal of every symlink (`discovery.rs:21-31`), same caps
(`MAX_RULES_DEPTH` 8, 200 files, 20,000 entries), same `SKIP_DIRS` and
hidden-entry skip, same re-scan-per-request-never-cache policy
(`rules_api.rs:84-91`).

It scans for `*.journal`, not just `projection-*.journal`, because the ask says
any journal with budget-like entries may be used. The listing groups them:
files matching `projection-*.journal` first, everything else under "Other
journals". Only the former get their `; projection:`/`; created:`/`; updated:`
header read, from a bounded head-read of the first few kilobytes — reading every
journal's head to display a picker is work nobody asked for.

The critical difference from the budget editor: `budget_api::journal_path:1250`
resolves a handle by membership in `Journal::source_files`, and a projection
file is by definition not in that set. Resolution is by membership in the
freshly-scanned discovery set instead (`discovery.rs:502`), and creation goes
through `resolve_new` (`discovery.rs:572-616`) + `create_exclusive`
(`rules_api.rs:1605`) — `O_EXCL`, so the refusal to overwrite is the kernel's
and is decided atomically, not `atomic_write`'s rename.

### Routes

| Route | Purpose |
|---|---|
| `GET /api/projections` | the discovery listing: `{rootLabel, files[], truncated, warnings[]}`, no absolute paths ever (`discovery.rs:33-41`) |
| `GET /api/projections/{*id}` | parse one file → `WireScenario` + `revision` |
| `PUT /api/projections/{*id}` | save; `revision: ""` means create (`NEW_FILE_REVISION`, `rules_api.rs:200-207`) |

Three things the budget write pipeline does that this one must **not**:

1. **Validation is a standalone parse.** `parse_journal_with_overrides` against
   the main journal (`budget_api.rs:800-811`) cannot see a file the main journal
   does not include. Validate with `parse::parse_journal(text, name)`
   (`parse.rs:286`) over the projection file alone, and require that the goals
   read back as the numbers requested — the `confirm_written_goal` step
   (`budget_api.rs:812`) is worth keeping.
2. **Do not call `state.reopen_editor()`** (`budget_api.rs:842`,
   `lib.rs:390-414`). It re-opens the main journal, which a non-included file
   cannot have changed.
3. **Do not create directories.** `resolve_new` requires a real, non-symlink
   parent to exist already (`discovery.rs:547-550`). The Save As dialog offers
   the directories the scan found; making new ones is the user's job, in their
   own file manager or shell.

Everything else is reused verbatim: `Fingerprint` revisions (`edit.rs:188-215`),
the three-point staleness check, `PeriodicDoc::parse/plan/apply/verify`
(`periodic.rs:488/1092/576/621`), and `atomic_write` for an update.

Write serialization takes `state.import_writes()` (`lib.rs:299-304`), **not** a
new mutex: a projection file is a journal file, and nothing stops a user from
`include`-ing one from their main journal later. Sharing the lock costs a little
contention and removes a whole class of future bug.

### Save As, and the name

The name governs both display and filename. `projection-<slug>.journal`, where
the slug is the name lowercased with runs of non-alphanumerics collapsed to `-`
and the ends trimmed. The dialog shows the resulting path before saving, offers
a directory from the scan, and refuses rather than overwriting — the `O_EXCL`
refusal surfaced as "a file already exists there".

Header comments are rewritten on every save: `created:` preserved from the
loaded file (or today for a new one), `updated:` set to today.

Last-loaded id goes in `settings.svelte.ts` beside `insightsTab:35`, validated
on load like every other field there, and is used on next mount to reopen the
same scenario.

### Tests

- `projections/discovery.rs` unit tests: the caps, the symlink refusal, the
  hidden skip, the header head-read, and that a path outside the root is
  refused.
- `tests/projection_files.rs`: round-trip a scenario through serialize → write →
  parse → deserialize and assert the model is identical; assert every byte
  outside the edited spans is unchanged on an update; assert a 409 on a stale
  revision; assert creating over an existing file fails.
- An hledger cross-check behind the existing opt-in pattern
  (`LEDGELINE_HLEDGER_*_CHECK`, `just hledger-checks`): a written scenario file
  parses under `hledger -f … print` and its `balance --budget -M` agrees with
  our goals for the no-growth lines. Opt-in, because `cargo test` stays
  hermetic.
- `e2e/projections.e2e.ts`: the tab is reachable, seeds from the fixture
  journal, and the three report tabs render. **Read-only** — it must not write a
  file, for the reason `budget.e2e.ts` gives about fixtures other specs assert
  exact numbers from. The write path is proved in the Rust endpoint tests,
  against bytes.

---

## Sequencing

```
plan 21 (parser) ──> Phase 1 (engine + wire) ──> Phase 2 (tab) ──> Phase 3 (files)
plan 20 (budget)  ──┘  [owns budget_gaps, used by Phase 1's seed]
```

Phases 1 and 2 may be different agents if Phase 1's wire types are fixed first;
Phase 3 must follow Phase 2 because the Save As dialog needs the tab. Phase 1 is
the one to get right — everything above it renders what it computes.

## Where the code is

| Path | Purpose |
|---|---|
| `crates/ledgeline-core/src/projections.rs` | `Scenario`, `project`, the growth expansion, the cash and net-worth rules |
| `crates/ledgeline-core/src/projections/discovery.rs` | the `*.journal` scan, rooted and capped like the rules scan |
| `crates/ledgeline-core/src/projections/serialize.rs` | scenario ⇄ journal text |
| `crates/ledgeline-core/src/reports/periods.rs` | `next_n_buckets` |
| `crates/ledgeline-core/src/reports/mixed_amount.rs` | `ma_scale` |
| `crates/ledgeline-server/src/projections_api.rs` | run, seed, list, load, save |
| `web/src/routes/projections/+page.svelte` | the route |
| `web/src/lib/projections/` | params, scenario store, model helpers |
| `web/src/lib/projections/ui/` | the what-if table, the report tabs, Save As |
| `web/src/lib/components/PeriodLineChart.svelte` | generalised from `HoldingsTrend` |
| `web/src/lib/components/PeriodFlowChart.svelte` | inflow/outflow bars + net line; the Cash Flow report adopts it later |
| `docs/projections.md` | the format, the tags, what hledger does and does not do with it |

`docs/projections.md` is warranted because this introduces a tag vocabulary
(`projection:`, `growth:`, `line:`) — which is the repo's stated trigger for a
doc — and because the honest caveat about `--forecast` and growth has to live
somewhere a user will find it. Link it from `README.md` beside the other feature
docs.

## Testing

| Level | Covers |
|---|---|
| `projections.rs` unit tests | growth steps, both kinds of one-off, the cash double-count guard, runway, warnings |
| `periods.rs` / `mixed_amount.rs` unit tests | `next_n_buckets`, `ma_scale` exactness |
| `projections/discovery.rs` unit tests | the caps, symlinks, hidden entries, containment |
| `tests/projection_endpoints.rs` | run, seed, the body-size 400, the token guard |
| `tests/projection_files.rs` | serialize round trip, byte-level non-disturbance, 409, create-over-existing |
| `hledger` opt-in check | a written scenario parses and balances under hledger 1.52 |
| `projections/params.test.ts` | the URL codec |
| `projections/scenarioModel.test.ts` | step split/merge, the revenue sign flip |
| `ProjectionsTable.svelte.test.ts` | seeded rows are flagged; an edit marks dirty |
| `nativeDecode.test.ts` | the seed golden; an absent amount throws rather than defaulting to zero |
| `e2e/projections.e2e.ts` | reachable, seeds, three tabs render — read-only |

Deliberately not tested: chart geometry, and any assertion that a projected
figure is "right" in the sense of matching a spreadsheet. The engine tests pin
the arithmetic; nothing pins the modelling, because the modelling is the user's.

## Definition of done (per phase)

- `just engine-check`, `just engine-test`, `just check`, `just test`,
  `just lint`, `just e2e` green.
- `cargo test` stays hermetic; the hledger cross-check passes locally via
  `just hledger-checks`.
- `just snapshot-native` re-run for the seed route; the diff reviewed.
- New behaviour has a test that failed before the change landed.
- UI exercised in the dev server at 375px and desktop width.
- `docs/projections.md` written; `README.md` links it; the TODO entry deleted
  when Phase 3 lands.
- Any contract in this doc that changed during implementation is amended here in
  the same commit, per `plans/00-overview.md` convention #9.

---

## Contract amendments made during implementation

**Phase 1 only.** Everything below was established while building the engine and
the wire; Phases 2 and 3 should trust these over the prose above.

### 1. `ScenarioLine` needed a `group` beside its `id`

The sketch gave `ScenarioLine` one identifier and said a step change is two
segments "sharing `id`". But §"What moves cash" states the double-count guard
per **rule** — "a rule which contains any cash posting of its own" — and a model
with one posting per line has nothing to group on. A user's

```journal
~ monthly  projection
    (expenses:rent)     $4200
    (assets:checking)  $-4200
```

would have become two independent lines, and the rent would have been counted
twice: once as the cash posting the user wrote, once as the leg the engine
derives.

So there are two identifiers, answering two different questions:

| field | means | used by |
|---|---|---|
| `id` | the LOGICAL ROW — two bounded segments of one step change share it | the editor, so a step is one row |
| `group` | the SOURCE RULE — every posting of one `~` block shares it | the two derived-leg guards |

`ScenarioEvent` also gained an **`id`**, for the same reason plus one more: an
event is its own group, and a group key that could collide with a line's would
merge two unrelated things. (The engine namespaces them internally as
`line:<group>` / `event:<id>`, so an id shared between the two is still safe.)

### 2. The net-worth rule as written DOUBLE COUNTS

§"What moves cash" ends: "Net worth moves by net income plus any posting to an
asset or liability account." That is wrong for any rule that states its own
funding leg. Rent $4200 with `(assets:checking) $-4200` gives net income −4200
AND an asset posting of −4200, for −8400 — twice the truth.

The corrected rule is the **symmetric** one, and it is now what the module doc
says and what the tests pin:

> For each group, the implied leg is the negation of the sum of that group's
> revenue and expense postings — i.e. its net income. It is contributed to the
> CASH series unless the group already posts to a **cash-like** account, and to
> the NET WORTH series unless the group already posts to an **asset or
> liability** account.

The cash half is exactly the plan's own sentence; only the net-worth half moved.
Checked against all four shapes the plan names (a P&L-only rule, a rule with its
own cash leg, the `assets:cash` + `equity:preferred` raise, the
`expenses:legal` one-off) and against an equity-only event, which correctly moves
nothing.

**Known limitation, deliberately kept:** an ACCRUAL — `(expenses:legal) $45000`
beside `(liabilities:payable) $-45000` — still implies a cash leg, because the
guard looks for a CASH posting and a liability is not one. Net worth is right;
cash is pessimistic by the accrued amount. Fixing it means replacing the guard
with "book the group's residual to cash", which is a better model but a
different contract from the one this plan locked. `docs/projections.md` (Phase 3)
should say so out loud.

### 3. `net_income` rows are in CASH-FLOW ORIENTATION — revenue POSITIVE

The plan says "Rows = projected revenue/expense accounts; totals = net income per
bucket" without saying which way up. Taken literally with natural posting signs,
`totals` would be the NEGATION of the row sum, breaking the one invariant every
`PeriodReport` in this codebase keeps and that `ReportTable` renders on.

So the rows are sign-flipped: **revenue positive (an inflow), expenses negative
(an outflow), and `totals[i] == Σ rows[i] == net income`.** This is the opposite
of `/api/reports/incomestatement`, and it is the orientation Phase 2's chart
already wants ("inflows above the axis, outflows below, net as a line").

**Phase 2 must not flip again.** A `$4200` rent line arrives as `-4200`.

### 4. `Dec::rounded` is half AWAY FROM ZERO, not half-even

Phase 1 §3 says "`rounded:199`, the latter rounding half-even via
`rounded_half_even:339`". It does not: `Dec::rounded` rounds half away from zero
(`decimal.rs:208`), matching `Data.Decimal`'s `roundTo`. `rounded_half_even` is a
**private** helper used only by `Dec::parse` to cap at `MAX_PARSE_PLACES`.

Half-away-from-zero is the right convention here anyway — it is what a reader
expects of a displayed figure — but the difference is observable. `$1000` at
`5%/yr`, whole dollars, steps `1050, 1103, 1158, 1216, 1277`; the second step is
`1102.50` and rounds UP. Pinned by
`growth_rounds_at_each_step_not_once_at_the_end`, which also shows the fifth year
is where stepwise (1277) and compound-then-round-once (1276) part company.

### 5. `project`'s signature

```rust
pub fn project(
    scenario: &Scenario,
    txns: &[Transaction],
    prices: &[PriceDirective],      // NOT `&Prices` — there is no such type
    opts: &ProjectionOpts,
) -> Result<Projection, ReportError>
```

`prices` is the journal's explicit `P` directives, exactly as `net_worth` takes
them; the inferred cost prices are derived inside, so opening balances value
identically to `/api/reports/networth`.

Opening net worth comes from the **public** `net_worth` with `count: 1`, not from
`net_worth_priced`. The plan said the latter is "already `pub(super)` for exactly
this kind of reuse" — but `pub(super)` means visible inside `reports`, and
`projections` is a crate-root module. The numbers are identical; the cost is one
extra `infer_market_prices` pass per request, which happens once.

### 6. `Projection` gained `start`, `ScenarioLine` gained `source`

- `Projection.start` — the first day of the first bucket. Decision 9 makes the
  start a derived fact ("the tab states the start date"), and a UI that
  re-derived it would be re-implementing "the next whole bucket".
- `ScenarioLine.source` — `LineSource::Journal | Unbudgeted`. Phase 2 requires
  seeded gap rows to "arrive flagged"; this is that flag, and it reaches the wire
  as `"journal"` / `"unbudgeted"`.

### 7. `occurrences` is shared, not lifted

`reports::budget::occurrences` became `pub(crate)` and `projections` calls it
directly. Lifting it to a new shared module would have moved the function the
`budget_golden.rs` hledger oracle validates away from the report it validates.
Its dates are ASCENDING (every branch builds them that way), which is now
documented, because the growth walk relies on it to step once rather than
recompute from the anchor per occurrence.

### 8. `parse::parse_period_spec` is now `pub`

It was `pub(crate)` (plan 21 amendment #4). The run endpoint takes a period as
the words a user typed, in a request BODY, and re-parsing them with the journal's
own grammar is what guarantees a projected line fires on the days the same line
fires on once saved and read back. The alternative was a second grammar at the
HTTP boundary, invisible to `budget_golden.rs`.

### 9. `ma_scale` exists and is used, but growth does not round at the `MixedAmount` level

`ma_scale` is on `MixedAmount` as specified. The growth step is
`ma_scale(1 + rate)` followed by a per-commodity `Dec::rounded(precision)`,
because "round to the amount's own `AmountStyle.precision`" is a per-AMOUNT fact
that a `MixedAmount` does not carry.

Worth knowing: **`Dec::mul` normalizes.** `$4200.00 × 1.02` is `4284` at scale 0,
not `4284.00`. The explicit `rounded` is what puts the cents back, so a scaled
amount without it would ship a different `places` on the wire.

### 10. Growth details the plan did not state

- **The growth anchor** is the line's `from` when it has one, else the
  projection's start. That is what makes the second segment of a step change grow
  from its NEW base (`~ monthly from 2027-04-01 … growth: 5%/yr` bumps on
  2028-04-01, not on the projection's own anniversary).
- **Months count ANNIVERSARIES, clamped**, not calendar-month differences. From
  2026-01-15: 2026-02-14 is zero months, 2026-02-15 is one. From 2026-01-31:
  2026-02-28 is one — the same clamp `periods::clamped_date` applies to a
  `~ monthly from 2026-01-31` rule's occurrences. Years are whole months / 12.
- **`rate` is a FRACTION everywhere in the model and on the wire** — `3%/yr` is
  `Dec::new(3, 2)`. The `%` belongs to the file format and the UI. `parse_growth`
  reads `3%/yr`, `0.03/year`, `2.5%/mo` and `-1%/wk`; anything it cannot read is
  a FLAT line, not a failed file.
- **A growth overflow is a warning, not a `500`.** A compounding amount can
  outgrow `i128` long before the span does (10%/week over 1200 weeks is ~10^49).
  The line holds its last representable amount and says so in `warnings`.
- **Growth on a `Once` line warns** that it never applies.

### 11. The seed route is `?end=&count=&depth=`, NOT `?from=&to=`

The wire table says `GET /api/projections/seed?from=&to=`. It cannot be: the seed
reuses `budget_gaps`, whose window is `BudgetOpts { end, interval, count, depth }`,
and the plan also requires the params to go through the shared `Window::resolve`.
A `from`/`to` pair would have had to be converted into an interval and a count,
which is a third way of saying the same window.

There is deliberately **no `interval` param**: the seeded lines are monthly and
their figures are monthly averages, so an interval could only offer a window
whose buckets do not match the lines it produces. `Window::resolve` is given
`None` and resolves to monthly.

Seed defaults: `end` = today, `count` = 12, `depth` = 2. The divisor for the
average is the BUCKET COUNT, and the last bucket is truncated at `end` — so a
part-month at the right-hand edge makes the average slightly conservative, on
purpose.

### 12. A two-commodity gap seeds TWO lines

One `ScenarioLine` holds one `Amount`. `fixtures/sample.journal`'s
`expenses:food` and `expenses:travel` each have `$` and `EUR` activity, so each
seeds two lines with distinct ids (`gap:<account>:<commodity>`). Collapsing them
would mean picking a commodity, which is a valuation the seed has no business
performing — and the projection warns when a scenario's commodities are not the
one the opening balances are valued in.

A seeded scenario has **no name**: naming it is the Save As dialog's job, and a
placeholder would become a filename nobody chose.

`seed_scenario` does **not** convert `~ DATE` rules into `ScenarioEvent`s — they
stay `Once` lines, because a `~` rule is a rule and the file round trip should say
so. Phase 2 may route `period.simple == null` plus a single-date `raw` into the
"One-off events" section for display; the engine treats both identically.

### 13. `Window::resolve_named`, so the date error names the field that was sent

`Window::resolve` hard-coded `"end"` in its date `400`. The run endpoint's date
is `asOf`. `resolve_named(field, …)` was added and `resolve` now delegates to it;
no existing call site changed.

`reports_api::checked_date` also became `pub(crate)`: a scenario carries event
DATES in a request body, and those reach the same bucket math with the same RPT-4
exposure. Note it **normalizes** as well as validates — `2026-9-1` is accepted
(as hledger's own `-b`/`-e` accept it) and becomes `2026-09-01`; `2026-02-30` and
`garbage` are `400`s.

### 14. Wire details Phase 2 needs

- `WireProjection` = `{buckets, start, netIncome, cash, netWorth, runway,
  warnings}`. `netIncome` is a `WirePeriodReport`, unchanged, so `ReportTable`
  renders it with no edits.
- `cash` / `netWorth` are `{opening, values}` — `opening` is a real figure from
  the journal, NOT `values[-1]` of some earlier window.
- `runway` is `null` or `{bucket, bucketKey, label, date, periods}`. `bucketKey`
  and `label` are beyond the engine's `Runway` struct: the sentence above the
  chart should not have to index back into `buckets` and hope the two agree.
  `date` is the bucket's LAST day; `periods` is `bucket + 1`. "Negative" means
  any commodity's closing balance is negative.
- **Every optional field serializes as `null`, never omitted.** A scenario is a
  round-trip shape, and absent-versus-null is the distinction a hand-written
  decoder gets wrong.
- `WireAmount` carries `precision` beside `quantity`. It is the DISPLAY
  precision, it is not `quantity.places`, and the growth curve is wrong without
  it — a client must send it back.
- `period` goes OUT as `{raw, simple, from, to}` and comes IN as an object with
  `raw` required. `PeriodIn` is the one inbound type WITHOUT
  `deny_unknown_fields`, precisely so a client can hand back the object it was
  given; the derived fields are ignored and `raw` is re-parsed.
- `growth` is `{rate, unit}` with `unit` ∈ `week|month|year`; `source` is
  `journal|unbudgeted`. An unrecognized value in either is a `400`, not a silent
  fallback.
- An event's postings are FLATTENED on the way out: one wire posting per
  (posting, amount) pair, so a row is always one account and one amount.

### 15. Limits, and what status each produces

| guard | limit | result |
|---|---|---|
| request body | `MAX_BODY_BYTES` = 256 KiB, route-local `DefaultBodyLimit` | `400` (via `json_body`, not a `413`) |
| `scenario.lines` | 1000 | `400` naming the limit and the count sent |
| `scenario.events` | 1000 | `400` |
| one event's `postings` | 100 | `400` naming the event |
| `count` | `1..=MAX_BUCKETS`, via `parse_count` | `400` |

The counts are checked BEFORE any line is decoded and before a `compute` slot is
claimed: refusing an oversized scenario has to be cheap.

### 16. Goldens, and the TypeScript side

- `just snapshot-native` was **not** run (it binds a port). The
  `projections-seed` golden was produced by replaying the manifest URI through
  the same `tower` oneshot `native_wire_golden.rs` uses, and is verified by it.
  Every pre-existing golden is byte-identical.
- `native_wire_golden.rs`'s manifest count assertion moved **15 → 16**.
- `POST /api/projections/run` is pinned by
  `crates/ledgeline-server/tests/projection_endpoints.rs`, as the plan says —
  though under that name alone, with no committed pair under
  `fixtures/projections/`: a committed request/response adds a second place for
  the wire to be stated, and the test asserts the same bytes from one.
- **`nativeDecode.test.ts` has NO entry for the seed golden yet, and that is
  Phase 2's job.** There is no `decodeScenario`/`decodeProjection` in
  `nativeDecode.ts` — Phase 1 stopped at the Rust wire. Nothing broke (that
  file's sweep iterates an explicit `DECODERS` list, not the fixtures
  directory), but until Phase 2 adds the decoders and their sweep entries, a
  renamed field in `projections_api.rs` fails only on the Rust side. **Phase 2
  must add both decoders to `DECODERS`, and must not add anything to
  `TOLERATED`.**

### 17. `next_n_buckets` / `nextNBuckets`

Both landed as specified. The Rust one clamps `n` at `MAX_BUCKETS` (its `n`
arrives from a query); the TS one does not, matching its own `lastNBuckets` —
which is stated in its doc comment rather than left to be discovered.

`reports::test_support` became `pub(crate)` so `projections`' tests build a
transaction with the same helpers every report test uses.

### 18. The two derived-leg guards were WRONG for accruals — the rule is the RESIDUAL

**This supersedes amendment 2 and the original §"What moves cash", which has
been rewritten above. Read the rewritten section, not either of the two
formulations this replaces.**

Amendment 2 fixed a genuine double-count in the net-worth half, but it kept the
plan's cash formulation intact: negate the revenue/expense sum, *except* that a
group containing its own **cash** posting contributes no implied leg. It then
named the consequence as a "known limitation, deliberately kept". That judgement
was wrong. This is not a rounding of the model; it is a class of scenario the tab
answers incorrectly:

```journal
~ 2027-06-15  legal fees, net 60
    (expenses:legal)          $45000
    (liabilities:payable)    $-45000
```

The guard looks for a CASH posting. `liabilities:payable` is not one, so it does
not fire, and the projection charges $45,000 of cash in June — cash the scenario
says plainly is not paid in June. For a tab whose entire purpose is "when does
the cash run out", a pessimistic runway that **no scenario can express its way
out of** is a wrong answer, not a caveat.

Both guards are replaced by the single residual rule now stated in §"What moves
cash": sum every posting in the group; the negation of that sum is the implied
cash leg, applied BESIDE whatever the group already states. The net-worth guard
from amendment 2 disappears entirely — the implied leg is cash, cash is an asset,
so the net-worth series simply takes it too.

Why this is strictly better rather than merely different:

- Every case the guards got right, the residual also gets right, and for a
  *reason* rather than a coincidence: a group that states its own funding leg has
  a residual of zero.
- It is **additive**, not exclusive, so a PARTIAL funding leg works — `$4200` of
  rent against `(assets:cash) $-2000` implies the remaining `$-2200`. A boolean
  guard could only choose all or nothing.
- It needs no notion of "which postings were the funding". That is the question
  the guard could not answer for a liability, and would equally have failed on a
  receivable, a prepaid, or an inter-account transfer — whatever a user writes
  next.

**What went dead:** the `has_cash` and `has_balance_sheet` flags, and the
`profit_and_loss` accumulator that existed only to form the implied leg. The
per-group `residual` is summed instead. `AccountTypes::is_cash` survives, now
classifying only stated postings and the opening balance.

**Nothing on the wire moved.** `WireProjection` and every field are exactly as
amendment 14 describes; only the numbers a scenario with a non-cash counter-leg
produces. `fixtures/native/v1/projections-seed.json` is byte-identical, as it
must be — the seed route does not run a projection.

`docs/projections.md` (Phase 3) should document the residual rule, and must NOT
carry amendment 2's "cash is pessimistic for accruals" caveat. It no longer is.
