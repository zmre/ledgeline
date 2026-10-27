# 29 — Projection cache: an edit costs the forecast, not the history

Every edit in the Projections what-if table re-runs the whole projection, and
the run walks the whole REAL journal several times although an edit changes
only the forecast. This plan splits every reading into "seed from the real
side" plus "accumulate the forecast", computes the real side once per journal
snapshot and key, and keeps it in a small server-side cache. Answers stay
**identical** to the full scan, down to each decimal's scale.

## The ask (Patrick, 2026-10-26)

> Option 2: cache everything derived from REAL transactions, keyed on what it
> depends on, so an edit only processes the forecast.

Option 1 (merely stop cloning the journal in `merged_transactions`) becomes
moot: the cached path never builds the merged list.

## The problem, measured

The table debounces 250 ms and then POSTs the whole scenario to
`/api/projections/run` (`crates/ledgeline-server/src/projections_api.rs`). Per
run:

1. `project()` (`projections.rs`) expands the forecast, and on the way makes
   full passes over the real history: `latest_real_date`,
   `infer_market_prices` (inside `real_prices`, twice), `account_balances_at`
   (twice), `CounterDefaults` (the trailing-year pairing table),
   `net_worth_priced` (twice: opening and lookback), `closing_cash`, and the
   journal's highest `Tindex`.
2. `merged_transactions` deep-clones every real transaction before the cutoff,
   then appends the forecast.
3. `balance_sheet_columns` (one pass), `income_statement_columns` (one
   `account_totals` pass PER COLUMN, 26 at the default), `cash_flow_statement`
   (one pass plus an `investments` pass) and, on the Accounts tab,
   `account_periods` (two passes) each scan the merged list again.
   `CfTags::from_journal` builds a price table from every `P` directive.

Bench (`projections` group, 24 monthly buckets, 4 rules): clone 63 ms / merge
72 ms / `project()` 343 ms at 200k transactions; 15 / 18 / 65 ms at 50k. The
statements come on top (the balance sheet columns alone ~83 ms at 200k).
Everything scales with the history; nothing an edit changes does.

## Decisions

