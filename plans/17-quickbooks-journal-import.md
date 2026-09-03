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

## Getting the export, and why re-downloading is safe

User-facing documentation requirement, per direct instruction — this needs to land in **two**
places, not one: `docs/imports.md` (or a new `docs/quickbooks-journal-import.md` this WP's Phase B/C
docs pass should decide between) for the durable reference, **and** as in-app copy on Phase C's new
panel, since that is where someone doing this for the first time is actually looking. Whichever
phase writes user-facing text should carry this forward rather than re-deriving it:

1. In QuickBooks Online: **Reports → Journal**.
2. Choose the time frame.
3. Optionally **Customize → Rows/Columns** to add the `Vendor`, `Customer` and `Class` columns —
   this WP reads and preserves them as tags when present, and does nothing different when absent.
4. **Export to Excel** (not CSV — the customized-column xlsx is what Phase A parses).

**Re-downloading is safe, including with overlapping or repeated time frames, including "All
Dates" every time.** This is a direct, load-bearing consequence of Phase B's id-based dedup (the
same `reimport.rs` machinery the CSV/OFX path already uses): every transaction is tagged with
QuickBooks' own Trans #, so a transaction already in the journal is recognized and left alone (or
status-synced) no matter how many times its row reappears in a wider re-download. This is worth
stating plainly and confidently in the docs — it is *why* re-running "All Dates" is the
recommended habit rather than something to work around by fiddling with date ranges to avoid
overlap. Phase B's test suite should include a case proving exactly this: parsing and importing
the same export twice, and a *wider* export that re-contains already-imported transactions
alongside new ones, in both cases with no duplication.

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

### Contract amendments made during implementation (Phase A)

Per convention #9 in `plans/00-overview.md`. Phase A landed as
`crates/ledgeline-core/src/qb_journal.rs` — the placement the sketch proposed, and the reasoning
holds up for a sharper reason than "it isn't `Tabular`-shaped": `convert::spreadsheet::cell_text`
renders `Data::Error(_)` as an **empty string**, which is correct for a rules-file column and
fatal here, because `#REF!` in a total row is the loudest sign an export is damaged and blanking
it makes a corrupted group look balanced at zero. Two more reasons on top: `convert`'s
preamble/trailer trimming and blank-row dropping *move rows*, and grouping is defined by
adjacency; and `Tabular` is `Vec<Vec<String>>`, so `Float` and `Error` are already the same thing
by the time a caller sees it. So the sheet is read directly as `Range<Data>`, while the four
cell-reading primitives whose reasoning is hard-won (`is_populated`, `float_text`, `date_text`,
`iso_date_text`) are reused from `convert::spreadsheet` as `pub(crate)` rather than re-derived —
a second date-serial reader is exactly how the two would come to disagree. `edit::render_dec` is
`pub(crate)` for the same reason, so error messages render money the way the rest of the app does.

**Findings, all measured against the real export with `calamine` itself before any code existed.
Several contradict the description above.**

- **The closing row's Debit/Credit cells are FORMULAS** (`=I23+I24+…+I32`), and what a reader
  sees is the value Excel **cached** beside them. Nothing evaluates a formula. This is the
  single biggest thing the sketch's description missed, and it has a consequence worth stating
  because the balance check looks tautological once you know it: in an *untouched* export the
  total is a sum over the very rows above it and cannot disagree with them. What the check
  actually catches is **structural** damage — rows deleted, so the references break and the
  cached value goes stale or becomes `#REF!`, or an amount edited in a spreadsheet that did not
  recalculate on open. That is still exactly the failure the sample has.
- **Excel stores those cached values at up to seventeen significant digits** —
  `70120.850000000006`, `79.989999999999995`. Both are the nearest `f64` to a tidy cent value,
  and Rust's shortest-round-trip `f64` formatting inverts that *exactly*, so the comparison is
  exact `Dec` equality with **no tolerance anywhere**. The limit of that is visible in the same
  file: the whole-report `TOTAL` row, summing four hundred–odd values, drifted past half an ULP
  and prints as `65510189.6700001`. It is also the one row nothing reads.
- **The sample's damage is not (only) the `#REF!` the sketch names.** The tell is that the group
  whose marker says `6` is closed by a surviving `Total for 11024`. Its four postings balance
  perfectly at 533.94, so every arithmetic check passes and *only the id* knows rows were
  deleted. Closing rows are therefore matched to markers **by id**, never by position, and this
  earns its own error — `TotalIdMismatch { opened, closed, row }` — which is what the real file
  now produces: *"transaction 6 is closed at row 199 by a total for 11024 — the export has had
  rows removed"*.
- **Account names DO contain colons.** Ten of the export's eighteen do
  (`1520 Computer & Office Equipment:1521 Computer & Equipment - Accum Depr`) — QuickBooks renders
  a sub-account as `parent:child`. **Phase B's alias section below is wrong where it says "QuickBooks
  account names in the sample never contain a colon" and must be rewritten before it is
  implemented**: hledger's plain-alias prefix-ending-at-`:` rule is not only reachable here, it is
  reachable on more than half the accounts, and an alias on `1520 Computer & Office Equipment`
  will silently also rewrite the `1521` sub-account. That is either the desired behaviour or a
  bug, and Phase B has to decide which on purpose.
- **`Item class` is present in the header and empty on all 102 rows.** The column being
  customizable does not mean a customized column carries data.
- **`Balance` was worked out rather than left mysterious**, which is what makes it safe to
  ignore: it accumulates each posting's amount **signed by that account's normal balance side**,
  resetting at every group (verified across the file — equity credited adds, equity debited
  subtracts, an asset debited adds). Unusable for three independent reasons: it needs each
  account's declared type, which is nowhere in the export; it is float-computed (a cell really is
  stored as `34918.979999999996`); and it is scoped to the report's date range. Never read.
  **Confirmed directly with the user (asked explicitly whether this should drive hledger balance
  assertions, since a customized export can carry it): skip it entirely.** It is not any account's
  balance at all — a single running figure crosses every account touched by a group, not one
  account's history — so it cannot become an `= amount` assertion (which claims the account's
  *whole-ledger* cumulative balance) without producing assertions that fail on the first
  `hledger balance`. `Layout` has no `balance` field; the column is not even located.
- **Unused text cells on a posting row are `Data::String("")`, not `Data::Empty`** — so "empty"
  must mean "nothing printable". Keyed on the variant, every posting acquires `vendor: Some("")`
  and a `vendor:` tag with nothing after it goes into the user's journal.
- Confirmed as described: no blank row between groups (all 46); exactly one of Debit/Credit per
  posting row (54 and 48 over 102, none with both or neither); date, type, `Num` and `Name` repeat
  on every posting row and **never vary within a group** (checked on all 46); `Description` **does**
  vary within a group (six distinct memos across one ten-line Journal Entry, and four across a
  Bill) and so is per-posting. Column A genuinely has no header label.

**Types and signatures as landed**, deviating from the sketch in three places:

- `parse(bytes: &[u8]) -> Result<QbJournal, QbJournalError>` — **not** `Vec<QbTransaction>`.
  `QbJournal { transactions: Vec<QbTransaction>, date_format: DateFormatGuess }`. The date format
  turns out to be a whole-file question the sketch did not consider: `01/02/2026` is two different
  days depending on a QuickBooks *account* preference the export does not record, and the only
  evidence is whether some other date in the file has a component above twelve. The sample resolves
  cleanly (`%m/%d/%Y`, `ambiguous: false`) **only because it happens to contain the 17th–20th**; an
  export confined to one short period does not. `rules::generate::guess_date_format` is reused
  rather than re-derived — it is already tested against the hledger binary — and its `ambiguous`
  flag is passed through for Phase B/C to resolve with the user rather than settled by a coin toss
  here. **Phase C needs a UI affordance for this that the sketch does not mention.**
- `QbPosting` gained nothing and lost nothing; `QbTransaction` is as sketched. `Distribution
  account number` is deliberately **not** carried: where populated (78 of 102 rows) it is the
  leading numeric token of the account name's own leaf segment, so it is redundant with `account`.
- The error enum is much wider than the sketch's four, because each of these is a distinct thing
  that can be wrong with a real file and "the import failed" over a 200-row report is not
  actionable: `Empty`, `TooLarge`, `Unreadable`, `NoHeader`, `PostingOutsideGroup`, `OrphanTotal`,
  `TotalIdMismatch`, `UnclosedGroup`, `EmptyGroup`, `MissingAccount`, `MissingDate`, `MissingType`,
  `AmountNotSplit`, `MalformedAmount`, `MalformedTotal`, `UnbalancedGroup`, `MismatchedTotal`,
  `AmountOverflow`, `UnreadableDates`, `UnreadableDate`, `NoTransactions`. Every variant that is
  about a group names the group; every variant that is about a row names the **1-based sheet row**,
  which is the number down the side of the user's own spreadsheet. None can carry a path.
