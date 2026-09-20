# The Projections tab, and the scenario files behind it

The Projections tab asks one question — *if things carry on like this, when does
the cash run out?* — and lets you argue with the answer. The top half is an
editable what-if table, seeded from your budget plus whatever your budget does
not cover. The bottom half is that table read forward in time as net income,
cash and net worth.

A scenario is **an ordinary hledger journal file that nothing includes**, so it
can be edited in your own editor, diffed, committed, and read by `hledger`
itself. Playing with a what-if can never damage a plan of record.

Design notes and the reasoning behind each decision live in
[`plans/22-projections.md`](../plans/22-projections.md).

## The file format

This is the whole of it:

```journal
; Ledgeline projection
; projection: Series A with a hiring ramp
; created: 2026-09-18
; updated: 2026-09-20

~ monthly  projection
    (revenues:salary)      $-12000  ; growth: 3%/yr
    (revenues:consulting)   $-2500
    (expenses:rent)          $4200  ; growth: 2%/yr
    (expenses:software)       $900

; A STEP: payroll jumps in April and grows from the new base. Two bounded rules
; for one account, linked by the `line:` tag so the editor shows one row.
~ monthly to 2027-04-01  projection
    (expenses:payroll)      $50000  ; line: payroll

~ monthly from 2027-04-01  projection
    (expenses:payroll)     $120000  ; line: payroll, growth: 5%/yr

; A ONE-OFF on the balance sheet: cash and equity, no net-income effect.
~ 2027-03-01  Series A
    (assets:cash)         $2000000
    (equity:preferred)   $-2000000

; A ONE-OFF on the P&L.
~ 2027-06-15  legal fees
    (expenses:legal)         $45000
```

Every line of it is hledger's own syntax. Nothing here is a Ledgeline format.

### The tag vocabulary

Three tags, and only three. Two live on a posting, one in a comment at the top
of the file.

| tag | where | means |
|---|---|---|
| `projection: <name>` | a comment at the top of the file | the scenario's display name |
| `growth: <rate>/<unit>` | a posting comment | the amount grows by `rate` per completed `unit` |
| `line: <id>` | a posting comment | two bounded rules are two segments of ONE row |

**`; projection: <name>` is the display name and the only marker.** A file
without it still loads — any journal with budget-like entries can be used as a
projection — and takes its name from its filename. `; created:` and
`; updated:` are the dates Ledgeline stamps when it writes the file; you can
delete them and nothing breaks.

**`growth:` reads `3%/yr`, `2.5%/mo`, `-1%/wk` and `0.03/year`.** The percent
sign is optional and the unit spelling is generous, because this is a tag a
human types into a journal by hand. Anything it cannot read makes the line
**flat** — never a failed file.

**`line:` is what makes a step change one row.** A step is a permanent change to
a recurring line: payroll jumps from `$50k/mo` to `$120k/mo` in April and stays
there, growing from the new base. That is two bounded `~` rules for the same
account, and the shared `line:` tag is what tells the editor to show them as one
row with a "from" on it rather than two unrelated lines. Without the tag they
are still two perfectly good rules; they just appear separately.

### Growth is stepwise, not smoothly compounded

A line marked `+3%/yr` holds flat for twelve months and then bumps 3%. That is
how a rent increase or a salary review actually lands. The stepped figure is a
real number you could have typed, so it is rounded back to the amount's own
display precision **at every step** — which is why a `$1,875.00` line is written
with its cents even when the figure is round.

Growth anniversaries are counted from the line's `from` date when it has one,
and from the start of the projection otherwise. That is what makes the second
segment of a step change grow from its **new** base.

### What hledger does, and does not, do with this

`hledger` reads these files. Both of these work:

```console
$ hledger -f projection-series-a.journal balance --budget -M
$ hledger -f projection-series-a.journal print --forecast=2026-10-01..2029-10-01
```

**What hledger will not do is apply `growth:`.** hledger has no arithmetic in
amounts — `$1200 * 1.03` is a parse error — so a journal file cannot state a
growing amount. `growth:` is a comment to hledger, and a `balance --budget`
report over a projection file shows the **base** amount in every period, in year
five exactly as in year one. Only Ledgeline expands it.

That was a deliberate trade. The alternative is writing the file out expanded,
one bounded rule per growth step, which plain hledger would read correctly and
which no human could then edit. The compact form won, and this paragraph is the
price of it. (Nothing is lost permanently: the tag is already there, so an
expanded export is possible later without a format change.)

> **Never round-trip a scenario through `hledger print`.** It drops `~` rules
> entirely, so the file would come back with every scenario line gone and
> nothing to say it had happened. Ledgeline writes these files itself.

## What moves cash: the residual rule

A projection line has no funding leg — it is an unbalanced virtual posting,
`(account) amount`, exactly as a budget goal is — so the cash effect has to be
implied. One rule, stated once:

> For each **group** — one `~` rule, or one dated event — sum **every** posting.
> The negation of that sum, the group's **residual**, is the implied cash leg,
> and it applies **in addition to** any cash postings the group already states.

Cash is an asset, so net worth takes that same implied leg, beside the group's
own asset and liability postings. Equity postings move neither series on their
own.

