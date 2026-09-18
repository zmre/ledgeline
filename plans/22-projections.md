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

A projection line has no funding leg, so the cash effect is implied. The rule,
stated once and tested:

> A period's cash delta is the sum of that period's postings to **cash-like
> asset accounts**, plus the negation of the sum of its **revenue and expense**
> postings — except that a rule which contains any cash posting of its own
> contributes no implied leg.

The exception is what stops a user who *does* write `(assets:checking) $-4200`
beside their rent line from having the rent counted twice. Cash-likeness is the
predicate the cash-flow report already uses (`cash_flow.rs:19` /
`cash_predicate`), so the two reports cannot disagree about what counts as cash.

Net worth moves by net income plus any posting to an asset or liability
account. Equity postings do not move it, which is what `net_worth.rs:179-192`
already means by net worth.

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