- `detect(bytes: &[u8]) -> bool` as sketched, content-only and taking no format hint — the
  opposite of `convert::spreadsheet::parse`, which takes the format from its caller so a file whose
  extension lies is refused rather than reinterpreted. Here there is no extension in play. It
  requires **two** conditions: the header triple (something Debit-like AND Credit-like AND
  account-name-like) *and* the grouping structure (a marker row plus a `Total for {id}` whose id a
  marker actually opened). The second carries the weight — `fixtures/import/qb-journal/near-miss.xlsx`
  is an ordinary bank export satisfying the first completely. Detection deliberately says **yes** on
  a damaged export and lets `parse` refuse it, so a truncated file reaches the named refusal
  instead of falling back to the CSV rules screen.

**One implementation rule worth carrying into Phase B/C:** a marker row is confirmed by what
*follows* it, not by its shape. The merged title band above the header and the timestamp footer
below the data have a marker's exact shape — one populated cell in column A, nothing else — so a
shape-only rule opens a group on the footer and refuses the file for never closing it.

**Verification against the real file** (which is not committed — it carries a real company's
accounts, payees and balances, and the corpus rule is that everything in `fixtures/` is synthetic
or scrubbed): `detect` returns true; `parse` refuses with the `TotalIdMismatch` message quoted
above; and with the six damaged rows removed it yields **45 transactions and 98 postings, all
balanced**, at `%m/%d/%Y` unambiguous. Spot-checked signs against what the accounting must mean
rather than against the arithmetic: a transfer credits checking (−1696.87) and debits the card
(+1696.87, paying the liability down toward zero); a card expense credits the card (−79.99) and
debits marketing (+79.99). Fixtures reproducing all nine shapes are in
`fixtures/import/qb-journal/`, built by the corpus-wide `generate.py` and documented in their own
`README.md` — including why that script has to rewrite `sheet1.xml` after openpyxl saves it
(openpyxl cannot write a formula's cached value, and a formula with no cached value is a workbook
no spreadsheet has ever produced).

## Phase B — the write pipeline (`crates/ledgeline-server`)

### The narrow alias exception

`docs/imports.md` states, deliberately: "Ledgeline reads aliases; it does not apply them" —
because reproducing hledger's regex alias dialect would be a near-miss silent-wrong-answer
generator. That policy is about *regex* aliases. A **plain** (non-regex) alias applied to an
**exact QuickBooks account string** needs no regex engine at all — it's string equality, plus
hledger's own plain-alias rule that a plain alias also matches a prefix ending at `:`
(`hledger_conf::conf_argument`'s module docs: "verified: it rewrites `a` and `a:sub` and leaves
`abc` alone").

**Resolved (was an open question after Phase A, now decided):** the prefix rule applies here,
on purpose, and cascades — an alias on `1520 Computer & Office Equipment` also rewrites
`1520 Computer & Office Equipment:1521 Computer & Equipment - Accum Depr`, preserving the
`:1521 …` suffix on the new name. This is not a new call; it is the same behaviour
`docs/imports.md`'s "Column interpolation composes with it" paragraph already relies on for
every other import ("a prefix alias rewrites the base and leaves `:cash` intact, so one alias
covers every subaccount rather than needing one per account × type") — and it is what real
hledger does with the same alias if the journal contained these account names directly. Ten of
the real export's eighteen accounts carry a colon (QuickBooks renders a sub-account as
`parent:child`), so this is the common case here, not an edge case: one alias per QuickBooks
*parent* account is normally enough, and Phase B's unmapped-account detection (below) must check
the prefix match too, not just exact equality — an account is "mapped" if some alias's pattern
equals it OR is a `:`-bounded prefix of it, exactly mirroring `conf_argument`'s own rule, so the
user is never asked for an alias on a sub-account whose parent already has one.

This is the one place in the codebase Ledgeline computes an aliased name itself rather than
forwarding to hledger — say so explicitly, add a clear doc-comment cross-reference from both this
code and `docs/imports.md`'s policy section, and keep the implementation to plain-alias
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
6. Write via `JournalEditor::add_transaction` (`edit.rs`) for each new transaction, using
   **`InsertPosition::DateOrdered`** — the same default `edit_api.rs` already uses for every GUI
   "add transaction" call — so a multi-year import routes each transaction into whichever
   `include`d per-year/per-month file its chronological neighbors already live in, matching
   `placement_for`'s existing behavior (`edit.rs`), rather than piling the whole batch into
   whatever single file the CSV `hledger import` path defaults to. **Edge case to carry into the
   UI**: `JournalEditor` never creates a new file, so a transaction older than everything the
   journal currently holds has no predecessor and lands in the *earliest existing* file — a QB
   export reaching back further than any file on disk does not auto-split into a new year file,
   and Phase C should surface that rather than let it land silently. Then one save — check whether
   multiple `add_transaction` calls can be batched before a single `save_and_publish`, matching how
   `edit_api.rs`'s multi-step patches already do this, rather than saving once per transaction.
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

### Contract amendments made during implementation (Phase B)

Per convention #9 in `plans/00-overview.md`, and following Phase A's own amendments section above.

**The pure alias/description logic landed in `ledgeline-core`, not `ledgeline-server`.**
`crates/ledgeline-core/src/qb_import.rs` is a new sibling of `qb_journal.rs` carrying
`plain_aliases`, `resolve_account`, `unmapped_accounts` and `description_for` — none of it touches
I/O or the HTTP layer, and it is unit-tested in isolation (inline `#[cfg(test)]`, plus
`crates/ledgeline-core/tests/qb_import.rs` against the real `simple.xlsx`/`many-postings.xlsx`
fixtures) the same way `reimport.rs`/`aliases.rs` are. The HTTP glue, wire types, staging, and the
`core::Transaction` builder (which needs `edit_api::infer_style`, a server-crate concern) live in
the new `crates/ledgeline-server/src/qb_journal_api.rs`.

**Which aliases count: `aliases_in_force`, not just `!regex`.** The sketch above says "Journal
.aliases, filtered to `!regex`" and does not mention `end aliases` scoping. Decided to also exclude
an alias an `end aliases` line has closed — i.e. `qb_import::plain_aliases` is
`journal.aliases_in_force().filter(|a| !a.regex)` — because that is the only existing precedent for
"the aliases in force" (`aliases::forward`'s CSV-path forwarding already excludes them for the same
reason: the user wrote down where the mapping stops) and using one they explicitly closed for a
brand-new mapping would contradict their own stated intent.

**The description fallback chain, decided against real data:** `name` when non-empty; otherwise
`"{transaction_type}: {first posting's memo}"` when the first posting has one; otherwise bare
`transaction_type`. Verified against `many-postings.xlsx`'s group 612 (a manual Journal Entry with
no `Name` at all) — `crates/ledgeline-core/tests/qb_import.rs`'s
`the_manual_journal_entry_has_no_name_and_falls_back_to_type_and_first_memo` pins the exact string
`"Journal Entry: Opening Balance Entry"`. Reasoning: `transaction_type` alone is a six-word closed
set, so on its own it gives every un-payee'd transaction of a kind the identical description; the
first posting's memo is what actually distinguishes one Journal Entry from the file's others.

**`class`/`customer`/`vendor` become a posting comment of `name: value` tags**, comma-joined
(`"class: Retail, vendor: Acme Cloud\n"`), via `qb_journal_api::posting_comment`. This is exactly
the tag-comment syntax `parse.rs::parse_tags` already reads (`name` is the last whitespace-run
before a `:`, value trimmed to the next `,`), so a value containing a comma or colon of its own
degrades the *derived* `Posting::tags` reading but never corrupts the write — `edit::
transactions_equivalent`'s round-trip guard does not compare comments/tags at all (only
date/date2/status/code/description/postings' account+ptype+assertion+amounts), so this was
verified rather than assumed.

