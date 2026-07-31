# Imports: the CSV rules-file editor

How Ledgeline finds, reads, presents and edits hledger CSV import rules files (`*.rules`).

Scope today: **discover, present, edit, save.** Not in scope yet: running an import, generating a
rules file from a CSV, writing a chosen category back as a new rule, or converting PDF / QIF / OFX /
XLS to CSV. The model below is shaped so each of those slots in additively.

Raw text editing is deliberately *not* offered — that is what a terminal is for. The GUI covers
preferences, the row mapping, the two default accounts, and an ordered list of `if` rules.

## Where the code is

| Path | What it holds |
| --- | --- |
| `crates/ledgeline-core/src/rules.rs` | The format-preserving document model: parse, classify, render, `EditPlan`/`apply`/`verify` |
| `crates/ledgeline-core/src/rules/discovery.rs` | The directory scan, `RulesPath`, `Discovery::resolve`, the CSV preview |
| `crates/ledgeline-server/src/rules_api.rs` | `/api/rules`, `/api/rules/{*id}`, `/api/rules-preview/{*id}` |
| `web/src/lib/imports/` | Pure form model, reorder, date-format catalogue, store, UI |
| `fixtures/rules/` | The corpus. See its `README.md` — it explains what each fixture is *for* |

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

LEDGELINE_HLEDGER_RENDER_CHECK=1 cargo test -p ledgeline-core --test rules_hledger_render
```

That last one is the only check that proves **our renderer emits syntax hledger accepts** — the
round-trip tests only prove we do not damage what we did not touch. It is opt-in so `cargo test`
stays hermetic.
