# WP-16: Import & Rules Enhancements II

Read `docs/imports.md` first — same rule WP-11 states. This WP builds directly on WP-11's shipped
pipeline (`convert/`, `rules.rs`/`rules/discovery.rs`/`rules/matching.rs`, `hledger.rs`, `git.rs`,
`prefs.rs`, `import_api.rs`, `aliases.rs`) and does not re-litigate any of it.

## Scope

Five pieces of work from `TODO.md`'s "Import improvements" section, done on one branch because
Phase 2 depends on Phase 1's components:

- **Phase 0** — two standalone bug/chore fixes.
- **Phase 1** — rules editor redesign: scannable display-list + per-item edit, plus AND-combined
  matchers become editable (still not full grammar parity).
- **Phase 2** — create a new `.rules` file from a dropped CSV, with auto-detected column mapping,
  date format, separator/skip/encoding (mostly already built in WP-11's `convert/`).
- **Phase 3** — a scriptable `ledgeline import` CLI subcommand, plus a "copy as CLI command"
  affordance in the GUI's dry-run panel.
- **Phase 4** — ID-based re-import matching (uses the `fitid` column OFX/QFX/QBO already produce),
  with status-only auto-sync and warn-don't-clobber for everything else.

**Out of scope, on purpose**: any QuickBooks-specific pipeline (IIF/CSV/XML export parsing,
per-source account-mapping store) — ruled out; `.qbo`/`.qfx` already convert via the existing OFX
path and that is the only QB surface this WP touches. Full `if`-block grammar parity (negation,
capture groups, if-tables) — still opaque after Phase 1. "Intelligent category suggestions" from
transaction history — a separate, already-tracked TODO item, not attempted by Phase 2's `account2`
defaulting (which just defaults to `expenses:unknown`).

## Phase 0 — two fixes, no new contracts

1. `web/src/lib/api/nativeDecode.ts`: `RawGitReport` gains `message?: string` (mirrors Rust's
   `Option<String>`, `#[serde(skip_serializing_if = "Option::is_none")]` at `import_api.rs:742-752`
   — so `message` is simply absent on success, never `null`). `decodeGitReport()` passes it
   through. `web/src/lib/imports/importTypes.ts`'s `GitReport` gains `message?: string`. Whichever
   panel renders `GitReport` (confirm exact component during implementation — candidates:
   `ResultPanel.svelte`, `DryRunPanel.svelte`) shows it beneath the committed/skipped summary when
   present, styled as a warning (a message only exists when something is worth surfacing — a
   rejected pre-commit hook, a GPG prompt that failed, etc; see `git.rs`'s doc comment on when this
   field is populated).
2. `crates/ledgeline-core/src/model.rs`: `AmountStyle.digit_groups: Option<DigitGroups>` becomes
   `Option<Arc<DigitGroups>>`. Every clone site listed in the plan gets `Arc::clone` (cheap) instead
   of a deep `Vec<u8>` clone. No wire-shape change — confirm during implementation whether any
   `Wire*` struct in `crates/ledgeline-server` embeds `AmountStyle`/`DigitGroups` directly (if so, it
   already goes through `Serialize`, which is unaffected by the `Arc` wrapper — `serde`'s blanket
   `impl<T: Serialize> Serialize for Arc<T>` serializes the pointee).

## Phase 1 — rules editor redesign

### Rust: `crates/ledgeline-core/src/rules.rs`

Today `IfBlock.matchers: Vec<Matcher>` is a flat OR list; any `&`/`&&`-joined line makes the whole
block `Opaque(CombinedMatcher)`. New shape:

```rust
/// One OR-branch of an if-block: matchers within a group are AND'd, groups are OR'd.
/// A block with exactly one matcher per group (today's shape) still round-trips through
/// this type unchanged — `Vec<MatcherGroup>` where every group has `matchers.len() == 1`.
pub struct MatcherGroup {
    pub matchers: Vec<Matcher>,   // AND'd; non-empty
}

pub struct IfBlock {
    pub layout: IfLayout,
    pub groups: Vec<MatcherGroup>,   // was: matchers: Vec<Matcher>; OR'd
    pub assignments: Vec<Assignment>,
    pub indent: ...,
}
```

Classifier change: a block is editable (not `Opaque(CombinedMatcher)`) when every `&`-continuation
line is a **plain, non-negated** matcher continuing the previous line's group (hledger's actual
`&` semantics — verify against 1.52, not the manual, per house rule). `!`-negated lines, capture
groups, `if`-tables, comment-like matcher lines: unchanged, still `Opaque` with their existing
`OpaqueReason`. This is additive — no existing `Opaque` classification becomes *more* opaque.

