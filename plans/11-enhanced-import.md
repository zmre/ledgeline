# WP-11: Enhanced Imports — the New Transactions tab

Read `docs/imports.md` first — it defines the span document model, the editable-vs-opaque
split, and the five security layers this WP must not weaken. Contracts referenced: WP-11
`RulesPath`/`DiscoveredRules`/`Discovery` (`crates/ledgeline-core/src/rules/discovery.rs`),
`Fingerprint`/`atomic_write` (`crates/ledgeline-core/src/edit.rs`), `parse::confine`
(`crates/ledgeline-core/src/parse.rs`), `recents` (`crates/ledgeline-server/src/recents.rs`).

Everything hledger-related below was verified empirically against **hledger 1.52** (the
version pinned in `flake.nix`), not taken from documentation. Where the docs and the binary
disagree, the binary won and the doc claim is called out.

## Scope

- An **Imports subnav** mirroring the Reports tab strip: **New Transactions** (first, default)
  and **Edit Rules** (today's page, moved wholesale).
- A **drop target + file picker** on New Transactions accepting `.csv`, `.tsv`, `.ssv`,
  `.ofx`, `.qfx`, `.xls`, `.xlsx`, `.xlsm`, `.xlsb`, `.ods`.
- A **preprocessor** normalising each of those to one in-memory CSV, with a preview of the
  first rows shown back to the user.
- **Rules-file candidate matching**: score every discovered `*.rules` against the incoming
  data, present ranked candidates, or say plainly that none fit.
- A form choosing the **rules file**, the **CSV destination**, the **target journal**, and an
  optional **statement balance**.
- **Import execution** by shelling out to `hledger`, dry-run first, with the proposed
  transactions and the balance reconciliation shown before anything is written.
- **Post-import date ordering**: detect with `hledger check ordereddates`, offer a
  format-preserving re-sort with a diff.
- A **preferences store** (first in the app) holding the resolved `hledger` path.

## Out of scope

PDF extraction — deferred to `docs/pdf-extraction.md`, and `.pdf` is refused with a specific
"not supported yet" message rather than a generic parse error. QuickBooks import — a
different shape entirely (account mappings, no intermediate CSV, no `hledger import`) and
gets its own WP. Generating a rules file from a CSV when nothing matches — the "Create
rules" button is stubbed to a disabled state with a tooltip; that is the next WP and is why
the candidate-scoring types below carry a `no_candidates` reason rather than just an empty
list. Editing raw journal text. Investment OFX (`INVSTMTRS`) — bank (`STMTRS`) and credit
card (`CCSTMTRS`) only; an investment statement is detected and refused by name.

## The five facts that shape this design

These are the empirical findings that drove every non-obvious decision. Verified locally
against hledger 1.52.

1. **`--rules-file` was renamed to `--rules` in hledger 1.40.** The old spelling survives as a
   hidden alias. We emit `--rules` and pin a minimum version.

2. **`import --dry-run` splits its streams cleanly.** The proposed transactions go to
   **stdout** as valid, re-parseable journal text; the `would import N new transactions from
   FILE:` status line goes to **stderr**. Dry-run writes no state file. This is the whole
   preview mechanism — we never scrape human-readable text for the entries.

3. **Balance assertions do NOT aggregate across multiple `-f` flags.** This is a silent
   wrong-answer, not an error:

   ```
   $ hledger -f main.journal -f new.journal balance assets:bank:checking
               $2945.05  assets:bank:checking          # correct

   $ hledger -f main.journal -f new.journal check      # assertion "= $2945.05"
   Balance assertion failed ... but the calculated balance is:  $1950.05
   ```

   The second file's assertions never saw the first file's balances. Verification must
   concatenate (`cat A B | hledger -f- check`) or use a temp wrapper of `include` lines.
   Both were verified to give the correct combined balance. **Never two `-f` flags.**

4. **A mismatched rules file frequently succeeds with garbage, exit 0.** Running a checking
   rules file against a credit-card CSV produced transactions with `income:unknown` postings
   and a posting with no amount at all — and `hledger check` was happy with it. Worse, a
   rules file lacking a `currency` rule yields *bare* amounts that form a separate commodity,
   so the import "succeeds" but the `$` balance never moves. **Parse success is not a
   matching signal.** Scoring must inspect the structured output (§ Rules matching).

5. **`hledger print` cannot be used to re-sort a journal.** It sorts by date, but it
   **flattens `include` directives into one file** and drops every `account`, `commodity`,
   `P` and standalone comment. Round-tripping a journal through it broke `check --strict`.
   The re-sort must be ours, over the existing span/`ropey` machinery.

Two further hazards the UI has to surface rather than hide:

- **`.latest.FILE` dedup silently drops back-dated rows.** State lives next to the *data
  file*, keyed to its name, and holds the newest imported date. A CSV containing a row older
  than that date is skipped with no mention. Verified: a 2026-01-20 row vanished from a
  dry-run whose `.latest` read `2026-02-05`.
- **hledger's own docs warn against importing one input file into different journals** —
  they share one `.latest` state file. The journal selector must warn when the chosen
  destination differs from the last one used for that CSV name.

## Preprocessor decisions

Each of these was settled by compiling and running the candidate, not by reading its README.

**OFX/QFX — hand-rolled, no crate.** The crate named `ofx` is OpenFX, the *visual effects*
plugin API, and is dead since 2019. `ofx-rs` 0.2.0 has the right architecture but **silently
corrupts payee names**: it drops entity references *and* the whitespace around them, so
`AT &amp;amp; T` parses as `ATT` and `caf&#233;` as `caf`, and it hard-errors on a raw
unescaped `&` — which is routine in real bank memos. In an app where `NAME` is what
categorisation rules match on, silent mangling is worse than a crash, so it is disqualified.
`ofxy` is GPL-3.0 and SGML-only; `qfx_parser` is a toy. We write our own (~1.5–2.4k LOC),
optionally over `sgmlish` (MIT, dormant but sound, and OFX 1.x is its stated use case).

What makes hand-rolling tractable, and the traps, all confirmed against real statements:

- **Leaf tags are unclosed, aggregates are closed**, and OFX never has mixed content. After
  an open tag, if the next non-whitespace byte is `<` it is an aggregate; otherwise the value
  runs to the next `<`. That single lookahead is the whole parser.
- **Header syntax and body syntax are independent.** ANZ ships an OFX 2.x XML header wrapping
  an SGML unclosed-tag body. Use the header *only* to choose the decoder; use one tolerant
  body parser for everything. **Never branch on the declared version.**
- **QFX is OFX plus `INTU.BID`/`INTU.USERID` in `SONRS`** — nothing else, and `.QBO` is
  identical. Dispatch on content, never on extension.
- **Do not route statement type on message set.** Citi delivers a credit card as
  `BANKMSGSRSV1/STMTRS` with `ACCTTYPE=CREDITLINE`, not `CCSTMTRS`.
- **Dates:** accept 8/10/12/14 digits with optional `[±H:TZ]`, offsets may be fractional
  (`[+5.5:IST]`), and the zone *name* is frequently wrong. **Keep the FI-local calendar date;
  do not normalise to UTC** or `20120720000000.000[-4:EDT]` lands on the previous day.
- **`TRNAMT` is not two-decimal** (`2500.0` occurs), and `NAME` truncates at exactly 32
  chars — which is why banks stuff the real payee into `MEMO`. Our field mapping must expose
  both.
- **`LEDGERBAL` lives inside `STMTRS`/`CCSTMTRS` after `BANKTRANLIST`.** Sign conventions for
  card balances are inconsistent between issuers, so we **reconcile rather than assume**.

**Arithmetic validation is a first-class feature, not a nicety.** Where a format gives us a
running balance or an opening/closing pair, we assert `opening + Σ(amounts) == closing` and
surface a loud failure on mismatch. This is the check that would have caught the `ofx-rs`
entity bug class immediately, and it is cheap. It also feeds the balance field in the UI.

**Spreadsheets — `calamine`, pinned.** Still uncontested for xls (BIFF2-8), xlsx, xlsm,
xlsb, ods. Pin the exact version: 0.36.0 silently began trimming XLSX whitespace, and the
de-facto lead opened an issue in July 2026 asking for maintenance help. Known traps: there is
**no access to number formats** (private module, request closed) so we get `Float(1234.56)`
and never `"$1,234.56"`; merged cells put the value top-left and `Empty` elsewhere; dates
need the `dates` feature and `as_datetime()`, never `as_f64()`; check `has_1904_epoch()` and
guard serial 60, the phantom 1900-02-29. Do **not** crib calamine's own
`examples/excel_to_csv.rs` — it emits raw date serials and does no quoting.

**Encoding — BOM sniffing first, then `chardetng`.** This order is mandatory, not an
optimisation: **`chardetng` cannot detect UTF-16 at all** and confidently returns
`windows-1252` for BOM'd UTF-16LE, which is exactly what Excel's "Unicode Text" export
produces. `encoding_rs_io::DecodeReaderBytesBuilder` does BOM sniffing and transcoding
correctly, so we do not hand-roll it. `chardetng` 1.0.0 changed its API (`new()` takes
`Iso2022JpDetection`, `guess()` takes `Utf8Detection`, `guess_assess` is gone) and most
advice online still describes 0.1. Two details that bite: `CHARSET:1252` means
**Windows-1252, not ISO-8859-1** — they differ precisely in `0x80–0x9F` where smart quotes
and em-dashes live — and `ENCODING:USASCII` should be decoded as cp1252 anyway. The `csv`
crate already strips a leading UTF-8 BOM itself; the widely-repeated advice to do it manually
is stale.

**Delimiter sniffing — ours, not a crate.** `csv-sniffer` is dead and panics on untrusted
input; `qsv-sniffer` declares itself superseded. `csv-nose` is sound in principle but
misfires on European files that have both a preamble and decimal-comma amounts, silently
returning a 2-column parse of a 3-column file. A row-consistency scorer (pick the delimiter
maximising modal field-count share, then detect a preamble offset) is ~40 lines and handled
every case we tried. Sniffing is bootstrap only — once a rules file exists, its `separator`
and `encoding` directives win.

## Interface contracts

### `crates/ledgeline-core/src/convert/mod.rs` (new)

The preprocessor boundary. Every input format collapses to one shape before anything else
in the pipeline sees it, so rules matching and preview have exactly one thing to handle.

```rust
/// A normalised tabular extract. Never contains a path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tabular {
    pub header: Option<Vec<String>>,
    pub rows: Vec<Vec<String>>,
    pub truncated: bool,
    /// Statement metadata a format volunteered. OFX gives us a closing balance for free;
    /// it pre-fills the balance-assertion field so the user does not retype their statement.
    pub statement: Option<StatementMeta>,
    pub notes: Vec<ConvertNote>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatementMeta {
    pub account_hint: Option<String>,   // OFX ACCTID, masked to the last 4 before it leaves core
    pub currency: Option<String>,       // OFX CURDEF
    pub ledger_balance: Option<String>, // OFX LEDGERBAL/BALAMT, verbatim decimal text
    pub balance_as_of: Option<String>,  // OFX DTASOF, normalised to YYYY-MM-DD
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceFormat { Csv, Tsv, Ssv, Ofx, Qfx, Xls, Xlsx, Xlsm, Xlsb, Ods }

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConvertError {
    #[error("unsupported format")]          Unsupported { ext: String },
    #[error("PDF conversion is not supported yet")] PdfNotSupported,
    #[error("investment statements are not supported yet")] InvestmentStatement,
    #[error("input is empty")]              Empty,
    #[error("input exceeds the size limit")] TooLarge { limit: usize },
    #[error("malformed {format}: {detail}")] Malformed { format: SourceFormat, detail: String },
    #[error("no worksheet contained tabular data")] NoTable,
}

pub fn detect(name: &str, bytes: &[u8]) -> Option<SourceFormat>;
pub fn convert(format: SourceFormat, bytes: &[u8]) -> Result<Tabular, ConvertError>;
pub fn to_csv(tabular: &Tabular) -> String;   // RFC 4180, LF, always quoted-as-needed
```

`detect` sniffs content first and falls back to the extension — a `.qfx` that is really OFX
2.x XML must not be parsed as SGML. `ConvertError` never carries a path or a raw cell value,
matching the no-disclosure rule the rules API already holds to.

### `crates/ledgeline-core/src/rules/matching.rs` (new)

Two-stage by design: a cheap pure-Rust pre-filter over data we already parse, then hledger
verification of the finalists only. With `MAX_RULES_FILES = 200` already permitted by
discovery, spawning 200 subprocesses per drop is not acceptable.

```rust
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub id: String,              // the existing rules-file handle
    pub label: String,
    pub score: Score,
    pub sample: Vec<ProposedTxn>, // first few, for the UI to show
}

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct Score(f32);           // 0.0..=1.0; newtype so it cannot be confused with a count

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signals {
    pub txns: usize,
    pub postings: usize,
    pub amountless_postings: usize,   // fact 4: silently broken
    pub bare_commodity_amounts: usize,// fact 4: the commodity trap
    pub unknown_accounts: usize,      // hledger's expenses:unknown / income:unknown fallback
    pub empty_descriptions: usize,
    pub column_count_matches: bool,
    pub header_matches_source: bool,
}

/// Stage 1. Pure, no I/O, no subprocess. Rejects the obviously-wrong so stage 2 is cheap.
pub fn prefilter(doc: &RulesDoc, data: &Tabular) -> Option<PrefilterPass>;

/// Stage 2 scoring from hledger's structured output. Pure — the caller runs hledger.
pub fn score(signals: &Signals) -> Score;
```

Stage 1 uses only what the existing parser already gives us: the `fields` count against the
data's column count, the `skip` value against the row count, and the rules' `date-format`
tried against the column `fields` maps to `date`. Stage 2 runs
`hledger print -f DATA --rules R -O json` on at most `MAX_SCORED_CANDIDATES = 8` survivors
and derives `Signals` from the JSON — `pamount` empty means amountless, `acommodity: ""`
means the trap, `paccount` ending `:unknown` means an unmatched rule. `-O json` is used
precisely so no human-readable output is ever regex-scraped.

Ranking is `(score DESC, mtime DESC)`. **`DiscoveredRules` does not currently record an
mtime** — `Fingerprint` deliberately dropped it — so this WP adds a `modified: Option<SystemTime>`
field, used *only* for ranking, never for change detection. That distinction goes in a
comment at the field, because the existing code has a deliberate rule against mtime and a
future reader will otherwise "fix" it.

### `crates/ledgeline-core/src/journals.rs` (new) — choosing the target file

**No naming assumptions are permitted.** Real layouts in the wild include a single file with
accounts at the top and transactions below; `main.journal` including `accounts.journal`,
`prices.journal`, `2025/2025.journal`, `2026/2026.journal`; the full-fledged-hledger layout
with `all.journal` including `2017.journal`, `2018.journal`; and one file per month. Guessing
from filenames fails on at least two of those, and would happily offer `prices.journal` as an
import target.

So the ranking is **derived from content we already parse**, not from names. Ledgeline
already knows every source file (`AppState::source_files`) and which file each transaction
came from, so this is a projection over the existing parse — no new file reads.

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalTarget {
    pub id: String,            // path relative to the include root, forward slashes
    pub label: String,         // the file's own name; NO ranking decision reads it
    pub txn_count: usize,
    pub last_txn_date: Option<String>,
    pub is_root: bool,
    pub writable: bool,        // regular file, not a symlink, inside the include root
}

/// Ranked best-first. Files holding zero transactions (pure `account`/`commodity`/`P`
/// directive files) rank last and are flagged, never hidden — someone's genuinely empty
/// 2027.journal is a legitimate target on 1 January.
pub fn targets(journal: &Journal) -> Vec<JournalTarget>;
```

**Two signature changes from the draft above, made when lane C landed** (convention #9):

- **`&Journal`, not `&Snapshot`.** `Snapshot` is `pub(crate)` in `ledgeline-server` and is a
  bag of precomputed wire `Value`s; `ledgeline-core` cannot see it and should not. Everything
  the ranking needs is already on `Journal` — `source_files` lists every file the parse read
  (including directive-only `include`s) and every `Transaction` carries its `source_file` — so
  the projection is over the model, and the server passes `snapshot.journal`.
- **`last_txn_date: Option<String>`, not `Option<Date>`.** There is no `Date` type in the
  engine: `model.rs` stores every date as an ISO `YYYY-MM-DD` `String`, normalized by the
  parser. Inventing one here would mean a conversion at every boundary for no gain — the
  lexical maximum of ISO dates is the chronological one, which is the only ordering this needs.

Ties are broken by the order the parse read the files (root, then each `include`), which is
deterministic and derived from the journal's structure rather than from any name.

Ranking: files with transactions first, ordered by `last_txn_date` descending — i.e. the file
whose newest transaction is closest to today. That one rule gives the right answer for
year-files, month-files, a single file, and per-account files alike, and it demotes
`accounts.journal` and `prices.journal` automatically because they contain no transactions.
The root journal is always offered regardless of rank. The user can pick any of them; this
only decides what is pre-selected.

### `crates/ledgeline-server/src/git.rs` (new) — commit around the import

The first time Ledgeline touches git. Until now version control has been left entirely to the
user; an import is the first operation that rewrites a journal in place, so it is the first
one that earns a safety net. The value is precise: if an import goes wrong, `git diff` and
`git revert` are the recovery path, and that only works if the pre-import state was committed.

```rust
pub struct Repo { toplevel: PathBuf }   // private; resolved via `git rev-parse --show-toplevel`

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileState { Untracked, Clean, Modified, Ignored }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitStatus {
    pub available: bool,                 // `git` on PATH AND a repo containing the targets
    pub files: Vec<(String, FileState)>, // relative to the toplevel
    pub dirty: Vec<String>,              // the subset that blocks
}

impl Repo {
    pub fn discover(path: &Path) -> Option<Self>;
    pub fn status(&self, paths: &[&Path]) -> Result<GitStatus, GitError>;
    pub fn commit(&self, paths: &[&Path], message: &str) -> Result<(), GitError>;
}
```

Sequence, wrapped around the existing commit step:

1. **Before writing anything**, resolve the repo for each target and run `status`. A target
   that is `Modified` blocks the import behind a "commit your changes first" panel that names
   the files and offers a one-click commit of *exactly those paths*. `Untracked` does not
   block — a brand-new CSV is expected to be untracked.
2. **After** the CSV write, the import, the sort and the checks have all succeeded, commit the
   touched paths with a generated message naming the source file and transaction count.

The rules this module does not get to break:

- **Explicit pathspecs only.** Never `git add -A`, never `git add .`, never `commit -a`. We
  stage the CSV and the journal we wrote, and nothing else. A user with unrelated work in
  progress elsewhere in the repo must find it untouched.
- **Arguments as a `Vec<OsString>`, never a shell string** — same rule as `hledger.rs`, and
  for the same reason. `--` terminates every pathspec list so a file named `-f` is a file.
- **Per-target repo resolution.** The CSV destination and the journal may sit in different
  repositories, or one may be outside version control entirely. Each is resolved on its own;
  a mixed situation degrades to committing what it can and saying what it skipped.
- **Ignored files are reported, not force-added.** If the chosen CSV path is gitignored the
  user meant it; we say so and move on rather than `add -f`.
- **Failure is never silent and never fatal to the import.** A rejecting pre-commit hook, a
  GPG passphrase prompt, an unset `user.email` — each surfaces its stderr in the result panel.
  The journal is already correctly written at that point; the commit is a bonus, so a failed
  commit reports and stops rather than attempting to roll anything back. Every invocation has
  a wall-clock timeout, because a signing prompt would otherwise hang the GUI forever.
- **Opt-out.** `Prefs.git_autocommit: Option<bool>` — `None` means "on when a repo is present",
  and the panel offers a "don't do this" toggle that writes `Some(false)`.

### `crates/ledgeline-server/src/hledger.rs` (new)

```rust
pub struct Hledger { path: PathBuf, version: Version }   // both private

impl Hledger {
    /// Resolution order: pref → $LEDGELINE_HLEDGER → compile-time Nix path → PATH.
    pub fn resolve(prefs: &Prefs) -> Result<Self, HledgerError>;
    pub fn version(&self) -> Version;
}

pub const MIN_HLEDGER: Version = Version { major: 1, minor: 40 }; // --rules rename

#[derive(Debug, Error)]
pub enum HledgerError {
    #[error("hledger was not found")]                  NotFound,
    #[error("hledger {found} is older than {min}")]    TooOld { found: Version, min: Version },
    #[error("could not run hledger")]                  Unrunnable,
}
```

Every invocation is `spawn_blocking` + the existing `reports_api::compute` semaphore, has a
wall-clock timeout, and passes arguments as a `Vec<OsString>` — **never a shell string**.
`Command::new` appears in production code for the first time here; this module is the only
place it may appear, and `docs/imports.md` gains a line saying so.

### `crates/ledgeline-server/src/prefs.rs` (new)

Modelled directly on `recents.rs` — same config dir, same `$LEDGELINE_CONFIG_DIR` override,
same `atomic_write`, same move-a-corrupt-file-aside behaviour.

```rust
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Prefs {
    pub hledger_path: Option<PathBuf>,
    /// `None` = commit around imports when a git repo is present. See `git.rs`.
    pub git_autocommit: Option<bool>,
}

pub fn load() -> Prefs;
pub fn store(prefs: &Prefs) -> Result<(), PrefsError>;
```

Stored at `dirs::config_dir()/ledgeline/prefs.json`. `hledger_path` is validated as an
existing regular executable file at store time; a bad value is rejected with 400 rather
than persisted and failing later.

### HTTP surface (`crates/ledgeline-server/src/import_api.rs`, new)

Registered beside the existing rules routes and, critically, **above** the bearer-token
`route_layer` — the same placement trap `lib.rs:480-493` already warns about.

| Method | Path | Purpose |
| --- | --- | --- |
| `GET` | `/api/import/capabilities` | hledger present? version? formats? journal candidates? |
| `POST` | `/api/import/stage` | raw bytes + `X-Ledgeline-Filename`; converts, returns preview + candidates |
| `POST` | `/api/import/dry-run` | proposed transactions + balance reconciliation |
| `POST` | `/api/import/commit` | writes CSV, runs the real import, reports ordering |
| `POST` | `/api/import/save-csv` | writes the converted CSV and nothing else |
| `POST` | `/api/import/sort` | the confirmed format-preserving re-sort |
| `GET`/`PUT` | `/api/prefs` | the preferences store |

`stage` is the one new **upload** primitive. The SPA's `mutate` helper only sends
`JSON.stringify`d bodies today, so this needs its own path: raw `application/octet-stream`
with the filename in a header, size-capped by `MAX_UPLOAD_BYTES = 16 MiB` enforced by an
axum `DefaultBodyLimit` on that route alone. The filename header is sanitised to a bare
name — no separators, no `..` — and used only for format detection and defaults.

Staged bytes live in a per-session directory created `0700` under the OS temp dir, tracked
by an opaque `StageId` (random, not a path), removed on drop and on server shutdown. **A
`StageId` is never a path and never resolves to one by arithmetic** — same discipline as
`RulesPath`.

### `crates/ledgeline-core/src/sort.rs` (new)

```rust
/// A transaction that would move, described for a diff the user confirms.
pub struct Move { pub date: String, pub description: String, pub from_line: u32, pub to_line: u32 }

pub struct SortPlan { pub moves: Vec<Move>, pub unchanged: bool }

/// Pure. Reorders whole transaction items (lead comments travel with their transaction),
/// leaves directives, standalone comments and includes exactly where they are.
pub fn plan(text: &str) -> Result<SortPlan, SortError>;
pub fn apply(text: &str, plan: &SortPlan) -> Result<String, SortError>;
```

Sorting is **stable** on date, so same-day transactions keep their relative order — which is
what `.latest`-based dedup assumes. `apply` must satisfy the same round-trip obligation the
rules editor holds to: re-parsing the output yields the same transaction set with the same
byte content per item, or the sort is refused. Directives never move, because moving an
`account` or `commodity` declaration past a use, or reordering `P` prices, changes meaning.

Two rules the implementation added, both load-bearing (convention #9):

- **Barriers.** Leaving directives in place is necessary but not sufficient: moving a
  transaction *across* a `Y` changes its date, across an `apply account` its accounts, across a
  `commodity`/`decimal-mark` how its amounts parse, and across an `include` all of those at
  once. So transactions are sorted only within the run between two barrier items. The barrier
  set is stated as an **allow-list** — `account`, `payee`, `tag`, `P` are position-independent
  and may be crossed; everything else at column 1, including anything hledger adds in future,
  is a barrier — so an unknown directive fails safe. A barrier costs reach, never correctness.
- **Blank runs do not travel with a transaction**, unlike `rules.rs`, where a conditional
  table's extent genuinely needs its blank line. No journal construct's extent depends on a
  following blank line, and pinning the blanks is what keeps the confirmation diff to the
  transaction bodies alone — an import's appended row is exactly the one with no blank line
  after it, so a travelling blank run would drag the file's spacing along behind it.

A **yearless date refuses the whole file** with `SortError::UnreadableDate`. `Y 2026` plus
`01/15` is legal hledger, but the sort key then depends on which `Y` is in scope; declining to
offer a sort is better than a subtly wrong one.

### The lane E wire contract

Written before either half of lane E starts, so the Rust and SPA sides can be built against it
concurrently. Every response is `camelCase`, `Cache-Control: no-store`, no ETag — same posture
as the rules routes, and for the same reason: none of this is derived from the journal snapshot.
**No field anywhere carries an absolute path.** A `journalId` and a `csvPath` are both relative
to the include root, exactly as a rules `id` is.

```jsonc
// GET /api/import/capabilities  — what the screen may offer at all
{
  "hledger": {"available": true, "version": "1.52"},
  //  or:    {"available": false, "reason": "notFound"|"tooOld"|"unrunnable", "message": "…"}
  "formats": ["csv","tsv","ssv","ofx","qfx","xls","xlsx","xlsm","xlsb","ods"],
  "journals": [{"id":"2026/2026.journal","label":"2026.journal","txnCount":412,
                "lastTxnDate":"2026-08-01","isRoot":false,"writable":true}],
  "git": {"available": true, "autocommit": true},
  "editable": true          // false ⇒ no journal bound; render read-only and say why
}

// POST /api/import/stage  — body: raw bytes; header: X-Ledgeline-Filename (bare name)
{
  "stageId": "opaque-token",          // NOT a path, and never resolvable to one by arithmetic
  "format": "ofx",
  "preview": {"header": ["date","amount","…"], "rows": [["…"]], "rowCount": 26, "truncated": false},
  "statement": {"accountHint":"7777","currency":"USD",
                "ledgerBalance":"-3238.65","balanceAsOf":"2026-08-12"},   // or null
  "notes": [{"kind":"preambleSkipped","lines":4}],   // ConvertNote, tagged by `kind`
  "candidates": [{"id":"import/2026/bank.csv.rules","label":"bank","score":0.98,
                  "signals":{"txns":26,"postings":52,"amountlessPostings":0,
                             "bareCommodityAmounts":0,"unknownAccounts":0},
                  "sample":[{"date":"2026-06-24","description":"…","postings":["…"]}],
                  // the rules file's own top-level accounts, omitted when it declares none.
                  // `account1` is where every imported posting lands, so it is what the
                  // balance-assertion account defaults to.
                  "account1":"assets:bank:checking","account2":"expenses:unknown"}],
  "defaults": {"csvPath":"import/2026/bank.csv","journalId":"2026/2026.journal"}
}

// POST /api/import/dry-run
// → {"stageId","rulesId","csvPath","journalId","balance":"2945.05"|null,"balanceAccount":"…"|null}
//   `rulesId` and `journalId` are REQUIRED and never null. A dry-run with no rules file has
//   nothing to propose and nothing to reconcile; that state is `save-csv` below.
{
  "ok": true,
  "entries": "2026-02-01 GROCERY STORE\n    …",   // hledger's dry-run stdout, verbatim, and so
                                                 // literally the bytes `commit` will append.
                                                 // NOT restyled — see § Commodity style.
  "count": 3,
  "status": "would import 3 new transactions from bank.csv:",   // hledger's stderr, verbatim
  "skipped": {"olderThan":"2026-02-05","count":1},              // .latest dropped rows — or null
  // ONE representation for all three amounts: the commodity hledger computed the balance in,
  // both numerals at the same scale. The user types `2945.05` and hledger answers `$2945.05`;
  // rendered unchanged they read as a mismatch beside a badge that says "match".
  // `difference` is null when either side is not one amount (a multi-commodity balance), and
  // `computed` is "" when hledger could not compute one at all — an absence, not a zero.
  "balance": {"statement":"$2945.05","computed":"$2945.05","matches":true,"difference":"$0.00"},
  "blockedByGit": ["2026/2026.journal"]            // modified targets; [] when clear
}
// on failure: {"ok": false, "stderr": "…"}  — rendered verbatim in a <pre>, never paraphrased

// POST /api/import/commit   — dry-run body + {"writeAssertion": true}
{
  "csvWritten": "import/2026/bank.csv",
  "journalWritten": "2026/2026.journal",
  "imported": 3,
  "ordering": {"inOrder": false, "moves": [{"date":"2026-01-20","description":"…",
                                            "fromLine":812,"toLine":540}]},
  "git": {"committed": true, "paths": ["…"], "skipped": []}    // or null when no repo
}

// POST /api/import/save-csv — {"stageId","csvPath"}   (no rules file fits; keep the CSV)
{
  "csvWritten": "import/2026/bank.csv",
  "git": {"committed": true, "paths": ["…"], "skipped": []}    // or null when no repo
}

// POST /api/import/sort     — {"journalId": "…"}   (only after commit reported inOrder:false)
{"moved": 3}

// GET/PUT /api/prefs
{"hledgerPath": null, "gitAutocommit": null}
```

Sequencing rules the server owns, not the SPA:

- `stage` writes the converted CSV to a per-session temp dir **named as the chosen destination
  will be**, and copies any existing `.latest.<name>` in beside it, so the dry-run sees the real
  dedup state. That is the only way `skipped` can be truthful before anything is written.
- `commit` writes the CSV to its final destination **first**, then previews and catches up there,
  so `.latest` lands next to the file hledger will look for next time. (`commit` no longer lets
  `hledger import` write the journal at all — see § Commodity style.)
- A `blockedByGit` non-empty from `dry-run` makes `commit` refuse. The UI must not be the only
  thing enforcing that. `save-csv` refuses on the same terms: overwriting an uncommitted edit
  is the one thing `git diff` could not have undone.
- The balance is verified by **concatenation** (`cat journal proposed | hledger -f- check`), never
  two `-f` flags — see fact 3.
- A statement balance written as an assertion **carries its commodity**, and a `commit` that
  asks for one verifies it *before* applying the import — see the amendments below.

### Contract amendments made during implementation

Per convention #9 in `plans/00-overview.md`, every contract change made while building is
recorded here rather than left as a surprise in the diff.

- **`detect` returns `Result<SourceFormat, ConvertError>`, not `Option`.** A `.pdf` has to be
  refused *by name* — `Option` would collapse "we don't support PDF yet" into the same `None`
  as "this isn't a file type we know", and the whole point of keeping `Pdf` visible was the
  specific message. Content is sniffed before the extension, so a PDF is caught even when the
  name lies.
- **`HledgerError` gained `TimedOut { after: Duration }`.** A hung hledger is genuinely
  distinct from a missing one; folding it into `Unrunnable` would send the user hunting for a
  binary that is sitting right there.
- **`Hledger::invoke()` returns a builder, not a `std::process::Command`.** Handing back a raw
  `Command` hands back the ability to skip the timeout, merge the streams, or inherit stdin —
  i.e. it turns three invariants into suggestions. The builder makes them unconstructible
  around, and adds `.stdin(bytes)` because fact 3's concatenation trick needs a pipe and the
  `Command::new` monopoly means the import lane cannot build one itself.
- **`Version` serialises as the string `"1.52"`**, matching hledger's own `--version` display,
  rather than `{"major":1,"minor":52}`. The wire carries it for display only.
- **`sort.rs` introduces *barriers*, which the contract did not anticipate.** Leaving
  directives in place is not sufficient: moving a transaction *across* a `Y` changes its date,
  across `apply account` its accounts, across `commodity`/`decimal-mark` how its amounts parse.
  So transactions sort only within the run between barriers, and the passable set is a tiny
  allow-list (`account`, `payee`, `tag`, `P`) — meaning any directive hledger adds in future
  fails safe instead of being silently crossed.
- **`sort.rs` does not move blank runs with a transaction**, deliberately diverging from
  `rules.rs`. `rules.rs` must carry them because an `if` table's extent is terminated by a
  blank line; no journal construct works that way. Pinning blanks keeps the confirmation diff
  to transaction bodies alone.
- **A yearless date (`Y 2026` + `01/15`) refuses the whole file.** Legal hledger, but the sort
  key would depend on which `Y` is in scope, and `sort::plan` takes `&str` with no parse
  context.

The next four were found by driving the real HTTP API against a live server, after lane E had
landed on both sides. Each is a contract change, so each is recorded here.

- **A balance assertion carries its commodity — fact 4, in our own output.** `writeAssertion`
  rendered the user's number verbatim, so a journal kept in `$` and a balance typed as
  `2949.80` produced `assets:bank:checking  0 = 2949.80`. That does not assert 2949.80
  dollars: an amount with no commodity is an amount in the **empty** commodity, so hledger
  computes 0 for it and the assertion fails — on the import that wrote it and on every
  `hledger check` afterwards. The balance field's own placeholder is `2945.05` and an OFX
  `LEDGERBAL` is a bare decimal, so this was the *normal* input. The commodity is now resolved
  as: what the user typed, if they typed one; else the commodity hledger computed for that
  account over the journal plus the proposed entries (a *bare* computed balance being an
  answer too — a journal with no commodity gets a bare assertion); else the assertion is
  **refused**, because a silently-wrong assertion is worse than none. The shape is
  `hledger close --assert`'s: `assets:bank:checking    $0 = $2949.80`.
- **A failing assertion refuses the commit *before* the import is applied.** It used to apply
  the import and then return `400`, so the journal had changed and the client had an error.
  `commit` now runs the dry-run's own preview first and puts journal + proposed + assertion to
  `hledger check`; a balance that does not hold writes nothing at all — not the CSV, not the
  journal. The post-import check stays, because what makes it safe to append those bytes is
  that hledger agreed to *those bytes*.
- **`POST /api/import/save-csv`, a new route, is the no-rules-file path.** The spec requires
  the screen to offer "even if no rules file applies, they can store the csv", and the SPA was
  sending `{"rulesId": null, "journalId": null}` to `dry-run`/`commit`, which
  `deny_unknown_fields` and non-nullable handles refused with a `400`. Nullable handles would
  encode a state that cannot happen — a dry-run with no rules file has nothing to propose —
  so the path is its own route with a two-field body, `{stageId, csvPath}` →
  `{csvWritten, git}`. `rulesId`/`journalId` on the dry-run body stay required, on both sides.
- **A candidate carries `account1` and `account2`.** Both `Option<String>`, projected from
  `DiscoveredRules`, which already had them. The SPA defaults the balance-assertion account to
  the chosen rules file's `account1`; without these it had to fetch the whole of `/api/rules`
  and join it onto the candidate list by `id`, a join whose failure mode was a silently empty
  field (the two listings come from two separate scans).
- **The reconciliation's three amounts are in one representation.** `statement` was the user's
  string verbatim and `computed` was hledger's, so `2949.80` sat beside `$2949.80` and a UI
  showing both read as a mismatch when it was a match. All three are now rendered in the
  commodity hledger reported for the account, at a shared scale; `difference` is
  `Option<String>` and was always a `null`-able field in the code, which the literal above did
  not say.

### Split layouts — an amendment made after WP-11 landed

Per convention #9, recorded here rather than left as a surprise in the diff. No wire *field*
changes; what changes is what one of them **means**, which is a contract change of the kind this
section exists for. The motivating case is a real user's books:

```
main.journal        include 2025/2025.journal
                    include 2026/2026.journal
2026/2026.journal   opens with a start-of-year assertion carrying 2025's closing balance
```

Verified against hledger 1.52, all of it against the binary:

- **`hledger import -f 2026/2026.journal` aborts on that assertion**, while
  `hledger -f main.journal check` passes. Reading a fragment alone evaluates assertions whose
  balances accumulate through files hledger was never asked to open, so the check is not merely
  disabled-or-not — it is *incapable* of being right there.
- **`import` does not evaluate CSV-derived assertions at all.** A `balance`-field rules file
  asserting `$880.00`, imported into a journal holding `$100.00`, exits zero. So the only
  assertions in reach of that invocation are the target fragment's own.
- **`-I` does not alter the proposed text.** Assertions a rules file generates are still written
  into the journal and still checked at the root; they are deferred, not lost.

Four changes:

- **`import_invocation` passes `--ignore-assertions`**, and is the only invocation that may. Two
  unit tests hold both directions of that: the import carries it ahead of the subcommand, and no
  balance invocation carries it at all.
- **`dry-run`'s `balance.computed` is now the balance of the whole TREE**, not of the target file.
  Same field, same type, different — and correct — meaning. On the layout above the two differ by
  the prior year's closing balance, and the old answer was silently wrong (`$2043.55` for a truth
  of `$2038.55`, `matches: false`, exit 200) whenever the target held no assertion of its own, and
  an empty string plus a refused `commit` whenever it did.
- **The assertion pre-flight reads the root too.** It used to put the *target* plus the proposal to
  `hledger check`, so the fragment's own start-of-year assertion failed first and a correct
  statement balance was refused, quoting hledger about a line the user never typed.
- **`import_api::Plan` has no field named `journal`.** It has `target` (the write destination) and
  `root_journal` (what balances are reckoned against), plus `root_dir` for the include root. One
  field named for neither job is how the two were conflated in the first place.

`fixtures/import/layouts/split-year-assert/` is the committed corpus, and it is the one tree whose
target file deliberately does not pass `hledger check` alone. `LEDGELINE_HLEDGER_LAYOUT_CHECK`
(new, in `just hledger-checks`) asserts that every layout root does, and that this one's fragment
does not — as a pair, since either half alone proves nothing.

### Commodity style — an amendment made after WP-11 landed, then partly withdrawn

Per convention #9, recorded here rather than left as a surprise in the diff. No wire *field*
changes. What changes is which subprocess writes the journal. The motivating case is a real user's
books: an accounts file declaring `commodity $1,000.00  ; comma thousands, 2 decimals`, and
imported transactions arriving as `$165.2` and `$-405`.

**The declared style is NOT applied. That is the decision, and this section exists so it is not
re-litigated from scratch.** Restyling was implemented in `b53323b`, verified working, and then
removed; what follows is what was learned, kept deliberately rather than deleted with the code.

Verified against hledger 1.52, all of it against the binary:

- **`import` applies a declared commodity's SEPARATOR and not its DECIMAL PLACES.** `12345.6`
  is written `$12,345.6`; `165.2` stays `$165.2`; `-405` stays `$-405`. This is true whether the
  import reads the root or a fragment, so it is not the include-scope problem fixed above.
- **No flag on `import` changes it.** `-c/--commodity-style` makes no difference to its output,
  and `--round` is rejected outright — `Unknown flag` — by the one subcommand that writes.
- **`print --round` does apply it**, and the pipeline built on that worked. `soft` pads `$165.2` to
  `$165.20` and `$-405` to `$-405.00` and, unlike `hard`, cannot change a value: hledger's own
  `--help` says `hard` "can unbalance transactions", and with two declared places it writes
  `12345.678` as `$12,345.68`.
- **`import --catchup` writes `.latest.FILE` and appends nothing.** The journal is byte-identical
  afterwards; the state file is byte-identical to a writing import's, repeated same-date lines
  included; a following dry-run reports no new transactions.
- **Prepending a `commodity` directive changes how the entries PARSE.** With
  `commodity 1.000,00 EUR` in scope, `print` re-reads its own `EUR165.2` as `1.652,00 EUR` and
  exits zero. Reachable exactly here, because the import reads the *fragment*, which is the file
  that does not carry the declaration.

**Why it was withdrawn.** That last point is the whole of it. A value-comparison guard did catch
the misparse and fall back to hledger's own text, and it worked — but on a multi-commodity book the
failure mode is not a tidy tenfold on a bank balance, it is a mangled **share quantity**, and a
cosmetic gain does not justify carrying that class of risk even behind a guard that holds today.
`$165.2` in books that write `$165.20` is accepted. A future implementer who wants the declared
style should make the **CSV carry correctly-scaled amounts** — a rules/preprocessor concern, where
the number is still just a number — not re-print finished entries under directives they were never
parsed with.

What was kept:

- **`hledger import` still never writes the journal.** `commit` runs `import --dry-run`, appends
  that stdout itself through `edit::atomic_write`, and then runs `import --catchup` so the dedup
  state stays exactly what hledger maintains. `ImportRun` has **no variant that writes**, which is
  what keeps that structural rather than remembered.
- **`dry-run`'s `entries` is literally the bytes the commit will append.**
  `import_endpoints.rs::the_preview_is_the_bytes_that_are_appended` asserts the appended region
  equals the previewed text plus hledger's own separator. Easier to satisfy with no restyling in
  the middle, and worth keeping for exactly that reason.
- **A failed `--catchup` rolls the journal back** to the bytes read under the write mutex and
  fails the whole commit, because the alternative — entries in the journal, marker not advanced —
  duplicates them on the next import of the same statement. A roll-back that itself fails reports
  the duplication risk in as many words.

What was removed: `crates/ledgeline-core/src/restyle.rs`, the `print --round=soft` invocation, and
`Plan.commodity_directives`.

The appended bytes are **byte-compatible with hledger's own append**, pinned in both directions:
`the_appended_bytes_match_hledgers_own_append` fixes the separator rule, and
`the_appended_bytes_are_hledgers_own_append` imports the same statement into a copy of the same
journal with a real `hledger import` and compares the two files — which, with nothing restyling the
proposal, now covers every journal rather than only the undeclared ones.

### Account aliases — a contract addition made after WP-11 landed

Per convention #9, recorded here rather than left as a surprise in the diff. The motivating case
is a real Morgan Stanley export whose `Account` column says `PW Roth IRA - 3077`; the user's
workaround was a `source ./x.csv | ./clean.py` line, which this codebase never executes.

Verified against hledger 1.52 (all of it against the binary, none of it from the manual):

- **An `alias` directive in the target journal does not reach the CSV during an import.** The
  account comes through unmapped. `--alias` does, in both `OLD=NEW` and `/REGEX/=REPL` forms;
  several compose; `import --dry-run` applies them, so the preview is free.
- **A plain alias splits at the FIRST `=`** (`alias a = b = c` maps `a` to the account `b = c`);
  a regex one does not (`alias /a=b/ = c` is a regex containing `=`), and `\/` is an escape
  inside it, not a terminator.
- **Neither side is comment-stripped.** `alias a = b ; note` declares the account literally named
  `b ; note`.
- **Aliases are positional and file-scoped**: in force from their line to EOF, flowing into
  anything `include`d after them, never back out, stopped early by `end aliases` (plural only;
  `end alias` is a parse error). An `end aliases` inside an include kills the parent's aliases for
  the rest of *that file* and the parent resumes afterwards.
- **Regex aliases are case-insensitive**, and there is no `/re/i` suffix.

Six contract changes:

- **`Journal` gains `aliases: Vec<AliasDirective>`**, modelled on `AccountDeclaration`, with
  `regex`, `source_file`, `position` and an `ended` flag. `parse.rs` previously failed the whole
  journal on an `alias` line, so a user with one could not open Ledgeline at all — and an import
  cannot write into a journal that will not parse.
- **They are read, not applied.** Account names stay exactly as written. Reproducing hledger's
  regex dialect (`regex-tdfa`, POSIX ERE, case-insensitive, `\1` replacements) over every account
  name in someone's books would be a near-miss silent wrong answer; declining is a visible one.
  `parse.rs`'s module docs carry the argument, and the Account Aliases tab says so on screen.
- **`crates/ledgeline-core/src/aliases.rs` (new)** — `forward` (the `--alias` arguments, with a
  named refusal for every alias that does not get one) and `AliasDoc`, a one-line-wide span
  editor holding to `rules.rs`'s discipline: splice only the pattern and replacement extents,
  `verify` or write nothing, present anything unmodelled read-only.
- **`--alias` goes on every invocation that reads the CSV and no other.** `import_invocation`
  takes the alias list and `--dry-run` as parameters, so the preview and the write cannot be
  given different sets. It is deliberately absent from the balance verifications, which read a
  journal whose accounts are already correct.
- **`capabilities` gains `aliases`, and `dry-run` gains `aliases: {forwarded, renames} | null`.**
  The renames are MEASURED — the same import repeated with no `--alias`, diffed — so a silent
  account rewrite is visible before the commit. Empty renames keep the section hidden.
- **`GET /api/aliases`, `PUT /api/aliases/{*journalId}` (new)**, with the same
  revision/409 machinery as the rules editor and one extra guard: the whole journal is re-parsed
  with the edited text in memory before anything is written.

## UI behavior

The subnav copies `web/src/lib/reports/ui/ReportTabs.svelte` and the `?tab=` mirroring in
`routes/reports/+page.svelte:46-77` verbatim — `$bindable` rune, `searchMirror()` debounced
`replaceState`, restore-once-on-mount with a `restored` latch. New route files stay flat
(`/imports?tab=new`), matching a codebase that has zero nested routes. The existing inner
`Preferences / Row mapping / Accounts` strip on Edit Rules stays as-is, nested under the new
outer one.

The New Transactions flow is a single page that reveals sections as they resolve:

1. **Drop target** — full-width dashed panel, plus a "Choose file…" button. Drag events are
   new to this codebase; the panel handles `dragenter/dragover/dragleave/drop` with
   `preventDefault` on all four, and the button uses a hidden `<input type="file">`.
2. **Preview** — first rows of the converted CSV in a plain table, with the detected format,
   row count, and any `ConvertNote`s (e.g. "sheet 2 of 3 used", "dates were serial numbers").
3. **Rules file** — ranked candidates as radio cards showing score, matched-rule coverage and
   a two-transaction sample. When none pass: a plain statement that no rules file fits, the
   Save-CSV path still available, and a disabled "Create rules file…" button.
4. **Destinations** — CSV path (defaults to the rules file's name minus `.rules`, same
   directory, editable) and target journal (defaults to the journal whose name/date is
   nearest today; all discovered journals + includes listed).
5. **Balance** — optional; pre-filled from OFX `LEDGERBAL` when the format volunteered one.
6. **Actions** — `Save CSV` alone when no rules file is chosen; `Save and Import` when one is.

`Save and Import` runs the dry-run and shows the proposed transactions, the `would import N`
count, any back-dated-row warning, and the balance reconciliation (statement vs computed,
with the difference) before offering `Write changes`. A failed dry-run shows hledger's
stderr verbatim in a `<pre>` — it is genuinely good output and paraphrasing it loses the
`record:` echo that tells the user which row broke.

Every async surface goes through `<AsyncSection>` and gets registered in
`web/src/routes/branchOrder.test.ts:41-46`, which lints exactly this.

## Fixture & test plan

The house rule from `fixtures/rules/README.md` holds: **a fixture hledger rejects is a bug in
the fixture**. `just rules-check` runs before `cargo test` for anything new under
`fixtures/rules/`.

New fixtures under `fixtures/import/`:

| Path | Proves |
| --- | --- |
| `ofx/bank-v1.ofx` | OFX 1.x SGML, unclosed tags, `STMTRS`, `LEDGERBAL` |
| `ofx/bank-v2.ofx` | OFX 2.x XML, same statement, must yield an identical `Tabular` |
| `ofx/creditcard.qfx` | `CCSTMTRS` + Quicken `INTU.BID`, sign convention |
| `ofx/investment.ofx` | `INVSTMTRS` → refused by name, not mis-parsed |
| `ofx/tz-dates.ofx` | `DTPOSTED` as `YYYYMMDDHHMMSS[-5:EST]` |
| `spreadsheet/simple.xlsx` | header row + date cells as serial numbers |
| `spreadsheet/multi-sheet.xlsx` | sheet selection + the `ConvertNote` |
| `spreadsheet/legacy.xls` | BIFF path |
| `delimited/semicolon.ssv`, `delimited/tab.tsv` | separator by extension |
| `delimited/latin1.csv` | non-UTF-8 detection |
| `match/checking.csv` + `match/*.rules` | one clearly-right and two clearly-wrong candidates |
| `match/garbage-success.rules` | **fact 4** — parses fine, scores near zero |
| `match/no-currency.rules` | **fact 4** — the bare-commodity trap is caught |
| `sort/interleaved.journal` | back-dated import, directives and includes must not move |
| `sort/comments.journal` | lead comments travel with their transaction |
| `layouts/single/main.journal` | accounts at top, transactions below, one file |
| `layouts/split-year/` | `main.journal` + `accounts.journal` + `prices.journal` + `2025/2025.journal` + `2026/2026.journal` |
| `layouts/full-fledged/` | the `all.journal` → `2017.journal`/`2018.journal` convention |
| `layouts/monthly/` | one file per month |
| `layouts/split-year-assert/` | a root whose 2026 file opens with a start-of-year assertion — the tree passes `hledger check`, the fragment cannot. See § Split layouts |

The four `layouts/` trees are the anti-assumption fixtures: `journals::targets` is asserted
against each, and the assertion is specifically that `accounts.journal` and `prices.journal`
rank **below** every transaction-bearing file without any filename ever being inspected. A
test that passes because a name was recognised is a failing test here.

Tests, by tier:

- **Unit, in-crate** — `convert::detect` on ambiguous extensions; `to_csv` RFC 4180 quoting;
  `score` monotonicity (more `amountless_postings` never raises a score); `sort::plan`
  stability.
- **Property (proptest)** — `to_csv` → `csv` crate round-trip for arbitrary cell content
  including embedded separators, quotes and newlines; `sort::apply` is a permutation that
  preserves every transaction byte-for-byte; sorting an already-sorted journal is the
  identity.
- **Integration, core** — every fixture above through `convert`, asserting `Tabular` shape;
  `bank-v1.ofx` and `bank-v2.ofx` produce equal output.
- **Integration, hledger-backed** — gated behind an env var exactly like the existing
  `LEDGELINE_HLEDGER_RENDER_CHECK`, so `cargo test` stays hermetic. Proves: dry-run stdout
  parses as a journal; the concatenation trick reports the correct combined balance where
  two `-f` flags do not (**fact 3, asserted directly**); a back-dated row is dropped and we
  detect it; scoring ranks the right rules file first.
- **Server** — every new route requires the token; `stage` rejects over-size bodies, path-ish
  filenames and `.pdf`; a `StageId` from one session cannot be read by another; no response
  body contains an absolute path (the existing golden test extends to the new routes);
  commit is refused when hledger is missing or too old.
- **SPA unit** — the tab-param codec round-trip (`params.test.ts` pattern), destination-default
  derivation, and journal-nearest-to-today selection, all as pure functions in
  `$lib/imports/model.ts` per the existing rule that logic does not live in components.
- **E2E** — extends `web/e2e/imports.e2e.ts`, seeding its own scratch tree under
  `fixtures/scratch/` as that spec already does. Drops a CSV, picks the suggested rules file,
  dry-runs, confirms, and asserts the journal changed and *nothing else did*.

The last one is the acceptance criterion from the request, so it is asserted mechanically: a
recursive hash of the scratch tree before and after, differing only at the CSV and the
journal.

## Key files created

`crates/ledgeline-core/src/convert/{mod,ofx,spreadsheet,delimited,encoding}.rs`,
`crates/ledgeline-core/src/rules/matching.rs`, `crates/ledgeline-core/src/sort.rs`,
`crates/ledgeline-core/src/journals.rs`,
`crates/ledgeline-server/src/{hledger,prefs,git,import_api,stage}.rs`,
`crates/ledgeline-server/tests/git_commit.rs`,
`crates/ledgeline-core/tests/{convert,matching,sort}.rs`,
`crates/ledgeline-core/tests/import_hledger.rs` (env-gated),
`crates/ledgeline-server/tests/import_endpoints.rs`,
`web/src/lib/imports/ui/{ImportTabs,DropTarget,PreviewTable,CandidateList,DestinationForm,BalanceField,DryRunPanel,SortDiff}.svelte`,
`web/src/lib/imports/{params,importModel,importStore.svelte}.ts` + tests,
`web/src/routes/imports/+page.svelte` (tab host; today's content moves into an `EditRules`
component), `fixtures/import/**`, `docs/pdf-extraction.md`.

Modified: `crates/ledgeline-core/src/rules/discovery.rs` (add `modified`),
`crates/ledgeline-server/src/lib.rs` (routes, `Hledger` + `Prefs` in `AppState`),
`flake.nix` (bake the hledger store path), `docs/imports.md`, `README.md` (TODO → shipped).

## Depends on / parallel

Six lanes. A and B have no dependencies on each other or on anything else and start
immediately; the split exists so three agents can work without touching the same files.

```
Lane A  prefs + hledger resolution + capabilities route      ─┐
Lane B  convert/ (ofx, spreadsheet, delimited, encoding)     ─┤
Lane C  sort.rs + journals.rs (pure, fixture-driven)         ─┼─→ Lane E  import_api wiring
Lane D  rules matching (needs B's Tabular type only)         ─┤        (needs A+B+D+G)
Lane G  git.rs (self-contained, subprocess + tmp repos)      ─┘
Lane F  SPA subnav + Edit Rules move (contract-only)         ──────────→ SPA New Transactions
```

Lane G is fully self-contained — it shells out to `git` against throwaway repositories it
builds in its own tests and shares no file with any other lane — so it parallelises cleanly
despite landing late in the runtime sequence.

Lane F's first half — the subnav and relocating today's imports page under it — touches no
Rust and can land first as its own commit, which de-risks the biggest UI refactor. Lanes B,
C and D are pure `ledgeline-core` with no I/O beyond fixture reads, so they are the ones
worth handing to parallel agents; each owns disjoint files and a disjoint fixture directory.
Lane E is the integration point and is deliberately single-threaded.

Per convention #9 in `plans/00-overview.md`: the types in **Interface contracts** above are
the coordination mechanism. An agent that needs to change one updates this document in the
same commit and says so.

## Definition of done

- The New Transactions tab accepts all listed formats by drop and by picker; `.pdf` is
  refused with its own message.
- A dropped file shows a preview, ranked rules candidates, and defaulted destinations.
- Dry-run shows the proposed transactions and, when a balance was entered, the
  reconciliation — computed via concatenation, with a regression test proving the two-`-f`
  form is wrong (**fact 3**).
- A back-dated row skipped by `.latest` is surfaced, not silently dropped.
- Commit writes exactly one CSV and one journal; the e2e tree-hash assertion proves nothing
  else on disk changed.
- Out-of-order dates are detected and the offered re-sort preserves directives, includes and
  comments byte-for-byte outside the moved transactions.
- A statement balance, when given, is written as an assertion transaction in the
  `hledger close --assert` shape, **carrying the journal's commodity for that account**
  (`assets:bank:checking    $0 = $2949.80`) — and `hledger check` over the resulting journal
  passes. A balance that does not hold refuses the whole commit before anything is written.
- When no rules file fits, the converted CSV can still be saved on its own, without a journal
  and without hledger.
- With git present: a modified target blocks the import until committed; a successful import
  commits exactly the CSV and journal it wrote and nothing else — asserted by staging
  unrelated dirty files in a test repo and proving they are still dirty afterwards.
- Missing / too-old hledger produces an actionable banner with a path-setting control, never
  a stack trace or a silent failure.
- `just engine-check`, `just engine-test`, `just check`, `just test`, `just rules-check` and
  `just e2e` all pass; `cargo test` remains hermetic with no hledger required.
- `docs/imports.md` updated (scope, the new `Command::new` rule, the stage-dir lifecycle);
  `docs/pdf-extraction.md` written; `README.md` TODO entry retired.
- Commits (conventional, ≤50-char subjects): `feat: imports subnav and tab host` →
  `feat: convert ofx and spreadsheets to csv` → `feat: score rules files against import data`
  → `feat: date-sort journals preserving format` → `feat: commit imports to git when present`
  → `feat: run hledger import from the ui` → `test: import fixtures and e2e` →
  `docs: import pipeline and pdf research`.
