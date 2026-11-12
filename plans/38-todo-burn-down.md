# 38 — TODO burn-down: insights speed, price graph, settings files

This plan clears the Known issues, Security and Performance sections of
`TODO.md`. They were triaged with Patrick on 2026-11-11/12. Each item was
checked against the code, then either kept and scheduled here or deleted from
`TODO.md`.

Each work package removes its own `TODO.md` entry in the commit that finishes
it.

## Triage outcome

| Item                                                  | Outcome                     |
|-------------------------------------------------------|-----------------------------|
| Insights: 8 `PriceDb::build`s, 5 holdings replays     | WP-1                        |
| `value_at` rebuilds `PriceGraph` per call             | WP-2                        |
| Dense many-to-many FX bench corpus                    | WP-2 (measure first)        |
| `reachable` O(V·E), `shortest_path` clones            | WP-2                        |
| `latest_directive_price` early exit                   | WP-2                        |
| `load_rss` takes no `Shape`                           | WP-2                        |
| Insights refetch shows stale numbers silently         | WP-3                        |
| Import preview diffs against the capped alias list    | WP-4                        |
| ~14 private "no ASCII control char" checks            | WP-4                        |
| Desktop app loses localStorage settings on relaunch   | WP-5 (redesigned, see below)|
| Pie chart oval on Linux                               | Kept in TODO; needs a WebKitGTK repro |
| CSP `src=` attribute match                            | Deleted                     |
| hledger `-f` pin check after each run                 | Deleted                     |
| `choose_base` → `coverage` single pass                | Deleted                     |
| Progressive Insights (split `/api/insights`)          | Deleted                     |
| Problems drawer virtualization                        | Deleted                     |
| "Never examined" (XLSX, 16 MiB import, concurrency)   | Deleted                     |
| Matching; Compare projections                         | Deleted as done (plans 35, 34) |

## Ordering

WP-1 and WP-2 both touch `reports/prices.rs` and `holdings/engine.rs`, so they
run one after the other: WP-1 first, then WP-2. WP-3, WP-4 and WP-5 don't
overlap with each other or with WP-1/WP-2, so they can run in parallel
worktrees.

---

## WP-1 — `/api/insights` under its 400 ms gate

**Today** (current line numbers): `insights` (`reports/insights.rs:993`)
builds `PriceDb` 8 times and replays the journal through holdings 5 times:

- `window_holdings` runs twice (`:1094`, `:1095`). Each run calls
  `compute_holdings`, which replays the journal and then replays it again
  through `report_at` for `gain_since` (`engine.rs:1853`, `:1876`).
- `movers` → `portfolio_at` (`:769`) runs a third `compute_holdings`, using
  explicit prices only.
- Six of the eight `PriceDb::build`s use the same combined price set. Each one
  deep-clones every directive (`prices.rs:43`).

**Fix:**

1. Build the combined `PriceDb` once per request, and the explicit-only one
   once. Thread `&PriceDb` into `net_worth_priced`, `compute_holdings` and
   `top_transactions` through internal entry points. The public signatures
   stay as they are.
2. `replay_pools` already takes several dates (`engine.rs:741`). Replay once
   over `[start, mid, end]` and serve both windows' current and `gain_since`
   snapshots from that one pass. `movers` keeps its explicit-only price
   database but shares the replay if the pools don't depend on prices. If
   they do, it keeps its own replay; check this before relying on it.

**Done when:**
- [x] The insights JSON for every fixture is byte-identical before and after. Take
  a snapshot of each fixture first.
- [x] The `insights` bench at 200k is ≤ 400 ms (baseline 968 ms).
- [x] `docs/development/perf-baseline.md` is updated.

**Status (2026-11-12): done.** The bench at 200k went from 916 ms to
**293 ms** over two commits. The pre-change baseline on this run was 916 ms,
not the 968 ms quoted above.

