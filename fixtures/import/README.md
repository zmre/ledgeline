# Statement-import fixtures

Corpus for `crates/ledgeline-core/src/convert/` — the preprocessor that collapses every
accepted statement format to one `Tabular` before rules matching, preview or CSV emission sees
it. See `plans/11-enhanced-import.md` § *Preprocessor decisions* and `docs/imports.md`.

The house rule from `fixtures/rules/README.md` carries over with one change. There, a fixture
hledger rejects is a bug in the fixture. Here there is no external oracle to run — a bank does
not publish a validator — so the rule becomes: **every fixture is a file some real exporter
actually writes**, and each one exists to make exactly one wrong implementation fail. A fixture
that passes under both the right and the wrong reading is not pulling its weight.

Everything in here is synthetic. No file contains a real account number, a real payee or a real
balance.

## Layout

`delimited/` and `spreadsheet/` are documented below. The sibling directories belong to the
other WP-11 lanes and are documented by them — `ofx/` (OFX 1.x/2.x and QFX), `match/` (rules
scoring), `sort/` (date re-ordering) and `layouts/` (journal-target ranking). Each lane appends
its own section here rather than starting a second README.

## `delimited/`

| File | What it proves |
| --- | --- |
| `tab.tsv` | The delimiter comes from the extension, and a declared delimiter earns no `ConvertNote` |
| `semicolon.ssv` | Same, for `;` — and the amounts use **decimal commas**, so splitting on `,` would double the column count. Re-read as `.csv` this is also the delimiter-sniffing fixture |
| `preamble.csv` | Two leading non-table lines, the second of which **contains commas** — so "the first record is the header" yields a two-column table. Must produce `PreambleSkipped { lines: 2 }` |
| `preamble.csv.rules` | Not a statement: the rules file `preamble.csv`'s owner would have written, saying **`skip 3`** — a number counted against the raw download. Its oracle is real hledger, not `just rules-check`; see § *The `skip` a rules file already says* |
| `ragged.csv` | Rows of 4, 3, 5 and 4 fields. Nothing may be dropped; the count must be reported as `RaggedRows { count: 2 }` |
| `trailer.csv` | Three traps in ten lines. A **trailer** of disclaimer paragraphs below the last transaction; a **blank row inside** the transactions; and a last transaction whose **final field is empty**, which is the row the trim must stop at. Both blank rows are spelled `,,,` — as many fields as the table and not one of them populated — so a rule keyed on width alone misses them. Must produce `TrailerSkipped { lines: 4 }` and `BlankRowsDropped { count: 1 }`, four rows, and no `RaggedRows` |
| `padded-prose.csv` | The trailer trap `trailer.csv` misses. Saved out of a spreadsheet, the title block and the disclaimer are padded to the table's width and are **not blank** — `Member FDIC,,,` is four fields holding one thing — so a rule keyed on field count calls them records, trims nothing, and hledger abandons the entire read on the first one. Must produce `PreambleSkipped { lines: 4 }`, `TrailerSkipped { lines: 3 }` and four rows |
| `trailing-delimiter.csv` | Every data row ends with the delimiter, so the body is one field wider than its own header. Counted, the header is the odd row out and is trimmed as preamble — which promotes the first **transaction** to header and loses it, silently, under a `skip 1` rules file. Must produce four rows, the real header, and no `PreambleSkipped` |
| `quoted.csv` | An embedded delimiter, an embedded newline and a doubled quote, all inside quoted fields. The embedded newline must not become an extra record |
| `latin1.csv` | Windows-1252. Carries `0x92`, `0x93`, `0x94` and `0x80` — the four bytes where Windows-1252 and ISO-8859-1 disagree, and where every smart quote and currency sign lives. None is valid UTF-8, so `chardetng` is the only thing that can read it, and only one of its two plausible answers is right |
| `utf16le-bom.csv` | **The ordering fixture.** UTF-16LE with a BOM and CRLF — what Excel's "Unicode Text" export writes — saved under a `.csv` name, as users do |

### The `skip` a rules file already says

