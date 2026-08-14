# Imports

Three surfaces behind one nav item, each with its own tab:

- **New Transactions** — drop a statement file, convert it to CSV, match it against your rules
  files, run `hledger import`, and check the result before anything is written.
- **Edit Rules** — find, read, present and edit hledger CSV import rules files (`*.rules`).
- **Account Aliases** — the `alias` directives in your journal, and the mapping table an import
  hands to `hledger --alias`. See § Account aliases, and § The two homes an alias can live in for
  why the same mapping in an `hledger.conf` is a different thing.

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
| `crates/ledgeline-core/src/aliases.rs` | `alias` directives: forwarding them to `--alias`, and the one-line-wide span editor |
| `crates/ledgeline-core/src/hledger_conf.rs` | `hledger.conf`: reading its `--alias` options, and the escaping rule for writing one |
| `crates/ledgeline-server/src/rules_api.rs` | `/api/rules`, `/api/rules/{*id}`, `/api/rules-preview/{*id}` |
| `crates/ledgeline-server/src/alias_api.rs` | `/api/aliases`, `/api/aliases/{*journalId}` |
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

- **Every hledger invocation passes `--no-conf`.** See § No hledger we run reads a config file —
  this one is a security property, not a tidiness one.
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

## No hledger we run reads a config file

> **Every invocation starts with `--no-conf`**, added in `Invocation::argv` — the single point
> every argument vector passes through on its way to `Command::args`.

hledger 1.40 introduced automatic config files, and the search is not opt-in: `hledger.conf` in the
working directory **or any directory above it**, then `$HOME/.hledger.conf`, then the XDG config
dir. `Invocation` deliberately sets no working directory, so without the flag we would inherit
whatever config happened to sit above wherever Ledgeline was launched — a file this application
never chose and cannot see.

That is not untidiness. **A config file can replace the command.** Verified against hledger 1.52,
with a `hledger.conf` whose entire content is the bare word `balance`:

```console
$ hledger import --dry-run -f rt.journal --rules ms.csv.rules ms.csv
hledger: Error: Unknown flag: --dry-run
* while parsing the following args, final command line:
*  balance import --dry-run -f rt.journal --rules ms.csv.rules ms.csv
```

Our command was demoted to an argument of somebody else's. hledger's manual states the rule: a first
word in the general section that does not begin with a dash is taken as the command, overriding the
command line. That example errors, but the shape generalises — a config could inject `--auto`,
`--forecast`, `-b`/`-e`, `--depth` or `--alias` into invocations whose output we *parse* and whose
results we *append to the user's journal*. Explicit flags do beat config ones (`-O json` on our
command line beats `-O csv` in a config, verified), so the danger is not only breakage: it is that
our subprocesses stop being determined by this process.

Two consequences worth stating:

- **The flag goes first, ahead of the subcommand**, because a config's injected command word is
  prepended to the argument list.
- **The version probe is the one exception, and only on retry.** `--no-conf` did not exist before
  1.40, so against a 1.39 it is an unrecognised flag and the probe would learn nothing — costing the
  user the actionable "hledger 1.39 is older than 1.40" banner. So an unparseable answer is retried
  once without the flag. Safe exactly there: it is only reached on a binary that has just rejected
  the flag (i.e. one too old to read a config at all), and the only use of the result is a
  comparison against the minimum version.

Tested as an **argument-level lint** (`import_api::tests::every_import_invocation_disables_config_files`)
plus a companion that counts `.invoke(` call sites, so a new invocation cannot be added without one.
A behavioural test would need a hostile `hledger.conf` planted above the test runner's working
directory — which is this repository — and would break every other test in the run. The flag is also
observed arriving at a real process in `tests/prefs.rs`.

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

## Account aliases

A statement's account column is frequently the bank's own words. A real Morgan Stanley export
says `PW Roth IRA - 3077` where the journal says `assets:morganstanley:pw-roth-ira`, and a rules
file that interpolates the column (`account1 %acct`) writes the bank's words straight into the
ledger. The usual workaround is a `source ./x.csv | ./clean.py` line adding a mapped column —
which this codebase **will never run** (see § Security). So the mapping is done natively instead.

### The finding this rests on

Verified against hledger 1.52, in both directions:

- An `alias` directive sitting in the target journal **does not reach the CSV** during
  `hledger import`. The account comes through unmapped.
- `--alias` **does**, in both `OLD=NEW` and `/REGEX/=REPL` forms; several compose; and
  `import --dry-run` applies them too, so the preview shows the final names for free.

So Ledgeline reads the journal's own `alias` directives and hands them to hledger as `--alias`.
The mapping is one the user already wrote down; the only thing added is delivery.

### The two homes an alias can live in, and what each affects

This is the distinction the whole command-line-parity feature turns on. The two look like the same
thing and are not:

| | applies when hledger READS the journal | applies to an `import`'s CSV |
| --- | --- | --- |
| `alias` directive in the journal | yes | **no** |
| `--alias` in `hledger.conf` | yes | **yes** |

