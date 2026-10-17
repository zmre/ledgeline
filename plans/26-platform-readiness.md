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
