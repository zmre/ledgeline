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
- [ ] The `insights` bench at 200k is ≤ 400 ms (baseline 968 ms).
- [x] `docs/development/perf-baseline.md` is updated.

**Status (2026-11-12): implemented; the gate is not met yet.**

- Two `PriceDb` builds per request, down from eight.
- One `FlowWindow::Chained` replay serves both windows, replacing four replays.
  This is `holdings_window_pair` in `holdings/engine.rs`.
- The movers probe reuses the windows' `HoldingsInputs` through `repriced`.
  It keeps its own replay because the pools read the price DB.

The bench at 200k went from 916 ms to **518 ms**. The pre-change baseline on
this run was 916 ms, not the 968 ms quoted above.

Identity was checked on 654 captures: every fixture journal at six spans or
options, plus the 5k/50k/200k bench corpora. All matched before and after.
`tests/insights_golden.rs` now pins this output; its snapshots were blessed on
the pre-change code.

The remaining cost is about 16 posting passes; `perf-baseline.md` §3 has the
breakdown. The next levers are memoizing account classification inside the
holdings engine, and sharing the sole-symbol facts with the movers probe. Both
are outside WP-1's approach. The `TODO.md` gate entry stays, with updated
numbers.

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
- All existing price and report tests pass unchanged.
- The dense-shape numbers improve and v2/v2fx don't regress.
- perf-baseline records the dense shape and the RSS numbers.

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
