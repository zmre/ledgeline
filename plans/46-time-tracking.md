# 46 — Time tracking: timedot and timeclock, the slot tracker, unbilled time

Ledgeline gains time tracking that is plain hledger from end to end. Time
lives in `.timedot` (and, when read, `.timeclock`) files beside the books,
under a separate tree of **time accounts** that are not money accounts.
Account tags link a time tree to a customer, mark it billable or not, and
suggest what it bills as. Entry tags record which invoice (or which payable
bill) an entry went into. The everyday tool is a **slot tracker** modelled on
an old accountant's multi-button job timer: ten numbered slots, one keystroke
to switch, a visible "recording" light everywhere in the app, and +/−
buttons for corrections. The billing tool is an **Unbilled** view that turns
time into invoice drafts (Invoicer) or contractor bills (accounts payable).

Nothing about time reaches the money journal unless the user includes it.

> Status: **reviewed by Patrick 2026-11-26**; decisions T1–T18 settled, no
> open questions. Nothing is built.

## The ask (Patrick, 2026-11-26)

> Timedot and timeclock support including: accounts management; linking time
> accounts to customers; tagging billable or not on accounts; tagging billed
> or not on entries (and which invoice); surveys and summaries of where time
> was spent; quick time logging and editing.

The tracker, in his words:

> I want to emulate an old device … a place to write down up to 5 clients on
> a piece of paper and there was a button next to each. You'd hit the button
> to start the clock for that client and a light would show it on. Then
> either hit the button again to stop it or hit another button to stop the
> previous and start a different one. He'd write notes to indicate +5 or −5
> and to note specifics … It would print a tag with totals that would pair
> with the card … I'd let the user set 5 top choices and for 6–10 do most
> recents that aren't in the top 5. When on the screen, a simple 1–9, 0 would
> toggle one. Buttons for +/− for making adjustments. Pick something else in
> the last spot. When not on that screen … something like cmd-shift-1 … and
> on any screen we should show what's currently being recorded … in the
> sidebar with a red/green (and maybe symbols for the color blind). Show most
> recent and hint for what number it is to restart.

Further requirements from the ask:

- Read **and** write timedot and timeclock, hledger-compatible. Our own
  tracking writes per-day counts in `1m` / `1h` style, not dots (dots are
  15-minute units).
- Time postings are commodity-less, unbalanced virtual postings that hledger
  sums. Hours are **not** added to the main journal by default; users may
  include them.
- Three pillars: (1) a separate time-account tree with roll-ups, probably in
  `time-accounts.journal`; (2) linking time trees to customers, with billing
  that can split across people, functions and rates, so the link to the
  catalog is ad hoc but remembered and suggested; tracking what is billed
  and what is billable is key; (3) reporting across billable and
  non-billable time, including unbilled hours, with help on which invoices
  to create and what period to bill.
- Several people contributing timesheets. Some timesheets are **self-bills**:
  an hourly contractor's time becomes accounts payable, not receivable.
- Defaults: a separate time-accounts file, not linked from the main journal;
  a separate roll-up file that includes the timedot/timeclock files; our own
  tracking in files named by date, ideally with a username, suggested at
  setup and editable on the tracking screen.
- Navigation "Time Tracking": Dashboard, View/Edit/Add (leading to invoices
  or payable bills), Track, and shortcuts to all of them.
- **Tags are the mechanism** for linking and tracking.
- Files we write are as pretty as possible: indents under dates, account
  trees laid out readably. The plain text must stay readable, editable and
  usable outside our UI.

## Research summary

The full notes are in the three research reports from 2026-11-26 (not
checked in). The findings that shaped this plan:

**The device.** The closest documented match is US patent 4,340,808,
*Time logging apparatus* (Donohoo, filed 1979). It has one key per account in
a row, each with a lamp; the last key pressed is charged; there's an IDLE
account for unassigned time; a removable card labels the keys. A sibling
patent (US 4,164,038) adds the printer. Neither brand name was found. What
made it good maps straight onto software: **one press switches** (no
stop-then-start), **exactly one lamp lit**, idle time visible instead of
lost, a small fixed set of slots that becomes muscle memory, and a paper
record at the end of the day.

**Products** (Toggl, Harvest, Clockify, Tyme, Timing, Timely/Memtime, EARLY,
klog, Timewarrior, Clio, Karbon/TaxDome):

| Idea                                                                    | Seen in               | Where it lands here      |
|-------------------------------------------------------------------------|-----------------------|--------------------------|
| Start X implicitly stops Y; resume-last                                 | Timewarrior, Toggl    | T-slots                  |
| Always-visible running timer with elapsed time and day total            | Tyme, Toggl menu bar  | T-light                  |
| Nudge the running timer's start: −1/−5/−15, **snap to end of last entry** | Tyme                | T-adjust                 |
| Idle/sleep return dialog: keep, discard, discard-and-continue, reassign | Toggl, Tyme           | Phase 6 (T-idle)         |
| Uninvoiced report → one click per client → draft invoice, entries locked | Harvest              | B-unbilled, B-invoice    |
| Approved ≠ locked ≠ invoiced, three separate states                     | Harvest, Clockify     | B-states                 |
| Rounding applied in reports/invoices, never to stored time              | Clockify, Tyme, Clio  | B-round                  |
| Rate hierarchy resolved then **frozen** on the bill                     | Clio, Tyme            | B-rates                  |
| Plain-text day records with open ranges as running timers               | klog                  | T-running                |
| Draft-then-commit for reconstructed time                                | Timely, Memtime       | Phase 6                  |

**Design awards.** No time tracker has a documented Apple Design Award, Red
Dot or Webby; most "award-winning" claims in this market are vendor copy.
Timery (a Toggl client) is the one with real recognition (MacStories Selects
2019 and 2022), for pinned timers and one-tap starts. What reviewers praise
across Timery, Tyme and Timing: **starting costs one action; state is always
visible; correcting costs less than being careful; local and quiet.**

**Literature.**

- People stop self-tracking because they forget, because upkeep costs too
  much, or because they skip on purpose (Epstein et al., UbiComp 2015 and
  CHI 2016). So: make the return after a gap cheap, and show untracked time
  without nagging.
- The widely quoted "same day loses 10%, next day 25%, after a week 50%"
  figures have **no primary source we could find**. Treat them as
  directional.
- **ABA Formal Opinion 93-379**: bill no more than the time spent, except
  rounding up to a stated minimum increment. Rounding each tiny entry
  inflates bills (three 2-minute emails become 0.3 h rather than 0.1 h).
  Store the actual time, apply rounding at a chosen scope, and show both
  values. Don't let one hour be billed to two clients, so flag overlaps.

## What hledger 1.52 actually does

Verified on 2026-11-26 against scratch files in `/tmp/tdtest` and
`/tmp/p46`, with hledger 1.52 from the dev shell. Facts that change the
design:

