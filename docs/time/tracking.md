---
title: Tracking time
description: "The Track page in Ledgeline Invoicer: ten slots with one key each, the running light in the sidebar, adjusting and noting time, today's card, the long-running check, keys on macOS, Linux and Windows, and the time command line for Raycast, Alfred and Stream Deck."
order: 10
tags: [invoicer, time]
---

# Tracking time

Track is a row of ten numbered slots, one per account you work on. Press a slot's number and its clock starts. Press another and the first one stops as the second starts, so only one clock ever runs. Press the running one again and it stops.

The idea comes from an old accountant's job timer: a card with a client written next to each button, a lamp on the button that was running, and a printed slip of the day's totals. Starting costs one key, and correcting a mistake costs one more.

Open it from the sidebar (**Time → Track**) or with <kbd>g</kbd> <kbd>r</kbd>.

## The slots

| Slot          | What's in it                                                                                     |
|---------------|--------------------------------------------------------------------------------------------------|
| <kbd>1</kbd>–<kbd>5</kbd> | Your pins: the accounts you choose.                                                  |
| <kbd>6</kbd>–<kbd>9</kbd> | The accounts you used most recently that aren't pinned, under a **recent** divider.  |
| <kbd>0</kbd>  | **Other…**: a search over every time account, with **Create account…** at the bottom. The account you choose there stays in slot 0 until you choose another. |