1. `perf(insights): share price DB and holdings replays` (916 → 518 ms):
   - Two `PriceDb` builds per request, down from eight.
   - One `FlowWindow::Chained` replay serves both windows, replacing four.
     This is `holdings_window_pair` in `holdings/engine.rs`.
   - The movers probe reuses the windows' `HoldingsInputs` via `repriced`. It
     keeps its own replay because the pools read the price DB.
2. The follow-up commit (518 → 293 ms):
   - The replay, the facts and `stock_accounts` walk only the stock-relevant
     transactions (`stock_order`), built once per request and shared.
   - One `SoleFacts` scan serves the window pair and movers.
   - `is_iso_code` has a length guard.
   - `add_posting` no longer allocates per posting.

Identity was checked on 654 captures after each commit: every fixture journal
at six spans or options, plus the 5k/50k/200k bench corpora. All matched the
pre-change output. `tests/insights_golden.rs` pins this output; its snapshots
were blessed on the pre-change code. `perf-baseline.md` §3 has the remaining
breakdown, and the `TODO.md` entry is gone.

## WP-2 — price graph at scale

1. **Measure first.**
   - Add a `V2_DENSE_FX` `Shape` to `benches/corpus.rs`: about 8 currencies
     that all quote each other, plus symbols priced in non-base currencies,
     so that `shortest_path` and graph misses actually run.
   - Give `examples/load_rss` a shape argument through `ensure_journal_for`.
   - Record baselines for the insights bench, the holdings series and peak
     RSS on v2, v2fx and the dense shape.
2. **`reachable`:** build an adjacency index once in `graph_at`, so
   `reachable` is O(V+E) and its doc comment becomes true.
3. **`shortest_path`:** stop cloning a `path`/`visited`/`unused` Vec for every
   partial path, and stop `PriceGraph::rate` from running the search twice.
   hledger's tie-breaking must stay exactly as `tests/reports_prices.rs` pins
   it.
4. **Hoist the graph.** `value_at` builds its graph lazily for each call. Add a
   per-`PriceDb` memo keyed by `as_of`, or a caller-owned graph parameter at
   the hot sites: `flows.rs:590` (per transaction), `net_worth.rs:66`,
   `holdings/other.rs:360` and `insights.rs:898`. There are about 10 call
   sites in total. A memo keeps the signatures but needs interior mutability.
   Pick whichever makes the diff smaller.
5. **`latest_directive_price`:** in `PriceDb::build`, record for each
   commodity which targets it is ever priced in. The reverse walk
   (`engine.rs:1084`) can then exit at once for a symbol that is never priced
   in the base.

**Done when:**
- [x] All existing price and report tests pass unchanged.
- [x] The dense-shape numbers improve and v2/v2fx don't regress.
- [x] perf-baseline records the dense shape and the RSS numbers.

**Status (2026-11-12): done.** The work is in four commits:

1. `test(bench): dense many-to-many FX corpus, flows bench, load_rss shapes`:
   - Adds `V2_DENSE_FX` (`v2dfx`), which passes `hledger check -s`.
   - Adds a `reports/income_statement_flows` bench.
   - `load_rss` and `gen_journal` now take a shape name.
2. `perf(prices): index the price graph and search it breadth-first`. This
   covers steps 2 and 3. The BFS was checked against the old enumeration on
   3,000 random graphs.
3. `perf(prices): memoize the price graph per date on PriceDb` (step 4). It
   memoizes graphs and their resolved rates, and no signature changed.
4. `perf(prices): index each commodity's price targets` (step 5). It covers
   both `lookup_in` and `latest_directive_price`.

On v2dfx:

| Bench | before | after |
|---|---|---|
| `income_statement_flows` | 2.43 s | 36 ms |
| `balance_sheet_columns/one_pass_30` | 4.79 s | 82 ms |
| `net_worth/count_60` | 4.30 s | 126 ms |
| `insights` | 793 ms | 480 ms |
| `holdings_series_60` | 479 ms | 141 ms |

On v2fx, `balance_sheet_columns` went from 193 ms to 79 ms and
`holdings_series_60` from 145 ms to 120 ms.

