# Journal layout fixtures

Corpus for `crates/ledgeline-core/src/journals.rs` — which file an import should be written to —
and, in `split-year-assert/`'s case, for what an import may safely be *checked against*.

The first four trees are the **anti-assumption** fixtures. `journals::targets` ranks candidate
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

Every root passes `hledger -f ROOT check --strict` as committed — asserted by
`journals.rs::every_layout_root_is_a_journal_hledger_accepts`, gated behind
`LEDGELINE_HLEDGER_LAYOUT_CHECK` and run by `just hledger-checks`. A fixture hledger rejects is a
bug in the fixture.

## The fifth tree: `split-year-assert/`

Not a ranking fixture. It is the regression corpus for a different assumption — that the file an
import is **written to** is also the file it can be **reckoned against** — and it is the only tree
here whose 2026 file does not pass `hledger check` on its own. That is deliberate, and it is not a
bug in the fixture:

| | `hledger -f main.journal check` | `hledger -f 2026/2026.journal check` |
| --- | --- | --- |
| `split-year-assert/` | passes | **fails** |

`2026/2026.journal` opens with a start-of-year assertion carrying 2025's closing balance — the
shape `hledger close --assert` writes. The assertion is correct in the tree and cannot be correct
in the fragment, because the balance it asserts accumulates through a file hledger was never asked
to open. Two consequences the import path is built around, and this tree exists to pin:

- `hledger import -f 2026/2026.journal …` aborts on a correct journal, so `import_invocation`
  passes `--ignore-assertions`;
- the checking balance is **$895.00** through the root and **$-5.00** in the fragment alone, so
  every balance verification reads the root. Both numbers are asserted in
  `crates/ledgeline-server/tests/import_endpoints.rs`, because equality with the second one is
  precisely the bug.

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
| `split-year-assert/` | `2026/2026.journal`, `2025/2025.journal`, `main.journal` — the newest year first, which is what makes it the *default* import target and therefore the tree the bug was found in |

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
