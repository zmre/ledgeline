# Imports

Two surfaces behind one nav item, each with its own tab:

- **New Transactions** — drop a statement file, convert it to CSV, match it against your rules
  files, run `hledger import`, and check the result before anything is written.
- **Edit Rules** — find, read, present and edit hledger CSV import rules files (`*.rules`).

Rules-file scope: **discover, present, edit, save.** Still not in scope: generating a rules file
from a CSV, and writing a chosen category back as a new `if` rule. The model below is shaped so
each of those slots in additively.

Raw text editing is deliberately *not* offered — that is what a terminal is for. The GUI covers
preferences, the row mapping, the two default accounts, and an ordered list of `if` rules.

Statement-conversion scope: CSV/TSV/SSV, OFX/QFX, and xls/xlsx/xlsm/xlsb/ods. **PDF is refused by
name** — see `docs/pdf-extraction.md` for the survey and the route back in. QuickBooks IIF/CSV
exports need an account-mapping store and do not belong in this pipeline at all; that doc explains
why.

## Where the code is

| Path | What it holds |
| --- | --- |
| `crates/ledgeline-core/src/rules.rs` | The format-preserving document model: parse, classify, render, `EditPlan`/`apply`/`verify` |
| `crates/ledgeline-core/src/rules/discovery.rs` | The directory scan, `RulesPath`, `Discovery::resolve`, the CSV preview |
| `crates/ledgeline-core/src/rules/matching.rs` | Scoring a rules file against dropped data — the two-stage matcher |
| `crates/ledgeline-core/src/convert/` | Statement preprocessors: `ofx`, `spreadsheet`, `delimited`, shared `encoding` |
| `crates/ledgeline-core/src/sort.rs` | Format-preserving date sort of a journal file |
| `crates/ledgeline-core/src/journals.rs` | Ranking candidate target journals, by content only |
| `crates/ledgeline-server/src/rules_api.rs` | `/api/rules`, `/api/rules/{*id}`, `/api/rules-preview/{*id}` |
| `crates/ledgeline-server/src/{hledger,git,prefs}.rs` | Subprocess invocation, the git safety net, the preferences store |
| `web/src/lib/imports/` | Pure form model, reorder, date-format catalogue, store, UI |
| `fixtures/rules/`, `fixtures/import/` | The corpora. See each `README.md` — they explain what every fixture is *for* |

## Subprocesses: exactly two modules, no shell, ever

`crates/ledgeline-server/src/hledger.rs` and `crates/ledgeline-server/src/git.rs` are the **only**
places in production code where `Command::new` may appear. That is a deliberate, enforceable
boundary, and it exists because this codebase already refuses to execute a rules file's
`source ... | CMD` directive (see § Security). Having won that argument, we do not then scatter
process spawning across the server.

The rules those two modules hold to:

- **Arguments as a `Vec<OsString>`, never a shell string.** No `sh -c`. `--` terminates every
  pathspec list, so a statement named `-f` is a file. git additionally gets `--literal-pathspecs`,
  because `statement[2026].csv` is otherwise a *glob*.
- **Every invocation has a wall-clock timeout.** A GPG passphrase prompt or a hung `hledger` must
  not hang the desktop window. Note the non-obvious part: on timeout the pipe-draining threads are
  *dropped, not joined*. `kill` reaches the child but not a grandchild still holding the same
  pipes, so joining would block for exactly as long as the timeout existed to prevent.
- **stdout and stderr are captured separately.** This is load-bearing, not tidiness:
  `hledger import --dry-run` writes the proposed transactions to stdout and its
  `would import N new transactions` status line to stderr, and the entire preview feature is that
  split.
- **git stages explicit pathspecs only.** Never `add -A`, never `add .`, never `commit -a`. A user
  with unrelated work in progress must find it untouched — asserted by a test that leaves an
  unrelated file dirty and proves it is still dirty afterwards.

## The one invariant everything rests on

> **Item spans partition the file exactly.** `items[0].span.start == 0`, each item ends where the
> next begins, the last ends at `text.len()`. No gaps, no overlaps.

From that, by construction rather than by testimony:

- rendering an unedited document is a concatenation of a partition, so it is byte-identical;
- reordering is a permutation of a partition, so it cannot lose or duplicate a byte;
- editing item 3 splices only inside item 3's body, so items 1, 2 and 4 are the same `&str` slices.

CRLF, a BOM, tabs, column alignment, trailing whitespace and a missing final newline all survive
with **zero special cases**, because nothing is re-rendered that was not edited.

This is why the model is spans over a `String` and not an AST with a pretty-printer. An AST's
failure mode for a construct it does not model is *silent data loss on save* — the file still
parses, it just no longer says what it said. A span model's failure mode is "you cannot edit this
one": visible, and safe.

## Editable vs opaque

Two obligations, kept strictly separate. Conflating them is how a span editor corrupts files.

> **(A) Extent must agree with hledger exactly.** If our idea of where an item stops differs from
> hledger's, editing or moving it damages its neighbour. When extent is uncertain, *widen* — swallow
> the ambiguity into one opaque item.
>
> **(B) Classification may be as narrow as we like.** Anything unclassified is opaque:
> byte-preserved, listed in order, still reorderable. When in doubt, opaque.

An `if` block is editable only if every matcher is plain (no `&`, `&&`, `!`), has no match group, and
does not begin with `;`/`#`/`*`; every body line is a known field assignment; and there is no
`skip`/`end`. Everything else — `if` tables, combined matchers, match groups, control flow — renders
as a dimmed read-only card that says *why* it is locked, and can still be moved.