v2 is within noise. Interleaved A/B medians are +1% to +4% against a ±5%
spread, and v2 never builds a graph.

Identity was checked on 127 pretty-`Debug` captures: every parseable fixture
plus v1-5k, v2, v2fx and v2dfx, across 13 reports. All are byte-identical
after each commit. `perf-baseline.md` §7.6 has the tables and peak RSS, and
the five `TODO.md` entries are gone.

## WP-3 — Insights shows when it is refreshing

While a refetch is in flight with an older payload still loaded:
- `InsightsDashboard` dims the boxes slightly and shows a small spinner next to
  `PeriodControl`.
- The old numbers stay on screen; nothing blanks out.
- The startup-metrics `sameSpan` guard stays as it is.

Expose the "loading with stale data" state from `createResource`, next to
`view`; don't change `dataView`, which `loadState.test.ts:17` pins. Add a
component test, and an e2e assertion if the period-control spec makes that
cheap.

## WP-4 — import parity and one control-character helper

1. **Alias parity.**
   - `ConfInForce` keeps the uncapped alias list as well as the forwarded
     (capped) one, and `cli_parity` diffs `conf_additions` against the uncapped
     list.
   - Make the preview diff against the same file the write route targets
     (`resolve_conf`, in the journal's own directory) rather than the config in
     force, which may be `../hledger.conf` or `~/.hledger.conf`.
   - Add a test with more than 200 aliases, and one where the parent directory
     has a config file.
2. **Control-character helper.** Add one helper in core, e.g.
   `text::first_ascii_control(s, allowed: &[char]) -> Option<char>`. Replace
   the copies at:
   - `accounts.rs:461`, `periodic.rs:1390`, `aliases.rs:253`
   - `rules.rs:3731`, `edit_api.rs:1201`
   - `serialize.rs:3037`/`:3090`, `parse.rs:1374`
   - `import_api.rs:1098`/`:1157`/`:5845`, `projections_api.rs:2492`,
     `stage.rs:493`, `rules_api.rs:1146`

   Each caller keeps its own error type. Leave the Unicode `is_control()`
   checks alone (`serialize.rs:799`/`:955`/`:2827`, `auto_form.rs:275`); they
   are deliberately different.

**Status (2026-11-12): done.** Two commits:

1. `fix(import): preview the hledger.conf lines the write will add`. The
   preview and the write now both call `ConfTarget::missing`, which diffs
   against every alias in the journal's own `hledger.conf`, the file the write
   targets. So `ConfInForce` needs no uncapped copy, and the config in force
   (capped, possibly `../hledger.conf`) is used only to measure the
   divergence. Two new hledger-backed tests in `import_endpoints.rs` cover a
   line past the 200-alias cap and a parent-directory config. Both failed
   against the old preview.
2. `refactor: one ASCII control-character helper`. This adds
   `ledgeline_core::text::{first_ascii_control, has_ascii_control}`. Every
   listed site used `char::is_ascii_control`, DEL included, and only
   `edit_api::check_text` allows any characters, so behaviour is unchanged.
   `edit_api::check_text` stays as the `AppError` wrapper. Not merged:
   `projections/discovery.rs` `clip`, which strips controls from a display
   value rather than refusing them.

## WP-5 — settings files move to TOML, with per-ledger UI prefs

### The ask

The desktop app loses every localStorage setting on relaunch, because the
port, and with it the WebKit origin, changes on each launch. Patrick
(2026-11-12) wants this solved more broadly:

- UI preferences live in files and can differ per ledger. Later this will
  allow a different theme per entity, so you can tell at a glance which one
  you're in.
- There is a global file, and each ledger can override it.
- Automatic saves always go to the ledger's own file.
- Settings files are TOML, not JSON, so people can edit them by hand. Recents
  stay JSON.

### Files

| File                                 | Location                      | Written by          | Holds                                    |
|--------------------------------------|-------------------------------|---------------------|------------------------------------------|
| `prefs.toml`                         | config dir (`AppDirs`)        | Settings form (`PUT /api/prefs`) | `hledgerPath` (was `prefs.json`) |
| `ui-prefs.toml`                      | config dir                    | **Hand-edit only**  | Global UI defaults                       |
| `ledgeline.toml`                     | the main journal's directory  | App, automatically  | Per-ledger UI overrides                  |
| `recents.json`                       | config dir                    | App                 | Unchanged                                |

### Decisions

- **Merge order.** For each key, the effective UI prefs are: the
  `ledgeline.toml` value if it has one, else the `ui-prefs.toml` value, else
  the built-in default.
- **Per-ledger file layout.** UI keys sit in a `[ui]` table in
  `ledgeline.toml`. That leaves room for later per-ledger settings, such as a
  theme, without a second file. `ui-prefs.toml` uses the same keys at top
  level.
- **Only changed keys are written.** An automatic save PATCHes just the keys
  that changed. `ledgeline.toml` therefore holds only what the user actually
  changed in that ledger, and defaults for untouched keys still come from the
  global file.
- **Comments survive.** Writes go through `toml_edit`, so comments,
  key order and unknown keys a person added are kept. This also removes
  `prefs.rs`'s old "unknown fields are not round-tripped" trade-off.
- **Never automatic.** The app never writes `ui-prefs.toml`.
- **`ledgeline.toml` is untrusted input.** It travels with a cloned or shared
  repo, so it may hold UI prefs only. Nothing in it is ever executed or
  security-relevant: no `hledgerPath`, no server URL or token. Every value is
  validated when loaded: numbers are clamped, enums checked, the file size is
  capped, and an invalid key falls back to the next layer with a Problems
  finding. Unknown keys are ignored, but kept on write.
- **Writes use the vault's pinned primitives.** `create_new_pinned` on first
  write, then `replace` against the pin, with `O_NOFOLLOW`. The client
  debounces saves (~500 ms).
- **`serverUrl` and `serverToken` stay in the browser's localStorage.** They
  describe the client, not the ledger.
- **Migration, done once and without loss:**
  - If `prefs.toml` is absent and `prefs.json` exists, convert it, write
    `prefs.toml`, then rename the old file to `prefs.json.migrated`.
  - The first time the client loads with a localStorage blob and the ledger
    has no `[ui]` table, it PATCHes the blob's UI fields (everything except
    url and token) to the server once, then removes them from localStorage.
