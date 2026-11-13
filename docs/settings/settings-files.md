---
title: Settings files
description: "The TOML files Ledgeline keeps its own settings in: prefs.toml for the hledger path, ui-prefs.toml for your view settings, and an optional ledgeline.toml you can put beside a journal to give that ledger its own. How they merge, where a change is saved, and what each key does."
order: 40
---

# Settings files

Ledgeline keeps its own settings in TOML files. You can edit any of them by hand.

| File | Where | Written by | Holds |
| --- | --- | --- | --- |
| `prefs.toml` | Ledgeline's config folder | The hledger path field on the Imports screen | `hledgerPath`, the hledger program Ledgeline runs |
| `ui-prefs.toml` | Ledgeline's config folder | Ledgeline, as you use it, and you | Your view settings, for every ledger |
| `ledgeline.toml` | The folder of your main journal | You, or Ledgeline when you ask it to [keep a setting for one ledger](#keeping-a-setting-for-one-ledger) | One ledger's own view settings, overriding `ui-prefs.toml` |

View settings are the choices Ledgeline remembers as you work: the theme and interface scale, which journal columns show, which panels are open, the Review and Find Matches numbers, the benchmark lines on the Stocks chart, and the like.

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

Opening a different journal (File → Open, or Open Recent) loads that ledger's `ledgeline.toml`, if it has one.

## Where a change is saved

When you change a view setting in the app, Ledgeline saves only that key, and picks the file one key at a time:

- If the open ledger has a `ledgeline.toml` and its `[ui]` table already sets that key, the change is saved there.
- Otherwise it goes to `ui-prefs.toml`, and applies to every ledger that doesn't set the key itself.

So a `ledgeline.toml` only ever holds the keys someone put in it. Changing the theme, for example, saves it to `ui-prefs.toml`, even in a ledger that has a `ledgeline.toml`. Once `theme` is in that file's `[ui]` table, whether you added it by hand or [ticked "Use for this ledger only"](#keeping-a-setting-for-one-ledger), theme changes made in that ledger are saved there instead.

The journal columns are decided one column at a time. A change to the Status column is saved in `ledgeline.toml` only if its `[ui.columns]` table already has `status`. The other columns still go to `ui-prefs.toml`.

Opening the app and looking around writes nothing. Ledgeline creates `ui-prefs.toml` the first time you change a view setting. It creates `ledgeline.toml` only when you ask it to keep a setting for one ledger.

## Keeping a setting for one ledger

The theme, the interface scale and the automatic price updates can be kept for one ledger from inside the app, so you can tell your business books from your personal ones at a glance, or let only your real books fetch prices. Tick "Use for this ledger only" next to the setting:

- Ledgeline writes the setting's current value into the `[ui]` table of the open ledger's `ledgeline.toml`. If the ledger has no `ledgeline.toml` yet, Ledgeline creates it. This is the only time Ledgeline creates that file, and it only happens because you asked.
- From then on, changing that setting while this ledger is open saves it in `ledgeline.toml`. Other ledgers keep using `ui-prefs.toml`.
- Untick the box to go back to the shared value. Ledgeline removes the key from `[ui]`, along with any comment lines directly above it. The rest of the file stays, and Ledgeline never deletes the file, even if nothing is left in it.

Only `theme`, `uiScale`, `autoPrices` and `autoPricesWeeks` can be kept this way from the app (the Prices tab keeps the last two together). For any other key, add it to `ledgeline.toml` by hand.

Ledgeline won't write to a `ledgeline.toml` that is a symbolic link, is larger than 64 KiB, or isn't valid TOML. In that case the box can't be ticked, and a notice says why. The checkbox isn't offered when [view settings are kept in the browser](#keeping-view-settings-in-the-browser).

## `ui-prefs.toml`

The view settings for all your ledgers: the keys from [the table below](#the-keys) at the top level, with `[columns]` for the journal columns. Ledgeline edits the file in place as you change things. Your comments, the order of the keys, and keys it doesn't know are kept, so you can also edit it by hand:

```toml
# Ledgeline's view settings for every ledger. Ledgeline saves the keys you
# change here. A ledger's own ledgeline.toml overrides any key its [ui] table
# sets. Comments and keys Ledgeline does not know are kept. Docs: "Settings files".

hideZeroBalances = false
reviewMaxDays = 90           # a quarter is plenty
holdingsPieDimension = "assetClass"

[columns]
status = false
```

A symbolic link here is fine for reading, because dotfile managers often install config files that way. Ledgeline won't write through the link or replace it, though: home-manager, for one, links the file into a read-only folder. With a linked `ui-prefs.toml`, a change you make in the app lasts until Ledgeline quits, and a notice says the file is managed outside Ledgeline. To keep the change, put it in your dotfiles.

If the file isn't valid TOML, is larger than 64 KiB, or isn't a regular file, Ledgeline ignores it and won't save over it. Fix or remove the file to start saving again.

## `ledgeline.toml`

Give one ledger its own values by creating `ledgeline.toml` in the folder of its main journal. Put the keys under `[ui]`, with `[ui.columns]` for the columns:

```toml
# This ledger's own view settings. Ledgeline keeps saving changes to these
# keys here; everything else comes from ui-prefs.toml.

[ui]
# The business card settles slowly, so look further apart for transfers.
matchWindow = 21
insightsTab = "balances"
benchmarks = ["SPY", "QQQ"]

[ui.columns]
status = false   # nobody here uses ! and *
```

Every key you list there overrides `ui-prefs.toml` in this ledger, and changes you make to those keys in the app are saved back to this file. Ledgeline edits values in place and keeps your comments, the order of the keys, and keys it doesn't know. The `[ui]` table is the only part it reads or writes. The rest of the file is left for later per-ledger settings.

To stop overriding a key, delete its line. The ledger then follows `ui-prefs.toml` for it again.

### It travels with the ledger

If your books are in a git repository or a shared folder, `ledgeline.toml` goes wherever the journal goes. Since Ledgeline never writes keys you didn't put there, the file only changes when someone changes one of its own settings. That makes it reasonable to commit, so everyone who opens the ledger gets the same layout.

A file somebody else wrote is treated as untrusted:

- It can only hold view settings. Ledgeline never reads the hledger path, a server address or an access token from it. If one of those keys is there, Ledgeline ignores it and says so.
- Every value is checked. A number outside its range is clamped. A choice that isn't one of the allowed values, or a value of the wrong type, is ignored, and that key falls back to `ui-prefs.toml` or the default. A change you make to that key in the app replaces the bad value.
- Unknown keys are ignored, and kept when Ledgeline saves.
- A file larger than 64 KiB, a file that isn't valid TOML, and a symbolic link are ignored. Ledgeline won't save to any of them, so your changes go to `ui-prefs.toml` until you fix or remove the file.

When Ledgeline ignores something in either file, a notice in the bottom corner lists what it skipped and why.

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
| `theme` | `"default"` | The colour theme. A family name follows your system's light or dark setting: `"default"`, `"github"`, `"catppuccin"`, `"gruvbox"`, `"tokyo-night"` or `"solarized"`. A variant always looks the same: `"default-light"`, `"default-dark"`, `"github-light"`, `"github-dark"`, `"catppuccin-latte"`, `"catppuccin-mocha"`, `"gruvbox-light"`, `"gruvbox-dark"`, `"tokyo-night-day"`, `"tokyo-night-night"`, `"solarized-light"`, `"solarized-dark"`, and the dark-only `"one-dark-pro"`, `"dracula"`, `"monokai"` and `"nord"` |
| `uiScale` | `100` | Interface scale in percent, 50 to 300. The app steps through 70 to 200 |
| `autoPrices` | `false` | Keep stock prices up to date in the background. See [Automatic price updates](../holdings/prices.md#automatic-price-updates) |
| `autoPricesWeeks` | `6` | 0 to 52: how many past weeks get a weekly closing price filled in. 0 means the latest close only |

## Keeping view settings in the browser

Set `LEDGELINE_UI_PREFS_IN_BROWSER` to any non-empty value when you start `ledgeline --server`, and the server reads and writes no view-settings file at all. It answers the built-in defaults, and the web app keeps its view settings in the browser's storage, as versions before these files did. Ledgeline's own automated tests run this way, so a test never changes your `ui-prefs.toml` and each browser session starts clean. It also suits embedding the server somewhere that should leave no files behind.

## Upgrading from an earlier version

Earlier versions kept the hledger path in `prefs.json`. The first time Ledgeline starts and finds `prefs.json` but no `prefs.toml`, it writes `prefs.toml` with the same path and renames the old file to `prefs.json.migrated`. Nothing is deleted.

Earlier versions also kept view settings in the browser's storage, which the desktop app lost every time it restarted. The first time Ledgeline finds view settings still in the browser's storage, it saves them the usual way, which normally means `ui-prefs.toml`, then removes them from the browser. A key the open ledger's `ledgeline.toml` sets is left out, because that ledger's own value wins anyway. If the save fails, the settings stay in the browser and are tried again next time. The address and access token of a separate Ledgeline server stay in the browser, because they describe your browser, not your books.