Each slot shows its account, its time today, and either when it started (if it's running), its latest note, or the day you last used it.

The account's name in a slot is the customer's name, if the account has one, then the last part of the account: `Acme Corp · tax return` for `clients:acme:tax-return`.

To pin an account, select slot 1–5 and press <kbd>e</kbd> (or **Change pin**), then choose the account. The same picker clears a pin. An empty pin is filled from your recent accounts, so all ten keys always do something.

> [!TIP]
> Pin the accounts you use every week. Until you do, slots 1–5 fill from your recent accounts too, and the recent accounts shift every time you switch, so a number doesn't always mean the same account.

Recent accounts come from your own time files, so there's nothing extra to keep. Your pins and slot 0 are saved for these books only.

## Starting, switching and stopping

| Key                       | Does                                                                                         |
|---------------------------|----------------------------------------------------------------------------------------------|
| <kbd>1</kbd>–<kbd>9</kbd> | Start that slot, stopping whatever runs. On the running slot, stop it.                       |
| <kbd>0</kbd>              | The same for slot 0. With nothing in slot 0 yet, open the account search.                    |
| <kbd>.</kbd>              | Stop the clock.                                                                              |
| <kbd>↑</kbd> / <kbd>↓</kbd> | Select the slot above or below. <kbd>Enter</kbd> or <kbd>Space</kbd> presses it.            |

Clicking a slot does the same as its key.

When a clock stops, the time goes into today's line for that account in your tracker file, rounded to the nearest minute, half up: 42 min 30 s is `43m`, 42 min 29 s is `42m`. A press that lasted less than 30 seconds records nothing, since it was most likely a slip.

Time adds to the account's **open line** for today: the first line for that account today that isn't on an invoice or bill. A line that's been billed is never changed. New time goes on a new line under it.

While a clock runs, the tracker file holds a comment line under today's date that says so:

```timedot
2026-11-26 Thu  ; person: pat
    clients:acme:advisory             1h  ; call with Fred about the K-1
    internal:admin                   15m
    ; ● running clients:acme:tax-return  since 15:00:00
```

hledger ignores the comment. Because the running state is in the file, it survives the app quitting, a crash, a restart, and a second computer reading the same folder. The other computer shows the clock running too. Stopping replaces the comment with the minutes.

A clock running past midnight is split at 00:00 when it stops: the minutes before midnight go to the first day, the rest to the next, in next month's file if the month changed.

## Adjusting and noting

| Key                       | Does                                                                                                     |
|---------------------------|----------------------------------------------------------------------------------------------------------|
| <kbd>+</kbd> (or <kbd>=</kbd>) | Add a step (5 minutes) to the selected slot's time today. On the running slot, start it a step earlier instead. |
| <kbd>-</kbd>              | Take a step off. On the running slot, start it a step later, but never later than now.                     |
| <kbd>_</kbd> (<kbd>Shift</kbd>+<kbd>-</kbd>) | Start the running clock where your previous entry ended, for "I started on this 20 minutes ago". |
| <kbd>n</kbd>              | Write a note on the selected slot's line today.                                                           |

The buttons under the slots do the same: **− 5m**, **+ 5m**, **Note** and **Stop**. A slot's time never goes below zero. The step is a setting (`step` in [Settings](formats.md#settings)).

<kbd>_</kbd> needs to know when you last stopped. In a timedot file that's the last stop the app saw today, since timedot lines have no times of day. In timeclock mode it's the end of your latest session.

A note becomes the line's comment, and later the *Work* column of the invoice's timesheet. The note field warns as you type when hledger would misread the text:

- a `word:` is read as a tag, so `re: K-1` would become a tag named `re`;
- a `;` or a line break ends the comment early.

It offers a rewording that reads back the same (`re K-1`, `10.30` for `10:30`), and you choose. Enter saves, Escape cancels.

## Today's card and the week

Above the slots, a strip shows each day of this week, Monday to Sunday, with its total. With a weekly target set (`weekly-target` in [Settings](formats.md#settings)), it shows the week against the target too.

<kbd>c</kbd> (**Today's card**) shows today's totals per account, with their notes, as a compact card. **Copy** puts it on the clipboard as text, and **Print** prints the card on its own.

## The light in the sidebar

At the top of the sidebar, on every screen:

| State   | Shows                                                                                           | Click   |
|---------|-------------------------------------------------------------------------------------------------|---------|
| Running | A green ▶, the account, the running time, and its key (<kbd>Option</kbd>+<kbd>Cmd</kbd>+<kbd>2</kbd>) | Stops it  |
| Stopped | A grey ■, the last account, and the key that resumes it                                         | Resumes it |

Colour is never the only sign. The shape (▶ or ■) and the time show it too, and a screen reader reads "running" or "stopped". Before setup, the light is a **Set up time tracking** link, and with nothing tracked yet it reads **Not tracking**.

## Keys on every screen

These work anywhere in the invoicer, except while you're typing in a field:

| macOS                                                     | Linux and Windows                                          | Does                                   |
|-----------------------------------------------------------|------------------------------------------------------------|----------------------------------------|
| <kbd>Option</kbd>+<kbd>Cmd</kbd>+<kbd>1</kbd> … <kbd>0</kbd> | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>1</kbd> … <kbd>0</kbd> | Start or stop that slot                 |
| <kbd>Option</kbd>+<kbd>Cmd</kbd>+<kbd>.</kbd>             | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>.</kbd>              | Stop the clock                          |
| <kbd>Option</kbd>+<kbd>Cmd</kbd>+<kbd>↑</kbd>             | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>↑</kbd>              | Start the running clock a step earlier  |
| <kbd>Option</kbd>+<kbd>Cmd</kbd>+<kbd>↓</kbd>             | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>↓</kbd>              | Start it a step later                   |

A slot key on an empty slot opens Track with the account search, so you can fill it.

The keys match the physical digit keys, so they work on any keyboard layout. They aren't the obvious <kbd>Cmd</kbd>+<kbd>Shift</kbd>+digit on macOS because macOS keeps <kbd>Cmd</kbd>+<kbd>Shift</kbd>+<kbd>3</kbd> to <kbd>6</kbd> for screenshots. On Linux and Windows, <kbd>Ctrl</kbd>+<kbd>Alt</kbd> is AltGr on many European layouts and types characters such as `{` and `@`, so the invoicer leaves it alone.

> [!NOTE]
> These keys work while the invoicer's window has focus. To drive the tracker from anywhere, use the [command line](#the-command-line) from a launcher.

Press <kbd>?</kbd> for the full list. Track's own keys are under **Track**, and the ones that work everywhere under **Time**.

## A clock left running

When you open the app, or come back to its window, a clock that has run longer than four hours, or since an earlier day, asks **Still working on …?**

| Choice                   | Does                                                                                    |
|--------------------------|-----------------------------------------------------------------------------------------|
| **Keep all**             | Leaves it running, and doesn't ask again for this start.                                 |
| **Stop at last activity** | Stops it when your last session ended. Shown only when that's known (timeclock mode). |
| **Stop at…**             | Stops it at a date and time you pick.                                                    |
| **Discard**              | Throws the session away. Nothing is recorded.                                            |

The four hours are a setting (`long-running`, in minutes, in [Settings](formats.md#settings)); `0` turns the check off.

## The command line

`ledgeline-invoicer time` drives the same tracker from a terminal or a script. It uses the same pins, slot 0 and name as the app, so `time toggle 2` toggles the slot 2 the app shows. Every command takes `--json`. See [Command line](../invoicer/cli.md) for how it finds your books.

| Command                         | Does                                                                                 |
|---------------------------------|--------------------------------------------------------------------------------------|
| `time status`                   | What's running, the ten slots and today's totals                                      |
| `time toggle SLOT`              | Start or stop a slot (`1`–`9`, `0`) or an account                                     |
| `time start ACCOUNT`            | Start timing an account, stopping whatever runs                                       |
| `time stop`                     | Stop. `--at YYYY-MM-DDTHH:MM:SS` stops at that moment; `--discard` records nothing     |
| `time adjust SLOT [MINUTES]`    | Add minutes (negative to take them off); default the step                              |
| `time note SLOT NOTE`           | Set the note on a slot's (or an account's) line today                                  |
| `time log`                      | List time lines: `--from`, `--to`, `--customer`, `--account`, `--billed`, `--all` for everyone's |
| `time setup`                    | Set up time tracking; `--adopt FILE` uses an existing file, `--dry-run` shows what it would create |
| `time unbilled`, `time unpaid`, `time bill`, `time pay`, `time release` | Billing: see [Billing time](billing.md#from-the-command-line) |

Two options work with every `time` command. `--person NAME` acts as someone else (default: your name from the app's settings), and `--now YYYY-MM-DDTHH:MM:SS` treats that moment as now.

```console
$ ledgeline-invoicer time toggle 3
Started clients:acme:tax-return
▶ running  Acme Corp · tax return  0m  since 2026-11-26T15:00:00
Today 2026-11-26: 1h 15m
  1 ○ Internal · admin                     15m
  2 ○ Acme Corp · advisory                 1h
  3 ▶ Acme Corp · tax return               —
  ...
$ ledgeline-invoicer time stop
Stopped clients:acme:tax-return (43m)
```

The app picks up the change at once, light included.

### Driving it from Raycast, Alfred or a Stream Deck

Each command is one quick run with no window, which makes it easy to bind to a key or a button anywhere on your computer:

- **Raycast**: make a script command per slot that runs `ledgeline-invoicer time toggle 1`, and give it a hotkey.
- **Alfred**: a workflow with a hotkey trigger and a **Run Script** action running the same command.
- **Stream Deck**: a button per slot, each running the command through a shell-script action, plus one for `time stop`.
- **macOS Shortcuts**: a **Run Shell Script** action.

Give the command your books' journal (`-f ~/books/books.journal`), since a launcher doesn't start in your books folder. For a status line or a menu-bar widget, `time status --json` has the running clock under `running`, or `null` when nothing runs:

```sh
ledgeline-invoicer time status --json -f ~/books/books.journal \
  | jq -r 'if .running then "▶ \(.running.label) \(.running.elapsed.text)" else "■" end'
```
