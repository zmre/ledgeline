---
title: Settings
description: "The Settings screen: the Accounts editor for your chart of accounts, and the Account Aliases editor that maps bank names onto your accounts."
order: 70
tags: [account-tags]
---

# Settings

Open Settings with the gear button at the right end of the top bar. It has no `g` shortcut of its own.

Settings has two tabs. Press <kbd>1</kbd> or <kbd>2</kbd> to switch between them, or click the tab. The open tab is kept in the address as `?tab=accounts` or `?tab=aliases`, so a reload brings you back to the same one.

| Tab | What it edits | Page |
| --- | --- | --- |
| Accounts | The `account` directives in your journal: each account's type, the report tags Ledgeline reads, other tags, and a note | [Accounts](accounts.md) |
| Account Aliases | The `alias` directives in your journal, which map the names a bank uses onto your own account names during an import | [Account aliases](account-aliases.md) |

Both tabs write to your journal files directly. Each save checks that the file has not changed on disk since Ledgeline read it. If it has, nothing is written, and the screen offers a button to reload and discard your edit. Neither tab can be changed when Ledgeline was started without a journal open for editing. In that case the tab shows what the journal declares and says it is read-only.

Account Aliases used to be a tab on the [Imports](../imports/index.md) screen. An old bookmark to that tab now opens Settings → Account Aliases instead.