`RulesDoc::apply`/`verify` extend to render `groups` back to hledger's `if A\n& B\nif C` shape
(matchers within a group on consecutive lines, first prefixed `if`/plain, rest prefixed `&`; groups
separated exactly as separate `if` blocks are today, i.e. as stacked matcher lines within the same
`IfBlock` item — the item's span/extent handling is unchanged, only what's inside it).

### Wire (`rules_api.rs`) and TS (`model.ts`, `importTypes.ts`)

`WireIfBlock.matchers: WireMatcher[]` becomes `WireIfBlock.groups: WireMatcherGroup[]` where
`WireMatcherGroup = { matchers: WireMatcher[] }`. `FormMatcher[]` in `model.ts` becomes
`FormMatcherGroup[]` (`{ matchers: FormMatcher[] }[]`); `toSaveRequest`/dirty-tracking/validation
(`model.ts:825-842` area) extend to the nested shape — a group must have ≥1 matcher, a matcher
inside a group is validated exactly as today (still rejects `&`/`!`/`(` — those chars can't appear
*inside* a matcher's own pattern, only as a line-prefix, which is now structural rather than
textual).

### UI

- `RulesList.svelte` becomes the **display** layer: each `IfBlockItem` renders as one compact,
  single-line-where-possible summary (e.g. `IF description ~ "AMAZON" AND account1 = "checking:.*" → account2 = expenses:shopping`), not the full `IfBlockCard` editor. Other item kinds
  (settings, opaque/kept items) keep their existing compact rendering.
- `EditRulesPanel.svelte` adds `editingItemId: string | null` state. Clicking a summary card sets
  it; `IfBlockCard` (now the **edit** view, opened for one item at a time) renders inline in place
  of that card, or in a slide-over — pick whichever keeps the rest of the list visible and
  scannable, confirm in-browser before calling it done. Save/cancel clears `editingItemId`.
- `IfBlockCard.svelte` UI gains "+ AND condition" (adds a matcher to the current group) and
  "+ OR group" (adds a new `MatcherGroup`) controls, replacing today's single flat "+ condition".

### Tests

`rules.rs` unit tests: AND-editable boundary (plain `&`-chain → editable; `& !` or `&` after a
capture group → still opaque); round-trip of multi-group blocks. `model.test.ts`: grouped-shape
validation, dirty-tracking, `toSaveRequest` diffing. Component tests for
`RulesList`/`EditRulesPanel` display↔edit transition. `rules_hledger_render.rs`
(`LEDGELINE_HLEDGER_RENDER_CHECK`): a real multi-group AND/OR rules file parses and matches the way
hledger itself says it should (cross-check via `hledger print -O json` on a fixture, same technique
`matching.rs` already uses). New fixture:
`fixtures/rules/simple/and-groups.csv.rules` (+ its `.csv`).

### Contract amendments made during implementation (Rust + wire half)

Per convention #9 in `plans/00-overview.md`. The Rust engine and the JSON wire landed as sketched
above; the SPA half (`model.ts`, `importTypes.ts`, the three components) is a separate pass and
should be written against **this** section, not the sketch above it.

Findings first — all verified against the hledger 1.52 binary (`hledger print -f DATA.csv --rules
FILE.rules`), several of which contradict what the sketch assumed:

- **AND-chains are not confined to the stacked layout.** An inline `if MATCHER` header followed by
  `& CONTINUATION` lines works and AND-s exactly as a bare `if` does. The sketch's `if A\n& B\nif C`
  phrasing also mis-stated the OR separator: a new OR branch is a *plain matcher line inside the
  same block*, not a second `if`.
- **The space after `&` is optional.** `&B`, `& B`, `&\tB` and `&    B   ` are one matcher, pattern
  trimmed both ends — so the parser must not require `"& "` and the renderer must splice an existing
  line's own spacing rather than normalise it.
- **A leading `&` on the first matcher line is legal hledger and is a no-op** (`if\n& COFFEE`
  imports exactly what `if\nCOFFEE` does). The sketch guessed hledger might error. It does not; we
  keep it `Opaque(CombinedMatcher)` anyway, by choice, and say so.
