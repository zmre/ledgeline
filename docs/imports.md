# Imports

Three surfaces behind one nav item, each with its own tab:

- **New Transactions** — drop a statement file, convert it to CSV, match it against your rules
  files, run `hledger import`, and check the result before anything is written.
- **Edit Rules** — find, read, present and edit hledger CSV import rules files (`*.rules`).
- **Account Aliases** — the `alias` directives in your journal, and the mapping table an import
  hands to `hledger --alias`. See § Account aliases, and § The two homes an alias can live in for
  why the same mapping in an `hledger.conf` is a different thing.

Rules-file scope: **discover, present, edit, save, and draft a new one from a dropped CSV.** Still
not in scope: writing a chosen category back as a new `if` rule. The model below is shaped so that
slots in additively.

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
| `crates/ledgeline-core/src/rules/generate.rs` | Drafting a NEW rules file from a CSV: column guessing, `date-format`, `decimal-mark` |
| `crates/ledgeline-core/src/convert/` | Statement preprocessors: `ofx`, `spreadsheet`, `delimited`, shared `encoding` |
| `crates/ledgeline-core/src/sort.rs` | Format-preserving date sort of a journal file |
| `crates/ledgeline-core/src/journals.rs` | Ranking candidate target journals, by content only |
| `crates/ledgeline-core/src/aliases.rs` | `alias` directives: forwarding them to `--alias`, and the one-line-wide span editor |
| `crates/ledgeline-core/src/hledger_conf.rs` | `hledger.conf`: reading its `--alias` options, and the escaping rule for writing one |
| `crates/ledgeline-server/src/rules_api.rs` | `/api/rules`, `/api/rules/{*id}`, `/api/rules-preview/{*id}`, `/api/rules-create` |
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

## Two journals: the one we write, and the one we reckon against

> **The file an import is written TO is not the file its balances are computed FROM.**

An import appends to one file — `2026/2026.journal`, say. A balance is a property of the whole
tree, so it is computed from the **root**, the journal Ledgeline was opened with. In a
single-file journal the two are the same file and nothing here matters. In every split layout
they are not, and treating them as one produced both of the bugs below.

`import_api::Plan` therefore has no field called `journal`. It has `target` (the write
destination) and `root_journal` (what everything is reckoned against), because one field named
for neither job is exactly how they got confused.

### Why the import disables balance assertions

> **`hledger import` runs with `--ignore-assertions`. Removing it does not restore safety.**

`import` is the only invocation that reads the target file *alone*: one `-f`, naming a fragment.
A balance assertion inside an included fragment is not a check we are choosing to switch off — it
is a check that is **structurally incapable of being correct in that context**, because the
balance it asserts accumulates through files hledger was never asked to read. Verified against
1.52 on the layout in `fixtures/import/layouts/split-year-assert/`:

```console
$ hledger -f main.journal check                 # the tree is fine
$ hledger -f 2026/2026.journal import --rules bank.csv.rules bank.csv
hledger: Error: …/2026/2026.journal:13:38:
  13 |     assets:bank:checking              $0 = $900.00
Balance assertion failed in assets:bank:checking
```

