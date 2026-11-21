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

| #   | Decision                                                                                                                                                       |
|-----|----------------------------------------------------------------------------------------------------------------------------------------------------------------|
| I1  | Ledgeline and the invoicer both become **AGPL-3.0-or-later**. MBR (GPL-3.0+) is then linkable.                                                                  |
| I2  | **Drafts never touch the books.** Sending books the invoice.                                                                                                   |
| I3  | Discounts, subtotals and taxes are **inline positional rows** in the one bill table. An adjustment applies to the nearest item or total row above it, skipping other adjustments. |
| I4  | Invoices name their template with MBR's **`template:` path** (`/invoices/tpl/x.html`). MBR renders styled invoices everywhere.                                   |
| I5  | Per-row accounting lives in a **books column**: the last column, with an italic header, hidden by CSS. A blank cell means the default account.                 |
| I6  | Partial up-front billing is a **separate deposit invoice**. The final invoice applies it with a row against the deposit liability. In accrual books, undelivered work books to deferred revenue or deposits. |
| I7  | `status: draft\|sent\|void` is intent. `paid:` is a mirror refreshed from the books: `no`, `partial 2026-10-28 $1,500, 2026-11-19 $3,500`, or a date.          |
| I8  | A sent invoice may be **edited and re-booked**. Rewriting it is refused on or before `closed-through`; void and reissue (later, a credit note) is offered instead. |
| I9  | `po_num` becomes **`reference`** (free text: `PO #105`).                                                                                                        |
| I10 | The binary is **`ledgeline-invoicer`**.                                                                                                                         |
| I11 | The default deposit account is **`liabilities:deferred-revenue`**, overridable with `[invoicer] deposit-account`.                                               |
| I12 | Final PDFs are **kept** in `pdf-dir` (default `invoices/pdf/`), with earlier exports kept too. Users delete old ones as they please.                            |
| I13 | Every English word the invoicer matches (column headers, total rows, the customer tag, contact labels) is a setting in `[invoicer.words]`. File-format keys stay fixed. |
| I14 | **Performance and correctness are release criteria.** The ARCHITECTURE §5.6 budgets are gated by benches on a 5,000-invoice corpus.                            |
| I15 | Customer `contracts:` take wikilinks or file paths (executed PDFs). `billing_portal:` records vendor-portal delivery.                                           |
| I16 | MBR ships **one release** once its P0 and P1 changes land. The invoicer uses it from crates.io: a git rev cannot work, because `mbr-core` embeds built JS bundles that only the crates.io package contains. The release gates Phase 3. |
| I17 | Cash books get **Apply payment** in v1 (Phase 2): pick an imported deposit and its income side is re-split into the invoice's row accounts, tagged `invoice:`. The invoicer never creates payment transactions. Both bases are first-class: Patrick runs accrual and cash businesses. |
| I18 | **No Intel Mac build of the invoicer** (Patrick, 2026-11-20). It ships for Apple Silicon macOS and Linux only. Ledgeline keeps its x86_64-darwin build.         |
| I19 | **The invoicer crates stay `publish = false`** until Patrick decides to share them (2026-11-20). Nothing about the invoicer goes to crates.io before then.      |
| I20 | **Quality is paramount: CI lints every web directory** (`web/`, `packages/ui`, `invoicer/web`) with prettier and eslint, not only pre-push (2026-11-20).        |

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

| #   | Decision                                                                                                                                                          |
|-----|-------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| C1  | Numbering already counts the books' `invoice:` tags (§5.1), except bills on the payable side. The accrual fixture's next number is 2005, not 109.                 |
| C2  | `set` refuses `status`, `paid`, and `invoice_num` on a sent or void document. Those changes wait for send/void/rebook; hand edits stay possible.                  |
| C3  | `recalc` with no arguments rewrites drafts only. Sent and void documents are reported as skipped and rewritten only when named.                                   |
| C4  | `check` exits 1 on errors only; `--strict` makes warnings fail too. Any failure to run exits 2.                                                                   |
| C5  | "Open" and "overdue" read the `paid:` mirror until the Phase 2 reconciler exists. Today is the **local** date (`--today` overrides); Ledgeline's server uses UTC. |
| C6  | The journal comes from `JOURNAL` or `-f JOURNAL` (hledger's flag), then `$LEDGELINE_FIXTURE`, then recents. The CLI never records recents.                        |
| C7  | `--row` takes `DESC\|QTY\|RATE\|ACCOUNT`. With no quantity, a money rate becomes the amount. A bare `Subtotal` writes a total row.                                |
| C8  | `--json` failures print `{"error": "…"}` on stdout (exit 2), so a script reads one JSON value either way.                                                         |

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
