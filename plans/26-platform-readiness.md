# 26 — Platform readiness: vault seam, logic in core, windowed journal, FFI facade

Make `ledgeline-core` the product and every UI a thin layer over it, so that
a mobile app (SwiftUI shell, any of the pathways) or a native desktop UI
(egui, GPUI) can be chosen later without re-architecting.

## The ask (Patrick, 2026-10-17)

- Do the groundwork that every UI pathway needs. Don't pick a pathway yet.
- Be set up for an iOS app that opens journals from iCloud / Files and
  displays them fast.
- Keep the freedom to move desktop to egui or GPUI later.

## Background

- Native GUI spikes (egui, GPUI + GPUI Kit) live on local branches
  `worktree-agent-a7c54c08916f82438` and `worktree-agent-afc473f9bd2bc9b36`.
  Each has a `spikes/native-*/RESULTS.md`.
- **Current app at 200k transactions.**
  - Launch to first rows on screen takes about 4.6 s. About 3.4 s of that is
    the SPA downloading all of `/transactions` and normalizing it.
  - Memory is about 4.3 GB total, with WebContent at 3.1 GB.
  - Scrolling was fine (0–1 dropped frames at 60 Hz).
- **The spikes** got rows on screen in 0.55–0.7 s using 615–714 MB, because
  they call core directly.
- **mbr** already did the iOS work this plan copies:
  - the vault trait (`mbr-core::vault`);
  - `mbr-ffi` with UniFFI proc-macros, gated to Apple targets;
  - each web request answered in-process through the axum router
    (`Router::oneshot`), with no socket;
  - a `FileCoordinator` foreign trait (implemented in Swift);
  - iCloud handling: placeholder files that aren't downloaded yet ("dataless")
    are detected, and the read fails fast with `NotDownloaded` instead of
    blocking until the file downloads.

  See `mbr-markdown-browser/main/design/ios-architecture-research.md`.

## Decisions

- **The SPA requires the native API.** Drop the plain-hledger-web fallbacks in
  the SPA. The hledger-web-compatible routes (`/version`, `/transactions`,
  `/prices`, …) stay, with their parity tests, for external tools.
- **Scope of this round:**
  - the vault seam and the `AppDirs` seam;
  - domain logic moved into core;
  - the windowed journal API;
  - the FFI facade.
- **Deferred until a UI is chosen:**
  - a view-model crate (report row shaping);
  - server-side xlsx export;
  - edit and import form protocols.
- **Conditional writes, not mbr's unconditional `write_atomic`.** Ledgeline's
  external-change guarantee must survive the seam.
- **Symlinks:** keep ledgeline's policy, not mbr's. Scans and writes refuse
  symlinks; the journal path is resolved once at open.
- **No platform `cfg` in core.** iCloud and coordinator code lives in
  `ledgeline-ffi`.

## Work packages

There are two tracks. They can run in parallel and merge at the end of each
wave.

### Track A — storage seams

**A1. Vault seam (`ledgeline-core::vault`)**

- **Types, mirroring mbr:**
  - `VaultPath`: root-relative, `/`-separated, cannot escape the root
    lexically.
  - `Entry { path, kind: File|Dir|Symlink|Other, size, modified, availability }`.
  - `Availability { Local, NotDownloaded, Unknown }`.
  - `VaultError { NotFound, NotDownloaded, PermissionDenied, OutsideRoot, ReadOnly, Changed, InvalidPath, Io }`,
    with EDEADLK mapped to `NotDownloaded`.
  - `Revision { fingerprint, identity: Option<opaque> }`.
- **Trait methods:**
  - `root`, `stat`, `list_dir`, `canonicalize`
  - `read`, `read_prefix`, `read_pinned(cap)`
  - `replace(path, bytes, &Revision)`, `create_new`
  - `create_dir_all`, `remove_file`
  - `availability`, `request_download`, `local_path`, `key`/`vault_path`
- **`LocalVault`** wraps `pinned.rs` unchanged: directory file descriptors,
  `O_NOFOLLOW`, fsync, `renameat`, and the new file inherits the old one's
  owner and mode.
- **`MemVault`** is for tests. It can simulate files that aren't downloaded
  yet and changes made behind the editor's back.
- **Parser:**
  - New entry point `parse_journal_in(vault, main, overrides)`.
  - It returns each source's `Revision`, so `JournalEditor::load` stops
    re-reading included files. That closes the gap where a file could change
    between parse and pin.
  - Include resolution stays lexical inside the vault.
  - Remove the `current_dir` fallback.
- **Editor, discovery and config:**
  - The editor, `dirscan`, rules and projections discovery, and `hledger_conf`
    go through the vault.
  - `hledger_conf` stops walking at the vault root.
  - Fold the two stray `create_exclusive` helpers (in `rules_api` and
    `projections_api`) into `create_new`.