**The commodity for a built amount, corrected during review: `Journal::default_commodity` alone is
not enough.** The export carries no currency column at all, so something has to decide it. The
first landed version used `Journal::default_commodity` (a `D AMOUNT` directive), falling back to
bare `Commodity("")` when the journal declares none — but `default_commodity` requires a literal `D`
line, which most real journals never write (they just write `$100.00` throughout), so on an
otherwise-ordinary `$`-denominated journal with no `D` directive every QuickBooks-imported amount
was silently written with **no commodity symbol at all** — a different style than the rest of the
file, and not something either the parser or `hledger check` catches, since a bare-number posting
still balances against another bare-number posting in the same transaction. Caught by strengthening
`qb_journal_endpoints.rs`'s `commit_writes_every_transaction_tagged_with_its_quickbooks_id` to assert
the `$` sign survives (its scratch journal fixture writes `$1000.00` but declares no `D`, which is
exactly the shape that tripped this) — the assertion failed against the landed code, confirming the
bug before it was fixed. `qb_journal_api::commodity_for` now prefers, in order: the declared
`default_commodity`; else the commodity used most often across the journal's own posting amounts
already (first-seen order breaking a tie — the same "first occurrence" precedent
`Journal::commodity_styles`'s own doc comment names); else bare, for a journal with no default and
no amount anywhere to learn from, which is the best any answer can be there. Computed once per
commit/preview (not once per transaction) and threaded through `build_and_classify` →
`build_transaction` → `build_posting`. `AmountStyle` is still inferred with the *existing*
`edit_api::infer_style` (`pub(crate)`, reused rather than re-derived), the same function the manual
"add transaction" editor already uses for a client-supplied amount with no declared style — that part
of the sketch was right; only *which* commodity was fed to it was wrong. Four new inline tests in
`qb_journal_api.rs` pin the four cases (declared default wins even over a more-frequent commodity;
no default falls back to the journal's own amounts; a genuine tie breaks by first occurrence; a
journal with neither has nothing to prefer).

**`reimport::classify` needed no generalization.** The sketch worried its signature might need
widening because it "compares field-by-field against hledger's own dry-run proposal" and this path
has none. In fact `classify(index, id, proposed: &Transaction, status_mapped: bool)` already takes
a plain `&Transaction` — nothing about it is CSV-specific — so `qb_journal_api::build_and_classify`
calls it completely unmodified, passing a QuickBooks-built `Transaction` as `proposed` and
`status_mapped: false` *always* (a QuickBooks-built transaction is always `Status::Unmarked`, since
nothing in the export maps to hledger's clearing status — see `WireQbIdMatches`'s doc comment for
why that makes `RowClassification::StatusOnly` unreachable here and folds any status difference
into `Conflicting`, the same rule the CSV path follows when its own rules file assigns no status).
`reimport.rs` itself is untouched, and its existing tests pass byte-for-byte unmodified — the
"prove it" the sketch asked for.

**No `journalId`, anywhere on this surface.** CSV import writes to one file the user names because
`hledger import` is pointed at one target. This pipeline writes through
`JournalEditor::add_transaction` with `InsertPosition::DateOrdered`, which already decides — per
transaction, from the journal's own chronology — which `include`d file receives each row. There is
therefore no "destination" to name at all, and the commit request carries only a `stageId`.

**Wire surface, pinned exactly (the sketch's "sketch — pin exactly once building against the real
stage/detect flow" resolved as):**
- `POST /api/import/stage` (existing route, `import_api.rs`) — `qb_journal::detect` is checked
  before `convert::detect`; on a match, `stage_qb_journal` parses **eagerly** (so a damaged export
  is refused by name at upload time, not staged and refused later) and stages the *parsed*
  `QbJournal` — see the next point — then answers with the **existing** `WireStage` shape:
  `format: "quickbooks-journal"`, `candidates: []`/`statement: None`/`preview` at its empty
  defaults (there is no CSV to preview or score rules files against), `defaults.journalId` still
  populated from the existing `defaults_for` helper purely for display continuity even though this
  pipeline never reads it back. This is the most literal reading of "reuses the existing stage
  upload" available: one new wire type for the whole route, not a parallel one.
- `GET /api/import/qb-journal/{stageId}` (new, `qb_journal_api::preview`) — read-only and
  idempotent, so `GET` with the handle as a path segment, not the `POST`-with-body shape the sketch
  implied by grouping it with the write routes. Re-parses nothing (see below) and re-classifies
  against the journal's aliases *as they stand at call time*, so calling it again after adding an
  alias through the existing `PUT /api/aliases/{*journalId}` is how "supplying aliases … unblocks
  it" is proven in the test suite.
- `POST /api/import/qb-journal/commit` (new, `qb_journal_api::commit`) — the only write route.

**Staging holds the *parsed* `QbJournal`, not raw bytes.** `qb_journal_api::QbStageArea` /
`QbStage` hold an in-memory `QbJournal` (never written to disk — this pipeline never shells out, so
there is no file-alignment concern `stage::Stage` has to solve). Parsing happens exactly once, at
upload time; `preview` and `commit` both read the same parse, so there is no second reader that
could come to disagree with the first about what the bytes mean, and re-classification against
fresh aliases costs no re-parse.

**The git safety net is applied, unlike the manual "add transaction" editor's own routes.**
Investigated per the plan's own instruction: `edit_api.rs`'s ordinary transaction endpoints
(`POST`/`DELETE`/`PUT`/`PATCH /api/transactions`) carry **no** git integration at all — only
`import_api.rs`'s CSV commit and sort routes do. Decided QuickBooks import should follow the import
precedent, not the manual-edit one: it is a bulk write from an external file with the same
undo-cost profile as a CSV import, not a one-transaction-at-a-time interactive edit. Two
consequences of not knowing which files `DateOrdered` will touch until after the write:
- the **pre**-write check (`import_api::blocked_by_git`, now `pub(crate)`) looks at *every* file in
  `Journal::source_files` — the superset of anything the write could touch — rather than one known
  target, which is more conservative than the CSV path but the only correct pre-check available;
  and
- a new `JournalEditor::dirty_files()` (`crates/ledgeline-core/src/edit.rs`) reports exactly which
  files ended up written, so the **post**-write git commit (`import_api::commit_targets`, now
  `pub(crate)`) is narrowed to precisely those — the same "commit exactly what was written"
  precision the CSV path has, achieved a different way since there is no single target to already
  know it from. `crates/ledgeline-core/tests/edit.rs`'s
  `dirty_files_lists_exactly_the_files_a_write_touched` pins this against a real two-file
  `include` tree.
- A new `edit_api::add_transactions` (batches several `add_transaction` calls behind one
  `save_and_publish`, mirroring `set_statuses`'s existing precedent exactly) is what both performs
  the write and returns the `dirty_files()` snapshot taken just before the save.
- `qb_journal_api::commit` does **not** additionally take `AppState::import_writes` — it goes
  through the editor mutex (via `add_transactions`), which already serializes against every other
  editor-based writer including itself; CSV import and the alias editor take `import_writes`
  because *they* bypass the editor and write the file directly. The one race this leaves — a QB
  commit and a concurrent CSV commit both touching the same file — is caught, not silently lost:
  `JournalEditor::save`'s external-change fingerprint check turns it into an `EditError::
  ExternalChange` → `409` the client re-fetches and retries, the same outcome the manual editor
  already accepts for the identical race.

**Ordering is reported per touched file**, not the CSV path's single `WireOrdering`/`WireMove`
(reused directly — both structs are now `pub(crate)` in `import_api.rs` — but wrapped in new
`WireQbOrdering { inOrder, files: [{ journalId, inOrder, moves }] }`), because a multi-year import
can land rows in more than one `include`d file. No new re-sort mechanism was built: each
`WireQbFileOrdering.journalId` is a relative handle in exactly the shape `journals::targets`
produces, so a client can hand it straight to the *existing* `POST /api/import/sort` route — reuse,
per the plan's own instruction, not a new one.

**No response list is capped.** Unlike `import_api::WireIdMatches` (`MAX_ID_REPORTS`/
`MAX_ID_DIFFS`), `WireQbIdMatches.conflicting` and its diffs are not bounded. A QuickBooks Journal
export is a spreadsheet within the parser's 16 MiB cap, and every fixture in the corpus tops out at
45 transactions; this is a deliberate simplification for a first cut, not an oversight, and the
same caps could be ported over verbatim if a future export size warrants it.

**The date-format ambiguity flag is surfaced, not resolved, at this layer.** `WireQbPreview.
dateFormat.ambiguous` passes `QbJournal::date_format.ambiguous` straight through so a future caller
can warn about it; Phase B always proceeds with the ISO dates `qb_journal::parse` already computed
(there is no interactive resolution mechanism at this layer to defer to). Phase A's amendment
already flagged this as needing a Phase C UI affordance — still true, still deferred.

**Balance column: still never read.** Nothing in Phase B changes Phase A's settled answer —
`QbPosting` carries no balance field, and no balance-assertion logic was added anywhere in this
phase.

**A new fixture, `fixtures/import/qb-journal/overlap.xlsx`, added for the "wider re-download" case.**
The plan's own "Getting the export, and why re-downloading is safe" section calls for a test proving
a re-download that mixes already-imported ids with new ones dedupes correctly in one commit; no
existing fixture's ids overlapped another's (verified directly against the sheet XML), so
`generate.py` gained `qb_overlap_xlsx`, following its existing `_qb_write`/`_qb_sheet` conventions
exactly: group `441` is `QB_DEPOSIT` byte-for-byte (the same transaction `simple.xlsx` carries under
that id), group `6` is `QB_BILL` (an id neither `simple.xlsx` nor `default-columns.xlsx` uses).
`qb_journal_endpoints.rs`'s `a_wider_re_download_imports_only_the_new_group_and_leaves_the_overlap_alone`
commits `simple.xlsx` then `overlap.xlsx` and asserts exactly one new transaction lands and the
overlapping one is not duplicated. (Regenerating fixtures locally requires the exact `nix-shell
-p python3Packages.openpyxl python3Packages.xlwt python3Packages.odfpy` toolchain the script's own
header documents — a bare `pip`-installed `openpyxl` produced byte-different output for every
*existing* fixture too, which is spurious zip/library-version noise, not a content change; only the
one new file was kept, and every previously-committed fixture was restored via `git checkout --`
rather than regenerated in place.)

**Test counts.** `ledgeline-core`: 12 inline `qb_import` unit tests + 4 fixture-based
`tests/qb_import.rs` + 1 new `edit.rs` test (`dirty_files_lists_exactly_the_files_a_write_touched`).
`ledgeline-server`: 4 inline `qb_journal_api::commodity_for` unit tests + 14 hermetic tests in
`tests/qb_journal_endpoints.rs` (token gate, stage detection/refusal, preview's unmapped-accounts
report, commit's refusal-by-name, the write itself (including that an imported amount keeps the
journal's own commodity symbol), re-commit idempotency, hand-edit-is-conflicting-and-never-
overwritten, the wider-re-download case, per-file ordering) + 2 opt-in tests in
`tests/qb_journal_hledger_check.rs` gated on `LEDGELINE_HLEDGER_QBJOURNAL_CHECK` (added to
`justfile`'s `hledger-checks` target), proving `hledger check` and `hledger print` accept the written
journal for both the two-transaction `simple.xlsx` and the full 45-transaction `report.xlsx`
round-trip fixture. The existing CSV-path suites (`import_endpoints.rs`, `import_cli.rs`,
`reimport.rs`, and the full `just hledger-checks` target) were re-run and pass unmodified, confirming
the several `pub(crate)` visibility widenings in `import_api.rs`/`edit_api.rs`/`stage.rs` changed no
behavior.

## Phase C — SPA

Per direct instruction: drop a file into the existing New Transactions drop target; if
`qb_journal::detect` (surfaced through `capabilities`/`stage`) says yes, the screen switches to a
**different** panel automatically rather than the ordinary rules-candidate-matching flow.

**Resolved before Phase C started (the sketch above assumed a confidence gradient that does not
exist): always auto-switch, never prompt first.** `qb_journal::detect` is a plain `bool`, not a
score — there is no "lower confidence" reading for a prompt to gate. It was proven accurate over
the whole fixture corpus in Phase A (zero false positives, including against a bank export built
specifically to look like a near miss) and it deliberately says **yes** even on a damaged real
export, so a truncated file still reaches the QuickBooks panel and gets the specific named parse
refusal (`qb_journal::parse`'s error, surfaced by `stage_upload`'s eager parse — see Phase B's
amendments) rather than silently falling through to the CSV rules screen it cannot actually match.
`POST /api/import/stage` already makes this decision server-side, before the SPA sees anything: the
response's `format` field is `"quickbooks-journal"` XOR the ordinary CSV/spreadsheet formats,
verified by Phase B's `a_quickbooks_journal_upload_is_detected_and_diverted` /
`an_ordinary_csv_still_takes_the_csv_path`. So the SPA's job is only to branch on `format`, nothing
more — no new client-side detection, confidence UI, or confirmation step.

New panel shows: the parsed groups (a count/preview is enough — this is not the rules editor, and
`WireQbPreview.sample` already gives up to 20 flattened transactions with descriptions and
postings for exactly this), input fields for every unmapped account (`WireQbPreview
.unmappedAccounts`; writing aliases through the existing alias-editing wire, `PUT
/api/aliases/{*journalId}`, not a new one — re-`GET /api/import/qb-journal/{stageId}` afterward to
see them drop off the list, same as Phase B's own test does), and a commit action
(`POST /api/import/qb-journal/commit`, body `{stageId}`, disabled while `unmappedAccounts` is
non-empty). After commit, the existing "journal is out of order, re-sort?" prompt if
`WireQbCommit.ordering.inOrder` is false for any file — `WireQbCommit.ordering.files[].journalId`
is a relative handle usable directly with the existing `POST /api/import/sort` route (reuse, not a
new component); Phase B deliberately made this per-file since a multi-year import can touch more
than one. Component-test the format-branch and the unmapped-account resolution flow; drive it in a
real browser if the sandbox allows (recent sessions on this branch have had Chromium/Firefox
blocked here — fall back to a live HTTP-route drive, as several prior phases on this branch did,
and say so plainly if that happens again rather than claiming a browser check that didn't occur).

### Contract amendments made during implementation (Phase C)

Per convention #9 in `plans/00-overview.md`, following Phases A and B's own amendments sections
above. Unlike those two, nothing here corrects a wrong assumption about the WIRE — building against
the real `qb_journal_api.rs` handlers and re-driving the whole flow over HTTP (see "Verification"
below) confirmed every field name, nullability and status code the plan and Phase B's amendments
already documented, byte for byte. What follows is Phase C filling in the decisions the plan
deliberately left to it ("decide the exact fallback chain once you're looking at real variety",
"Phase C needs a UI affordance for this that the sketch does not mention") plus a couple of
structural calls the sketch didn't need to make because it wasn't looking at the SPA's existing
conventions yet.

**New files.** `web/src/lib/imports/qbJournalModel.ts` (pure decisions, mirroring
`importModel.ts`/`aliasModel.ts`'s own house rule: no Svelte/DOM/`fetch`, tested under the `unit`
vitest project) — `isQuickbooksJournalStage` (the one branch point), `dateFormatNotice`,
`mappingDraft`/`mappingProblems`/`mappingEdits`/`hasMappingsToSave`, `defaultAliasTargetFile`,
`canCommitQbJournal`, `qbIdMatchesSummary`, `filesNeedingSort`/`qbReorderOffer`.
`web/src/lib/imports/qbJournalStore.svelte.ts` (the data layer: a `preview` and a `commit`
`createResource`, a mapping-draft dispatcher, a per-file re-sort dispatcher — shaped like
`importStore.svelte.ts`). `web/src/lib/imports/ui/QbJournalPanel.svelte` (the panel
`NewTransactionsPanel` mounts instead of `StagedPanel`/`DryRunPanel`/`ResultPanel`) and
`web/src/lib/imports/ui/QbUnmappedAccounts.svelte` (the mapping form, its own file because it is the
most stateful piece and the plan calls out testing it specifically). Wire decoders
(`decodeQbPreview`/`decodeQbCommitResult` plus their private helpers) and the domain types
(`QbPreview`/`QbCommitResult`/`QbIdMatches`/`QbDateFormat`/`QbSample`/`QbOrdering`/`QbFileOrdering`)
landed in the existing `nativeDecode.ts`/`importTypes.ts` rather than new files, matching how every
other `/api/import/*` type already lives there. Two new `LedgelineApi` methods,
`qbJournalPreview`/`qbJournalCommit`, in the existing `native.ts`.

**`QbIdMatches` deliberately does NOT reuse the CSV path's `IdMatches` domain type**, even though
`WireQbIdMatches` reuses `conflicting`/`diffs` byte-for-byte (`RawConflict`/`RawFieldDiff` are
shared as-is). `IdMatches` carries `statusChanged`/`statusChangedTotal`, which `WireQbIdMatches` has
no wire fields for at all (see that struct's own doc comment in `qb_journal_api.rs`: a
QuickBooks-built transaction is always `Status::Unmarked`, so `RowClassification::StatusOnly` is
unreachable here). A shared type would need those two fields to be optional or defaulted on a shape
that can never actually produce them — exactly the kind of "the type can express a state that cannot
happen" gap `docs/imports.md`'s conventions warn against — so `QbIdMatches` is its own four-field
interface and `qbIdMatchesSummary` (the QB analogue of `importModel.idMatchesSummary`) only writes
the conflict half of that function's sentence, because there is no status-sync half to report.

**Resolving an unmapped account reuses the alias editor's OWN validation, not a re-derived copy.**
`qbJournalModel.mappingDraft(account, replacement)` builds an `aliasModel.AliasDraft` with the
QuickBooks account name as a FIXED, non-regex `pattern` and the user's typed text as `replacement`,
and `mappingProblems`/`mappingEdits` call `aliasModel.validateRow` on it directly. The plan's Phase B
section already established that a plain alias's prefix rule cascades to sub-accounts
(`1520 …` also covers `1520 …:1521 …`); this means the SAME rule applies here, for free, with no
extra code — an alias added for `6000 Sales and Marketing` from this screen also covers
`6000 Sales and Marketing:6001 Sales & Marketing Tools`, verified directly against the real
`simple.xlsx` fixture (see "Verification" below). `mappingEdits` silently skips a blank or
`mappingProblems`-flagged row rather than refusing the whole batch, so typing three good mappings
and leaving a fourth half-finished still submits the three — the fourth stays listed on the next
preview rather than blocking on it.

**Which journal file a new alias is appended to, when the SPA has to choose: the first WRITABLE file
`GET /api/aliases` lists** (`qbJournalModel.defaultAliasTargetFile`), mirroring
`AliasPanel.svelte`'s own default-selection rule (`files.find(...) ?? files[0]`) rather than
inventing a second "which file" policy. The plan's wire contract has no field for this (Phase B's
"No `journalId`, anywhere on this surface" amendment is about the QB *commit* route, not the
pre-existing alias route this reuses), so Phase C had to decide, and picked the same file the
Account Aliases tab already defaults to for consistency between the two screens.

**One batched `PUT` per "Map accounts" press, not one per row.** The plan says "writing aliases
through the existing alias-editing wire… not a new one" but does not say whether each unmapped
account gets its own round trip or all of them travel together.
`SaveAliasesBody.edits` is already an array, and `qbJournalStore.saveMappings` sends every valid
typed row as one `edits` array in a single `aliasStore.save` call — proven against the real server in
"Verification" below (four accounts, one `PUT`, one new revision, one re-fetched preview with all
four gone from `unmappedAccounts`).

**The date-format ambiguity affordance Phase A's amendments flagged as missing from the sketch:**
`qbJournalModel.dateFormatNotice(dateFormat)` reads `WireQbPreview.dateFormat.ambiguous` and, when
true, tells the user which reading was assumed and asks them to check the sample before committing —
`QbJournalPanel.svelte`'s `qb-date-notice` testid. Nothing resolves the ambiguity (there is nothing
to resolve it WITH, per Phase B's own amendment — the ISO dates are already fixed by the time this
screen sees them); this is surfacing, exactly as Phase A's amendment anticipated.

**No new "confirm before write" step, and no way to hide the commit button once every account
resolves** — the plan's "always auto-switch, never prompt first" instruction for the format branch
extends naturally to the commit action too: `canCommitQbJournal` is a pure function of whether
`unmappedAccounts` is empty, with nothing else gating it (no separate "I've reviewed the sample"
checkbox). A re-press after a successful commit is left enabled rather than disabled, on purpose:
Phase B's id-based dedup makes a repeat commit inert (`imported: 0`, every id `unchanged` — verified
live, see below) exactly the way a repeat CSV import already is, so disabling it would be protecting
against a failure mode that cannot occur.

**Verification against the real server, not just fixtures (no browser available — see below).**
Built `cargo build -p ledgeline`, started `ledgeline --server` against a throwaway one-transaction
journal, and drove the exact sequence the SPA now performs, over `curl`, against `simple.xlsx`:
`POST /api/import/stage` → `format: "quickbooks-journal"`; `GET /api/import/qb-journal/{stageId}` →
four `unmappedAccounts` including the colon-bearing `"6000 Sales and Marketing:6001 Sales & Marketing
Tools"`; `POST …/commit` while unmapped → `400` naming all four; `PUT /api/aliases/main.journal` with
four batched `append` edits (one of them `6000 Sales and Marketing` — the PARENT, not the child) →
`200`; re-`GET` the preview → `unmappedAccounts: []`, `idMatches.new: 2`; `POST …/commit` → `200`,
`imported: 2`, the sub-account's alias-rewritten name
(`expenses:marketing:6001 Sales & Marketing Tools`) present in the written journal exactly as the
prefix rule predicts, `class`/`customer`/`vendor` tags present in posting comments, `$` commodity
matching the journal's own style, `git.committed: true`; a second, identical commit → `200`,
`imported: 0`, `idMatches.unchanged: 2`, `git: null` (nothing to commit); and
`POST /api/import/sort` accepting the `journalId` handle the commit response named. Every field name
and status code matched this doc and `qb_journal_api.rs` exactly — nothing here produced a Rust-side
contract amendment, so none was needed.

One unintended side effect from that exercise, caught and corrected before finishing: the scratch
journal directory was created *inside* this git worktree, so the server's own git-autocommit feature
found the enclosing repo and committed the scratch file into this branch's real history. Caught
immediately by `git log`, undone with `git reset --mixed` back to this doc's own prior commit (no
`--hard`, nothing else touched), and the scratch directory itself was deleted rather than left in the
tree. Worth recording so a future verification pass uses a directory genuinely outside any git
working tree (`/tmp` in a sandbox that allows writing there) instead.

**No real browser was available.** Chromium failed to launch in this sandbox with
`Check failed: kr == KERN_SUCCESS. bootstrap_check_in … MachPortRendezvousServer … Permission denied`
— a macOS sandbox mach-port restriction, not an application bug — confirming the plan's own warning
that recent sessions on this branch have had this blocked. One attempt was made
(`chromium.launch()` directly through `@playwright/test`) before falling back to the `curl` sequence
above, per this doc's own instruction not to claim a browser check that did not occur.

**Test counts.** `qbJournalModel.test.ts`: 25. `qbJournalStore.test.ts`: 14 (every test gets an
isolated module graph via `vi.resetModules()` + dynamic re-import, because `qbJournalStore` and the
`aliasStore` singleton it reuses are both module-scope state shared across a test file).
`nativeDecode.test.ts` gained 10 (the new `decodeQbPreview`/`decodeQbCommitResult` describe blocks —
hand-written literal wire JSON rather than a `fixtures/native/v1/*.json` golden, since this route
has no golden fixture the way the reports routes do). Two new component-test files:
`QbJournalPanel.svelte.test.ts` (6 — lists unmapped accounts and disables Import while any remain;
submits a mapping through the real alias wire and watches it drop off the list; refuses to call the
network with nothing typed; shows the imported count on a clean commit; offers and performs a
per-file re-sort; shows the 400 refusal's exact account list) and
`NewTransactionsPanel.qbJournal.svelte.test.ts` (2 — the format branch itself, both directions).
57 new tests total; the full suite (`vitest run`, both projects) is 1951 passed / 26 skipped, up
from 1894 before this phase, with nothing pre-existing broken.

**Verification commands, all clean:**
`node node_modules/vitest/vitest.mjs run` (both projects — 1951 passed, 26 skipped, 0 failed);
`node node_modules/.bin/svelte-kit sync && node node_modules/.bin/tsc --noEmit` (0 errors);
`node node_modules/.bin/svelte-check --tsconfig ./tsconfig.json` (0 errors, 0 warnings);
`node node_modules/.bin/prettier --check .` (clean); `node node_modules/.bin/eslint .` (clean).

## Phase D — CLI (deprioritized; do after A-C land and are verified)

Per direct instruction: the **same** `ledgeline import` subcommand, not a new one — for this path
there is no `-o`/rules file, since there's no intermediate CSV. Sketch:
`ledgeline import -i Journal.xlsx -j main.journal [--sort]`.

**Detection, resolved now that it can be:** content sniffing alone, no explicit flag —
`ledgeline_core::qb_journal::detect(bytes)`, the exact same check `stage_upload` runs for the GUI.
Phase A proved it accurate over the whole fixture corpus (zero false positives, including a
deliberate near-miss export) and Phase B/C's own live verification confirmed it in practice; an
explicit `--quickbooks-journal` flag would be a second way to say the same thing a byte-sniff
already says reliably, and CLI/GUI detection diverging is exactly what this branch's "GUI and CLI
cannot diverge" rule (see `run_cli_import`'s own module docs) exists to prevent.

**`CliImport`'s clap struct (`import_api.rs`) needs `-o`/`-r` to become optional, not stay
mandatory**, since today both are plain `PathBuf` fields clap requires unconditionally
(`import_api.rs:4680-4737`). They become `Option<PathBuf>`, validated at runtime (not via clap's
static `required`, since the branch depends on the file's *content*, which clap cannot see before
parsing) inside `cli_import`, right after `args.input` is read into `bytes`:
- `qb_journal::detect(&bytes)` true → `-o`/`-r` must be **absent**; if either was given, refuse by
  name ("`--output`/`--rules` are not used for a QuickBooks Journal export — there is no CSV or
  rules file in this path"), rather than silently ignoring a flag the user explicitly typed. Also
  refuse `--balance`/`--balance-account`/`--write-assertion` the same way: a QuickBooks Journal
  import has no single statement-closing-balance to reconcile against, since it can write into
  several accounts across several files in one run.
- `qb_journal::detect(&bytes)` false → `-o`/`-r` **required**, exactly today's behavior, just
  re-checked at runtime instead of by clap (same error message clap would have given, so a script
  written against today's CLI sees no behavior change).
- `-j`/`--sort`/`--dry-run`/`--no-git` apply to both branches unchanged.

**No second write path — extract, don't duplicate.** `qb_journal_api::run_commit` (and
`preview_of`) are shaped as HTTP handlers reading from `QbStageArea` by `stageId`. The CLI has no
stage (it reads the file directly), so factor the part of `run_commit` from "unmapped accounts
block the write" onward into a function taking `&QbJournal` directly rather than a `stageId` —
mirroring exactly how `run_cli_import` already reuses `run_dry_run`/`run_commit`/`run_sort`'s own
functions for the CSV path (see that function's "Why this reuses the HTTP routes' own functions"
doc comment) rather than re-deriving the sequence. The HTTP `commit` handler becomes a thin
`resolve_stage` + call to that extracted function; behavior must be provably unchanged (its
existing `qb_journal_endpoints.rs` tests passing unmodified is the proof, exactly as Phase B's own
amendments used `reimport.rs`'s untouched tests as proof there).

Unmapped accounts on the CLI path have no one to prompt, so the run refuses and lists them — same
"ask, don't guess" policy, a non-interactive shape of it, using the same refusal message text
`run_commit`'s HTTP handler already produces.

### Contract amendments made during implementation (Phase D)

Per convention #9 in `plans/00-overview.md`, following Phases A, B and C's own amendments sections
above.

**`CliImport`'s `-o`/`-r` became `Option<PathBuf>` exactly as this section already specified**
(`crates/ledgeline-server/src/import_api.rs`, `struct CliImport`, now around line 4680) — no
surprise there. What the plan's sketch did not resolve, because it could not without the real
`clap` behavior in hand, is **what the runtime refusal text should be**. Characterized first, per
this section's own instruction ("write a characterization test proving this first"): before
touching anything, `cargo build -p ledgeline` then running `ledgeline import -i statement.csv -r
rules -j main.journal` (omitting `--output`) against `HEAD` on this branch printed `clap`'s own
message verbatim —

```
error: the following required arguments were not provided:
  --output <OUTPUT>

Usage: ledgeline import --input <INPUT> --output <OUTPUT> --rules <RULES> --journal <JOURNAL>

For more information, try '--help'.
```

— exit code 2. That exact text turns out **not preservable**, for a reason worth stating plainly:
once `--output`/`--rules` are `Option`, `clap` itself no longer considers them required and could
not regenerate that `Usage:` line even if asked to — the whole point of the change is that
required-ness now depends on `--input`'s content, which `clap` cannot see while parsing arguments.
Reproducing the string by hand would mean this library crate depending on `main.rs`'s own
`Cli`/`Command` derive (to get a synopsis that stays in sync), which it must not: `main.rs` already
depends on `ledgeline-server`'s library, not the other way around. So the runtime check
(`cli_import_csv`, `import_api.rs`) refuses in this crate's own established one-sentence
`AppError::BadRequest` style — the same style every other `cli_*` runtime refusal in this file
already uses (`cli_journal_id`'s "is not part of this journal", `cli_csv_path`'s "is not inside
this journal's own directory", etc.) — naming the missing flag(s) by long name and, when both are
missing, both in one sentence (`missing_sentence`, mirroring `clap`'s own "list everything that's
missing, not just the first" property). Exit code becomes 1 (`ExitCode::FAILURE`, via
`AppError::Import`) rather than `clap`'s 2. This is a real, deliberate difference from a strict
byte-for-byte reading of "same error message clap would have given" — but no test in
`import_cli.rs`, before or after this phase, ever asserted a *specific* exit code, only
`!status.success()`, so no existing script contract this codebase actually tests for is broken.
Three new hermetic tests in `import_cli.rs`
(`a_missing_output_flag_is_refused_before_anything_runs`,
`a_missing_rules_flag_is_refused_before_anything_runs`,
`omitting_both_output_and_rules_names_both_in_one_refusal`) are the regression guard going forward,
proving: refused before anything runs, on stderr, naming the missing flag(s), tree byte-identical
afterward. All 15 of `import_cli.rs`'s pre-existing tests pass unmodified alongside them (18 total)
— the CSV path's own behavior (including its exit-non-zero-on-refusal property) is unchanged.

**No second write path: `qb_journal_api::run_commit`'s body from "unmapped accounts block the
write" onward is now `qb_journal_api::commit_journal(state, parsed: &QbJournal, git: GitPolicy) ->
Result<WireQbCommit, AppError>`**, `pub(crate)`, exactly as this section specified — `run_commit`
is now `resolve_stage` + one call to it. One deviation the sketch did not anticipate: **`git` had
to become a parameter**. The original `run_commit` hardcoded `GitPolicy::FromPrefs` twice inside
the body (the pre-write `blocked_by_git` check and the post-write `commit_targets` call), which was
correct when the HTTP route was the only caller — it has no `--no-git`-equivalent request field and
never has, git behavior there is governed purely by Preferences. But `CliImport::no_git` has no
representation on the HTTP wire at all, so the only way for it to reach `commit_journal` is as an
argument; threading `GitPolicy` through (the exact pattern `run_dry_run`/`run_commit`/`run_sort`
already use in `import_api.rs`, for the identical reason) was the natural fix. `run_commit` now
passes `GitPolicy::FromPrefs` explicitly, which is bit-for-bit what the two `GitPolicy::FromPrefs
.enabled(&prefs)` call sites already did — `GitPolicy::enabled`'s own definition
(`self == Self::FromPrefs && autocommit_enabled(prefs)`) makes this substitution provably behavior-
preserving — and `qb_journal_endpoints.rs`'s existing 14 tests pass **unmodified**, which is the
"prove it" this section itself asked for. `WireQbCommit`/`WireQbIdMatches`/`WireQbOrdering`/
`WireQbFileOrdering` gained `pub(crate)` on the handful of fields the CLI path reads (`imported`,
`id_matches.{new,unchanged,conflicting_total}`, `ordering.{in_order,files}`,
`WireQbFileOrdering.{journal_id,in_order}`) — the same "widen a wire struct's field visibility for
cross-module reuse, change no behavior" move Phase B's own amendments already made in
`import_api.rs`/`edit_api.rs`/`stage.rs`.

**The dry-run decision (the plan's point 6): a second read-only function, not a `write: bool`
parameter on `commit_journal`.** `qb_journal_api::classify_report(state, &parsed) ->
Result<QbClassifyReport, AppError>` shares `commit_journal`'s own refusal-or-classify prefix
(factored once more into a private `classify_parsed` helper both call, so the unmapped-accounts
refusal text — `unmapped_refusal`, one function — is written exactly once and reached from three
places: the HTTP `commit` route, `classify_report`, and `commit_journal`'s own pre-write check).
`cli_import_qb_journal` (`import_api.rs`) always calls `classify_report` first, mirroring
`cli_import_csv`'s own "the dry run always happens" rule (see `run_cli_import`'s doc comment) for
an analogous reason: it is the only way to know, before writing, whether any account is unmapped or
any id would conflict. Unlike the CSV path, this costs **nothing** worth avoiding — an alias scan
plus an in-memory classify over at most a few dozen fixtures'-worth of transactions, no subprocess,
no I/O beyond one `state.snapshot()` — so a committing CLI run classifies twice (once via
`classify_report` for the report, once more inside `commit_journal` for the write) exactly as a
committing CSV run pays for one real `hledger import --dry-run` subprocess it does not strictly
need either. The alternative (threading `write: bool` through `commit_journal`) was rejected on
purpose: it would leave that function's own name and doc comment — "the part of `run_commit` from
'unmapped accounts block the write' onward," which this section itself specifies as always
writing — no longer an accurate description, and would open the door to a future caller passing
`write: false` into the HTTP `commit` handler by mistake. Both functions are reached from
`cli_import_qb_journal` only; the HTTP surface still calls only `commit_journal` (via `run_commit`),
unchanged.

**Output shape (the plan's point 7): a new `CliRunReport` enum, not one struct wide enough for
both formats.** `run_cli_import` now returns `Result<CliRunReport, String>` where `CliRunReport` is
`Csv(CliImportReport) | QbJournal(CliQbReport)` — `CliImportReport`/`CliImportWritten` are
untouched. A shared subset was considered and rejected: `CliImportReport::balance`/`status` name a
single statement-closing balance and hledger's own dry-run status line, neither of which exists on
this path, and `CliQbReport::new`/`unchanged`/`conflicting` (the id-match counts) have no CSV-path
analogue at all (the CSV path's own preview never separates these out — `run_dry_run`'s proposal is
pre-dedup). Forcing one struct to cover both would leave fields that can never be populated on one
variant or the other, which is exactly the "the type can express a state that cannot happen" gap
this codebase's conventions warn against (the same reasoning Phase C's own amendments used for why
`QbIdMatches` does not reuse the CSV path's `IdMatches` on the SPA side — this is the identical call
made again on the Rust side). `CliQbReport`/`CliQbWritten` (new, `import_api.rs`, re-exported from
`lib.rs`) follow `CliImportReport`/`CliImportWritten`'s own convention exactly: one printable fact
per field, no ANSI, `main.rs` decides rendering. `main.rs::run_import` now matches on `CliRunReport`
and dispatches to `print_csv_report`/`print_qb_report` — `print_csv_report` is `run_import`'s own
prior body, unmoved in substance, just extracted into its own function; `print_qb_report` is new
and mirrors its shape (command line to stderr; counts, what was written, an out-of-order note, a
re-sort report, to stdout).

**`-j` is resolved via `cli_journal_id` on both branches, as this section said it must be, but its
resolved handle is used only for the echoed command line on the QuickBooks Journal branch** — there
is still no `journalId` anywhere `commit_journal`/`edit_api::add_transactions` touch (Phase B's own
"No `journalId`, anywhere on this surface" amendment stands unchanged; `InsertPosition::DateOrdered`
still decides every transaction's destination file from the journal's own chronology). Resolving it
anyway buys two things: a bogus `-j` refuses with the same "is not part of this journal, it
includes: …" message on both branches rather than a QuickBooks-Journal-specific one, and the
re-runnable command line the QuickBooks branch echoes (`qb_cli_invocation`, new — the
QuickBooks-Journal analogue of `cli_invocation`/`CliRun`, not those reused, since this format has no
rules file, CSV path, or balance to hold in a `WireDryRunRequest`-shaped `plan` field) names `-j`
the same portable, relative way the CSV branch's does.

**`--sort` sums across every touched file, per the plan's own note that a multi-year import can
touch more than one.** `cli_import_qb_journal` loops `commit.ordering.files`, calling the existing
`run_sort` (unchanged, `import_api.rs`) once per file reported not-in-order, and
`CliQbWritten.sorted` is the total moved across all of them — `CliQbWritten.in_order` is `true` once
every touched file either started in order or was just fixed, mirroring the CSV path's own
`commit.ordering.in_order || sorted.is_some()` idiom exactly.

**No new `LEDGELINE_HLEDGER_QBJOURNAL_CHECK`-style opt-in test was added**, per this section's own
"use your judgment, but don't add one just to have one." The CLI path calls `commit_journal` — the
identical function, unchanged — that `qb_journal_hledger_check.rs`'s two existing opt-in tests
(`hledger_accepts_and_balances_the_journal_qb_journal_commit_writes`,
`hledger_accepts_the_full_report_fixture`) already prove `hledger check`/`hledger print` accept.
There is no new hledger-facing surface a CLI-specific opt-in test would exercise that those two
do not already cover; re-run as part of `just hledger-checks` for this phase, both still pass.

**New files and test counts.** `crates/ledgeline-server/tests/import_cli_qb_journal.rs` (new, 8
hermetic tests: detected-and-committed happy path plus the command-line echo; `-o`/`-r` refused by
name; `--balance`/`--balance-account`/`--write-assertion` refused by name; unmapped accounts refuse
and list them; `--dry-run` reports and writes nothing; a second commit of the same export imports
nothing new; the out-of-order note without `--sort`; `--sort` restoring date order). `import_cli.rs`
gained 3 (the `-o`/`-r` runtime-refusal characterization above); its pre-existing 15 pass unmodified
(18 total). `qb_journal_endpoints.rs`'s existing 14 tests pass unmodified (the "prove it" for the
`commit_journal`/`GitPolicy` extraction). No `ledgeline-core` changes were needed for this phase —
everything new lives in `ledgeline-server` (`import_api.rs`, `qb_journal_api.rs`, `main.rs`,
`lib.rs`'s re-exports).

**Verification commands, all clean:** `cargo fmt --check`; `cargo clippy --workspace --all-targets
-- -D warnings`; `cargo test --workspace` (hermetic, every pre-existing suite still green); `just
hledger-checks` (every opt-in suite, including the two QuickBooks-Journal ones, still green against
real `hledger 1.52`).

## Real-world scale finding: the write pipeline was O(N²) (2026-09-02)

A user's real QuickBooks Online export — 10,515 transactions, 23,890 postings, a scale none of
Phases A-D's fixtures approach (`report.xlsx`, the corpus-level fixture, is 45 groups) — timed out
client-side at 30 seconds and never completed. Root cause, traced to `JournalEditor::add_transaction`
(`crates/ledgeline-core/src/edit.rs`): every call — singular, one transaction — calls
`self.validate_with(...)`, which materializes every loaded file's rope to a full `String` and
re-parses the ENTIRE journal from scratch via `parse_journal_with_overrides`. `edit_api::add_transactions`
(the server function `commit_journal` calls) looped this once per new transaction, with the journal
growing by one row each iteration. For a batch of N added to a journal of size M that is `O(N × (M +
N))` — genuinely too slow to ever finish for a several-thousand-row batch, not merely slow. Nothing in
Phases A-D caught this because every existing test — hermetic and the two opt-in
`LEDGELINE_HLEDGER_QBJOURNAL_CHECK` suites alike — adds at most a handful of transactions per batch;
the quadratic term is invisible until N reaches the thousands.

**The fix: a genuine bulk-insert path in `ledgeline-core`, not a client-side timeout increase.**
Raising the HTTP timeout would still hold the single global editor mutex — blocking every other
request — for however many minutes the O(N²) loop actually took; the number of expensive whole-journal
reparses had to drop from O(N) to O(1) per batch.

**`JournalEditor::add_transactions(&mut self, transactions: &[Transaction], position: InsertPosition)
-> Result<(), EditError>`** (plural — new, `crates/ledgeline-core/src/edit.rs`) is a NEW public method
alongside the existing singular `add_transaction`, which is completely UNCHANGED — its own
`check_single_change` "at most one transaction may differ" invariant still governs every other edit in
the app (the manual add-transaction UI, status changes, description edits, deletes) with the exact
strictness it always had. The bulk method:

1. Structurally validates every input transaction (the same multiple-commodity-amount and
   balance checks `add_transaction` already does) BEFORE touching any rope, so a bad transaction
   anywhere in a several-thousand-row batch is caught before any edit is attempted.
2. Computes placement for every transaction — via the SAME private `placement_for` (and, through
   it, `insert_after`/`insert_before`/`append_to_main`), reused verbatim — against the CURRENT,
   UNMODIFIED journal/files. Every call sees the identical starting state, so N calls are as cheap
   as one; none of them needs a reparse.
3. Groups transactions that resolved to the identical (file, offset) — i.e. the identical
   anchor/gap — into ONE combined, date-ordered insertion per gap (`build_bulk_groups`).
4. Splices each file's combined insertions in DESCENDING offset order against that file's
   ORIGINAL rope (`apply_bulk_groups`), so an offset computed in step 2 is never invalidated by an
   insertion still pending at a lower position in the same file; this also computes every new
   transaction's FINAL header position analytically (accounting for every other, lower-offset
   insertion that will shift it), rather than re-scanning the mutated rope.
5. Runs EXACTLY ONE whole-journal reparse-and-validate for the entire batch (`validate_bulk_with`,
   a new function; `check_single_change` is untouched and unused here), checking the new count is
   `original + N`, that every one of the N new transactions (located by file + header line, a
   generalization of the existing `locate_in_file`) balances and round-trips, and that every OTHER
   transaction is still byte-identical to its pre-edit source — proven by removing the N known-new
   entries from the reparsed sequence at their known indices and diffing what remains against the
   original sequence element-for-element (see that function's own docs for why this is cleaner than
   generalizing `check_single_change`'s prefix/suffix scan, whose "at most one" assumption does not
   extend gracefully to N insertions scattered across possibly-many gaps). The existing
   `check_no_new_imbalance` is reused with NO changes — verified it generalizes as-is, since it
   compares SETS of imbalance keys and never assumed exactly one transaction changed.
6. On success, adopts every touched file's candidate rope (dirty flag set) and the reparsed
   journal; `dirty_files()` correctly reflects every file the batch touched, not just one. On ANY
   validation failure `self` is left completely unchanged — nothing is mutated until every check has
   passed.

### Contract amendments made during implementation

Per convention #9 in `plans/00-overview.md`, following Phases A-D's own amendments sections above.

**Attributing a bulk failure to one transaction needed a new `EditError` variant, not a bare
`(usize, EditError)` tuple threaded through the core API.** The brief's own sketch left this to
judgment. `EditError::BulkTransaction { index: usize, source: Box<EditError> }` (new variant) lets
the core method's return type stay exactly `Result<(), EditError>` — no second error type, no change
to `EditError`'s existing shape for every other caller — while still letting `edit_api::apply_additions`
recover which of the N inputs failed, when that's determinable. `edit_api.rs`'s `labels: &[String]`
mechanism (added recently for exactly this "which of several thousand failed" naming problem) is
preserved and works unchanged against the new bulk path: `apply_additions` now calls the bulk core
method ONCE and destructures its error — `EditError::BulkTransaction { index, source }` becomes
`(Some(index), *source)` (the existing "transaction {label} (N of M)" framing); any other `EditError`
(a whole-batch failure — the combined text failed to reparse at all, or a stray change was found
outside the batch, neither attributable to one row) becomes `(None, error)`, which
`edit_api::add_transactions` frames as "batch of N transactions" instead of naming one. Both
pre-existing labeling unit tests (`add_transactions_names_which_one_failed_and_writes_nothing`,
`add_transactions_falls_back_to_a_position_when_no_label_was_given`) pass **unmodified** — the
structural pre-check (step 1 above) that both tests exercise is cleanly attributable, so nothing about
their expected wording changed. `crates/ledgeline-server/src/error.rs`'s `From<EditError> for AppError`
match gained a `BulkTransaction` arm for exhaustiveness (delegates to the wrapped `source`'s own HTTP
classification, keeping the outer message) — reached only if a `BulkTransaction` ever escapes
`apply_additions` unwrapped, which nothing in the current wiring does; added for correctness under a
future caller, not because today's code path needs it.

**Chaining N single-transaction insertions into one combined splice, byte-for-byte identical to N
sequential `add_transaction` calls, required reverse-engineering (not inventing) the existing
single-insert shapes.** Reading `insert_after`/`insert_before`/`append_to_main`: for ONE transaction,
either (a) `insertion.body` is the formatted transaction unchanged and `insertion.prefix` supplies the
separating blank (`insert_after` not at EOF; `append_to_main`/EOF), or (b) `insertion.body` is the
formatted transaction PLUS one extra trailing terminator and `insertion.prefix` is empty
(`insert_before` — the extra terminator IS the separating blank before the untouched successor).
Verified by hand (and now by `bulk_add_matches_sequential_add_in_the_same_gap_regardless_of_input_order`
and `bulk_add_handles_insert_before_and_append_style_groups_in_the_same_call`, the latter exercising
BOTH shapes in one call) that chaining a date-sorted group of transactions this way — every member but
the last gets its own body plus one glue terminator, the last member reuses its own single-insert
`insertion.body` exactly as computed — reproduces N sequential `add_transaction` calls exactly,
regardless of the batch's input order (`InsertPosition::DateOrdered` always finds the chronologically
correct spot per call, so sequential adds converge on the same sorted result regardless of call order
too — the equivalence property this whole design leans on).

**A `#[cfg(test)]` reparse-call counter, not only wall-clock timing, for the "not quadratic" proof.**
`BULK_REPARSE_COUNT` (a `static AtomicUsize`, `edit.rs`) is incremented exactly where
`validate_bulk_with` calls `parse_journal_with_overrides`, and two in-crate unit tests
(`add_transactions_reparses_the_whole_journal_exactly_once_regardless_of_batch_size`,
`add_transactions_bad_batch_member_does_not_reparse_at_all`) assert it is exactly 1 for a
500-transaction batch and exactly 0 when the batch is rejected before any rope is touched — a
deterministic, machine-speed-independent proof the reparse count is genuinely O(1) per batch, stronger
than the wall-clock evidence below. It lives in `edit.rs`'s own `#[cfg(test)]` unit tests (not
`tests/edit.rs`) because a `#[cfg(test)]` item of a library crate is not visible to that crate's own
integration tests — they link the NORMAL (non-test-cfg) build of the library.

**No new opt-in `LEDGELINE_HLEDGER_*_CHECK` suite, and no new multi-thousand-row synthetic fixture,
for a real-hledger check at true QuickBooks-import scale.** The existing opt-in
`hledger_accepts_the_full_report_fixture` (`qb_journal_hledger_check.rs`) already runs the FULL
`commit_journal` → `edit_api::add_transactions` → (now) the bulk core path against `report.xlsx` (45
groups) and real `hledger check`/`hledger print`, so the wiring is proven against the real binary at
that scale without a new suite. A genuinely multi-thousand-transaction fixture large enough to
reproduce this bug's actual scale would be slow to generate and slow to run routinely, and the
hermetic core-level tests above (byte-identical equivalence to sequential adds at small scale, plus
the deterministic O(1)-reparse-count proof at 500 transactions, plus a 1000-transaction wall-clock
test — `bulk_add_of_many_transactions_into_an_existing_journal_completes_quickly`, under a
deliberately generous 20-second margin (needed in practice: under full `cargo test --workspace`
parallel contention this took ~4s, vs well under 1s run in isolation — the margin is sized to still
catch a real regression to O(N) reparses, which per this section's own root cause would mean the
operation does not finish in any practical time at all for a batch this size, not "a few seconds
slower")) already prove both correctness and the absence of the specific quadratic
behavior that broke the real import. Judged not worth adding; revisit if a future real-world import at
this scale surfaces something these tests do not cover.

**New tests, all hermetic unless noted.** `edit.rs` (core, `#[cfg(test)] mod tests`): 2 new (the
reparse-call-counter proofs above). `tests/edit.rs` (core, integration): 8 new — equivalence to
sequential add in the same gap regardless of input order; both chaining shapes in one call, also
cross-checked against sequential add; landing across different `include`d year files in one call (with
`dirty_files()` proven to list exactly the two touched files); an unbalanced batch member anywhere in
the batch rejects the WHOLE batch and writes nothing; a round-trip-mismatch member (the EUR
decimal-mark guard, mirroring the existing singular-path test) likewise; an uncheckable (invalid-date)
member is a BATCH-level failure, not a `BulkTransaction`, proving the attributable/non-attributable
split; an empty batch is a no-op; the 1000-transaction wall-clock proof. `edit_api.rs`'s two
pre-existing labeling unit tests pass unmodified (above). Every pre-existing test in
`crates/ledgeline-core/tests/edit.rs`, `crates/ledgeline-server/tests/qb_journal_endpoints.rs`, and
`crates/ledgeline-server/tests/import_cli_qb_journal.rs` passes unmodified.

**Verification commands, all clean:** `cargo fmt --check`; `cargo clippy --workspace --all-targets --
-D warnings`; `cargo test --workspace` (hermetic, every pre-existing suite still green); `just
hledger-checks` (every opt-in suite, including both QuickBooks-Journal ones, still green against real
`hledger`).

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
