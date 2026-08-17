# Rules-matching fixtures

Corpus for `crates/ledgeline-core/src/rules/matching.rs` — deciding which of the user's
`*.rules` files fits a statement file they just dropped.

This directory exists because of one empirical finding, and every file in it is shaped by it:

> **A mismatched rules file frequently succeeds with garbage, exit 0.**
> Parse success is not a matching signal. (`plans/11-enhanced-import.md`, fact 4.)

Verified against hledger 1.52. Running a checking-account rules file against a credit-card CSV
produced transactions with `income:unknown` postings and a posting with **no amount at all** — and
`hledger check` was perfectly happy. Worse, a rules file merely lacking a `currency` rule yields
*bare* amounts that form a separate commodity, so the import "succeeds" and the `$` balance never
moves.

So the corpus is not "some valid rules files". It is **two files that are right and three that are
wrong in three different ways**, where being wrong is invisible to hledger's exit code.

## Files

| File                     | What it is for                                                                                |
| ------------------------ | ----------------------------------------------------------------------------------------------- |
| `checking.csv`           | Withdrawal/Deposit split, US `%m/%d/%Y` dates, 4 columns. The statement everything is scored against |
| `checking.csv.rules`     | The **clearly-correct** match. Scores 1.0: no amountless posting, no bare amount, every row categorised |
| `creditcard.csv`         | A deliberately different shape — 5 columns, ISO dates, one signed amount, a bank-supplied category |
| `creditcard.csv.rules`   | Correct for **its own** data, and a clear mismatch for `checking.csv`                            |
| `garbage-success.rules`  | **Fact 4.** Reads `checking.csv`, exits 0, and produces 4 amountless postings and an `income:unknown` |
| `no-currency.rules`      | **Fact 4.** `checking.csv.rules` with `currency $` deleted, and nothing else. Every amount is bare |
| `wrong-dateformat.rules` | Rejected by **stage 1**, without hledger being run at all                                       |
| `golden/`                | Real `hledger print -O json` output for the four scoreable pairs. See below                     |

## The three wrong files are wrong in three different ways, on purpose

Each isolates exactly one failure, so a test that passes cannot be passing for the wrong reason:

- **`wrong-dateformat.rules`** is right about everything stage 1 can check *except* the date format
  — four fields against four columns, so the column test passes and only `%d.%m.%Y` vs `01/15/2024`
  can reject it. It is a genuine rules file for a German bank export, which is how it stays a file
  hledger accepts while being useless for this statement.
- **`garbage-success.rules`** is *cheap-checkable-identical* to the correct file: same width, same
  date format. Nothing pure can tell them apart, which is the entire argument for stage 2 existing.
  Its empty third field name means "ignore column 3", so `amount` reads the Deposit column — blank
  on four of five rows, each of which becomes a transaction with one amountless posting.
- **`no-currency.rules`** is a **one-line delta** from `checking.csv.rules`. That is deliberate: it
  is the control for the experiment, so any score difference between the two is caused by the
  missing `currency` and by nothing else.

`checking.csv.rules` and `creditcard.csv.rules` reject each other's data for two *different* stage-1
reasons — five fields against four columns one way, `%m/%d/%Y` against `2024-02-03` the other — so
neither rejection can be passing by accident.

## `golden/` — real hledger output, committed

`matching::signals_from_hledger_json` is pure, and the only authority on what
`hledger print -O json` actually looks like is hledger. Hand-written JSON would test our *idea* of a
shape we invented. These are the real bytes.

Committing them is what keeps `cargo test` **hermetic** — no test requires hledger on `PATH` — while
still anchoring the parser to the binary.

Regenerate with `./scripts/gen-match-golden.sh` after an hledger upgrade, and **review the diff**: a
change here is a change in the contract stage 2 reads. The generator strips the repo-root prefix from
the `sourceName` hledger absolutizes into `tsourcepos`, because a golden carrying somebody's home
directory is committed path disclosure. Nothing under test can notice — `sourceName` is the one field
of this JSON that `matching.rs` is forbidden to read (`docs/imports.md` § Security).

`LEDGELINE_HLEDGER_MATCH_CHECK=1 cargo test -p ledgeline-core --test matching` re-runs hledger and
compares, mirroring the opt-in `LEDGELINE_HLEDGER_RENDER_CHECK` pattern.

## What the signals come out as

The numbers the suite pins, all from real hledger runs that **exited 0**:

| Golden            | txns | postings | amountless | bare | unknown | score |
| ----------------- | ---- | -------- | ---------- | ---- | ------- | ----- |
| `checking`        | 5    | 10       | 0          | 0    | 0       | 1.00  |
| `creditcard`      | 4    | 8        | 0          | 0    | 0       | 1.00  |
| `garbage-success` | 5    | 6        | **4**      | 0    | 1       | 0.18  |
| `no-currency`     | 5    | 10       | 0          | **10** | 0     | 0.00  |

## Adding a fixture

Add the file, then run `just rules-check` **before** `cargo test`. The same house rule as
`fixtures/rules/README.md`: **a fixture hledger rejects is a bug in the fixture**, not in the matcher.

The three non-sibling `*.rules` files here are checked with an explicit `--rules`, against the data
they are wrong about (`garbage-success`, `no-currency` — both must exit **0**, that being the point)
or the data they are right about (`wrong-dateformat`, driven from a generated German-format
statement). Driving `wrong-dateformat.rules` at `checking.csv` fails, and that failure is precisely
what stage 1 reaches without spawning hledger at all.

`just rules-check` runs hledger against **fixtures only**, never a user's file — a `source … | CMD`
rule is a shell command hledger executes.