1. **Timedot entry = `account␣␣amount`.** The two spaces are required. With
   one space or one tab, the amount becomes part of the account name and the
   posting is silently 0. One quantity per line. A space inside a number is
   a digit-group mark (`1 2` = 12). Units are `s m h d w mo y`, with no space
   before the unit.
2. **1.52 rounds unit amounts to 0.01 h** (`20m` → 0.33). It does not round
   unitless decimals. 1.99.6 removes the rounding. **1.52's `mo`/`y` units
   are 7× too big**: never write them.
3. **One transaction per date line**, cleared, with the date line's text as
   its description and the date line's tags as transaction tags. Each entry
   is a virtual posting `(acct)` with no commodity, or the parent's `D`
   commodity if one applies. **Duplicate account lines in a day are separate
   postings. Duplicate and unsorted dates are fine.**
4. **Entry tags work:** `acme:dev  45m  ; invoice:108` matches
   `tag:invoice=108`. Date-line tags match on every posting of that day. An
   *indented* `; …` line under an entry is a comment and **is dropped**: it
   does not attach to the entry.
5. **No directives in timedot files.** A `.journal` that includes them
   provides `account`, `alias`, `apply account`, `D`/`commodity`; all of
   these were verified to apply.
6. **`include` needs the extension** (`.timedot`, `.timeclock`) or a
   `timedot:` prefix. **Globs work, including `**`**:
   `include time/**/*.timedot` was verified.
7. **Account tags are inherited, and can't be overridden for queries.**
   With `clients:acme ; billable: yes` and `clients:acme:travel ;
   billable: no`, `tag:billable=yes` **also matches travel**. `--pivot
   billable` resolves correctly (nearest declared value). Also,
   `billable: no` matches a bare `tag:billable`. So the documented hledger
   recipe is `hledger bal --pivot billable`, and Ledgeline resolves
   nearest-ancestor itself.
8. **Indented `account` directives are silently ignored** (`  account a:b`
   declares nothing). So account trees in the accounts file can't use
   indentation; they're laid out by ordering and blank lines (F-accounts).
9. **Timeclock** (1.50+ syntax):
   - `i DATE TIME ACCOUNT␣␣DESCRIPTION ;comment` and `o DATE TIME [ACCOUNT] ;comment`.
   - Comments on the `i` line, on indented lines under it, and on the `o`
     line all become **transaction** tags.
   - A bare `o` closes the most recent open session.
   - Two open sessions on **different** accounts are fine; the **same**
     account twice is an error in 1.52.
   - A session still open at end of file is clocked to *now*.
   - Sessions split at midnight. Pairing is per file.
   - Output is `(acct)  1.50h` in commodity `h`, rounded to 0.01 h in 1.52.
   - `b`/`h`/`O` lines with free text fail to parse in 1.52 (the manual says
     they should).
10. **Strict mode** (`-s`) needs `commodity ""` declared (with a `D`, that
    commodity declared instead) and every account declared, both in the
    including journal. **Timeclock hours are commodity `h`**, which needs
    `commodity h` too: without it `hledger -s` refuses the file. Setup
    writes it for a timeclock tracker, the first timeclock include adds it,
    and adopt reports it missing.

## What is true today (in the code)

From the 2026-11-26 code survey:

- **Own parser.** `ledgeline-core/src/parse.rs` parses journals itself; the
  hledger binary runs only in `ledgeline-server/src/hledger.rs` (imports).
  `include` is supported (`parse.rs:772`) but takes the path literally: **no
  globs, no format prefixes, no timedot or timeclock reader**. An included
  `.timedot` would fail to parse today, so a user who includes time in their
  main journal currently breaks Ledgeline.
- **Round-trip writing.** `edit.rs` has `JournalEditor` with per-file
  fingerprints, verify-by-reparse, an atomic write, and refusal when a file
  changed on disk since it was read. It is journal-syntax only.
- **Sibling files.** The recognition file (plan 45) is the model to follow:
  a `[books]` key with a default name (`accrual/book.rs:90`), explicit
  creation (`ops.rs:527`), and added to the watcher by hand because the main
  journal doesn't include it (`ledgeline-server/src/lib.rs:891`).
- **Navigation.** Ledgeline has **no sidebar**: its sections are header tabs
  (`web/src/routes/+layout.svelte:99`). **The Invoicer has the sidebar**
  (`invoicer/web/src/lib/components/Sidebar.svelte`), with customers,
  invoices and the catalog.
- **Keys.** `packages/ui/src/keys` is shared by both apps: layers,
  priorities, `g x` chords, `?` help, `mod+`. **Nothing binds ⌘⇧digit, and
  `chord.ts` drops Shift on printable keys**, so on a US layout ⌘⇧1 arrives
  as `meta+!`. A digit binding with Shift needs the physical `event.code`.
- **No OS-global hotkey and no tray/menu-bar icon.** Neither crate is in
  `Cargo.lock`. The shell is tao/wry/muda.
- **Invoicer.** Customers are markdown notes (`invoicer-core/src/customers.rs`)
  identified in the books by `ledger_customer` (the `customer:` tag). The
  catalog is a table with `code`/`unit`/`rate`/`books`
  (`catalog.rs`). Invoice rows are `DraftRow` (`draft.rs:44`).
  `POST /api/invoices` creates an empty draft and rows go in afterwards; the
  CLI `new --row` takes rows directly. Numbers are max+1. Supporting tables
  (timesheets) beside the bill table are already allowed and left alone
  (ARCHITECTURE §3.3).
- **AP bills exist in core but nothing calls them.**
  `accrual::ops::write_document` with `Direction::Expense` books a bill
  against a payable account (`accrual/invoice.rs:51,109`). Its only caller
  today is the Invoicer, for revenue.
- **Tag constants.** `accrual/mod.rs` already defines `customer`, `vendor`,
  `invoice`, `reference`, `document` and others.
- **Prior sketch.** Invoicer ARCHITECTURE §7 already reserves "Time
  tracking": account prefix maps to customer and project, rate rules,
  indented `#` lines become row detail, an unbilled view, billed entries
  tagged `invoice:` in the timedot file. Invoicer PLAN Phase 7 item 4. This
  plan supersedes that row; the `#`-detail idea is replaced by entry
  comments (fact 4: indented lines are dropped by hledger anyway).

## Decisions (Patrick, 2026-11-26)

Patrick reviewed the draft in mbr. He accepted T1, T2, T5–T8 and T10–T14 as
proposed. T3, T4, T9 and T13 were changed by his notes, and T15–T18 are new.
The table shows the result.

