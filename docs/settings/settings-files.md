---
title: Settings files
description: "The three TOML files Ledgeline keeps its own settings in: prefs.toml for the hledger path, ui-prefs.toml for your default view settings, and ledgeline.toml beside each journal for that ledger's view settings. How they merge, what each key does, and whether to commit ledgeline.toml."
order: 40
---

# Settings files

Ledgeline keeps its own settings in three TOML files. You can edit any of them by hand.

| File | Where | Written by | Holds |
| --- | --- | --- | --- |
| `prefs.toml` | Ledgeline's config folder | The hledger path field on the Imports screen | `hledgerPath`, the hledger program Ledgeline runs |
| `ui-prefs.toml` | Ledgeline's config folder | You. Ledgeline never writes it | Your default view settings for every ledger |
| `ledgeline.toml` | The folder of your main journal | Ledgeline, as you use it | This ledger's view settings |

View settings are the choices Ledgeline remembers as you work: which journal columns show, which panels are open, the Review and Find Matches numbers, the benchmark lines on the Stocks chart, and the like. Each ledger keeps its own, so a business ledger and a household ledger can be set up differently.

The list of recently opened journals stays in `recent.json`, in the same config folder.

## The config folder

| System | Folder |
| --- | --- |
| macOS | `~/Library/Application Support/ledgeline/` |
| Linux | `~/.config/ledgeline/`, or `$XDG_CONFIG_HOME/ledgeline/` if you set that variable |

Set `LEDGELINE_CONFIG_DIR` to use a different folder. It moves all of these files at once.

## Which value wins

Ledgeline looks up each view setting on its own, in this order, and uses the first usable value it finds:

1. the `[ui]` table in `ledgeline.toml`, beside the journal you have open;
2. `ui-prefs.toml` in the config folder;
3. Ledgeline's built-in default.

When you change something in the app, Ledgeline writes only that key to `ledgeline.toml`. Everything you haven't changed for this ledger keeps coming from `ui-prefs.toml` or the defaults. So to change a default for all your ledgers, edit `ui-prefs.toml`. A ledger that has its own value for that key keeps it.

Opening a different journal (File → Open, or Open Recent) loads that ledger's settings.

## `ledgeline.toml`

Ledgeline creates this file beside your main journal the first time you change a view setting. Here is one after a few changes, with comments added by hand:

```toml
# Ledgeline's settings for this ledger. Ledgeline saves your view choices
# to the [ui] table automatically, writing only the keys you change.
# Comments and keys it does not know are kept. Docs: "Settings files".

[ui]
# The business card settles slowly, so look further apart for transfers.
matchWindow = 21
insightsTab = "balances"
benchmarks = ["SPY", "QQQ"]

[ui.columns]
status = false   # nobody here uses ! and *
```

Ledgeline edits the file in place. Your comments, the order of the keys, and keys it does not know are kept. The `[ui]` table is the only part it reads or writes. The rest of the file is left for later per-ledger settings.

### It travels with the ledger

If your books are in a git repository or a shared folder, `ledgeline.toml` goes wherever the journal goes. A file somebody else wrote is treated as untrusted:

- It can only hold view settings. Ledgeline never reads the hledger path, a server address or an access token from it. If one of those keys is there, Ledgeline ignores it and says so.
- Every value is checked. A number outside its range is clamped. A choice that isn't one of the allowed values, or a value of the wrong type, is ignored, and that key falls back to `ui-prefs.toml` or the default.
- Unknown keys are ignored, and kept when Ledgeline saves.
- A file larger than 64 KiB, a file that isn't valid TOML, and a symbolic link are ignored. Ledgeline won't save over any of them. Fix or remove the file to start saving again.

When Ledgeline ignores something in either file, a notice in the bottom corner lists what it skipped and why.

### Commit it or ignore it

If your books are under version control, you choose whether view settings belong in the history. Commit `ledgeline.toml` to share a layout with everyone who opens the ledger. If you'd rather not see a change every time someone collapses a panel, add it to your `.gitignore`:

```gitignore
# Ledgeline's per-ledger view settings
ledgeline.toml
```

## `ui-prefs.toml`

The same keys as `[ui]`, at the top level, with `[columns]` for the journal columns. Ledgeline reads this file and never writes it. Create it yourself:

```toml
# My defaults for every ledger. A ledger's own ledgeline.toml wins key by key.
hideZeroBalances = false
reviewMaxDays = 90
holdingsPieDimension = "assetClass"

[columns]
status = false
```

A symbolic link here is fine. Dotfile managers often install config files that way.

## `prefs.toml`

The hledger path you enter on the Imports screen is saved here. Ledgeline checks the program before saving it: it has to be a runnable hledger that no other user can modify. Leave the field empty, or remove the key, to go back to finding hledger on its own.

```toml
# Ledgeline preferences. Setting the hledger path in the app writes `hledgerPath`
# here; you can also edit this file by hand. Comments and unknown keys are kept.
hledgerPath = "/opt/homebrew/bin/hledger"
```

A path you type into the file by hand isn't checked until Ledgeline runs it. If it doesn't work, Ledgeline tries the other places it looks for hledger.

## The keys

| Key | Default | Allowed values |
| --- | --- | --- |
| `columns.date`, `columns.status`, `columns.description`, `columns.accounts`, `columns.amount` | `true` | `true` or `false`: whether the journal table shows that column |
| `insightsOpen` | `true` | Whether the panel above the journal is open |
| `insightsTab` | `"activity"` | `"activity"` or `"balances"` |
| `hideZeroBalances` | `true` | Balances tab: leave out accounts holding nothing |
| `flowsInOpen`, `flowsOutOpen` | `true` | The Money in and Money out panels |
| `netWorthChartOpen`, `cashFlowChartOpen` | `true` | The charts above the Net Worth and Cash Flow tables |
| `budgetGapsOpen` | `false` | The Budget tab's "not budgeted" section |
| `startupMoreOpen` | `false` | The "More metrics" list under the startup figures |
| `holdingsPieDimension` | `"holding"` | `"holding"`, `"assetClass"`, `"sector"`, `"industry"`, `"securityType"`, `"category"` or `"risk"` |
| `lastProjectionId` | none | The scenario file the Projections tab reopens, such as `"plans/series-a.journal"`. `""` means none |
| `benchmarks` | `[]` | Ticker symbols such as `"SPY"`, at most 32 |
| `reviewRecentLimit` | `50` | 1 to 1000 |
| `reviewMaxDays` | `365` | 0 to 36500. 0 means no limit |
| `reviewMarkers` | `true` | The journal's review stripes |
| `matchWindow` | `10` | 1 to 60 days |
| `matchLookback` | `365` | 0 to 36500 days. 0 means no limit |
| `matchTransitDays` | `10` | 1 to 365 days |

## Upgrading from an earlier version

Earlier versions kept the hledger path in `prefs.json`. The first time Ledgeline starts and finds `prefs.json` but no `prefs.toml`, it writes `prefs.toml` with the same path and renames the old file to `prefs.json.migrated`. Nothing is deleted.

Earlier versions also kept view settings in the browser's storage, which the desktop app lost every time it restarted. The first time you open a ledger that has no `[ui]` table, Ledgeline copies the view settings it still finds in the browser's storage into that ledger's `ledgeline.toml`, then removes them from the browser. The address and access token of a separate Ledgeline server stay in the browser, because they describe your browser, not the ledger.