- **A bare `&` line is a hard parse error** in hledger, which is why an empty `MatcherGroup` is
  refused rather than dropped.
- **A leading `&&` is also an AND join to hledger**, not the error the old code's blanket
  `contains("&&")` implied. It stays opaque regardless: on one line `&&` may be two literal
  ampersands in a regex, and that ambiguity is unresolvable without hledger's parser.
- **`&` is a prefix only at the head of a line.** `%description &COFFEE` is a regex containing a
  literal `&`; it matches no record containing plain `COFFEE`. So the `%field &pattern` form the
  sketch asked about does *not* AND.
- **An indented `& X` is rejected by hledger** ("expecting conditional block"), which is why
  continuation lines render at column 1 and no indent logic was added.

Type and contract shapes as landed:

- `MatcherGroup { matchers: Vec<Matcher> }` and `IfBlock.groups: Vec<MatcherGroup>` — as sketched.
- **`ItemBody::IfBlock` gained a mirrored `MatcherGroupSpec { matchers: Vec<MatcherSpec> }`**, and
  its field is `groups`, not `matchers`. The sketch named only the parsed side; the edit side has to
  move in lockstep or a client could not express an AND at all.
- **`check_body` refuses an empty group** with "a conditional block's OR-group needs at least one
  matcher". An empty group would vanish on flattening and silently re-group its neighbours.
- **`check_matcher` is unchanged**: a *pattern* still may not start with `&` or `!` nor contain
  `&&`. The AND is carried only by nesting, so there remains no text path from a client to a
  combinator. The SPA's per-matcher validation therefore needs no change either — only its shape.
- Wire: `WireItemBody::IfBlock.groups: Vec<WireMatcherGroup>` and `WireItemIn::IfBlock.groups:
  Vec<WireMatcherGroupIn>`, both `{ "matchers": [{ "field"?, "pattern" }] }`. `layout`,
  `assignments`, `WireMatcher`'s own two fields and every other variant are untouched.

Rendering conventions, which the sketch left open:

- An **existing** matcher line's prefix — nothing, `if `, or its own `& `/`&\t`/`&   ` — is spliced
  from the file verbatim whenever the matcher keeps its OR-group role. Only a matcher that changed
  role is re-prefixed (gaining `"& "` or losing its `&`), which is the sole thing grouping can do to
  a line that already exists.
- **Added** matchers keep the existing placement rule exactly (`column one below the last one`) and
  differ only by that prefix, so "+ AND condition" and "+ OR group" are one code path.

Fixtures and tests as landed, beyond the plan: `fixtures/rules/tree/import/2026/bank.csv.rules`
gained a two-matcher AND-group, because the byte-pinned wire golden could not otherwise pin the
`groups[].matchers[]` **nesting** (every other block is one matcher per group). That moved
`rules_security.rs`'s counts to `if_block_count == 5` / `editable_block_count == 4` and required
`just snapshot-rules-wire`. `rules_hledger_render.rs` gained a second test that asserts hledger
*routes each CSV row* the way the re-parsed groups say it should — the existing "hledger reads it"
scenarios all pass against a renderer that writes OR where the model says AND, and this one does
not.

### Contract amendments made during implementation (SPA half)

The `groups` wire above landed in the SPA exactly as the Rust half describes it — `RulesMatcherGroup`
in `imports/types.ts`, `RulesMatcherGroupInput` in `api/native.ts`, `FormMatcherGroup` in
`imports/model.ts`, one `decodeMatcherGroup` calling the existing `decodeMatcher` per matcher. What
follows is everything the plan left open, decided here.

Model:

- **`itemSignature` needed a third separator.** It already joins with the control characters U+0000
  (fields), U+0001 (a matcher’s own two halves) and U+0002 (matchers from assignments); matchers
  within an AND-group now join with U+0003. Without it `[[A],[B]]` and `[[A,B]]` flatten
  to the same string, and a *regrouping* — which changes which rows the file matches — would go back
  to the engine as an unchanged `{kind:"keep"}`. There is a test that fails if the separator is
  removed.
