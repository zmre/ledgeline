# 23 — Assets that grow: a balance, a rate, and what you put into it

A third recurring section in the what-if table, for the things a projection
holds rather than the things it earns and spends. An asset row is an opening
balance, a growth rate, and an optional contribution — enough to model a
brokerage account, a 401k or a house in one line. Driven by the gap Patrick hit
on first use of the Projections tab, and by
[`22-projections.md`](22-projections.md)'s deliberate exclusion of it.

## The ask

> "I want to model growth of assets over time, but as that isn't an inflow or
> outflow, that isn't possible."

Clarified in the same session:

> asset rows should be **"Opening balance + growth + contributions"**; growth is
> **"net worth only, but also make sure negative cash flow negatively impacts
> net worth"**; a recurring contribution belongs **"on the asset row itself"**.

Plan 22 §Scope excluded this on purpose — "asset-class growth (stocks, home
value) — an asset's balance is held flat". This plan removes that exclusion and
nothing else from it.

## Decisions (locked with Patrick 2026-09-20)

1. **An asset row is a balance, not a flow, and that is the whole difference.**
   Every row in the table today is a per-period amount. An asset row states a
   *stock*: what you hold, what rate it compounds at, and what you add to it.
   The engine has to carry a running balance for it rather than summing
   occurrences.
2. **Appreciation moves net worth and never touches net income or cash.** Paper
   gains do not pay salaries, and a runway number that counted them would be
   worse than one that ignored them. This also matches how `net_worth.rs`
   already values holdings.
3. **Negative cash flow must still drag net worth down**, and it already does —
   `projections.rs:1695` pins `[-4200, -8400, -12600]` for a recurring $4200
   expense, because the residual rule sends the implied cash leg to both series
   (amendment 18). This plan must not break that, and adds an explicit
   regression test combining a burning scenario with a growing asset: the asset
   must not mask the burn.
4. **The contribution lives on the asset row, and the cash outflow is derived
   from it.** One row per asset reads as one idea. Deriving the cash movement
   rather than asking for a second row is what stops the two drifting apart.
5. **A contribution is net-worth neutral.** $2,000 leaving cash and arriving in
   savings changes where your money is, not how much you have. Only the growth
   moves net worth.
6. **Growth is stepwise**, exactly as plan 22 Decision 4 has it for flows: a
   `7%/yr` row holds flat for twelve months and then bumps. One growth model in
   the product, not two.
