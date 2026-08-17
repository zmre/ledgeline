# Journal sort fixtures

Corpus for `crates/ledgeline-core/src/sort.rs` — the format-preserving date sort that puts a
journal back in order after `hledger import` appended back-dated rows to the end of it.

Two independent things are checked, and it matters that they are separate:

| Check                                                          | What it proves                                       |
| -------------------------------------------------------------- | ------------------------------------------------------ |
| `cargo test -p ledgeline-core --test sort`                      | We move transactions and damage nothing else           |
| `LEDGELINE_HLEDGER_SORT_CHECK=1 cargo test -p ledgeline-core --test sort` | The **sorted** files are journals real hledger accepts, in date order |

The first alone is not enough: a sort could shuffle bytes into something that round-trips
through our own reader perfectly and that hledger rejects. Only the binary answers that, so the
second exists — opt-in, so `cargo test` stays hermetic and needs no hledger. As with
`fixtures/rules/`, **a fixture hledger rejects is a bug in the fixture**, and hledger is run
against fixtures we author and never against a user's file.

Every fixture here passes `hledger -f FILE check --strict` as committed. All but
`already-sorted.journal` and `interleaved-include.journal` deliberately **fail**
`hledger -f FILE check ordereddates` — being out of date order is what they are for — and all of
them pass it after sorting.

## Files

| File                         | What it proves                                                                       |
| ---------------------------- | -------------------------------------------------------------------------------------- |
| `interleaved.journal`        | The realistic case: two back-dated rows appended after later ones. `account`, `commodity`, an `include` and a mid-file `P` (with its own lead comment) must all come back on the same line numbers, and the transactions must sort **around** the `P` |
| `interleaved-include.journal`| The include target. It exists to be a file that is not sorted and not flattened — and to be the reason an `include` is a barrier, since an included file may set `Y`, `D`, `decimal-mark` or `apply account` |
| `comments.journal`           | A transaction's lead comment run travels with it; a file-header comment separated by a blank line does not |
| `crlf.journal`               | Every terminator is CRLF. A sort that normalises one shows the whole file as changed in the user's diff |
| `no-final-newline.journal`   | The last line has no terminator, and its transaction is already the newest — so it stays last and the file must come back without one |
| `already-sorted.journal`     | The identity case, byte for byte. Also carries a `comment` … `end comment` block holding a line that looks exactly like an out-of-order transaction, which must never be read as one |

## What the sort does *not* do

Two behaviours are deliberate and are what the fixtures pin, so a future reader does not
"fix" them:

- **Blank runs never move.** They are items of their own, not part of the transaction above
  them. No journal construct's extent depends on a following blank line (a transaction ends at
  the first unindented *or* blank line), so nothing requires the blank to travel — and pinning
  it is what keeps the diff to the transaction bodies alone. This is a deliberate divergence
  from `rules.rs`, where a conditional table really does need its blank line.
- **Transactions never cross a barrier.** Anything at column 1 that `sort.rs` does not
  positively recognise as position-independent — `commodity`, `decimal-mark`, `D`, `Y`,
  `include`, `alias`, `apply account`, and anything it has never heard of — splits the file, and
  transactions sort only within a run. `interleaved.journal`'s transactions all sit after its
  `include`, which is why they can all reach each other.