- **Server:**
  - Every journal-tree read or write in the `*_api.rs` files, `stage.rs`
    (`.latest.*`), `sidecar.rs` and `budget_api.rs` goes through the vault.
  - New entry point `AppState::from_vault(Arc<dyn Vault>, VaultPath)`.
    `from_journal_path` builds a `LocalVault` and delegates to it.
  - `Transaction.source_file` keeps its `PathBuf` keys, produced by
    `vault.key()`. Convert at the edges, as mbr does.
- **Watcher:**
  - The `notify` watcher moves behind a server `watcher` feature (on by
    default), keyed by `local_path`.
  - Add a vault-based `refresh()` diff entry point for hosts that have no file
    watcher (iOS).
- **External tools:** the hledger and git subprocesses go behind an
  `external-tools` feature (on by default) and require `local_path()`.

**A2. `AppDirs` seam**

- A small injected struct `{ config_dir, cache_dir, temp_dir, hledger_bin }`
  replaces `dirs::*`, `temp_dir` and env lookups in:
  - `prefs.rs`, `recents.rs`, `stage.rs`, `hledger.rs`.
- Recents store an opaque location token. On desktop it is the canonical
  path; on iOS it will be a bookmark id.

**A3. `ledgeline-ffi` crate**

- `crate-type = ["lib", "staticlib"]`.
- A `mobile` feature carries the server router, but not `watcher` or
  `external-tools`.
- UniFFI proc-macros, active only on `target_vendor = "apple"`, so it builds
  and its tests run everywhere.
- **Exports:**
  - `LedgelineApp::open(root, main, cache_dir, coordinator)`
  - `open_file(path, coordinator)`
  - `async handle(method, url, headers, body) -> Response`, which runs the axum
    router in-process (adapter modelled on mbr `mobile/transport.rs`)
  - `async refresh()`
- **`FileCoordinator` foreign trait:**
  - It has a **conditional** replace: re-read inside the coordinated write
    block, then compare against the expected revision.
  - A `CoordinatedVault` wraps `LocalVault`. It detects dataless files, does
    the per-thread `setiopolicy_np` so reads fail fast, and lists legacy
    `.icloud` stub files.
  - Swift-side code is out of scope.
- **Build check:** `cargo check -p ledgeline-ffi --features mobile --target aarch64-apple-ios`.
  Add the iOS std targets to the flake's dev toolchain if they are missing,
  as mbr does.

### Track B — logic into core, windowed journal

**B0. Cleanup**

- Delete the dead TS modules: `reports/prices.ts`, `imports/reorder.ts`,
  `holdings/commodities.ts`.
- Remove the SPA's plain-hledger-web fallbacks: `NativeApiUnavailableError`
  paths, the local `unbalanced` rule, and fallback comments in
  `journal.svelte.ts` and `checks/rules.ts`.

**B1. Richer metadata**

- `/api/journal` gains:
  - `reportStyles`: one style rule (first seen, precision raised to the
    maximum observed);
  - `dominantCommodity`: one rule replacing the 5 competing heuristics;
  - `txnCount`, `firstDate`, `lastDate`, `generation`.
- `/accounts` (or `/api/journal`) carries each account's resolved type and
  term. Add `infer_bs_term` in Rust.
- Benchmark labels come from Rust.
- Period payloads carry each bucket's `label`, `start` and `end`.
- **Result:**
  - Reports, budget, projections and holdings stop reading `journal.txns`.
  - Delete `domain/accountTypes.ts` and most of `accountTerms.ts`.
  - The shared classification golden stays.

**B2. Display formatter in core**

- Port `formatDec`, `displayPlaces` (2-decimal display cap, relaxed when
  rounding would show zero) and `formatAmount`.
- Lock it with a shared golden fixture that both the Rust and TS tests run,
  following `fixtures/account-types/`.
- Search text (the haystack), check messages and future UIs depend on it.

**B3. Cash balances endpoint**

- `/api/balances/cash?asOf=` ports `reports/cashBalances.ts`.
- Flag in the commit any deliberate semantic alignment with the grouped
  balance sheet: virtual postings, `bsgroup`.

**B4. Checks into diagnostics**

- Port pending, uncategorized, negative-cash, missing-description and
  future-date into `journal_to_all_diagnostics`.
- Pass the client's date as `?today=`, so the server's time zone doesn't
  change results.
- Entries carry `tindex`, `date` and `description`, plus `{count, maxSeverity}`.
- `checks/rules.ts` goes away. The problems badge no longer needs the full
  journal.

**B5. Windowed journal API** (the big one)