`preamble.csv` and `preamble.csv.rules` are a **pair**, and they only prove anything together.
The rules file says `skip 3` because the download has a title line, an account line and a
header. Converting the download strips the preamble, so the header is line 1 — and the same
`skip 3` now eats the header and the first two transactions. hledger has no header concept: it
skips three *records* and reads whatever is left.

The measured answer over this pair, hledger 1.52: the raw download gives **3** transactions and
the converted CSV gives **1**, both at exit code 0 with nothing on stderr. On a statement of two
rows or fewer the converted answer is **0** — the shape the bug was reported in. Either way it is
a wrong number reported as a right one, which is why the check is behavioural:
`converted_and_aligned_imports_what_the_raw_download_does` takes all three counts (raw,
unaligned, aligned) and asserts the wrong one as well as the right one. A test asserting only
`aligned == raw` would still pass on a fixture with no preamble for `skip` to disagree about.

`convert::align_to_skip` is the fix — `skip - 1` comma-only records in front of the converted
CSV, so the two frames line up again. Comma-only and not blank: hledger discards truly blank
lines *before* `skip` counts them, so blank padding would be invisible to it and the transactions
would still be eaten.

### Why `utf16le-bom.csv` is the important one

`chardetng` cannot detect UTF-16 at all, and it does not decline: handed these bytes it returns
`windows-1252` with confidence, and every cell comes back with a NUL after every character.
`convert_tabular.rs` asserts that directly — it runs the detector against the fixture and pins
the wrong answer — so the test fails if anyone ever reorders `decode` to consult the detector
before sniffing the BOM. A fixture that only asserted the *right* answer would still pass with
the guard removed on a machine where the detector happened to guess differently.

## `spreadsheet/`

| File | What it proves |
| --- | --- |
| `simple.xlsx` | Header row plus date cells stored as **serial numbers** with a date number format, and an Amount column carrying a **currency** number format. The dates must come out `YYYY-MM-DD` via `as_datetime`; the amounts must come out `-54.2` and never `($54.20)` |
| `multi-sheet.xlsx` | Three sheets. `Cover` holds one populated cell, so selection must walk past it; `Transactions` starts at **C4**, so blank leading rows and columns must be trimmed before it is a table at all; `Summary` is a second genuine candidate, which is what makes `SheetChosen { name: "Transactions", of: 3 }` owed to the user |
| `preamble.xlsx` | A **floating title block** above the header — two blank rows, a one-cell title, a blank row, a second one-cell title, a blank row, then the labels on row 7. Trimming the blank edges is not enough; the title rows are inside the trimmed rectangle. Must produce the same `Tabular` as `simple.xlsx` plus `PreambleSkipped { lines: 4 }` |
| `trailer.xlsx` | The *other* end of `preamble.xlsx`, and the synthetic twin of what a real export ships below its transactions: two blank rows and two one-cell disclaimer paragraphs. Left in place, `to_csv` renders each as `,,,` and **hledger abandons the whole file** on the first one. The sheet also carries a blank row *between* the transactions, and a last transaction whose **Balance is empty** — the row the trim must stop at. Must produce `simple.xlsx`'s table with that one cell blanked, plus `TrailerSkipped { lines: 4 }` and `BlankRowsDropped { count: 1 }` |
| `brokerage-activity.rules` | Not a workbook: the realistic rules file the env-gated end-to-end check drives `real-brokerage-preamble.xlsx` through, once converted. Its oracle is real hledger, not `just rules-check` — see below |
| `single-column.xlsx` | The counter-example to that rule. A title over a genuine one-column list, so **every** row holds exactly one populated cell. A rule spelled "a row with one cell in it is a title" eats the sheet a row at a time; a one-wide table carries no signal, so nothing may be skipped and the answer is `NoTable` |
| `no-table.xlsx` | A valid workbook holding no table. This is `ConvertError::NoTable` — a specific answer about a fine file — and never `Malformed` |
| `legacy.xls` | The BIFF path, which is a completely different reader inside `calamine`. Asserted to produce a `Tabular` **equal** to `simple.xlsx`'s, so the two readers cannot drift apart |
| `sheet.ods` | ODS stores a date as ISO 8601 text in `office:date-value`, never as a serial — so the dates match the xlsx output while `DatesFromSerial` is correctly *absent* |

