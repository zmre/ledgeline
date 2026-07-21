# ![](web/static/ledgeline-icon.png) Ledgeline (hledger GUI)

A fast, local, privacy-centric desktop app for [hledger](https://hledger.org) plain-text accounting. Ledgeline is a **single binary** that opens a native window showing a modern web UI, with a Rust journal engine and API server running in-process. It parses your journal file directly and reproduces hledger's numbers exactly (differential-tested against hledger 1.52).

I built this because I was dissatisfied with existing GUIs. They often hard code expectations for where files are and how they link. They rarely handle stocks well. If they allow editing, I found the editing to be problematic and buggy. I love the command line and editing in the terminal, but sometimes I want graphs and something pretty and ledgeline scratches that itch.

> [!WARNING]
> I built this for myself and based it on patterns I've built by hand in the past (see [mbr](https://github.com/zmre/mbr-markdown-browser/)), but this was built using AI.  It's okay if you don't use it.

## What it does

- **Journal view** with live filtering and an insights panel (pie / line charts, account-depth control).
- **Reports** — balance sheet, income statement, cash flow, net worth, and (coming soon) budgets (`~` periodic
  goals vs. actuals), computed in Rust with exact decimal math and hledger parity. XLSX exports.
- **Holdings** — average-cost basis, unrealized gain (all-time / year-to-date / trailing-12-months),
  value-over-time, per-symbol names from `commodity` directives, partial portfolio totals; XLSX export.
- **In-process, same-origin API** exposing both the hledger-web-compatible wire endpoints
  (`/version`, `/transactions`, `/prices`, …) and native report / holdings / budget JSON (`/api/*`) and
  edit endpoints.
- **No preconceived notions** on how accounts are setup or where things live or how they're organized.

## Install / Use

For now, requires [Nix](https://nixos.org) with flakes and works on Linux and Mac. In theory it should run on Windows, too, but I haven't tested that.  

On Mac, there's a native application bundle.  If there's demand (submit an issue), I'll build releases and maybe even publish them places.

**To run it directly in Nix**:

```sh
nix run github:zmre/ledgeline -- ~/finance/2026.journal   # opens the desktop window on the specified journal (or don't specify and you can open from inside the app)
```

**Install the binary + app** into your Nix profile:

```sh
nix profile install github:zmre/ledgeline
# macOS → installs bin/ledgeline (on PATH) AND Applications/Ledgeline.app
# then:  ledgeline ~/finance/2026.journal        # or launch Ledgeline.app
```

**Build the macOS app bundle** to open or drag into `/Applications`:

```sh
nix build github:zmre/ledgeline        # or, in a local checkout: nix build
open result/Applications/Ledgeline.app # macOS — real UI embedded

just package-mac                       # macOS: a writable dist/Ledgeline.app to open / drag to /Applications
```

## Development (or if you don't have nix)

```sh
direnv allow          # or: nix develop path:.
just --list           # available tasks
just engine-test      # cargo test over the workspace
just check            # SPA type-check + unit tests
cd web && bun run build && cd .. && cargo build --release && ./target/release/ledgeline ~/.../Ledger/main.journal
```

See **[docs/development.md](docs/development.md)** for the Nix + Crane build cache, the
`nix build .#{ledgeline,clippy,tests,fmt,macApp}` outputs, CI, and how the SPA is built and embedded.

## Architecture

This spins up a local tokio axum API server and uses the native OS browser as a GUI window (via wry, part of the tauri project) hosting a svelte frontend app.  All assets are built into the single binary, which is pretty snappy.

## TODO

- A QuickLook plugin for journal files — render a file's transactions nicely for fast Finder browsing
  (see `mbr-markdown-browser` for the approach).
- bug: the monitor for updates thing only watches the main file, not includes
- feat: budgeting
- feat: hledger check in the background including the various extras i use (and recheck issues after new updates)
- feat: preferences?