- **New endpoints** (additive; never `GET /api/transactions`, because the
  "can I edit?" probe relies on that answering 405):
  - `GET /api/journal/rows?from&to&accounts&q&offset&limit&order` returns
    `{generation, total, offset, rows}`. Amount mantissas are strings, and
    rows embed their flags.
  - `GET /api/journal/summary?<filter>` returns the count, `visibleNet`,
    per-commodity income, expenses and net, commodities in use, categories in
    use, and max depth.
  - `GET /api/journal/chart?<filter>&mode&depth&commodity&category&interval&max`
    returns the insights pie or line data. Sign-convention detection is
    ported and memoized per snapshot.
  - `GET /api/journal/position?index&<filter>`
  - `GET /api/transactions/{index}`
- **Server:**
  - Per snapshot, lazily build the sort order and the lowercase search text,
    using the B2 formatter so search results match today's exactly.
  - Keep a small LRU of filter results.
  - Make the `/transactions` body lazy (`OnceLock`), so edits stop paying
    about 563 ms and 347 MB unless an external client asks for it.
- **SPA:**
  - Split `journal.svelte.ts` into a metadata store and a filter-keyed view
    store (summary plus a cache of 200-row blocks).
  - The table renders skeleton rows for blocks not yet loaded.
  - `listCursor` gets a variant whose items load lazily.
  - The 30 s poll target becomes `/api/journal`.
  - The edit modal fetches the single transaction it edits.
  - Journal xlsx export pages through `rows?order=asc`.
- **Success criteria at 200k:**
  - first rows on screen well under 1.5 s;
  - WebContent memory under 500 MB;
  - no regression in scroll frames.
  - Measure with the same probe method as the baseline. That probe was a
    throwaway patch to `gui.rs`, env-gated by `LEDGELINE_PERF`. Re-create it
    as a documented dev tool.

**B6. Budget summary and projection rules into core**

- `summarizeBudget`, `budgetLeaves`, `budgetTotals`, `elapsedFraction`, pace
  and health go into `/api/budget`. Use exact rational pace and note any 1e-6
  differences.
- The `BudgetSummary.svelte` and `BudgetGaps.svelte` logic, `annualised`, and
  `joinableRule`/`alreadyBudgeted` move too.
- The projections API accepts magnitude plus section, and resolves the sign
  by account type on the server.

## Sequencing

- **Wave 1 (parallel):** A1 and B0 → B1 → B2.
- **Wave 2 (parallel):** A2 → A3, and B3 → B4 → B5.
- **Wave 3:** B6. Then an end-to-end perf re-measure, and docs.

Each wave merges into this branch, followed by a full `just pre-push`.

## Verification

- `just pre-push` is green after every merge. That covers fmt, clippy, Rust
  tests, vitest, svelte-check, e2e and audits.
- The hledger-web parity tests stay untouched and passing.
- New Rust tests:
  - `MemVault`-backed parse, edit and save, including the not-downloaded case
    and a file changed behind the editor's back;
  - golden parity for the formatter, checks, cash balances and sign
    conventions against the TS outputs they replace. Capture the goldens
    *before* deleting the TS.
- E2E footer counts are unchanged: "28 transactions" and "189 transactions".
- Perf: re-run the launch and scroll probe at 200k and record the results in
  `docs/development/perf-baseline.md`.
- FFI: `cargo test -p ledgeline-ffi --features mobile` exercises `handle()`
  against a `MemVault` journal. The iOS target `cargo check` passes.

## Status (2026-10-17)

All four work packages of this round landed on branch `2026-10-16-no-web`
(`git log --oneline f858be3..HEAD`). `just pre-push` is green after each merge
and after B6.

### What landed