The consequence is a silent divergence between the GUI and the terminal. Same statement, same rules
file, same journal, two different sets of account names depending on which tool the user reached
for — verified: a plain `hledger import` writes `PW Roth IRA - 3077:cash` where Ledgeline writes
`assets:morganstanley:pw-roth-ira:cash`.

So Ledgeline **reads `hledger.conf` itself** and merges its `--alias` values into the ones it
forwards. Note *reads*, not *delegates*: we never pass `--conf`. § No hledger we run reads a config
file exists precisely so no external file steers our subprocess, and a config may hold `--depth` or
`-b`/`-e`, which would change output we parse. Taking only the `--alias` values we understand is the
narrow version of the same benefit.

Where we look, and where we deliberately do not:

- **Upward from the journal's own directory**, nearest first — because that is where a user runs
  hledger for these books, whereas a server process's working directory is an accident of how it
  was launched.
- **Not `$HOME/.hledger.conf`, and not the XDG config dir**, though hledger falls back to both.
  Those are outside the tree the user pointed us at. The cost is stated where it is felt: the
  divergence notice names the file it found, or says there was none.

**Order: the config's aliases go first.** `--alias` options compose left to right and the first to
match an account wins, so config-first means that wherever the two disagree the config's answer
stands — and the config's answer is the terminal's. Journal-first would make Ledgeline disagree with
the command line in a second, opposite direction.

### The whitespace trap, and the escaping rule

> **hledger's config parser splits on whitespace and IGNORES QUOTES.** There is no escape for a
> space.

This is undocumented upstream as far as we could find, and it costs real time. All three of these
fail with a parse error:

```
--alias="/^PW Roth IRA - 3077/=assets:morganstanley:pw-roth-ira"
--alias='/^PW Roth IRA - 3077/=assets:morganstanley:pw-roth-ira'
--alias=/^PW Roth IRA - 3077/=assets:morganstanley:pw-roth-ira
```

The workaround is that the pattern is a **regex**, and `.` matches a literal space:

```
--alias=/^PW.Roth.IRA.-.3077($|:)/=assets:morganstanley:pw-roth-ira\1
```

`hledger_conf::conf_argument` is the whole of our answer, in order:

1. **No whitespace anywhere ⇒ write it verbatim**, in whichever form the user declared. Provably
   identical to what we forward, because it is the same string.
2. **Whitespace in the replacement ⇒ refuse.** An account name is matched literally; there is no
   wildcard to stand in for its space, and a mapping that looks installed but never matches is
   worse than a visibly missing one.
