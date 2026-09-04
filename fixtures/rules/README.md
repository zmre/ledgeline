# Import-rules fixtures

Corpus for `crates/ledgeline-core/src/rules.rs` — the format-preserving model of hledger CSV
import rules files.

Two independent things are being checked, and it matters that they are separate:

| Check                                     | What it proves                                                    |
| ----------------------------------------- | ------------------------------------------------------------------ |
| `cargo test -p ledgeline-core --test rules` | We do not damage what we did not touch (round-trip, isolation)      |
| `just rules-check`                        | These files are rules files **real hledger accepts**                |

The first alone is not enough: a fixture full of invalid syntax would round-trip perfectly and
prove nothing. `just rules-check` is what keeps the corpus honest, and it runs against hledger
fixtures **only** — never a user's file, because a `source … | CMD` rule is a shell command
hledger executes.

## Files

| File                            | What it is for                                                                |
| ------------------------------- | ------------------------------------------------------------------------------ |
| `simple/checking.csv.rules`     | The friendly, realistic shape most rules files have. Inline `if`, stacked-matcher `if`, and a commented `if` |
| `simple/and-groups.csv.rules`   | hledger's AND, in **both** spellings: a line-prefix `&` (an inline AND chain, an OR of two AND-groups, a plain OR list) and a same-line `&&` (on its own, and composed with a `&` continuation). Its CSV is built so each row's account **says** which group matched, which is what `rules_hledger_render.rs` checks against hledger itself |
| `simple/control-flow.csv.rules` | hledger's block-level `skip` and `end`, the two words that change which records are read at all. Its CSV is built so the surviving rows **say** where each word took effect |
| `simple/creditcard1.csv.rules`  | `amount-in`/`amount-out`, `balance-type`, and **column-aligned values** — exists to prove alignment survives editing |
| `advanced/mixed.csv.rules`      | Every construct that must stay `Opaque`, in one file: `if` table, a line-leading `&&`, `& !`, `!`, a match group, a `skip N` + `end` pair (an argument *and* two control words), `commentN`, `separator:`, tab indentation |
| `edge/crlf.rules`               | Every terminator is CRLF                                                        |
| `edge/bom.rules`                | Leading UTF-8 byte-order mark                                                   |
| `edge/no-final-newline.rules`   | Last line has no terminator — the case a naive reorder glues together           |
| `edge/empty.rules`              | Zero bytes                                                                      |
| `edge/only-comments.rules`      | Comments and blanks, no rules at all                                            |
| `tree/`                         | A **discovery** fixture — its shape is the point, not its syntax. See below     |
| `golden/`                       | Byte-pinned HTTP responses for the `/api/rules` wire. See below                 |

`simple/*.csv`, `advanced/mixed.csv` and `tree/import/2026/bank.csv` are the data files their
sibling rules describe. They are what `just rules-check` drives hledger from (`-f FILE.rules` is
*not* equivalent — since hledger 1.50 that form demands an explicit `source` rule), and they double
as the corpus for the CSV column preview.

## `tree/` — the discovery fixture

`rules::discover` walks the open journal's own directory tree and decides which `*.rules` files the
imports feature may ever look at (and therefore which a later `PUT` may overwrite). `tree/` is one
realistic journal directory, and the only fixture where the **directory layout** is the thing under
test:

| Path                              | Must discovery find it?                                                |
| --------------------------------- | ---------------------------------------------------------------------- |
| `tree/main.journal`               | It *is* the scan root — its directory is what the walk is confined to    |
| `tree/import/2026/bank.csv.rules` | **Yes**, and its label is `bank`. The `import/YYYY/` layout the depth cap exists to allow |
| `tree/import/2026/bank.csv`       | No — not a rules file. It is what `just rules-check` drives hledger from |
| `tree/node_modules/dep.rules`     | **No** — `node_modules` is in the scan's skip list                      |
| `tree/.hidden/hidden.rules`       | **No** — the scan skips every directory whose name starts with `.`       |
| `tree/.hidden.rules`              | **No** — and every *file* whose name starts with `.` as well             |

`.hidden.rules` is the newer of the two decoys. The dot check used to sit inside the scan's
`is_dir()` branch, so a hidden **file** was listed and offered for editing while a hidden
*directory* was not. A hidden entry is one the user's own file browser does not show them, and a
dot-file in a journal directory is far more often a tool's leftover than a rules file someone
wants listed. It is valid hledger syntax on purpose: it must be refused for being hidden, not for
being unparseable. (`just rules-check` deliberately does not drive it — the whole point is that
nothing reaches it.)

`.hidden/` stands in for `.git/` for a dull reason: git refuses to track any path with a `.git`
component, so a committed `.git/hidden.rules` cannot exist. The real `.git/` case is built at test
time in `crates/ledgeline-core/tests/rules_security.rs`, which is also where the cases that cannot
be committed at all live — symlinks, FIFOs, unreadable directories and a 20,000-entry tree.

`tree/node_modules/` needs a `!fixtures/rules/tree/node_modules/` negation in the repo `.gitignore`
to be tracked at all. That is deliberate: the fixture would be worthless if the tool it exists to
test could never see it. `.hidden/` and `.hidden.rules` need no such negation — nothing in this
repo's ignore rules excludes a dot entry — but check with `git check-ignore -v` before adding the
next decoy, because a fixture git silently refuses to track is a test that silently passes.

`tree/import/2026/bank.csv.rules` does double duty: it is the file discovery must find, **and** it
is the document `golden/rules-doc.json` describes. That is why it carries more than the discovery
test needs — every top-level setting a preferences panel renders, a one-matcher `if`, a
two-matcher OR list, a two-matcher **AND-group** (the only thing that pins the wire's
`groups[].matchers[]` nesting byte for byte), and one conditional **table**, which is the `opaque`
item the golden exercises. Editing it changes both the discovery assertions in
`crates/ledgeline-core/tests/rules_security.rs` and the golden bytes.

## `golden/` — the `/api/rules` wire, byte-pinned

The native `/api/*` wire has no schema codegen, and neither does this one: the `Wire*` structs in
`crates/ledgeline-server/src/rules_api.rs` are mirrored by hand on the SPA side. Renaming a Rust
field compiles, typechecks and passes on both sides while the imports screen quietly renders
nothing — the CLEANUP.md DRY-3 shape. `golden/*.json` are the RAW response bytes for the requests
in `golden/requests.tsv`, served over `tree/main.journal`, and
`crates/ledgeline-server/tests/rules_endpoints.rs` compares them byte for byte.

Regenerate with `just snapshot-rules-wire` — **only when the wire contract changed on purpose**, and
review the diff. These bodies cannot live in `fixtures/native/v1/` because that suite's
`every_pinned_request_fixes_its_own_dates` guard demands every URI carry an `asOf=`/`end=`/`to=`,
and a rules response has no dates in it at all.

## Deliberately excluded from `just rules-check`

`edge/empty.rules` and `edge/only-comments.rules`. A rules file with no `date` field is not a
valid rules file — and that is exactly what they are for: proving Ledgeline still opens, lists and
byte-preserves a file hledger itself would refuse. Asserting hledger accepts them would assert the
opposite of their purpose.

## Adding a fixture

Add the `.rules` file, add a data `.csv` beside it if it is named `*.csv.rules`, then run
`just rules-check` **before** `cargo test`. A fixture that hledger rejects is a bug in the fixture,
not in the parser.
