---
title: Settings
description: "The Settings screen: the Accounts editor for your chart of accounts, the Account Aliases editor that maps bank names onto your accounts, and the hledger config tab for --auto and report depths."
order: 70
tags: [account-tags]
---

# Settings

Open Settings with the gear button at the right end of the top bar. It has no `g` shortcut of its own.

Settings has four tabs. Press <kbd>1</kbd>–<kbd>4</kbd> to switch between them, or click the tab. The open tab is kept in the address as `?tab=accounts`, `?tab=aliases`, `?tab=hledgerConf` or `?tab=auto`, so a reload brings you back to the same one.

| Tab | What it edits | Page |
| --- | --- | --- |
| Accounts | The `account` directives in your journal: each account's type, the report tags Ledgeline reads, other tags, and a note | [Accounts](accounts.md) |
| Account Aliases | The `alias` directives in your journal, which map the names a bank uses onto your own account names during an import | [Account aliases](account-aliases.md) |
| hledger config | The `hledger.conf` file hledger reads for every command: whether automatic postings apply (`--auto`), and each report's starting depth (`--depth`) | [hledger config](hledger-config.md) |
| Automatic postings | Read-only: the `=` auto-posting rules in your journal, what each one added, and whether `--auto` is on | [Automatic postings](../journal/automatic-postings.md) |

Accounts and Account Aliases write to your journal files directly; hledger config writes to your `hledger.conf`. Each save checks that the file has not changed on disk since Ledgeline read it. If it has, nothing is written, and the screen offers a button to reload and discard your edit. No tab can be changed when Ledgeline was started without a journal open for editing. In that case the tab shows what the journal declares and says it is read-only.

Account Aliases used to be a tab on the [Imports](../imports/index.md) screen. An old bookmark to that tab now opens Settings → Account Aliases instead.