- **`describeIfBlock(rule): string` is its own export**, not a branch of `describeItem`.
  `describeItem` returns an `ItemSummary` for `KeptItemCard`'s title/detail/`<pre>` chrome, which a
  one-line rule summary has no use for. Format:
  `IF description ~ AMAZON AND card ~ personal → account2 = expenses:shopping:online`; `row ~ X` for
  a whole-record matcher; brackets around an AND-group **only** when a second OR-branch shares the
  line. Truncation is structural, not a character cut: at most 2 OR-branches, 3 conditions per
  branch, 2 assignments and 32 characters per pattern/value, each overflow rendered as `+N more`.
  A rule with no pattern typed anywhere reads `New rule`; an assignment with no value yet is named
  without an `=`.
- **Validation names a group only when there is one to name.** One group keeps today's
  `Rule 1, match 2`; two or more give `Rule 1, group 2, match 1`. An empty group is refused with the
  engine's own reason. `checkMatcher` itself is unchanged, as the Rust half predicted.
- Two user-facing strings went stale and were rewritten: the `&`/`!` matcher error now points at
  "+ AND condition" rather than saying Ledgeline cannot edit an AND, and `OPAQUE_REASONS`'
  `combinedMatcher` now names what is actually still opaque (`!`, `&&`, a leading `&` with nothing
  above it).

UI:

- **Expand/collapse state lives in `RulesList`**, beside the keyboard cursor, not in
  `EditRulesPanel` — it is a fact about how the list is being read. The panel keys the subtree on
  `form.id#formEpoch`, so a file switch or Revert already discards it. A **save** is the one event
  the list cannot see from its own props, so the panel passes `savedAt` (state it already had) and
  the list closes the open card when that value *moves* — a latch on the value, not on truthiness,
  or the card could never be reopened.
- **New component `imports/ui/RuleSummaryCard.svelte`** — the collapsed line, styled after
  `KeptItemCard` but not dimmed. It carries `data-testid="imports-rule"`, as the expanded
  `IfBlockCard` does, so "the rule cards" is one locator in either state.
- **The summary row carries ↑/↓ and nothing else; delete stays inside the opened editor.** Reordering
  is a list-level act done while reading; deleting is destructive and should follow reading the rule.
- **"+ Add rule" opens the new rule immediately** — a blank summary line has nothing to scan.
- **Keyboard**: `Enter` now opens/closes the cursored rule (and focuses its first control after a
  tick), matching `BalanceSheetView`/`IncomeStatementView`; it previously focused a control in an
  always-expanded card because there was no collapsed state. `Escape` closes the open rule first and
  clears the cursor only when nothing is open — the same two-stage shape `TransactionTable`'s Escape
  already has. j/k/J/K/gg/G are untouched.
- An open card is carried through a reorder by position (`afterMove`), because position is the only
  identity these entries have — a rule the user just added has no id at all.

Fixtures and tests, beyond the plan: `rulesLive.integration.test.ts`'s scratch file and
`e2e/imports.e2e.ts`'s scratch file each gained an AND-group block, which moved the e2e listing
assertion to `5 rules, 1 advanced`. `RulesList.svelte.test.ts` builds its items with `$state(...)`
— a plain array renders once and then stops agreeing with itself, because the card's `rule.groups =
…` is a nested write.

## Phase 2 — create rules files from a CSV

### Rust: `crates/ledgeline-core/src/rules/generate.rs` (new)

```rust
pub struct ColumnGuess { pub index: usize, pub field: RulesField, pub confidence: f32 }
pub struct DateFormatGuess { pub format: String, pub confidence: f32 }

/// Pure. Scores each header cell against known hledger field names/synonyms
/// ("date","posted","txn date" -> date; "debit"/"credit" -> amount-in/amount-out; etc).
pub fn guess_columns(header: &[String], sample_rows: &[Vec<String>]) -> Vec<ColumnGuess>;

/// Pure. Tries sample date-column values against a format catalog (share/port
/// `web/src/lib/imports/dateFormats.ts`'s catalog if it already enumerates the formats
/// hledger's `date-format` accepts — confirm before duplicating it).
pub fn guess_date_format(samples: &[String]) -> Option<DateFormatGuess>;

