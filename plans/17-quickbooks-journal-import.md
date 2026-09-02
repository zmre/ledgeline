# WP-17: QuickBooks General Journal import

Read `docs/imports.md` first — this WP adds a second import pipeline alongside the existing
CSV/OFX one and has to respect everything that doc's security/subprocess/aliasing sections
already establish, especially "No hledger we run reads a config file", the five-layer write
security discipline, and the existing account-aliases design ("Ledgeline reads aliases; it does
not apply them" — this WP is where that policy gets a narrow, deliberate, documented exception;
see Phase B below).

## Why this is a second pipeline, not an extension of the first

QuickBooks Online's "Journal" report (`Journal.xlsx` in the motivating case; `General_ledger.xlsx`
is a different, single-entry report with no categorization and is out of scope here) is **not**
one-row-per-transaction. hledger's CSV import rules — the entire mechanism `rules.rs`/`convert/`
build on — has no way to express "keep reading rows until a total line closes the group": that is
a structural limit of hledger's CSV importer, not a gap in our rules-file model. So this format
cannot go through `hledger import` at all. Transactions are constructed directly from the parsed
groups and written into the journal through the same transaction-writing capability
(`JournalEditor::add_transaction`, `crates/ledgeline-core/src/edit.rs`) the manual edit UI already
uses — nothing new is being taught to write a transaction; what's new is building one from this
shape of input.

## The real export's shape (verified against an actual QuickBooks Online export, not a description)

- Rows 1-3: a title block (company name, "Journal", date range) — ordinary preamble, already
  handled by existing spreadsheet-preamble detection.
- A header row, 14 columns in the motivating sample (customizable — QBO's report column picker
  lets a user add/remove columns, so detection must not require an exact set): `Transaction date`,
  `Transaction type`, `Num`, `Name`, `Description`, `Distribution account number`, `Account Name`,
  `Debit`, `Credit`, plus optional `Item class`, `Balance`, `Customer full name`, `Vendor`. Column A
  (the transaction-id/total marker column below) has **no header name at all**.
- Repeating groups, **no blank row between them**:
  - A marker row: column A holds a bare transaction id (QuickBooks' own internal Trans #, e.g.
    `"1"`, `"612"`) — every other column blank.
  - Two or more posting rows: column A blank; every OTHER column populated per posting (date and
    type are repeated on every posting row, not just the first — verified). Exactly one of
    Debit/Credit is populated per row; `amount = debit if debit else -credit` reproduces the
    correct signed amount for every account without needing to know the account's normal side.
  - A closing row: column A holds the literal text `"Total for {id}"`, with Debit/Credit columns
    both equal to the group's total — a strong, literal-string "end of group" signal, and a free
    balance-check oracle (Σ posting debits == Σ posting credits == this row's own two numbers; all
    three should agree, and a hand-truncated/corrupted export can show a formula error here —
    `#REF!` was observed directly — which must be a named refusal, not a crash or a silent zero).
- A trailing `TOTAL` row and a generation-timestamp footer row — ordinary trailer, same treatment
  as the title block.

`Item class`/`Customer full name`/`Vendor` are QuickBooks dimensional metadata with no hledger
equivalent; per direct instruction, they are preserved as tags on the written transaction/posting
rather than discarded (`class:…`, `customer:…`, `vendor:…`).

## Phase A — parse and group (`crates/ledgeline-core`, pure, no I/O beyond a byte slice)

New top-level module (`crates/ledgeline-core/src/qb_journal.rs` — a sibling of `reimport.rs`,
which is the precedent for "a self-contained concern that isn't the `Tabular`/CSV-rules shape and
doesn't belong under `convert/` or `rules/`"; confirm this placement makes sense once you're
looking at how much of `convert/spreadsheet.rs`'s raw-cell reading is actually reusable, and say
why in a contract amendment if you land somewhere else).

```rust
pub struct QbPosting {
    pub account: String,          // raw QuickBooks account name, e.g. "1520 Computer & Office Equipment"
    pub amount: Dec,               // signed: debit positive, credit negative
    pub memo: Option<String>,
    pub class: Option<String>,
    pub customer: Option<String>,
    pub vendor: Option<String>,
}

pub struct QbTransaction {
    pub id: String,                 // QuickBooks' own Trans #, e.g. "612" — becomes the id: tag
    pub date: String,               // normalized ISO YYYY-MM-DD
    pub transaction_type: String,   // "Deposit", "Expense", "Journal Entry", ...
    pub num: Option<String>,
    pub name: Option<String>,       // payee, when present — the transaction description candidate
    pub postings: Vec<QbPosting>,
}

#[derive(Debug, Error)]
pub enum QbJournalError {
    NoHeader,
    UnbalancedGroup { id: String, debit_total: Dec, credit_total: Dec },
    MismatchedTotal { id: String, computed: Dec, reported: Dec },   // our sum vs the report's own "Total for N" line
    MalformedTotal { id: String, cell: String },                    // e.g. the observed `#REF!`
    OrphanTotal { id: String },                                     // a "Total for N" with no matching marker row
    // ...
}