- **A1, vault seam.** `f37b31a` (types, `LocalVault`, `MemVault`), `a5c767a`
  (parse and discovery through the vault), `bfeab2f` (editor pinned to the
  parse's own reads, closing the parse-to-pin gap), `56b0f5d`
  (`JournalLocation` on every snapshot; `AppState::from_vault`, `reload`,
  `refresh`), `ef4b014` `1cf2bc3` `c8e3390` `9ff595e` `acc5365` `525b35a`
  `780edb9` (every `*_api` read and write through the vault; the two
  `create_exclusive` helpers folded into `create_new`), `6a788c5` (`watcher`
  and `external-tools` features), `1d1aeed` (restored the desktop's
  `hledger.conf` walk above the root and the umask mode for authored files),
  `d382636` `40f3035` `8936651` (tests and doc fixes). Merged in `2e95016`.
- **A2, `AppDirs`.** `e9afd82`: config, cache, temp and `hledger` injected;
  recents store an opaque `RecentLocation` token, and existing `recent.json`
  files load unchanged.
- **A3, `ledgeline-ffi`.** `f487e33` (injectable availability probe,
  fingerprint components), `b213a5c` (in-process hooks; `NotDownloaded` is a
  503 with `Retry-After`), `3b048ff` (the crate: `CoordinatedVault`,
  `SingleFileVault`, `FileCoordinator`, fail-fast iopolicy,
  `LedgelineApp::{open, open_file, handle, refresh}`). Merged with A2 in
  `42d6c26`.
- **B0, cleanup.** `562a4c1`: dead TS modules and the plain-hledger-web
  fallbacks removed.
- **B1, metadata.** `82ba76b` (engine: `reportStyles`, `dominantCommodity`,
  `txnCount`, dates, `generation`, account types and terms, period info,
  benchmark labels), `d15127c` (SPA reads them; `accountTypes.ts` and most of
  `accountTerms.ts` deleted).
- **B2, display formatter.** `644a6d6`, with the shared golden
  `fixtures/display-format/cases.json`.
- **B3, cash balances.** `483f371`: `/api/balances/cash`. A straight port, with
  no alignment to the grouped balance sheet.
- **B4, checks.** `949fa26`: the five local rules in `/api/diagnostics?today=`;
  `checks/rules.ts` deleted.
- **B5, windowed journal.** `95034b5` (engine: `/api/journal/{rows,summary,
  chart,position}`, `GET /api/transactions/{index}`, per-snapshot memoized
  views and an 8-entry filter LRU, lazy `/transactions`), `7be2be1` (SPA:
  metadata store plus a filter-keyed view store of 200-row blocks), `acaa6e3`
  (the `LEDGELINE_PERF` launch and scroll probe as a dev tool). At 200k
  transactions first rows went from 4.25 s to 1.01 s and WebContent from
  2.7–3.5 GB to 144–185 MB, with no scroll regression
  (`docs/development/perf-baseline.md` §8). All three success criteria met.
- **B6, budget summary and projection rules.** `fafbc74`.
  - `/api/budget` gains `summary` (span, `elapsedDays`/`spanDays`, Income and
    Expenses sections by resolved type, each bar's magnitudes, pace,
    `remaining`, health and label kind plus amount), and takes `?asOf=`.
  - `/api/budget/gaps` gains `revenueTotal`, `expenseTotal`, `headline` and a
    per-row `magnitude`.
  - `/api/budget/lines` gains each goal's `annual` (exact × 365/52/12/4/1,
    absent for an unmodelled period) and `joinable`, the rule a new goal of
    each period joins.
  - Projections: a scenario line may carry `entry`, a magnitude the server
    signs by account type (`account_types::signed_entry`); every outbound line
    carries `section` (`projections::line_section`). Bodies without `entry`
    mean what they always did. Saved scenario files are unchanged.
  - Goldens captured from the TypeScript first: `fixtures/budget-summary/`
    (8 fixture journals, 7 with budgets; 176 window × as-of cases; 1,067 bars) and
    `fixtures/projections-signs/`.

### Deviations

- **Pace is exact, so it differs slightly from the TypeScript.** The TS
  snapped the elapsed fraction to six places before multiplying, an error of
  up to `|goal| × 5e-7`. The engine keeps the ratio exact: health compares
  `actual × span` with `goal × done`, and the pace amount is rounded six places
  past the goal's scale. 453 of the captured pace figures differ, the largest
  by $0.0143 on a $30,450 span goal. Above roughly $20,000 that can move a
  displayed "ahead of / behind pace" figure by a cent; the engine's is the
  correct one. Every health verdict and label kind matched exactly. The
  golden compares pace (and the two labels quoted from it) within
  `|goal| × 5e-7` plus one unit in its last place, and everything else
  exactly.
- **Projection sections are previewed in TS while typing.** The engine's
  `section` is authoritative for a line as it was sent. While a user retypes an
  account, the table still has to place the row synchronously for a name the
  engine has not seen, so `resolvedSection` keeps a preview of the same rule
  over the engine's account table until the line round-trips. The two are
  locked together by `scenario_signs_golden.rs`.
- **`goalChange` and `alreadyBudgeted` stay in TS** as request assembly over
  the engine's `joinable` answer. `joinableRule` is now a lookup.
- **B3** deliberately did not align cash balances with the grouped balance
  sheet (virtual postings, `bsgroup`).
- The wave-3 end-to-end perf re-measure was not repeated. B6 does not touch the
  launch path; the B5 measurement stands.

### Deferred

- By decision: a view-model crate (report row shaping), server-side xlsx
  export, and edit and import form protocols.
- The journal summary and chart are not cached per filter (only the row filter
  results are, in the LRU).
- The SPA sends `Cache-Control: no-store`, so it does not use the `ETag`s the
  journal routes serve.
- The prewarm thread computes the problem list for the server's UTC date; a
  browser in another date pays for its own.
- Some `*_api` read paths still answer 500, not 503, on `NotDownloaded`.
- On mobile, `PUT /api/prefs` has no config directory to write to.
- The desktop watcher's reload does not rebind the editor after a failed
  reload.
- No Swift shell or xcframework yet.
- Pre-existing: the Insights panel squeezes the table at 1280×720.
