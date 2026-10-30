---
title: Architecture
description: "How Ledgeline is layered: the core library, the axum server around it, and the hosts that run them (desktop, headless, iOS in-process)."
order: 5
tags: [development]
---

# Architecture

This page is for maintainers. It describes how the code is layered after plan 26 (platform readiness), and where a new user interface would plug in.

The short version: **`ledgeline-core` is the product.** It reads journals, edits them safely, and computes every report and view. `ledgeline-server` wraps it in an HTTP API. Hosts decide how that API is reached: a desktop window, a headless server, or an iOS app calling the router in-process. The Svelte app in `web/` only presents what the API returns.

```
hosts      desktop (wry window)   headless (--server)   iOS (ledgeline-ffi, in-process)
              │                       │                        │
server     ledgeline-server: axum router, snapshots, AppState, AppDirs
              │   features: gui · watcher · external-tools
core       ledgeline-core: vault · parse/edit · reports · view · display
```

## Core: `crates/ledgeline-core`

A plain library with no platform `cfg`s, no clock reads in report code, no HTTP, and no knowledge of where files live.

| Area | Modules | What it owns |
|---|---|---|
| Storage | `vault/`, `pinned.rs` | The `Vault` trait (`VaultPath`, `Entry`, `Availability`, `Revision`, `VaultError`). `LocalVault` wraps the pinned, symlink-refusing, fsync'd writes; `MemVault` is for tests and can simulate files that are not downloaded yet and edits made behind the editor's back. Writes are conditional on a `Revision`, so an external change is a conflict rather than a clobber. |
| Reading | `parse.rs`, `dirscan.rs`, `hledger_conf.rs`, `journals.rs` | `parse_journal_in(vault, main, …)` resolves includes inside the vault and returns each source's revision. |
| Writing | `edit.rs`, `periodic.rs`, `accounts.rs`, `aliases.rs`, `rules/`, `projections/serialize.rs` | Span-splice edits that change only the bytes they mean to, verified by a re-parse before anything is written. |
| Reports | `reports/` | Balance sheet, income statement, cash flow, net worth, budget and the budget summary (span totals, pace, health, labels), insights, subscriptions, holdings. Exact decimals throughout (`Dec`, `MixedAmount`). |
| Journal view | `view/` | The windowed journal: filter, sort, search text, summary, charts, local checks, cash balances. |
| Classification and display | `reports/account_types.rs`, `reports/account_class.rs`, `display.rs` | One resolver for account types and terms (declared, else inferred), the sign rule for entered amounts (`signed_entry`: revenue is entered positive and written negative), and the number formatter the UI uses. |
| Projections | `projections.rs` | The what-if engine, the scenario model, and the section a scenario line belongs in (`line_section`). |

Pure functions that used to live in TypeScript were ported here with golden fixtures captured from the TypeScript first; see `fixtures/journal-view/`, `fixtures/journal-meta/`, `fixtures/display-format/`, `fixtures/budget-summary/` and `fixtures/projections-signs/`, and the `*_golden.rs` tests that read them.

## Server: `crates/ledgeline-server`

An axum `Router` over an `AppState`. The library is what every host links; `main.rs` is the desktop and headless binary.

- **Snapshots.** `AppState` holds the current parsed journal as an immutable `Snapshot` (with lazily built, memoized views such as the sort order and search text). A reload builds a new snapshot and swaps it in; requests in flight keep the one they started with. `GET /api/journal` reports the snapshot `generation`, and the journal-view routes carry an `ETag` derived from it.
- **Entry points.** `AppState::from_vault(Arc<dyn Vault>, VaultPath)` is the general constructor; `from_journal_path` builds a `LocalVault` and delegates. `reload()` re-parses; `refresh()` is a vault-based content check for hosts with no file watcher.
- **`AppDirs`.** Config, cache and temp directories and the `hledger` binary are injected rather than looked up, so a sandboxed host can point them inside its container. Recents store an opaque location token.
- **Routes.** `reports_api.rs` (reports, budget), `journal_api.rs` (the windowed journal), `budget_api.rs`, `projections_api.rs`, `edit_api.rs`, `import_api.rs` and the rest. The hledger-web-compatible routes (`/version`, `/transactions`, `/prices`, …) remain for external tools, with their parity tests.
- **Cargo features.**
  - `gui` (default): the wry/tao desktop window, menus and file dialogs.
  - `watcher` (default): live reload from a `notify` file watcher. Without it, the host calls `refresh()`.
  - `external-tools` (default): running `hledger` and `git` as subprocesses. They need a real path (`Vault::local_path`); without the feature those features report that this build cannot run them.

## Hosts

- **Desktop** (`ledgeline`, default features). `gui.rs` binds axum to an ephemeral loopback port and opens a wry webview at it. The SPA is embedded in the binary.
- **Headless** (`ledgeline --server`, or a build with `--no-default-features --features watcher,external-tools`). The same router on a fixed port, token-guarded, for a browser or another client.
- **iOS** (`crates/ledgeline-ffi`, `mobile` feature). No socket: `LedgelineApp::handle(method, url, headers, body)` runs the router in-process (`Router::oneshot`) behind the app's custom-scheme handler. Storage is a `CoordinatedVault` that writes through a Swift `FileCoordinator` (conditionally, against the expected revision), detects iCloud placeholder files and fails fast with `NotDownloaded` instead of blocking. The `mobile` feature builds the server without `gui`, `watcher` or `external-tools`. UniFFI exports are compiled on Apple targets only, so the crate builds and tests everywhere. The Swift shell itself is not written yet.

## The web app

`web/` is a SvelteKit SPA. Since plan 26 it requires the native API and holds no journal-sized data: the Journal tab reads windows of rows, the budget bars come with their pace and verdicts, and a projection line is sent signed as the file writes it, which the engine writes unchanged. What remains in TypeScript is presentation: layout, formatting with the engine's styles, chart and bar geometry, and form state. A component may still preview something while the user types (for example, whether a projection rule's postings balance on their own), but the engine's answer is the authority once it arrives.

## Adding a native UI

A native desktop UI (egui, GPUI) or a SwiftUI app has two ways in:

1. **Link core directly.** Parse through a vault, call the report and view functions, and draw the results. This is the fastest path (the native spikes put rows on screen in well under a second at 200k transactions) and needs nothing from the server. Writes go through the same core editors, so the conditional-write guarantee holds.
2. **Call the router in-process.** Build an `AppState` and send requests to the router without a socket, as `ledgeline-ffi` does. This reuses every route's validation, caching and JSON shape, which is the shorter path if a UI already speaks the API.

Either way, nothing in core needs to change, and new domain logic belongs in core rather than in a UI.

Deferred until a UI is chosen (plan 26): a view-model crate that shapes report rows for display, server-side xlsx export, and protocols for the edit and import forms.
