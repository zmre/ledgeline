# Rules-file inference fixture (plan 32 D27)

A journal whose unknowns each came from a rules file in this tree, for "Remember as a rule" from
the Journal view: which rules file a transaction came from (`ledgeline_core::rules::infer`), and
the live check against that file's statement. Pinned by `crates/ledgeline-core/tests/rules_infer.rs`,
`crates/ledgeline-server/tests/review_rules_endpoints.rs` and, through `fixtures/sample.journal`'s
discovery root, `web/src/lib/review/rulesLive.integration.test.ts`.

Every rules file uses `date-format %Y-%m-%d` on purpose. `fixtures/` is also `sample.journal`'s
discovery root, and the import live tests stage a `%m/%d/%Y` statement there: ISO dates keep these
files out of that statement's candidate ranking at the prefilter.

| Transaction (`main.journal`)   | Rules files that book to its account               | What it exercises                                                         |
| ------------------------------ | -------------------------------------------------- | ------------------------------------------------------------------------- |
| `BLUE BOTTLE COFFEE 1001` 2025 | `import/citi-2025.csv.rules`, `citi-2026.csv.rules` | A year family: the 2025 file, with the newer one offered second (importing ahead) |
| `BLUE BOTTLE COFFEE 4412` 2026 | the same                                           | The transaction's year picks 2026; `citi-2026.csv` is the CSV-basis live check |
| `SHELL OIL 77`                 | the same                                           | A row the file's own `if SHELL` already categorises                       |
| `CHASE THING` (`id:`)          | `import/chase.csv.rules` (`comment id:%fitid`)      | One file: recommended                                                     |
| `AMEX STORE`                   | `import/amex.csv.rules`, `amex-business.csv.rules`  | Two families for one account: a tie, `needsChoice`                        |
| `WELLS ATM` (`id:`)            | `import/wells.csv.rules`, `wells-ofx.csv.rules`     | The FITID convention picks `wells-ofx`                                    |
| `WELLS BRANCH` (no `id:`)      | the same                                           | ... and `wells` for a transaction without an id                           |
| `VENMO FRIEND` 2026-01-10      | `import/venmo.csv.rules`, `venmo-legacy.csv.rules`  | `.latest.venmo-legacy.csv` stops at 2025-06-30: the legacy file is ruled out |
| `SCHWAB FEE`, `SCHWAB INTEREST` | `bank/2025/`, `2026/`, `2027/schwab.csv.rules`     | Sibling year directories are one family; no CSV, so the journal fallback |
| `CORNER STORE`                 | none (`assets:cash`)                               | No candidates: the toggle is hidden                                       |

`import/paypal.csv.rules` takes everything from `include paypal-common.rules`, so a rule cannot be
previewed on a temporary copy of it (`ok: false`).

The `.latest.*` files are hledger's import dedup markers. `.latest.citi-2025.csv` has no CSV beside
it — statements are often moved away after an import — and is still found by the rules file's name.