/// Renders a starting-point RulesDoc from guesses + user-supplied account1/separator/skip/encoding
/// (separator/skip/encoding come from convert/'s existing sniffing, not reguessed here).
pub fn draft(tabular: &Tabular, columns: &[ColumnGuess], account1: &str) -> RulesDoc;
```

### HTTP surface

`POST /api/rules-create` (sibling to `/api/rules`, same auth posture): body carries a `StageId`
(reuse WP-11's `stage.rs` upload — the user already dropped the file to get here) plus the target
directory (must resolve inside the confined root exactly as `Discovery` confines today) and
filename (validated exactly like an edit `id`: no `..`, no leading `/`, must end `.rules`, and this
route 409s rather than silently overwriting an existing file — creation and edit stay distinct
code paths on purpose). Response: the drafted `WireRulesDoc` (same shape `GET /api/rules/{*id}`
returns) plus a `WireRulesPreview` (reuse `rules-preview`'s shape) so the SPA can show "here's what
this turns into" before the user commits to writing it, exactly as the original ask wanted. A
follow-up `PUT /api/rules/{*id}` (today's existing edit route) does the actual write, once the user
is happy with the draft — `rules-create` never writes to disk itself, keeping "generate a plausible
draft" and "write a file" as separate, separately-testable operations.

### UI

New `web/src/lib/imports/ui/CreateRulesPanel.svelte`, reached from `NewTransactionsPanel`'s
existing disabled "Create rules file…" button (find its current `no_candidates`-gated stub and
un-stub it). Reuses Phase 1's summary-card display components to show the drafted rules file
before save, and `PreviewTable.svelte`-style rendering for the "what your rows become" preview.

### Tests

Unit tests for `guess_columns`/`guess_date_format` against a fixture set of realistic messy headers
(`fixtures/import/generate/headers/*.csv` — several real-shaped bank export header rows, no two
identical). Server tests mirroring `rules_security.rs`'s discipline for `rules-create`'s new write
boundary. SPA component/e2e test: drop a CSV with no matching rules file → Create flow → preview →
save → new file appears in Edit Rules.

## Phase 3 — scriptable CLI import

### Extraction

`import_api.rs`'s handlers currently build a `Plan` and call `hledger` inline. Extract the
`stage → convert → match → dry-run → commit → sort` sequence into
`crates/ledgeline-server/src/import_runner.rs` (new): functions taking already-resolved inputs
(paths, not `StageId`s — the HTTP layer keeps owning the upload/staging lifecycle; the CLI reads
files directly since there's no upload step at all in a local process) and returning the same
result types the HTTP responses already serialize (`DryRunResult`, `CommitResult`, etc., promoted
from private handler-local types if needed). Handlers become thin: resolve the HTTP-specific input
(`StageId` → path), call the runner, serialize the result. Write characterization tests against the
**current** HTTP behavior before this move, so the extraction is provably behavior-preserving —
snapshot a handful of `import_endpoints.rs` cases' exact response bodies first.

### CLI

`main.rs`'s `Cli` gains an optional subcommand without disturbing the existing flat invocation:

```rust
#[derive(Parser, Debug)]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Option<Command>,
    // existing fields (journal, server, host, port, allow_origin) unchanged —
    // used when `command` is None, exactly as today.
    ...
}

#[derive(clap::Subcommand, Debug)]
pub(crate) enum Command {
    /// Non-interactive import: stage, dry-run or commit, optionally sort.
    Import {
        #[arg(short = 'i', long)]
        input: PathBuf,
        #[arg(short = 'o', long)]
        output: PathBuf,           // CSV destination
        #[arg(short = 'r', long)]
        rules: PathBuf,
        #[arg(short = 'j', long)]
        journal: PathBuf,
        #[arg(long)]
        balance: Option<String>,
        #[arg(long)]
        sort: bool,
        #[arg(long)]
        dry_run: bool,             // preview only, writes nothing, exit 0/1 on match/mismatch
        #[arg(long)]
        no_git: bool,              // opt out of the git safety net for this invocation
    },
}
```

`Command::Import` calls `import_runner`'s functions directly — same code path the GUI's `commit`
route uses, so behavior can't diverge between GUI and CLI. Every result prints the
`ledgeline import ...` equivalent line to stderr before exiting (useful for logging/audit even when
run non-interactively) — the **same renderer** that produces the GUI's "copy as CLI command"
string, so the two can't drift: `import_runner::as_cli_invocation(&Plan) -> String`, single
function, two call sites (CLI's own echo, and a new field on the HTTP dry-run/commit response,
`cliEquivalent: string`, rendered in `DryRunPanel.svelte`).

### Tests

CLI integration tests spawn the built binary (mirroring `justfile`'s `test-integration`) against
`fixtures/import/` cases, asserting exit code + resulting journal bytes. Unit tests for flag
validation (e.g. `--balance` without `--journal` implied by required-ness, not a runtime check).
Round-trip parity test: run the GUI path against a fixture, capture `cliEquivalent`, actually
execute it as a subprocess against a fresh copy of the same fixture, diff the two resulting
journals byte-for-byte — must be identical.

## Phase 4 — ID-based re-import matching & status sync

### Rules-file concept: an id column

Smallest addition that reuses existing machinery rather than inventing a new directive: a rules
file marks a column as the dedup id via the existing `comment`/tag-assignment mechanism — e.g.
`comment id:%fitid`. **Verify the exact hledger 1.52 tag-comment grammar against the binary before
writing `reimport.rs` against it** (this repo's rule, restated because it matters here more than
usual — this is the one new piece of hledger-facing grammar this WP introduces). If tag-comments
turn out not to round-trip the way assumed, the fallback is a Ledgeline-only convention: a
dedicated `; ledgeline-id: VALUE` line our own renderer controls, which does not depend on hledger
tag semantics at all — record whichever is chosen as a contract amendment here.

### `crates/ledgeline-core/src/reimport.rs` (new)

```rust
pub struct IdIndex(HashMap<String, TransactionRef>);  // built once per dry-run/commit

pub fn build_index(journal: &Journal, id_tag: &str) -> IdIndex;

pub enum RowClassification {
    New,
    Unchanged,
    StatusOnly { existing: TransactionRef, new_status: Status },
    Conflicting { existing: TransactionRef, diffs: Vec<FieldDiff> },
}

/// Pure. `proposed` is hledger's own dry-run proposal for this row (so classification reuses
/// the same JSON hledger already produced — no reimplementing hledger's rules evaluation).
pub fn classify(index: &IdIndex, id: &str, proposed: &ProposedTxn) -> RowClassification;
```

`import_api.rs`'s dry-run/commit sequence: after getting hledger's normal proposal, rows with a
resolved id get reclassified via `reimport::classify` and split out of what's handed to
`hledger import` — `New` rows still go through hledger's own import+`.latest` bookkeeping unchanged;
`StatusOnly` rows go through a new, narrow **status-flip span editor** (new file,
`crates/ledgeline-core/src/status_edit.rs` or a small addition to `edit.rs` — scope: locate and
rewrite exactly the one status character/marker before an existing transaction's date, verify
(re-parse, confirm nothing else changed) before write, same discipline as `sort.rs`); `Conflicting`
rows are neither imported nor edited, only reported.

### Wire

`dry-run`/`commit` responses gain (structure to pin exactly when this phase starts, sketched here):

```jsonc
"idMatches": {
  "new": 3,
  "unchanged": 12,
  "statusChanged": [{"id": "...", "from": "pending", "to": "cleared"}],
  "conflicting": [{"id": "...", "diffs": [{"field": "amount", "existing": "$40.00", "incoming": "$42.00"}]}]
} // or null when the rules file declares no id column — existing behavior, unchanged
```

### Tests

Unit tests for `build_index`/`classify` (the three-way split, including the "id present but
identical" no-op case). Round-trip tests for the status-flip editor (byte-identical everywhere
except the one marker; refuses to write if verify finds any other change). Integration test:
`fixtures/import/reimport/pending-then-cleared/` — an OFX statement imported once, a second
"redownload" of the same statement where one txn's status changed and one row is genuinely new and
one existing journal transaction was hand-edited (amount changed) after the first import — asserts
the hand-edit survives untouched and is reported as `conflicting`, not overwritten. This fixture
also stands in as the regression test for the `TODO.md` pending/settled bug this phase closes.

## Sequencing

```
Phase 0 (parallel, independent)
  → Phase 1 → Phase 2
  → Phase 3 → Phase 4
```

Phase 3's extraction and Phase 1/2's SPA+rules.rs work touch disjoint files and can run as
parallel lanes once Phase 0 lands. Phase 4 waits on Phase 3's `import_runner.rs` existing, so its
new classification step is written once against the extracted core.

## Definition of done (per phase)

- `just check`, `just test`, `just e2e` green.
- `cargo test` stays hermetic; relevant `LEDGERLINE_HLEDGER_*_CHECK` opt-in suites pass locally.
- New/changed behavior has a test that failed before the change landed.
- UI changes exercised in the dev server at 375px and desktop width.
- Any contract in this doc that changed during implementation is amended here in the same commit,
  per `plans/00-overview.md` convention #9.