Order is semantics: every matching block applies and the **last** assignment to a field wins. The UI
says "later matches win" for that reason.

### Three grammar facts that are easy to get wrong

Verified against hledger 1.52's `RulesReader.hs`, not just the manual:

- A **matcher line must start at column 1** with a non-space character. A body line must be indented.
  An indented *blank* line is consumed by the block; a truly empty line ends it.
- A `#` or `;` line **between matchers is a regex to hledger, not a comment.** We refuse to edit such
  a block rather than cement a reading the author almost certainly did not intend.
- Field-assignment values run **verbatim to end of line** — no comment stripping. `account2 x ; note`
  assigns the literal `x ; note`.

An `if` **table**'s extent is terminated by a blank line, so when something is placed after a table
that ran to EOF, the renderer supplies that blank line. Without it the new rule would be read back as
another table row.

## Security

A `PUT` keyed by a client-supplied path is a **write-anywhere primitive** — strictly worse than the
read oracle the SEC-6 include guard defends against. Five independent layers; none leans on another.

1. **Syntactic id validation**, before any filesystem call. No `..`, no leading `/`, no `\`, no `:`,
   no control characters, must end `.rules`. A hostile id never reaches the filesystem, and
   400-vs-404 is decided on syntax rather than existence.
2. **Discovery-set membership.** `Discovery::resolve` is the *only* id → path resolution, by exact
   string equality against a set scanned in that request. `root.join(id)` exists nowhere in either
   crate. `RulesPath` has a private field and no public constructor, so "you can only write to a file
   discovery returned" is enforced by the type system.
3. **Confinement, file type, symlinks.** Confined to the journal's own directory (the same root
   `include` is confined to, via the shared `parse::confine`). Symlinks are **refused outright**,
   which is stricter than `admit_include` and removes cycles and a TOCTOU class. Regular files only —
   a FIFO named `x.rules` would otherwise hang the request forever on `read`.
4. **Content provenance.** Every byte written is either a byte read from that file moments earlier or
   renderer output over validated typed fields. Structural, not a promise: the wire's item type has
   **no raw-text variant**.
5. **No path is ever echoed.** Errors quote only the caller's own id; every resolution failure returns
   the same string, so the route is not an existence oracle.

**The one to remember:** hledger's `source` directive accepts `| CMD`, which `hledger import` runs
through the user's shell. So `source`, `archive` and `include` can be kept, moved or deleted — never
written. Without that rule this endpoint would be a remote-code-execution primitive. For the same
reason the test suite **never runs hledger against a user's file**, only against fixtures we author.

## Concurrency

Each response carries a `revision` — a fingerprint of the file's **raw bytes**. A save must echo it,
and it is re-checked immediately before the write, so a file edited in vim underneath you produces a
409 rather than a silent clobber. Raw bytes, never rendered text: a rendered hash is blind to exactly
what we preserve but do not model (trailing whitespace, CRLF, a table's interior), which is most of a
real rules file.

Rules files stay out of the journal watcher, the snapshot and the ETag. A `.rules` change invalidates
no transaction, so routing it through a reload would cost a full reparse for nothing.

## Running the checks

```sh
just rules-check           # every fixture is a rules file REAL hledger accepts
cargo test -p ledgeline-core --test rules            # round-trip + isolation + properties
cargo test -p ledgeline-core --test rules_security   # the scan's guards
cargo test -p ledgeline-server --test rules_endpoints
just snapshot-rules-wire   # ONLY when the wire contract changed on purpose

# New Transactions
cargo test -p ledgeline-core --test convert_ofx      # OFX/QFX, incl. the entity matrix
cargo test -p ledgeline-core --test convert_tabular  # delimited + spreadsheet
cargo test -p ledgeline-core --test matching         # rules-file scoring
cargo test -p ledgeline-core --test sort             # format-preserving date sort
cargo test -p ledgeline-core --test journals         # target ranking, by content only
cargo test -p ledgeline-server --test prefs          # prefs store + hledger resolution
cargo test -p ledgeline-server --test git_commit     # the git safety net
cargo test -p ledgeline-server --test import_endpoints  # the /api/import/* routes
```

Four opt-in checks shell out to a real binary and are therefore **not** part of `cargo test`,
which stays hermetic:

```sh
LEDGELINE_HLEDGER_RENDER_CHECK=1 cargo test -p ledgeline-core --test rules_hledger_render
LEDGELINE_HLEDGER_MATCH_CHECK=1  cargo test -p ledgeline-core --test matching
LEDGELINE_HLEDGER_SORT_CHECK=1   cargo test -p ledgeline-core --test sort
LEDGELINE_HLEDGER_IMPORT_CHECK=1 cargo test -p ledgeline-server --test import_endpoints
```

`import_endpoints`' gated half is where the whole import *sequence* is proved: that the
proposed entries come from stdout and the status line from stderr, that a row `.latest` would
silently drop is reported, that a commit writes exactly one CSV and one journal, and — the one
that matters most — that **balance assertions do not aggregate across two `-f` flags** while the
concatenation does. That last one is a silent wrong answer rather than an error, so it is the
only bug in this feature a user would never notice.

`rules_hledger_render` is the only check that proves **our renderer emits syntax hledger
accepts** — the round-trip tests only prove we do not damage what we did not touch. `sort`'s
variant proves every sorted output still passes `hledger check --strict ordereddates`, and
`matching`'s compares our scoring against real `hledger print -O json` output.

`git_commit` is gated differently — **skip-if-absent, not opt-in.** git is present on every
machine that can clone this repo, and the property it protects (*an import never touches your
unrelated work*) is too important to sit behind an environment variable nobody exports. A test
that silently never runs is worse than no test.