1. **Identical, not close.** Every cached reading equals the full-scan
   reading over `real ++ forecast`, compared by `Debug` (which prints each
   `Dec`'s mantissa AND scale), not by `==` (which compares `Dec` by value).
   The full-scan functions stay, unchanged in behaviour, as the reference
   implementation and the test oracle.
2. **Core stays pure; the server owns the cache.** Core gains plain values
   (`JournalFacts`, `RealKey`, `RealSide`) and pure functions over them. No
   clock, no lock, no global, nothing `cfg`-gated, so `ledgeline-ffi` and a
   future mobile UI get the same speed by holding the values themselves. The
   server holds them per snapshot.
3. **The key is what the real side depends on, and nothing else.**
   `(cutoff, interval, lookback, value_in)` within one journal snapshot.
   `today` enters only through the cutoff; `count`, `depth`, valuation basis,
   the accounts mode and the empty toggle are applied per run.
4. **Lives in the snapshot.** The cache is a field of the server's `Snapshot`,
   so the journal's identity IS the snapshot: an edit, a file-watcher reload,
   a `refresh()` or File → Open publishes a new snapshot with an empty cache,
   and the old cache is freed with the last request holding the old snapshot.
   No revision counter to get wrong.
5. **Bounded by entries.** Four `RealSide`s per snapshot, least recently used
   evicted. One is the steady state (the user edits; the key does not move);
   the others cover flipping interval or lookback and back.
6. **Wire unchanged.** The SPA needs no change. Same JSON, byte for byte.

## Design

### What the real side is

Two values, by lifetime.

**`JournalFacts`** — per journal, independent of any request:

| field | what | who read it per run before |
|---|---|---|
| `latest_real` | latest non-generated transaction date | `expand` (cutoff) |
| `first_index` | highest `Tindex` + 1 | `expand` (generated `Tindex`) |
| `inferred` | `infer_market_prices(all txns)` | `real_prices`, ×2 |
| `known` | declared accounts ∪ every account any real txn posts | `known_accounts` |
| `shape` | `StatementShape` (declared types, `bsgroup:`, `bsterm:`, `issection:`, `isgroup:`) | every handler |
| `cf_tags` | `CfTags::from_journal` | the cash flow statement |

`StatementShape` moves from `reports_api.rs` into `ledgeline_core::reports`
unchanged, so core can build it and the Reports handlers keep calling the same
constructor.

**`RealKey`** = `{cutoff, interval, lookback, value_in}`, with
`JournalFacts::key(today, interval, lookback, value_in)` computing the cutoff
exactly as `ForecastWindow::resolve` does.

**`RealSide`** — everything a run reads from the real journal, for one key:

- *Window*: the cutoff, `as_of` (the day before), the lookback keys and spans,
  `from`.
- *Prices*: `real_prices` (inferred + explicit, dated before the cutoff, in
  `net_worth`'s order), their `PriceDb`, the valuation `target`; the real
  EXPLICIT directives before the cutoff and the set of commodities they price
  (`merged_prices`' two inputs).
- *Expansion inputs*: `balances` (every account's balance at `as_of`, by
  posting date, unpruned — `account_balances_at`), and `CounterDefaults`
  built with NO header. The scenario's `; cash:` header is the only part of
  `CounterDefaults` that depends on the scenario, and it is a stored field, so
  each run sets it (`with_header`) on a clone.
- *`project()` openings*: raw and valued opening cash, opening net worth, the
  lookback cash and net-worth histories. Computed by the same calls over the
  same full transaction slice as before.
- *Statement seeds*, from the transactions `merged_transactions` keeps (dated
  before the cutoff), split by each POSTING's effective date:
  - **early** postings (before the cutoff) are folded into seeds;
  - **late** postings (a real transaction before the cutoff with a posting
    dated on or after it, rare) are kept as `late` transactions carrying only
    those postings, plus the whole transaction's closing-entry flag for the
    cash flow statement. They are replayed per run beside the forecast. This
    is what makes the split exact rather than "exact unless posting dates".

  Because the cutoff and `from` are period starts, every lookback column ends
  before the cutoff and every forecast column starts on or after it. So early
  postings land only in lookback columns or before `from`, and the forecast
  plus `late` land only in forecast columns or after `end`. Each seed is
  therefore the lookback columns WHOLE, plus a running state at the cutoff:

  | reading | seed |
  |---|---|
  | balance sheet | the cumulative `{written, at_cost, precision}` snapshot at each lookback end, and the running totals after every early posting |
  | income statement | each lookback window's direct totals as written and at cost (forecast windows hold no early posting) |
  | cash flow | the per-account accumulators over the lookback columns (net income, non-cash lines, cash opening and deltas, exchange) and the `investments` set |
  | accounts | each lookback column's direct totals, and the balances before `from` for Balance mode |

  Nothing in a seed is valued, depth-clamped, grouped or sectioned: that is
  per-account work, done per run over seed + tail, so valuation basis and
  depth stay out of the key and every rule (valuation per column end with
  the grown prices, the valuation adjustment, retained earnings, the
  sub-cent `WrittenPrecision` verdict, depth roll-ups, `cfsection:`/
  `bsterm:`/`bsgroup:` resolution, the cash flow statement's closing-entry and
  virtual-posting rules) is executed by the SAME code as in the full scan.

### Every builder becomes seed + tail

Each full-scan function is refactored into an accumulator plus a finisher, and
the full scan is "empty seed, accumulate everything, finish". The cached path
is "cached seed, accumulate the tail, finish". One implementation, two entry
points:

- **Balance sheet:** `balance_sheet_columns` already carries `RunningTotals`
  across columns. The per-column `SheetBuilder::report` is unchanged.
- **Income statement:** `statement()` takes per-window direct totals instead
  of computing them; `window_totals` becomes "direct totals" + "narrow and
  value", both shared.
- **Cash flow:** `cash_flow_statement` becomes `CfAccumulator::add(txn,
  closing)` + `finish(tags, depth)`. During accumulation an account needs only
  its CLASS (cash / net income / line), which `investments` cannot change; the
  line's section (operating vs investing) is resolved at `finish`, once the
  `investments` set includes the forecast, exactly as the full scan resolves it
  with the set built over the merged list.
- **Accounts:** `account_periods` takes direct per-column totals and the
  before-`from` balances.
- **`project()`:** `expand` and the series roll-forward take a `RealInputs`
  (window, prices, target, balances, defaults, first index). The reference
  `project()` builds it by scanning, as today; `project_cached` reads it off
  the `RealSide`. The roll-forward over the generated transactions is shared.

Accumulation order never matters: every sum is `MixedAmount::accumulate` into
a `BTreeMap` (commodity-keyed, no intermediate pruning) and `Dec::add` is exact
with result scale `max(places)`, so regrouping addends cannot move a value or
a scale — the argument `net_worth_priced` and `balance_sheet_columns` already
rely on. Every place that DOES prune mid-stream (`ma_add_assign`) is kept in
the same column order.

### Core API

```rust
// ledgeline_core::projections
pub struct JournalFacts { .. }
impl JournalFacts {
    pub fn new(journal: &Journal) -> Result<Self, ReportError>;
    pub fn key(&self, today: &str, interval: Interval, lookback: usize,
               value_in: Option<Commodity>) -> Result<RealKey, ReportError>;
}
pub struct RealKey { pub cutoff: String, pub interval: Interval,
                     pub lookback: usize, pub value_in: Option<Commodity> }
pub struct RealSide { .. }      // Send + Sync, owned, no borrows
impl RealSide {
    pub fn build(journal: &Journal, facts: &JournalFacts, key: &RealKey)
        -> Result<Self, ReportError>;
}
pub fn project_cached(scenario: &Scenario, real: &RealSide, facts: &JournalFacts,
                      count: usize, depth: usize) -> Result<Projection, ReportError>;
pub fn projected_statements(real: &RealSide, facts: &JournalFacts, forecast: &Forecast,
                            opts: &StatementOpts) -> Result<ProjectedStatements, ReportError>;
pub fn projected_accounts(real: &RealSide, facts: &JournalFacts, forecast: &Forecast,
                          opts: &AccountsOpts) -> Result<PeriodReport, ReportError>;
// The reference (full scan over merged_transactions/merged_prices), kept for
// tests, benches and as the definition of "correct":
pub fn reference_statements(journal: &Journal, facts: &JournalFacts, forecast: &Forecast,
                            opts: &StatementOpts) -> Result<ProjectedStatements, ReportError>;
pub fn reference_accounts(..) -> Result<PeriodReport, ReportError>;
```

`RealSide` holds no `AccountTypes` (its memo is a `RefCell`, so it is not
`Sync`); it holds the declared map and builds the memo per run.

### The server's cache

`Snapshot` gains `projections: Arc<ProjectionCache>`:

```rust
pub(crate) struct ProjectionCache {
    facts: OnceLock<Result<Arc<JournalFacts>, ReportError>>,
    sides: Mutex<Vec<(RealKey, Arc<RealSide>)>>,   // most recently used last
}
```

- **Concurrency.** Handlers run on `compute` threads concurrently. `facts` is a
  `OnceLock` (one build; racers share it). `sides` is a short-held
  `std::sync::Mutex`: look up, release, build outside the lock on a miss,
  re-check and insert. Two simultaneous misses on one key both build and the
  second insert is dropped — the same trade `ViewCache::diagnostics` makes, and
  correct because both values are identical. A poisoned lock is recovered
  (`PoisonError::into_inner`), as everywhere in the server. `papaya` was
  considered and rejected: the map holds at most four entries, the LRU order
  needs a sequence, and no other code in the server uses it.
- **Memory.** At most 4 `RealSide`s per live snapshot, plus one
  `JournalFacts`. The large parts are the real price list (inferred + `P`) and
  the counter-pairing table (one record per account and commodity per
  transaction in the trailing year); the seeds are per-account. Measured
  sizes are in §Results.
- **Eviction.** LRU at 4; the whole cache dies with its snapshot.
- **Errors** are not cached (a `ReportError` here is a decimal overflow, and
  the request reports it).

### The run handler, after

`run`: decode the scenario → `facts = snapshot.projections.facts(journal)` →
`key = facts.key(window.end, interval, lookback, value_in)` →
`real = snapshot.projections.side(journal, &facts, &key)` →
`project_cached` → `projected_statements` (+ `projected_accounts` when asked)
→ the same wire structs. `merged_transactions` is no longer called on the
serving path; it stays public for the reference and the parity suite.

## Phases

1. **Plan** (this file) and the `run_cold` / `run_warm` benches over the
   current code, with a `--save-baseline before`.
2. **Core refactor, no behaviour change:** the accumulator/finisher split of
   each builder; `RealInputs` for `expand`/`project`. `just engine-test` and
   the hledger parity suite unchanged.
3. **Core cache values:** `JournalFacts`, `RealKey`, `RealSide`,
   `project_cached`, `projected_statements`, `projected_accounts`, the
   references; `StatementShape` into core.
4. **Equivalence suite** (below).
5. **Server:** `ProjectionCache` in `Snapshot`, the run handler on the cached
   path, cache-behaviour tests.
6. **Bench after**, numbers into this plan, docs.

## Tests

**Equivalence** (`crates/ledgeline-core/tests/projection_cache.rs`): for every
case, `project_cached` vs `project` (the whole `Projection`), and
`projected_statements` / `projected_accounts` vs the references, compared by
`Debug`:

- every scenario under `fixtures/projections-parity/` over its `main.journal`,
  `fixtures/asset-growth-scenario.journal`, the
  `fixtures/projections-expense-as-asset/` pair, and `fixtures/sample.journal`
  with its own `~` rules as the scenario;
- intervals daily, weekly, monthly, quarterly, yearly; lookback 0, 2 and 7;
  depth 1, 2 and unclamped; valuation market, cost and none;
- multi-commodity with grown prices, legacy (counter-less) rules, gains
  postings, events, accounts Change / Balance / empty;
- a journal with a late posting (posting date after the cutoff on a
  transaction before it), a real transaction after the cutoff, a closing
  entry, a virtual posting and a costed cash posting;
- a proptest over random scenarios (flows with and without counters, growth,
  asset rows, events, mixed commodities) on a generated journal, with random
  today, interval, lookback and count;
- one `RealSide` reused across several scenarios, which is what the server
  does.

**Cache behaviour** (server unit tests on `ProjectionCache`, plus HTTP tests):
a repeated run hits; changing interval, lookback, `valueIn`, or a `today` in
the next period misses; a `today` within the same period hits; four keys
evict the least recently used; a journal reload, an edit adding a real
transaction and an edit to an `account` declaration's tags each publish a
snapshot whose run reflects the change.

**Unchanged:** `just engine-test`, the hledger parity suite
(`LEDGELINE_HLEDGER_PROJECTION_CHECK=1 cargo test -p ledgeline-core --test
projection_files`), `projection_endpoints.rs`, the live integration tests and
the e2e specs.

## Benchmarks

`projections` group, the corpus at 5k / 50k / 200k
(`LEDGELINE_BENCH_SIZES=5000,50000,200000`), the server-equivalent run:
`project` + balance sheet columns + income statement columns + cash flow +
accounts (Change):

- `run_cold`: everything a first request pays (`JournalFacts::new`,
  `RealSide::build`, then the run).
- `run_warm`: the run against an already-built `RealSide` — every edit after
  the first.

Before this plan both ids measure the full-scan run (there is no cache), so
`--baseline before` compares like for like.

**Targets:** warm, single-digit ms at 50k and ~10 ms at 200k, flat in history
length; cold no slower than today.

## Definition of done

- The run endpoint serves from the cache; no per-run clone of the journal.
- The equivalence suite and the cache tests pass; every gate passes:
  `just engine-test`, `just engine-check`, the hledger parity suite,
  `just test`, `just check`, `just lint`, `just test-integration`, `just e2e`.
- The numbers table below is filled in.

## Open questions

None at the start. Product questions that come up are recorded here with the
conservative choice.

## Results

To be filled in after Phase 6.
