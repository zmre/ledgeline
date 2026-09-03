# QuickBooks Online "Journal" report fixtures

Corpus for `crates/ledgeline-core/src/qb_journal.rs` — the second import pipeline, the one
that reads a **grouped** report rather than one row per transaction. See
`plans/17-quickbooks-journal-import.md` and `docs/imports.md`.

The house rule from `fixtures/import/README.md` carries over unchanged: **every fixture is a
file some real exporter actually writes**, and each one exists to make exactly one wrong
implementation fail. There is no external oracle — QuickBooks does not publish a validator —
so the discipline is that every shape here was measured against a real export first.

Everything here is synthetic. The shapes are real; the company, the payees, the accounts and
the amounts are not.

## What "measured against a real export" means here

The source was one real QuickBooks Online Journal export: 204 rows, 46 groups, 102 posting
rows, 18 distinct accounts, six transaction types, read with **both** `openpyxl` and this
repo's own `calamine` before a line of the parser existed. Six properties came out of that
reading, and every one of them is load-bearing somewhere below.

| Measured | Why it matters |
| --- | --- |
| A closing row's Debit/Credit cells are **formulas** (`=I23+…+I32`), and what a reader sees is the **cached** value stored beside them | Nothing may evaluate a formula, and a cell with no stored value is an error cell, not a zero |
| Excel stores that value at up to **seventeen significant digits** — `70120.850000000006` | Shortest-round-trip `f64` printing recovers `70120.85` exactly, so the comparison needs no tolerance. Reading the stored digits as text does not |
| `#REF!` arrives as `Data::Error(Ref)`; `convert::spreadsheet` renders that as `""` | A damaged total would otherwise read as **zero** and a corrupted group would look balanced |
| Unused text cells on a posting row are `Data::String("")`, not `Data::Empty` | "Empty" has to mean "nothing printable", or every posting gets a `vendor:` tag with nothing after it |
| Ten of the eighteen account names contain a **colon** (`1520 Computer & Office Equipment:1521 …`) | The WP's Phase B sketch says they never do. They do |
| Date, type, `Num` and `Name` repeat on **every** posting row and never vary within a group; `Description` **does** vary | The memo belongs to the posting, the other four to the transaction |

## The files

| File | What it proves |
| --- | --- |
| `simple.xlsx` | The baseline, and the **sign** check. Two two-posting groups — the shape 44 of the real export's 46 groups have — under the customized 14-column header, with the merged title band above and the `TOTAL` row and timestamp footer below. `amount = debit if debit else -credit` has to make the deposit `+74999.71`/`-74999.71` and the card charge `-79.99`/`+79.99`, with no knowledge of any account's type. Also the empty-string fixture: `Item class` is present-but-blank on every row, so a reader keyed on `Data::Empty` reports `class: Some("")` |
| `many-postings.xlsx` | The ten-line manual Journal Entry, and **two** traps. Six of its ten postings carry a different `Description`, so a parser that reads one memo for the group loses five; and the total is stored as `70120.850000000006`, so a parser that compares in `f64` or reads the stored text disagrees with its own sum by 6e-12 and refuses a good file |
| `default-columns.xlsx` | The **stock** column set: four columns fewer, and `Memo/Description`/`Account` where the other files say `Description`/`Account Name`. Asserted to parse to exactly what `simple.xlsx` does, so a detector keyed on the 14 labels, on a column count, or a mapper that knows one spelling fails here and only here |
| `truncated-tail.xlsx` | The real export's **own damage**, reproduced. Its four postings balance to 533.94 both ways — every arithmetic check passes — and the only thing that knows rows were deleted is that the group opened by `6` is closed by a surviving `Total for 11024`. A parser that pairs marker to closing row by *position* imports this silently and is wrong about it |
| `malformed-total.xlsx` | The other half of that: matching ids, so the total cell is actually reached, and `#REF!` in it. Must be `MalformedTotal`, never a total of zero |
| `mismatched-total.xlsx` | A stale cached value — postings summing to 533.94 under a total row that says 500.00. **Constructed, not observed**, because an untouched export's total is a formula over the very rows above it and cannot disagree; it can only go stale once a human edits an amount in a spreadsheet that did not recalculate. The only fixture that fails a parser which trusts its own sum and never reads the closing row |
| `orphan-total.xlsx` | Truncation from the *top*: a `Total for 99` closing nothing. A parser that reads "Total for" as "flush whatever I am holding" emits a transaction with no id, or nothing at all |
| `overlap.xlsx` | Phase B's (`crates/ledgeline-server`), not Phase A's: a WIDER re-download, for the "re-downloading is safe" property the write pipeline rests on. Group `441` is `simple.xlsx`'s deposit byte-for-byte, so committing this file after `simple.xlsx` must classify it `Unchanged` and write nothing for it; group `6` is the `QB_BILL` four-posting group under an id neither `simple.xlsx` nor `default-columns.xlsx` ever uses, so it is the one row a commit of this file actually writes |
| `zero-placeholder.xlsx` | Found in a real, full-size export after Phase A shipped, not in the original 204-row sample: the row right after marker `5221` repeats the date/type/Num but names no account and posts `$0.00` on both sides. Not corruption — every other cell on it is exactly what a real posting row's would be. `qb_journal::posting` must skip it (it moves no money either way) rather than refuse the group for "no account name"; a row with a REAL amount and no account is still refused, unchanged |
| `summation-drift.xlsx` | Found in the same real, larger export: group `7237`'s total row cached `975546.6699999999` for postings summing to exactly `975546.67`. Excel's own `SUM` is IEEE 754 addition over many terms and is under no obligation to land on the double nearest the tidy decimal answer — the original assumption that a group's total (at most ten terms in the 204-row sample) would always stay inside half a ULP does not hold for larger groups. `qb_journal::close` rounds the reported total to the computed sum's own precision (`Dec::rounded`) before comparing, rather than refusing a well-formed file as `MismatchedTotal` |
| `zero-net-leg.xlsx` | Found in a real export: group `7513`, a Bill Payment (Check) an offsetting credit memo reduced to net `$0.00`. Its Accounts Payable leg names a real account but leaves BOTH Debit and Credit blank — QuickBooks writes nothing once a leg nets to zero rather than an explicit `$0.00`. `qb_journal::posting` must read that as an implicit zero rather than refuse it as `AmountNotSplit`; the group's other row has no account and is dropped, so this becomes a single `$0.00` posting — verified against real hledger 1.52 to be a transaction it accepts |
| `semicolon-in-payee.xlsx` | Found in a real, much larger export: group `8801`'s payee is `Smith; Jones LLP` — a real business-name shape, not invented punctuation. hledger's own grammar has no way to write a literal `;` in a transaction's description (`parse::split_comment` is a plain `line.find(';')`), so writing it verbatim reached the journal-writing round-trip guard as an unnamed `EditError::RoundTripMismatch`. This is a Phase B (`crates/ledgeline-server`) fixture, not Phase A's — `qb_import::journal_safe` replaces the `;` before the transaction is ever built, and the commit test (`qb_journal_endpoints.rs`) asserts the written file contains `Smith, Jones LLP`, never the raw semicolon |
| `near-miss.xlsx` | **Not** a QuickBooks Journal, and the reason detection cannot stop at the header. An ordinary bank export carrying `Account Name`, `Debit` *and* `Credit` — the exact triple the header is recognised by — closing with a `Total` row of two numbers. Everything a name-based detector wants is here; what is absent is a bare-id marker row and a `Total for {id}`. A false positive costs the user the rules-matching flow they actually needed |
| `report.xlsx` | The whole export at full size: **45 groups, 100 posting rows**, the real file's group-size mix (43 of two, one of four, one of ten) and all six transaction types. The round-trip fixture — the transaction count must equal the number of `Total for ` rows anyone can count by hand, and every group must balance |