/// Does `bytes` look like a QuickBooks Journal export? Cheap, content-based — the header's
/// distinguishing columns (must have something recognizable as Debit AND Credit AND an account
/// name column) plus the marker/"Total for"-row grouping pattern, NOT an exact column-name or
/// column-count match, since QBO's report customizer lets a user add/remove columns.
pub fn detect(bytes: &[u8]) -> bool;

/// Parse and group. Every group must balance and its own total-row must agree with the computed
/// total, or the whole parse is refused naming which group and why — no partial-import-and-warn
/// for a construct this load-bearing to money.
pub fn parse(bytes: &[u8]) -> Result<Vec<QbTransaction>, QbJournalError>;
```

### Tests

Build fixtures from the shapes above (not from the pasted-text version in this WP's originating
conversation — verify against a real export's bytes, following this codebase's own rule).
Cover: a simple 2-posting group, a many-posting group (a manual Journal Entry with 10 lines), a
group whose reported total disagrees with the computed one, a `#REF!`/malformed total cell, an
orphan total row, a customizable column set (with and without `Item class`/`Balance`/`Customer`/
`Vendor` present — `Balance` in particular is a report-computed running figure with no clear
per-transaction meaning; confirm it is simply never read for anything, and say why in the module
docs so a future reader does not go looking for a use of it), and detection true/false-negative
cases against ordinary CSV/OFX/spreadsheet fixtures already in the corpus (must never misfire on
those).

## Phase B — the write pipeline (`crates/ledgeline-server`)

### The narrow alias exception

`docs/imports.md` states, deliberately: "Ledgeline reads aliases; it does not apply them" —
because reproducing hledger's regex alias dialect would be a near-miss silent-wrong-answer
generator. That policy is about *regex* aliases. A **plain** (non-regex) alias applied to an
**exact QuickBooks account string** needs no regex engine at all — it's string equality (and
hledger's own plain-alias rule, already documented in `aliases.rs`, that a plain alias also
matches a prefix ending at `:`, which is worth keeping for consistency but confirm it's actually
reachable here before relying on it, since QuickBooks account names in the sample never contain a
colon). This is the one place in the codebase Ledgeline computes an aliased name itself rather
than forwarding to hledger — say so explicitly, add a clear doc-comment cross-reference from both
this code and `docs/imports.md`'s policy section, and keep the implementation to plain-alias
exact/prefix matching only. A `/regex/` alias is not eligible and is left alone (the QuickBooks
account name it might have matched is simply reported as unmapped — see below — rather than
guessed at).

### The pipeline

1. Read every already-declared plain `alias` in the target journal (`Journal.aliases`, filtered to
   `!regex`).
2. Collect every distinct `QbPosting.account` across the parse. Partition into mapped (an alias
   covers it) and unmapped.
3. **Any unmapped account blocks the write** and is reported to the caller (see wire contract
   below) — per direct instruction, present them and ask for aliases rather than guessing or
   falling back to some default account. Writing an alias for a newly-resolved account uses the
   *existing* alias-writing path (`alias_api.rs`/`AliasDoc`) — this WP does not grow a second way
   to write an alias line.
4. Once every account resolves, build real `Transaction`s: one per `QbTransaction`, `description`
   from `name` when present else `transaction_type` (+ the first posting's memo if that still
   leaves nothing informative — decide the exact fallback chain once you're looking at real
   variety in the data, and record it), postings from `QbPosting`s with the resolved hledger
   account, the signed `amount`, and `class`/`customer`/`vendor` folded into the posting comment as
   tags. Tag the **transaction** itself with `id:{QbTransaction.id}` — the exact convention
   `crates/ledgeline-core/src/reimport.rs`'s `ID_TAG` already reads, so re-import matching is reuse,
   not new code (see next point).
