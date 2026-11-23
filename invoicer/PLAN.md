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

**4b. Customers:**
- A list from the `customer` tag.
- A form with the contact fields in MBR's schema, plus the invoicing keys.
- An MBR contact-card preview.
- Backlinked invoices (M1), AR balance and last payment.
- Creating a customer writes a new person or organization note in
  `customers-dir`.

**4c. Catalog and snippets** — grid and markdown editors over their notes.

**4d. Templates** — a list, a code editor (CodeMirror, HTML/Tera), and live
preview against a chosen invoice.

**Tests:**
- Component tests (vitest + jsdom) for the grid's row arithmetic. They mirror
  the Rust goldens, and the TypeScript is display-only: the server is the
  source of truth for amounts.
- e2e: create a customer, create an invoice with discount, subtotal and tax,
  save, send, check the journal text, and export a PDF.

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
