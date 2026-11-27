---
title: Time file formats
description: "The timedot and timeclock files Ledgeline Invoicer writes and reads: what a line looks like, the writing rules, the hledger 1.52 pitfalls it avoids and warns about, and the [time] settings for file names, places, tag names and languages."
order: 50
tags: [invoicer, time]
---

# Formats

Your time is in hledger's own [timedot](https://hledger.org/dev/hledger.html#timedot) and [timeclock](https://hledger.org/dev/hledger.html#timeclock) formats. This page shows what the invoicer writes, what it reads, and which hand-written lines hledger reads differently from what they seem to say. You don't need it to track time in the app.

## Timedot

A timedot file is a list of days. Each day is a date line with the day's entries indented under it, one account and one amount per line. This is a tracker file as the invoicer writes it:

```timedot
# Time — pat — November 2026
# Written by Ledgeline; hand edits welcome. Keep two spaces before amounts.

2026-11-25 Wed  ; person: pat
    clients:acme:tax-return         130m  ; reviewed schedules C and E
    internal:admin                   15m

2026-11-26 Thu  ; person: pat
    clients:acme:tax-return          45m  ; invoice: 108
    clients:acme:tax-return          20m  ; call with Fred about the K-1
    clients:acme:advisory             1h
    ; ● running clients:acme:advisory  since 14:05:12
```

- **The date line** has the date, the weekday, and your `person:` tag. hledger makes each day one transaction, with the weekday as its description and the date line's tags on every entry of the day. So `hledger reg tag:person=pat` finds all of your time without a tag on each line. With no name set, the tag is left out.
- **Each entry** is an account, at least two spaces, and an amount. The amount is whole minutes (`45m`, `130m`), or whole hours when it's exact (`1h`, `2h`). The amounts line up in one column per day.
- **A comment** after the amount holds the note and the tags. Billing appends its tag after the note: `; reviewed schedules C and E, invoice: 108`.
- **The running line** (`; ● running …`) is a comment hledger ignores. It's there while a clock runs. See [Tracking time](tracking.md#starting-switching-and-stopping).
- **Two lines for one account on one day** are fine. hledger adds them up. The tracker adds new time to the account's first line of the day that isn't billed, and starts a new line when that one is on an invoice.

The invoicer only changes the lines it needs to. A line it doesn't touch stays byte for byte as it was, including your spacing and any amounts written another way. When it adds or changes a line, it lines that line up with the rest of its day.

### What the invoicer reads

Any timedot hledger reads, the invoicer reads too:

| You write              | Means                                                         |
|------------------------|---------------------------------------------------------------|
| `....`                 | Dots: a quarter hour each, so this is an hour. Spaces between them don't count. |
| `aabbc`                | Letters: a quarter hour each. Each letter is its own entry, tagged `t:a`, `t:b`, … |
| `1.5`, `1,5`           | Hours, with no unit.                                          |
| `90s`, `45m`, `2h`, `1d`, `1w` | Seconds, minutes, hours, days, weeks. No space before the unit. |
| `-15m`                 | A negative amount, to correct a day.                          |

Date lines can be written `2026-11-26`, `2026/11/26` or `2026.11.26`, and org-mode `*` headings around them are fine. A line starting with `#` or `;` is a comment.

The invoicer keeps exact time: `20m` is 20 minutes, not 0.33 hours.

## Timeclock

A timeclock file records sessions: a clock-in line and a clock-out line, each with a date and time.

```timeclock
; Time — sam — November 2026
; Written by Ledgeline; hand edits welcome. Keep two spaces after the account.

i 2026-11-26 09:00:05 internal:admin  ; person: sam
o 2026-11-26 10:30:17 internal:admin
```