3. **Whitespace in a regex pattern ⇒ substitute**, one `.` per whitespace character (never one per
   run — `.` matches exactly one character). Refused when the pattern also holds `[`, where a `.`
   is an ordinary dot, or `\`, where substituting beside somebody else's escape is a guess.
4. **Whitespace in a plain pattern ⇒ convert to a regex.** A plain alias matches the whole account
   name or a prefix ending at a `:` (verified: it rewrites `a` and `a:sub` and leaves `abc` alone),
   which is exactly `/^PATTERN($|:)/` with `\1` carrying the boundary into the replacement. **Regex
   metacharacters are escaped first, then whitespace becomes `.`** — in that order, or a literal `.`
   in the bank's name silently becomes a wildcard.

**Two widenings, both deliberate.** A `.` matches any character, not only a space; and hledger's
regex aliases are case-insensitive where plain ones are not (both verified). A converted alias
therefore matches everything the original did and can match a little more. That is the price of
being expressible at all, which is why the resulting line is **shown on screen before it is
written** rather than after.

### Writing a config file

A new write target, and it gets the same discipline as every other one here:

- **The location is fixed**: `hledger.conf` in the journal's own directory. No component of the path
  comes from the client, so there is no handle to validate.
- **`$HOME` and the XDG config dir are never written**, though hledger reads both. They are outside
  the tree the user pointed us at, they affect every set of books on the machine rather than these
  ones, and a GUI quietly editing a home-directory dotfile is not a thing to do. A config in force
  *above* the journal's directory is reported and left alone; the fix creates one beside the journal
  instead, and says on screen that hledger uses the nearest file only, so the outer one will stop
  applying.
- **`parse::confine`** (the containment `include` and the rules scan share), then
  `symlink_metadata`: absent or a regular file. A symlink is refused, not followed.
- **Content provenance**: the request body carries a revision and *nothing else*. What to write is
  recomputed server-side from the journal's own `alias` directives, so the route cannot be used to
  put arbitrary text into a file hledger reads options out of.
- **Revision / 409**, re-checked immediately before the write, with the empty string as the revision
  of "there was no file". `edit::atomic_write` does the write.
- **A new option lands in the general section, not at EOF.** Everything after the first `[heading]`
  belongs to that command's section, so a line appended to a file ending in `[balance]` would be a
  balance-only option: present, plausible, and never applied to an import.

Column interpolation composes with it, which is what keeps the rules file small: with
`account1 %acct:cash`, a **prefix** alias rewrites the base and leaves `:cash` intact, so one
alias covers every subaccount rather than needing one per account × type.

### Where the flags go, and where they do not

> **`--alias` goes on every invocation that reads the CSV, and on no other.**

That is `import` (dry-run, the dedup measurement, and the real commit — all one argv builder,
`import_invocation`, with `--dry-run` as a parameter so the preview and the write **cannot** be
given different aliases) and the candidate-scoring `print`, so a rules card's sample accounts are
the accounts the dry-run will propose.

It is deliberately **not** on the balance verifications. Those read a journal, and a journal's
accounts are already the names it was written with — hledger applies the journal's own `alias`
directives when reading, as it always has. Adding `--alias` there would apply the mapping twice,
and a regex alias broad enough to match its own output would rewrite an account that was correct.

### Scope

hledger's aliases are positional and file-scoped: in force from their line to the end of their
file, flowing into anything `include`d after them, never back out, and stopped early by
`end aliases`. All three were checked against the binary.

`--alias` is global and has no way to express any of that, so Ledgeline forwards **every alias
not closed by an `end aliases` in its own file** — the set in force where an import appends. An
alias the user explicitly bounded is listed, is editable, and is never forwarded, with the reason
on screen. Which file an alias is in does not change whether it is forwarded; that is a
simplification, and the mitigation is that the whole set is shown rather than applied invisibly.

### Ledgeline reads aliases; it does not apply them

`Journal.aliases` is populated and `Journal` account names are left exactly as written. This is a
deliberate narrowing of `parse.rs`'s "reject rather than misparse" rule and it is argued at
length in that module's docs. In short: until now an `alias` line failed the **whole** journal,
so a user who has one could not open their books here at all — and an import cannot write into a
journal that will not parse. Applying the `/REGEX/` form would mean reproducing hledger's regex
dialect (Haskell `regex-tdfa`, POSIX ERE, case-insensitive, `\1` in the replacement) over every
account name in someone's books, and Rust's `regex` crate is a different dialect. A near-miss
would be a silent wrong answer; declining is a visible one. The Account Aliases tab says so in as
many words, and a test pins the sentence.

### What the editor refuses to model

Same discipline as the rules editor, one line wide. `AliasDoc` splices **only** the pattern and
replacement extents of the line being changed, so the `alias` keyword, the `=`, and every space
between them are re-emitted verbatim and a column-aligned block stays aligned with no alignment
code. `verify` then re-renders, re-parses, and requires every unedited alias line back
byte-identical; the server adds a whole-journal re-parse with the edited text in memory, and only
then writes.

A line it cannot promise to rewrite is presented **read-only** with the reason, exactly as an
unclassified rules construct is:

| Lock | Why |
| --- | --- |
| `commentLike` | A `;` or `#` on the line. `alias a = b ; note` declares the account **literally named** `b ; note` — hledger does not treat it as a comment (verified). Rewriting would cement a reading its author almost certainly did not intend. |
| `empty` | An empty pattern or replacement: no separator to re-emit, and no mapping. |
| `delimiter` | `=` inside a plain pattern (hledger splits at the first one, so the line does not say what it appears to) or `\/` inside a regex one. Re-escaping is a guess. |
| `control` / `tooLong` | A byte this module will not write, or too many of them. |

Reordering is not offered at all: aliases are positional, so a reorder is a semantic change
wearing a cosmetic's clothes. An inserted alias goes immediately after the file's last alias
line — the furthest-forward position that is provably still in force where an import appends and
provably unable to change what anything already in the file means — or at EOF when there is none,
or when an `end aliases` would otherwise swallow it.

### What the user sees before committing

A silent account rewrite immediately before an irreversible write is exactly what must not
happen, so the dry run reports the renames — **measured**, not inferred. The engine repeats the
same import with no `--alias` at all and diffs the two proposals, so the before/after pairs are
hledger's own answer rather than our reimplementation of its regexes. It is the technique
`skipped_by_dedup` already uses, and it costs one extra subprocess only when the journal declares
an alias. An empty rename list means the aliases matched nothing in this statement, and the
section stays hidden.

The same panel carries the **command-line divergence notice**, and it is measured the same way: the
engine repeats the import with exactly the aliases a config file supplies — which is exactly what a
terminal would apply — and diffs that proposal against the one on screen. With no config file the
two baselines are the same command line, so the second measurement is free; with one it costs a
third `--dry-run`, and only then.

Measuring it rather than comparing alias *strings* matters: a user who hand-wrote an equivalent
mapping into their config in a spelling of their own gets silence, which is the correct answer for a
config that already works. When they do diverge, the notice names the accounts that would differ,
shows the exact `--alias` lines the fix would write, lists any alias that cannot be expressed in a
config file with the reason, and offers the one-click fix. `web/src/lib/imports/aliasModel.ts` holds
every sentence and every decision (`parityNotice`, `parityWarning`, `parityFixLabel`,
`canInstallParityFix`); `ui/DryRunPanel.svelte` only renders them.

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