| the group | residual → implied leg | net cash | net worth |
|---|---|---|---|
| `(expenses:rent) $4200` | −4200 | −4200 | −4200 |
| …beside `(assets:checking) $-4200` | 0 | −4200, the stated leg | −4200 |
| `(expenses:legal) $45000` beside `(liabilities:payable) $-45000` | 0 | **0** | −45,000 |
| `(assets:cash) $2M` beside `(equity:preferred) $-2M` | 0 | +2,000,000 | +2,000,000 |
| `(expenses:rent) $4200` beside `(assets:cash) $-2000` | −2200 | −4200 | −4200 |

Row two is a double-count that a naive rule would make, and the residual avoids
it for a good reason rather than a special case: the group nets to zero.

**Row three is the one worth understanding.** It is an accrual — a bill incurred
in June and paid in August. The residual books **no cash in June**, because the
group already balances itself against a liability, so the runway is not charged
for money that has not left. Write the settlement as a separate dated event when
it happens:

```journal
~ 2027-06-15  legal fees, net 60
    (expenses:legal)          $45000
    (liabilities:payable)    $-45000

~ 2027-08-15  legal fees paid
    (liabilities:payable)     $45000
```

June moves net worth by −45,000 and cash by nothing; August moves cash by
−45,000 and net worth by nothing. That is the right answer, and it is one you
can express.

**Row five is a partial payment**, which is the case no boolean "did this fund
itself?" guard could express at all.

**Net income is untouched by any of this.** It reads revenue and expense
postings only.

### What is NOT modelled, on purpose

- **Asset growth.** A stock, a house, a pension: its balance is held flat. There
  is no return assumption anywhere, because a return assumption that is not
  yours is a number you would have to discover and undo.
- **Liability amortisation.** A mortgage payment cannot be split into principal
  and interest from anything the journal says, and guessing the split would
  silently invent a net-worth curve.
- **Tax and retirement modelling**, per-account balance projection, and any
  distribution over outcomes (Monte Carlo and friends).

The engine tells you when it could not do something: a period it cannot
enumerate, a growth rate that overflows, a commodity your opening balances are
not valued in. A projection that quietly drops a line is worse than one that
says so, so every one of those appears in the warnings above the charts.

## Where the files live, and what Ledgeline will write

### The scan

Ledgeline looks for `*.journal` files **in your main journal's own directory
tree** — the same root an `include` is confined to. Files named
`projection-<something>.journal` are listed first, under "Projections"; every
other journal is listed under "Other journals" so you can load a budget file or
last year's ledger and project from it.

The scan is deliberately narrow, and every one of these is a refusal you may
notice:

- **Symbolic links are skipped**, files and directories alike, and the listing
  says so. Stricter than `include`, which resolves them — a walk that follows
  links has to contend with cycles and with a target swapped between the walk
  and the write, and refusing them removes all three at once.
- **Hidden entries are skipped** (`.git/`, `.direnv/`, `.anything.journal`), as
  are `node_modules/`, `target/`, `vendor/`, `dist/`, `build/` and
  `__pycache__/`.
- **It descends eight directories** and lists at most 200 files, examining at
  most 20,000 directory entries. If a cap trips, the picker says the list is
  incomplete rather than showing a subset that looks complete.
- **No path outside the journal directory is ever reachable**, and no absolute
  path ever appears in the app or in an error message.

### Saving

**Saving is always Save As.** The Projections tab never writes to
`budget.journal`, to your main journal, or to anything reachable by `include`
from it — those files are offered for **loading** and are marked read-only.
Playing with a what-if cannot damage a plan of record.

The name governs both the display name and the file name. `Series A with a
hiring ramp` becomes `projection-series-a-with-a-hiring-ramp.journal`: lowercase,
runs of non-alphanumerics collapsed to `-`, the ends trimmed. The dialog shows
you the resulting path **before** it writes.

- **Ledgeline never creates a directory.** The dialog offers the folders the
  scan already found a journal in; making a new one is your job, in your own
  file manager or shell.
- **It never overwrites on a create.** The file is opened with `O_EXCL`, so the
  refusal is the kernel's and is decided atomically — you get "a file already
  exists there" and nothing is touched.
- **A save rewrites only what changed.** Your own comments, `account`
  directives, transactions, blank lines, tabs, column alignment and CRLF line
  endings all survive: a rule you did not edit comes out the bytes it went in
  as, and a save that changes nothing writes nothing at all.
- **The header is rewritten on every save**: `created:` is carried forward from
  the file, and `updated:` is set to today by the engine's clock, not your
  browser's.
- **If the file changed on disk** since you opened it, the save is refused with
  a conflict and nothing is written. Re-open it and re-apply your change.

A `; projection:` name may not contain a comma, because hledger ends a tag's
value at one and the name would read back truncated. Ledgeline refuses the save
rather than silently shortening it.

### One normalization to know about

A single-date rule — `~ 2027-03-01  Series A` — is loaded back as a **one-off
event**, which is the row the tab shows it as. If you had it as a recurring row
with a single-date recurrence, it will come back in the "One-off events" section
after a save-and-reload. The engine treats the two identically, so no projected
figure moves.

## Reading a projection file by hand

Everything the tab shows, you can get from `hledger` — minus growth.

```console
# The goals, as written, month by month.
$ hledger -f projection-series-a.journal balance --budget -M -b 2027-01-01 -e 2027-04-01

# The same rules read as future transactions.
$ hledger -f projection-series-a.journal register --forecast=2027-01-01..2028-01-01

# Both files at once — what you plan, beside what you did.
$ hledger -f main.journal -f projection-series-a.journal balance --budget -M
```