| #   | Decision                                                                                                                                                                                                                                                       |
|-----|----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| T1  | **Home.** Time Tracking is a **sidebar section of the Invoicer** (Dashboard · Entries · Track · Accounts), because customers, the catalog, invoices and the sidebar already live there. All logic lives in a new `ledgeline-core::time` module (UI-agnostic, plan 26 rule), so Ledgeline can later show time reports and iOS can reuse it. Ledgeline itself gains only include-safety (T12) in this plan. |
| T2  | **Files.** A roll-up journal (default `time.journal`) sits beside the main journal. It holds directives, `include time-accounts.journal` and the `include` globs for the time files. The main journal does **not** include it. It is created only by an explicit **Set up time tracking** action, like the prices and recognition files. Every name and location is configurable (T15). |
| T3  | **Our own tracking file:** default `time/<person>/YYYY-MM.timedot`, or `time/YYYY-MM.timedot` when there is no person (T16). It is configurable as a pattern (T15).                                                                                              |
| T4  | **Amounts** are whole minutes `Nm`, or `Nh` when exact (`45m`, `2h`, `130m`). **Each session is rounded to the nearest minute, half up**: 30 s → 1m, 1m20s → 1m, 1m30s → 2m. No seconds are carried in timedot mode. A toggle lasting under 30 s records nothing; that's an accidental press. Ledgeline reads exact minutes. hledger 1.52 rounds each line to 0.01 h, which goes away in 1.99.6, and invoices never depend on hledger's figure. Dots, letters and decimals are read, and preserved on lines we don't touch. |
| T5  | **Tags** (T-vocab below): `customer`, `billable: yes\|no`, `item` and `archived` on accounts; `invoice`, `bill` and `person` on entries and days. Ledgeline resolves account tags nearest-ancestor-wins (fact 7) and documents `--pivot` for hledger users. The names can be overridden (T15). |
| T6  | **Tracker lines.** The tracker adds minutes to the day's **open line** for that account: the first line for that account that day with no `invoice:`/`bill:` tag. A billed line is never changed; new time goes on a new line.                                                 |
| T7  | **Running state is in the file**: a comment line under today's date, `; ● running clients:acme:tax  since 14:05:12`. hledger ignores it, and it survives crashes, restarts, other machines and a git pull. Stopping adds the rounded minutes and removes the line. |
| T8  | **Slots.** Slots 1–5 are pinned by the user. Slots 6–9 are the most recent accounts that aren't pinned. Slot 0 is **Other…** (a fuzzy picker) and shows whatever was last chosen through it. Pins are per user, in `ui-prefs.toml [time]`. Recents are derived from the files, so there is no state file. |
| T9  | **Keys.** On Track, the bare keys: `1`–`9`, `0` toggle; `+`/`−` adjust; `n` note; `.` stop. App-wide, by platform (K):<br>• **macOS: ⌥⌘1 … ⌥⌘0.** ⌘⇧3/4/5/6 are screenshot shortcuts that the OS takes first.<br>• **Linux and Windows: Ctrl+Shift+1 … 0.** Ctrl+Alt is avoided because it *is* AltGr on many European layouts (AltGr+7 = `{` on German), so binding it would eat characters people type.<br>All of these can be rebound. OS-global hotkeys and a menu-bar/tray timer come in Phase 6, behind new crates that need approval. |
| T10 | **Billing.** "Bill time" makes an Invoicer **draft** and tags the lines `invoice:N` at once, so they leave Unbilled (Harvest semantics). Deleting the draft releases the tags. A sent invoice's lines are locked against edits in the UI. Contractor time becomes an AP bill through `write_document(Expense)` and is tagged `bill:REF`. One line may carry both, for contractor time rebilled to a customer. |
| T11 | **Rounding for billing** is never stored. It's a per-customer setting (increment, direction, scope; the default scope is `row`), applied when an invoice row is made, with the actual and the rounded hours both shown. The default is none.                                                         |
| T12 | **Ledgeline include-safety.** `parse.rs` learns `.timedot`/`.timeclock` includes, `timedot:`/`timeclock:` prefixes and globs. Commodity-less virtual time postings must not reach money reports (Insights, Budget, KPIs, Problems). A user who includes the roll-up in the main journal still has working books. |
| T13 | **Timeclock is read and edited, and the tracker can write it.** `[time] record = "timedot" \| "timeclock"` (default `timedot`). In timeclock mode the tracker writes `i`/`o` pairs **with seconds** (`HH:MM:SS`), and they are kept exactly. Sessions are the record there, so they aren't rounded, and billing totals sum the exact durations before any customer rounding (T11) ([[accounting-flows-configurable]]). |
| T14 | **CLI parity.** `ledgeline-invoicer time start\|stop\|toggle N\|status\|log\|unbilled` exists from Phase 2. That also gives Raycast, Alfred, Stream Deck and shortcuts tools a way to drive the tracker from anywhere, before we ship our own global hotkeys. |
| T15 | **Bring your own layout.** Our defaults are only defaults. Ledgeline must work with existing time files, other layouts and other languages (T-config):<br>• every file name and location can be changed (roll-up, accounts file, folder, tracker file pattern);<br>• setup can **adopt an existing roll-up or time file** instead of creating ours;<br>• the reader never assumes our layout: whatever the roll-up includes is the time book;<br>• tag names and words we write (weekday names, file headers, seeded account prefixes such as `clients`/`internal`) can be overridden, as `[invoicer.words]` already allows. |
| T16 | **The person is optional** (P-person). It's suggested at setup from the OS account's full name, then its login name, and can be edited or cleared on Track. With no person set, files go in `time/` directly and date lines carry no `person:` tag. Single-user books never need a name. |
| T17 | **Lamp colours:** **green ▶ = running, grey ■ = stopped.** No red.                                                                                                                                                                                              |
| T18 | **Time cards for accounts payable link to a note**, either `type: person` or `type: organization`. The note says whose time it is and who gets paid. Defaults such as rate, accounts and payee live in its frontmatter; anything missing is asked for when the bill is made, with an option to save it to the note (B-ap). |

## Design

### F-files. Layout and what each file looks like

```text
books/
├── books.journal               main journal: does NOT include time
├── time.journal                roll-up: directives + includes (T2)
├── time-accounts.journal       the time account tree with tags
└── time/
    ├── pat/
    │   ├── 2026-10.timedot     tracker files (T3)
    │   └── 2026-11.timedot
    ├── sam/                    a contractor's submitted timesheets
    │   └── 2026-11.timedot
    └── imported/
        └── 2026-q3.timeclock   anything hledger reads
```

**`time.journal`**, written once at setup and then left to the user:

```journal
; Time tracking roll-up. Not included by books.journal on purpose:
; hours are not money. To report on time:
;   hledger -f time.journal bal --tree -p thismonth
;   hledger -f time.journal bal --pivot billable -p lastmonth
;   hledger -f time.journal reg not:tag:invoice tag:customer=Acme
; To see time beside the books, include this file from books.journal.

commodity ""                      ; time has no commodity (needed for -s)

include time-accounts.journal
include time/**/*.timedot
include time/**/*.timeclock
```

