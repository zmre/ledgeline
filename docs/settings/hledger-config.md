---
title: hledger config
description: "Settings → hledger config: the hledger.conf file hledger reads for every command, and the two things Ledgeline takes from it — automatic postings (--auto) and each report's starting depth (--depth)."
order: 30
tags: [reports]
---

# hledger config

hledger reads one config file for every command you run in a terminal. Settings → hledger config shows which file that is for your books. It also edits the two settings in it that Ledgeline uses:

- **Apply automatic postings (`--auto`).** When this is on, the journal's `= QUERY` rules add postings to the transactions they match.
- **Report depth (`--depth`).** This sets how deep each report starts. You can set one depth for all reports and override it per report or per account.

The file is the setting. Ledgeline has no separate preference for either, so the terminal and the app cannot disagree.

## Which file is in force

hledger looks for its config file in this order and uses only the first one it finds:

1. `hledger.conf` in your journal's folder, then in each folder above it;
2. `~/.hledger.conf`;
3. `~/.config/hledger/hledger.conf`, or the same path under `$XDG_CONFIG_HOME` if you set that variable.

Ledgeline searches the same way, starting from your main journal's folder. The tab shows the file it found as a short name such as `hledger.conf`, `../hledger.conf` or `~/.hledger.conf`.

If none of the three exists, the tab says "none". Saving then creates `hledger.conf` beside your main journal, with a short comment at the top explaining what the file is.

Steps 2 and 3 apply to the desktop app only. A journal in a sandboxed folder, such as on iOS, has no home directory, so the search stops at the folder you opened.

## What saving changes

Saving edits only the lines for the settings you changed. Comments, blank lines, line endings and any options Ledgeline does not handle stay exactly as they were. When an option is already there, Ledgeline rewrites that line in place. When it is not, Ledgeline adds a line to the right section, and creates the `[command]` heading at the end of the file if the file has no such section yet.

Saving does nothing in these cases, and the tab tells you why:

- **The file changed on disk since the tab read it.** Use **Reload and discard my changes** and make the change again.
- **The first word of the file is not an option.** hledger reads a first word such as `balance` or `print` as the command to run, whatever you type. Every hledger command is then affected, and none of the file's options apply. Remove that word, or make it a `#` comment.
- **The file is a symbolic link.** Dotfile managers often create these. Edit the file the link points to.
- **The setting can't be written in a config file.** A config file has no quoting, so an account pattern can't contain a space; use `.` to match one. It also can't contain `#`, `=`, or a leading `-`, and it must be a valid regular expression.
- **Ledgeline was started without a journal open for editing.**

After a save, every report reloads with the new settings. Ledgeline also notices when you edit the file yourself.

## Automatic postings

The checkbox writes `--auto` into the file's general section, the part before any `[command]` heading, or removes it from there. An `--auto` inside a command section applies to that command only, and Ledgeline does not read it as this setting.

## Report depth

There is one depth box for the general section and one for each report:

| Section | Report in Ledgeline | What the depth does |
| --- | --- | --- |
| general | all of them | Applies to every report below that does not set its own depth |
| `[balancesheet]` | Balance Sheet | Sets which groups start expanded |
| `[incomestatement]` | P&L | Sets which groups start expanded |
| `[cashflow]` | Cash Flow | The default of the tab's depth control |
| `[balance]` | Net Worth | The default of the tab's depth control |

hledger reads a section only when its heading is the command's full name, spelled exactly as above. hledger silently ignores `[bs]`, `[is]`, `[cf]`, `[bal]` and `[BalanceSheet]`, and the tab warns you when it finds one. If the same heading appears twice, hledger reads only the last one, and the tab warns about that too.

A report's own section overrides the general depth. Per-account depths from both sections add up.

**Per-account depths** (`--depth ACCOUNTREGEX=N`) set the depth for the accounts whose names match the pattern. The match is case-insensitive and can be anywhere in the name. If several patterns match one account, the one that matches furthest to the right in the name wins. For `assets:inv:broker:fund`, `broker=3` beats `inv=4`.

### Cash Flow and Net Worth

The section's plain depth is where the tab's depth control starts. A link that includes its own `depth=` keeps that depth. You can still change the depth on the report; Ledgeline applies the configured one again only when you next open the tab, or when the file changes. These tabs have a single depth control, so per-account depths in these sections apply to hledger in the terminal but not to Ledgeline.

### Balance Sheet and P&L

These reports have no depth control. Instead, the depth decides which groups start expanded. A group opens when hledger, at that depth, would show its accounts as more than one line. For example, with `--depth 2`, a "Cash" group holding `assets:cash:checking` and `assets:cash:savings` rolls up to `assets:cash` and stays collapsed. With `--depth 3` it opens. Per-account depths do the same thing for the accounts they match. An account no pattern matches has no limit when the section sets no plain depth, so its group opens.

With no depth configured, every group starts collapsed, as it always has. Collapsing a group the config opened keeps it collapsed when the report reloads.

## What Ledgeline does not do with this file

Ledgeline never hands the file to hledger. Every hledger command Ledgeline runs itself, such as an import, passes `--no-conf`. Ledgeline reads `--auto`, `--depth` and `--alias` from the file (see [Account aliases](account-aliases.md)) and leaves every other option to hledger in your terminal. The tab has no controls for options Ledgeline does not use.
