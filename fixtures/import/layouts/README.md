# Journal layout fixtures

Corpus for `crates/ledgeline-core/src/journals.rs` — which file an import should be written to.

These four trees are the **anti-assumption** fixtures. `journals::targets` ranks candidate
target files, and the one rule it must never break is that the ranking is derived from content
and never from a filename. Guessing from names fails on at least two of the layouts below, and
would happily offer someone's `prices.journal` as the place to put their bank statement.

So each tree is chosen to defeat a different plausible heuristic:

| Tree            | The heuristic it breaks                                                          |
| --------------- | ---------------------------------------------------------------------------------- |
| `single/`       | "the root is the one with includes" — there are none; one file is the whole journal  |
| `split-year/`   | "`accounts.journal` and `prices.journal` are not targets" — true here, but only because they hold no transactions. `prices.journal`'s newest line is dated **2026-06-30**, later than every transaction in the tree, so a ranking that read dates without asking whether they belong to transactions would put it *first* |
| `full-fledged/` | "the root is called `main.journal`" — this one is `all.journal`, the full-fledged-hledger convention |
| `monthly/`      | "sort the ids" — the months are spelled as words, so alphabetical order is `february, january, march` and date order is `january, february, march`. Name-sorting answers february; the right answer is march |

Every root passes `hledger -f ROOT check --strict` as committed. A fixture hledger rejects is a
bug in the fixture.

## Expected ranking

Best-first, as `crates/ledgeline-core/tests/journals.rs` asserts. Files holding transactions come
first, ordered by their newest transaction descending; files holding none come last, in the order
the parse read them (root, then each `include`).

| Tree            | Ranking                                                                             |
| --------------- | ------------------------------------------------------------------------------------- |
| `single/`       | `main.journal`                                                                        |
| `split-year/`   | `2026/2026.journal`, `2025/2025.journal`, `main.journal`, `accounts.journal`, `prices.journal` |
| `full-fledged/` | `2018.journal`, `2017.journal`, `all.journal`                                          |
| `monthly/`      | `march.journal`, `february.journal`, `january.journal`, `main.journal`                 |

Note what is *not* hidden: a file with zero transactions ranks last but is always listed and is
flagged by `txn_count == 0`. Someone's genuinely empty `2027.journal` is a legitimate target on 1
January — the test suite builds exactly that case at run time. The root is likewise always
listed, however it ranks.

## The cases these files cannot express

Two live in `crates/ledgeline-core/tests/journals.rs` rather than here, because a committed tree
cannot make them convincing:

- **A tree where every name lies** — `prices.journal` holding the newest transactions,
  `accounts.journal` holding older ones, `2026.journal` holding only declarations. Committing
  that would be confusing to anyone browsing the fixtures; building it at test time is not.
- **An empty file for the coming year**, which git will not track.