## Why detection is two conditions and not one

`detect` needs a header carrying something Debit-like **and** Credit-like **and**
account-name-like, *and* the grouping structure itself: a bare-id marker row plus a
`Total for {id}` row whose id a marker actually opened.

`near-miss.xlsx` is why the first condition never decides alone. The other direction is
covered by `truncated-tail.xlsx` and `malformed-total.xlsx`, which are asserted to **detect
yes and parse no** — a damaged export is still unmistakably this format, and saying "no"
would route the user to the CSV rules screen instead of to the refusal that tells them their
file lost rows.

The negative half of the suite runs `detect` over the *whole* rest of `fixtures/import/`,
not a sample: every workbook, every delimited file including the two that are not UTF-8, and
the whole rules-generator corpus. Three of those are the near misses worth naming —
`capitalone-card.csv` and `ambiguous-dates.csv` both carry Debit and Credit columns, and
`generate/isolated/quickbooks-label.csv` carries an unnamed first column holding one isolated
cell, which is the closest thing in the repo to a marker row.

## `Balance` is read for nothing, and here is what it is

The customized fixtures carry a `Balance` column, populated on every posting row, and nothing
ever reads it. Not because it is mysterious — it was worked out — but because knowing what it
is makes it clear it can never check anything. It accumulates each posting's amount **signed
by that account's normal balance side**, and resets at every group. Measured on the real
export: `3000 Member Equity` credited 70,000 gives +70,000; `3900 Retained Earnings` debited
35,131.01 takes it to 34,868.99; `1520 Computer & Office Equipment` debited 49.99 *adds*, to
34,918.98.

Three independent reasons that is unusable: reproducing it needs each account's declared type,
which appears nowhere in the export; it is computed in floating point (that last cell really is
stored as `34918.979999999996`); and it is scoped to the report's own date range, so it means
something different the moment the user changes the filter. The fixtures write it as float noise
on purpose, so a test that started reading it would fail.

## Regenerating

Built by the corpus-wide `fixtures/import/generate.py` and committed, like every other binary
fixture here:

```sh
nix-shell -p python3Packages.openpyxl python3Packages.xlwt python3Packages.odfpy \
    --run "python3 fixtures/import/generate.py"
```

One thing in that script is worth knowing about before editing it. **openpyxl cannot write a
formula's cached value** — it emits `<f>…</f><v/>`, a workbook no spreadsheet has ever
produced — so `_qb_patch_formulas` rewrites `xl/worksheets/sheet1.xml` after the save to put
the stored value back, and to spell an error cell `t="e"` with `<v>#REF!</v>`. That patch is
the only reason these fixtures resemble the real file at all, and it asserts on every
substitution rather than failing quietly. The same pass turns a `@@BLANK@@` sentinel into a
genuinely empty inline string, because openpyxl drops a cell whose value is `""`.

## Running the checks

```sh
cargo test -p ledgeline-core --test qb_journal   # every fixture here, plus detection over the whole corpus
cargo test -p ledgeline-core --lib qb_journal    # the constructed cases, on in-memory rows
```