**`time-accounts.journal`** (F-accounts). Indented directives are ignored
(fact 8), so the tree is shown by order and spacing: one block per top-level
branch, parents before children, each block sorted, tag comments aligned in
one column.

```journal
; Time accounts. Tags: customer (links to an Invoicer customer), billable
; (yes/no, nearest ancestor wins), item (catalog code to bill as),
; archived (hide from pickers). Edit freely.

account clients                                   ; billable: yes
account clients:acme                              ; customer: Acme Corp, item: ARCH
account clients:acme:advisory
account clients:acme:tax-return                   ; item: TAX
account clients:acme:travel                       ; billable: no

account clients:globex                            ; customer: Globex
account clients:globex:audit

account internal                                  ; billable: no
account internal:admin
account internal:sales
```

**A tracker file** (`time/pat/2026-11.timedot`):

```timedot
# Time — pat — November 2026
# Written by Ledgeline; hand edits welcome. Keep two spaces before amounts.

2026-11-25 Wed  ; person: pat
    clients:acme:tax-return         130m  ; reviewed schedules C and E
    internal:admin                   15m

2026-11-26 Thu  ; person: pat
    clients:acme:tax-return          45m  ; invoice: 108
    clients:acme:tax-return          20m  ; call with Fred re: K-1
    clients:acme:advisory             1h
    ; ● running clients:acme:advisory  since 14:05
```

- The weekday becomes the hledger description, so `hledger reg` reads
  `Thu`.
- Entries are indented under their date. Amounts are right-aligned in one
  column per day. A note is the entry's comment and goes onto the invoice
  later.
- `person:` on the date line makes every posting queryable by person without
  repeating it on each line (fact 4). It's redundant with the folder, but the
  folder is invisible to hledger.
- The layout above is the default; T-config covers other names, places
  and languages.
- **Writing rules** (from facts 1–4):
  - always two or more spaces before the amount;
  - never tabs or spaces inside a number;
  - never `mo`/`y`;
  - tags only on the entry's own line;
  - account names validated against `;`, double spaces, a leading `* ` and
    date-like prefixes.
- **Untouched lines are byte-for-byte preserved.** Only the lines we change
  are re-aligned. **Tidy file** (an explicit action) re-aligns everything.

### T-config. Other layouts, names and languages (T15)

The defaults in F-files are only defaults. Users arrive with their own
timedot files, their own roll-up, accounts in other languages
(`kunden:acme`, `interne:verwaltung`), or a layout from another tool.

- **Reading never assumes our layout.** The time book is whatever the
  roll-up includes, wherever those files are and whatever they're called.
  With no roll-up, setup offers to find `*.timedot`/`*.timeclock` files
  under the books folder, or to pick one by hand.
- **Setup has two paths:**
  - **Create** writes our default files. Every name can be edited on the
    setup screen first.
  - **Use existing** points `[time] file` at an existing journal, or at a
    single `.timedot`/`.timeclock` file, which is then treated as a roll-up
    of one. It writes nothing else.
- **Where the tracker writes** is `track-file`, a pattern with `{person}`,
  `{yyyy}`, `{mm}` and `{dd}`. Setting it to a single existing file
  (`time.timedot`) is valid: the tracker then appends date blocks there. If
  the pattern's target isn't covered by the roll-up's includes, setup says
  so and offers to add the `include` line.
- **Tag names** can be renamed per books in `[time.tags]` (for example
  `invoice = "rechnung"`). The engine works on roles, not literal strings,
  as plan 45's constants do.
- **Words we write**, including weekday names, the file headers and the
  seeded `clients`/`internal` prefixes, come from `[time.words]` and
  default to the OS locale's weekday names. `weekday = "none"` leaves date
  lines bare.
- **Accounts don't have to sit under one root.** "Time account" means any
  account that appears in the time book, so an existing tree such as
  `projects:…` or `kunden:…` works as is.

### P-person. Who the time belongs to (T16)

- **Where the name comes from**, in order:
  1. `ui-prefs.toml [time] person`, if set.
  2. At setup, a suggestion: the OS account's full name, then the login
     name, reduced to a tag-safe short form (`Patrick Walsh` → `patrick`).
     It can be edited or cleared before anything is written.
  3. Nothing. That's a valid answer.
- **With no person:**
  - The tracker writes to `track-file` with `{person}` and its folder
    level removed (`time/2026-11.timedot`).
  - Date lines carry no `person:` tag.
  - The UI never asks again unless a second person appears.
- **Reading someone else's time.** A posting's person is, in order:
  1. its own `person:` tag;
  2. its date line's `person:` tag;
  3. the folder name, when the file sits under a `{person}` folder of
     `track-file`'s pattern;
  4. otherwise **unattributed**.

  Unattributed time reports normally. It just can't be grouped by person
  or turned into a contractor bill until someone assigns it (bulk *Set
  person*, which writes the date-line tag).
- **Changing your name later** only affects new writes. *Rename person* (in
  Accounts → People) rewrites the `person:` tags and can move the folder,
  with a preview, like account rename.

### T-vocab. Tags

| Tag         | On                          | Values                                      | Meaning                                                                                       |
|-------------|-----------------------------|---------------------------------------------|-----------------------------------------------------------------------------------------------|
| `customer`  | time account                | the customer's `ledger_customer`            | Links the subtree to an Invoicer customer. Nearest ancestor wins.                             |
| `billable`  | time account                | `yes` / `no`                                | Default `no` when nothing in the chain says so. Ledgeline warns about a `customer:` subtree with no `billable`. |
| `item`      | time account                | catalog code                                | The suggested catalog row (rate, account) when billing. Written by "Remember for this account". |
| `archived`  | time account                | `yes`                                       | Hidden from pickers and slots; history still reports.                                         |
| `person`    | date line (or entry)        | short name                                  | Who did the work. Entry-level wins over the date line.                                        |
| `invoice`   | entry                       | invoice number                              | Billed to a customer on that invoice. Same tag name as the books (`accrual::INVOICE_TAG`), so `hledger reg tag:invoice=108` across books and time shows both sides. |
| `bill`      | entry                       | bill reference                              | Paid to the person through that AP bill (self-bill).                                          |
| `nobill`    | entry                       | reason, optional                            | Written off or deliberately unbilled, so it leaves Unbilled.                                  |

Free text in an entry comment beside tags is the **note**.

### T-slots. The tracker

**Track** screen, at its default width:

```text
┌ Track ─────────────────────────────────────── Thu 26 Nov · 4h 05m today ┐
│                                                                          │
│  1 ○ Acme · tax return                 2h 55m    ▸ last: K-1 call         │
│  2 ● Acme · advisory          RECORDING 1h 22m   since 14:05              │
│  3 ○ Globex · audit                       —                              │
│  4 ○ Internal · admin                    15m                             │
│  5 ○ Internal · sales                     —                              │
│  ─ recent ───────────────────────────────────────────────────────────── │
│  6 ○ Acme · travel                        —     (yesterday)              │
│  7 ○ Initech · discovery                  —     (Mon)                    │
│  8 ·                                                                     │
│  9 ·                                                                     │
│  0 ⋯ Other…                                                              │
│                                                                          │
│  [ − 5m ]  [ + 5m ]  [ Note  n ]  [ Stop  . ]        today · week · card │
└──────────────────────────────────────────────────────────────────────────┘
```

