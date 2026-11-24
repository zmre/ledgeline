# Ledgeline Invoicer — Plan

[ARCHITECTURE.md](ARCHITECTURE.md) is the design; this file orders the work.
[MBR-CHANGES.md](MBR-CHANGES.md) is the hand-off for MBR.

## The ask (Patrick, 2026-11-19)

> The primary goal and purpose of the app is to generate PDF invoices to be
> sent to customers or clients. It must store the invoices in plain text and
> also link the invoices into the books. Everything can be manually edited.
> Most of what you can do from the GUI should be doable from the command
> line, too.

The secondary goals:

- shared prefs and theme with Ledgeline;
- MBR for display, with customers as person and organization notes;
- multiple templates;
- recognition through plan 45;
- room for Stripe, QuickBooks import, timedot, estimates and recurring
  invoices;
- mobile later, built the way Ledgeline is.

## Decisions (Patrick, 2026-11-19)

| #   | Decision                                                                                                                                                                                                                                                                              |
|-----|---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| I1  | Ledgeline and the invoicer both become **AGPL-3.0-or-later**. MBR (GPL-3.0+) is then linkable.                                                                                                                                                                                        |
| I2  | **Drafts never touch the books.** Sending books the invoice.                                                                                                                                                                                                                          |
| I3  | Discounts, subtotals and taxes are **inline positional rows** in the one bill table. An adjustment applies to the nearest item or total row above it, skipping other adjustments.                                                                                                     |
| I4  | Invoices name their template with MBR's **`template:` path** (`/invoices/tpl/x.html`). MBR renders styled invoices everywhere.                                                                                                                                                        |
| I5  | Per-row accounting lives in a **books column**: the last column, with an italic header, hidden by CSS. A blank cell means the default account.                                                                                                                                        |
| I6  | Partial up-front billing is a **separate deposit invoice**. The final invoice applies it with a row against the deposit liability. In accrual books, undelivered work books to deferred revenue or deposits.                                                                          |
| I7  | `status: draft\|sent\|void` is intent. `paid:` is a mirror refreshed from the books: `no`, `partial 2026-10-28 $1,500, 2026-11-19 $3,500`, or a date.                                                                                                                                 |
| I8  | A sent invoice may be **edited and re-booked**. Rewriting it is refused on or before `closed-through`; void and reissue (later, a credit note) is offered instead.                                                                                                                    |
| I9  | `po_num` becomes **`reference`** (free text: `PO #105`).                                                                                                                                                                                                                              |
| I10 | The binary is **`ledgeline-invoicer`**.                                                                                                                                                                                                                                               |
| I11 | The default deposit account is **`liabilities:deferred-revenue`**, overridable with `[invoicer] deposit-account`.                                                                                                                                                                     |
| I12 | Final PDFs are **kept** in `pdf-dir` (default `invoices/pdf/`), with earlier exports kept too. Users delete old ones as they please.                                                                                                                                                  |
| I13 | Every English word the invoicer matches (column headers, total rows, the customer tag, contact labels) is a setting in `[invoicer.words]`. File-format keys stay fixed.                                                                                                               |
| I14 | **Performance and correctness are release criteria.** The ARCHITECTURE §5.6 budgets are gated by benches on a 5,000-invoice corpus.                                                                                                                                                   |
| I15 | Customer `contracts:` take wikilinks or file paths (executed PDFs). `billing_portal:` records vendor-portal delivery.                                                                                                                                                                 |
| I16 | MBR ships **one release** once its P0 and P1 changes land. The invoicer uses it from crates.io: a git rev cannot work, because `mbr-core` embeds built JS bundles that only the crates.io package contains. The release gates Phase 3.                                                |
| I17 | Cash books get **Apply payment** in v1 (Phase 2): pick an imported deposit and its income side is re-split into the invoice's row accounts, tagged `invoice:`. The invoicer never creates payment transactions. Both bases are first-class: Patrick runs accrual and cash businesses. |
| I18 | **No Intel Mac build of the invoicer** (Patrick, 2026-11-20). It ships for Apple Silicon macOS and Linux only. Ledgeline keeps its x86_64-darwin build.                                                                                                                               |
| I19 | **The invoicer crates stay `publish = false`** until Patrick decides to share them (2026-11-20). Nothing about the invoicer goes to crates.io before then.                                                                                                                            |
| I20 | **Quality is paramount: CI lints every web directory** (`web/`, `packages/ui`, `invoicer/web`) with prettier and eslint, not only pre-push (2026-11-20).                                                                                                                              |

These were decided in drafting and are open to veto. They are listed in
ARCHITECTURE.md §3:

- a customer is a person or organization note tagged `customer`;
- the `customer:` link plus `invoicee_*` snapshot fields;
- `terms` (net days) plus a written `due_date`;
- the catalog and snippets are markdown notes;
- settings go in `ledgeline.toml [invoicer]` and `ui-prefs.toml [invoicer]`,
  with no new config files;
- cash books book an invoice at payment;
- void is a reversing entry linked with `match:`.

## Configurable accounting flows

> I don't want to predetermine how people do their accounting. There are
> many valid ways. Sane defaults, but allow for the flows people use.
> (Patrick, 2026-11-21)

So every accounting-policy choice has a default plus a setting or an
advanced option: built where it was cheap, planned here otherwise. Settings
are `ledgeline.toml [invoicer]` keys unless the row says otherwise.