7. **A contribution earns no growth until the next rate boundary.** Growth
   boundaries and contribution occurrences are merged in date order; a
   contribution dated on a boundary does not earn that step. So with a `7%/yr`
   rate and monthly contributions, a January deposit and a November deposit both
   earn nothing that year and both take the full bump on 1 January.

   **This supersedes the first drafting of this decision**, which said growth
   applies "before that period's contributions" and illustrated it as
   time-weighting ("a contribution made in month eleven does not earn a full
   year's return"). That was wrong about the mechanism: nothing is
   time-weighted, and a January deposit is treated exactly like a November one.

   Confirmed with Patrick on 2026-09-20, against the two alternatives —
   time-weighting within a period, and an annual rate compounded monthly. The
   argument for keeping it: the error runs **conservative**, understating growth
   on contributions rather than inflating a retirement or runway figure; it is
   one rule rather than two; and it keeps a single balance per row, so the
   table's Balance column shows exactly the figure the engine used. A user who
   wants finer granularity enters a monthly rate.

   This belongs in `docs/projections.md` stated plainly, because it is exactly
   the kind of convention that makes a user's spreadsheet disagree with ours.
8. **Liabilities are still out.** The row shape would extend to them, but a
   mortgage needs principal-vs-interest, which plan 22 Decision 8 already
   established cannot be recovered from the journal. Assets only; revisit with
   an amortisation model or not at all.

## The file format, verified against hledger 1.52

An asset row is a posting in the same `~` rule as everything else. Its **amount
is the contribution**, and its tags carry what a posting cannot say:

```journal
~ monthly  projection
    (assets:brokerage)      $0  ; growth: 7%/yr
    (assets:savings)     $2000  ; growth: 4%/yr
    (expenses:rent)      $4200
```

A row with no contribution is written `$0`. That is not a trick — hledger
accepts it, `print` round-trips it with the tag intact, and `balance --budget`
simply shows no goal for it, which is the honest answer because a growth rate is
not a goal:

```
                || Commodity       Jan       Feb
================++===============================
 assets:savings || $          0 [2000]  0 [2000]
 expenses:rent  || $          0 [4200]  0 [4200]
```

Note `assets:brokerage` is absent, and `assets:savings` shows only its
contribution. Verified directly; `tag:growth` also matches the generated
transactions under `--forecast`.

An opening balance is **not** written unless the user overrides the journal's —
see §"Not double-counting the opening balance". When overridden it rides along
as `; opening: 200000`.

## Scope

In: the asset row (model, file format, engine, UI section), its effect on the
net-worth series, and the docs.

Out of scope, on purpose: liabilities and amortisation (Decision 8); per-holding
or per-lot modelling — a row is one account, not a portfolio; tax on gains;
rebalancing; and any attempt to infer a growth rate from price history, which
would make a projection quietly depend on whether `P` directives happen to
exist.

---

## Phase 1 — The engine

### Model

`ScenarioLine` gains a role, or a sibling type is introduced — implementer's
call, but the discriminator must be explicit in the wire, not inferred from the
account's type. A reader that guesses "assets: account ⇒ asset row" would
reclassify a legitimate one-off posting to `assets:cash`.

```rust
pub struct ScenarioAsset {
    pub id: String,
    pub account: AccountName,
    /// Overrides the journal's balance at the projection start. `None` = use it.
    pub opening: Option<Amount>,
    pub growth: Option<Growth>,       // the same stepwise Growth flows use
    /// Per-period addition. `None` (or zero) = the balance just compounds.
    pub contribution: Option<Amount>,
    pub period: PeriodSpec,
    pub note: String,
}
```

### Per-bucket arithmetic

For each asset, carry a running balance seeded from the journal's actual balance
for that account at the projection start (the same as-of machinery
`net_worth_priced` uses):

1. At each growth-step boundary, `balance *= (1 + rate)`, rounded to the
   amount's own precision at the step, exactly as flow growth is (plan 22
   Phase 1 step 3).
2. Then add the bucket's contribution occurrences.
3. `net_worth_delta += growth_amount` only. The contribution is already handled
   as a flow, below.

The contribution needs **no new cash logic**: it is a posting to an asset
account inside a rule, so the residual rule (amendment 18) already implies the
cash leg — cash −2000, asset +2000, net worth unchanged. Do not add a second
path for it; if you find yourself writing one, the residual rule is being
bypassed and the numbers will drift.

### Not double-counting the opening balance

**This is the trap.** `Projection.net_worth.opening` already includes every
asset account's real balance, because it comes from `net_worth` over the actual
journal. An asset row must therefore contribute only its **growth** and its
**contributions** to the series — never its opening balance, which is already
in there.

When the user overrides `opening`, the difference between the override and the
journal's real balance is applied once, at bucket 0, and labelled as an
adjustment in the warnings. A silent override would make the chart start at a
number the balance sheet disagrees with.

A test must assert that an asset row with a growth rate of zero and no
contribution leaves every net-worth bucket **byte-identical** to the same
scenario without the row at all. That is the double-count detector.

### Tests

- Stepwise compounding on a balance across a year boundary; the bump lands on
  the anniversary and not before.
- Contribution implies a cash outflow of the same size, and net worth is
  unchanged by it.
- Growth moves net worth and leaves net income and cash untouched.
- Growth applies before the period's contribution (Decision 7).
- The zero-growth no-contribution row changes nothing (the double-count guard).
- **A burning scenario with a growing asset still shows net worth falling when
  the burn exceeds the growth** — Decision 3, stated as a case rather than a
  hope.
- An asset row whose account resolves to a non-asset type produces a warning
  rather than silently modelling it.

---

## Phase 2 — The wire and the file

