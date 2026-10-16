---
title: Ledgeline manual
description: "The Ledgeline user manual: a fast, local desktop app for viewing, editing, importing into and reporting on hledger journals."
order: 0
tags: []
---

# Ledgeline

Ledgeline is a desktop app for [hledger](https://hledger.org) plain-text accounting. It opens your journal file, shows it in a native window, and lets you browse, edit, import into and report on it. Everything runs on your own machine: one binary holds the engine and the UI, and your journal never leaves your disk.

Ledgeline reads the journal itself and reproduces hledger's numbers exactly, with exact decimal arithmetic. It doesn't assume where your files live or how your accounts are named. When it writes, it writes ordinary hledger syntax that the `hledger` command reads the same way.

<!-- screenshot: the Journal tab on the sample journal, full window, as the first thing a new user sees -->

## Install

**macOS.** Download the `.dmg` for your Mac from the [latest GitHub release](https://github.com/zmre/ledgeline/releases/latest). Use `arm64` for Apple silicon and `x86_64` for Intel. The app is signed and notarized. Drag it to Applications.

**Nix (macOS and Linux).** Run it straight from the flake:

```sh
nix run --accept-flake-config github:zmre/ledgeline -- ~/finance/2026.journal
```

`--accept-flake-config` lets Nix download prebuilt binaries from the project's cache instead of compiling. On macOS, `nix build github:zmre/ledgeline` produces `result/Applications/Ledgeline.app`, which you can open or copy to Applications.

**Cargo.** Without Nix, install from crates.io:

```sh
cargo install ledgeline                        # desktop window; on Linux needs webkitgtk-4.1, gtk3, libsoup-3 and libxdo
cargo install ledgeline --no-default-features  # no window; serves the UI to your browser
```

The headless build prints a `http://127.0.0.1:PORT/#token=…` address. Open it in your browser.

Imports need hledger 1.40 or newer. The macOS app bundles its own copy. Elsewhere, Ledgeline looks for `hledger` on your `PATH` and lets you point it at a binary if it can't find one. See [Imports](imports/index.md).

## Opening a journal

Pass the journal on the command line, as in the `nix run` example above. Ledgeline follows its `include` directives, so open your main file, not one of the files it includes.

With no argument, Ledgeline reopens the last journal you used. In the desktop app, **File → Open** (<kbd>Cmd</kbd>+<kbd>O</kbd> on macOS, <kbd>Ctrl</kbd>+<kbd>O</kbd> on Linux) picks another one, and **File → Open Recent** lists the ones you've had open. On macOS, double-clicking a `.journal` or `.hledger` file opens it in Ledgeline. Ledgeline is also offered under **Open With** for `.ledger` and `.j` files.

On Linux the window has no title bar or menu bar until you press <kbd>F10</kbd>.

Ledgeline watches the journal for changes. If you edit it in another editor, the app reloads.

## Getting around

The top bar has a tab for each part of the app. Most things can be done from the keyboard: <kbd>?</kbd> shows the shortcuts for the page you're on, and two-key sequences such as <kbd>g</kbd> <kbd>b</kbd> jump between tabs. See [Keyboard shortcuts](journal/keyboard.md).

## Map of the manual

- [Journal](journal/index.md): the transaction list, [filtering](journal/filtering.md), editing, [problems](journal/problems.md) the background checks find, [keyboard shortcuts](journal/keyboard.md), and [exporting the journal to XLSX](journal/export.md).
- [Imports](imports/index.md): [bringing in bank and card statements](imports/new-transactions.md), [creating](imports/create-rules.md) and [editing](imports/rules-editor.md) CSV rules files, and [importing from QuickBooks](imports/quickbooks.md).
- [Reports](reports/index.md): [Insights](reports/insights.md), the [balance sheet](reports/balance-sheet.md), the [income statement](reports/income-statement.md), [cash flow and net worth](reports/cash-flow-and-net-worth.md), and [subscriptions](reports/subscriptions.md).
- [Holdings](holdings/index.md): stocks and other assets, the [`holdings:` and `valuation:` tags](holdings/tags.md), [price updates](holdings/prices.md), [allocation](holdings/allocation.md) and [benchmarks](holdings/benchmarks.md).
- [Budget](budget.md): goals written as hledger `~` rules, and how you're tracking against them.
- [Projections](projections/index.md): what-if scenarios for cash runway and net worth.
- [Settings](settings/index.md): [declaring accounts and their types](settings/accounts.md), and [account aliases](settings/account-aliases.md).
- [Development](development/index.md): building, releasing, and how the internals work.

Several account tags change how Ledgeline groups and reports accounts, such as `bsgroup:`, `isgroup:`, `holdings:` and `valuation:`. Every page that explains one carries the `account-tags` tag.

The source code, issue tracker and releases are on [GitHub](https://github.com/zmre/ledgeline).