| Policy                                             | Setting                         | Default                                                                                                            | Alternatives                                                                  | State                   |
|----------------------------------------------------|---------------------------------|--------------------------------------------------------------------------------------------------------------------|-------------------------------------------------------------------------------|-------------------------|
| Cash books: a deposit's payment                    | `cash-deposits`                 | `income-on-receipt`                                                                                                | `liability-until-final` (held until the final invoice)                        | Built (2b)              |
| Cash books: a payment over an invoice's rows       | `payment-allocation`            | `match-rows` (each account ends at its row, deposits counted)                                                      | `pro-rata`; planned: `oldest-row-first`                                       | Built (2b), one planned |
| Cash books: the revenue account a deposit lands in | —                               | the row's account, else the customer's `revenue_account`, `[invoicer] revenue-account`, the single revenue account | planned: a per-deposit-invoice override (frontmatter or the row's books cell) | Planned                 |
| Cash books: a deferred row's payment               | —                               | into the account it recognizes into                                                                                | planned: `cash-deferred-rows = "defer"` (modified cash: keep the deferral)    | Planned                 |
| Discounts with a blank books cell                  | `discount-account`              | pro rata over the revenue lines                                                                                    | a contra-revenue account                                                      | Built (1a)              |
| Sending into a closed period                       | —                               | refused                                                                                                            | planned: book it dated the first open day                                     | Planned                 |
| Editing a sent invoice                             | —                               | replace the transaction in place (§4.5)                                                                            | planned: reverse and re-book, for an audit trail of every change              | Planned                 |
| The void's date                                    | `void --date`                   | today                                                                                                              | any date from the invoice's on, outside the closed period                     | Built (2b)              |
| Which deposit an application uses                  | —                               | the one its row names, then the oldest                                                                             | planned: oldest only, or ask                                                  | Planned                 |
| Voiding a final invoice that applied a deposit     | —                               | the deposit is open again for a reissue                                                                            | planned: keep it applied (forfeited)                                          | Planned                 |
| Who a person at a company bills                    | `ledger_customer` on the person | the company's customer note                                                                                        | the person's own `ledger_customer`                                            | Built (2b)              |
| When an invoice books at all                       | `[books] basis`                 | plan 45's gate (accrual when it has receivable accounts)                                                           | `cash`, `accrual`                                                             | Built (plan 45)         |
| What the `paid:` mirror counts as paid (accrual)   | —                               | every payment and credit linked to the receivable (`match:`); paid on the date of the one that completes it        | planned: a credit note or write-off mirrored as its own state                 | Planned                 |
| Recognition due in a closed period                 | `recognize --include-closed`    | left unposted (listed), as Ledgeline leaves it                                                                     | one catch-up adjustment per schedule, dated today                             | Built (2c)              |
| The DSO window                                     | `receivables --dso-days`        | 90 days                                                                                                            | any number of days                                                            | Built (2c)              |
| Aging buckets                                      | —                               | Ledgeline's: Current, 1–30, 31–60, 61–90, 91+ days past due                                                        | planned: custom bucket edges, in `ledgeline-core` so both apps share them     | Planned                 |
| A draft's amounts after an outside edit            | `recalc-on-save`                | `drafts`: the app recalculates a draft once the edit settles (a sent invoice never: its amount is booked)          | `never` (the page shows the mismatch and a Recalculate button)                | Built (feedback F10)    |

Technical choices that are not policy stay fixed: a zero-total accrual
invoice has no receivable posting (Ledgeline's receivables ignore a zero
line), tag values drop commas, journal-then-markdown write order, a books
write retried once when the files changed under it, and where Apply
payment's undo records live (2c).

## What is true today

These facts come from surveys on 2026-11-19. They are summarized here so the
phases can cite them.

- **Plan 45 already built the booking API for this app** (D-lib).
  - `ledgeline_core::accrual::ops::Books::{open, load}`
  - `Loaded::{write_document, save, preview, post_due, link_settlement, generation}`
  - `accrual::invoice::{DocumentSpec, DocumentLine, Deferral, build_document, money}`
  - `settle::{candidates, propose}`
  - `RecognitionIndex`
  - `crates/ledgeline-core/tests/accrual_library.rs` already runs an
    invoicing app's flow through public paths only.
- **Gaps in that API** (ARCHITECTURE §8, L1–L4):
  - no per-line memo and no extra header tags;
  - positive lines only;
  - `write_document` has no generation check;
  - no find-by-invoice;
  - no receivables, aging or N:1 settlement;
  - no stable transaction id.
- **The Ledgeline shell is single-app.**
  - Rust side: `ui_prefs`, `app_dirs`, `recents`, `security`, `spa` and `gui`
    are private to `ledgeline-server`.
  - Web side: themes, components and the API client live inside the `web/`
    SvelteKit app.
  - Build side: `release.sh`, the flake (`spaNodeModules` FOD, `macApp`),
    `ci.yml` and `release.yml` assume one app.
- **MBR state.**
  - PDF export (#386) and per-note templates (#387) are merged on main but
    not released; crates.io has 0.7.3.
  - `pdf_export::ExportSession::start` is already public and takes the
    host's tao event loop; wry 0.57 and tao 0.37 match Ledgeline's.
  - Frontmatter wikilinks aren't resolved.
  - `site.json` assembly is private and strips contact fields.
  - The crates are GPL with "no API stability guarantees".
  - See MBR-CHANGES.md.

## Phase map

| Phase | Name                                   | Ships                                                     | Depends on                     |
|-------|----------------------------------------|-----------------------------------------------------------|--------------------------------|
| 0     | Foundations                            | AGPL, shared shell/prefs/ui extraction, empty crates, fixtures, CI wiring | —                              |
| 1     | Document engine + read-only CLI         | `invoicer-core` docs model, table engine, discovery, numbering, index, benches; `new/list/show/set/recalc/check/next-number/config` | 0 (no MBR dependency) |
| 2     | Books bridge                           | Ledgeline L1–L4; `send/void/rebook/reconcile/receivables/recognize`; deposits; cash vs accrual | 1                              |
| 3     | App shell, rendering, PDF, branding    | The GUI app boots, MBR mounted, preview, starter templates, `pdf`, icon | 2; MBR M3, M4, M5 (crates.io)  |
| 4     | Editors                                | Invoice, customer, catalog, snippet and template editors   | 3; MBR M1, M2 (M6 optional)    |
| 5     | Dashboard and lists                    | Receivables dashboard, invoice and customer lists          | 2, 4                           |
| 6     | Packaging, release, docs               | Signed DMGs in lock-step, docs section, README             | 3 (can start in parallel with 4) |
| 7     | Later                                  | Each item is its own plan                                  | —                              |

After Phase 2, every booking feature works from the CLI. That makes the
invoicer useful for scripting before any GUI exists, and it pins the core
behaviour under tests before the UI builds on it.

---

## Phase 0 — Foundations (PRs 0a–0d)

Ledgeline-side refactors that change nothing a Ledgeline user can see.

**0a. License (L8).** Change these to AGPL-3.0-or-later:

- `LICENSE`;
- `[workspace.package] license = "AGPL-3.0-or-later"`;
- `web/package.json`;
- the README badge and License section;
- `docs/` mentions.

Then check `cargo deny` / audit license allowlists if any. One commit, nothing
else in it.

**0b. Rust shell extraction (L5).**
- **New crate `crates/ledgeline-shell`.** It holds `security.rs` (token, Host
  guard, CSP), the `spa.rs` embed helper, the `gui.rs` boot sequence, menus
  scaffolding, `app_dirs.rs` and `recents.rs`. Embedding is generic over a
  `RustEmbed` type and an app name, and the init-script globals are
  parameterized (`__LEDGELINE_EMBEDDED__` stays for Ledgeline).
- **UI-prefs data model to `ledgeline-core`** (`ui_prefs` SPECS, validation,
  merge order, save routing, theme pin, `edit_ledger_text`). Each app
  registers its own SPECS table; the theme and scale keys are common.
- **`ledgeline-server` re-exports and uses them.** No route or behaviour
  changes.
- **Gate:** the full `just pre-push`, including e2e, run by the lead.

**0c. Web extraction (L6).**
- Root `package.json` with bun workspaces `["web", "invoicer/web", "packages/*"]`.
- Move to `packages/ui`:
  - `web/src/themes/*.css`;
  - the theme-related part of `app.css` (container-query variants, scale);
  - `lib/theme/*`, `lib/ui/*`;
  - the generic components (`DateInput`, `InfoTip`, `AsyncSection`,
    `FoldedControls`, `HScroll`, `ErrorToast`, `ChartLegend`);
  - the base of `lib/api/client.ts`;
  - `lib/native/bridge`.
- `web/` imports them through `#ui/*`.
- The catalog/swatch test moves with the themes.
- Re-pin the `spaNodeModules` hashes (`just repin-spa-hashes`); update
  `spa-hash-check`.
- **Gate:** `just check test lint e2e`, and the docs screenshots are
  unchanged.

**0d. Scaffolding.**
- Add `invoicer/crates/{invoicer-core,ledgeline-invoicer}` to the workspace.
  For now they hold a lib with one test and a bin printing its version.
- Add `invoicer/web` (a SvelteKit SPA, `paths.base = "/app"`, importing
  `packages/ui`, showing an empty themed shell).
- `scripts/release.sh` learns the new version literals and the second
  `package.json` (L7, part 1).
- CI: `.#tests`, `.#clippy` and `.#fmt` already cover workspace crates.
  Confirm this, and add `invoicer/web` to the `spa` job.
- **Fixtures.**
  - `invoicer/fixtures/accrual/` is a copy of the accrual books plus
    `customers/`, `invoices/` (draft, sent, partially paid, paid, void,
    deposit and final), `invoices/tpl/`, `catalog.md`, snippets and
    `ledgeline.toml [invoicer]`.
  - `invoicer/fixtures/cash/` is the cash-basis variant.
  - Check `git status --ignored`: the `*-journal` and `ledgeline.toml`
    gitignore traps can hide new fixture files.

## Phase 1 — Document engine and read-only CLI (PRs 1a–1c)

All of this is in `invoicer-core`. Nothing here writes the journal.

**1a. Table engine** (`invoicer_core::table`):
- Parse GFM pipe tables with spans, so edits are surgical.
- Recognize columns by header alias, and total rows by their words, using
  the `[invoicer.words]` lists with their defaults (ARCHITECTURE §3.8).
- Classify rows: spacer, total, adjustment, fixed adjustment, item.
- Compute amounts with `Dec` and the journal's commodity style. Round per row,
  half away from zero.
- Parse the books-column grammar into `account | Deferral`.
- Write the table back aligned: column widths padded and alignment markers
  kept. Rows the user didn't touch keep their cell text.
- **Tests:**
  - goldens for every row kind and every column order;
  - the §3.2 example, plus the tax-above vs tax-below variants;
  - hand-written oddities: missing alignment row, extra spaces, `$-800` vs
    `-$800`, `&nbsp;`, escaped pipes;
  - errors that name the row.
- **Proptests:**
  - `parse(write(t)) == t`;
  - recomputing twice is a no-op;
  - written totals always equal the sum of the rows above;
  - adjustments never apply to adjustments.
- **Benches** (criterion, as in `ledgeline-core/benches`): recomputing a
  100-row table and re-writing one cell. Budget: under 1 ms.

**1b. Documents and discovery** (`invoicer_core::{doc, frontmatter, notes, customers, catalog, snippets, templates, numbering, index, draft, settings, problem}`):
- Frontmatter read: lossless, to `serde_json::Value`, with `yaml-rust2`, the
  same crate MBR uses. 1b has no MBR dependency (decided 2026-11-20; see
  I16).
- Frontmatter edit-by-key, preserving order and comments (ARCHITECTURE §5.5),
  verified by re-parsing.
- The `Invoice` model: frontmatter fields, prose before, bill table, prose
  after, supporting tables.
- Validation produces `Vec<Problem>` with a file, a line and a fix-it where
  possible.
- Discovery through `notes`: its own walk over `Vault`, following MBR's
  ignore, type, tag and wikilink rules, behind an API shaped like MBR's `Site`
  façade (M2) so that it can become a thin wrapper over it.
  - Invoices by `type`.
  - Customers by type plus the `customer` tag, with billing-contact
    selection.
  - The catalog, snippets, and templates (by `invoice_num` reference plus the
    `{# invoicer: … #}` header).
- Numbering: maximum over files (and over the books, in Phase 2) plus 1, with
  trailing-digit increment.
- Creating a draft: snapshot the customer and company, terms → `due_date`,
  the filename pattern, an atomic write with a content-hash check through
  `Vault`.
- Customers: `contracts:` entries (wikilinks or file paths, checked for
  existence) and `billing_portal:` (with `requires_reference` enforced by
  `check`).
- **The document index** (ARCHITECTURE §5.6): in memory, keyed by path with
  `(mtime, size, hash)`, frontmatter plus bill-table summary, parallel open,
  per-file invalidation. `check --full` rebuilds it and diffs.
- **The corpus generator:** 5,000 invoices, 500 customers and 10 years of
  journal, deterministic, extending `ledgeline-core/benches/corpus.rs`.
  Benches for open, list and `check`, with a stored baseline. A >20%
  regression fails the gate (`just invoicer-bench-gate`; ARCHITECTURE §5.6).
- **Measure Ledgeline's journal reload** on the corpus. If it threatens the
  §5.6 budgets, file the Ledgeline change now rather than in Phase 3.

**1c. CLI** (`ledgeline-invoicer`, clap):
- Commands: `new`, `list`, `show`, `set`, `recalc`, `check` (documents only),
  `next-number`, `config`, `customers list|show`.
- `--json` everywhere.
- Opening the books: positional journal or `$LEDGELINE_FIXTURE`, then recents,
  through `ledgeline-shell`.
- **Tests:** CLI integration tests against `invoicer/fixtures`, each on a
  copy under `target/`. JSON output is snapshotted.

*Done (2026-11-21).* ARCHITECTURE §6.4 "As built" has the commands, the
`--json` shapes and the rules. The logic is in `invoicer_core::{session,
views, edit}`, and `main.rs` only parses and prints. The tests are
`ledgeline-invoicer/tests/cli.rs`: each runs the built binary with an empty
environment on its own fixture copy in `CARGO_TARGET_TMPDIR`. The snapshots
are committed files in `tests/snapshots/` (`insta` isn't in the lockfile),
re-recorded with `UPDATE_SNAPSHOTS=1`. These decisions are open to veto:

| #   | Decision                                                                                                                                                                                                  |
|-----|-----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| C1  | Numbering already counts the books' `invoice:` tags (§5.1), except bills on the payable side. The accrual fixture's next number is 2005, not 109.                                                         |
| C2  | `set` refuses `status`, `paid`, and `invoice_num` on a sent or void document. Those changes wait for send/void/rebook; hand edits stay possible.                                                          |
| C3  | `recalc` with no arguments rewrites drafts only. Sent and void documents are reported as skipped and rewritten only when named.                                                                           |
| C4  | `check` exits 1 on errors only; `--strict` makes warnings fail too. Any failure to run exits 2.                                                                                                           |
| C5  | "Open" and "overdue" read the `paid:` mirror until the Phase 2 reconciler exists (superseded in 2c: they read the books). Today is the **local** date (`--today` overrides); Ledgeline's server uses UTC. |
| C6  | The journal comes from `JOURNAL` or `-f JOURNAL` (hledger's flag), then `$LEDGELINE_FIXTURE`, then recents. The CLI never records recents.                                                                |
| C7  | `--row` takes `DESC\|QTY\|RATE\|ACCOUNT`. With no quantity, a money rate becomes the amount. A bare `Subtotal` writes a total row.                                                                        |
| C8  | `--json` failures print `{"error": "…"}` on stdout (exit 2), so a script reads one JSON value either way.                                                                                                 |

Left for later: `new --status sent` (send is Phase 2), `customers new` and
`import-csv`, `check --full` (a CLI index is always fresh), and prose
flags on `new` (snippets fill the prose after the table).

**Gate:** `just engine-check engine-test`, plus the new crate tests, the
benches against their baseline, clippy `-D warnings`, and fmt.

## Phase 2 — Books bridge (PRs 2a–2c)

**2a. Ledgeline core changes** (L1–L4, in `crates/ledgeline-core`):
- **L1.** `DocumentLine.tags` (extra posting tags) and `DocumentSpec.tags`
  (extra header tags). Values are validated with `check_tag_value`.
- **L2.** Negative `DocumentLine` amounts: the deposit application and
  contra-revenue discounts. `Direction` still sets the sign of the rest.
- **L3.**
  - `write_document` / `save` take an expected `generation`.
  - New `Loaded::find_document(number) -> Option<FoundDocument { tindex, fingerprint, transaction }>`,
    matching `invoice:` exactly on a transaction with a settlement posting.
- **L4.** A `receivables` module:
  - open documents per settlement account;
  - N:1 settlement (one invoice, several payments), extending plan 45's
    1:1 `LinkIndex` rule so several payment postings may join one invoice's
    link;
  - a void reversal counts as settlement;
  - aging buckets by `due:` tag (fallback: date plus terms);
  - DSO and average days to pay.

  Plan 45's UI keeps working. Its "Phase 5 — later" N:M item becomes N:1 now
  and M:N later.
- **Tests:**
  - extend `accrual_library.rs` with the full invoicer flow: send, partial
    pay, pay, rebook, void, deposit and final;
  - check fixture results against real `hledger` output in `expected/`, as
    plan 45's `capture.sh` does: `bal tag:invoice=^105$`,
    `bal tag:kpi=receivable not:tag:match`.

**2b. Invoicer booking** (`invoicer_core::books`):
- Invoice → `DocumentSpec`:
  - one posting per resolved account (and deferral), with adjustments netted;
  - pro-rata split for subtotal adjustments over revenue (item-derived)
    lines only, with the remainder to the largest posting;
  - header tags `invoice`, `document`, `due` and `reference`, with commas
    sanitized;
  - `item:` on each posting.
- Basis policy: accrual books get AR plus revenue/deferred; cash books get
  nothing at send.
- **Apply payment (cash books, I17).** Candidates are imported deposits
  ranked by amount, date and payer (plan 45's `settle::candidates` scoring,
  without the AR leg). Applying rewrites the chosen transaction's income
  postings into the invoice's row accounts, adds `invoice:` and `customer:`,
  and keeps the bank posting byte-identical. It goes through Ledgeline's
  transaction editor with the `ExternalChange` guard. Partial payments split
  pro rata over the rows. Undo restores the original text. The reconciler
  then mirrors `paid:` from `invoice:`-tagged income, exactly as it does
  from settlement links in accrual books.
- Operations:
  - **send** sets `status: sent` and `sent_date`, then books the invoice;
  - **void** writes the reversal plus a `match:` link, then sets
    `status: void` and `void_date`;
  - **rebook** follows ARCHITECTURE §4.5, including the `closed-through`
    refusal;
  - **apply deposit** lists the customer's credit balances on the deposit
    account.
- **Write order and crash safety.**
  - The journal is written first, then the markdown. If the markdown write
    fails, the next `check` sees a booked invoice with `status: draft` and
    offers to finish (`status: sent`).
  - This is tested by failing the second write, as plan 45 does.

**2b as built (2026-11-21).** `invoicer_core::books`:

| Call                                           | Does                                                                                    |
|------------------------------------------------|-----------------------------------------------------------------------------------------|
| `spec::booking_of`                             | Pure: a document to its `InvoiceBooking` (lines, header tags, customer, total)          |
| `InvoiceBooks::preview`                        | What it would book, what is booked, whether they agree (`in_step`), the `Generation`    |
| `InvoiceBooks::send`                           | Book (accrual), then `status: sent`, `sent_date`, `sent_via`, `paid: no`                |
| `InvoiceBooks::rebook`                         | §4.5: replace in place, keep the link and schedule ids; no-op when in step              |
| `InvoiceBooks::void`                           | Reversal + `match:` (accrual), then `status: void`, `void_date`                         |
| `InvoiceBooks::open_deposits`                  | A customer's deposits left on the deposit account                                       |
| `InvoiceBooks::apply_deposit`                  | Write a `Less deposit (Invoice N)` row into a draft and recompute                       |
| `InvoiceBooks::payment_candidates`             | Cash books: deposits that might pay a sent invoice, best first, each with a `reference` |
| `InvoiceBooks::apply_payment` / `undo_payment` | Cash books: re-split a deposit; undo from a serializable `PaymentUndo`                  |

Every result is `serde::Serialize` for `--json`; `BooksError::code()` names
the error. Tests: `tests/books_{spec,accrual,cash}.rs` on fixture copies
under `target/`, journal text asserted exactly; with
`LEDGELINE_HLEDGER_INVOICER_CHECK=1` in the dev shell, hledger checks the
results (`-s check`, `bal tag:invoice=…`). One Ledgeline core change:
`accrual::apply::plan` takes a negative split row (a discount booked to a
contra-revenue account; after decision A below, no longer a deposit
applied), behaviour-neutral for Ledgeline.

Decided while building, open to veto:

- **Sending into a closed period is refused** (dated on or before
  `closed-through`), like re-booking.
- **Send writes `paid: no`** when the file has no `paid`, so a sent
  invoice always has its mirror.
- **Send is idempotent across a crash.** A draft already booked exactly as
  its file says only has its file finished; booked differently, it is
  refused. A file already `status: sent` but never booked (a script wrote
  it) is booked.
- **Void of a draft** changes the markdown only (the number stays used).
  A cash-books invoice with a payment applied can't be voided until the
  payment is undone.
- **`item:`** names the item rows a line came from, joined with ` + `;
  a line no item feeds (tax, a deposit) names its own rows.
- **Tag values:** a comma between digits (`$16,700`) is dropped, every other
  comma becomes a space. The `document:` path is sanitized the same way, so
  a file name with a comma won't match its tag (2c's `check` should say so).
- **The description** is `<invoicee_name> | Invoice <number>`.
- **A person at a company** with no `ledger_customer` of their own books to
  the company's customer note (its `ledger_customer` and revenue account).
- **Apply deposit is for drafts only**; a sent invoice is edited, then
  re-booked. It doesn't write `deposit_for` (that field says what the
  deposit is *for*; the row's description names the deposit invoice).
- **Open deposits** count an invoice's applications from its file, never
  from the journal, so a draft or an unpaid cash-books invoice already uses
  its deposit up; an application uses the deposit its row names first, then
  the oldest.
- **Cash books, deferred rows:** a payment of a deferred row goes to the
  account it recognizes into (cash books have no deferred revenue).
- **Cash books, the last payment** takes each row's remainder rather than
  its pro-rata share, so the rows end exactly at the invoice's amounts.
- **Apply payment never writes the markdown**; the 2c reconciler mirrors
  `paid:`.

Answered by Patrick (2026-11-21):

- **The send budget is relaxed to 300 ms** (ARCHITECTURE §5.6) until L10 or
  an in-place re-parse in `ledgeline-core` lands; 150 ms stays the goal. On
  the corpus send takes 291 ms: 72 ms loading the journal and about 190 ms
  in Ledgeline's `write_document` re-parsing and saving the 9.2 MB journal;
  the invoicer's own part is under 30 ms. `invoicer/send` (289 ms) and
  `invoicer/preview` (92 ms) are now gated in `baseline.json`; only those
  two entries were added, recorded with other work running on the machine.
- **A final invoice its deposit fully covers (a zero total) is booked at
  send with no receivable posting**: its rows against the deposit
  liability. A `$0.00` receivable line was rejected: Ledgeline's documents,
  receivables and matching deliberately ignore a zero settlement posting,
  and making them count one would change what Ledgeline shows. So core
  gains `accrual::invoice::build_settled_document` (opt-in; Ledgeline's
  paths keep the strict builder), and the bridge finds the booking itself
  by `invoice:` plus `document:` with no settlement leg. Consequences:
  - it is never in receivables or aging (nothing is owed); the 2c
    reconciler should mirror `paid:` as the invoice date for it;
  - re-book moves either way between zero and a positive total, but never
    drops a receivable with payments linked;
  - void reverses the rows, with no `match:` (no receivable to net);
  - send writes `paid: <invoice date>` rather than `paid: no` for any zero
    total, so `list --open` doesn't show it before the reconciler runs
    (approved by Patrick, 2026-11-21);
  - in cash books it is a `$0` marker instead (decision A below).
- **The fixture's 104 `item:`** now reads `($16700)`, as the bridge writes
  it (hledger ends a tag value at any comma). The root
  `fixtures/receivables/books.journal` (plan 45's) keeps its own shorter
  item, which `accrual_library.rs` writes literally; nothing else captures
  it.

**Cash-books deposits: decision A, income on receipt** (Patrick,
2026-11-21). Accrual books are unchanged (the deposit is held in the
liability and the final invoice moves it to revenue). In cash books:

- **A deposit invoice's payment is revenue the day it arrives.** Apply
  payment re-splits the bank deposit into revenue, never the liability.
  The account: the deposit row's own when it is not the deposit account;
  when it is (the usual case, `liabilities:customer-deposits`), the
  customer's `revenue_account`, else `[invoicer] revenue-account`, else the
  books' single revenue account; with none of those, Apply payment refuses
  and says what to set. *For veto:* it doesn't try to guess the final
  invoice's rows, which don't exist yet when the deposit arrives.
- **A final invoice's deposit row books nothing.** How a payment is split
  over the other rows is `[invoicer] payment-allocation` (Patrick,
  2026-11-21):
  - `match-rows` (default): each row account ends at the invoice's row,
    counting what the deposit already put there. 207: the $5,000 deposit is
    in consulting, the rows are consulting $8,000 and delivery $4,000, so
    the $7,000 splits $3,000 / $4,000. A partial payment fills what each
    row still lacks pro rata; the settling one takes the remainders. A
    deposit in an account no row uses stays there, and the rows give up
    that much pro rata. A deposit larger than its row leaves that row at
    zero (never a negative share) and the excess comes off the other rows
    pro rata; that account ends over its row, since the deposit can't be
    moved. Contra rows keep their amount.
  - `pro-rata`: every payment over the rows' proportions, whatever the
    deposit booked (207: $4,666.67 / $2,333.33; consulting ends at
    $9,666.67).
  - The deposit a row applies is the deposit invoice it names (`Less
    deposit (Invoice 206)`); its applied payments say which accounts it
    went to. A row naming none counts as a deposit in an account no row
    uses.
- **`[invoicer] cash-deposits`** chooses the deposit treatment:
  `income-on-receipt` (default, decision A) or `liability-until-final`, the
  earlier behaviour: the deposit's payment goes to the deposit account, the
  final invoice's deposit row debits it in the payment split, a fully
  covered final books its rows against the liability at send, and open
  deposits are read from the journal as in accrual books.
- **A fully covered final invoice books a `$0` marker at send**: its
  header (`invoice:`, `document:`, `due:`, `reference:`) and one `$0.00`
  posting per row, the row's cash account (the deposit account for the
  deposit row), each with `customer:` and `item:`. hledger reads it as
  balanced (`-s check` passes). It lets `check` and the reconciler see it
  was sent; the reconciler should mirror `paid:` as the invoice date, and
  send already writes that. Re-book replaces it; void reverses it (another
  `$0` transaction tagged `void:`).
- **Voiding a final invoice gives its deposit back** for a reissue: open
  deposits in cash books are what deposit invoices were paid (their applied
  payments, up to their deposit rows), less the applications in non-void
  files. *For veto.*
- A cash-books deposit held in the liability by hand (a journal credit on
  the deposit account) is not counted as an open deposit.
- The cash fixture's 206 wire now lands in `revenue:consulting`; 207's
  markdown is unchanged (its deposit row is what the customer sees). The
  CLI snapshots (#86) are unaffected; the CLI's open/overdue read only the
  `paid:` mirror.
- Core's `build_settled_document` stays: accrual books need it.

Left for 2c: the CLI commands, `check` over the books (including the
booked-draft finding from a failed markdown write), and the reconciler and
the `paid:` mirror.

**2c. Reconciler and CLI:**
- `reconcile`: payment state to the `paid:` mirror, with book/markdown
  mismatch findings.
- `receivables [--aging]`.
- `recognize --through`, which wraps `post_due` and never posts after today.
- `send`, `void`, `rebook`, `apply-payment NUM [--txn REF]` (cash books;
  without `--txn` it lists candidates).
- `check` now also checks the books: duplicate numbers, missing transactions,
  orphan transactions, moved `document:` paths.
- **Benches:** send, and the reconciler after a journal change, on the
  corpus (budgets in ARCHITECTURE §5.6).
- **Tests:**
  - a scripted end-to-end CLI scenario on a fixture copy, checking the
    journal text and the markdown after each step;
  - a concurrent write (Ledgeline's editor saves between load and save),
    which must give `ExternalChange`, write nothing, and retry once.

**2c as built (2026-11-22).** ARCHITECTURE §6.4 "As built (Phase 2c)" has
the commands and their `--json` shapes; §5.4 the reconciler.

| Piece                                     | Does                                                                                                       |
|-------------------------------------------|------------------------------------------------------------------------------------------------------------|
| `reconcile::BookStates`                   | One pass over the journal: each number's bookings, voids, settlements (accrual) or applied payments (cash) |
| `BookStates::paid`                        | The books' `paid:` for a document; `None` when the books can't say                                         |
| `BookStates::changed_since`               | The numbers a journal change touched, so the app re-reconciles only those (§5.6)                           |
| `reconcile::reconcile` / `reconcile_with` | Mirror `paid:` where it differs (one key, conditional on the revision read), plus the findings             |
| `reconcile::audit`                        | The books checks `check` adds (below), each with a fix-it; scoped to some numbers or all                   |
| `receivables::receivables`                | Open invoices, aging, DSO, days to pay, unapplied payments, drafts waiting                                 |
| `InvoiceBooks::recognize`                 | Plan 45's `post_due` through a date, never after today; `--dry-run` lists                                  |
| `InvoiceBooks::payment_options`           | Cash books: total, paid, due, and the candidates                                                           |
| `books::undo`                             | Apply payment's undo records (`.ledgeline-invoicer/undo/*.json`)                                           |
| `books::retry_once`                       | Re-run a books write once after `external-change`                                                          |
| `StepCheck`                               | Compare many documents with their bookings in one pass (catalogs, account, id generator read once)         |

`check` now finds, over the books: a number booked twice (error), a sent
invoice with no transaction (error; fix `send N`), a transaction with no
invoice file (warning; the accrual fixture's plan 45 invoices 2001–2004 are
four), a `document:` that no longer names its file (warning; fix `rebook N`,
or by hand in a closed period) or a file name a tag can't hold exactly (a
comma), a booked draft (error; in step, fix `send N` to finish a send whose
markdown write failed), a void on one side only (error; fix `void N --date
D` when the books have it), books out of date (warning; fix `rebook N`
outside the closed period), a cash-books void with payments applied
(warning), and a stale `paid:` (warning; the fix sets it). A new
`FixEdit::Command` carries the command. `list` and `customers` count the
document checks only; `show N` adds N's books checks.

Decided while building, open to veto:

- **Open and overdue read the books** (C5 is superseded): settlement links
  in accrual books, applied payments in cash books, a zero total paid on
  its date. An invoice the books can't speak for (accrual, never booked)
  falls back to its mirror, and `check` reports it. `list --open` is right
  before `reconcile` runs.
- **The mirror's text**: `no`; `partial 2026-11-05 $1,000, 2026-11-20 $500`
  (payments on one day merged, amounts in the table's money style, whole
  amounts without cents, as I7's example); or the date of the payment that
  completed it. A mirror that already says the same, however its amounts
  are written, is left alone, so the fixtures' `$1,000` stays.
- **Only sent invoices are mirrored.** A void invoice keeps whatever its
  mirror says.
- **Cash books: what a payment paid** is what its income side was re-split
  into (its `customer:`-tagged postings off the cash accounts): the gross,
  before a processor's fee, which is your expense.
- **Credits count as paid** in the accrual mirror (a credit note or write-off
  linked to the receivable). A planned alternative is in the flows table.
- **Every CLI books write is retried once** when the journal or the file
  changed between reading and writing (Ledgeline saved meanwhile). The first
  attempt wrote nothing; the second reads afresh; a note goes to stderr.
  A second change in a row is returned (`code: external-change`).
- **The CLI refreshes the mirror after a write that moves payments**
  (`apply-payment`, its `--undo`, and a `rebook` that changed the books),
  reporting it as `mirror`. `send` writes `paid` itself; `void` leaves it.
- **Apply payment's undo records** are JSON files in the books folder under
  `.ledgeline-invoicer/undo/`, one per applied deposit
  (`apply-payment-<number>-<ref>.json`): the invoice, the deposit's date and
  description, and its text before and after. Hidden, so discovery skips
  them; not in the journal; readable by hand. `apply-payment N --undo`
  writes the deposit back exactly and deletes the record; with several, it
  asks for `--txn`. A record whose deposit was edited since is refused by
  the books and kept.
- **`--txn` takes a prefix** of a candidate's reference (6 characters or
  more) when it names only one.
- **`--json` errors from the books carry a `code`** beside `error`
  (`BooksError::code`: `status`, `refused`, `external-change`, …); added,
  per the shapes rule.
- **`reconcile` exits 1 on error findings** (like `check`) and 2 when a
  mirror couldn't be written.
- **The CLI parses the journal once.** The session keeps its parse;
  the first books load reuses it (`InvoiceBooks::with_editor`), and later
  loads read the files again.
- **The perf corpus books `item:`** (v2), as the bridge writes it, so every
  corpus invoice is in step and `books_check` measures a clean pass.

Left for later: an `--only-books` or `--fast` `check` (the books checks
read every booked invoice's file; see the bench), mirroring `sent_hash`
(Phase 3c), and `customers` open counts from the books for cash books'
deposits.

**Gate:** everything from Phase 1, plus `just test-integration`. Ledgeline's
API shapes are untouched, but run it anyway, since `accrual_api` uses the
changed core.

## Phase 3 — App shell, rendering, PDF, branding (PRs 3a–3d)

**3a. Server and GUI boot:**
- `ledgeline-invoicer` with the `gui` feature boots like Ledgeline, through
  `ledgeline-shell`.
- The MBR `Server.router` is merged at `/`, the SPA is at `/app`, and the API
  is at `/api`.
- `--server` mode for e2e, on ports **5199** and **4273**. They differ from
  Ledgeline's 5099 and 4173, so the two e2e suites can run side by side.
- The watcher covers journal files plus the markdown under the vault, and
  triggers the reconciler.
- The same token and Host guard. Theme and scale come from shared prefs.

*3a built (2026-11-22).* The app is `ledgeline-invoicer/src/app/`: `mod.rs`
(identity, router), `books.rs` (the open books: session plus MBR), `api.rs`,
`mbr.rs` (MBR's mount and its gate), `prefs.rs`, `watch.rs`, `serve.rs`
(`--server`) and `gui.rs` (the window). The router tests run in-process on
a fixture copy (`app/tests.rs`). The API so far: `GET /api/version`,
`/api/books`, `/api/invoices` (`list --json`), `/api/invoices/{num}` (`show
--json`), `/api/customers`, `GET`/`PATCH /api/ui-prefs`, and `POST
/api/session`. The SPA stub loads the saved theme and scale and asks for the
session cookie; `vite dev` and `preview` run on 4273 and proxy `/api` to
5199. Decided while building, open to veto:

| #  | Decision                                                                                                                                                                                                                                                                                                                                                                                                                                |
|----|-----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| A1 | **GUI or CLI.** A subcommand is always a CLI job, in any build. With no subcommand, `--server` runs the headless server and otherwise the window opens. Features: `server` (API, SPA, MBR, watcher), `gui` (implies `server`; the default, as Ledgeline's is). `--no-default-features` is the CLI alone, and with no subcommand it asks for one. `--json` with no subcommand is an error in every build: a script never wants a window. |
| A2 | **MBR's pages are gated.** They are the books' notes, so they need the token like `/api`. A browser loads them by navigation, which can't send a header, so `POST /api/session` (token-gated) sets a second per-process secret as an `HttpOnly`, `SameSite=Strict` cookie, named after the port. A page is served with the bearer token or that cookie; otherwise `401`.                                                                |
| A3 | **MBR's pages get their own CSP**: inline scripts allowed (MBR's page config needs them), `https:` images, media and frames, `connect-src 'self'`, `frame-ancestors 'self'` (so the SPA can frame a page). The SPA shell keeps the shell's script-hash policy.                                                                                                                                                                          |
| A4 | **MBR's in-browser editing is off** (`edit_enabled = false`, also applied when its `.mbr/config.toml` reloads). Every write goes through the invoicer's token-gated, hash-checked API. MBR runs as a `LiveServer` read at the books folder (`Config::read_at_root`), with its own watcher on.                                                                                                                                           |
| A5 | **The token globals are Ledgeline's** (`__LEDGELINE_TOKEN__`, `__LEDGELINE_EMBEDDED__`): the shared API client in `packages/ui` reads them. The token variable is the invoicer's own: `LEDGELINE_INVOICER_TOKEN`. The e2e switch is shared: `LEDGELINE_UI_PREFS_IN_BROWSER`.                                                                                                                                                            |
| A6 | **`--server` needs a journal** (JOURNAL, `-f`, `$LEDGELINE_FIXTURE` or recents) and never records recents, since it runs over fixtures; the window records the books it opens. With nothing to open, the window starts with no books (File → Open).                                                                                                                                                                                     |
| A7 | **The watcher** covers the books folder recursively (hidden folders skipped) plus the folder of any journal file outside it, debounced 250 ms. Markdown changes refresh the index; a journal change reloads the journal and `[books]` (`Session::reload_books`) and reconciles only the invoices whose payment state moved; a `ledgeline.toml` change reopens the session and reconciles all. The reconciler also runs once on open.    |
| A8 | **The invoicer's UI-prefs schema** is the two common keys (`theme`, `uiScale`), both pinnable. A key only Ledgeline knows is refused by `PATCH` and survives this app's saves.                                                                                                                                                                                                                                                          |
| A9 | **File → New Invoice** loads `/app/new`, the page 4a builds; until then the SPA shows its empty shell there.                                                                                                                                                                                                                                                                                                                            |

**MBR 0.7.4** (crates.io, pinned `=0.7.4`; `mbr-server` with `watcher`)
has everything MBR-CHANGES asks for P0/P1: `site::Site` (M2),
`frontmatter_links` (M1), `standalone::render_note_source` (M3),
`pdf_export::ExportSession::start` in `mbr-markdown-browser` (M4, with
`@page` winning), `/.mbr/_export_ready.html` (M5), `<mbr-markdown-field>`
(M6), `ServerConfig::{extra_head_html, theme_vars}` (M8), the
frontmatter-cap fallback (M9) and `render_source_with_template` (M10). Gaps:

- **M7** (P2, `data-col` on table cells) isn't in 0.7.4. Not needed: the
  books column uses the `:has()` rule.
- **`Server::retire` is `pub(crate)`.** An embedder swapping folders can't
  stop a plain `Server`'s background tasks. Worked around by using
  `LiveServer`, whose `Drop` retires its server; the old one stops once its
  router's last request ends.
- **Tera turns on serde_json's `preserve_order`**, workspace-wide in any
  build that includes the app: JSON objects keep insertion order instead of
  sorting. `cargo build -p ledgeline` (and `.#ledgeline`) is unaffected,
  but a workspace build or `.#tests` is. Three places assumed sorted keys:
  bank sync's `extra` column (`feeds::simplefin`, now sorted explicitly, so
  its CSV is the same either way), the CLI snapshot tests (now normalized
  with sorted keys) and one frontmatter test (accepts both). The invoicer's
  `--json` frontmatter maps follow insertion order in the app build.

Left for later: the `notes` → `mbr_core::Site` swap (not low-risk: the two
APIs differ in path types, note stamps and line lookup, and it would put
mbr-core into the CLI-only build; Phase 4, or once `Site` can be shared
with the server's own via `Server::site()`), pushing the theme into MBR's
chrome (M8 `theme_vars`, 3b), saving the View-menu zoom back to the prefs,
the recognition file in the watcher. (The e2e harness on 5199/4273 came
in 3b.)

**3b. Templates and preview:**
- Two starter templates in `invoicer/crates/ledgeline-invoicer/templates/`,
  copied into the user's `invoices/tpl/` on first use. Each has:
  - print CSS;
  - the books-column `:has()` rule;
  - MBR's export-ready partial (M5).
- `POST /api/preview` uses M3 `render_note_source`.
- A minimal SPA:
  - the sidebar;
  - an invoice list (read-only, from Phase 1's API);
  - an invoice page showing the MBR-rendered invoice and a Books panel.

*3b built (2026-11-23).* The starters are `ledgeline-invoicer/templates/`
(`include_str!`, in flake.nix's source filter) and `src/starters.rs`. The
API gains `POST /api/preview` (`app/preview.rs`: `{number|path, source?,
template?}` → `{html, path, template}`, MBR's standalone `Renderer`, one
per MBR server, with its live wikilink index; `Server-Timing: render;dur=`;
400/404/422; writes nothing), `GET /api/templates` (the books' templates,
then the starters not copied in) and `GET /api/invoices/{num}/books` (the
Books panel: `invoicer_core::views::invoice_books`). The watcher refreshes
`.html` files too. The SPA has the sidebar, the read-only list
(`/app/invoices`) and the invoice page (`/app/invoices/[number]`: MBR's page
in a frame, a template picker that previews through `/api/preview`, the
Books panel), with vitest `unit` and `components` projects. The vite proxy
now forwards everything outside `/app` to 5199, so MBR's pages and their
cookie are same-origin under `vite dev`/`preview`. The Playwright suite is
`invoicer/web/e2e/` (`just invoicer-e2e`: open, list, open an invoice,
preview, plus a same-origin CSP spec); written, not yet run.

**Pending:** the `--server` smoke test of `/api/preview` and its timing
against the 50 ms keystroke-to-preview budget were not run (the agent's
permission to start the server was denied). Router tests render every case
in process; the budget still needs a release-build measurement
(`Server-Timing`), with Patrick.

Decided while building, open to veto:

| #  | Decision                                                                                                                                                                                                                                                                                                                                                                                                       |
|----|----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| B1 | **Starters are copied one at a time, on first use**: when `new` writes an invoice whose `template:` is a starter's path in `templates-dir` (named by `--template` or the customer's `invoice_template`), or Classic when the books have no invoice template at all. The copy is create-new (an existing file is never replaced), made only once the invoice is written; a dry run or a preview copies nothing. |
| B2 | **Invoice frames are sandboxed `allow-same-origin`, with no scripts.** MBR's pages are same-origin with the SPA, so a script in a template could otherwise read the app's token from the parent window. A template needs no script to show an invoice; the export-ready partial matters only in the PDF export's own webview.                                                                                  |
| B3 | **A preview frame (`srcdoc`) inherits the SPA's CSP**, so a template's remote (`https:`) images show on MBR's page but not in a preview of another template or of unsaved text.                                                                                                                                                                                                                                |
| B4 | **The SPA sends the browser's local date as `today`** on the list and the invoice, so overdue follows the user's calendar (and a pinned clock in e2e), not the server process's.                                                                                                                                                                                                                               |
| B5 | **invoicer/web declares no test libraries of its own.** jsdom, `@testing-library/svelte` and `@playwright/test` resolve from the hoisted root `node_modules`, where `web/` and `packages/ui` declare them; declaring them again would change `bun.lock` and so the `spaNodeModules` pin. Their versions follow `web/`'s.                                                                                       |
| B6 | **Template details:** `paid` is read with `\| as_str` (it may be a YAML date); dates print through Tera's `date` filter ("November 10, 2026"); Paid/Void/Draft marks come from `status` and a dated `paid:`. A preview through a starter not copied in uses the shipped text (M10).                                                                                                                            |

**3c. PDF:**
- `pdf NUM [-o PATH]` and `POST /api/invoices/{num}/pdf`, by calling MBR's
  `ExportSession::start` on the shell's own event loop.
- Final exports are kept in `pdf-dir`, and earlier exports are kept with
  their date appended (I12).
- The portal Send flow for customers with `billing_portal`: instructions,
  Open portal, copy buttons, `sent_via: portal`.
- The draft → "Mark as sent?" prompt.
- The `sent_hash` mirror and the changed-since-sent warning.
- Share… (`NSSharingServicePicker`) and Copy file.
- **Spike:** native drag-out of a temp PDF from the webview, timeboxed to 1
  day. If it isn't feasible, ship Share and Copy and record why.

*3c built (2026-11-23).* `ledgeline-invoicer pdf NUM [-o PATH]` and the
API: `GET`/`POST /api/invoices/{num}/pdf` (the Send panel's state; an
export), `POST /api/invoices/{num}/send` (`{via}`), `POST
/api/invoices/{num}/portal`, `POST /api/pdf/share` and `/api/pdf/copy`.
Code: `src/pdf.rs` (naming, filing, every build), `invoicer_core::sent` (the
mirror), `app/native.rs` (the seam), `app/desktop.rs` (the event-loop host:
MBR's `ExportSession`, `NSSharingServicePicker`, the pasteboard),
`app/pdf.rs` (routes and the CLI loop). The SPA's invoice page has a Send
panel (`SendPanel.svelte`, `lib/delivery.ts`): Export PDF, the prompts,
Share… and Copy file when the window backs them, the kept PDFs, and the
portal's instructions, Open portal and copy buttons. `pdftotext`
(poppler-utils) is in the dev shell; the e2e spec exports 105 through the
API and the panel and reads the PDF's text.

**Pending (the lead):** nothing here has printed a real PDF yet. The agent
didn't launch the app; router tests use a mock printer. Run
`nix develop -c cargo test -p ledgeline-invoicer --test cli -- --ignored pdf_`
(prints through WKWebView: needs a desktop session) and `just
invoicer-e2e`, and try Share… and Copy file in the window (the picker is
documented as wanting a mouse-down; it is shown after an API round trip).

Decided while building, open to veto:

| #  | Decision                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
|----|---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| P1 | **One print path, three event loops.** The window's loop, `--server`'s (a desktop build runs a windowless loop on the main thread, no Dock icon, so the e2e can print; where MBR's `preflight` fails, as on Linux with no display, the server runs alone and the PDF routes answer `503`), and the CLI's own `pdf` loop (a private server on an ephemeral port). The API reaches the loop through a `Native` trait (`app/native.rs`), which the router tests mock. A CLI-only build's `pdf` says it needs the `gui` feature (`code: needs-gui`).                                                                                                                                                                                                              |
| P2 | **One-time export links.** The hidden export webview can carry neither the token nor the cookie, so the PDF route mints a link, `/.ledgeline-invoicer/export/<ticket>`, for one page: good once, within 60 s, it sets the session cookie and redirects to that page. The ticket names the page server-side, so it redirects nowhere else.                                                                                                                                                                                                                                                                                                                                                                                                                     |
| P3 | **Where a PDF goes.** Sent: kept as `pdf-dir/<the markdown's name>.pdf`, printed to a hidden file beside it and moved in only once MBR has verified it. Draft or void: a temporary file (`$TMPDIR/ledgeline-invoicer/`). `pdf -o PATH`: a copy there, nothing kept, no mirror. An existing kept PDF is first renamed `<name> <its own date>.pdf` (then ` (2)`, …); a failed move puts it back.                                                                                                                                                                                                                                                                                                                                                                |
| P4 | **`sent_hash`** is FNV-1a 64 (16 hex digits) of the whole file minus the `paid` and `sent_hash` lines, so a payment never reads as a change; the template is not hashed. Every kept export writes it, unless the file changed while printing (then the next export warns). `set` refuses it, like `paid`.                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| P5 | **The prompts.** Exporting a draft asks "Mark invoice N as sent?": *Mark as sent and export* sends (books it) with `sent_via` (default `portal` when the customer has one, else `email`), then exports the kept PDF; *Export a draft PDF* writes a temporary one. Exporting a sent invoice whose hash differs asks first and warns in the result. There is no separate "mark sent" button: one path.                                                                                                                                                                                                                                                                                                                                                          |
| P6 | **`send` refuses** an invoice whose customer's portal has `requires_reference` and which has none (in core, so the CLI too; ARCHITECTURE §3.4 said Send refuses, and it didn't).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| P7 | **Templates without MBR's export-ready partial** (the fixture's own) print on `load` plus fonts (`ReadySignal::Load`), not after the 30 s timeout. Paper and margins: MBR's `[pdf]` settings, the template's `@page` winning.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| P8 | **The API never shows full paths.** Kept PDFs are named by books-folder path; Share… and Copy file take `{destination: kept|temp, file}` and only reach a plain `.pdf` name in `pdf-dir` or the temp folder. Open portal opens only the customer note's plain `https:` URL; without a window the SPA opens it in a tab.                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| P9 | **Drag-out (spike): not shipped.** WKWebView has no drag-out of a file: an HTML drag gives a URL or an image, never a file, and the `DownloadURL` drag type is Chromium's. A native drag needs `beginDraggingSessionWithItems:event:source:` with the live mouse-down event, but the click reaches Rust only after a round trip through WebKit's web-content process (IPC or HTTP), when AppKit's current event is no longer that drag. It would mean intercepting `mouseDown` on the WKWebView itself (subclassing or swizzling tao/wry's view) and asking the page what is under the pointer: possible, but not verifiable without a GUI session, so it was not attempted. Share… and Copy file cover Mail, Messages and a browser upload (paste the file). |

*Templates revisited (2026-11-23).* Patrick's review: the fixture's MyCo
default had no addresses or contact and an unruled table; an invoice opened
in MBR lost MBR's sidebar, search and navigation. Now:

| #  | Decision |
|----|----------|
| T1 | **The fixture's template** (both books) shows "From" the company and "Bill to" the customer, each with address and email, a dates line (date, due, terms, reference, deposit for), and a bill table with a rule on every cell and padding. It stays plain test data, without MBR's export-ready partial (P7's print-on-load path keeps its test). |
| T2 | **MBR's chrome where it works, the sheet alone elsewhere**, using only what 0.7.4 has: when MBR serves the page (`server_mode`) a template includes MBR's partials (head, nav, breadcrumbs, previous/next footer, scripts) around `<main>`; a `<noscript>` rule hides everything but `<main>` (the app's frame runs no scripts, B2); MBR's print rule does the same on paper and in PDF export; a standalone render (`/api/preview`, QuickLook) has `server_mode` false and no chrome. The app needs no change. No MBR change is proposed. |
| T3 | **The sheet is guarded against MBR's stylesheet**: Pico's variables are pinned on the sheet, and the templates avoid elements Pico or MBR's theme restyle inside `<main>` (`article`/`header`/`footer` cards, `dl` lists), so the sheet looks the same in MBR, the frame, the preview and on paper. Pico's print root size (62.5%) is reset to 100%. |

**Pending (the lead):** the `<noscript>` rule was checked in a sandboxed
(no-scripts) frame in Firefox; check it in the app's WKWebView frame
(`just invoicer-e2e`, and a look at 105), and print one PDF from a template
with chrome (`cargo test ... --ignored pdf_`) to see MBR's print rules keep
only the sheet. If a frame ever runs scripts, the chrome would show there:
see MBR-CHANGES "Not needed", `?embed=1`.

**3d. Branding:**
- The icon is derived from `graphics-sources/ledgeline-logo-no-word.svg`.
  - The indigo `#524ED2` becomes a green of the same OKLCH lightness and
    chroma, hue about 150°, clamped to sRGB. Check that the `#EAEAEA` glyph
    still has contrast ≥ 3:1.
  - A bold lowercase "i" in the glyph colour goes in the bottom-right
    quadrant, not touching the arrow.
- New `graphics-sources/ledgeline-invoicer-logo*.svg` and
  `assets/LedgelineInvoicer.icon/` (an Icon Composer package with
  light/dark).
- Extend `scripts/gen-icons.sh` to produce `Assets.car` and the PNGs, and
  `invoicer/web/static/*-icon.svg`.
- Patrick reviews the icon before 3d merges.

**Gate:**
- `just pre-push`, plus the invoicer e2e: open the fixture, list, open an
  invoice, preview, export a PDF and check that it starts with `%PDF-` and
  has the expected text (`pdftotext`, in the dev shell).
- The lead runs Playwright; agents don't.

## Phase 3 feedback (2026-11-23)

Patrick tried the 3a–3c app on a copy of `fixtures/accrual`. Fixed on
`2026-11-23-invoicer-feedback` (stacked on 3c):

| #  | Patrick saw / asked                                                            | What changed, and why                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
|----|--------------------------------------------------------------------------------|------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| F1 | A starter template's preview was a blank white frame.                          | The preview was a `srcdoc` frame, which inherits the SPA's CSP; its `frame-ancestors 'none'` is applied to it by WebKit (not Chromium, so the e2e passed). A preview is now a page, `GET /.ledgeline-invoicer/preview?number=&template=`: MBR's gate (the session cookie), MBR's CSP, a `<base>` at the note's page. The SPA frames it by URL. `POST /api/preview` stays for Phase 4's unsaved text.                                                                   |
| F2 | Editing the open invoice in another editor changed nothing in the app.         | Nothing told the SPA. `OpenBooks` broadcasts a change event once each settled watcher burst is applied (markdown, `.html`, journal files, `ledgeline.toml`; never a PDF), naming files by the books folder. `GET /api/events` streams them (server-sent events, token-gated, read with `fetch`; `ready` first; ends on File → Open, and the SPA reconnects and reloads). The list, the invoice page (detail, Books, Send, the frame) and the sidebar re-fetch quietly. |
| F3 | Mark as sent: "invoice 108 is sent, but the books have no transaction for it". | Send did book it, through the books bridge's own copy of the journal editor; the app's session kept the old journal, and the SPA asked at once. `POST /api/invoices/{num}/send` now reloads the session's journal before answering (accrual and cash).                                                                                                                                                                                                                 |
| F4 | Click anywhere on a list row to open it.                                       | A row click opens the invoice unless it lands on a link or button. The number stays the row's one link (Tab, Enter), so nothing interactive is nested.                                                                                                                                                                                                                                                                                                                 |
| F5 | The Books panel under the invoice, full width.                                 | The page is Send panel (two columns), the invoice, the Books panel, each full width.                                                                                                                                                                                                                                                                                                                                                                                   |
| F6 | Show where it was booked; the payments list and `paid:` line confused.         | One plain status line from a new `settlement` (Paid in full on …, Partly paid: $X of $Y, $Z due …, Unpaid: … due …, Void); each booking's postings (receivable marked); a payments-and-credits table (date, account, description, amount; `ReceivableState.settled_by` carries each one's transaction); cash books' payment transactions. The file's `paid:` is mentioned only when it disagrees (`mirror_problem`, plain words).                                      |
| F7 | Click the saved PDF to open it; one line, not two.                             | `POST /api/pdf/open` takes only the `{destination, file}` handles of Share…/Copy file and runs `/usr/bin/open` on the vetted file (capability `open_file`: the window, macOS). The current PDF is one line (Saved … or Kept …), clickable, and earlier PDFs are listed and clickable.                                                                                                                                                                                  |
| F8 | "No invoices yet" should say how to make one.                                  | It shows `ledgeline-invoicer new --customer … --row …` and what it needs (the company and customer notes, `[invoicer]`), until Phase 4's editor.                                                                                                                                                                                                                                                                                                                       |

**Pending (the lead):** the e2e (`just invoicer-e2e`) and the window. The
srcdoc root cause (F1) is WebKit behaviour inferred, not observed: check a
starter preview in the window. Playwright's Chromium never showed the bug.
New e2e: 108 through both starters (content visible in the frame), a row
click and Enter, the new Books lines, live reload of an edited 106 (restored
after), and Mark as sent on 108 (last in the file: it sends 108). The
starters' markers (`No. 108`, `#108`) assume the templates as of 3b.

**Second pass (2026-11-23).** Patrick on a copy of the accrual fixture again:

| #   | Patrick saw / asked                                                                           | What changed, and why                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
|-----|-----------------------------------------------------------------------------------------------|------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| F9  | The template picker only previews; editing `template:` to the Modern starter's path "failed". | Starters are copied in only on first use (B1), so `/invoices/tpl/modern.html` wasn't in the books: MBR's page for the invoice was its own 500 page, "Page template error: invoices/….md: template `/invoices/tpl/modern.html` was not found", inside the frame (`show` and `check` warned `unknown-template`). Now the picker says which template the invoice uses and has **Use this template**: `POST /api/invoices/{num}/template` writes `template:` through `set` and copies a starter in (create-new). `set template=…` on the CLI copies it too. A hand edit naming a starter is copied in when the watcher sees it, and when the books open. |
| F10 | Editing a row in an editor updates the view, but subtotals and totals stay stale.             | The watcher recalculates a **draft** once an outside edit settles (`edit::recalc_on_save`, `[invoicer] recalc-on-save = "drafts"`, the default; `"never"` turns it off). A sent invoice never: the invoice page shows "Totals don't match the rows" (each written amount next to the computed one, from `show`'s `table.discrepancies`) with **Recalculate**: `POST /api/invoices/{num}/recalc`, the CLI's `recalc`; for a sent invoice only after a confirmation (`rebook: true`), and then re-booked by `rebook`'s rules. A void invoice gets the notice and no button.                                                                            |
| F11 | Phone numbers for the company and customers.                                                  | `phones:` on person and organization notes, MBR's labelled list (its contact card reads the same key), picked by the billing labels as `emails:` is. `new` snapshots `invoicee_phone` and `invoicer_phone`; `show`, `customers` and the API carry them; Classic, Modern and the fixtures' `myco-default.html` print both. The fixture company, Customer X and Contoso have phones; Customer X's 105, 108, 204 and 207 have the snapshot.                                                                                                                                                                                                             |

Decided while building, open to veto:

| #    | Decision                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
|------|------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| F9a  | **A starter an invoice names is copied in on demand**, rather than `check` reporting it with a fix: it is what the user plainly meant, and the copy is create-new (never over a file already there), so it only ever adds the shipped text. It does write a file the user didn't save: the app logs it ("copied the starter template to …"). The CLI copies only when it writes the `template:` itself (`new`, `set`); `check` stays read-only and keeps the `unknown-template` warning. |
| F9b  | **Use this template follows `set`'s rules**: any template the books have or a starter, for the document's type; a sent or void invoice may change it too (styling; the books never see it). The `template:` line is part of what `sent_hash` covers, so a sent invoice whose template changed warns at its next export that the PDF differs from what the customer has (P4, P5).                                                                                                         |
| F10a | **Loops and clobbering.** `recalc_on_save` rewrites only a draft whose file is still exactly what the index read for the settled change (a newer edit is left for its own settle), and the save is atomic and conditional on the revision read (`Vault::replace`). Its write comes back as a change whose amounts agree, so it ends there; no "ignore my own write" list is needed. An editor holding the file open sees it change on disk, as with any outside write.                   |
| F10b | **Recalculating a sent invoice checks rebook's refusals before writing** (`InvoiceBooks::check_rebook`: a sent invoice, neither its booked nor its own date on or before `closed-through`), so a closed period leaves the file as it was. A refusal only the new booking shows (a dropped schedule) comes after the markdown is rewritten; the answer says so and names `ledgeline-invoicer rebook N`.                                                                                   |

**Pending (the lead):** the e2e, new specs: Use this template on 108
(Modern, the file and the starter put back after), an outside edit to 108's
hours recalculated, 105's totals notice and its confirmation (cancelled, so
the books stay), and the phone numbers on 105's page. The real-watcher
recalculation is covered in process (`the_watcher_recalculates_an_edited_draft_and_stops`);
try it once in the window with an editor that keeps the file open.

## Phase 4 — Editors (PRs 4a–4d)

**4a. Invoice editor** (ARCHITECTURE §6.3):
- **Header form:**
  - customer and template comboboxes with inline Add new;
  - number, dates, terms;
  - reference with `open_pos` suggestions;
  - custom fields with `/api/fields` autocomplete.
- **Bill-table grid:**
  - drag to reorder (keyboard reorder too);
  - row-type buttons: item, discount %, discount $, subtotal, tax, apply
    deposit, spacer;
  - a catalog combobox;
  - an account picker using Ledgeline's account list plus the deferral
    popover;
  - live amounts, with each adjustment's base shown inline.
- **Prose fields** use MBR M6, or a CodeMirror markdown field as a stopgap.
- **Live preview pane.**
- **Books panel:** the would-book or booked postings, schedules and payments.
- **Save:** content hash plus generation, with conflict UI.
- **Send / Void / Rebook / PDF** buttons with the confirmations from §4.

*4a built (2026-11-24).* Core: `invoicer_core::editor` (`form_of`,
`apply`, `save`), `InvoiceBooks::preview_of`. App:
`app/editor.rs`, merged into the API: `GET`/`PUT /api/invoices/{num}/form`,
`POST …/compute`, `GET …/deposits`, `POST …/void`, `POST …/rebook`, `POST
/api/invoices` (`new`), `GET /api/editor/options`, `GET
/api/fields/{key}/values`, `POST
/api/preview/draft`; errors are `{error, code}`. SPA: `/app/new`,
`/app/invoices/{n}/edit` (header form, `BillGrid`, deferral popover, prose
fields, preview frame, Books panel, Send/Void/Re-book with confirmations,
conflict notice), an Edit link on the invoice page, New invoice in the
sidebar and the empty list; `packages/ui` gains `Combobox` and Ledgeline's
`AccountInput`. Router tests run the whole path in process (create, a
discount, a subtotal and a tax row, save, send, the journal text; conflicts;
a sent invoice's re-book and the closed period; void, re-book, deposits, a
new customer; the unsaved preview behind the gate). The e2e spec
`invoicer/web/e2e/editor.e2e.ts` is written, not run; `playwright.config.ts`
runs it after `invoices.e2e.ts` (one books copy; since 4b, `customers.e2e.ts`
runs before both, while the balances are the fixture's). Decided while building,
open to veto:

| #   | Decision                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
|-----|-----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| E1  | **The editor sends a form, never markdown.** `GET /api/invoices/{num}/form` gives the header, the custom fields, the bill table as a grid and the prose; the server makes the text from the file as it is (`invoicer_core::editor::apply`): only changed keys are edited, the grid is written aligned with every computed amount recomputed, prose is replaced only when it changed. An unchanged form is the file byte for byte.                                                                                                                                                                                                                                     |
| E2  | **The grid is raw markdown cells under the table's own headers**, so nothing is lost (Code, Unit, Date and unknown columns, `**Subtotal**` labels, `\|`). Roles are always read from the headers again; the form's `role` is informative.                                                                                                                                                                                                                                                                                                                                                                                                                             |
| E3  | **New invoice writes the draft at once** through `new` (customer, template, date at `/app/new`), then opens `/invoices/{n}/edit`: drafts have numbers and never touch the books. 4a has no delete: an unwanted draft is removed by hand.                                                                                                                                                                                                                                                                                                                                                                                                                              |
| E4  | **Save is `PUT /api/invoices/{num}/form` over the revision read** (the content hash): a file changed since is `409 changed` with its current revision and is never written. The editor ignores its own save coming back, adopts an outside change silently when nothing is unsaved, and otherwise shows the conflict: Reload theirs, or Keep mine (an explicit save over their revision). The books write keeps its own guard (`retry_once` on `external-change`, as the CLI); the books generation isn't sent.                                                                                                                                                       |
| E5  | **Saving a sent invoice re-books it**: only with `rebook: true` (the confirmation, `409 confirm-rebook` without), refused before the file changes when dated on or before `closed-through` (`check_rebook`), then `rebook`, and the `paid:` mirror refreshed. A void invoice isn't edited (`409 void`).                                                                                                                                                                                                                                                                                                                                                               |
| E6  | **The number follows `set`**: a draft's only, never one in use. The file name and title stay as `new` wrote them (no rename), as with `set`.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| E7  | **Changing the customer re-snapshots `invoicee_*`** from its note. The SPA copies the customer's terms, and its template when the invoice has none; the due date follows date + terms until edited by hand (a `null` due date is written as date + terms).                                                                                                                                                                                                                                                                                                                                                                                                            |
| E8  | **Unsaved previews are pages, not `srcdoc`** (F1): `POST /api/preview/draft` keeps the text server-side under a random id (the 16 newest, 10 minutes, dropped on File → Open) and the frame loads `/.ledgeline-invoicer/preview?draft=<id>` behind MBR's gate. Posted 400 ms after typing stops.                                                                                                                                                                                                                                                                                                                                                                      |
| E9  | **The Books panel previews unsaved text**: `POST …/compute` with `books: true` runs `InvoiceBooks::preview_of` on the session's warm journal (would-book for a draft; booked and in step for a sent one, with Re-book). Amounts are computed every 120 ms of quiet, books every 500 ms; a stale answer is dropped.                                                                                                                                                                                                                                                                                                                                                    |
| E10 | **TypeScript arithmetic is display-only** (`lib/arithmetic.ts`, BigInt decimals, half away from zero), tested against the Rust goldens (§3.3's worked example, 105, 108). While a compute is pending the grid's own estimate shows muted and italic, marked "calculating"; the server's numbers replace it.                                                                                                                                                                                                                                                                                                                                                           |
| E11 | **Row types** insert after the focused row, never below the final total (else just above it): Item; Discount % (`-10%`); Discount $ (`-$0.00`); Subtotal (`**Subtotal**` in the rate column); Tax (`Sales tax`, `0%`, no account assumed); Spacer; Apply deposit, from `GET …/deposits` (the server's suggested row, all that is left, the deposit account), inserted client-side, since core `apply_deposit` writes the file under unsaved edits. Each adjustment names its base inline ("-10% of Senior architect hours, $8,000.00"). Reorder: a drag handle, Alt+↑/↓, Move up/down buttons.                                                                        |
| E12 | **Catalog and books cells.** A catalog pick fills code, description, unit, rate and account, adding a Code (and Unit) column when the table lacks one. The books cell is Ledgeline's account picker, now in `packages/ui` (`AccountInput`, `accountMatch`, `TYPING_ATTRIBUTE`; web/ imports them from there), plus the invoicer's own deferral popover writing `a → b, N months [from D] [calendar]`; its deferred account starts as the row's account (no assumed default). A `through` term is edited as cell text. Ledgeline's keymap and its plan 45 schedule form weren't extracted: the editor's keys are local, and the schedule form edits a different model. |
| E13 | **Add new customer is the customers API** (`POST /api/customers`, 4b's `customer_edit::create`, so K3/K4's naming rules): the dialog asks a name, person or organization, a billing email and address, terms, and sends only those filled in (4b's `newCustomerFields`); the answer's `link` goes into `customer:`. 4a's own `POST /api/editor/customer` and `invoicer_core::customer_note` were dropped when the branches merged: one way to create a customer. The template combobox has no Add new (4d).                                                                                                                                                           |
| E14 | **Prose fields are MBR 0.7.4's M6 `<mbr-markdown-field>`**, loaded from `/.mbr/components/mbr-markdown-field.min.js` after the session cookie, with `catalog-url=/.mbr/site.json`; a labelled textarea when it doesn't load (component tests, an older server). No CodeMirror.                                                                                                                                                                                                                                                                                                                                                                                        |
| E15 | **Lifecycle buttons** are disabled while anything is unsaved ("Save first"). Send… (drafts) shows the total, the receivable and the would-book text, with how it was sent; Void… asks for the date (a reversing entry in accrual books, the markdown only for a draft or in cash books); Re-book… (sent) says the transaction is replaced. PDF is the invoice page's Send panel, reused (`SendSection`).                                                                                                                                                                                                                                                              |
| E16 | **Custom fields**: keys autocomplete from those used on documents, values from `GET /api/fields/{key}/values`. Text, numbers and booleans are editable; lists and maps show read-only and go back unchanged.                                                                                                                                                                                                                                                                                                                                                                                                                                                          |

**Pending (the lead):** `just invoicer-e2e` (the editor spec: 2005 for
Customer X with ARCH, -10%, Subtotal, 8.25% tax to `liabilities:sales-tax`,
$7,794.00; save; send; the journal; the PDF; and an outside edit to 108
while editing). In the window: the M6 markdown field in WKWebView (its
`change` event, wikilink completion), the combobox and account popups inside
the dialogs, a pointer drag, the preview frame's reload on each pause, and
the leave-with-unsaved-changes prompt. The 10 ms compute budget is
unmeasured in a release build.

**4b. Customers:**
- A list from the `customer` tag.
- A form with the contact fields in MBR's schema, plus the invoicing keys.
- An MBR contact-card preview.
- Backlinked invoices (M1), AR balance and last payment.
- Creating a customer writes a new person or organization note in
  `customers-dir`.

*4b built (2026-11-24).* Core: `invoicer_core::customer_edit` (`create`,
`update`, `fields_of`, `CustomerFields`, `CustomerSaved`) and
`views::account` (`customer_accounts`, `customer_account`). App:
`app/customers.rs` (routes, merged into `api::routes`) and
`app/customers_tests.rs`; `OpenBooks::mbr_site` exposes MBR's index for
backlinks. SPA: Customers in the sidebar, `/app/customers` (list),
`/app/customers/new`, `/app/customers/[...path]` (form, account, invoices,
contact card); `lib/customers.ts`; components `CustomerList`,
`CustomerForm`, `LabelledList`, `CustomerAccount`, `NoteFrame`, with unit
and component tests. e2e `invoicer/web/e2e/customers.e2e.ts`: written, not
run. The CLI has no `customers new`/`set` yet; both would call the same
core functions.

**4a's inline Add new uses it** (E13): `POST /api/customers` with
`{"title": "Northwind Traders", "kind": "organization", ...}` answers `201`
`{path, title, link, ledger_customer, kind, hash, written, changed}`; write
`link` (`[[Northwind Traders]]`) into the invoice's `customer:`. Any other
`CustomerFields` key may be sent too (`emails: [{label, value}]`, `terms`,
…); absent keys are left out of the note. The SPA's client is
`createCustomer` in `invoicer/web/src/lib/customers.ts` (a `409` throws
`ApiStatusError` with `status`).

| Route                              | Body                                          | Answers                                                                                                                                    |
|------------------------------------|-----------------------------------------------|--------------------------------------------------------------------------------------------------------------------------------------------|
| `POST /api/customers`              | `CustomerFields`: `title` and `kind` required | `201` `CustomerSaved`; `409` name taken or file exists; `422` a field that doesn't read; `400` an unknown field                            |
| `GET /api/customers/{*key}?today=` | —                                             | `customers show --json` plus `account`, `hash`, `form` (`CustomerFields`), `backlinks`                                                     |
| `PUT /api/customers/{*key}`        | `{hash, fields: CustomerFields}`              | `CustomerSaved`; `409` the note isn't the bytes `hash` names; `422` a field, or a layout the editor refuses                                |
| `GET /api/customers?today=`        | —                                             | `customers list --json` rows, each plus `account: {balance: [{commodity, amount}], overdue, oldest_overdue, last_payment: {date, amount}}` |

`CustomerFields` (every key optional; absent leaves the key, `""`, `[]`
or `null` removes it): `title`, `kind` (`person`|`organization`),
`emails`/`phones`/`addresses` (`[{label: string|null, value}]`, an address
as lines), `company`, `terms` (net days, `null` removes), `revenue_account`,
`invoice_template`, `ledger_customer`, `contracts` (strings),
`billing_portal` (`{name, url, login, supplier_id, requires_reference,
instructions}` or `null`). Unknown keys are a `400`.

Decided while building, open to veto:

| #   | Decision                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
|-----|---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| K1  | **Edits are by key, and only what changed.** The SPA sends only the fields changed since the note was read; the core (`customer_edit::update`) also skips a named key whose value already reads the same. So an untouched address stays MBR's map (`street`, `city`, …), a `|` block stays one, and comments, unknown keys and the body stay byte for byte (`frontmatter::edit`'s verification). An edited address is written as text (a `|` block). A layout the editor refuses (§5.5) is a `422`: edit that note by hand.                                                     |
| K2  | **Conflicts are by content hash** (`Fingerprint::token`, the note's bytes), not the books generation: a customer note isn't the journal. A save names the hash it was shown; a different file is `409`, and the write is conditional on the revision read. The page then offers Reload (discard mine) or **Apply my changes to the new version**: the same changed keys over the new text, which by K1 touches no other key. Live reload refreshes a clean page; with unsaved changes it says the note changed and keeps the form.                                              |
| K3  | **Create** writes `customers-dir/<Title>.md` (`: * ? " < >` become `-` in the file name; the title keeps them), frontmatter only, in a fixed order: `title`, `type`, `tags: [<customer-tag>]`, `emails`, `phones`, `addresses`, `company`, `terms`, `revenue_account`, `invoice_template`, `ledger_customer`, `contracts`, `billing_portal`. It never replaces a file, and refuses a title any note already answers to (customer or not), so `[[Title]]` links only the new note. An existing contact becomes a customer by adding the tag by hand (an action for it is later). |
| K4  | **A title is link-safe**: no `[ ] \| # ^ / \`, not ending in `.md`, one line, at most 120 characters.                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| K5  | **Renaming keeps history**: a new title on a note without `ledger_customer` writes the old name as `ledger_customer` (the books' `customer:` tag), unless the form sets it. The file keeps its name, so invoices' `[[Old name]]` still resolve by file name (§3.1's rules).                                                                                                                                                                                                                                                                                                     |
| K6  | **The form edits what the core reads, nothing new**: `title`, `type`, `emails`, `phones`, `addresses`, a person's `company` (a name is written as `[[Name]]`), `terms`, `revenue_account`, `invoice_template`, `ledger_customer`, `contracts` and `billing_portal`. `open_pos` and `snippets` are shown, edited by hand; `tags` is never touched. No accounting key was added.                                                                                                                                                                                                  |
| K7  | **The portal** is written over the note's own map: keys the form doesn't know stay (in their order); a text differing only in trailing white space stays as written; `requires_reference: false` is written only where the key already was. A request naming any other portal key (a `password`) is refused before anything is read (`400`).                                                                                                                                                                                                                                    |
| K8  | **The account** is the dashboard's: the balance sums `receivables`' open items that are one of the customer's invoices or carry its `customer:` tag (opening balances, plan 45's own), per commodity; overdue counts those past due. The last payment is the latest payment (not a credit) the books link to one of its invoices, same-day payments added up.                                                                                                                                                                                                                   |
| K9  | **The contact card is MBR's page for the saved note**, framed like the invoice (by URL, sandboxed `allow-same-origin`, B2), shown again after a save. It doesn't preview unsaved edits: that needs a page per draft (a `srcdoc` frame is blank in WebKit, F1); later, if wanted.                                                                                                                                                                                                                                                                                                |
| K10 | **Backlinks**: the invoice table is the core's `customer:` resolution (the same rules as MBR's); MBR's own backlink index (`Server::site().backlinks`, M1: body and frontmatter links) is in `backlinks`, and the page lists the notes that aren't its invoices as "Also linked from".                                                                                                                                                                                                                                                                                          |
| K11 | **URLs**: the SPA's page is `/app/customers/<the note's path without .md>`; the API's `{*key}` is the note's path (`.md` optional) or a name `customers show` takes.                                                                                                                                                                                                                                                                                                                                                                                                            |

**Also fixed:** main's `invoicer/web/src/routes/+layout.svelte` had an
unclosed `<div>`/`<main>` from the icon branch's merge (svelte-check and the
build failed); the old header was dropped, the sidebar layout kept, the icon
stays the favicon.

**Pending (the lead):** `just invoicer-e2e` (the new spec creates
Northwind Traders and deletes it after); in the window, the contact-card
frame (MBR's person/organization page with no scripts) and Apply my
changes after an outside edit in an editor that keeps the file open.

**4c. Catalog and snippets** — grid and markdown editors over their notes.

*4c built (2026-11-24).* Core: `invoicer_core::catalog_edit` (`list`,
`form_of`, `check_grid`, `apply`, `save`, `create`, `empty_catalog`) and
`invoicer_core::snippet_edit` (`list`, `create`, `update`, `delete`);
`catalog::catalog_table` is shared with `Catalog::parse`, and
`customer_edit::link_safe_title` with customers. App: `app/library.rs`
(routes, merged into `api::routes`) and `app/library_tests.rs`. SPA:
Catalog and Snippets in the sidebar, `/app/catalog` (`CatalogGrid`, reusing
the editor's `AccountInput`, `DeferralPopover`, `moveRow`/`removeRow` and
`ConflictNotice`), `/app/snippets` (`SnippetEditor`, the body in
`ProseField`, so MBR's M6 field or a textarea); `lib/library.ts`. The
invoice editor re-reads its options after each change. e2e
`invoicer/web/e2e/library.e2e.ts`: written, not run; `playwright.config.ts`
runs it last, as the `library` project after `editor` (that spec picks ARCH
as the fixture has it); it puts the catalog back and removes its snippet.

| Route                          | Body                                     | Answers                                                                                                                         |
|--------------------------------|------------------------------------------|---------------------------------------------------------------------------------------------------------------------------------|
| `GET /api/catalog`             | —                                        | `{catalogs: [{path, title, hash, table: {columns: [{header, role}], rows}, problems}], path}` (`path`: `[invoicer] catalog`)    |
| `POST /api/catalog`            | —                                        | `201` `{path, hash, written, items}`; `409` `exists`                                                                            |
| `PUT /api/catalog/{*path}`     | `{hash, table}`                          | `{path, hash, written, items}`; `409` `changed` (+ `revision`); `422` `invalid` (+ `problems: [{row, column, message}]`); `404` |
| `GET /api/snippets`            | —                                        | `[{path, title, kind, aliases, body, hash, used_by: [{path, title, how: default\|copy, number}]}]`                              |
| `POST /api/snippets`           | `{title, kind?, body?}`                  | `201` `{path, title, link, kind, hash, written, changed}`; `409` `name-taken`/`exists`; `422`                                   |
| `PUT /api/snippets/{*path}`    | `{hash, fields: {title?, kind?, body?}}` | as `POST`; `409` `changed` (+ `revision`)                                                                                       |
| `DELETE /api/snippets/{*path}` | `{hash, force?}`                         | `{deleted}`; `409` `in-use` (+ `customers`) without `force`                                                                     |

Decided while building, open to veto:

| #   | Decision                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
|-----|------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| G1  | **The catalog is edited as its table, nothing else.** The grid is the note's table as raw cells under its own headers (E2's rule); a save replaces only the table's bytes, so the frontmatter, prose, other tables and line endings stay byte for byte, and an unchanged grid is the file. A changed table is written aligned, keeping its alignments, indent and hand-widened columns while the header is the same. The page edits rows, not columns: add a column by hand.                                                                   |
| G2  | **Validation is what `check` would report, plus the CLI's grammar**: a row needs a code and a description; codes are unique case-insensitively, one word without `:`, `\|` or a backslash, at most 40 characters (`--from-catalog CODE:QTY` must name them); a rate is money in the books' style; the books cell reads (an account, or a deferral). Wholly blank rows are dropped. The SPA checks codes and descriptions itself (Save waits); the server judges rates and accounts, and its `422` `problems` (`row`, `column`) mark the cells. |
| G3  | **Conflicts are by content hash** (K2). A refused save shows the conflict notice: Reload theirs, or Keep mine, which saves the grid over their hash (a table has no keys to re-apply, unlike K2's customers). Live reload adopts an outside change when nothing is unsaved and shows the notice otherwise; the page's own save coming back isn't a conflict.                                                                                                                                                                                   |
| G4  | **Every `type: catalog` note is editable**, with a picker when there are several. With none, the page offers Create: an empty catalog at `[invoicer] catalog` (`title: Catalog`, `type: catalog`, a table headed with the item, description, unit and rate words and `*Account*`), never replacing a file.                                                                                                                                                                                                                                     |
| G5  | **The invoice editor follows the catalog live**: it re-reads `/api/editor/options` after every change event, so an item saved here (or written by hand) is offered without reopening the invoice. The catalog is read fresh on each request (`edit::catalogs`).                                                                                                                                                                                                                                                                                |
| G6  | **Snippets are edited by key, the body replaced**: `title` and `snippet_kind` through `frontmatter::edit` (comments, unknown keys and layout stay), the body after the frontmatter replaced (`\r\n` to `\n`, blank lines trimmed, one final newline). A form that reads the same writes nothing. `snippet_kind` is any word (one line, at most 40 characters; payment, terms and note are suggested); `""` removes it.                                                                                                                         |
| G7  | **A rename keeps the old title as an alias** (`aliases:`), so the customers' `[[Old title]]` still resolve, and the file keeps its name (as K5); taking an old title back drops it from the aliases. Titles follow K4 (`link_safe_title`, shared with customers), and a title another note answers to is refused (`409` `name-taken`).                                                                                                                                                                                                         |
| G8  | **New snippets go in `<templates-dir>/snippets/<Title>.md`** (K3's file-name rule), with `title`, `type: snippet` and `snippet_kind`, never replacing a file.                                                                                                                                                                                                                                                                                                                                                                                  |
| G9  | **Where a snippet is used**: the customers whose `snippets:` resolve to it (`default`: their new drafts copy it), and the documents whose text holds its body as it is now (`copy`). Snippets are copied, never linked (§3.7), so a copy made before an edit isn't found; MBR's backlinks aren't used.                                                                                                                                                                                                                                         |
| G10 | **Delete is refused while customers name the snippet** (`409` `in-use`, with their titles) unless `force`: the dialog names them and says Delete anyway. Invoices keep their copies. The hash is checked first, but the removal itself isn't conditional (the vault has no conditional remove): a narrow race, accepted.                                                                                                                                                                                                                       |
| G11 | **A path that isn't a catalog (or a snippet) is `404`** whatever the hash: the type is checked before the hash.                                                                                                                                                                                                                                                                                                                                                                                                                                |
| G12 | **No CLI yet.** `catalog` and `snippets` commands would call `catalog_edit` and `snippet_edit` as the routes do; left for when the CLI wants them (customers' `new`/`set` are in the same state).                                                                                                                                                                                                                                                                                                                                              |

**Pending (the lead):** `just invoicer-e2e` (the library spec: JR to $250
and AUDIT added and saved, AUDIT offered in 108's editor, a PENTEST row
written on disk offered there by live reload; Late fee created, its text
written through the prose field, renamed with its alias, deleted). In the
window: the M6 field in the snippets page (the spec clicks away after
typing, for its `change` event), the account popup inside the catalog grid,
and a pointer drag of a catalog row.

**4d. Templates** — a list, a code editor (CodeMirror, HTML/Tera), and live
preview against a chosen invoice.

**Tests:**
- Component tests (vitest + jsdom) for the grid's row arithmetic. They mirror
  the Rust goldens, and the TypeScript is display-only: the server is the
  source of truth for amounts.
- e2e: create a customer, create an invoice with discount, subtotal and tax,
  save, send, check the journal text, and export a PDF.

## Phase 4 feedback (2026-11-24)

Patrick used the invoice editor (4a–4c) hands-on. Fixed on
`2026-11-24-invoicer-editor-polish` (stacked on 4c): friendly problem
messages, links without brackets, Deposit for note, the template dropdown,
comboboxes that open without typing, the grid's total footer and its
subtotal, spacer and column work, new rows at the bottom, drop lines, tables
in prose, the unsaved-changes dialog, and the layout. `/api/editor/options`
gains `total_words`, `subtotal_words` and `link_notes`. Narrow windows and the
keyboard were asked for too (H11, H14, H16).

| #   | Decision                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
|-----|----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| H1  | **Problems name fields in words, in the app only.** `friendlyProblem` (`lib/labels.ts`, one `FIELD_LABELS` map) turns "`invoicee_email` is missing" into "Invoicee email is missing", a backticked `key: value` into the label and the value in quotes, other code into quotes. The core's text, the codes and `check --json` are unchanged: CLI readers edit the file and want its keys. Applied to the editor's problems and table error, the invoice page's Books problems and the catalog note's problems; the list shows counts only. The customer page (parallel work) can call the same function.                                                                                     |
| H2  | **Links are names.** The customer (editor and New invoice) and Deposit for note show the note's name and write `[[Name]]`; brackets are never typed. A name typed for the customer is always written as a link (an unknown one is `check`'s `unknown-customer`); a known note's spelling wins (`customer x` → `[[Customer X]]`). The fields keep the text as typed, so a space mid-name isn't trimmed away.                                                                                                                                                                                                                                                                                  |
| H3  | **Deposit for note** (Patrick's wording) shows only on a deposit invoice (a row charging the deposit account; a negative "less deposit" row doesn't count) or where `deposit_for` is set, with a one-line hint. Suggestions are `link_notes` from `/api/editor/options`: estimates first, then notes that aren't documents, the catalog or snippets (at most 2,000). A suggestion's name is written as a link; anything else stays free text.                                                                                                                                                                                                                                                |
| H4  | **The template is a dropdown** of the books' invoice templates and the starters, by name (`/api/templates`), plus "The books' default" (no `template:`). A `template:` naming nothing the books have stays selected, shown as the path "(not found)", so opening an invoice never changes it.                                                                                                                                                                                                                                                                                                                                                                                                |
| H5  | **Comboboxes open without typing** (`packages/ui`, every use): a click in the field, ArrowDown or the chevron shows every option, the current one highlighted, until something is typed. Not on focus: tabbing through the grid would pop a list per cell. The chevron isn't a tab stop; the grid's cells turn it off.                                                                                                                                                                                                                                                                                                                                                                       |
| H6  | **The total is the grid's footer**, not a row: its amount, and its label chosen from `[invoicer.words] total` (`total_words` in the options), since the engine reads only those words as a total; another label ("Amount owing") is added to that setting first. It can't be deleted or blanked. A table without one gets `**Total due**` with its first added row (or a label chosen), never on load, so an untouched file stays byte for byte (E1). A one-item invoice keeps its total too: one rule, and `new` writes one. The reported bugs were the total row itself: a new draft's table is only that row, so typing "in row 1" edited it, and its label sat in the narrow Rate field. |
| H7  | **A subtotal is its label and its amount**: the label a dropdown of the subtotal and total words, in the cells before the amount column; no quantity, unit or rate fields.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| H8  | **A spacer is an empty separator row** (a dashed rule) with only its handle and Move/Delete buttons.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| H9  | **New rows go at the bottom**, just above the total, whatever is focused (supersedes E11's after-the-focused-row); focus moves into the new row. The catalog grid's Add item goes last too.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| H10 | **A drag shows where the row lands**: a line above the row it would go before, or below the last row (`tr[data-drop]`, `app.css`), in both grids. Nothing lands below the total.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| H11 | **Column widths fit their content**: the description takes what's left (at least 14rem); quantity and unit 5rem; rate, amount and code 7rem; date 9rem; the account 13rem, its full text in a tooltip. A narrow window scrolls the grid instead of crushing it. **Unit and Code columns** are added with **+ Unit** / **+ Code** in the toolbar when the table lacks them (a catalog pick still adds them itself); the app never removes a column.                                                                                                                                                                                                                                           |
| H12 | **Tables in prose.** MBR's field (Milkdown Crepe) has Table in its `/` menu; a hint under the field says so. **Insert table** adds an empty two-column table: at the caret in the plain field, at the end in MBR's (its caret isn't exposed to the host). **Spreadsheet paste**: a capture-phase listener reads an HTML `<table>` (first) or tab-separated text (every line has a tab), makes a markdown table (first row the header, `\|` escaped, a `colspan` padded) and hands it back to MBR's field as a plain-text paste, which its clipboard parses as markdown; the plain field inserts it at the caret. The browser's `DOMParser`, no dependency.                                   |
| H13 | **Leaving with unsaved edits asks Save / Discard / Cancel** (a link, the sidebar, Back): Save has focus, Tab stays in the dialog, Escape cancels. Closing the window is still the browser's own prompt. Saving a sent invoice asks the re-book confirmation instead; the user goes on from there. **Discard changes** next to Save puts the form back as last saved.                                                                                                                                                                                                                                                                                                                         |
| H14 | **Save, Discard changes and the save state are top right**, as on the customer page. **Send, Void, Re-book and the Send panel (PDF) show only when nothing is unsaved** ("Save your changes to send it…" meanwhile). The page is a container: preview and Books beside the form from 80rem of its own width (`@7xl`, rem-based, so it follows the UI scale), below it otherwise; the header form goes to two columns at `@lg` and three at `@3xl`.                                                                                                                                                                                                                                           |
| H15 | **No implementation talk in the editor**: dropped the estimate tooltip ("this app's estimate until the server answers"; now "(updating)" for screen readers), "starter, copied in on save", the `[[Customer]]` and `/invoices/tpl/…` placeholders, and "writes only the markdown".                                                                                                                                                                                                                                                                                                                                                                                                           |
| H16 | **Keyboard, for the keyboard phase's shared keymap**: ⌘/Ctrl+S saves; Alt+↑/↓ moves the focused row (both grids); in a combobox ArrowDown opens the full list, ↑/↓ or Ctrl+P/N move, Enter chooses, Escape closes the list first; the total and subtotal labels are selects; dialogs: Escape cancels, the unsaved dialog focuses Save and keeps Tab inside, the confirmations focus Cancel. Everything is reachable by Tab; nothing new is bound globally.                                                                                                                                                                                                                                   |

**Pending (the lead):** `just invoicer-e2e` (editor: 2005 with the footer
total, Send only after Save; a new 108 spec pastes cells into the prose field,
then Escape, Discard changes and the dialog's Discard, the file untouched;
library: Add item last with row 1 focused). In the window: a real paste from
Excel, Numbers and Google Sheets into MBR's field in WKWebView (the hand-back
is a synthetic `ClipboardEvent`; where WebKit refuses one, the table is
appended at the end instead), Crepe's `/` → Table, the drop line during a
pointer drag, the dialog on Back (a `popstate` cancelled), the editor at a
narrow window and at zoomed UI scales.

## Phase 5 — Dashboard and lists (PR 5)

- **Dashboard:**
  - the outstanding bar (red overdue, yellow open, grey scale to $10k
    minimum, big total);
  - aging buckets;
  - oldest overdue;
  - DSO and average days to pay;
  - drafts waiting;
  - recognition due (a count, plus "Open in Ledgeline").
- **Invoice list:** filters, sort, overdue highlighting, quick jump (MBR
  search).
- **Charts:** follow the `dataviz` skill and the Ledgeline palette.
- **Tests:** API goldens on the accrual and cash fixtures, and dashboard
  screenshots for the docs.

## Phase 6 — Packaging, release, docs (PRs 6a–6b)

**6a. Release (L7, part 2):**
- **Flake:**
  - `invoicerWithSpa`, `invoicerMacApp` (`Info.plist.in`, icns,
    `Assets.car`, bundled hledger);
  - `linuxDist` gains the invoicer;
  - `checks` gains `invoicerSpaShellCsp`.
- **`ci.yml`:** the macOS job builds `.#invoicerMacApp`.
- **`release.yml`:**
  - the package matrix builds the invoicer on arm64 only (I18): sign,
    notarize, and produce `LedgelineInvoicer-<ver>-arm64.dmg`;
  - the release asserts all three DMGs exist (Ledgeline arm64 and x86_64,
    the invoicer arm64);
  - `SHA256SUMS` covers all of them;
  - crates.io publishing is unchanged: the invoicer crates stay
    `publish = false` (I19).
- **`package-crates`:** excludes the ffi crates and the invoicer crates
  (I19). `release.sh check` covers the new literals.

**6b. Docs:**
- `docs/invoicer/` in the same mbr site:
  - index;
  - files and formats (the ARCHITECTURE §3 spec, user-facing);
  - booking;
  - templates;
  - CLI reference;
  - scripting recipes, e.g. `grep -l 'paid: no'` and `hledger bal tag:invoice`.
- The README gains an Invoicer section.
- `docs/development/` gains the invoicer layout.
- `mbr -b --fail-on-broken-links` is clean. No `[!INFO]` callouts.

## Phase 7 — Later (each its own plan when started)

The order is a suggestion:

1. Estimates and convert-to-invoice.
2. Recurring invoices (`recur`).
3. Credit notes.
4. Timedot and unbilled time.
5. Stripe read-only sync.
6. QuickBooks customer CSV import.
7. Late fees.
8. Ledgeline "Open invoice" link from `document:` (L9).
9. E-invoice (UBL) export.
10. Email and reminders.
11. iOS (`invoicer-ffi`).
12. M:N settlement.
13. Multi-currency.

ARCHITECTURE §7 sketches how each fits.

## Execution

| Step | Owner                         | Scope                                         | Gate                                                       |
|------|-------------------------------|-----------------------------------------------|------------------------------------------------------------|
| 0    | Lead                          | 0a–0d (refactors touch everything; one owner) | Full `just pre-push` incl. e2e                              |
| 1    | Engineer A                    | 1a, then 1b                                   | Crate tests, proptests, clippy, fmt                          |
| 1'   | Engineer B (parallel with 1)  | 2a (Ledgeline core L1–L4)                     | `just engine-check engine-test`, `accrual_library.rs`, hledger captures |
| 2    | Engineer A                    | 1c, 2b, 2c                                    | CLI scenario tests                                           |
| 3    | Engineer B + Lead             | 3a–3c (the lead runs e2e), 3d with Patrick    | `just pre-push` + invoicer e2e                               |
| 4–5  | Engineers A/B                 | 4a–4d, 5                                      | vitest, svelte-check, lint; e2e by the lead                  |
| 6    | Lead                          | 6a, 6b (a writer agent for docs)              | Release dry run (`workflow_dispatch dry_run`)                 |

Worktree agents follow the repo's established practice:

- the lead creates each worktree from HEAD, not from main;
- the lead copies `target/debug` with `/bin/cp -Rc` so builds start warm;
- agents never run Playwright.

MBR work (MBR-CHANGES.md) runs in parallel in the MBR repo. M3, M4 and M5
gate Phase 3. M1 and M2 gate Phase 4. MBR cuts one release after all its P0
and P1 changes land (M0), and the invoicer depends on that crates.io release
from Phase 3 on (I16). Phase 1 depends on no MBR crate: `invoicer_core::notes`
does its own discovery behind an API shaped like `mbr_core::Site`, and swaps
to `Site` when the dependency arrives.

## Risks

- **The Phase 0 extraction breaks Ledgeline in subtle ways** (CSP hashes,
  theme first paint, iOS bridge). Mitigations: it is behaviour-neutral by
  rule, it gets the full e2e plus iOS build check, and the docs screenshots
  are diffed.
- **MBR timing.** Everything the invoicer needs is on MBR's main or in its
  last open PR; the risk is the release. `mbr-core` cannot be used from git
  (I16), so the MBR release (M0) gates Phase 3.
- **Arithmetic disagreement between Rust and TypeScript.** The server is the
  only calculator. The grid shows the server's numbers after a debounced
  round trip, and optimistic display is labelled.
- **Books/markdown divergence through hand edits or git merges.** The
  reconciler and `check` surface every divergence. Nothing auto-rewrites
  except the `paid:` mirror.
- **AGPL change.** It is a one-way door for contributors' expectations.
  Nobody has forked Ledgeline yet (Patrick, 2026-11-19).

## Open questions

All five were answered on 2026-11-19 (I1, I10, I11, I12, I17). New
questions go here as they come up.

### Q1 (2026-11-23): how templates are customised, in the invoicer and in MBR

**Why it's open:**

- **Whole pages today.** An invoice's `template:` names a whole page, and MBR's
  `index.html` has no `{% block %}`s to extend.
- **Copied set-up.** The chrome switch (T1–T3: `server_mode`, MBR's partials,
  the `<noscript>` rule, the Pico pins, the print reset) is copied into every
  template. A custom template without it looks bare in MBR.
- **No template.** An invoice note with no `template:` shows only its markdown
  body in MBR, and the addresses in its frontmatter never appear.

**What MBR 0.7.4 can and can't do** (read in its source):

| Fact                                                                    | Where (mbr 0.7.4)                                            |
|-------------------------------------------------------------------------|--------------------------------------------------------------|
| A note's `template:` replaces `index.html` for the whole page           | `mbr-core/src/page_template.rs`, `src/templates.rs`          |
| `.html` files beside a template can be included or extended by name     | `page_template.rs` (siblings), `templates.rs:128-150`        |
| MBR's own partials can be included as `/.mbr/<name>`                    | `templates.rs:802-806`                                       |
| MBR's `index.html` has no blocks, so extending it overrides nothing     | `templates/index.html`                                       |
| A template can't see the request, only `server_mode`                    | `mbr-server/src/server.rs:8183`, `standalone.rs:321`         |
| PDF export is marked only on the client (`window.__MBR_EXPORT__`)       | `mbr-markdown-browser/src/pdf_export/mod.rs`                 |
| The note's `type` becomes a `<body>` class (`invoice`)                  | `templates.rs:687`                                           |
| A books folder can override MBR's templates and partials with `.mbr/*.html` | `templates.rs:173-180`                                   |
| There's no default template by type or folder                           | `page_template.rs`; nothing in `config.rs`                   |

**The options:**

| Option                                        | What a user writes                                                                              | In MBR                                                                                     | MBR release? |
|-----------------------------------------------|-------------------------------------------------------------------------------------------------|--------------------------------------------------------------------------------------------|--------------|
| A. Whole-page templates (today)               | A whole page, plus the copied chrome switch                                                     | Nothing new                                                                                | No           |
| **B. A shipped base with blocks**             | `{% extends "_invoice_base.html" %}` and only the blocks they change: `theme`, `header`, `parties`, `details`, `body`, `footer` | Nothing new                                                                                | No           |
| C. CSS only                                   | Colours and fonts over a fixed structure. Too limited on its own, so it becomes B's `theme` block. | Nothing new                                                                                | No           |
| D. MBR supplies the page (M11 and M12)        | Only the invoice, `{% block invoice %}`; MBR wraps it in its own page, or a bare one             | M11: blocks in `index.html` plus a bare layout chosen by `?mbr-bare`. M12: a default template by note `type` | Yes          |

**B in detail:**

- **The base file.** The invoicer ships one versioned `_invoice_base.html`. It
  holds the chrome switch, the structure, the Pico pins, the print reset and
  the books-column rule.
- **The children.** Classic, Modern and the fixture's MyCo become children of
  10–40 lines.
- **Discovery and the starter copy.** Discovery skips `_*.html`, so the base is
  never offered in the picker. Copying a starter also copies the base.
- **Existing templates.** Whole-page templates keep working.
- **Updates.** The base is versioned (`_invoice_base.v1.html`) because the user
  owns the copy; a later fix ships as `v2`.

**To check before building B:**

- whether the preview of an unsaved template (M10) finds the base file next to
  it;
- how `.mbr/` overrides resolve when MBR is opened at a folder above the books.

**D in detail:**

- **M11.** MBR splits `index.html` into blocks (`head_extra`, `chrome_top`,
  `content`, `chrome_bottom`) and adds `/.mbr/_layout_bare.html`. It sets
  `page_layout` to the bare layout for standalone renders, for `?mbr-bare`,
  and for export, which would need to send that flag.
- **Templates under M11.** A template becomes `{% extends page_layout %}` plus
  `{% block content %}`.
- **M11 defaults.** Today's behaviour stays the default.
- **M12.** `.mbr/config.toml` gets `[page_templates] invoice =
  "/invoices/tpl/x.html"`, keyed by note `type`. It applies only when a note
  has no `template:`, which fixes how an invoice without a template looks in
  MBR.

**Ruled out:** shipping a `.mbr/index.html` override in the user's books. It
changes every note, and it depends on which folder MBR is opened at.

**Recommendation:** B now. Propose M11 and M12 for the next MBR release; once
they land, `_invoice_base.html` shrinks to `{% extends page_layout %}`, and
invoices without a template look right in MBR.

**Decision:** B (Patrick, 2026-11-23), and built on
`2026-11-23-invoicer-template-base`. M11, M12 (and M13, found building it)
are proposed in MBR-CHANGES as P2.

**The two unknowns, checked:**

| Unknown                                                              | Finding (mbr 0.7.4)                                                                                                                                                                                                                                                                                                                                                                                                                                              | What B does                                                                                                                                                                                                                                                                                                                                     |
|----------------------------------------------------------------------|------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| Does the M10 preview of an uncopied starter find the base?           | Only when the base is in the books. `render_source_with_template` reads the template's siblings from disk (`unsaved_page_template` → `read_siblings`), and nothing hands it one that isn't there: Tera says the template "is inheriting from `_invoice_base.v1.html`, which doesn't exist". Also: a sibling that extends a missing file fails Tera's inheritance check for every template in the folder (MBR registers all siblings in one `add_raw_templates`). | Books with the base: the starter extends it (the user's copy). Without: the preview folds the starter into the shipped base (Q1f); a test renders both ways and gets the same pages.                                                                                                                                                            |
| How do `.mbr/` overrides resolve when MBR is opened above the books? | MBR reads `.mbr/*.html` and `.mbr/config.toml` only at its root: the folder it was opened at, or (plain `mbr`) the nearest ancestor with `.mbr`, `.git`, `.zk` or `.obsidian` (`find_root_dir`). The books' own `.mbr/` is then ignored, and `/.mbr/_nav.html` and the rest are the root's overrides or MBR's defaults.                                                                                                                                          | Nothing to change. `extends "_invoice_base.v1.html"` is a sibling name, found beside the template whatever the root. What breaks above the books is older: a `/`-rooted `template:` (I4) is read from MBR's root, so `/invoices/tpl/x.html` isn't found there. The app always opens MBR at the books folder (`read_at_root`, no search upward). |

**Decided while building, open to veto:**

| #   | Decision                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
|-----|-----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| Q1a | **The blocks**, in page order: `settings`, `ready`, `head`, `theme`, `header`, `parties`, `details`, `body`, `footer`. Two beyond the proposal: `settings`, because Tera ignores a child's top-level `{% set %}` (a child sets `chrome` or `kind` there), and `ready` (MBR's export-ready partial), so the fixture's template can leave it out (P7). The base's comment header says what each is for: it is the user-facing doc until 6b's `docs/invoicer/` templates page. |
| Q1b | **The base's own blocks are a whole template**: the fixture's look (From and Bill to, each with address, email and phone; a dates line; a ruled bill table) plus a Draft, Void or Paid mark. So a child that overrides only `theme` still has both parties, and the fixtures' MyCo is the base as it comes (it overrides only `ready`).                                                                                                                                     |
| Q1c | **Classic and Modern keep their markup** (their own `header` and `parties`, with the dates inside `parties`; `details` is empty), so their pages stay exactly as they were. That makes them about 200 lines, mostly their `theme` CSS, rather than Q1's 10–40.                                                                                                                                                                                                              |
| Q1d | **The base is copied with a starter**, first, create-new, and only when that starter is copied now (`new` and the editor's New invoice, `set` and an editor save, Use this template, a hand edit, opening the books). A base already there is the user's and stays; a starter already there doesn't bring back a base the user removed.                                                                                                                                     |
| Q1e | **Discovery** skips `_*.html` (never in the picker or `/api/templates`, never `unknown-template`) and counts a file that extends `_invoice_base…` as a template even when it never names `invoice_num`. An invoice whose `template:` names a base is told to name a template that extends it.                                                                                                                                                                               |
| Q1f | **The preview's fold**: a starter not copied in, in books without the base, is rendered as the shipped base's text with the starter's blocks put in (`starters::fold_into_base`, a scan of the tags, not a Tera parse). It covers only the shipped starters: top-level blocks, no `super()`.                                                                                                                                                                                |
| Q1g | **PDF readiness (P7) follows `extends`**: `pdf::signals_ready` reads the `ready` block, else the parent beside the template, so Classic and Modern still print on MBR's signal and the fixture's template on `load`. Reading only the child's text would have printed every child on `load`.                                                                                                                                                                                |
| Q1h | **`extends` is the second line** of a child, right after the `{# invoicer: … #}` line: Tera allows only one comment before `extends` (two, with a newline between, fail to parse).                                                                                                                                                                                                                                                                                          |
| Q1i | **The fixtures carry the base** (`invoices/tpl/_invoice_base.v1.html`, both books), byte for byte the shipped one (a test).                                                                                                                                                                                                                                                                                                                                                 |

**Before and after.** Rendered for 101, 103, 104, 105 and 108 (accrual)
and 201, 204 and 207 (cash), through each template, as a preview and as
MBR's page:

| Template | Difference                                                                                                                                                                                                                                                                                                                                            |
|----------|-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| Classic  | None: the same markup (whitespace aside) and the same CSS, with two colours now variables (`--desk`, `--paper`).                                                                                                                                                                                                                                      |
| Modern   | None, as Classic.                                                                                                                                                                                                                                                                                                                                     |
| MyCo     | Intended: the status mark (101, 104 and 201 Paid, 103 Void, 108 and 204 Draft); `box-sizing: border-box`, so the sheet is 8.5in wide on screen with its padding (it was 8.5in plus padding); `print-color-adjust: exact`, so the header and total row shading print; the desk colour behind the sheet in MBR's no-script frame covers the whole page. |

**Pending (the lead):** the e2e (no spec changes: the frame texts it checks
are unchanged; MyCo's 108 now also shows "Draft"), a look at 105 and 108
through each template in the window and in MBR, and one real PDF of a
Classic invoice (`cargo test ... --ignored pdf_`), to see it still waits
for MBR's signal.
