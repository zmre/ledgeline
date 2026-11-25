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

| Policy                                             | Setting                                                        | Default                                                                                                                                                                                            | Alternatives                                                                                | State                        |
|----------------------------------------------------|----------------------------------------------------------------|----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|---------------------------------------------------------------------------------------------|------------------------------|
| Cash books: a deposit's payment                    | `cash-deposits`                                                | `income-on-receipt`                                                                                                                                                                                | `liability-until-final` (held until the final invoice)                                      | Built (2b)                   |
| Cash books: a payment over an invoice's rows       | `payment-allocation`                                           | `match-rows` (each account ends at its row, deposits counted)                                                                                                                                      | `pro-rata`; planned: `oldest-row-first`                                                     | Built (2b), one planned      |
| Cash books: the revenue account a deposit lands in | —                                                              | the row's account, else the customer's `revenue_account`, `[invoicer] revenue-account`, the single revenue account                                                                                 | planned: a per-deposit-invoice override (frontmatter or the row's books cell)               | Planned                      |
| Cash books: a deferred row's payment               | —                                                              | into the account it recognizes into                                                                                                                                                                | planned: `cash-deferred-rows = "defer"` (modified cash: keep the deferral)                  | Planned                      |
| Discounts with a blank books cell                  | `discount-account`                                             | pro rata over the revenue lines                                                                                                                                                                    | a contra-revenue account                                                                    | Built (1a)                   |
| Sending into a closed period                       | —                                                              | refused                                                                                                                                                                                            | planned: book it dated the first open day                                                   | Planned                      |
| Editing a sent invoice                             | —                                                              | replace the transaction in place (§4.5)                                                                                                                                                            | planned: reverse and re-book, for an audit trail of every change                            | Planned                      |
| The void's date                                    | `void --date`                                                  | today                                                                                                                                                                                              | any date from the invoice's on, outside the closed period                                   | Built (2b)                   |
| Which deposit an application uses                  | —                                                              | the one its row names, then the oldest                                                                                                                                                             | planned: oldest only, or ask                                                                | Planned                      |
| Voiding a final invoice that applied a deposit     | —                                                              | the deposit is open again for a reissue                                                                                                                                                            | planned: keep it applied (forfeited)                                                        | Planned                      |
| Who a person at a company bills                    | `ledger_customer` on the person                                | the company's customer note                                                                                                                                                                        | the person's own `ledger_customer`                                                          | Built (2b)                   |
| When an invoice books at all                       | `[books] basis`                                                | plan 45's gate (accrual when it has receivable accounts)                                                                                                                                           | `cash`, `accrual`                                                                           | Built (plan 45)              |
| What the `paid:` mirror counts as paid (accrual)   | —                                                              | every payment and credit linked to the receivable (`match:`); paid on the date of the one that completes it                                                                                        | planned: a credit note or write-off mirrored as its own state                               | Planned                      |
| Recognition due in a closed period                 | `recognize --include-closed`                                   | left unposted (listed), as Ledgeline leaves it                                                                                                                                                     | one catch-up adjustment per schedule, dated today                                           | Built (2c)                   |
| The DSO window                                     | `dso-days`; `receivables --dso-days`                           | 90 days                                                                                                                                                                                            | any number of days                                                                          | Built (2c; the setting 5a)   |
| Aging buckets                                      | `aging-buckets`                                                | `[30, 60, 90]`: Ledgeline's Current, 1–30, 31–60, 61–90, 91+ days past due                                                                                                                         | one to eight edges; planned: in `ledgeline-core`, so `receivables` and Ledgeline share them | Built for the dashboard (5a) |
| The dashboard's paid window                        | `paid-window-days`; the page's period selector (`?paid_days=`) | 30 days                                                                                                                                                                                            | 7, 30, 90 or 365 on the page; any 1–3660 in the setting                                     | Built (R14)                  |
| What "paid in the last N days" counts              | —                                                              | money received, by its date: accrual, payment settlements of any receivable; cash, the payments applied to an invoice number, gross of a fee booked apart; credits and voids aren't money in (R14) | planned: count credits too                                                                  | Default built, one planned   |
| A draft's amounts after an outside edit            | `recalc-on-save`                                               | `drafts`: the app recalculates a draft once the edit settles (a sent invoice never: its amount is booked)                                                                                          | `never` (the page shows the mismatch and a Recalculate button)                              | Built (feedback F10)         |

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
| 5k    | Keyboard-first                         | Ledgeline's keymap in `packages/ui`, app-wide and list/editor keys | 4, 5                    |
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

| #   | Decision                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
|-----|-----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| H1  | **Problems name fields in words, in the app only**, through the one `lib/labels.ts` shared with J10 (`FIELD_LABELS`, `fieldLabel`, `fieldList`, `friendlyMessage`; the two branches' modules merged). A backticked known key becomes its label ("`invoicee_email` is missing" → "Invoicee email is missing", lower case mid-sentence), a backticked `key: value` the label and the value in quotes, other backticked code is quoted, and a bare known key before a colon is labelled. The core's text, the codes and `check --json` are unchanged: CLI readers edit the file and want its keys. Applied to the editor's problems and table error, the invoice page's Books problems and the catalog note's problems (and J10's customer pages); the list shows counts only. |
| H2  | **Links are names.** The customer (editor and New invoice) and Deposit for note show the note's name and write `[[Name]]`; brackets are never typed. A name typed for the customer is always written as a link (an unknown one is `check`'s `unknown-customer`); a known note's spelling wins (`customer x` → `[[Customer X]]`). The fields keep the text as typed, so a space mid-name isn't trimmed away.                                                                                                                                                                                                                                                                                                                                                                 |
| H3  | **Deposit for note** (Patrick's wording) shows only on a deposit invoice (a row charging the deposit account; a negative "less deposit" row doesn't count) or where `deposit_for` is set, with a one-line hint. Suggestions are `link_notes` from `/api/editor/options`: estimates first, then notes that aren't documents, the catalog or snippets (at most 2,000). A suggestion's name is written as a link; anything else stays free text.                                                                                                                                                                                                                                                                                                                               |
| H4  | **The template is a dropdown** of the books' invoice templates and the starters, by name (`/api/templates`), plus "The books' default" (no `template:`). A `template:` naming nothing the books have stays selected, shown as the path "(not found)", so opening an invoice never changes it.                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| H5  | **Comboboxes open without typing** (`packages/ui`, every use): a click in the field, ArrowDown or the chevron shows every option, the current one highlighted, until something is typed. Not on focus: tabbing through the grid would pop a list per cell. The chevron isn't a tab stop; the grid's cells turn it off.                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| H6  | **The total is the grid's footer**, not a row: its amount, and its label chosen from `[invoicer.words] total` (`total_words` in the options), since the engine reads only those words as a total; another label ("Amount owing") is added to that setting first. It can't be deleted or blanked. A table without one gets `**Total due**` with its first added row (or a label chosen), never on load, so an untouched file stays byte for byte (E1). A one-item invoice keeps its total too: one rule, and `new` writes one. The reported bugs were the total row itself: a new draft's table is only that row, so typing "in row 1" edited it, and its label sat in the narrow Rate field.                                                                                |
| H7  | **A subtotal is its label and its amount**: the label a dropdown of the subtotal and total words, in the cells before the amount column; no quantity, unit or rate fields.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| H8  | **A spacer is an empty separator row** (a dashed rule) with only its handle and Move/Delete buttons.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| H9  | **New rows go at the bottom**, just above the total, whatever is focused (supersedes E11's after-the-focused-row); focus moves into the new row. The catalog grid's Add item goes last too.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| H10 | **A drag shows where the row lands**: a line above the row it would go before, or below the last row (`tr[data-drop]`, `app.css`), in both grids. Nothing lands below the total.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| H11 | **Column widths fit their content**: the description takes what's left (at least 14rem); quantity and unit 5rem; rate, amount and code 7rem; date 9rem; the account 13rem, its full text in a tooltip. A narrow window scrolls the grid instead of crushing it. **Unit and Code columns** are added with **+ Unit** / **+ Code** in the toolbar when the table lacks them (a catalog pick still adds them itself); the app never removes a column.                                                                                                                                                                                                                                                                                                                          |
| H12 | **Tables in prose.** MBR's field (Milkdown Crepe) has Table in its `/` menu; a hint under the field says so. **Insert table** adds an empty two-column table: at the caret in the plain field, at the end in MBR's (its caret isn't exposed to the host). **Spreadsheet paste**: a capture-phase listener reads an HTML `<table>` (first) or tab-separated text (every line has a tab), makes a markdown table (first row the header, `\|` escaped, a `colspan` padded) and hands it back to MBR's field as a plain-text paste, which its clipboard parses as markdown; the plain field inserts it at the caret. The browser's `DOMParser`, no dependency.                                                                                                                  |
| H13 | **Leaving with unsaved edits asks Save / Discard / Cancel** (a link, the sidebar, Back), with J3's `SaveDiscardDialog` (one component for both pages): Cancel has focus, the safe choice, Tab and Shift+Tab stay in the dialog, Escape cancels. Closing the window is still the browser's own prompt. Saving a sent invoice asks the re-book confirmation instead; the user goes on from there. **Discard changes** next to Save puts the form back as last saved.                                                                                                                                                                                                                                                                                                          |
| H14 | **Save, Discard changes and the save state are top right**, as on the customer page. **Send, Void, Re-book and the Send panel (PDF) show only when nothing is unsaved** ("Save your changes to send it…" meanwhile). The page is a container: preview and Books beside the form from 80rem of its own width (`@7xl`, rem-based, so it follows the UI scale), below it otherwise; the header form goes to two columns at `@lg` and three at `@3xl`. Narrow windows are J12's standing requirement; this is the editor's part of it.                                                                                                                                                                                                                                          |
| H15 | **No implementation talk in the editor**: dropped the estimate tooltip ("this app's estimate until the server answers"; now "(updating)" for screen readers), "starter, copied in on save", the `[[Customer]]` and `/invoices/tpl/…` placeholders, and "writes only the markdown".                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| H16 | **Keyboard, for the keyboard phase (5k, J13: Ledgeline's keymap extracted into `packages/ui`)** to fold in: ⌘/Ctrl+S saves; Alt+↑/↓ moves the focused row (both grids); in a combobox ArrowDown opens the full list, ↑/↓ or Ctrl+P/N move, Enter chooses, Escape closes the list first; the total and subtotal labels are selects; dialogs: Escape cancels, focus starts on Cancel, the Save/Discard dialog keeps Tab inside. Everything is reachable by Tab; nothing new is bound globally.                                                                                                                                                                                                                                                                                |

**Pending (the lead):** `just invoicer-e2e` (editor: 2005 with the footer
total, Send only after Save; a new 108 spec pastes cells into the prose field,
then Escape (focus on Cancel), Discard changes and the dialog's Discard, the file untouched;
library: Add item last with row 1 focused). In the window: a real paste from
Excel, Numbers and Google Sheets into MBR's field in WKWebView (the hand-back
is a synthetic `ClipboardEvent`; where WebKit refuses one, the table is
appended at the end instead), Crepe's `/` → Table, the drop line during a
pointer drag, the dialog on Back (a `popstate` cancelled), the editor at a
narrow window and at zoomed UI scales.

### Round 2 (2026-11-24): counts, New, empty states

Patrick's next pass, on the same branch: sidebar counts that overlapped,
a customer's overlapping invoice lists, a next number that looked like a
bug, a real New button, and four different empty states.

| #   | Decision                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
|-----|---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| H17 | **Sidebar counts say what they count, and don't overlap.** Invoices shows overdue (red badge) and open-but-not-yet-due (quiet number), exclusive: 2 overdue + 1 not yet due, not "2" and "3 open". Each is shown only when more than none; each carries its words for screen readers and as a tooltip ("2 overdue", "1 open, not yet due"). **Drafts are counted in neither**: no third number (clutter), the link's tooltip lists them ("… · 1 draft, not counted"). Customers, Catalog and Snippets show their totals with the noun ("6 customers", "4 items", "2 snippets"), zero included. `/api/books` is unchanged (`open` still includes overdue; the SPA subtracts, `invoiceCounts`).                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| H18 | **A customer's invoices are Open, then Other** (paid, drafts, void, …), never overlapping; replaces J4's Open + All. Open always has its heading once the customer has any invoice ("Nothing open." when empty); Other shows when it has rows. Testids `customer-open-invoice-row`, `customer-other-invoice-row`.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| H19 | **The proposed number says where it comes from**; the rule is unchanged (never reuse a booked number). Core: `numbering::explain_next_number` → `NextNumber {number, after, source}` with `NumberSource::{Document, Books, Start}` (a number both filed and booked counts as the document's); `DocumentIndex`/`Session::explain_next_number`. Wire: `/api/books` adds `nextNumberAfter`, `nextNumberSource`; `/api/editor/options` adds `next_number_after`, `next_number_source`; `next-number --json` adds `after`, `source` (the plain output stays the number alone, for scripts). New invoice reads "It will be number 2005 (next after 2004, booked in the journal)", or "(next after 108)", or "(the first number)". New is the only place a number is proposed today.                                                                                                                                                                                                                                                                                                                                                                                       |
| H20 | **A New split button at the top of the sidebar** (under the app's name; in the narrow top bar it sits at the right of the name row, its menu right-aligned so it stays on screen). The main part is New invoice; the arrow opens a menu: New invoice, New customer, New catalog item (`/catalog?new=1`: adds a row at the bottom, focused; writes the catalog note first when there is none), New snippet (`/snippets?new=1`: opens the form). The pages drop `?new=1` after acting on it (a reload doesn't repeat it), and a same-page query change isn't "leaving" for the unsaved-changes prompt. **No estimates or credit notes**: `DocType` has them, but they are "later": `POST /api/invoices` creates invoices only and nothing lists, counts or edits the other types, so they join the menu when they work end to end. The old "New invoice" section link is gone. Keyboard (`NewMenu.svelte`, WAI-ARIA menu button): ↓ on either part or Enter/Space on the arrow opens on the first item (↑ on the last); ↑/↓ wrap, Home/End, Enter follows, Space too, Escape closes with focus back on the arrow, Tab closes and moves on; a click outside closes it. |
| H21 | **One empty state** (`EmptyState.svelte`): a centred card, a title ("No invoices yet"), one sentence, a centred primary button (a link to a page, or an action in place), for Invoices, Customers, Catalog and Snippets. The invoices one has no terminal instructions. With items, each list's New button is top right and primary: New invoice (new, on the list), New customer, New item (catalog: right of Save; the grid's own Add item stays at its foot), New snippet (was outline). While the list is empty the header has none, so there's never two. The catalog's empty button is New item, not "Create the catalog": it writes the note and adds the first row in one step. A catalog that exists with no items still shows its grid ("No items yet." row): that is where items go.                                                                                                                                                                                                                                                                                                                                                                     |
| H22 | **An empty books fixture**, `invoicer/fixtures/empty/` (`books.journal` with account declarations and nothing booked, `ledgeline.toml` with `[invoicer]` and `number-start = 1001`, `company/My Company.md`). Not caught by `.gitignore`: `/fixtures/**/ledgeline.toml` is rooted at the repo's `fixtures/`, and no file ends `-journal`; flake.nix's src filter keeps every `/fixtures/` path. `e2e/empty.e2e.ts` runs over a scratch copy on a second server (:5198, its own Playwright project, alongside the others) and loads the SPA that server embeds (vite preview proxies only to :5199), skipping loudly on the placeholder SPA as the CSP spec does. Router test `empty_books_answer_every_list_empty`; the session test checks the start number.                                                                                                                                                                                                                                                                                                                                                                                                       |

**Pending (the lead):** `just invoicer-e2e`: the new `empty.e2e.ts` (its own
server on :5198; check the port is free), and the changed assertions in
`customers.e2e.ts` (Open/Other rows, "6 customers", "2 overdue" and "1 open,
not yet due") and `editor.e2e.ts` (New invoice from the split button's main
part, "next after 2004, booked in the journal"). In the window: the New menu
by keyboard and pointer at a narrow width and at 150–200 % scale; the
catalog's New item from the menu while the catalog has unsaved edits.

## Phase 4 feedback (2026-11-24) — customers & chrome

Patrick used the 4b customer pages and the app chrome. Fixed on
`2026-11-24-invoicer-customer-polish` (stacked on 4c):

| #   | Patrick saw / asked                                                                           | What changed, and why                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
|-----|-----------------------------------------------------------------------------------------------|---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| J1  | Show a customer read-only; Edit for the form.                                                 | **The customer page opens as a view** (`CustomerView`): name, kind, labelled emails, phones and addresses, a person's company, terms, the template by name, revenue account, name in the books, the portal, contracts, open POs, default snippets; beside it the account (`CustomerAccount`). **Edit** swaps in the form; Save and Discard sit top right; Cancel returns to the view, and a save does too. Replaces K9: MBR's contact-card frame is gone from the page.                                                                                                                                                                                                                                                                     |
| J2  | On the organization form the Name field overlaps Kind.                                        | daisyUI's `.label` is `white-space: nowrap`, so Name's help text ran under the Kind column. Help texts now wrap (`whitespace-normal`), every field is `min-w-0`, the kind radios wrap, labelled-list rows shrink, and the grid is one column below `sm` (a container query, plan 39). Person and organization alike.                                                                                                                                                                                                                                                                                                                                                                                                                        |
| J3  | Unsaved changes: prompt like the invoice editor.                                              | Leaving the page (a link, the sidebar) or Cancel with unsaved changes opens **Save / Discard / Cancel** (`SaveDiscardDialog`; focus on Cancel, Escape cancels). Save writes then goes on; a refused save keeps the dialog with the reason. Closing or reloading the window gets the browser's own prompt.                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| J4  | Open invoices, and the full list.                                                             | `CustomerAccount` lists the open invoices first, then every invoice (number, date, due, total, status), each opening the app's invoice page; balance, overdue and the last payment above.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| J5  | Adding a contract needs the exact name.                                                       | **Add contract…** opens `ContractPicker`. In the window it offers **Choose a file…**: the system's open dialog (rfd, new native capability `pick_file`, macOS window only) at the books folder, `POST /api/files/pick`. A file outside the books folder (after resolving links) is refused, saying to move or copy it in. In a browser (`--server`) the picker searches `GET /api/files`: every note except invoices and other documents, plus documents and scans (pdf, doc(x), odt, pages, rtf, txt, images, spreadsheets), skipping hidden and ignored folders and `pdf-dir`, at most 50,000 entries walked. A pick writes `[[Title]]` for a note whose title links only it, else the `/`-rooted path (`customer_edit::contract_entry`). |
| J6  | No raw `[[…]]` anywhere on customer pages.                                                    | Names, never syntax: the company is typed or picked by name (organization customers suggested) and the server writes the link (K6); contracts show `ContractView.name` (the note's title or the file's name); default snippets and templates by name; server refusals no longer quote `[[…]]` ("other notes link to it by name").                                                                                                                                                                                                                                                                                                                                                                                                           |
| J7  | Opening a contract.                                                                           | A note opens in the app's note view (J9). A file opens in its viewer in the window (`POST /api/files/open`, a path inside the books folder only, capability `open_file`), else in a new browser tab at MBR's URL for it (behind the session cookie).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| J8  | Remove implementation detail shown to users.                                                  | Swept customer, catalog and snippet pages and the chrome: "MBR's page for the saved note…", "Invoices link it as [[Name]]", "Edit `open_pos:` in the note itself", "a note with `type: catalog`", "Create invoices/catalog.md", "A customer's `snippets:` names…", "Customers link it as [[Title]]", "Also answers to [[…]]" now say what they mean in plain words.                                                                                                                                                                                                                                                                                                                                                                         |
| J9  | "Also linked from": Fred Smith labelled a company; a click left the app.                      | The label was the linking frontmatter key (`company:`). Backlinks now carry the note's `kind` (its `type:`), a document's `number` and `customer`; the page shows the type (Person, Organization, Invoice, Contract…). Links stay in the app: an invoice opens its page, a customer its page, any other note **`/app/notes/<path>`**, MBR's page in the sandboxed frame under the sidebar with Back. The frame runs no scripts, so its links move only the frame.                                                                                                                                                                                                                                                                           |
| J10 | Friendly field names in messages.                                                             | `invoicer/web/src/lib/labels.ts`: one flat `FIELD_LABELS` map (customer and invoice keys), `fieldLabel`, `fieldList`, `friendlyMessage` (a known key in backticks, or before a colon, becomes its label). Customer pages run save errors and problems through it; Saved lists labels ("Saved: Phones, Terms."). Core messages say "supplier ID", "URL", "an email" instead of keys.                                                                                                                                                                                                                                                                                                                                                         |
| J11 | Status area: "9 documents" is unclear; show the books' name; the basis badge looks clickable. | "N documents" is gone. Counts sit beside the sidebar items from `GET /api/books?today=`: Invoices (open, with overdue as a red badge), Customers, Catalog (items), Snippets. The foot shows the books' name from Ledgeline's own source (`ledgeline_core::title::journal_title`: the main file's leading comment if it reads as a title, else its folder), the journal file and the basis as plain text, no border.                                                                                                                                                                                                                                                                                                                         |
| J12 | Narrow widths now, mobile later.                                                              | **A standing requirement for every page from now on**: usable in a narrow window, no overlapping fields, tables scroll in their own box. Breakpoints are the shared container queries (plan 39: `sm`–`2xl` follow the UI scale); media queries only for true viewport facts. Below `md` the sidebar becomes a bar across the top, sections in a wrapping row, the books' name beside the app's. A narrow-window e2e covers the customer form.                                                                                                                                                                                                                                                                                               |

New routes:

| Route                         | Body     | Answers                                                                                                   |
|-------------------------------|----------|-----------------------------------------------------------------------------------------------------------|
| `GET /api/files[?list=false]` | —        | `{pick, open, files: [{path, name, kind, entry}]}`; `list=false`: the capabilities only                   |
| `POST /api/files/pick`        | —        | `{cancelled: true}` or `{cancelled: false, file}`; `422` a file outside the books folder; `503` no window |
| `POST /api/files/open`        | `{path}` | `{done: true}`; `404` not a file in the books; `422` a link out of the folder; `503` no window            |
| `GET /api/books?today=`       | —        | adds `title`, `invoices: {all, drafts, open, overdue}`, `catalog` (items), `snippets`                     |

`GET /api/customers/{key}` adds `contracts[].name` and `backlinks[].kind`,
`.number`, `.customer`.

Planned from this round (open to veto):

| #   | Decision                                                                                                                                                                                                                                                              |
|-----|-----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| J13 | **Keyboard-first** is its own phase (5k): the app should make the user fast, fully on the keyboard where possible. Reuse Ledgeline's keymap (`web/src/lib/keys/`: layers, chords, the help sheet) by **extracting it into `packages/ui`**, not building a second one. |
| J14 | **Mobile later** (Phase 7): the invoicer follows Ledgeline's iOS approach (plan 42, `apple/ios`) when it comes; until then narrow windows (J12) are the bar.                                                                                                          |
| J15 | **Sorting and filtering both lists** (invoices and customers) are Phase 5 work, in the URL so a filtered list can be linked and survives reload.                                                                                                                      |
| J16 | **Managing a customer's open POs in the app** is Phase 5 (today read-only, edited by hand): add, edit and close POs on the customer page, written by key like K1, and the invoice editor's reference suggestions follow.                                              |

**Pending (the lead):** `just invoicer-e2e` (customers spec: the view,
in-app links and the note view, Edit/Cancel, a narrow window, adding a
contract by search, the Save / Discard / Cancel prompt; library spec's
alias text). In the window: **Choose a file…** (the rfd dialog over the
app; a file inside and one outside the books folder), opening a PDF
contract in Preview, the note view's frame, and the sidebar at a narrow
width and at 150–200 % scale.

## Phase 5 — Dashboard and lists (PR 5)

- **Dashboard** (**built: 5a**, below):
  - the outstanding bar (red overdue, yellow open, grey scale to $10k
    minimum, big total);
  - aging buckets;
  - oldest overdue;
  - DSO and average days to pay;
  - drafts waiting;
  - recognition due (a count, plus "Open in Ledgeline").
- **Invoice list (built, 5b S1, S3, S4, S8):** filters (status, customer,
  open/overdue, date range, age), sort by any column, overdue highlighting,
  quick jump (the index, not MBR search); the filter and sort live in the
  URL (J15).
- **Customer list (built, 5b S2):** sort (name, owed, overdue, last
  payment) and filter (owes, overdue, person or organization, a text
  search), in the URL as above (J15).
- **Open POs on the customer page (built, 5b S5–S7):** add, edit and close
  a customer's `open_pos` in the app, written by key (K1); the editor's
  reference suggestions read them (J16).
- **Narrow windows** for every page (J12, a standing requirement).
- **Charts:** follow the `dataviz` skill and the Ledgeline palette.
- **Dashboard rework (built: 5c):** no New button of its own, paid in the
  last N days with a period selector, the get-paid funnel, recently updated
  invoices, the outstanding bar on an axis, hover and focus throughout.
- **Tests:** API goldens on the accrual and cash fixtures, and dashboard
  screenshots for the docs.

### 5a — Receivables dashboard (built 2026-11-24)

Built on `2026-11-24-invoicer-dashboard`. Core: `invoicer_core::views::dashboard`
(`dashboard`, `dashboard_for`, `DashboardOptions`), computed from the index,
the book states and the recognition due, with `today` injected; everything
owed comes from `receivables`, so the dashboard, `receivables` and a
customer's account agree. App: `app/dashboard.rs`. SPA: `routes/+page.svelte`,
`DashboardView`, `ReceivablesPanel`, `lib/dashboard.ts`.

| #   | Decision                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
|-----|------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| R1  | **The dashboard is the app's home** (`/app/`) and the sidebar's first section, **Dashboard** (no counts; active on the root only). The old redirect to the invoice list is gone; the e2e specs that start on the list open `/app/invoices#token=…`.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| R2  | **What is owed is what the books say**, as in Ledgeline: in accrual books every receivable still owed, with those that have no invoice file here (an opening balance, the fixture's `2001`) included; in cash books sent invoices less the payments applied. Only positive amounts count. The part without a file is said under the bar ("$11,300.00 of it (4 items) was booked without an invoice here…", `unfiled`), since the invoice list can't show it.                                                                                                                                                                                                                                                                                                                                                                 |
| R3  | **The outstanding bar** is a tall segmented bar (28px, dataviz): overdue in the theme's `error`, not yet due in its `warning`, a grey rest (`base-content/10`) up to the scale: the total, or **10,000 in the commodity's units** when that is more (`SCALE_MINIMUM`; a display constant, not accounting policy). Status tokens, not a series palette: they mean what the sidebar's red badge and Ledgeline's budget bars mean, and every theme validates them. The validator: CVD ΔE 16.1, normal vision 23.4; on white the contrast is below 3:1 (yellow 1.72, red 2.8), so the relief is the figures printed beside the bar with a swatch each, the aging list as the table view, and 2px gaps. No text in the bar. The total is the page's one hero figure (`text-5xl`); a second commodity gets its own panel, smaller. |
| R4  | **Aging buckets are a setting**: `[invoicer] aging-buckets = [30, 60, 90]` (one to eight increasing day counts). Labels are Ledgeline's (Current, 1–30, 31–60, 61–90, **91+**, not "90+"); URL keys `current`, `1-30`, `91+` (the list's `aged`, S1; `91-` reads too). A meter list: name, amount and count, a thin bar of its share of the total; Current yellow, the rest red, as in the bar; an empty bucket isn't a link. `receivables` and Ledgeline keep the fixed buckets (planned: the edges in `ledgeline-core`, shared).                                                                                                                                                                                                                                                                                           |
| R5  | **Every figure opens the invoice list, filtered** by the query parameters below (`listHref`, `lib/dashboard.ts`). The list's parser (5b, S1) reads them: each figure lists the invoices behind it that have a file (`dashboard.e2e.ts`); `customer=` takes the books' name too.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| R6  | **DSO** is Ledgeline's: owed at the end of today (invoices dated on or before it) over what was billed in the window, times its days; a void counts in neither. The window is `[invoicer] dso-days` (90); `receivables --dso-days` now defaults to it. **Average days to pay**: the mean days from an invoice's date to the payment that completes it, over every invoice paid in full (voids out), all time. The tiles show the first commodity's DSO and oldest overdue.                                                                                                                                                                                                                                                                                                                                                   |
| R7  | **The oldest overdue** is the earliest due date (then date, number). Its tile opens the invoice; one with no file opens `?status=overdue&customer=…`.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| R8  | **Drafts waiting**: the count, their totals per commodity and the oldest draft's date; the tile opens `?status=draft`.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| R9  | **Recognition due** shows when the books have a recognition file: the entries Ledgeline's **Post through ‹today›** would post (open periods, customers' and vendors' schedules alike), with those in closed periods counted apart. **Open in Ledgeline**: Ledgeline has no URL scheme for a page, so the button opens the books in it (`/usr/bin/open -b com.zmre.ledgeline <journal>`, which Ledgeline takes as a document open), in the window on macOS only (capability `open_ledgeline`, `POST /api/ledgeline/open`); elsewhere the tile says "Post them in Ledgeline: Journal → Recognition." Opening the Recognition overlay itself needs a Ledgeline URL scheme: planned.                                                                                                                                             |
| R10 | **Speed**: the `invoicer/dashboard` bench (the API's work, serialized) is **11.8 ms** on the 5,000-invoice corpus, against the 300 ms open → dashboard budget; added to `benches/baseline.json`. It was 32 ms: `receivables` found each open item's file by scanning the index; it now builds a number → file map once (the customer list gains too). Most of what is left is the recognition index (~10 ms).                                                                                                                                                                                                                                                                                                                                                                                                                |
| R11 | **Tests**: `dashboard` unit tests and `tests/dashboard.rs` (both fixtures on the e2e date, custom edges, a later today); router goldens `tests/snapshots/{accrual,cash}-api-dashboard.json` (today 2026-11-22; `UPDATE_SNAPSHOTS=1` re-records), which the SPA's unit and component tests read too, so the two can't drift; `e2e/dashboard.e2e.ts` in its own Playwright project, first (it reads the committed figures; later projects send 108 and add 2005); the empty books' dashboard in `empty.e2e.ts`. **Screenshots**: the e2e writes wide and narrow, dark and light, to `target/e2e-invoicer-shots/` for review. A docs harness like Ledgeline's `web/docs-screenshots/` is Phase 6 work.                                                                                                                          |
| R12 | **No `dashboard` CLI command**: `receivables --json` (with `--aging`) and `recognize --dry-run` already give every figure.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |

The invoice list's query parameters (R5). They combine
(`?status=overdue&customer=Tailspin`):

| Parameter                            | Shows                                                              | Emitted by                                |
|--------------------------------------|--------------------------------------------------------------------|-------------------------------------------|
| `status=open`                        | invoices still owed (sent, not paid in full), overdue included     | the total, DSO                            |
| `status=overdue`                     | still owed and past due on today                                   | the overdue block and figure              |
| `status=draft`                       | drafts                                                             | Drafts waiting                            |
| `status=paid`                        | paid in full                                                       | Average days to pay                       |
| `aged=current`                       | still owed, not yet past due                                       | the not-yet-due block and figure; Current |
| `aged=<from>-<to>`                   | still owed, `from` to `to` days past due, inclusive (`aged=31-60`) | an aging bucket                           |
| `aged=<from>+`                       | still owed, `from` or more days past due (`aged=91+`; `91-` reads) | the last aging bucket                     |
| `customer=<name>`                    | billed to that customer (the name the books use, `Tailspin`)       | the oldest overdue, when it has no file   |
| `paid_from=<date>`, `paid_to=<date>` | money received on or after / on or before (R14)                    | Paid in the last N days                   |

New routes:

| Route                       | Body | Answers                                                                                            |
|-----------------------------|------|----------------------------------------------------------------------------------------------------|
| `GET /api/dashboard?today=` | —    | the `Dashboard` view, plus `open_ledgeline` (whether this process can open the books in Ledgeline) |
| `POST /api/ledgeline/open`  | —    | `{done: true}`; `503` without the window (macOS); `500` when Ledgeline isn't installed             |

**Pending (the lead):** `just invoicer-e2e` (the new `dashboard` project runs
first; every spec that started on `/app/` now opens `/app/invoices#token=…`;
`empty.e2e.ts` gains the dashboard's empty state); look at the screenshots in
`target/e2e-invoicer-shots/`. R5's parameters and the list's parser are
reconciled at merge (the last bucket is `91+`, `91-` reads too; the spec
checks each figure filters the list). In the window: **Open in Ledgeline** with Ledgeline
installed (it opens these books) and without (the tile's error), the
dashboard at a narrow width and at 150–200 % scale, in light and dark themes.

### 5b — Lists and open POs (built 2026-11-24)

Built on `2026-11-24-invoicer-lists`: the invoice and customer lists filter
and sort, all of it in the URL (J15), and a customer's open POs are added,
edited and closed in the app (J16). The dashboard (5a, above) links into
the invoice list with the contract below, and its figures filter it (R5).

| #  | Decision                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
|----|-----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| S1 | **The invoice list's state is its URL, and the server filters and sorts.** `GET /api/invoices` takes the URL contract below as it is (blank values are no filter), through `views::ListQuery` (`from`, `to`, `aged`, `text`, `sort`, `dir`, `paid`, `with_status`) and `views::query`; the SPA (`lib/listQuery.ts`) parses the page's query, drops what the server wouldn't read (a mistyped link shows the whole list, not an error; the API itself answers `400`), and hands the rest over. One parser, so the dashboard's links, the CLI later and the list agree. Choices are a history step each (Back undoes one); the search is debounced (250 ms) and replaces the step, so typing doesn't fill the history. The default list (Date, newest first, no filter) writes no query: `/app/invoices` stays bare.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| S2 | **The customer list likewise** (`GET /api/customers` with `owes`, `overdue`, `kind`, `q`, `sort`, `dir`; `views::CustomerQuery`, `account::query_customers`). Owed sorts by the first commodity's balance (nothing owed is zero); Overdue by the count of overdue invoices, then the older oldest due date; Last payment by date. The list gains an **Overdue** column (the count as the red badge, the oldest due date as its tooltip) so it has a header to sort by; Name, Owed, Overdue and Last payment sort, Billing email and Invoices don't.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| S3 | **Quick jump is the search box's Enter**: the invoice whose number is exactly what was typed (any case), else the only invoice the search leaves; otherwise a line says to pick one below. It asks the index (the same match as the list), not MBR's search: MBR searches prose too and would need a page of its own; later if wanted. Enter in the customer search opens the customer when one is left.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| S4 | **Sorting**: a button in each sortable `<th>` (Tab reaches it, Enter or Space sorts), `aria-sort` on the `<th>`, the arrow only for the eye (`SortHeader.svelte`). A new column starts its own way round (amounts and dates biggest or latest first, names A first, due dates soonest first, status in lifecycle order: draft, open, overdue, paid, void); the same column flips. A missing value (no due date, total or payment) sorts last either way; ties keep the list's own order (newest first; by name).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| S5 | **Open POs are managed on the customer page** (J16), in the view (Add PO, Edit, Close, each saved at once) and in the form (rows saved with Save). The fields are the ones the core reads: `reference` (required, one per PO, any case), `amount` (money, `Money::read`: the default commodity's style, then each declared one's) and `expires` (YYYY-MM-DD); no note field. **Closing removes the PO from `open_pos` and notes it in the note's body** (Patrick, 2026-11-24): a line under a `## Closed POs` section at the end of the body (made when missing, appended to when present), `- PO #PY75334 — $40,000, expires 2027-06-30; invoiced $8,350.00; closed 2026-11-24.` (the amount and expiry as written, what was invoiced from S6 in the books' money style, the date the SPA sends as `today`, the server's local date otherwise; `customer_edit::update_on`, `closed_po_line`, `note_closed_pos`). A PO the new list leaves out (none names it by `was`) is closed, from the view or the form; a rename isn't. The body edit is in the same conditional write as the frontmatter (one hash check, one replace) and leaves every other byte; nothing reads the note, the user may delete it. No `closed:` key; the invoices that quoted it keep their `reference:`; the last one closed removes the key. The save answers `noted` (the references), and the page says where they went. Written by key (K1) with the note's content hash (K2): only `open_pos` is rewritten; inside it an entry keeps keys the form doesn't know, found by `was` (the reference it had when read), and a value that reads the same keeps its written form (`amount: 40000` stays a number). A `409` offers Reload or Apply my changes, which re-sends the PO list. |
| S6 | **What a PO has left** (`customers show`'s new `po_usage`, in `open_pos` order): the sent invoices (not drafts, not void) billed to the customer whose `reference` is the PO's (trimmed, any case: `PO #PY75334, rev. 2` is another PO), their totals added up in the PO's commodity, and the amount less that (negative when over), plus `expired` against `today`. An amount that isn't money, or invoices in another commodity, leave `invoiced`/`remaining` empty rather than guess.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| S7 | **Reference suggestions follow at once**: the editor and New invoice already fetch `/api/editor/options` again on any change on disk, and the session re-indexes a note it writes; a PO expired by the invoice's date is still offered, last and marked "expired".                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| S8 | **The filter bars** (`InvoiceFilters`, `CustomerFilters`): the open/overdue toggle is the Status select's Open and Overdue (one `status` parameter, so a dashboard link shows in the control); the customer is picked by name (the combobox, every customer); the dates are `DateInput` (never WebKit's painted "today"); "Late by" offers Ledgeline's aging buckets, and shows any other `aged` a link brings. The customer bar has "Owes money" and "Overdue" toggle buttons (`aria-pressed`) and a kind select. **Clear filters** shows only while a filter is on and keeps the sort. A count line says "N invoices" or "N invoices match"; an empty filtered list says nothing matches with Clear filters (not H21's first-invoice card), and the New button stays. Overdue invoice rows and customers with something overdue are tinted. Narrow windows (J12): the controls wrap, the search taking its own row below `sm`; tables scroll in their own box.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |

**The invoice list's URL** (`/app/invoices?…`), the same parameters as
`GET /api/invoices` (which also takes `type`, `open`, `overdue` and
`today`). Unknown values are dropped by the page and refused (`400`) by the
API; blank values are no filter. Written in this order, defaults omitted:

| Parameter              | Values                                                 | Means                                                                                                                                                                                                                                                    |
|------------------------|--------------------------------------------------------|----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `status`               | `draft`, `sent`, `paid`, `void`, `open`, `overdue`     | `draft`/`sent`/`void`: the `status:` value; `paid`: sent and paid in full; `open`: sent, not paid in full (overdue included); `overdue`: open and past due. The API also takes any other `status:` value (`estimate-open`).                              |
| `customer`             | a name                                                 | The customer as the invoice links or names it (`Contoso`, `[[Contoso]]`), or the books' name (`ledger_customer`, which the dashboard links with), any case.                                                                                              |
| `from`, `to`           | YYYY-MM-DD                                             | Dated on or after `from`, on or before `to` (inclusive).                                                                                                                                                                                                 |
| `aged`                 | `current`, `A-B`, `N+` (or `N-`)                       | Open and A to B days past due (inclusive), or N days or more; `current` is not yet due. Any edges (`[invoicer] aging-buckets`, R4); the dashboard emits `current`, `1-30` … `91+`. `N-` reads as `N+` (the page rewrites it); `90+` includes day 90.     |
| `paid_from`, `paid_to` | YYYY-MM-DD                                             | Money received on or after `paid_from`, on or before `paid_to` (inclusive): by the books' payments when they can say, else the `paid:` mirror; a partial payment counts, so an invoice still open can match (R14). The bar shows it as a removable chip. |
| `q`                    | text                                                   | Number, title, customer name or link, `reference` or total holding it, any case.                                                                                                                                                                         |
| `sort`                 | `number`, `customer`, `date`, `due`, `total`, `status` | The column; absent: Date, newest first (the server's order).                                                                                                                                                                                             |
| `dir`                  | `asc`, `desc`                                          | Which way; `asc` when absent. Written only with `sort`.                                                                                                                                                                                                  |

For example `?status=overdue`, `?customer=Contoso`, `?aged=31-60`,
`?from=2026-10-01&to=2026-10-31`, `?q=PY75334&sort=total&dir=desc`,
`?paid_from=2026-10-24&paid_to=2026-11-22`.

**The customer list's URL** (`/app/customers?…`), the same as
`GET /api/customers` (plus `today`):

| Parameter | Values                                    | Means                                                                         |
|-----------|-------------------------------------------|-------------------------------------------------------------------------------|
| `owes`    | `true`                                    | Only customers that owe something (`1` and `yes` read too).                   |
| `overdue` | `true`                                    | Only customers with an overdue invoice.                                       |
| `kind`    | `person`, `organization`                  | People or organizations.                                                      |
| `q`       | text                                      | Title, name in the books, billing name or billing email holding it, any case. |
| `sort`    | `name`, `owed`, `overdue`, `last-payment` | The column; absent: by name.                                                  |
| `dir`     | `asc`, `desc`                             | As above.                                                                     |

Routes changed: `GET /api/invoices` and `GET /api/customers` take the
parameters above; `GET /api/customers/{key}` adds `po_usage` (`reference`,
`amount`, `expires`, `expired`, `invoices`, `invoiced`, `remaining`) and its
`form` adds `open_pos` (`reference`, `amount`, `expires`, `was`), which
`PUT /api/customers/{key}` and `POST /api/customers` take. The CLI's `list`
and `customers list` don't take the new filters yet (later: flags over the
same `ListQuery`/`CustomerQuery`).

Later, from this round: MBR's search for quick jump (S3); the inline PO row
isn't covered by the unsaved-changes prompt (it saves on Enter or Add PO;
leaving drops what was typed); keyboard list navigation (`j`/`k`, `/` to
search) is 5k's.

**Pending (the lead):** `just invoicer-e2e`: the new `lists.e2e.ts` (its own
project after `library`; it relies only on facts the earlier specs leave
alone, and puts Customer X's `open_pos` back) and the customer list's new
Overdue column under `customers.e2e.ts` (the badge moved there and reads
"1 overdue"). In the window: the filter bars at a narrow width and at
150–200 % scale, Back/Forward through filters in WKWebView, the date filters'
empty state (DateInput), and Add PO by keyboard.

### 5c — Dashboard rework (built 2026-11-25)

Patrick's feedback on 5a, built on `2026-11-25-invoicer-dashboard-v2`
(stacked on 5b). Layout patterns borrowed from the reference screenshots
he shared (a stacked bar on an axis with the aging list beside it, a row
of recent invoice cards with a status band, a funnel with a period
selector), in Ledgeline's own styling. Core: `views::dashboard` gains
`paid` (`paid_view`, `paid_window`) and `recent`; `BookStates::money_received`
and `views::paid_dates`; `ListQuery.paid_from`/`paid_to`. SPA:
`PaidFunnel`, `OutstandingBar`, `RecentInvoices`, a rewritten
`ReceivablesPanel`, the paid chip in `InvoiceFilters`.

| #   | Decision                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
|-----|---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| R13 | **No New invoice button on the dashboard**: the sidebar's New split button is the one way to start (no duplicate actions). Its empty state has no button either: "Start one with New at the top of the sidebar." `EmptyState`'s action is optional for this one use; every list page keeps its button (H21).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| R14 | **Paid in the last N days**: `[invoicer] paid-window-days = 30` (1–3660), `GET /api/dashboard?paid_days=N` overrides it, and the page offers Last 7 / 30 / 90 / 365 days (plus the setting's own) in the funnel's header; the choice is the page's URL (`/app/?paid_days=90`, a history step; the setting's own writes no query). The window is the N days ending today, both ends counted (30 days to 2026-11-22 is 2026-10-24 on). **What counts is money received, by its date**, per commodity: in accrual books the payment settlements of every receivable (Ledgeline's links, so an item without an invoice file counts and is counted apart, `unfiled`); in cash books the payments applied to an invoice number, their customer income legs, so a fee booked apart doesn't shrink it (201: $3,000 from a $2,910 payout and a $90 fee). Credit notes, write-offs and voids are not money in (a setting to count credits is planned). The count is the invoices paid into: a partial payment counts its invoice, still open. The card opens the list at `?paid_from=…&paid_to=…` (money received in the range, partial payments included, by the books else the `paid:` mirror), not `status=paid`, so the list holds what the figure counts; the list shows the range as a removable chip rather than gaining two more date fields. |
| R15 | **Recently updated invoices**: the latest of an invoice's date, its file's modification day (local time), its sent date, the last money received and its void date; anything after `today` doesn't count (the dashboard is as of `today`, and so a fixed clock in tests ignores checkout times). A same-day tie goes to the later event in its life (voided, paid, sent, edited, dated), then the later modification, then the higher number. **Six** (one row wide), invoices only, from `recent` in the API: number, customer, date, `updated`, `event`, `status` (draft, open, overdue, paid, void), total. A card: "No. 108", the customer, "Paid 2026-11-05", the amount (struck through when void), and a band with the status in words on the list badges' colours (paid success, open and partly paid warning, overdue error, draft and void neutral). One sideways-scrolling row when narrow, three columns from `@3xl`, six from `@5xl`; each card opens its invoice.                                                                                                                                                                                                                                                                                                                                                             |
| R16 | **The outstanding bar on an axis**: round ticks from 0 to the first step at or past the scale (the total, or **$10,000** at least, R3), at most about six steps of 1, 2, 2.5 or 5 × 10ⁿ, labelled compactly over the bar (0, 2k … 10k; 12.5k; 1.5M), a short tick under each. A quiet track replaces the grey "rest" block and its legend entry. Overdue then not yet due, end to end, 2px apart. The total is the hero figure to the bar's right with the caption "Total outstanding" and the count; the aging buckets are a compact aligned list beside it (a dot, the range, the amount, the count; the meter bars are gone), each row a link (`aged=`). All three in a row from `@5xl` of the panel, the total and the list side by side from `@md`, stacked below.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| R17 | **The funnel is the headline**: Drafts → Outstanding → Paid in the last N days, cards with chevrons between (pointing down when they stack), each with the amount, a count badge ("1 draft", "7 owed", "2 paid") and a line under it (the oldest draft; the overdue amount with a red dot; "since ‹from›"), each one link: `status=draft`, `status=open`, `paid_from`/`paid_to`. A separate row of big numbers would repeat it, so the overdue figure lives in the Outstanding card and the bar's legend, and the panel's total stays the page's one hero figure. The Drafts tile is gone (the funnel has it); the tiles are the oldest overdue, DSO and average days to pay, plus recognition due. Order: funnel, outstanding, recently updated, tiles.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| R18 | **Hover and focus everywhere**: a bar segment shows its tooltip (the amount first, then the name with a line key, the count and its share of what is owed) on hover and on keyboard focus, outlines itself and dims the other; Escape hides it. Aging rows highlight; funnel cards, recent cards and tiles lift (2px and a shadow) on hover and on `focus-visible`. No transform or transition under `prefers-reduced-motion`. Colour is never alone: every amount is printed (the legend, the list, the cards' status words). Only movement, shadow and opacity animate, never colours: a theme switch (the prefs loading, the OS scheme flipping a family theme) repaints every card at once instead of leaving them half-switched. Every surface and text is a daisyUI theme token; nothing reads `prefers-color-scheme`.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |

Routes changed: `GET /api/dashboard` takes `paid_days` (`400` outside 1–3660)
and adds `paid` (`days`, `from`, `to`, `commodities` of `commodity`, `amount`,
`text`, `count`, `unfiled`), `recent` and `paid_window_days`; `GET
/api/invoices` takes `paid_from` and `paid_to` (`400` for a bad date).

**Tests**: core `tests/dashboard.rs` (paid on both fixtures and windows,
recently updated), `views` (the paid-date filter, by books and by mirror),
unit tests (the window, local dates, event order); router goldens
re-recorded (additive: `paid`, `recent`, `paid_window_days`; the copies'
file times are pinned so 106 and 201 read as edited on 2026-11-21) and the
`paid_days` and list round trip; the config goldens gain the key. SPA:
`dashboard.test.ts` (ticks, labels, segments, windows, URL), component
tests for `OutstandingBar` (geometry, tooltip on hover and focus, Escape),
`PaidFunnel` (links, badges, the selector), `RecentInvoices` and the filter
chip; `dashboard.e2e.ts` and `empty.e2e.ts` rewritten, not run.

**Speed**: the `invoicer/dashboard` bench is 12.3 ms on the 5,000-invoice
corpus (+3.7 % on R10's 11.8 ms; the views part, paid and recent included,
is ~3 ms), against the 300 ms budget; the baseline isn't re-recorded.

**Pending (the lead):** `just invoicer-e2e` (the `dashboard` project: the
funnel's click-throughs, the period selector and Back, a card opening its
invoice, the hover tooltip, the narrow window; `empty.e2e.ts`'s dashboard
case; the new two-theme contrast check). The screenshots now pin the app's
own theme (`openThemed`: `GET /api/ui-prefs` answered with the variant, a
full reload, transitions settled; the first run's light shots were taken
mid-switch, after a hash-only navigation). Look at `target/e2e-invoicer-shots/dashboard-{wide,narrow}-{dark,light}.png`,
`dashboard-tooltip-{dark,light}.png` and `dashboard-medium-light.png`. In
the window (WKWebView): the tooltip on hover and Tab, the cards' lift, the
bar's axis at 150–200 % scale, the funnel stacking, reduced motion.

## Phase 5k — Keyboard-first (TODO)

The goal: the app makes the user fast, and works fully from the keyboard
where it can (J13).
- **Extract Ledgeline's keymap** (`web/src/lib/keys/`: layers, chords,
  dispatch, hints, the `KeyHelp` sheet) into `packages/ui`, Ledgeline
  importing it from there, as `AccountInput` was (E12). One keymap, two apps.
- **App-wide:** go-to chords (`g i` invoices, `g c` customers, `g k`
  catalog, `g s` snippets, `n` new invoice), `?` for the help sheet, `/`
  to search the current list; maybe a command palette.
- **Lists:** `j`/`k` to move, Enter to open, sort and filter keys (J15).
- **Editors:** save, discard, add each row type, move rows (Alt+↑/↓ today),
  open the customer, template and catalog pickers; Edit/Cancel on the
  customer page.
- **Tests:** the keymap's own unit tests move with it; component tests per
  binding; one e2e that does an invoice end to end without the mouse.

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
11. Mobile: iOS (`invoicer-ffi`), following Ledgeline's plan 42 approach (J14).
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