5. **Re-import / incremental dedup**: before writing, build `reimport::IdIndex` from the target
   journal exactly as the CSV path does, and classify each `QbTransaction` by its id: new (write
   it), unchanged/conflicting (do not — `reimport::classify` compares field-by-field against
   hledger's own dry-run proposal for the CSV path; this path has no such proposal, so you'll need
   to construct the equivalent comparison directly against the freshly-parsed `QbTransaction` —
   check whether `classify`'s signature is already generic enough to take that, or needs
   generalizing; if you widen it, keep the CSV path's existing behavior byte-for-byte identical,
   proven by its existing tests still passing unmodified). A transaction whose id already exists
   and disagrees in some field is reported, never overwritten — same rule as the CSV path, for the
   same reason (a hand-edit is more likely than the export having changed).
6. Write via `JournalEditor::add_transaction` (`edit.rs`) for each new transaction, then one save —
   check whether multiple `add_transaction` calls can be batched before a single
   `save_and_publish`, matching how `edit_api.rs`'s multi-step patches already do this, rather than
   saving once per transaction.
7. **Sort afterwards** (direct instruction): after writing, run the same `sort::plan`/ordering-check
   this app already uses post-CSV-commit, and offer the same re-sort flow — reuse, not a new
   mechanism.

### Wire surface (sketch — pin exactly once you're building against the real stage/detect flow)

Reuses the *existing* stage upload (`POST /api/import/stage` already detects format and would need
`qb_journal::detect` wired into that dispatch) and needs new routes for: previewing the parsed
groups + which accounts are unmapped, submitting the aliases the user just typed for the unmapped
ones, and committing. Keep every existing invariant this surface already holds to: no absolute
path in any response body, token-gated, `Cache-Control: no-store`.

### Tests

Server-level tests mirroring `import_endpoints.rs`'s discipline: the write refuses while any
account is unmapped and says which ones; supplying aliases for exactly the unmapped set unblocks
it; a re-import of an overlapping export (some ids already present, one hand-edited afterward)
behaves like the CSV path's own `reimport` tests — same fixture-style regression coverage. A
`LEDGELINE_HLEDGER_*_CHECK`-style opt-in test (name it appropriately,
`LEDGELINE_HLEDGER_QBJOURNAL_CHECK`) proving `hledger check`/`hledger print` accept the written
journal and it balances.

## Phase C — SPA

Per direct instruction: drop a file into the existing New Transactions drop target; if
`qb_journal::detect` (surfaced through `capabilities`/`stage`) says yes with high confidence, the
screen switches to a **different** panel automatically rather than the ordinary
rules-candidate-matching flow; a lower-confidence detection prompts instead of silently switching.
New panel shows: the parsed groups (a count/preview is enough — this is not the rules editor),
input fields for every unmapped account (writing aliases through the existing alias-editing wire,
not a new one), and a commit action; after commit, the existing "journal is out of order, re-sort?"
prompt if the write left it out of order (reuse, not a new component). Component-test the
detection branch and the unmapped-account resolution flow; drive it in a real browser if the
sandbox allows (recent sessions on this branch have had Chromium/Firefox blocked here — fall back
to a live HTTP-route drive, as several prior phases on this branch did, and say so plainly if that
happens again rather than claiming a browser check that didn't occur).

## Phase D — CLI (deprioritized; do after A-C land and are verified)

Per direct instruction: the **same** `ledgeline import` subcommand, not a new one — for this path
there is no `-o`/rules file, since there's no intermediate CSV. Sketch:
`ledgeline import -i Journal.xlsx -j main.journal [--sort]`, detected the same way the GUI detects
it (missing `-r`/`-o` plus content sniffing, or an explicit flag if detection alone feels too
implicit for a script — decide once Phase A's detector is in hand and its false-positive rate is
known) — reusing `qb_journal::parse` + Phase B's write pipeline exactly, so GUI and CLI cannot
diverge, matching this branch's existing CLI's own governing rule. Unmapped accounts on the CLI
path have no one to prompt, so the run refuses and lists them — same "ask, don't guess" policy, a
non-interactive shape of it.

## Definition of done (per phase)

- `cargo test` stays hermetic; the new opt-in `LEDGELINE_HLEDGER_QBJOURNAL_CHECK` suite passes
  locally against real hledger.
- Every new/changed behavior has a test that failed before the change landed.
- `cargo fmt`, `cargo clippy -- -D warnings`, full `cargo test --workspace`, and (once SPA work
  lands) `vitest`/`tsc`/`svelte-check`/`prettier`/`eslint` all clean.
- Any contract in this doc that turns out wrong once real code/real hledger is in front of you is
  amended here in the same commit, per `plans/00-overview.md` convention #9 — this branch's other
  four phases each did this and each amendment caught something the sketch got wrong; expect the
  same here.