- `WireScenarioAsset` beside the existing line/event wire types, every optional
  field `null` rather than omitted (plan 22 amendment 17's rule).
- Serializer: write the contribution as the posting amount (`$0` when none),
  `growth:` and optional `opening:` as posting tags, bounds in the rule header.
- Reader: a posting carrying `growth:` on an asset-typed account is an asset
  row. A posting with no `growth:` stays a flow line, whatever its account — so
  existing scenario files keep their current meaning exactly.
- `fixtures/native/v1/` golden for a scenario containing an asset row.
- Extend the opt-in hledger cross-check (`LEDGELINE_HLEDGER_PROJECTION_CHECK`)
  to prove a written asset row round-trips through `hledger print` and reports
  the contribution — and only the contribution — under `balance --budget`.

---

## Phase 3 — The table

A third recurring section, **"Assets and balances"**, after Income and Expenses
and before One-off events. Columns: Account · Balance · Growth · Contribution ·
Per · From · To · row menu.

- **Balance** shows the journal's real figure, greyed, until the user types over
  it; an overridden value is visually distinct and clearable back to the
  journal's. The user must always be able to see what the ledger actually says.
- The section obeys whatever section-stability rule the UX fix establishes — a
  row must not move or lose focus while it is being edited. Read that plan
  amendment before touching the table.
- The **Net worth** report tab gains a breakdown of which assets contributed
  the growth, since with this feature the single line stops being
  self-explanatory.

`docs/projections.md` gains a section on asset rows: the `growth:`/`opening:`
tags, the start-of-period convention (Decision 7), that appreciation is
unrealised and never reaches cash or net income, and that the `$0` contribution
row is deliberate and readable by hledger.

## Sequencing

```
Phase 1 (engine) ──> Phase 2 (wire + file) ──> Phase 3 (table)
```

Sequential — Phase 2 serializes what Phase 1 models, and Phase 3 edits what
Phase 2 carries. **Do not start before the Projections-table UX fix has landed**;
Phase 3 edits the same component and would conflict with it throughout.

## Where the code is

| Path | Purpose |
|---|---|
| `crates/ledgeline-core/src/projections.rs` | `ScenarioAsset`, the running-balance walk, the growth-then-contribution order |
| `crates/ledgeline-core/src/projections/serialize.rs` | the `$0` posting, `growth:` / `opening:` tags |
| `crates/ledgeline-server/src/projections_api.rs` | `WireScenarioAsset` |
| `web/src/lib/projections/{types,scenarioModel}.ts` | the asset row and its section |
| `web/src/lib/projections/ui/ProjectionsTable.svelte` | the third section |
| `web/src/lib/projections/ui/ProjectionReports.svelte` | the net-worth contribution breakdown |
| `docs/projections.md` | asset rows, the tags, the conventions |

## Testing

| Level | Covers |
|---|---|
| `projections.rs` unit tests | compounding, the contribution's cash leg, the ordering convention, the double-count guard, burn-beats-growth |
| `projections/serialize.rs` tests | the `$0` posting round-trips; a tagless posting is still a flow line |
| hledger opt-in check | an asset row parses and reports only its contribution as a goal |
| `tests/projection_endpoints.rs` | the asset wire shape, and a non-asset account warning |
| `scenarioModel.test.ts` | section assignment and the balance override |
| `ProjectionsTable.svelte.test.ts` | the third section renders; an overridden balance is distinguishable and clearable |

## Definition of done

- `just engine-check`, `just engine-test`, `just check`, `just test`,
  `just lint`, `just e2e` green.
- `docs/projections.md` amended.
- A human has looked at the third section at 375px and desktop.
- Any contract in this doc that changed during implementation is amended here in
  the same commit, per `plans/00-overview.md` convention #9.

---

## Contract amendments made during implementation

**Phases 1 and 2 only.** Everything below was established while building the
engine, the wire and the file format; Phase 3 should trust these over the prose
above. Where something in `plans/22-projections.md` changed, it is amended there
instead and named here.

### 1. `ScenarioLine` gained a ROLE. There is no `ScenarioAsset`

§Model offers "a role, or a sibling type … implementer's call". The role won,
and the deciding argument is a file-format one the sketch could not have seen.

A `Vec<ScenarioAsset>` beside `Vec<ScenarioLine>` loses the **order of the
postings inside a `~` block**. The serializer renders one block per group and
compares what it would write against what the block already says (plan 22
amendment 28); with two collections it has to pick a canonical order — assets
first, or flows first — and any file that interleaved them would be silently
reordered on the next save of an unrelated rule. That is precisely the
byte-preservation discipline amendment 28 exists to keep.

So:

```rust
pub enum LineRole { Flow, Asset }

pub struct ScenarioLine {
    …
    pub role: LineRole,
    /// Asset rows only. `None` = use the journal's balance.
    pub opening: Option<Amount>,
}
```

The sketch's other fields map straight onto the ones already there: `account`,
`period`, `note`, and **`amount` IS the contribution**. The discriminator is
still explicit on the wire (`role: "flow" | "asset"`, always present outbound),
which is the constraint §Model actually locks.

**Consequently there is no `WireScenarioAsset`** (Phase 2's first bullet).
`WireScenarioLine` gained `role` and `opening` instead, both of which a client
must send back.

### 2. The contribution is a REQUIRED `Amount`, not an `Option`

The sketch has `contribution: Option<Amount>` and says "`None` (or zero) = the
balance just compounds". Zero won, because `None` cannot be written: the file
format states the contribution as the posting's amount, `$0` when there is none,
and a `None` carries no commodity to write that `$0` in. Guessing one from
elsewhere in the scenario is a valuation the serializer has no business making.

Zero is also what the row means, exactly: the wire and the model both carry
`amount` on every row, and `$0` round-trips through `hledger print` unchanged.

### 3. `opening:` carries a BARE number, and a mismatch is refused

§"The file format" writes `; opening: 200000`, and the bare spelling is
load-bearing rather than shorthand: hledger ends a tag's value at the next
comma, so `$200,000.00` would read back as `$200`. The commodity and the display
style therefore come from the row's own amount — which every asset row has, `$0`
or not.

Two consequences:

- **An `opening` in a different commodity from the row is a `400`**
  (`SerializeError::Invalid`). It cannot be written truthfully, and silently
  rewriting a balance's currency on a save is money.
- **An `opening` on a `role: "flow"` row is a `400`** too: a flow has no balance
  to open, and the engine would read it from nowhere.

An unreadable value is `None` — the journal's own balance — rather than a failed
file, the same way `parse_growth` degrades.

### 4. An asset row with no rate writes `; growth:` — the tag, empty

The reader rule this plan fixes is "a posting carrying `growth:` on an
asset-typed account is an asset row", so the marker is the tag's **presence**.
A row whose growth is `None` therefore still writes one, with an empty value.
hledger 1.52 reads that as the tag `growth` with the value `""` and `tag:growth`
matches it; this crate's `parse_tags` produces the same pair.

The alternative — writing `growth: 0%/yr` for a blank rate — would put a rate in
the file that the user never typed and come back filled in the Growth column
after a save. `Option<Growth>` round-trips exactly as it is.

### 5. Reading a file needs the ACCOUNT TYPES, so they are threaded in

"An asset-typed account" is not a name test, and a projection file usually
declares no `account` directives of its own — its accounts belong to the main
journal. So `scenario_from_text`, `ProjectionDoc` and `new_file` all take the
journal's `declared_types` map, and the file's own declarations are layered on
top of it (a self-contained scenario classifies with no journal open).

`ProjectionDoc` carries the map so that `confirm_round_trip` re-reads the written
text under the same classification it was written under. Reading the result any
other way would compare an asset row against the flow line it was not.

The rule-header `growth:` is deliberately NOT considered for the role, only the
posting's own: a rate meant for a whole block is a rate, but reclassifying every
asset posting in that block is a reclassification, and one wants to be written on
the row it applies to.

### 6. The arithmetic, stated exactly

Per asset row, over the projected span:

1. Seed a running balance from the journal's **subtree** balance for the account
   at `as_of`, valued into the same commodity the opening net worth is. An
   `opening` override replaces it, and the DIFFERENCE from the journal's figure
   is added to net worth once, in bucket 0, with a warning naming both figures.
2. Enumerate the row's **growth boundaries** (anchor + n units, clamped, capped
   at `MAX_GROWTH_STEPS`) and its **contribution occurrences**, and merge them in
   DATE ORDER.
3. At a boundary: `grown = round(balance × (1 + rate))`; add `grown − balance` to
   that bucket's net-worth-only accumulator; `balance = grown`.
4. At a contribution: place the amount as an ordinary group leg — so the residual
   rule implies its cash outflow exactly once — and add it to `balance`.
5. **A tie goes to GROWTH** (decision 7). This is the one thing the plan did not
   pin and the tests do: a contribution dated on a step's anniversary does not
   earn that step.

Decision 7's illustration ("a contribution made in month eleven does not earn a
full year's return") does not hold literally under this walk, and the mechanics
in §"Per-bucket arithmetic" are what was implemented: `balance *= (1 + rate)`,
**then** that period's contributions. A contribution made during a growth period
IS in the balance the next boundary compounds. Deferring it instead would mean
carrying two balances — the one that grows and the one the row actually holds —
and Phase 3's Balance column would show the wrong one.

**A zero rate is short-circuited to no steps at all.** Not an optimisation: the
step rounds, so walking a 0% row would round a valued balance to the row's
display precision and call the difference appreciation. A row that states no
growth must move nothing, which is what the double-count detector asserts.

### 7. Appreciation is not a posting, so it is not a group leg

`Layout` grew a per-bucket `net_worth_only` accumulator beside `per_bucket`.
Placing appreciation as a leg would sum it into the group's residual and imply a
cash leg of the same size — a paper gain putting money in the bank, which is the
error decision 2 exists to prevent. The same accumulator carries the `opening`
override's one-off adjustment, for the same reason.

### 8. The non-asset warning names the type it actually resolved to

"A warning rather than being silently modelled" is implemented as: warn,
contribute **nothing** — no growth and no contribution — and quote the resolved
type (`liability`, `expense`, or "of no type this journal declares or infers").
A row the engine will not model must not half-model itself.

### 9. The `fixtures/native/v1/` golden is a FILE READ, not the seed

Phase 2 asks for a golden "for a scenario containing an asset row". The seed
route builds its lines from the main journal's `~` rules, and
`fixtures/sample.journal` deliberately declares **none** — `budget.e2e.ts`'s "No
budget goals yet" empty state and `projections.e2e.ts`'s "every seeded row is an
average of the journal's own history" both rest on that, and adding a rule put
three asset accounts into `fixtures/native/v1/budget.json` as a side effect.

So the golden comes from the other route that serves a `WireScenario`:
`GET /api/projections/{*id}` over a committed
`fixtures/asset-growth-scenario.journal`. Reading ONE file by id is fully
replayable from a URI manifest — only the directory LISTING is not, which is what
plan 22 amendment 36 actually rules out. See plan 22 amendment 40 for what that
cost in `native_wire_golden.rs`.

The fixture is deliberately **not** named `projection-*.journal`, so it lands in
the picker's existing "Other journals" group and `projections.e2e.ts`'s optgroup
assertions are untouched.

### 10. Phase 2 reached `web/src/lib/api/` and two model files, and stopped there

`role` and `opening` are REQUIRED by `decodeScenarioLine` — no default — because
`nativeDecode.test.ts` renames every key of every golden and requires the decoder
to notice, and a `role` that defaulted to `"flow"` would be absorbed by that
sweep over the seed golden (where every row is a flow). Nothing was added to
`TOLERATED`.

That required, and was limited to:

| file | change |
|---|---|
| `web/src/lib/api/native.ts` | `WireScenarioLineIn` gained `role` and `opening` |
| `web/src/lib/api/nativeDecode.ts` | `RawScenarioLine` + `decodeScenarioLine` read both |
| `web/src/lib/api/nativeDecode.test.ts` | `projections-asset` added to the rename sweep |
| `web/src/lib/projections/types.ts` | `LineRole`, and the two fields on `ScenarioLine` |
| `web/src/lib/projections/scenarioModel.ts` | `scenarioToWire` sends both; `blankLine` is a flow |

`ProjectionsTable.svelte.test.ts`'s inlined wire literal gained the two keys
because the decoder now demands them. **No component changed**, and nothing in
`web/src/lib/projections/ui/` other than that fixture was touched — Phase 3 owns
the table, the third section and the net-worth breakdown, and none of it exists.

### 11. What Phase 3 gets, and what it still has to decide

Already in place:

- `ScenarioLine.role` / `.opening` decode, encode, and round-trip through a file.
- `blankLine` produces a flow; an asset row is something the table creates
  deliberately, never something a blank row becomes by having `assets:` typed
  into it.
- Every projection warning an asset row can produce is already on the wire and
  already rendered by the existing warnings list.

Still Phase 3's:

- The **"Assets and balances"** section, its columns, and the section-stability
  rule (plan 22 amendment 37) — `sectionOfLine` currently knows two sections and
  must learn a third that keys off `role`, not off the account.
- **The Balance column.** The engine does not put the journal's real balance for
  an account on the scenario wire — it is not part of the scenario. Phase 3
  needs a source for the greyed figure: either a new field on
  `WireScenarioLine` (computed per request), or the existing balance-sheet
  report read beside it. Deciding that is Phase 3's first job, and it is the
  only wire question left.
- The **net-worth contribution breakdown**, which likewise needs the engine to
  attribute growth per row — `Projection` carries only the total today.

---

**Phase 3 from here.**

### 12. ONE engine field answers both of Phase 3's open questions

Amendment 11 left two, and treated them as separate: a source for the Balance
column, and a per-row growth attribution for the breakdown. They are the same
question. The walk already computes both figures for every asset row it places —
`seed_asset_balance` produces the journal's balance, and the step loop produces
the appreciation — and neither survives past the per-bucket accumulator.

So `Projection` gained `assets: Vec<AssetRow>`: one entry per PLACED row, in
scenario order, carrying `group`, `account`, `journal_opening`, `opening` and
`growth`. `WireProjection` gained the same list.

**The Balance column therefore reads the RUN, not the scenario and not a second
report.** The alternative amendment 11 offers — "the existing balance-sheet
report read beside it" — was rejected for a reason that is not a preference: the
engine seeds an asset row's running balance from a SUBTREE balance valued into
the net-worth series' own commodity, at the engine's `as_of`. A client reading
`/api/reports/balancesheet` would have to reproduce the subtree roll-up, the
valuation and the as-of date, and any of the three drifting would put a figure
under the user's cursor that the curve below it disagrees with. The column exists
to tell the user what the projection used; the projection is the only thing that
knows.

Three consequences worth stating:

- **Both balances travel, never one.** `journal_opening` is what the ledger says
  and `opening` is what the row compounded. They are equal on every row without
  an override, which is exactly why the decoder DEMANDS both: a `journalOpening`
  that defaulted to `opening` would be indistinguishable from a correct one until
  the first user typed over a balance, and would then show the override as the
  ledger's own figure — the one failure this column exists to prevent.
- **A row the engine refused to model gets NO entry.** It contributed nothing
  (amendment 8), and a balance reported beside it would say it had been
  modelled. The table shows a dash; the warning already says why.
- **Keyed by `group`, not `id`.** `id` is the logical row and two segments share
  one; `group` is the source rule and is what `place_asset` runs per.

`fixtures/native/v1/` is untouched: `POST /api/projections/run` has no golden and
cannot have one (it carries its scenario in a body), which `requests.tsv` already
says. The route is pinned by `projection_endpoints.rs`, which gained the
assertions, and by `nativeDecode.test.ts`'s inline `PROJECTION_RUN` literal,
which gained a two-row `assets` array — the second row OVERRIDDEN, so the rename
sweep reaches the field the column depends on. Nothing was added to `TOLERATED`.

### 13. The Balance column reads the LAST GOOD projection, and checks the account

`projection.value`, not `current`. Everywhere else `routes/projections/+page.svelte`
trusts only a projection that answers the scenario on screen (FE-1), because a
held payload makes a claim about a scenario that has since changed. This figure
does not: it is a fact about the JOURNAL as of today, and the journal does not
change between two keystrokes. Keying it to `current` would blank the column on
every letter typed anywhere in the table.

What can go stale is which ACCOUNT a row names, and that is handled where it
actually is — `journalBalanceFor` refuses an entry whose `account` is not the
row's current one. A stale figure under a freshly-typed account is a claim about
the ledger that the ledger is not making, and the recompute that corrects it is a
debounce away.

Null from that lookup is "no answer", never zero, and the column renders it as a
dash. Three cases reach it: nothing projected yet, a row the engine declined, and
the stale-account case.

### 14. `LineSection` gained `asset`, and `role` beats BOTH other tests

`resolvedSection` now tests `role === "asset"` FIRST — ahead of the period and
ahead of the account type. Two consequences, one of them a decision the plan did
not pin:

- **An asset row with a single-date period is an ASSET row, not a one-off.** The
  plan is silent and `oneoff` would have claimed it. A balance with a dated
  contribution is still a balance, and the one-off table has no Growth column to
  show its rate in.
- **`HeldSection` did NOT grow.** Plan 22 amendment 37's hold exists because a
  flow row's section is re-derived from text that changes under the user's
  fingers. An asset row's section is `role`, which no box on the row edits, so
  the property the hold provides this section has for free. `sectionOfLine`
  returns `asset` ahead of any hold, `holdSection` declines to take one, and the
  Assets table carries no `onfocusin`/`onfocusout` at all.

The keystroke-by-keystroke regression test was written anyway, in the same shape
as the Income one and typing a route (`income:consulting`) that would move a flow
row twice. It passes for a stronger reason than the original does, and that is
worth having pinned.

### 15. An asset row has no "Add a step", and the docs say why

`splitStep` produces two bounded segments sharing an `id`. For a FLOW that is the
feature. For an asset row it is a double count: `place_asset` seeds each
`ScenarioLine` from the journal independently, and `growth_boundaries` walks the
PROJECTION's span rather than the segment's, so both halves compound the whole
balance over the whole window.

Phase 3 does not expose it — the Assets row menu offers Duplicate and Delete
only — and `docs/projections.md` states the general rule ("one row per account"),
because a hand-written file can still contain two rows naming one account and the
engine does not warn about it. **Making the engine warn is deliberately NOT done
here**: it is an engine change with no UI path to it, and the plan's Phase 3 is
the table. It is the obvious next thing if a real file ever hits it.

### 16. A fourth section broke `rowNames`' uniqueness guarantee

`rowNames` makes a row's accessible name unique WITHIN its section, and that was
enough while the only two sections were Income and Expenses: an account cannot be
both revenue and not-revenue, so the same name could not appear in both.

An asset row can name an account a flow row also names, and legitimately — a
monthly transfer into `assets:checking` beside an `assets:checking` row earning
interest is an ordinary thing to write. That would have put two boxes labelled
"Growth rate for assets:checking" on one page, which a screen reader cannot
distinguish and `getByLabelText` refuses outright.

So the Assets table qualifies the labels a flow row also has — "Growth rate for
**the** `assets:checking` **balance**", the row menu, both date bounds, the
segment delete — and leaves Balance and Contribution unqualified, because those
two words appear in no other table and "Balance for the … balance" reads as
nonsense. The "Per" column is "Contribution period", which is both unique and
more accurate than "Period" was: it is the contribution's cadence, not the
growth's.

### 17. What Phase 3 shipped, file by file

| file | change |
|---|---|
| `crates/ledgeline-core/src/projections.rs` | `AssetRow`; `Layout.assets`; `seed_asset_balance` returns both balances; three unit tests |
| `crates/ledgeline-server/src/projections_api.rs` | `WireAssetRow`; `WireProjection.assets` |
| `crates/ledgeline-server/tests/projection_endpoints.rs` | the attribution's wire shape, and both balances under an override |
| `web/src/lib/api/nativeDecode.ts` | `decodeAssetRow`; `assets` required on `decodeProjection` |
| `web/src/lib/projections/types.ts` | `AssetRow`, `AddSection`, `LineSection` gained `asset` |
| `web/src/lib/projections/scenarioModel.ts` | `resolvedSection`/`sectionOfLine` role-first; `blankAssetLine`; `sectionPrefix` takes `asset`; `cloneScenario` deep-clones `opening` |
| `web/src/lib/projections/projectionView.ts` | `assetRowsByGroup`, `journalBalanceFor`, `assetContributions` |
| `web/src/lib/projections/ui/ProjectionsTable.svelte` | the third section, the Balance column, the override and its clear |
| `web/src/lib/projections/ui/ProjectionReports.svelte` | the growth breakdown; the stale "held flat" caption corrected |
| `web/src/routes/projections/+page.svelte` | `assetBalances` and `styles` into the table |
| `docs/projections.md` | the Assets table, Decision 7 stated plainly, one row per account |
| `web/e2e/projections.e2e.ts` | two specs, **UNVERIFIED** — Playwright could not run |

`just e2e` was NOT run (the environment has no Playwright driver), and the two
specs added are marked unverified in the file itself. Everything else in
§"Definition of done" is green: `just engine-check`, `just engine-test`, and the
web gates run directly (`svelte-check`, `tsc --noEmit`, `vitest run`, `eslint`,
`prettier`) because `bun`, which the `just` recipes wrap, is sandbox-blocked.
**A human has not yet looked at the section at 375px and desktop** — that is the
one line of the definition of done this commit cannot close by itself.