The start-of-year assertion carrying the prior year's closing balance is the `hledger close
--assert` shape and a very common way to keep books, so this aborted the import for a whole class
of ordinary journals. The place that assertion is meaningful is the root, where it is still
evaluated and still protects the user.

Two things the flag does not cost, both checked against the binary:

- **Assertions a rules file generates are deferred, not lost.** A `balance` field still emits
  `assets:bank:checking  $-20.00 = $880.00` into the proposed entries, `-I` leaves that text
  alone, and it is checked at the root from then on.
- **Nothing that was ever checked stops being checked.** hledger does not evaluate CSV-derived
  assertions during an import at all — importing a `balance`-field CSV asserting `$880.00` into a
  journal holding `$100.00` exits zero. The only assertions suppressed are the target fragment's
  own.

The flag goes on **that one invocation and no other**, the same discipline `--alias` follows, and
two unit tests hold the line in both directions: one asserts the import carries it ahead of the
subcommand, the other asserts no balance invocation does.

### Why the balance verifications read the root

`verify_balance` and `check_assertion` send `include <ROOT>` plus the proposed entries down one
pipe — never two `-f` flags (fact 3), and never the target. `include <root>` + proposed is
precisely *what the tree will look like once this import lands*: the proposed entries are hledger's
own dry-run stdout, which by definition is not in the target yet, and the target is reached through
the root's own `include`, so nothing is counted twice.

Reading the target instead produced two failures, and the second is the worse one:

| | what the user saw |
| --- | --- |
| target holds an assertion | `computed: ""` and a **refused commit**, quoting hledger's complaint about a start-of-year line the user never typed |
| target holds none | a plausible balance that is silently wrong by whatever the other files hold — `$2043.55` where the truth is `$2038.55`, `matches: false`, and no error anywhere |

Both are pinned in `import_endpoints.rs` against the committed
`fixtures/import/layouts/split-year-assert/` tree, with the right number *and* the wrong number
asserted — equality with the wrong one is the bug, so a test that only checked the right one
would pass whenever they coincided.

Two reads deliberately stay on the target, and they are not oversights:

- **The assertion's date.** It is appended to the target, so it is dated after the target's own
  last entry; the root of a split layout holds no transactions at all and could not supply one.
- **The post-import ordering check.** Date order is a per-file property and `sort::plan` is our
  own pure pass over the file's text — it runs no subprocess, so it cannot fail for an assertion
  reason.

## The `skip` a rules file already says

> **`skip N` is counted against the file the BANK produced. Our conversion strips the preamble
> out from under it, so every copy hledger reads is padded back into that frame.**

hledger has no header concept. `skip N` discards N *records* and every record after them is
data, so a rules file written for a two-line preamble plus a header says `skip 3`. Ledgeline
converts that download to a canonical CSV with the header on line 1 — and hands the same rules
file the same statement. Verified against hledger 1.52:

| the file hledger reads | `skip` | transactions |
| --- | --- | --- |
| the raw download (2 preamble lines, header, 3 rows) | 3 | 3 |
| our converted CSV (header, 3 rows) | 3 | **1** |
| our converted CSV (header, 2 rows) | 3 | **0** |

Exit code **0** every time, nothing on stderr. The user's own correct rules file, their own
statement, and a silently truncated import reported as a successful one. On a short statement it
is total loss; on a long one it is worse, because transactions do arrive.

`convert::align_to_skip` reconciles the two frames by prepending **`skip - 1`** empty records:
one for the header hledger still has to spend a skip on, and the rest standing in for the
preamble that is gone. `skip 0` and `skip 1` are already correct against a header-on-line-1 file
and are returned byte for byte, so nothing about an import that works today changes.

Three details are measurements rather than choices:

- **The padding cannot be blank lines.** hledger discards a truly empty line *before* `skip`
  counts, so blank padding is invisible to it and the transactions are still eaten. It has to be
  a record that is empty but present: `,,` — the same shape a spreadsheet's own CSV export writes
  for a trailer row. A one-column table pads with `""`, because there RFC 4180 gives an empty
  record and an empty line the same bytes.
- **The padding is as wide as the table**, so the file still reads as a rectangle to a person who
  opens it, and so our own re-read counts those lines as blank *rows* rather than as a ragged
  margin.
- **It is applied per rules file, not once.** Candidate scoring runs each candidate's rules file
  against the same statement and they do not agree on a `skip`, so `Stage::aligned` writes one
  copy per distinct value. Before this, a genuine `skip 3` candidate was scored against a file it
  could not read — and scored **1.0** on the one transaction it managed to reach, which is a
  perfect mark for importing a third of the statement.

Where it lands, and why the list is exactly this:

| What reads it | How it is aligned |
| --- | --- |
| candidate scoring's `print` | `Stage::aligned(candidate's skip)` |
| the dry-run, the dedup measurement, the alias diffs, the balance preflight | `Stage::materialize(…, plan.skip)` |
| the commit's `import` and `--catchup` | the CSV written to the user's destination |

The last row is the one worth stating plainly: **the CSV a commit saves carries the padding.**
It has to. Those two invocations read the destination file — they must, or hledger would key
`.latest.NAME` to a name in a temp directory that is about to be deleted — so an unpadded file
there is a commit that imports the wrong rows and reports the number it got as the number there
was. It is also the file the user keeps and re-imports later, from this screen or from a
terminal, with that same rules file; padding is what makes those two agree. `save-csv` writes the
canonical CSV unpadded, because that route has no rules file and so no `skip` to align to.

The **canonical** staged CSV (`Stage::data()`) is never padded. It is what the preview is
rendered from and what `save-csv` keeps, and `matching::prefilter` and `sample_dates` work on the
in-memory `Tabular` rather than on any file — they already compensate in that frame, and a second
compensation there would be a double one.

`fixtures/import/delimited/preamble.csv` + `preamble.csv.rules` are the pair, and both checks
that use them assert the **wrong** number as well as the right one:
`converted_and_aligned_imports_what_the_raw_download_does` (core, `LEDGELINE_HLEDGER_CONVERT_CHECK`)
and `a_rules_files_own_skip_still_counts_after_the_preamble_is_stripped` (server,
`LEDGELINE_HLEDGER_IMPORT_CHECK`, covering the candidate card, the dry-run and the commit).

## hledger proposes; Ledgeline appends

> **`hledger import` never writes a user's journal. It previews, and it remembers.**

The commit runs three steps where the obvious design runs one:

1. `hledger import --dry-run` → the deduped proposal, on stdout;
2. Ledgeline appends that text, unaltered, with `edit::atomic_write`;
3. `hledger import --catchup` → hledger records `.latest.NAME` itself.

### Why

Because it makes **the preview the bytes.** The text the dry-run route returned is the text the
commit appends — the same string, not a second rendering of the same proposal that could drift
from the one the user approved on screen. Everything below rests on that: the byte-compatibility
oracle can compare whole files, and the entry count is taken from the text that actually landed.

Splitting the write also means Ledgeline owns the one atomic operation it needs to own, and hledger
keeps owning `.latest` — the dedup state stays a thing hledger maintains in hledger's format,
rather than something Ledgeline now has an opinion about.

### Commodity style — why imported amounts keep hledger's own spelling

> **Read this before proposing to re-print imported entries in the journal's declared style. It
> was built, it worked, and it was removed on purpose.**

A journal declaring `commodity $1,000.00` receives `$165.2` and `$-405` where its author writes
`$165.20` and `$-405.00`. That is real, it is mildly annoying, and it is **accepted**.

The facts, all verified against hledger 1.52:

- `hledger import` applies a declared commodity's **digit-group separator** and *not* its **decimal
  places**, and no flag on `import` changes that. `-c/--commodity-style` makes no difference to its
  output; `--round` is rejected outright:

  ```console
  $ hledger -f main.journal import --dry-run --rules bank.csv.rules bank.csv
      assets:bank:checking           $-405        # CSV said `-405`
      assets:bank:checking          $165.2        # CSV said `165.2`
      assets:bank:checking       $12,345.6        # separator applied, places not
  $ hledger -f main.journal import --round=soft …
  hledger: Error: Unknown flag: --round
  ```

- **Re-styling does work.** `hledger -I -f - print --round=soft`, fed the tree's own `commodity`
  directives followed by the proposal, produces exactly the wanted `$165.20` and `$-405.00`. It was
  implemented, tested and working (`restyle.rs`, commit `b53323b`) before being taken back out.
  `soft` rather than `hard`, because `hard` rounds — hledger's own `--help` says it "can unbalance
  transactions" — and would write a `12345.678` statement row into the books as `$12,345.68`.

**It was removed because prepending those directives changes how the entries *parse*, not merely
how they print.**

```console
$ hledger -f 2026/2026.journal import --dry-run …    # the fragment declares nothing
    assets:bank:checking        EUR165.2
$ printf 'commodity 1.000,00 EUR\n\n…' | hledger -I -f- print --round=soft
    assets:bank:checking     1.652,00 EUR             # exit 0
```

Ten times the money, silently. It is reachable precisely here because the import reads a
**fragment** — the file that does not carry the declaration — so hledger writes the CSV's own `.`
while the tree declares `,`.

A value-comparison guard *did* catch it: parse both texts bare, compare dates, status, code, tags,
accounts, amounts and assertions, and discard the restyle on any disagreement. The guard worked.
It was still not worth it. On a multi-commodity book the failure mode is not a tidy tenfold on a
bank balance but a **mangled share quantity**, and a cosmetic gain does not justify carrying that
class of risk at all — not even behind a guard that currently holds. (The motivating user's own
rules file carries a commented-out `#currency $ # messes up stock quantities`; they had already met
the shape of this problem once.)

So: imported amounts keep hledger's own spelling.
`imported_amounts_keep_hledgers_own_spelling_not_the_declared_style` asserts it, so the behaviour
cannot drift back by accident.

If a future implementer wants the declared style, the safe direction is to make the **CSV carry
correctly-scaled amounts** — a rules-file or preprocessor concern, where the number is still just a
number — rather than re-printing finished entries under directives they were not parsed with.

### The catch-up, and what happens when it fails

`--catchup` records `.latest.NAME` without appending anything. Verified: the journal is
byte-identical afterwards, the state file is byte-identical to the one a writing import leaves
(including the repeated same-date lines that encode how many records share the newest date), and a
following dry-run reports no new transactions.

If it fails after the append, the journal would hold the entries while the marker still pointed at
the previous import — and the **next** import of that statement would propose them again. So a
failed catch-up **rolls the journal back** to the bytes read moments earlier under the write mutex
and reports the whole commit as failed, which is the same all-or-nothing property
`preflight_assertion` gives a mistyped balance. A roll-back that itself fails says so in as many
words and names the duplication risk; that is a state a person has to be told about.

### Byte compatibility with hledger's own append

Pinned, because taking over someone else's write is not the place to improve on it. A dry-run
writes its proposal followed by a **blank line**; a real import appends a leading newline and the
same text with that blank line removed:

```text
stdout   "2026-02-01 A\n    …$-405\n\n2026-02-03 B\n    …$165.2\n\n"
appended "\n2026-02-01 A\n    …$-405\n\n2026-02-03 B\n    …$165.2\n"
```

Note what hledger does *not* do: it never checks whether the file already ends in a newline, so a
journal saved without one gets the first imported transaction on the line straight after the last
posting. Reproduced exactly. An empty proposal appends nothing at all.

`the_appended_bytes_are_hledgers_own_append` proves it end to end: the same statement is imported
into a copy of the same journal by a real `hledger import`, and the two files are compared byte for
byte. With nothing re-styling the proposal in between, that now describes **every** journal rather
than only the ones declaring no commodity style.

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

An `if` block is editable only if every matcher is plain or a **plain line-prefix `&`
AND-continuation** (no `!`, no `&&`, and no leading `&` on the *first* matcher line), has no match
group, and does not begin with `;`/`#`/`*`; every body line is a known field assignment; and there is
no `skip`/`end`. Everything else — `if` tables, negated or `&&` matchers, match groups, control flow
— renders as a dimmed read-only card that says *why* it is locked, and can still be moved.

A block's matchers are therefore an **OR of AND-groups**: a plain line opens a new OR branch and each
`&` line below it is AND-ed onto that branch. `if\nA\n& B\nC\n& D` selects `(A and B) or (C and D)`.
The `&` is grammar, never content — the model carries the AND as *nesting* (`groups[].matchers[]` on
the wire), so no matcher pattern anywhere can contain a combinator.

Order is semantics: every matching block applies and the **last** assignment to a field wins. The UI
says "later matches win" for that reason.

### Grammar facts that are easy to get wrong

Verified against hledger 1.52's `RulesReader.hs` **and the binary**, not just the manual:

- A **matcher line must start at column 1** with a non-space character. A body line must be indented.
  An indented *blank* line is consumed by the block; a truly empty line ends it.
- A `#` or `;` line **between matchers is a regex to hledger, not a comment.** We refuse to edit such
  a block rather than cement a reading the author almost certainly did not intend.
- Field-assignment values run **verbatim to end of line** — no comment stripping. `account2 x ; note`
  assigns the literal `x ; note`.
- The `&` AND-prefix takes **optional** whitespace: `&B`, `& B` and `&\tB   ` are one matcher, and
  the pattern is trimmed at both ends. It works under an inline `if X` header as well as under a
  bare `if`. An **indented** `& B` is not a matcher line at all and hledger rejects the file for it.
- `&` is a prefix only at the head of a matcher line. In `%description &COFFEE` the `&` is a literal
  ampersand in the regex, so that matcher hits nothing containing plain `COFFEE`.
- A **leading `&` on the first matcher line** (`if\n& X`) is accepted by hledger and is a no-op — it
  imports exactly what `if\nX` does. We keep it opaque rather than promise to preserve it.
- A leading `&&` really is an AND join to hledger, same as `&`. We still refuse it, because on one
  line the same bytes may be two literal ampersands inside a regex and telling those apart needs
  hledger's own parser.

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

### Creating a file: the one place a client string becomes a path

> **Layer 2 cannot serve a CREATE.** `Discovery::resolve` only ever returns a file the scan already
> found; a file that does not exist yet cannot be found. So `Discovery::resolve_new` performs the
> **only `root.join(id)` in either crate**, and every guard below is what earns it.

1. **Shape**, before any filesystem call — deliberately a *second* copy of the question
   `validate_id` asks, because neither layer may assume the other ran.
2. **Discoverability.** No hidden component, and none in the scan's `SKIP_DIRS`. Not a security
   guard so much as a coherence one: creating a file the scan will never list would write something
   the user cannot then open, which is worse than refusing.
3. **Confinement**, via the same `parse::confine` everything else here uses.
4. **A real, non-symlink parent directory.** **No directory is ever created** — a rules file goes
   beside a journal that already exists.
5. **Nothing already at the name.**

Guard 5 is *not* what makes it safe, and must not be read as if it were: it expires the moment it
returns. The write is `create_new` — `O_EXCL` — so the refusal is the **kernel's**, decided
atomically at the open. `edit::atomic_write` is deliberately not used: its rename-over-the-top is
exactly the property that makes it right for a save and wrong for a create.

Two of the four refusals collapse into the ordinary `404`. "Resolves outside the root" and "that
directory is not there" are answers *about the filesystem*, and a route that told them apart would
report whether `/etc/ledgeline/` exists. "Already exists" is safe to report as itself, because it is
only reachable for a confined, non-hidden `*.rules` name below the root — precisely the set
`GET /api/rules` already publishes.

Content provenance is unchanged, and that is the point of routing the write through the ordinary
`PUT`: a create is a `revision: ""` request over the same typed item vocabulary, rendered by the
same renderer. `POST /api/rules-create` itself **writes nothing at all** — it drafts, and the user
saves. Drafting a plausible file and writing one stay separate, separately-testable operations.

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
cargo test -p ledgeline-core --test rules_generate   # drafting a new file from a CSV
cargo test -p ledgeline --test rules_endpoints
cargo test -p ledgeline --test rules_create_endpoints  # the create boundary
just snapshot-rules-wire   # ONLY when the wire contract changed on purpose

# New Transactions
cargo test -p ledgeline-core --test convert_ofx      # OFX/QFX, incl. the entity matrix
cargo test -p ledgeline-core --test convert_tabular  # delimited + spreadsheet
cargo test -p ledgeline-core --test matching         # rules-file scoring
cargo test -p ledgeline-core --test sort             # format-preserving date sort
cargo test -p ledgeline-core --test journals         # target ranking, by content only
cargo test -p ledgeline --test prefs          # prefs store + hledger resolution
cargo test -p ledgeline --test git_commit     # the git safety net
cargo test -p ledgeline --test import_endpoints  # the /api/import/* routes
```

Five opt-in checks shell out to a real binary and are therefore **not** part of `cargo test`,
which stays hermetic:

```sh
LEDGELINE_HLEDGER_RENDER_CHECK=1 cargo test -p ledgeline-core --test rules_hledger_render
LEDGELINE_HLEDGER_MATCH_CHECK=1  cargo test -p ledgeline-core --test matching
LEDGELINE_HLEDGER_GENERATE_CHECK=1 cargo test -p ledgeline-core --test rules_generate
LEDGELINE_HLEDGER_SORT_CHECK=1   cargo test -p ledgeline-core --test sort
LEDGELINE_HLEDGER_LAYOUT_CHECK=1 cargo test -p ledgeline-core --test journals
LEDGELINE_HLEDGER_IMPORT_CHECK=1 cargo test -p ledgeline --test import_endpoints
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

`journals`' variant is the corpus check: every root under `fixtures/import/layouts/` passes
`hledger check --strict` as committed, and — separately — `split-year-assert/`'s target file
*fails* on its own, for the assertion reason, while its root passes. Those two are asserted as a
pair on purpose. "The fragment fails" alone could mean the fixture is broken; "the root passes"
alone could mean the fragment was harmless. Together they are the premise
`--ignore-assertions` rests on, and if hledger ever stops behaving that way this is where it
surfaces rather than in someone's books.

`git_commit` is gated differently — **skip-if-absent, not opt-in.** git is present on every
machine that can clone this repo, and the property it protects (*an import never touches your
unrelated work*) is too important to sit behind an environment variable nobody exports. A test
that silently never runs is worse than no test.