### The one file here that is not synthetic

`real-brokerage-preamble.xlsx` is a **real** brokerage "All Activity" export, scrubbed. It is the file the
preamble *and* trailer rules were written against, and it earns its place by being messier than
anything anyone would invent: a title block above the header, 34 transactions, a 26-row disclaimer
block *below* them, and a `Description` column with **embedded newlines** in it. Two of its
properties are worth naming because they are why the rule is scored on a row's extent and not on
how many cells it has populated:

- The header row has 15 populated cells; the transaction rows have 9, 10 or 11, because
  `Check Number`, `Cusip` and `Memo` are blank on most of them. Score by population and the body
  agrees on *ten* — so the header gets discarded along with the titles.
- Score by **extent** — one past the last populated cell — and every one of those rows ends at
  column 15 while the titles end at column 1. That is the split we want, and it is the honest
  analogue of a delimited record's field count, since `a,b,,,` is five fields and not two.

Its trailer is the other half of the story, and it is the bug a real user hit. Fourteen of those
26 rows are entirely blank and twelve hold one paragraph of legal text in column one. Converted
with the trailer left in, hledger says

```
could not parse "" as a date using date format "%m/%d/%Y"
record: ,,,,,,,,,,,,,,
```

and **abandons the entire read** — not 34 transactions with one skipped, but zero — so the
candidate scorer saw a hard failure and ranked a perfectly good rules file at zero. The user
reasonably concluded their rules file was broken.

Because it may be re-scrubbed, `convert_tabular.rs` asserts only on its **shape**: the column
labels, the column count, the notes, the exact row count (34, a property of the file rather than
of any row in it), that every surviving row is as wide as the header, and that no surviving row's
first cell is prose. Nothing is asserted about a payee, an amount or an account.

`brokerage-activity.rules` closes the loop: `LEDGELINE_HLEDGER_CONVERT_CHECK=1` converts the
workbook, writes the CSV to a scratch directory and runs real hledger over it, asserting 34
transactions come back. That check — not `just rules-check`, which has no data file to drive this
pair from — is what keeps that rules file honest. It runs as part of `just hledger-checks`.

### The two calamine traps these pin

`calamine` exposes **no access to number formats** (private module, request closed), so a cell
arrives as `Float(45678.0)` with nothing attached. Two consequences the fixtures exist to catch:

- `Data::as_datetime()` will happily convert an `Int` or a `Float`. Called on the `-54.20` in
  `simple.xlsx` it returns a date in 1899. Only the `DateTime` variant — which the *reader*
  produced because the cell's format said date — may become a date.
- Serial 60 is 1900-02-29, a day that never existed. `as_datetime` resolves it to 1900-02-28,
  which is also what serial 59 gives, so the two collide silently. Covered by a unit test in
  `spreadsheet.rs` rather than a fixture: no exporter writes that cell on purpose, and a
  committed workbook containing it would look like a mistake rather than a guard.

## Regenerating the binaries

The text fixtures under `delimited/` are edited by hand. The synthetic binaries — seven workbooks
and the two non-UTF-8 CSVs — cannot be, so they are built by `generate.py` and committed.
`real-brokerage-preamble.xlsx` is not among them: it is a real export and is committed as received (scrubbed).

```sh
nix-shell -p python3Packages.openpyxl python3Packages.xlwt python3Packages.odfpy \
    --run "python3 fixtures/import/generate.py"
```

Regenerate **only when a fixture's meaning changes on purpose**, and re-run the tests: the
assertions in `crates/ledgeline-core/tests/convert_tabular.rs` name specific cells, sheet names
and note counts.

## `generate/headers/` — the rules-file GENERATOR's corpus

Seven real-shaped bank export **header rows** (plus a few data rows each), for
`crates/ledgeline-core/src/rules/generate.rs` — the module that drafts a `*.rules` file for a CSV
that has none. Driven by `crates/ledgeline-core/tests/rules_generate.rs`.

The house rule above carries over unchanged: every file is a shape some real exporter writes, and
each exists to make exactly **one wrong implementation** fail. What is different is the oracle.
These fixtures have two, and both are used:

| Oracle | What it proves |
| --- | --- |
| the per-fixture assertions | the mapping, the date format and the decimal mark are the ones we meant |
| **real hledger** (`LEDGELINE_HLEDGER_GENERATE_CHECK=1`) | the drafted file actually imports — every row arrives, every posting has an amount, and the numbers are right |

The second is the one that matters, for `docs/imports.md`'s fact 4: **parse success is not a
matching signal.** A drafted file that is valid hledger syntax and produces garbage exits 0.

| File | What it proves |
| --- | --- |
| `chase-checking.csv` | The date column is `Posting Date`, not `Date` — a synonym table that only knows the bare word maps nothing here. `Details` holds the *words* `DEBIT`/`CREDIT` and must not become an amount column; `Type` and `Check or Slip #` are nobody's field and are **named, not mapped**, so `%type` stays reachable |
| `capitalone-card.csv` | **Two** date columns. The second is `date2` rather than dropped — a demotion, so its confidence is lower than the winner's. Also the split `Debit`/`Credit` scheme, and a `Category` column that must stay `%category` rather than being guessed at |
| `uk-current-account.csv` | Day-first dates (`31/01/2026`), unambiguously — some sample has a day > 12. Every cell carries `£`, so **no `currency` is declared**: the directive is a blind string prefix and would produce `££3.60`, a distinct commodity, at exit 0 |
| `euro-decimal-comma.csv` | Decimal commas, dotted day-first dates, and a per-row `Currency` column. The `-1.500` row is the one that bites — see below |
| `paypal-activity.csv` | Three amount-shaped columns (`Gross`, `Fee`, `Net`) and not one named anything hledger knows, so the **value-shaped fallback** claims the first at low confidence. And `Status`, whose values are `Completed`/`Pending`, which must never reach hledger's own `status` field (it wants `*`/`!`) |
| `ambiguous-dates.csv` | Every date component ≤ 12, so nothing in the data can tell month-first from day-first: the draft must say so rather than pick silently. Also offers a signed `Amount` *and* `Debit`/`Credit`, and hledger errors on a record with two non-zero amounts — so exactly one scheme may be mapped |
| `thousands-trap.csv` | The US mirror of the euro trap: `-1,200` is twelve hundred dollars |

### The whole-thousands row, and why both files have one

`euro-decimal-comma.csv` carries `-1.500` and `thousands-trap.csv` carries `-1,200`, and both
rows exist for one reason: **they are the only shape that discriminates.**

A lone separator followed by exactly three digits is ambiguous, and hledger resolves it by
assuming a decimal point. So `-1,200` is read as −1.2 and `-1.500` as −1.5 — a factor of a
thousand — and `print` re-renders each one *exactly as it was written*, so the error appears
nowhere in hledger's own output. Measured against 1.52:

| cell | no `decimal-mark` | `decimal-mark .` | `decimal-mark ,` |
| --- | --- | --- | --- |
| `-1,200` | −1.200 | **−1200** | −1.200 |
| `-1.500` | −1.500 | −1.500 | **−1500** |

A value carrying **both** separators (`2.400,00`) is resolved correctly with no directive at all,
which is why the original fixtures could not prove anything: the check passed against a generator
that emitted no `decimal-mark` whatsoever. `the_decimal_mark_the_generator_chose_is_the_one_that_changes_the_number`
therefore runs each file **twice** — as drafted, and with the `decimal-mark` line stripped out —
and asserts the **wrong** answer as well as the right one.

## Running the checks

```sh
cargo test -p ledgeline-core --test convert_tabular   # every fixture here, plus the properties
cargo test -p ledgeline-core --test rules_generate    # the generator, hermetically
LEDGELINE_HLEDGER_GENERATE_CHECK=1 \
  cargo test -p ledgeline-core --test rules_generate  # ...and against the real binary
```

## Adding a fixture

Say in the table above which **wrong** implementation it makes fail, then add the assertion.
A fixture whose test would still pass with the guard it covers removed belongs in the delimited
text corpus as documentation, not here as a test.