- **Toggle.** A digit (or a click) on a stopped slot stops whatever is
  running, banks its minutes, and starts this one. On the running slot it
  stops. One press, one lamp lit (the device's rule).
- **Adjust.** `+`/`−` move the **selected** slot's today total by the step
  (default 5 min, `[time] step`). On the running slot they move its start
  time instead, as Tyme does, so the result is the same. `⇧−` is **Snap
  start to the end of the previous entry**, for "I started 20 minutes ago".
  Totals never go below zero.
- **Note** (`n`) edits the note of the slot's open line for today. The field
  takes inline tags too.
- **Other…** (`0`) opens the account picker with fuzzy search, recent
  accounts first and **Create account…** at the bottom. The chosen account
  starts and stays in slot 0 until something else is chosen there.
- **Pins.** Slots 1–5 are edited in place (`e` on a slot, or drag). An empty
  pin is filled from recents so all ten keys always do something.
- **Day card** (`c`) is the printed tag of the original device: today's
  totals per account, with notes, as a compact view to copy, print or share.
  It's the same data as the Entries day view.
- **Narrow width:** the slots become a two-column grid of large buttons with
  the digit badge kept. That's the future phone layout ([[ios-app-plan-42]]).

### T-light. "Recording" everywhere

A sidebar block, pinned above the section links, on every Invoicer screen:

```text
 running:   ▶ Acme · advisory      1h 22m    ⌥⌘2     (green)
 stopped:   ■ Acme · advisory      ⌥⌘2 to resume      (grey)
```

- **Running:** a **green ▶** with the slot's name, live elapsed time, and its
  key hint. Click it to stop.
- **Stopped:** a **grey ■** with the most recent account and "⌥⌘N to
  resume" (Ctrl+Shift+N on Linux/Windows). Click it to resume.
- No red anywhere (T17). Colour is never the only signal: the shape (▶ vs
  ■), the word "running"/"stopped" in the accessible name, and the elapsed
  time carry it too.
- The window title and the Dock badge show the elapsed time while running
  (cheap, and visible when the app is in the background).

### T-running. Running state, midnight, forgotten timers

- **Start** writes the marker line (T7) under today's date, creating the
  date block if it's missing.
- **Stop or switch** rounds the session to the nearest minute, half up
  (T4). It adds that to the open line (T6) and deletes the marker, in one
  verified write. A session under 30 s writes nothing but the marker
  removal. The marker's `since` keeps seconds so the rounding is of the true
  duration.
- **Timeclock mode** (T13): start writes an `i` line with `HH:MM:SS` (no
  marker needed: an open `i` *is* the running state, and hledger clocks it to
  now). Stop writes the matching `o ACCOUNT` line, also with seconds.
  Nothing is rounded.
- **Across midnight**, stop splits the time at 00:00 into each day's block
  (and each month's file).
- **Long-running guard.** On launch, or on wake, a marker older than
  `[time] long-running` (default 4 h), or from a previous day, opens a
  dialog: *Keep all · Stop at ‹last activity› · Stop at… · Discard*.
  Phase 6 adds real idle detection behind the same dialog.
- **Two machines.** The marker is in the file, so the other machine sees the
  timer running. Toggling on machine B while machine A's write is
  outstanding hits the existing `ExternalChange` guard. We re-read and
  re-apply, never overwrite.

### T-entries. View, edit, add

- **Entries** is a table over the roll-up with these columns: Date · Person
  · Account · Hours · Note · Billing (invoice/bill/nobill or *unbilled*) ·
  File.
  - It is filtered by period, person, customer, billable and billed state,
    with the URL filter/sort contract from Invoicer Phase 5.
  - Grouping: day (the default; a timesheet), week (a weekly grid, like
    Harvest/Clockify: accounts × days), or account.
- **Add** (`a`) opens a quick row: date (default today) · account (picker) ·
  duration (`45m`, `1h30`, `1.5`, `0:45` all accepted) · note. The row is
  appended to the right person's month file, inside its date block.
- **Edit** works in place on any of our lines, and on hand-written lines in
  any timedot file:
  - Changes are verified by re-parse.
  - A line tagged with a **sent** invoice or a booked bill is read-only. The
    unlock path is to void or release the invoice/bill.
  - Timeclock sessions edit as start/end; tags go on the `o` line.
- **Bulk:** select rows, then *Mark billable / Mark nobill / Move to account
  / Set person*.
- **Overlaps.** Timeclock sessions that overlap across customers, and a day
  over 24 h per person, are flagged (ABA 93-379).

### T-accounts. Accounts management

The **Accounts** page is a tree over `time-accounts.journal`, plus any time
account used in files but not declared (shown as *undeclared*, with
**Declare**).

- Columns: account · customer (resolved, with "inherited from …") · billable
  · item · hours this month · unbilled hours.
- Actions: **Add child**, **Link customer** (picker over Invoicer
  customers, or create new), **Billable yes/no**, **Default item**,
  **Archive**, and **Rename**.
  - Rename rewrites the declaration and every matching line in every time
    file, with a preview and a confirmation. Billed lines keep their tags.
- **Setup** seeds the file from the Invoicer's customers. Optional, one
  checkbox each: `clients:<customer-slug>` with `customer:` and
  `billable: yes`, plus `internal:admin` with `billable: no`.

### T-dash. Dashboard and surveys

Patrick, 2026-11-26: the dashboard has to be useful for time tracking
**outside invoicing**, not just as a route to billing. That means plenty of
graphs, top-N lists, filters, period selection, billable vs not, and
account depth. His answers to the design questions:

| #  | Decision                                                                                                    |
|----|-------------------------------------------------------------------------------------------------------------|
| W1 | **Invoicer only.** No Ledgeline Time tab, now or later. (This replaces Phase 6 item 6.)                     |
| W2 | **Views:** KPI tiles, hours over time, the account tree with depth, and top-N lists.                       |
| W3 | **A fixed, well-designed page.** No customizable widgets or saved views. Every figure drills into Entries with the same filters. |

**Filter bar**, at the top of the page and written into the URL:

- period: presets plus custom, with **compare to the previous period** on
  by default;
- person;
- customer;
- billable: all / billable / non-billable;
- account depth (1–N);
- account filter (a prefix or a query).

It uses the same URL contract as Entries, so drilling down keeps every
filter.

**Page, top to bottom:**

1. **KPI tiles:**
   - total hours;
   - billable hours and billable %;
   - average per working day;
   - days tracked out of working days;
   - unbilled hours, shown only when some billable account has a customer.

   Each tile shows the change against the previous period.
2. **Hours over time:**
   - stacked bars, by day, week or month (chosen automatically from the
     period length, and switchable);
   - the split is by top-level branch, customer, person or billable;
   - an optional weekly-target line.
3. **Account tree with depth:** a collapsible tree table with roll-ups at
   each level (`bal --tree`). Columns: hours, % of total, billable hours,
   change. Depth follows the filter bar, and each level can be expanded on
   its own.
4. **Top-N lists** (N = 10, plus "Other"), each with its change against the
   previous period: accounts at the chosen depth, customers, people (hidden
   when there's only one), and notes (most frequent note text).

Built under the dataviz skill. Every figure's tooltip gives the hledger
command that reproduces it, as plan 37 does. Data comes from
`GET /api/time/summary`, which supports grouping by any one or two
dimensions, depth, top-N and period series. The billing pieces (Unbilled by
customer with **Bill…**) live on the Unbilled view (B-unbilled), not on the
dashboard.

### B-unbilled. Which invoices to create

**Unbilled** = postings on a `billable` account with a `customer`, and no
`invoice` or `nobill` tag. For each customer it suggests a period:

- With `cadence: monthly` (customer note): the **last complete month** that
  has unbilled time, and older months as their own suggestions. With
  `weekly`/`quarterly`: the same idea per period.
- Without a cadence: everything up to the end of last month.
- A banner: *"3 customers have unbilled time through October: Acme 23.5 h ·
  Globex 4 h · Initech 11 h"*, with **Bill all…**.

Billable time with **no customer** (a `billable: yes` subtree with no link)
is listed separately as *Needs a customer*.

### B-invoice. Bill time → invoice draft

The **Bill…** dialog for a customer and period:

1. Groups unbilled entries by a choice of **account** (the default), account
   × person, or account × note.
2. For each group, chooses a catalog **item**. The suggestion order is:
   - the account's `item:` (nearest ancestor);
   - the item used for this account on the customer's last invoice;
   - the customer's most-used item.

   Rate and books account come from the item and can be changed per row.
   **Remember** writes `item:` onto the account declaration.
3. **Rounding preview** (T11): actual hours, rounded hours, and the
   difference per row and in total.
4. **Detail**: which supporting **timesheet table** to add under the bill
   table, from the customer's `time_detail: none | daily | entries` (default
   `daily`). The columns are `Date · Who · Work · Hours`, and the notes fill
   *Work*. Each bill row's description defaults to the account's leaf name
   (or its `description:` tag) plus the period, e.g. *Tax return review, Nov
   2026*. The user's own wording from the original device ("tax return
   review") is just the row description, editable.
5. **Create draft** creates the draft through invoicer-core (the CLI's
   `new --row` path, so it is one write, not create-then-PUT) and tags every
   included line `invoice: N` in the same operation.
   - The tags are written first; the draft second. On failure the tags are
     removed, as plan 45 does with its two files.
   - A failure halfway shows up as a `check` finding with a fix.

Lifecycle:

- **Delete a draft** removes its `invoice:` tags (releases the time).
- **Void** a sent invoice asks: *Release the time* (the default) or *Mark it
  nobill*.
- **Rebook** after editing hours leaves the time tags alone. The invoice is
  the truth for money (ARCHITECTURE §1.3).

### B-ap. Self-bills: contractor time → accounts payable

- **A time card links to a note** (T18): an MBR `type: person` or
  `type: organization` note. That note says whose time it is and who is
  ultimately paid. Using an organization note covers an agency or a
  contractor's company.
  - **The link.** The note has `time_person: sam`, which matches the
    `person:` tag on the time card. Without that key, the note's title
    reduced to a tag-safe short form is used, as in P-person. The note is
    tagged `timekeeper` so pickers can list just those notes.
  - **Optional frontmatter** on the note:

    ```yaml
    ---
    title: Sam Lee
    type: person
    tags: [timekeeper]
    time_person: sam
    pay_rate: $85                         # default hourly rate
    vendor: Sam Lee                       # the vendor: tag in the books; defaults to the title
    expense_account: expenses:contractors
    payable_account: liabilities:accounts-payable
    pay_terms: 15
    pay_to: "[[Lee Consulting LLC]]"      # optional: pay someone else, e.g. their company
    ---
    ```

  - **Anything missing is asked for.** Create bill opens with the gaps as
    empty, focused fields: rate, expense account, and payable account if
    the books have more than one. *Save to Sam Lee's note* (checked by
    default) writes the answers into the frontmatter. That is an explicit
    action, so it's allowed by [[settings-files-toml]]. Unchecked, they're
    used for this bill only.
  - **No note at all** for a `person:` value: the bill dialog offers
    *Pick a note* or *Create note…* (person or organization, prefilled
    from the person tag) before it can continue. A bill always has a payee.
- **Unbilled to pay** works like B-unbilled, but per person and by the `bill`
  tag, for **all** of that person's time, billable or not.
- **Create bill** calls `accrual::ops::write_document` with
  `Direction::Expense`, `Party::Vendor` (the `pay_to` note's vendor name,
  or else the linked note's), the payable account, one line per
  account group (or one line in total, the person's choice), and
  `reference: <person> timesheet 2026-11`. Every included line gets
  `bill: <reference>`.
- Contractor time can still be rebilled to the customer. The `invoice` and
  `bill` tags are independent.
- **Getting their time in.** A contractor can:
  1. send a `.timedot` file that we drop into `time/<person>/`;
  2. use the same app pointed at a shared folder;
  3. be entered by hand in Entries with Person set.

  CSV import from Toggl, Harvest and Clockify is Phase 6.

### B-states. Approved, locked, billed

v1 has two states per line: *unbilled* and *billed* (`invoice`/`bill`/
`nobill`). Phase 6 adds `approved: DATE` (a date-line tag, per person per
week) and a `[time] closed-through` lock, following plan 45's
`closed-through`. Lines in a closed or approved week then need an explicit
unlock. The three states are kept apart as Harvest does, and the tag names
are reserved now.

### B-round. Rounding

Customer note keys:

- `time_increment` (minutes: `6`, `15`; default none);
- `time_rounding` (`up` | `nearest`; default `up`);
- `time_round_scope`: `row` (the default: round the grouped invoice row,
  which is ABA-friendly), `day` (each account-day), or `entry`.

The stored time never changes. The invoice row's *Hours* is the rounded
quantity, and the timesheet table shows the actual hours. A global default
can sit in `[invoicer]`.

### B-rates. Rates

v1 takes the rate from the catalog **item** and freezes it on the invoice
row, which invoices already do: a catalog change doesn't rewrite history.
Per-person rates (Sam at $150 and Pat at $250 on the same account) use the
*account × person* grouping with a different item per group. The suggestion
remembers per person in the customer note:

```yaml
time_items: [{account: clients:acme, person: sam, item: JR}]
```

That's the kairos-style longest-match rule ARCHITECTURE §7 sketched.

### K. Keys

| Where           | Keys                                                                      |
|-----------------|---------------------------------------------------------------------------|
| App-wide        | `g t` Track · `g e` Entries · `g a` time Accounts · `g w` time dashboard (checked against existing `g` chords) |
| App-wide, macOS | `⌥⌘1`–`⌥⌘9`, `⌥⌘0` toggle slot · `⌥⌘.` stop · `⌥⌘↓` / `⌥⌘↑` adjust the running slot |
| App-wide, Linux/Windows | `Ctrl+Shift+1`–`9`, `0` toggle slot · `Ctrl+Shift+.` stop · `Ctrl+Shift+↓` / `↑` adjust (outside text fields only, where those keys extend a selection) |
| Track           | `1`–`9`, `0` · `+` / `−` · `⇧−` snap start · `n` note · `.` stop · `e` edit pin · `c` day card · `↑↓` select slot |
| Entries         | `a` add · `j`/`k` · `Enter` edit · `x` select · `b` bill selected · `/` filter |
| Help            | Everything above in `?`, in a new *Time* group                            |

`chord.ts` gains digit handling by `event.code` (`Digit1`…) when a modifier
is held, so `⌥⌘1` and `Ctrl+Shift+1` match on every layout. On macOS, `⌥`
plus a digit types a symbol (`¡`, `™`, …), and Shift plus a digit types
`!`, `@`, … so both need the code path. The keymap gets a per-platform
binding form (today `mod+` maps Cmd↔Ctrl only); the help sheet shows the
current platform's keys.

Why not the obvious ones:

- **⌘⇧digit:** macOS reserves ⌘⇧3, ⌘⇧4, ⌘⇧5 and ⌘⇧6 for screenshots.
- **Ctrl+Alt+digit:** on Windows, and in many Linux layouts, Ctrl+Alt is
  AltGr, which types `{ [ ] }` and `@` on German, Polish and Nordic
  layouts.
- **Ctrl+digit:** browsers use it for switching tabs, and GNOME/KDE often
  use it for workspaces.

Ctrl+Shift+digit is free in GNOME, KDE and Windows by default. The one
exception is Windows' optional "switch keyboard layout" hotkey, which is
off unless someone turns it on.

Tests cover US, German and Polish layouts on all three platforms'
modifier sets. Every binding can be rebound in Settings → Time.

### Settings

- `ui-prefs.toml [time]` (per user):
  - `person` (optional; see P-person);
  - `pinned` (five account names);
  - `step` (5);
  - `long-running` (4h);
  - `weekly-target`;
  - the key overrides.
- `ledgeline.toml [time]` (per books, only when someone sets it; see
  T-config):
  - `file` (`time.journal`), the roll-up;
  - `accounts-file` (`time-accounts.journal`);
  - `track-file` (`time/{person}/{yyyy}-{mm}.timedot`, which becomes
    `time/{yyyy}-{mm}.timedot` with no person);
  - `record` (`timedot` | `timeclock`);
  - `weekday` (`short` | `none`): the weekday text written on date lines;
  - `[time.tags]`: renames for each tag in T-vocab;
  - `[time.words]`: words written into files (headers, seeded account
    prefixes);
  - `closed-through` (Phase 6).
- Customer note keys: `cadence` (exists), `time_detail`, `time_increment`,
  `time_rounding`, `time_round_scope`, `time_items`.
- The Settings page gets a *Time* section for the ui-prefs keys
  ([[settings-files-toml]]: nothing written to the books folder without an
  explicit action).

## Performance

Budgets (release criteria, as in Invoicer ARCHITECTURE §5.6):

- **Toggle**, from keydown to the lamp lit, **< 50 ms**. The UI updates
  optimistically and the file write follows. If the write fails, the slot
  reverts with a message.
- **Reading the time tree**: 10 years × 5 people × 8 lines/day ≈ 100 k lines
  **< 150 ms** cold. Incremental re-read per changed file via the watcher.
- **Unbilled and the dashboard** over that data **< 100 ms**.
- Benches go in `ledgeline-core/benches/` next to the existing ones.

## Scope

In:

- The core reader and writer for timedot and timeclock.
- The roll-up, glob and include support in the core parser.
- Ledgeline include-safety.
- Tracker, sidebar lamp, Entries, Accounts, Dashboard.
- Unbilled → invoice draft, and contractor time → AP bill.
- Rounding.
- CLI.
- Docs, fixtures, e2e.

Out (Phase 6, or **(Q)**):

- OS-global hotkeys and a menu-bar timer.
- Idle detection.
- Approval and lock.
- CSV imports.
- Calendar-event drafts.
- Activity-log drafts (Timing-style).
- Budgets per project with alerts.
- A Ledgeline Time tab.
- iOS.

## Phase 1 — Core engine (PR 1)

New module `crates/ledgeline-core/src/time/`:

- `timedot.rs`: a lossless parser (every line kept with its span and kind:
  date, entry, comment, marker), matching facts 1–4. It reads dots, letters,
  numbers and units, with exact minutes (`Dec`) and **no 0.01 h rounding**.
  It reports a 1.52 discrepancy as a finding only where it matters (a `20m`
  line totals 0.33 in hledger).
- `timeclock.rs`: a parser with 1.52's pairing semantics (fact 9): bare `o`
  closes the most recent session, same-account overlap is an error, open
  sessions run to now, and sessions split at midnight.
- `model.rs`: `TimeEntry { date, account, minutes, person, note, tags,
  source: SourceRef }`, `TimeAccount` with resolved tags
  (nearest-ancestor), and `TimeBook` (the loaded roll-up).
- `write.rs`: pure text edits on a lossless file:
  - `add_minutes(open line)`, `insert_entry`, `set_entry_tag` /
    `remove_entry_tag`, `set_note`;
  - `start_marker` / `stop_marker`, `split_at_midnight`;
  - `tidy`, timeclock `set_session`.

  Each edit is verified by re-parse, then written through the
  `JournalEditor`-style fingerprint/atomic-write machinery. Extract the
  shared part of `edit.rs` rather than copying it.
- `book.rs`: roll-up discovery and creation (the plan 45 `book.rs`
  pattern), the accounts-file writer (F-accounts layout), and account
  rename across files.
- `query.rs`: summaries by period, account tree, customer, person and
  billable; unbilled per customer and per person; suggested periods.
- `parse.rs`: `include` of `.timedot`/`.timeclock`, the `timedot:`/
  `timeclock:` prefixes, and globs (`*`, `**`, sorted for a stable order).
- **Money-report safety.** `Insights`, budget, KPIs and Problems exclude
  commodity-less virtual postings from time files. There is a test with the
  roll-up included in the main journal.
- **Fixture** `fixtures/time/`:
  - two people, three customers, one contractor;
  - a timeclock file;
  - billed, unbilled and nobill lines;
  - a midnight session.

  `capture.sh` records hledger 1.52 outputs (`bal --tree`, `--pivot
  billable`, `reg tag:invoice=…`) as goldens, and core results are compared
  in hours to 0.01 tolerance. The fixtures must pass the flake `src` filter.
- Property tests: parse→write→parse round-trips, and minutes are conserved
  under split and switch.

## Phase 2 — Server and CLI (PR 2)

- `invoicer-core::time`: a thin layer over core. It joins time accounts to
  customers and catalog items, and adds the billing operations (B-invoice,
  B-ap) through the existing draft and `write_document` paths.
- Invoicer HTTP:
  - `GET /api/time/state` (slots, running, today);
  - `POST /api/time/toggle|adjust|note|stop|snap`;
  - `GET/POST/PUT/DELETE /api/time/entries`;
  - `GET /api/time/accounts` and `POST` for the account actions;
  - `GET /api/time/summary`;
  - `GET /api/time/unbilled`;
  - `POST /api/time/bill`;
  - `POST /api/time/pay`.
- The watcher follows the roll-up and every file it includes, as the
  recognition file is followed.
- CLI `ledgeline-invoicer time …` (T14), with `--json` throughout.
- Endpoint tests and goldens.

## Phase 3 — Track, the lamp, and keys (PR 3)

- Track screen, the sidebar lamp, the long-running dialog, setup (person
  name and file creation), and pins.
- The `chord.ts` `event.code` digit fix in `packages/ui`, plus the global
  bindings and the help group.
- Container-query layouts down to phone width.

## Phase 4 — Entries, Accounts, Dashboard (PR 4)

- The Entries table, add/edit/bulk, the weekly grid, and overlap flags.
- The Accounts tree with link, billable, item, archive and rename.
- The Dashboard (dataviz).

## Phase 5 — Billing, docs, e2e (PR 5)

- The Unbilled banner and lists, the Bill dialog, contractor pay, rounding,
  the release/nobill lifecycle, and timesheet tables.
- Docs:
  - a new `docs/time/` (formats, tags, the hledger recipes from F-files,
    the tracker, billing, contractors);
  - the Invoicer ARCHITECTURE §7 row replaced with a link;
  - mbr with no `[!INFO]`.
- e2e:
  - tracker toggle, switch, adjust and note;
  - lamp on every screen;
  - bill → draft → delete releases;
  - contractor → bill;
  - a hand-edited file picked up live.
- Screenshots.

## Phase 6 — Later (each its own decision)

1. **OS-global hotkeys** (`global-hotkey` crate) and a **menu-bar timer**
   (`tray-icon`). Both are new dependencies and need approval, and they
   need a running-app story: launch at login, or keep running in the
   background.
2. **Idle and sleep detection** feeding the long-running dialog:
   keep, discard, discard and continue, or reassign.
3. **Approval and lock** (B-states).
4. **Imports**: Toggl, Harvest and Clockify CSV; calendar (ICS) events as
   draft entries.
5. **Project budgets** with 75/90/100% alerts.
6. ~~A Ledgeline Time tab~~: dropped (W1).
7. **iOS**: Track as the phone's main screen, a widget, and a Watch
   complication.
8. **Hardware**: an EARLY/Timeular cube or Stream Deck mapped to slots
   through the CLI or a URL scheme.

## Execution

| Step | Owner                          | Scope                                           | Gate                                                         |
|------|--------------------------------|-------------------------------------------------|--------------------------------------------------------------|
| 1    | Engineer A                     | Fixture + capture, Phase 1, benches             | `just engine-check`, `just engine-test`, goldens vs hledger  |
| 2    | Engineer A                     | Phase 2                                         | Endpoint tests, CLI tests, `just test-integration`           |
| 3    | Engineer B (after 2)           | Phase 3                                         | `just check`, `just test`, `just lint`                       |
| 4    | Engineer C (parallel with 3)   | Phase 4                                         | same                                                         |
| 5a   | Engineer B (after 3+4)         | Phase 5 billing UI                              | same + invoicer e2e written                                  |
| 5b   | Writer (parallel with 5a)      | Docs                                            | `mbr -b docs` clean                                          |
| 6    | Lead                           | e2e, perf figures, full pre-push, diff review   | Everything green                                             |

Worktrees are created from HEAD by the lead, following
[[worktree-agents-branch-from-main]].

## Risks

- **The 1.52 quirks are real and silent.** One space before an amount gives
  a zero posting; `mo`/`y` are wrong; rounding drifts. Mitigations: the
  writer never produces those forms, and `check` flags hand-written lines
  that hit them ("hledger reads this as 0").
- **Two writers on one file.** Tracker writes go through fingerprints and
  re-apply on conflict. Two machines syncing a folder (iCloud, git) can
  still produce a conflict copy. The marker makes a lost stop visible
  instead of silent.
- **Users including `time.journal` in the books** (the hledger docs teach
  it). That's why T12 is in Phase 1, not later.
- **Inheritance surprises** (fact 7): a user's own `tag:billable=yes` query
  is wrong when a child overrides. Mitigations: the docs and the generated
  file header teach `--pivot`, and Accounts shows the resolved value with
  its source.
- **Tagging billed lines in hand-edited files** rewrites lines the user
  wrote. Only the tag is appended, the rest of the line is untouched, and
  this is tested on odd formats (dots, letters, org headings).
- **⌥⌘digit conflicts.** It's free in macOS and in our menus today. Check
  against muda menu accelerators and common launchers, and keep it
  rebindable.

## Open questions

Answered by Patrick, 2026-11-26 (review notes in mbr):

1. **(Q1) Home:** the Invoicer sidebar (T1).
2. **(Q2) File naming:** monthly per person, made configurable (T3, T15).
3. **(Q3) Running state:** in the file (T7).
4. **(Q4) Global keys:**
   - ⌥⌘digit on macOS (T9).
   - Patrick asked for Linux/Windows alternatives. Proposed: Ctrl+Shift+digit
     (T9, K). Confirmed.
5. **(Q5) Colours:** green ▶ running, grey ■ stopped, no red (T17).
6. **(Q6) Tag names:** as proposed, and renameable (T5, T15).
7. **(Q7) Billing at draft time** (T10).
8. **(Q8) Rounding scope:** `row` by default (T11).
10. **(Q10) The person's name:** optional, suggested from the OS account,
    and nothing breaks without one (T16, P-person). Confirmed.
11. **(Q11) Minute rounding:**
    - Timedot mode: nearest minute, half up (T4).
    - Timeclock mode: keeps seconds (T13).

9. **(Q9) Contractor identity:** a time card links to a person or
   organization note, which also names the payee. Rate and accounts come
   from its frontmatter, and anything missing is asked for (T18, B-ap).

No open questions remain.