- **No ledger open:** the app uses global prefs plus defaults, and saves stay
  in memory only.

### Work

- **Core/server:**
  - Add a `toml_edit` direct dependency. It is already in `Cargo.lock`
    through other crates; check `cargo deny` and the audit.
  - Move `prefs.rs` to TOML, including the migration.
  - Add a new `ui_prefs.rs` module: load and merge, validate, apply a patch.
  - Add `GET /api/ui-prefs` (returns the effective values) and
    `PATCH /api/ui-prefs` (a partial update to `ledgeline.toml`).
  - Reload when a different ledger is opened (File → Open, recents).
  - Wire-contract tests, plus tests for the migration, the merge order,
    invalid values from a hostile `ledgeline.toml`, comment preservation, and a
    symlinked `ledgeline.toml` (refused).
  - Run `just test-integration`, since this is a new `/api` shape.
- **Web:**
  - `settings.svelte.ts` loads the UI fields from the server and PATCHes
    changes. Typed decoder in `nativeDecode.ts`.
  - The one-time localStorage migration.
  - Settings stops writing those fields to localStorage.
- **Docs:** a settings-files page covering the three files, the merge order,
  an annotated example of each, and a suggestion to gitignore
  `ledgeline.toml` if you don't want UI changes in the repo history.

**Done when:**
- Relaunching the desktop app keeps the column, chart and benchmark settings.
- Two ledgers keep different settings.
- A hand-written comment in `ledgeline.toml` survives an automatic save.
- An existing `prefs.json` user ends up with `prefs.toml`, and their hledger
  path still works.