- `i DATE TIME ACCOUNT  DESCRIPTION  ; comment` starts a session. The description is optional, and two spaces separate it from the account.
- `o DATE TIME ACCOUNT  ; comment` ends it. hledger allows a bare `o`, which ends the latest session.
- Comments on the `i` line, on lines indented under it, and on the `o` line all become tags of the session. The invoicer puts billing tags on the `o` line.
- An `i` with no `o` yet is a session still running. hledger counts it up to now.

The invoicer reads timeclock files anywhere the roll-up includes them, and edits a session by its start and end in [Entries](entries-and-accounts.md#changing-and-deleting).

### Tracking in timeclock

With `record = "timeclock"` in your [settings](#book-settings), the tracker writes timeclock instead of timedot, to `time/<name>/YYYY-MM.timeclock`. Each start writes an `i` line and each stop its `o` line, both with seconds. Nothing is rounded, since the session is the record, and billing adds up the exact times before any [rounding](billing.md#rounding) the customer asks for.

There's no running line in timeclock: the open `i` line is what says a clock runs. The tracker can't write notes in this mode. Write the session's comment by hand.

## Writing rules

These are the rules the invoicer follows when it writes, and the ones to follow when you edit by hand:

- **Two spaces or more** between the account and the amount.
- **No tabs**, and no spaces inside a number.
- **No `mo` or `y` units.** Write `30d`, or hours.
- **Tags and notes on the entry's own line**, after `;`.
- **A note can't contain `;`, a line break, or a word followed by a colon.** A `;` starts a new comment, and hledger reads `re: K-1` as a tag named `re`. Write `re K-1`, or `10.30` for a time of day. The app's note field warns and offers that rewording.
- **Account names** can't contain `;`, a tab or two spaces in a row, and can't start with `*`, `#`, `(`, `[` or something that looks like a date.

## Lines hledger misreads

These lines don't mean what they seem to in hledger 1.52. The invoicer never writes them, and Entries warns about them in files you write by hand:

| You write                        | hledger 1.52 reads                                                                    | The invoicer reads |
|----------------------------------|----------------------------------------------------------------------------------------|--------------------|
| `clients:acme 30m` (one space)   | An account named `clients:acme 30m` with **0** hours. A single tab does the same.        | The same, with a warning |
| `internal:admin  1 2`            | `1 2` as a number with a digit-group space: 12 hours.                                   | The same, with a warning |
| `internal:sales  1mo`            | 5040 hours: 1.52's `mo` and `y` are seven times too big.                                  | 30 days (`y`: 365), with a warning |
| `clients:globex  20m`            | 0.33 hours: 1.52 rounds each unit amount to 0.01 hour. hledger 1.99.6 drops the rounding. | 20 minutes exactly |
| `internal:admin` with no amount  | 0 hours.                                                                                 | The same, with a warning |
| `b`, `h` or `O` timeclock lines with text | A parse error, though the manual allows them.                                   | Ignored, with a warning |

The rounding is why hledger's totals can be off by a few hundredths of an hour from the invoicer's: three `2m` lines are 0.09 hours in hledger and 0.10 in the invoicer. Invoices never use hledger's figure.

> [!CAUTION]
> In hledger 1.52, an indented `;` line under an entry attaches to that entry: `; invoice: 108` on the line after `clients:acme  30m` tags it. The invoicer reads an indented line as a plain comment, so the two would disagree on whether the time is billed. Keep tags on the entry's own line. An indented `#` line, hledger ignores.

A few more facts about how hledger reads these files:

- A timedot or timeclock file can't hold directives such as `account` or `commodity`. The roll-up journal that includes it can.
- `include` needs the extension (`.timedot`, `.timeclock`) or a `timedot:` / `timeclock:` prefix. Globs work, `**` included: `include time/**/*.timedot`.
- In timeclock, the same account clocked in twice at once is an error. Two sessions open on different accounts are fine. A session over midnight is split at 00:00.
- With hledger's strict mode (`-s`), the roll-up must declare `commodity ""` and every account, and `commodity h` too if you have timeclock files. The roll-up setup writes declares `commodity ""` only. See [Strict mode](hledger.md#strict-mode).

## Settings

### Book settings

The names and places of the time files, and the words written into them, belong to the books. Set them under `[time]` in the books' `ledgeline.toml`. Every key is optional:

```toml
[time]
# The roll-up journal. Default: time.journal
file = "time.journal"
# The time account tree. Default: time-accounts.journal
accounts-file = "time-accounts.journal"
# Where the tracker writes. Default: time/{person}/{yyyy}-{mm}.timedot
track-file = "time/{person}/{yyyy}-{mm}.timedot"
# What the tracker writes: "timedot" (default) or "timeclock".
record = "timedot"
# The weekday on date lines: "short" (default) or "none".
weekday = "short"

[time.tags]
# Other names for the tags, per role.
invoice = "rechnung"

[time.words]
# Words written into new files.
weekdays = ["Mo", "Di", "Mi", "Do", "Fr", "Sa", "So"]
months = ["Januar", "Februar", "März", "April", "Mai", "Juni",
          "Juli", "August", "September", "Oktober", "November", "Dezember"]
clients = "kunden"
internal = "intern"
admin = "verwaltung"
title = "Zeit"
```

| Key             | Means                                                                                                                              |
|-----------------|------------------------------------------------------------------------------------------------------------------------------------|
| `file`          | The roll-up: a `.journal`, or a single `.timedot` or `.timeclock` file that's the whole time book. **Use existing…** at setup writes this. |
| `accounts-file` | The time account tree.                                                                                                             |
| `track-file`    | A pattern for the tracker's files, with `{person}`, `{yyyy}`, `{mm}` and `{dd}`. With no person, a folder that's only `{person}` is left out. One fixed file, such as `time.timedot`, works too: the tracker adds its days there. The default is `time/{person}/{yyyy}-{mm}.timeclock` when `record` is `timeclock`. |
| `record`        | `timedot` writes rounded minutes per day; `timeclock` writes `i`/`o` sessions with seconds.                                       |
| `weekday`       | `none` leaves date lines with the bare date.                                                                                       |
| `[time.tags]`   | A new name for any of `customer`, `billable`, `item`, `archived`, `person`, `invoice`, `bill`, `nobill`. The key is the default name. |
| `[time.words]`  | `weekdays` (seven, Monday first) and `months` (twelve) for date lines and file headers; `clients`, `internal` and `admin` for the accounts setup seeds; `title` for the first word of a tracker file's header. |

Paths are relative to the journal's folder, and must stay inside it. A value that isn't usable is ignored, and the default applies.

When the invoicer writes a new time file the roll-up doesn't include yet, such as the first file in a new folder, it adds the `include` line the roll-up needs. It only adds lines, and leaves the ones you wrote alone.

> [!NOTE]
> If you rename `invoice`, the books still use `invoice:` on the invoice's booking. `hledger reg tag:invoice=108` then finds the booking but not the hours. See [Settings files](../settings/settings-files.md) for how `ledgeline.toml` works.

### Your own settings

Your own choices are kept with your other view settings, under `[time]` in `ui-prefs.toml`:

| Key             | Default | Means                                                                                  |
|-----------------|---------|----------------------------------------------------------------------------------------|
| `person`        | none    | Your name in the time files. Set at setup, or with **Change** on Track.                |
| `step`          | `5`     | Minutes that <kbd>+</kbd> and <kbd>-</kbd> add or take off.                              |
| `long-running`  | `240`   | Minutes before a running clock asks whether it was left on. `0` turns it off.          |
| `weekly-target` | `0`     | Hours a week, drawn on Track's week strip and the dashboard's chart. `0` is no target. |

```toml
[time]
person = "pat"
step = 10
weekly-target = 35
```

Your pins and slot 0 are kept for each set of books, and you set them on Track. The `time` command line reads the same file, so the app and your scripts share your name, your pins and your slot 0.
