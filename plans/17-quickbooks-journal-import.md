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
