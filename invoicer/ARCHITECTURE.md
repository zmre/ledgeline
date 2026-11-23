# Ledgeline Invoicer — Architecture

Ledgeline Invoicer is a desktop app (mobile later) that makes invoices,
renders them to PDF, and books them into the same hledger journal that
Ledgeline reads. Every artifact is a plain-text file in the books' folder:

- an invoice is a markdown note;
- a customer is a person or organization note;
- a template is a Tera HTML file;
- the accounting lives in the journal, as tags.

The GUI is a structured editor over those files. A CLI does nearly
everything the GUI does, for scripts. Third parties can read and write the
same files without asking us.

This document describes what the app is and how it fits together.
[PLAN.md](PLAN.md) breaks it into phases, and [MBR-CHANGES.md](MBR-CHANGES.md)
lists what MBR has to grow.

## 1. Principles

1. **Plain text is the database.** No sidecar state, no SQLite, no hidden
   cache that matters. Delete every cache and the app rebuilds its state from
   the markdown plus the journal. Anything the GUI shows can be found with
   `grep` or `hledger`.
2. **Readable without us.** An invoice renders correctly in any markdown
   previewer that supports GFM tables: GitHub, Obsidian, MBR without the
   template. Totals are written into the table as values. There are no HTML
   comments in markdown.
3. **The books are the truth for money; the markdown is the truth for the
   document.** What was billed, to whom, and how it reads comes from the
   markdown. What was booked, paid and recognized comes from the journal. The
   `paid:` field in an invoice is a mirror the invoicer refreshes, never an
   input.
4. **Hand edits are first-class.** Every file may be edited by hand or by a
   script between runs. The app re-derives, validates, and offers to
   reconcile. It never silently overwrites.
5. **One engine.** All booking goes through `ledgeline-core`: plan 45's
   `accrual::ops`, the editor's `ExternalChange` guard, and the journal's
   amount styles. The invoicer writes no journal text of its own.
6. **UI-agnostic core.** All logic lives in `invoicer-core`: parsing,
   arithmetic, numbering, booking, reconciliation. The GUI, CLI, and a future
   iOS app are thin layers. That is the plan 26 rule, applied again.
7. **Lock-step with Ledgeline.** Same repo, same version, same release,
   same GUI stack, same themes, same prefs.
8. **Fast, and never wrong.** Everything is local, so every interaction
   should feel instant. Speed comes from indexing and incremental work, never
   from skipping a check. Money is always exact `Dec` arithmetic, and every
   write is verified. See §5.6 for budgets and how they are met.

## 2. The pieces

```text
            ┌─────────────────────────── books folder (a git repo, usually) ───────────────────────────┐
            │  books.journal ── include ─► 2026.journal        recognition.journal (not included)       │
            │  ledgeline.toml  [ui] [books] [sync] [invoicer]                                           │
            │  invoices/2026-11-17 Customer X Invoice 105.md      invoices/tpl/myco-default.html        │
            │  invoices/catalog.md                                 invoices/tpl/snippets/net-30.md       │
            │  customers/Customer X.md  (type: organization)       customers/Fred Smith.md (type: person)│
            └───────────────▲──────────────────────────────▲───────────────────────────▲───────────────┘
                            │ ledgeline-core (books)       │ mbr-core (notes, render)   │ invoicer-core (docs)
             ┌──────────────┴──────────────────────────────┴───────────────────────────┴─────────────┐
             │ invoicer-core: document model · table engine · customers · catalog · templates ·       │
             │                numbering · booking bridge · reconciliation · receivables view          │
             └──────────────▲───────────────────────────────────────────────────▲────────────────────┘
                            │                                                   │
          ┌─────────────────┴──────────────────┐                ┌───────────────┴───────────────┐
          │ ledgeline-invoicer (bin)           │                │ invoicer-ffi (later, iOS)     │
          │  CLI (clap) · axum API · SPA embed │                │  same router via URL scheme   │
          │  wry/tao shell · MBR router · PDF  │                └───────────────────────────────┘
          └─────────────────▲──────────────────┘
                            │ HTTP on 127.0.0.1 (token)
          ┌─────────────────┴──────────────────┐
          │ invoicer SPA (SvelteKit, /app/)    │  shares packages/ui with Ledgeline's SPA
          └────────────────────────────────────┘
```

### 2.1 Repo layout

The invoicer lives under `invoicer/`, so it can be moved out later. It joins
the existing Cargo workspace.

```text
invoicer/
  ARCHITECTURE.md  PLAN.md  MBR-CHANGES.md
  crates/
    invoicer-core/        lib: everything but UI and platform
    ledgeline-invoicer/   bin + lib: CLI, axum API, SPA embed, desktop shell, PDF
    invoicer-ffi/         later: UniFFI staticlib for iOS
  web/                    the invoicer SvelteKit SPA
  fixtures/               invoice/customer/template fixture folders (with a journal)
packages/
  ui/                     shared web package: themes, scale, breakpoints, components,
                          API client + token, native bridge (extracted from web/)
crates/
  ledgeline-core/         gains: receivables, document memo/extra tags, prefs/app-dirs
  ledgeline-shell/        new: the shared desktop/server shell pieces (see 2.3)
```

The workspace members gain `invoicer/crates/*` and `crates/ledgeline-shell`.
A root `package.json` makes `web/`, `invoicer/web/` and `packages/*` one bun
workspace, with one `bun.lock` and one `spaNodeModules` fixed-output
derivation.

### 2.2 Crates

| Crate                | Kind        | Depends on                                            | Holds                                                                                                                                     |
|----------------------|-------------|-------------------------------------------------------|-------------------------------------------------------------------------------------------------------------------------------------------|
| `invoicer-core`      | lib         | `ledgeline-core`; `mbr-core` (crates.io, Phase 3)     | The document model, table engine, customers, catalog, snippets, templates, numbering, booking bridge, reconciliation, receivables view.    |
| `ledgeline-invoicer` | bin + lib   | `invoicer-core`, `ledgeline-shell`, `mbr-server`, mbr (PDF) | The CLI, the axum `/api` routes, the SPA embed, the MBR router mount, the desktop window, and PDF export.                           |
| `ledgeline-shell`    | lib         | axum, wry/tao/muda/rfd (behind `gui`)                 | Token and Host guard, rust-embed SPA serving with init script, the GUI boot sequence, `AppDirs`, recents, and UI-prefs merge, all extracted from `ledgeline-server`. |
| `invoicer-ffi`       | staticlib   | `ledgeline-invoicer` (no gui), uniffi 0.31            | Later. Mirrors `ledgeline-ffi`: `handle(method, url, headers, body)` behind a `ledgeline-invoicer://` URL scheme.                          |

`invoicer-core` follows the plan 26 rules:

- no globals;
- `today` is always a parameter;
- IO goes through `ledgeline_core::vault::Vault`, so the same code runs on an
  iCloud vault later;
- no platform `cfg`.

### 2.3 What is shared with Ledgeline, and how

| Shared thing                                          | Today                                                   | After                                                                  |
|-------------------------------------------------------|---------------------------------------------------------|------------------------------------------------------------------------|
| Theme CSS, theme catalog/resolve, UI scale, breakpoints | `web/src/themes`, `web/src/lib/theme`, `web/src/app.css`   | `packages/ui` (both SPAs import it)                                     |
| Common Svelte components (`DateInput`, `InfoTip`, …)   | `web/src/lib/components`                                | `packages/ui/components`                                               |
| API client, token, native bridge                      | `web/src/lib/api/client.ts`, `web/src/lib/native`        | `packages/ui` (the app-specific endpoint wrappers stay in each app)     |
| Token, Host guard, SPA embed, CSP, GUI boot, menus     | `ledgeline-server/src/{security,spa,gui}.rs`              | `ledgeline-shell`                                                      |
| UI prefs (SPECS, merge, save routing, theme pin), `AppDirs`, recents | `ledgeline-server/src/{ui_prefs,app_dirs,recents}.rs` | `ledgeline-shell` (prefs data model in `ledgeline-core`)           |
| Journal engine, accrual ops, settlements              | `ledgeline-core`                                        | unchanged, plus the gaps in §8                                           |

Themes therefore come for free. The invoicer reads the same global
`ui-prefs.toml` and the same `ledgeline.toml [ui]` pin. So if a books folder
is pinned to `catppuccin`, both apps open it in catppuccin. Recents are
shared too, so "open the books Ledgeline last had open" just works.

**Licensing.** Ledgeline and the invoicer both move to
**AGPL-3.0-or-later** (decided 2026-11-19). MBR is GPL-3.0-or-later, which
is compatible with AGPL-3.0 when combined under GPLv3 §13. `mbr-apple-contact-sync` is already
AGPL.

## 3. Files and formats

### 3.1 Where things live

Everything must be inside the books folder, meaning the canonical directory of
the main journal. That folder is the vault root for both Ledgeline and MBR.
Inside it, organization is the user's choice. The defaults are below, and
each one can be overridden in `ledgeline.toml [invoicer]`.

| What               | Default                            | Found by                                                                 |
|--------------------|------------------------------------|--------------------------------------------------------------------------|
| Invoices           | `invoices/`                        | Any note with `type: invoice` (or `estimate`, `credit-note`) anywhere     |
| Customers          | `customers/`                       | Any `type: person` / `type: organization` note tagged `customer`         |
| Templates          | `invoices/tpl/`                    | Any `.html` file whose Tera source references `invoice_num`              |
| Snippets           | `invoices/tpl/snippets/`           | Any note with `type: snippet`                                            |
| Product catalog    | `invoices/catalog.md`              | Any note with `type: catalog`                                            |
| Your company       | named by `[invoicer] company`      | An organization note (it may live anywhere)                              |

The default folders only decide where **new** files go. Discovery is always
by content, across the whole books folder, so a user who keeps
`customer-x/Customer X.md` next to `customer-x/invoices/105.md` loses
nothing. Discovery follows MBR's repo scan, so MBR's ignore rules apply: hidden
folders, `templates/`, `node_modules/` and so on, plus the invoicer's own
`pdf-dir`. That is one reason the default template folder is `invoices/tpl/`
and not `templates/`.

`invoicer_core::notes` implements that scan itself until MBR's `Site` façade
(MBR-CHANGES M2) is on crates.io: mbr-core can't be a git dependency, because
its built JS assets are gitignored (decided 2026-11-20). Its API is shaped
like `Site` (`notes_of_type`, `note`, `field_values`, `resolve_wikilink`), so
the swap touches no caller. It reads frontmatter with `yaml-rust2`, MBR's
parser, matches `type` and tags case-insensitively as MBR does, and resolves
wikilinks by MBR's body-link rules: same-folder file stem, then title, then
alias, then stem, the smallest path winning a tie.

### 3.2 An invoice

```markdown
---
title: "2026-11-17 Customer X Invoice 105"
type: invoice
template: /invoices/tpl/myco-default.html
status: sent
invoice_num: 105
date: 2026-11-17
terms: 30
due_date: 2026-12-17
reference: "PO #PY75334"
customer: "[[Customer X]]"
invoicee_name: Customer X
invoicee_email: accounts_receivable@customerx.com
invoicee_address: |
  1234 Main St. #555
  Springfield, CO 80000
invoicer: "[[My Company]]"
invoicer_name: My Company
invoicer_email: finance@mycompany.com
invoicer_address: |
  1234 Background Blvd.
  Anywhere, CA 90000
paid: no
---

Hi Fred, this is the work we discussed, less the deposit you already paid.

| Description                  | Qty | Rate          | Amount       | *Account*                     |
|:-----------------------------|----:|--------------:|-------------:|:------------------------------|
| Senior architect hours       | 10  | $800          | $8,000       | revenue:consulting            |
| Friends and family discount  |     | -10%          | -$800        |                               |
| Junior engineer              | 50  | $200          | $10,000      | revenue:consulting:delivery   |
| Friends and family discount  |     | -5%           | -$500        |                               |
|                              |     | **Subtotal**  | **$16,700**  |                               |
| Less deposit (Invoice 104)   |     |               | -$8,350      | liabilities:customer-deposits |
|                              |     | **Total due** | **$8,350**   |                               |

## Payment

Pay by ACH to account 123456789, routing 987654321, within 30 days.
```

**Frontmatter fields.** The keys are flat snake_case, MBR's convention, so
`grep -l 'status: sent'` works. Dates are ISO, and amounts never appear in
frontmatter.

| Field                            | Required | Meaning                                                                                                                       |
|----------------------------------|----------|-------------------------------------------------------------------------------------------------------------------------------|
| `type`                           | yes      | `invoice`. Later: `estimate`, `credit-note`.                                                                                   |
| `invoice_num`                    | yes      | The document number. Unique across the books, and the `invoice:` tag value in the journal. No commas.                         |
| `date`                           | yes      | The issue date, which is also the journal transaction date.                                                                   |
| `status`                         | yes      | Author intent: `draft` \| `sent` \| `void`. Estimates use `estimate-open` \| `estimate-closed`.                               |
| `template`                       | yes      | MBR's per-file template path (`/`-rooted, `.html`). MBR itself renders the styled invoice.                                    |
| `customer`                       | yes      | Wikilink to the customer note. This is the stable link. The `invoicee_*` fields are a snapshot of what was sent.              |
| `invoicee_name`, `_email`, `_phone`, `_address` | yes  | A snapshot copied from the customer at creation. Edits to the customer later don't change sent invoices. Use `\|` (literal) for addresses, never `>`, which folds lines. `_phone` is optional (copied when the customer has `phones:`). |
| `invoicer`, `invoicer_*`         | yes      | The same pattern for your own company.                                                                                        |
| `terms`                          | no       | Net days (integer). The customer's default is copied in.                                                                      |
| `due_date`                       | no       | Written for grep and templates: `date + terms` unless set by hand. Overdue is always derived from it, never stored.          |
| `reference`                      | no       | Free text the customer quotes back: `PO #PY75334`. (This was `po_num` in the first sketch.)                                   |
| `paid`                           | mirror   | `no`, then `partial 2026-10-28 $1,500, 2026-11-19 $3,500`, then `2026-11-19` when paid in full. Written only by the reconciler (§5.4). |
| `sent_date`                      | mirror   | When the status first became `sent`.                                                                                          |
| `sent_via`                       | no       | How it was delivered: `email`, `portal`, `mail`, … Set by Send, editable.                                                     |
| `void_date`                      | mirror   | When it was voided.                                                                                                           |
| `deposit_for`                    | no       | On a deposit invoice: what it is a deposit on (free text or a wikilink to an estimate).                                       |
| `stripe_id`, `stripe_url`, …     | no       | Later: sync identity (§7).                                                                                                    |
| *anything else*                  | no       | Custom fields. Templates see them, MBR autocompletes them, and the invoicer keeps them byte-for-byte.                         |

`check` reports a missing required field as an error, except the snapshot
emails and addresses (`invoicee_email`, `invoicee_address`, `invoicer_email`,
`invoicer_address`), which are warnings: a customer without one can still be
invoiced.

**Title and filename.** The default filename pattern is
`{date} {customer} Invoice {invoice_num}.md`, and `title` mirrors it. Both are
configurable in `[invoicer]`.

**Things the first sketch used that change.**
- `po_num` becomes `reference`.
- `term` was a duplicate of `terms`; only `terms` stays.
- `invoicee_name: "[[Customer X]]"` splits into a `customer:` link plus a
  plain-text snapshot name, because the PDF must print a name, not a link.
- `>` addresses become `|`.
- The "Less 50% now" row became a deposit invoice (§4.4).

### 3.3 The bill table

The **bill table** is the first table in the body whose header has an
`Amount` column. Other tables (timesheets, usage, payment history) are
supporting tables. The invoicer leaves them alone, and templates show them
as-is. Prose above and below the bill table is free markdown; the GUI edits it
with a markdown editor (§6.3).

**Columns are recognized by header text**, case-insensitively, with aliases.
Order is free, except that the books column comes last. The aliases below are
defaults. Each list can be replaced in `[invoicer.words]` (§3.8), and the
first entry is the header the invoicer writes in new tables.

| Role        | Header aliases                               | Notes                                                         |
|-------------|----------------------------------------------|---------------------------------------------------------------|
| description | Description, Item, Service, Product          | Required.                                                     |
| quantity    | Qty, Quantity, Hours, Units                  | Decimal.                                                      |
| rate        | Rate, Price, Unit price, Unit cost           | Money, or a signed percentage on adjustment rows.             |
| amount      | Amount, Total, Line total                    | Required. Money, written by us.                               |
| date        | Date, Service date                           | Optional.                                                     |
| unit        | Unit                                         | Optional (`h`, `ea`, `mo`).                                   |
| item        | SKU, Code                                    | Optional. A catalog key.                                      |
| books       | *Account* (italic header, **last** column)   | Optional. The accounting for the row; hidden when rendered.   |

**Row kinds.** These are inferred from content, so a hand-written table
classifies the way a reader would expect.

| Kind       | How it's recognized                                                                 | Its amount                                                                                         |
|------------|-------------------------------------------------------------------------------------|----------------------------------------------------------------------------------------------------|
| spacer     | every cell blank or `&nbsp;`                                                         | none                                                                                               |
| total      | a cell is bold and its text is one of the total words (default `Subtotal`, `Total`, `Total due`, `Balance due`, `Amount due`; configurable) | the running total of every non-total row above it, written bold |
| adjustment | no quantity, and the rate is a signed percentage (`-10%`, `8.25%`)                   | that percentage of the **nearest row above that is an item or a total**, skipping other adjustments |
| fixed adj. | no quantity, no rate, and a signed amount                                            | as written                                                                                         |
| item       | everything else                                                                      | `qty × rate` when both are present, otherwise as written                                           |

This gives QuickBooks' positional semantics with no extra syntax:

- a discount under an item discounts that item;
- a discount under a Subtotal discounts everything above it;
- a sales-tax row (`Sales tax | | 8.25% | $1,377.75 | liabilities:sales-tax`)
  under the Subtotal is tax after discounts;
- placed higher up, the same tax row taxes only the item above it.

The last total row is **the** total, which is the amount booked and the
amount due.

**Arithmetic.** All of it uses `ledgeline_core::decimal::Dec`, never floats.

- Each computed amount is rounded to the commodity's precision as it is
  computed, half away from zero.
- Totals are sums of the rounded row amounts, so the table always adds up to
  the cent.
- Money is parsed and formatted with the journal's commodity style
  (`JournalStyles`), so `$8,000` and `-$800` match how the books write `$`.
- An invoice is one commodity. Multi-currency is later.

**Recalculation.** Amounts in the table are written values. On every
save the editor recomputes all computed amounts. `ledgeline-invoicer recalc`
does the same from the CLI. `ledgeline-invoicer check` reports a table whose
written amounts disagree with its own arithmetic, for example after a hand
edit, without changing anything. The app recalculates a draft after an
outside edit settles (`[invoicer] recalc-on-save = "drafts"`, the default,
or `"never"`), only if the file is still what it read; a sent invoice it
never rewrites on its own: the invoice page shows the mismatch, and its
Recalculate re-books a sent invoice after a confirmation (PLAN F10).

**The books column.** Its header is italic and it is the last column. The
cell grammar is:

```text
<account>
<deferred-account> → <recognize-into>, <N> months [from <YYYY-MM-DD>] [calendar]
```

`->` is accepted for `→`. A deferred cell maps one-to-one onto plan 45's
`Deferral { target_account, term: Months(N), service_start, proration }`. A
blank cell falls back through these defaults, in order:

1. **Adjustment rows** take the account(s) of the row they apply to. For a
   percentage of a subtotal, that means pro rata across the subtotal's
   **revenue lines**, meaning the lines that come from item rows (an item
   plus the adjustments targeting it, deferred items included). Tax,
   deposit and other rows with their own account never share a discount.
   If `[invoicer] discount-account` is set, negative adjustments go there
   instead (decided 2026-11-19).
2. The row's catalog item (§3.6).
3. The customer's `revenue_account`.
4. `[invoicer] revenue-account`.
5. The single account typed `R` or carrying `kpi: revenue`.

If none of these resolves, the row is an error the editor shows.

**Edge rules.** These were settled while building the engine (2026-11-19),
and the table engine's goldens pin them:

- **Headers.** Two columns that map to the same role are a header error.
  `Item` is a description header; catalog codes use `Code` or `SKU`.
- **Fixed adjustment vs. item.** A row with no quantity and no rate is a
  fixed adjustment only if its amount carries an explicit sign (`-$5,000`,
  `+$12`). Otherwise it is an item, as written (`Shipping | | | $12`).
- **Percent signs** are optional on adjustment rates (`8.25` and `8.25%` are
  the same).
- **Percentage base.** A percentage applies to its target's own amount (not
  net of earlier discounts) or to the total's amount. Spacers and fixed
  adjustments are skipped when finding the target.
- **Totals** are bold cells, in any column except Amount and books, whose
  text is a subtotal or total word. A total sums every non-total row above
  it. Rows below the final total are allowed in the document but refused
  at booking.
- **Writing amounts.** The sign goes before the symbol (`-$250.00`). A cell
  whose value already matches keeps its text, and hand-written amounts are
  never re-rounded.
- **Cells.** Non-blank cells beyond the header's width are an error, since
  GFM would silently drop them.

**Hiding it.** Every invoice template ships this CSS, and anyone using a
different renderer can paste it:

```css
/* The books column: last column, italic header. Works in any engine with :has(). */
table:has(thead th:last-child > em:only-child) :is(th, td):last-child { display: none; }
```

`display: none` is used deliberately. `visibility: collapse` on cells is
honored inconsistently by WebKit and Blink, and an `nth-child(5)` rule breaks
as soon as the column count varies. Later, MBR can emit a `data-col` slug on
cells (MBR-CHANGES M7), which makes this more robust, but the rule above
doesn't depend on it.

**Worked example.** This table uses every row kind. You type descriptions,
quantities, rates and accounts; the invoicer writes the Amount column (by
hand you can type it too, and `check` tells you if your arithmetic is off).

```markdown
| Description                | Qty | Rate          | Amount         | *Account*                                                                  |
|:---------------------------|----:|--------------:|---------------:|:---------------------------------------------------------------------------|
| Website redesign           | 1   | $12,000       | $12,000.00     | revenue:projects                                                           |
| Senior architect hours     | 10  | $250          | $2,500.00      | revenue:consulting                                                         |
| Loyalty discount           |     | -10%          | -$250.00       |                                                                            |
| Annual support plan        | 1   | $6,000        | $6,000.00      | liabilities:deferred-revenue → revenue:support, 12 months from 2026-12-01  |
|                            |     |               |                |                                                                            |
|                            |     | **Subtotal**  | **$20,250.00** |                                                                            |
| Early-payment discount     |     | -2%           | -$405.00       |                                                                            |
|                            |     | **Subtotal**  | **$19,845.00** |                                                                            |
| Sales tax                  |     | 8.25%         | $1,637.21      | liabilities:sales-tax                                                      |
| Less deposit (Invoice 104) |     |               | -$5,000.00     | liabilities:customer-deposits                                              |
|                            |     | **Total due** | **$16,482.21** |                                                                            |
```

| Row                    | Kind       | Its amount                                                                                                  |
|------------------------|------------|-------------------------------------------------------------------------------------------------------------|
| Website redesign       | item       | 1 × $12,000                                                                                                 |
| Senior architect hours | item       | 10 × $250                                                                                                   |
| Loyalty discount       | adjustment | 10% of the nearest item above, so only the architect hours                                                  |
| Annual support plan    | item       | $6,000, booked to deferred revenue and recognized at $500 a month from December                             |
| (blank)                | spacer     | none                                                                                                        |
| **Subtotal**           | total      | the sum of every non-total row above it                                                                     |
| Early-payment discount | adjustment | 2% of the subtotal above it, so the whole bill. Its blank account spreads it pro rata over the accounts above |
| **Subtotal**           | total      | $19,845. This row is what makes the tax come after the 2% discount; without it, the tax would skip the adjustment and use $20,250 |
| Sales tax              | adjustment | 8.25% of $19,845, rounded to the cent                                                                       |
| Less deposit           | fixed adj. | as written; it uses up the deposit invoice's liability                                                      |
| **Total due**          | total      | the last total: the amount booked and the amount due                                                        |

The customer sees the first four columns only. In accrual books, sending
it books (tags trimmed for reading; see §4.2):

```journal
2026-11-17 Customer X | Invoice 106  ; invoice: 106, document: invoices/2026-11-17 Customer X Invoice 106.md, due: 2026-12-17
    assets:accounts receivable       $16,482.21
    revenue:projects                $-11,760.00   ; 12,000 − 240 (its 2% share)
    revenue:consulting               $-2,205.00   ; 2,500 − 250 − 45
    liabilities:deferred-revenue     $-5,880.00   ; 6,000 − 120, recognized monthly into revenue:support
    liabilities:sales-tax            $-1,637.21
    liabilities:customer-deposits     $5,000.00   ; applies the deposit
```

### 3.4 Customers

A customer is a normal MBR `type: person` or `type: organization` note,
**tagged `customer`**. The tag matters because contact sync puts every
contact in the vault, and only some of them are customers. MBR's tag page
then lists all customers for free. The contact fields (`emails`, `phones`,
`addresses`, `company`, …) follow MBR's existing schema (see
`mbr-apple-contact-sync/docs/markdown-schema.md`). Invoicing adds flat keys:

```yaml
---
title: Customer X
type: organization
tags: [customer]
emails:
  - billing: accounts_receivable@customerx.com
  - work: hello@customerx.com
phones:                             # MBR's labelled list, chosen as emails are
  - billing: "+1 (303) 555-0142"
addresses:
  - billing: |
      1234 Main St. #555
      Springfield, CO 80000
terms: 30
revenue_account: revenue:consulting
invoice_template: /invoices/tpl/myco-default.html
ledger_customer: Customer X         # the customer: tag value in the books; defaults to the title
contracts:
  - "[[Customer X MSA 2026]]"                                    # a markdown note
  - "/customers/contracts/Customer X SOW 3 renewal (signed).pdf" # an executed PDF
cadence: monthly
renewal_date: 2027-11-17
open_pos:
  - reference: "PO #PY75334"
    amount: $40,000
    expires: 2027-06-30
billing_portal:
  name: Coupa
  url: https://customerx.coupahost.com/
  login: finance@mycompany.com       # the username only; never a password
  supplier_id: SUP-44821
  requires_reference: true           # the portal rejects invoices without a PO
  instructions: |
    Upload the PDF under Invoices → Create from PO, then attach the timesheet.
stripe_customer: cus_Q1w2e3          # later
---

Fred Smith is the champion.
```

- **Billing contact.** The invoicer picks the email or address labelled
  `billing`, then `work`, then the first one. A person note's
  `company: "[[Org]]"` makes the org the default bill-to: the org's name, and
  its billing email and address when it has them (the person's otherwise).
  Addresses may be text or MBR's map (`street`, `city`, `region`,
  `postcode`, `country`), which is written as lines.
- **Identity in the books.** The books identify a customer by
  `ledger_customer`, falling back to the title. That value is the
  `customer:` tag, plan 37's attribution, so Ledgeline's SaaS metrics and
  plan 45's settlement ranking already use it. Renaming a note doesn't rename
  history unless the user asks; `ledgeline-invoicer customers rename` is
  later.
- **Contracts.** Each entry is either a wikilink to a note or a path to any
  file: `/`-rooted from the books folder, or relative to the customer note.
  Executed contracts are usually PDFs, so paths are the common case. The
  invoicer lists them in the customer view and opens them with the system
  viewer, and `check` reports paths that don't exist. Today MBR resolves
  neither form in frontmatter. MBR-CHANGES M1 asks for both, so the
  contract shows up in MBR as a link too.
- **Vendor portals.** Some customers take invoices only through a portal
  (Coupa, Ariba, Tungsten, and so on). `billing_portal` records how:
  - `name` and `url`;
  - `login`, the username only. Secrets never go in notes. If a password is
    stored at all, it goes in the OS keychain, as Ledgeline's `keychain`
    feature already does.
  - `supplier_id`, your vendor number in their system;
  - `requires_reference`: when true, `check` and Send refuse an invoice with
    no `reference`;
  - `instructions`, free markdown.

  When a customer has a portal, the invoice's Send step changes. It shows
  the instructions, an **Open portal** button, the PDF (for drag or
  Share…), and copy buttons for the number, reference and total. The
  invoice then gets `sent_via: portal` next to `sent_date`.
- **Invoices link the customer.** No back-pointers are written into the
  customer note. Once MBR resolves frontmatter wikilinks (MBR-CHANGES M1),
  the customer's "Linked from N notes" lists its invoices automatically, and
  the invoicer shows the same backlinks in its customer view. Leaving the
  customer note untouched also avoids fighting contact sync over it.

### 3.5 Templates

A template is an HTML file with Tera directives. It is used through MBR's
per-file template mechanism (the `template:` key; MBR #387). So MBR renders a styled invoice everywhere:

- in MBR's browser;
- in `mbr -o` static builds;
- in MBR's PDF export;
- inside the invoicer.

The template receives MBR's context:

- the lossless `frontmatter` (nested maps, lists of maps, numbers);
- `markdown`, the rendered body HTML, which includes the bill table;
- the `url()` helper;
- MBR's partials, under `/.mbr/`.

The invoicer adds nothing magic: a template sees exactly what MBR sees. That
is the point, since a third party can render our invoices with plain `mbr`.

- **Discovery.** A template is any non-ignored `.html` file whose source
  mentions `invoice_num`. Its display name and kind can come from a Tera
  comment on its first line, for example
  `{# invoicer: name="MyCo default" kinds="invoice,estimate" #}`. Tera
  comments aren't HTML comments, and this is an HTML file anyway. With no
  such comment, the name is the file stem and the kind is `invoice`.
- **Starter templates.** The invoicer ships two (Classic, Modern). Each is
  copied into `invoices/tpl/` the first time an invoice uses it, and from
  then on it is the user's file. Each starter includes:
  - print CSS (`@page` size and margins);
  - the books-column CSS;
  - MBR's export-ready script (MBR-CHANGES M5);
  - MBR's chrome only where it works: see "MBR's chrome" below.
- **MBR's chrome.** Served by MBR (`server_mode`), a template includes MBR's
  partials (`/.mbr/_head_markdown.html`, `_nav.html`, `_breadcrumbs.html`,
  the previous/next footer, `_footer.html`, `_scripts.html`), so an invoice
  opened in MBR keeps its sidebar, search and navigation, and the sheet sits
  in `<main>`. The sheet alone shows: in a standalone render (`/api/preview`,
  QuickLook: `server_mode` is false); where scripts are off (the app's
  frame, B2), by a `<noscript>` rule hiding every child of `<body>` but
  `<main>`; and on paper, by MBR's own print rule doing the same. The
  sheet pins Pico's variables so MBR's stylesheet doesn't restyle it.
- **Template editor.** The GUI has a code editor with live preview against a
  sample invoice. This is the "customize templates" path.
- **App theme vs. invoice look.** The app's theme (purple, catppuccin, …)
  styles the invoicer UI. An invoice's look is whatever its template says,
  because it's customer-facing and must not change with the user's dark mode.

### 3.6 Catalog (products and services)

The catalog is optional. When it exists, it is a markdown note whose body is
one table, using the same grammar and the same books column:

```markdown
---
title: Catalog
type: catalog
---

| Code     | Description              | Unit | Rate   | *Account*                                                    |
|:---------|:-------------------------|:-----|-------:|:-------------------------------------------------------------|
| ARCH     | Senior architect hours   | h    | $800   | revenue:consulting                                           |
| JR       | Junior engineer          | h    | $200   | revenue:consulting:delivery                                  |
| SAAS-Y   | Platform subscription, 1 year | ea | $12,000 | liabilities:deferred-revenue → revenue:saas, 12 months |
```

Choosing a catalog item fills the row. The resolved account is still written
into the invoice's books column, so the invoice is self-contained and a later
catalog change doesn't rewrite history. Ad-hoc rows need no catalog entry at
all; they use the default account.

### 3.7 Snippets

Payment instructions, terms paragraphs, late-fee wording and thank-you notes
are `type: snippet` notes:

```yaml
title: Net 30 ACH
type: snippet
snippet_kind: payment      # payment | terms | note
```

The note's body is markdown. Inserting a snippet **copies** its text into
the invoice. It is not a live include, because a sent invoice must not change
when a snippet does. A customer note can name default snippets, for example
`snippets: ["[[Net 30 ACH]]"]`.

### 3.8 Settings

There are no new config files. This follows the rules in
[Settings files are TOML] and plan 43's precedent for `[sync]`.

| Setting                                                                                         | Where                              | Written by                                                              |
|-------------------------------------------------------------------------------------------------|------------------------------------|-------------------------------------------------------------------------|
| `invoices-dir`, `customers-dir`, `templates-dir`, `catalog`, `company`                          | `ledgeline.toml [invoicer]`        | Hand, or the first-run setup action (an explicit action, like `[sync]`) |
| `receivable-account`, `revenue-account`, `deposit-account`, `discount-account`                  | `ledgeline.toml [invoicer]`        | Hand / first-run setup                                                  |
| `cash-deposits`, `payment-allocation` (accounting policy; PLAN "Configurable accounting flows") | `ledgeline.toml [invoicer]`        | Hand                                                                    |
| `recalc-on-save` (`drafts` or `never`; PLAN F10)                                                | `ledgeline.toml [invoicer]`        | Hand                                                                    |
| `filename`, `number-start`, `pdf-dir`                                                           | `ledgeline.toml [invoicer]`        | Hand                                                                    |
| Recognized words (below)                                                                        | `ledgeline.toml [invoicer.words]`  | Hand                                                                    |
| `basis`, `closed-through`, `recognition-file`                                                   | `ledgeline.toml [books]` (plan 45) | Shared with Ledgeline                                                   |
| theme, uiScale                                                                                  | `ui-prefs.toml` + `[ui]` pin       | Shared with Ledgeline                                                   |
| invoicer UI prefs (default template, list sort…)                                                | `ui-prefs.toml [invoicer]`         | Settings screen                                                         |

Unset accounts are inferred the same way plan 45's gate does it: by declared
type or `kpi:` first, and by name only as a fallback. `deposit-account`
defaults to `liabilities:deferred-revenue`, or to the account carrying
`kpi: deferred-revenue` when there is one. `pdf-dir` defaults to
`invoices/pdf/` (§5.3).

**Recognized words.** Wherever the invoicer recognizes something by its
English text, that word list is a setting. Setting a key replaces its whole
default list. The first entry is the word the invoicer writes when it
creates that thing. `ledgeline-invoicer config` prints the effective lists.

```toml
[invoicer.words]
description = ["Description", "Item", "Service", "Product"]
quantity    = ["Qty", "Quantity", "Hours", "Units"]
rate        = ["Rate", "Price", "Unit price", "Unit cost"]
amount      = ["Amount", "Total", "Line total"]
date        = ["Date", "Service date"]
unit        = ["Unit"]
item        = ["Code", "SKU"]
subtotal    = ["Subtotal"]                                  # total rows the editor writes mid-table
total       = ["Total due", "Total", "Balance due", "Amount due"]  # first = the final total row
deposit     = ["Less deposit"]                              # label for an "apply deposit" row
tax         = ["Sales tax"]                                 # default label for a tax row
customer-tag   = "customer"                                 # the tag marking a contact as a customer
billing-labels = ["billing", "work"]                        # email/address labels, in preference order
```

Some words stay fixed on purpose, because they are the file format that
third-party scripts and MBR rely on:

- frontmatter keys (`invoice_num`, `status`, `paid`, …);
- `type` values (`invoice`, `estimate`, `person`, `organization`, …);
- status values (`draft`, `sent`, `void`) and the `paid:` grammar;
- journal tag names.

The books column needs no words at all: it is found by position and its
italic header.

## 4. Accounting

### 4.1 What gets booked, and when

| Event                         | Accrual books (`basis = "accrual"` or inferred)                                                                 | Cash books                                                              |
|-------------------------------|-----------------------------------------------------------------------------------------------------------------|-------------------------------------------------------------------------|
| Draft created or edited       | Nothing.                                                                                                        | Nothing.                                                                |
| Marked **sent** (or "Export final PDF" → yes) | One transaction dated `date`: AR debit for the total; each row's credit to its books account; deferred rows also get their recognition schedule in `recognition.journal`. | Nothing in the books. The invoice is tracked from its markdown (`status: sent`, `paid: no`). |
| Payment arrives               | Ledgeline imports or syncs it. Plan 45 settlement matching links it to the invoice (`match:`). The invoicer's reconciler mirrors `paid:`. | **Apply payment:** pick the deposit Ledgeline imported or synced, and its income side is re-split into the invoice's row accounts, tagged `invoice:`. The invoicer never creates a payment transaction, which would duplicate the bank import. |
| Edited after sent             | Re-book: the transaction is replaced and the schedules restated through `ops::save` (§4.5).                     | The markdown changes only (before payment).                              |
| **Void**                      | A reversing transaction dated the void date, tagged `invoice:` and `void:`, linked to the original AR posting with `match:` so the AR nets to zero. | `status: void`.                                                         |
| Recognition periods           | Plan 45's due entries, posted by Ledgeline (or `ledgeline-invoicer recognize`), never after today.              | n/a                                                                     |

Details settled in Phase 2b (2026-11-21):

- Sending an invoice dated on or before `closed-through` is refused.
- Send writes `status: sent`, `sent_date` (when absent), `sent_via` (when
  given) and `paid` (when absent): `no`, or the invoice date for a zero
  total.
- Voiding a draft changes only its markdown. A cash-books invoice with a
  payment applied can't be voided until the payment is undone.
- In cash books, a deferred row's payment goes to the account it recognizes
  into. A payment that settles the rest takes each row's remainder, so the
  rows end exactly at what is due on them.
- In cash books a deposit is income on receipt (§4.4).

**Drafts never touch the books** (decided 2026-11-19). "Expected billings"
can still be reported from draft files without polluting the journal.

### 4.2 The transaction an invoice becomes

For the invoice in §3.2, in accrual books:

```journal
2026-11-17 Customer X | Invoice 105  ; invoice: 105, document: invoices/2026-11-17 Customer X Invoice 105.md, due: 2026-12-17, reference: PO #PY75334
    assets:accounts receivable          $8,350.00  ; customer: Customer X
    revenue:consulting                 $-7,200.00  ; customer: Customer X, item: Senior architect hours
    revenue:consulting:delivery        $-9,500.00  ; customer: Customer X, item: Junior engineer
    liabilities:customer-deposits       $8,350.00  ; customer: Customer X, item: Less deposit (Invoice 104)
```

- **One posting per resolved account.** Each item row nets its own
  adjustments ($8,000 − $800). Pro-rata subtotal adjustments are spread over
  the revenue lines (§3.3) by amount and rounded, with the remainder going to the largest posting.
  Postings with the same account and the same deferral merge. So the journal
  stays readable while each line still carries `customer:`.
- **Tags.**
  - `invoice:` is the link to the invoice. It is the strongest match signal in
    plan 45's D-settle, and it goes on the payment too when we write the
    payment.
  - `document:` is the relative path to the markdown, so the books point at
    the file (and later at estimates or replaced drafts).
  - `due:` and `reference:` give Ledgeline aging and search without reading
    markdown.
  - `item:` keeps the row description on the posting: the item rows the line
    came from, joined with ` + `, or, for a line no item feeds (tax, a
    deposit applied), its own rows.
  - Tag values can't contain commas. The bridge drops a comma between digits
    (`$16,700` → `$16700`) and turns every other comma, and any line break,
    into a space. (`reference:` and the `document:` path are the likely
    offenders.)
- **Description and customer.** The description is `<invoicee_name> |
  Invoice <number>`. `customer:` is the customer note's `ledger_customer`
  (else its title); a person at a `company:` with no `ledger_customer` of
  their own books to the company's note (decided 2026-11-21).
- **Plan 45 vocabulary.** Deferred rows add `recognition:`, `service-start:`,
  `term-months:`, `proration:` and `contract:` on their posting, plus the rule
  group in `recognition.journal`. They are built by
  `accrual::invoice::build_document` and written by
  `Books::load()?.save(SaveRequest{…})`. One call writes the invoice and its
  schedules, behind the `ExternalChange` guard, so a running Ledgeline and the
  invoicer can't clobber each other.
- **Placement.** `InsertPosition::DateOrdered`, the way Ledgeline does it, so
  per-year include files receive the transaction.

### 4.3 Finding an invoice's transaction again

There is no stable transaction id in `ledgeline-core`: a `Tindex` shifts on
any insert. The invoicer re-finds a transaction after every load by the
header tag `invoice:` (an exact match, `^105$`) on a transaction with a
receivable posting. That is unique by construction, because `invoice_num` is
unique. Its `transaction_fingerprint` is the optimistic-concurrency token for
a replace.

A final invoice its deposit fully covers has no receivable posting (§4.4),
so Ledgeline never finds it; the invoicer finds it by `invoice:` plus
`document:` on a transaction with no settlement leg.

`check` reports these problems (`reconcile::audit`, Phase 2c), each with a
fix-it, a command where one fixes it (`FixEdit::Command`):

| Finding                                                                  | Severity | Fix                                                    |
|--------------------------------------------------------------------------|----------|--------------------------------------------------------|
| An invoice number booked twice                                           | error    | remove the extra transaction                           |
| A sent invoice with no transaction (accrual, or a cash zero total)       | error    | `send N` books it                                      |
| A transaction (booking, payment applied, void) with no invoice file      | warning  | restore the file, or leave it if another app booked it |
| A `document:` that names another path                                    | warning  | `rebook N`; by hand in a closed period                 |
| A file name a tag can't hold exactly (a comma, a run of spaces)          | warning  | rename it, then re-book                                |
| A draft already booked as its file says (a send stopped after the books) | error    | `send N` finishes it                                   |
| A draft booked differently                                               | error    | mark it sent and re-book, or remove the transaction    |
| Void in the file, not reversed in the books                              | error    | by hand: `status: sent`, then `void N`                 |
| Reversed in the books, sent in the file (a void stopped after the books) | error    | `void N --date D` finishes it                          |
| Cash books: void with payments applied                                   | warning  | `apply-payment N --undo`                               |
| Booked differently from what the file now says (§4.5)                    | warning  | `rebook N`; in a closed period, undo the edit or void  |
| A booking that can't be derived (an account that doesn't resolve)        | error    | fix the row                                            |
| `paid:` disagrees with the books                                         | warning  | the edit that sets it (`reconcile` does it)            |

### 4.4 Deposits (partial up-front billing)

"Bill 50% up front" is **not** a discount, so it is a separate **deposit
invoice** (decided 2026-11-19):

| Invoice 104 (deposit)                            | Amount  | *Account*                     |
|--------------------------------------------------|--------:|-------------------------------|
| Deposit: 50% of proposal P-12 ($16,700)           | $8,350  | liabilities:customer-deposits |

Invoice 104 books AR against the deposit liability. In accrual books that
could equally be deferred revenue, since the work isn't delivered yet; the
row's books column decides. The final invoice 105 bills the full scope and
applies the deposit with a fixed adjustment row whose account is the same
liability (as in §3.2). That debits the liability, so it clears.

The editor's **Apply deposit** button lists the customer's open deposits:

- each is a credit on `deposit-account` with that customer's `customer:`
  tag, less what has been applied from it;
- an invoice's applications are counted from its file (its rows booked to
  the deposit account), never from the journal, so a draft, or an unpaid
  invoice in cash books, already uses its deposit up; a void invoice counts
  for nothing;
- an application uses up the deposit its row names first (`Less deposit
  (Invoice 104)` names 104), then the oldest;
- choosing one writes the row into the draft, above its final total, and
  recomputes the table. The row's description is the back-reference;
  `deposit_for` keeps what the author wrote (decided 2026-11-21).

`invoicer_core::books` implements this (`open_deposits`, `apply_deposit`).

**A deposit that covers the whole final invoice** leaves a total of zero.
In accrual books, sending it books the rows against the deposit liability
with **no receivable posting**: there is nothing to receive, and
Ledgeline's receivables and matching ignore a zero settlement line by
design (decided 2026-11-21).

**Cash books: a deposit is income on receipt** (Patrick, 2026-11-21).

- The deposit invoice is booked only when paid, and its payment goes to
  revenue, never the liability: the row's own account, or, when the row
  names the deposit account, the customer's `revenue_account`, else
  `[invoicer] revenue-account`, else the books' single revenue account.
- A final invoice's deposit row books nothing. By default
  (`payment-allocation = "match-rows"`) its payments go where the rows
  still lack revenue once the deposit is counted, so each account ends at
  its row; `"pro-rata"` splits every payment by the rows' proportions.
- `cash-deposits = "liability-until-final"` keeps the earlier treatment
  instead: the deposit is held in the deposit account until the final
  invoice's payment (or its send, when fully covered) clears it.
- A fully covered final invoice books a `$0` marker at send: its header
  tags and a `$0.00` posting per row, each with `customer:` and `item:`.
  It is how the books show the invoice was sent; it is paid on its date.
- Open deposits are what deposit invoices were paid, less the applications
  in the files, so voiding a final invoice gives its deposit back.

### 4.5 Editing after sending

Editing a sent invoice is allowed (decided 2026-11-19). On save, the invoicer
rebuilds the `DocumentSpec` and does the following:

1. **Unpaid, all dates after `closed-through`:** replace the transaction
   (`SourceChange::Replace` with its fingerprint) and restate its schedules
   (plan 45 D4).
2. **Paid or partially paid:** warn ("the payment link will be re-checked").
   Replace the transaction while keeping the `match:` tag on the AR posting.
   If the AR amount changed, settlement drops to "check", and the reconciler
   recomputes `paid:`.
3. **On or before `closed-through`:** refuse to rewrite the original. Offer
   either a **credit note** or **void + reissue**. Credit notes are a later
   document type; void + reissue is available from v1.

**Hand edits.** On load, the reconciler compares each sent invoice's
markdown, re-derived as a `DocumentSpec`, with its transaction. If they
differ, it shows a "Books out of date" row with **Re-book** or
**Keep books**. Nothing is re-booked silently.

### 4.6 Receivables view

The dashboard and the open-invoice lists come from the books, not the
markdown:

- **Open invoices:** AR debits tagged `invoice:` whose settlement is
  incomplete.
- **Partial payments:** an invoice settled by several payments (N:1). This
  is a `ledgeline-core` extension of plan 45's 1:1 settlement (§8).
- **Aging buckets:** Current, 1–30, 31–60, 61–90, 91+, by `due:` (or `date`
  plus terms).
- **Days sales outstanding (DSO), oldest overdue, average days to pay:**
  computed from settled pairs.
- **Drafts and cash-basis sent invoices:** from the markdown, labelled as
  such.

This lives in `ledgeline-core` as a new `receivables` module, so Ledgeline
can show the same aging (plan 45 lists aging as "later") and the invoicer is
one more reader.

## 5. Core behaviours

### 5.1 Numbering

The next number is `max(invoice_num in files, invoice: tags in the books) + 1`,
or `[invoicer] number-start` when there are none. It is assigned when the
invoice is created (drafts have numbers), and it stays editable. Duplicates
are an error at save. Numbers aren't forced to be integers: if the maximum
is `2026-0042`, the next is `2026-0043`, because a trailing digit run
increments. There is no counter file.

"In the books" means every `invoice:` tag on a transaction or a posting
(`numbering::booked_numbers`), so a number used by a void reversal or a
cash Apply payment still counts. One exception: a vendor's bill carries the
same tag in its own namespace (plan 45), so a transaction whose settlement
posting is payable-side is left out. In the accrual fixture, plan 45's
invoices 2001–2004 have no files and the bill `PA-9` is skipped, so the next
number is 2005.

### 5.2 Creating an invoice

1. Choose a customer and a template. The customer's `invoice_template` is
   the default template.
2. Snapshot the customer and company contact fields into frontmatter, and
   copy in `terms`, the default snippets and the `reference` (choosing from
   `open_pos`).
3. Build the bill table. For each row, pick a catalog item, enter an ad-hoc
   row, add a discount or subtotal, or apply a deposit. Later, pull in
   unbilled time.
4. Write prose above and below the table.
5. Save. The file is written atomically with a content-hash check, as MBR's
   editor does, and the status is `draft`. A new file first claims its name
   exclusively (an empty file), then is atomically replaced, conditional on
   still being that empty file: a crash leaves an empty note or the whole
   draft, and two writers can't both take the name. Every write is verified
   first: the text must read back with its frontmatter and a bill table that
   adds up.
6. **Send:** this marks the invoice sent, books it (§4.1), and offers the
   PDF.

### 5.3 PDF

- **Rendering.** A PDF is MBR's page for the invoice, with the template,
  printed by the OS webview (MBR #386). The invoicer already owns a tao event
  loop, so it calls MBR's public `pdf_export::ExportSession::start` with that
  loop and the invoice's URL on its own server. MBR opens a hidden webview,
  waits for the page's ready signal, and prints. Paper size and margins come
  from the template's `@page` rule, with config only as a fallback
  (MBR-CHANGES M4, M5).
- **Where it goes.** Every final export (sending, or exporting a sent
  invoice) is **kept**, in `pdf-dir` (default `invoices/pdf/`), named like the
  markdown: `2026-11-17 Customer X Invoice 105.pdf`. Re-exporting after an
  edit keeps the earlier file with its export date appended, so the folder
  is an audit trail of exactly what was sent. Users delete old ones whenever
  they like; nothing depends on them. Draft previews go to a temp file. A
  copy can also be saved anywhere else. The path is never written into the
  books.
- **Status prompt.** Exporting a draft asks: "Mark invoice 105 as sent?"
  Exporting a sent invoice after edits warns that the PDF differs from what
  was sent before; we keep a hash of the last exported body in `sent_hash`
  (a mirror field).
- **Drag out.** Drag the PDF from the invoicer into Mail or a browser upload
  field, and a temp file is created for the drag. This needs a native drag
  source, since wry has no drag-out API. On macOS that means
  `NSDraggingSource`, through the same objc bindings tao uses. It's a spike in
  the plan. The fallbacks are **Share…**, using `NSSharingServicePicker`
  (Mail and Messages), and **Copy file** (a file URL on the pasteboard).

### 5.4 Reconciler (the paid mirror)

The reconciler runs on load, on every journal or markdown change (through the
shell's watcher, as Ledgeline does), and from `ledgeline-invoicer reconcile`.
For each sent invoice it:

1. computes the payment state from the books' settlement links;
2. rewrites `paid:` if it differs, as one surgical frontmatter edit: one key,
   the rest of the file byte-identical;
3. reports any book/markdown mismatch (§4.5).

Ledgeline never writes markdown, so when a payment is matched in Ledgeline,
the invoicer picks up the change through its watcher, or on the next run.

**As built (Phase 2c).** `invoicer_core::reconcile`:

- **`BookStates`** reads the journal once: per invoice number, its
  bookings (a receivable document, or a zero-total booking or cash `$0`
  marker found by `invoice:` plus `document:`), its void reversals, and its
  payments. Accrual books take settlement from Ledgeline's `receivables`
  (N:1 links, voids apart); cash books take the deposits Apply payment
  tagged `invoice:` (no `document:`, no `void:`), each paying what its
  `customer:`-tagged postings off the cash accounts hold (the gross).
  Bills (payable side) are another namespace and skipped.
- **The mirror** (`BookStates::paid`): a zero total is paid on its date;
  accrual: open is `no`, partial lists the payments and credits (merged by
  day), paid is the date of the one that completed it; cash: the applied
  payments against the table's total. Amounts are written in the table's
  money style, whole amounts without cents. A written mirror that says the
  same (same dates, same amounts however written) is left alone.
- **`reconcile`** mirrors every sent invoice (or the numbers asked for)
  whose `paid:` differs, each file re-read and written only over the
  revision read, and returns the findings of `audit` (below) for the same
  numbers. A file changed meanwhile is reported as `failed` and left for
  the next run.
- **Incremental** (§5.6): `BookStates::changed_since(&before)` names the
  numbers a journal change touched, so the app reloads, diffs and
  reconciles only those (`reconcile_with`).
- **Open and overdue** (`list`, `show`, `customers`) read `BookStates`, so
  they are right before the mirror is refreshed; an invoice the books can't
  speak for falls back to its mirror.

### 5.5 Frontmatter editing

Rust has no YAML library that preserves formatting. The invoicer therefore
edits frontmatter **by key**:

- it replaces or inserts top-level `key: value` lines in the YAML block,
  including `|` blocks; a new key goes after the last one;
- it preserves order, comments and unknown keys, and a replaced one-line
  value keeps its trailing `# comment` and its quoting style;
- it verifies the result by re-parsing and comparing every other key, and
  the body byte for byte;
- it refuses, writing nothing, when the block's layout isn't plain
  top-level `key: value` lines (a top-level flow map, a `? ` key, a column-0
  comment inside a list). Those files are edited by hand.

Values are written plain when they read back as the same string (dates stay
`2026-11-17`), double-quoted otherwise (`"[[Customer X]]"`, `"PO #1"`), and
multi-line text as a `|` block. `|+` is never written, since it would absorb
blank lines after it. Nested maps are written in key order. Frontmatter is
read as MBR reads it: YAML 1.2 (`no` is text), integers as numbers, floats as
`f64`. `invoice_num` is taken as written, so `0042` stays `0042`.

The same edit is how scripts are expected to modify invoices. It is exposed
as `ledgeline-invoicer set <invoice> key=value`.

### 5.6 Performance and correctness

Everything is local, so the invoicer should be faster than any hosted
invoicing app. The risk is re-parsing a lot of text over and over. These
budgets are release criteria. They are measured on the
`benches/` corpus described below, on an M-series Mac, in a release build:

| Interaction                                                   | Budget (p95)                  |
|---------------------------------------------------------------|-------------------------------|
| Open a books folder → dashboard and invoice list shown        | 300 ms                        |
| Keystroke in the bill table → amounts and books panel updated | 10 ms server time             |
| Keystroke → live preview re-rendered                          | 50 ms                         |
| Save a draft (hash check, atomic write, re-index)             | 20 ms                         |
| Send (book, write the journal, update the markdown)           | 300 ms (relaxed; goal 150 ms) |
| A journal change from Ledgeline → `paid:` mirrors refreshed   | 100 ms                        |
| CLI `list --json` / `check` on the corpus                     | 300 ms / 1 s                  |

The send budget is **relaxed to 300 ms** for now (Patrick, 2026-11-21):
almost all of it is Ledgeline loading, re-parsing and saving the whole
journal, which only L10 (incremental reload) or an in-place re-parse in
`ledgeline-core` can cut. Bringing it back to 150 ms is a later goal.

The corpus is 5,000 invoices, 500 customers and 10 years of journal. It is
produced by a deterministic generator that extends Ledgeline's
`benches/corpus.rs`.

**How the budgets are met:**

- **One index, held in memory.** The index maps each document path to its
  `(mtime, size, content hash)`, its parsed frontmatter and its bill-table
  summary (total, accounts, number). Lists, the dashboard and numbering read
  the index, never the files. The index is a cache and nothing else: it is
  rebuilt from the files at open, and never persisted, so principle 1 holds.
- **Incremental by default.** Watcher events re-parse only the changed
  files. A change to a customer touches only that customer. A journal change
  reloads through ledgeline-core, then re-runs the reconciler only for the
  invoices whose `invoice:` tags or settlement links changed. Ledgeline's
  reload cost on large journals is measured in Phase 1; if it misses the
  budget, making it incremental is a Ledgeline change (§8), which helps both
  apps.
- **Frontmatter first, bodies lazily.** The open-time scan reads frontmatter
  plus the bill table only. Prose is parsed only when an invoice is opened.
  This is how MBR's repo scan already works.
- **Recompute on spans, not documents.** The table engine keeps each cell's
  byte span. An edit recomputes one table's amounts and rewrites only the
  changed cells. No markdown re-render is involved: rendering happens only
  for the preview, from a string (MBR-CHANGES M3), and is debounced to the
  next animation frame.
- **No subprocesses on interactive paths.** Booking, settlement, receivables
  and reconciliation use ledgeline-core's own engine. `hledger` runs only in
  tests, which cross-check our results against it.
- **Parallel open.** The open-time scan reads and parses files on scoped
  threads, one chunk per core (rayon isn't in the lockfile, and the scan is
  one parallel map). Each file is read once: the scan keeps the text just
  long enough to index the documents' bill tables.
- **Benchmarks are tests.** `invoicer-core/benches` has criterion benches for
  the scan, table recompute, booking and reconciliation, in the same style
  as `ledgeline-core/benches`. A bench that regresses more than 20% against
  the stored baseline fails the gate. The baseline is committed
  (`benches/baseline.json`, the median per bench and the machine it was
  taken on), since criterion's own baselines live under `target/`.
  `just invoicer-bench-gate` runs the benches and then `examples/bench_gate`,
  which fails on any bench more than 20% slower or not run;
  `just invoicer-bench-baseline` re-records it.

**Measured (Phase 1b, 2026-11-20).** On the corpus (6,104 notes; a journal of
59,250 transactions, 9.2 MB), Apple M5 Max, `cargo bench` medians:

| Bench                              | Median   | Feeds the budget                                |
|------------------------------------|----------|-------------------------------------------------|
| Index open (walk, read, parse all) | 121 ms   | open → list (300 ms); with the journal, ~200 ms |
| `list --json` on a warm index      | 3.2 ms   | `list --json` (300 ms); cold, ~200 ms           |
| `check` on a warm index            | 3.8 ms   | `check` (1 s); cold, ~200 ms                    |
| Refresh one changed file           | 1.3 ms   | save a draft (20 ms)                            |
| Save (verify, write, re-index)     | 5.8 ms   | save a draft (20 ms)                            |
| Ledgeline journal open             | 74 ms    | open; a journal change                          |
| Ledgeline journal reopen           | 73–92 ms | a journal change → `paid:` refreshed (100 ms)   |
| Bill-table keystroke (100 rows)    | 0.17 ms  | keystroke (10 ms)                               |

Every budget holds, but the journal reload alone uses most of the 100 ms
for a journal change, which leaves the reconciler under 10–25 ms. That is
L10 in §8.

**Measured (Phase 2b, 2026-11-21).** Same corpus and machine:

| Bench                                   | Median | Feeds the budget                          |
|-----------------------------------------|--------|-------------------------------------------|
| `invoicer/preview` (load + derive)      | 95 ms  | the books panel (warm in the app: < 1 ms) |
| `invoicer/send` (load, book, write, md) | 291 ms | send (300 ms, relaxed; goal 150 ms)       |

Send's time is the journal: 72 ms to load it and about 190 ms in
`write_document`, which re-parses and saves the whole 9.2 MB file. The
invoicer's own part is under 30 ms. Getting back to 150 ms needs L10, or an
in-place re-parse in `ledgeline-core`. Both benches are gated in
`baseline.json` (preview 91.6 ms, send 289.1 ms, recorded 2026-11-21 with
other work running on the machine; only those two entries were added).

**Measured (Phase 2c, 2026-11-22).** Same machine; the corpus is v2 (its
bookings carry `item:`, as the bridge writes them, so every invoice is in
step):

| Bench                                                                                                       | Median | Feeds the budget                              |
|-------------------------------------------------------------------------------------------------------------|--------|-----------------------------------------------|
| `invoicer/reconcile_change` (a payment linked: reload, book states, diff, mirror one invoice, its findings) | 99 ms  | a journal change → `paid:` refreshed (100 ms) |
| `invoicer/books_check` (the books checks over 5,000 invoices, books loaded)                                 | 277 ms | `check` (1 s): with a cold open, about 0.6 s  |
| `invoicer/send`                                                                                             | 297 ms | send (300 ms, relaxed; goal 150 ms)           |

`reconcile_change` is 70 ms of Ledgeline's journal load, 27 ms building the
book states (Ledgeline's receivables and role inference over 59,250
transactions), 1 ms diffing them and 14 ms writing the one mirror with its
findings. It meets the budget with no margin; L10 (an incremental reload)
is what would give it room. `books_check` is mostly reading each booked
invoice's file again (about 90 µs each); deriving and comparing a booking
takes about 20 µs once the journal's account defaults are computed once per
pass (`JournalAccounts`: before, each invoice re-inferred the books' roles,
3.6 ms, and the pass took 15 s). Both are gated in `baseline.json` (only
those two entries were added, recorded with other work on the machine).

**Correctness is never traded for speed:**

- money is `Dec`, rounded per row, and totals add up to the cent;
- every surgical edit is verified by re-parsing and comparing what must not
  have changed;
- the index can always be rebuilt, and a debug check (`check --full`) rebuilds
  it from scratch and diffs it against the live one;
- booking results are cross-checked against real `hledger` output in the
  fixtures (PLAN Phase 2);
- proptests cover the table engine's round trip and arithmetic;
- every write carries a content hash or books generation, so a stale view
  can never overwrite newer data.

## 6. Applications

### 6.1 Desktop shell

The desktop shell is the same as Ledgeline's:

- tao window, wry webview, muda menus, rfd pickers;
- axum on an ephemeral `127.0.0.1` port with the bearer token and Host
  guard, from `ledgeline-shell`;
- the SPA embedded with rust-embed.

It opens a books folder exactly as Ledgeline does: the positional journal,
then recents. File → Open swaps state. The bundle is
`Ledgeline Invoicer.app`, with bundle id `com.zmre.ledgeline.invoicer`.

**Routing inside the one server:**

| Path           | Served by                                                                                                      |
|----------------|----------------------------------------------------------------------------------------------------------------|
| `/app/*`       | The invoicer SPA (SvelteKit `paths.base = "/app"`).                                                             |
| `/api/*`       | Invoicer API (token required).                                                                                 |
| everything else | MBR's `Server.router`, merged at the root, unmodified: notes, `/.mbr/*` assets, `site.json`, `links.json`, search. |

Putting MBR at the root and the SPA under `/app` avoids any need for MBR to
support a base path. The invoicer shows MBR pages in two ways:

- **Browse mode:** MBR itself (a browser tab or MBR's own app) with MBR's
  chrome, so the user can jump between invoices, customers and backlinks.
  The templates include that chrome when MBR serves the page.
- **Preview mode:** the invoice template page alone, which is what a
  customer sees: the app's frame runs no scripts, so the template's
  `<noscript>` rule hides the chrome; a standalone render has none.

A live preview of unsaved edits renders from a string (MBR-CHANGES M3). The
theme reaches MBR's chrome through MBR-CHANGES M8. The SPA frames a preview
by URL (`/.ledgeline-invoicer/preview`, behind MBR's gate and CSP), never
as `srcdoc`: a srcdoc document inherits the SPA's CSP, whose
`frame-ancestors 'none'` WebKit applies to it (PLAN, Phase 3 feedback).

### 6.2 HTTP API (sketch)

| Route                                         | Does                                                                          |
|-----------------------------------------------|-------------------------------------------------------------------------------|
| `GET /api/invoices?status=&customer=&open=`   | List: frontmatter, total, books state, paid mirror, aging bucket.             |
| `GET/PUT /api/invoices/{num}`                 | The structured document (frontmatter, rows, prose blocks), with its content hash. |
| `POST /api/invoices` · `/{num}/send` · `/{num}/void` · `/{num}/rebook` | Lifecycle (§4).                                      |
| `POST /api/invoices/{num}/pdf`                | Export to a path, or to a temp file for drag/share.                           |
| `POST /api/preview`                           | Render an unsaved document to HTML.                                           |
| `GET/PUT/POST /api/customers…`                | Customers (MBR contact schema + invoicing keys).                              |
| `GET /api/catalog`, `/api/snippets`, `/api/templates` | The library.                                                          |
| `GET /api/receivables`                        | Dashboard: outstanding bar, aging, overdue, DSO.                              |
| `GET /api/fields/{key}/values`                | Autocomplete for custom frontmatter fields.                                   |
| `GET /api/books`                              | Basis, accounts, gate, recognition file, closed-through.                       |
| `GET /api/events`                             | Server-sent change events after the watcher re-reads a change (live reload).  |

Every write returns the new content hash and the books generation, and
accepts the expected ones (`409` on mismatch), as Ledgeline's API does.

### 6.3 GUI

Two shared pieces carry over from Ledgeline: the `packages/ui` look (themes,
scale, container-query breakpoints) and the same native-mode contract.

- **Left sidebar.** Dashboard, Invoices, Estimates (later), Customers,
  Catalog, Templates, Snippets, Settings. It has a prominent **New…** split
  button (Invoice, Deposit invoice, Customer).
- **Dashboard.**
  - A FreshBooks-style horizontal bar of outstanding receivables: overdue in
    red, open in yellow. It has a grey scale on top, drawn to $10k even when
    empty, and the big outstanding total to its right.
  - Below it: aging buckets, the oldest overdue invoices, DSO, average days
    to pay, drafts waiting to be sent, and recognition due (from plan 45,
    linking to Ledgeline).
  - Charts use LayerChart and the dataviz palette rules, the same as
    Ledgeline.
- **Invoice list.** Filterable by status, customer and paid state, with
  overdue highlighted. Clicking a row opens the editor, and there is a
  quick-jump picker (MBR search).
- **Invoice editor.**
  - **Header:** customer combobox with inline "Add new", template, number,
    dates, terms, reference with `open_pos` suggestions, and custom fields
    with autocomplete from `/api/fields`.
  - **Bill table:** a structured grid. Rows can be dragged to reorder.
    Catalog combobox with inline "Add new". Row-type buttons (item, discount,
    subtotal, tax, apply deposit, spacer). The books column is shown as an
    "Account" picker plus a deferral popover (term, start, calendar) and
    hidden in previews. Amounts are always live-computed.
  - **Prose above and below:** MBR's editor component (MBR-CHANGES M6), or
    a CodeMirror markdown field as a stopgap.
  - **Right side:** a live preview, using the template without chrome.
  - **Books panel:** what will be booked, or what was booked. Postings,
    schedules, settlement and payments, re-using Ledgeline's transaction-card
    rendering.
- **Customer view.** The contact card (MBR render), invoicing fields, open
  POs, contracts, backlinked invoices with paid state, AR balance, and the
  last payment.
- **Templates.** A list, plus a code editor with live preview against a
  chosen invoice.

### 6.4 CLI

Everything scriptable has a command, and every command takes `--json`. It
opens the books the same way the GUI does: the positional journal path or
`$LEDGELINE_FIXTURE`, then recents.

```text
ledgeline-invoicer [JOURNAL]                         # GUI
ledgeline-invoicer new --customer "Customer X" [--template T] [--row "Desc|qty|rate|account"]... [--from-catalog ARCH:10] [--status draft|sent]
ledgeline-invoicer list [--status S] [--customer C] [--open] [--overdue]
ledgeline-invoicer show NUM
ledgeline-invoicer set NUM key=value...              # surgical frontmatter edit
ledgeline-invoicer recalc [FILE|NUM...]              # rewrite computed amounts
ledgeline-invoicer check                             # validate files vs arithmetic vs books; exit 1 on problems
ledgeline-invoicer send NUM | void NUM [--date D] | rebook NUM
ledgeline-invoicer pdf NUM [-o PATH]
ledgeline-invoicer apply-payment NUM [--txn REF]     # cash books: list candidates, or apply one
ledgeline-invoicer reconcile                         # refresh paid: mirrors
ledgeline-invoicer receivables [--aging]
ledgeline-invoicer next-number
ledgeline-invoicer config                            # effective settings, including [invoicer.words]
ledgeline-invoicer customers list|show|new|import-csv FILE
ledgeline-invoicer recur [--through DATE]            # later
ledgeline-invoicer recognize [--through DATE]        # plan 45 post_due, never after today
```

The CLI and the API share one library function per operation, as
`run_cli_import` does in Ledgeline.

**As built (Phase 1c, 2026-11-21).** The document commands are in:
`new`, `list`, `show`, `set`, `recalc`, `check`, `next-number`, `config`
and `customers list|show`. The rest arrive with Phase 2 (books) and 3 (PDF,
GUI). `main.rs` only parses, opens and prints; each command is one
`invoicer_core` function:

| Module                   | Holds                                                                                         |
|--------------------------|-----------------------------------------------------------------------------------------------|
| `invoicer_core::session` | `Session`: the vault, `[books]`, the journal's money styles and `invoice:` numbers, the index |
| `invoicer_core::views`   | `list`, `show`, `check`, `customers`, `customer_detail`, `config`: serializable shapes        |
| `invoicer_core::edit`    | `create` (`new`), `set`, `recalc`, and the `--row` / `--from-catalog` grammars                |

- **Opening the books.** `JOURNAL` (positional, before the command) or
  `-f JOURNAL` (anywhere), then `$LEDGELINE_FIXTURE`, then the most recent
  journal in the recents Ledgeline shares (`ledgeline_shell::recents`). The
  last two print a note on stderr. `$LEDGER_FILE` is never read. The CLI
  doesn't record recents; opening a books folder in a window does.
- **Today.** `--today DATE` overrides the local date. It is `new`'s default
  date and what "overdue" is measured against.
- **Exit codes.** 0 done; 1 `check` found errors (with `--strict`, warnings
  too); 2 the command couldn't run: a usage error, books that don't open, a
  document that isn't found, or a refused write.
- **`--json`.** One JSON value on stdout. Field names are a contract: they
  are added, never renamed. A failure prints `{"error": "…"}` and exits 2.
- **Paths** in output are vault-relative (`invoices/…md`). Documents are
  named by number or by path (vault-relative, local, `.md` optional).

| Command                  | `--json` prints                                                                                                                                                             |
|--------------------------|-----------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `list`                   | `[DocEntry]`: `path`, `title`, the header fields, `total`, `commodity`, `open`, `overdue`, `problems {errors, warnings}`                                                    |
| `show NUM`               | `DocEntry`'s fields plus `frontmatter` (every field, lossless), `table {commodity, total, rows, discrepancies}`, `prose_before`, `prose_after`, `problems`                  |
| `check`                  | `{documents, customers, errors, warnings, problems: [Problem]}`                                                                                                             |
| `new`                    | `{path, number, total, written, problems}`, plus `text` with `--dry-run`                                                                                                    |
| `set NUM k=v… --unset k` | `{path, number, changed: [key], written, problems}`                                                                                                                         |
| `recalc [NUM…]`          | `{changed: [{path, number, status, changes: [{row, line, description, before, after}], written}], skipped, failed}`                                                         |
| `next-number`            | `{next}`                                                                                                                                                                    |
| `config`                 | `{journal, books_folder, books {basis, closed-through, recognition-file}, invoicer {…, words}, warnings}`                                                                   |
| `customers list`         | `[{path, title, kind, ledger_customer, terms, billing, portal, invoices, open}]`                                                                                            |
| `customers show NAME`    | a `customers list` row plus `contact`, `revenue_account`, `invoice_template`, `contracts` (with `found`), `billing_portal`, `open_pos`, `snippets`, `documents`, `problems` |

The header fields are `doc_type`, `number`, `date`, `status`, `template`,
`invoicee` and `invoicer` (`{link, name, email, address}`), `terms`,
`due_date`, `reference`, `paid` (`{"state": "no" | "partial" | "full", …}`),
`sent_date`, `sent_via`, `void_date` and `deposit_for`. A table row is
`{row, line, kind, description, code, quantity, unit, rate, amount,
written, computed, applies_to, books, account, deferral}`; `kind` is `item`,
`adjustment`, `fixed-adjustment`, `total` or `spacer`. The
`ledgeline-invoicer/tests/snapshots/` files are worked examples of every
shape.

Rules the commands follow:

- **Open and overdue read the books** (Phase 2c; until then, the `paid:`
  mirror): open means sent and not paid in full by the books, or by the
  mirror when the books can't say. Overdue means open with `due_date`
  before today.
- **`check`** covers the documents, the customers, the catalog and (Phase
  2c) the books (§4.3). `list` and `customers` count the document checks
  only; `show N` adds N's books checks. `check --full` is left out, because
  a CLI run builds its index fresh, so there is nothing to diff.
- **`new --row 'DESC|QTY|RATE|ACCOUNT'`.** Trailing fields may be left off,
  and `\|` is a literal pipe. With no quantity, a percentage rate makes an
  adjustment (`Discount||-10%`). With no quantity, a money rate is the row's
  amount (`Less deposit||-$5,000|liabilities:customer-deposits`). A total
  word alone (`Subtotal`) writes a total row. `--from-catalog CODE[:QTY]`
  fills a row from the catalog. Rows keep their command-line order across
  both flags. `--field key=value` adds custom fields. `--dry-run` prints the
  draft without writing it.
- **`set` values.** `key=value` writes text. A whole number that reads back
  the same is written as a number, so `terms=45` is a number and
  `invoice_num=0042` stays text. `key:=JSON` writes any value.
- **`set` refuses** three edits that would make a document disagree with
  its books. `status` is a lifecycle step, left to send and void. `paid` is
  the reconciler's mirror (I7). `invoice_num` is how the books find a sent
  document (§4.3), so it can change only on a draft. A new number must be
  unused. A document whose bill table already fails verification can still
  have its frontmatter set, because `set` makes nothing worse.
- **`recalc`** with no arguments rewrites drafts only. Sent and void
  documents with wrong amounts are reported as skipped, and are rewritten
  only when named, since their books need re-booking after (Phase 2).
  Rewriting re-aligns the whole table.

**As built (Phase 2c, 2026-11-22).** The books commands are in: `send`,
`void`, `rebook`, `apply-payment`, `reconcile`, `receivables` and
`recognize`; `check` gains the books checks (§4.3). Each is one
`invoicer_core` call (`books`, `reconcile`, `receivables`).

| Command                                                  | `--json` prints                                                                                                                                                                           |
|----------------------------------------------------------|-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `send N [--via HOW]`                                     | `{number, path, basis, booked: {date, description, text} \| null, journal_written, markdown_written}`                                                                                     |
| `send N --dry-run`, `rebook N --dry-run`                 | `{number, path, status, basis, booking, receivable, would_book, booked, in_step, linked, generation}`                                                                                     |
| `void N [--date D]`                                      | `{number, path, basis, reversal, link, journal_written, markdown_written}`                                                                                                                |
| `rebook N`                                               | `{number, path, basis, changed, previous, booked, warnings, mirror}`                                                                                                                      |
| `apply-payment N`                                        | `{number, path, commodity, total, paid, remaining, candidates: [{reference, date, description, received, exact, by_number, by_customer, gap_days, text}]}`                                |
| `apply-payment N --txn REF`                              | `{number, transaction, received, applied: [[account, amount]], remaining, undo, undo_file, mirror, warnings}`                                                                             |
| `apply-payment N --undo [--txn REF]`                     | `{number, reference, deposit_date, deposit_description, restored_text, undo_file, mirror, warnings}`                                                                                      |
| `reconcile [N…] [--dry-run]`                             | `{basis, checked, unchanged, changed: [{path, number, before, after, paid, written}], failed: [{path, number, code, error}], errors, warnings, problems}`                                 |
| `receivables [--aging] [--dso-days D]`                   | `{basis, today, open: [{number, path, customer, account, commodity, date, due, amount, outstanding, days_past_due, bucket, source}], aging, dso, average_days_to_pay, unapplied, drafts}` |
| `recognize [--through D] [--include-closed] [--dry-run]` | `{today, through, written, entries: [{id, date}], adjustments, closed}`                                                                                                                   |

`mirror` is a `reconcile` change (`{path, number, before, after, paid,
written}`) or `null`; `paid` is `{"state": "no" | "partial" | "full", …}`.
Amounts in these shapes are plain decimals (`"8350.00"`) beside their
`commodity`. A books error prints `{"error": "…", "code": "…"}` (codes:
`not-found`, `ambiguous`, `invalid`, `booking`, `status`, `setup`,
`refused`, `external-change`, `stale`, `books`, `markdown-not-written`,
`markdown`, `frontmatter`, `storage`, `overflow`).

- **Retried once.** Every books write runs again, once, when the journal
  or the file changed between its read and its write (`external-change`:
  nothing was written). A note goes to stderr.
- **The mirror follows a payment write.** `apply-payment`, its `--undo`,
  and a `rebook` that changed the books refresh that invoice's `paid:` and
  report it as `mirror`.
- **Undo records** for Apply payment are kept in
  `.ledgeline-invoicer/undo/` in the books folder (PLAN 2c).
- **`receivables`** reads the books in accrual books (every receivable still
  owed, numbered or not, with Ledgeline's aging, DSO and days to pay) and
  the files in cash books (sent invoices less what is applied, labelled
  `markdown`). `--aging` adds the buckets to the text; `--json` always has
  them.
- **`recognize`** posts plan 45's due entries through `--through` (default
  today) and refuses a later date; entries due in a closed period are
  listed, and posted as one adjustment per schedule with
  `--include-closed`.
- **Exit codes.** `reconcile` exits 1 on error findings (like `check`) and
  2 when a mirror couldn't be written.

**Measured (2026-11-21).** On the §5.6 corpus, the release binary (on an
Apple M5 Max, wall clock, opening the journal and the index from scratch)
runs `list --json` in 0.24 s against a 300 ms budget, and `check` in
0.23 s against 1 s.

## 7. Future features this design leaves room for

| Feature                  | How it fits                                                                                                                                                       |
|--------------------------|-------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| Estimates                | `type: estimate`, `status: estimate-open\|estimate-closed`, `expires:`. Never booked. **Convert to invoice** copies it and sets `estimate: "[[…]]"` on the invoice. Scripts can report open estimate totals with grep or MBR's index. |
| Credit notes             | `type: credit-note`, `credit_for: 105`. Books the reverse lines and links to the invoice's AR with `match:`.                                                        |
| Recurring invoices       | A `type: recurring-invoice` note: a period expression in Ledgeline's periodic grammar (`every month from 2027-01-01`), a body that serves as the template, and `next_date`. `ledgeline-invoicer recur --through today` writes drafts (or sends), and optionally PDFs. |
| Time tracking            | Read hledger timedot through ledgeline-core (or hledger). The account prefix (`time:cust:acme:proj`) maps to customer and project. kairos-style rate rules (customer/project/person, longest whole-segment match) live in the customer note. Indented `#` lines become row detail. An `unbilled` view, with "bill these" filling rows. Billed entries are tagged `invoice:` in the timedot file. |
| Stripe sync              | Read-only first, with a restricted key in the keychain (Ledgeline already has a `keychain` feature). Customers, invoices, credit notes and quotes become local files with `stripe_id`/`stripe_customer`/`stripe_url` fields and `metadata.ledgeline_invoice` written back. Stripe-finalized invoices are read-only locally; a revision is a new file. Positional rows convert losslessly to Stripe's per-line discounts. Amounts are integer minor units, converted by currency exponent. |
| QuickBooks import        | Customers come from the Customer Contact List Excel/CSV export, through a column-mapping step reusing Ledgeline's import UI pieces (calamine is already a dependency). Invoices come from the Invoice List or Sales by Customer Detail reports, as best effort. The API/CDC importer is later still. |
| Email and reminders      | Out of scope for now. `sent_date`, `due_date` and the receivables view are what a reminder script needs.                                                          |
| Late fees                | A new dated invoice (never a back-dated line), using a late-fee snippet and a `late_fee_for: 105` reference.                                                     |
| E-invoicing (UBL/Factur-X) | The field names map onto EN 16931: `invoice_num`→BT-1, `date`→BT-2, `due_date`→BT-9, `reference`→BT-13, `terms`→BT-20, adjustments→BG-20/21, the deposit row→BT-113, the last total→BT-115. An exporter is a pure function of the parsed document. |
| iOS                      | `invoicer-ffi` mirrors `ledgeline-ffi`: the same router through a URL scheme, WKWebView `createPDF` for PDF, and the share sheet in place of drag. The layout already follows container-query breakpoints. |
| Third-party scripts      | The file formats in §3 are the API. A script that writes an invoice markdown file with `status: draft` gets a draft the GUI opens. `check --json` validates it, and `send` books it. |

## 8. Changes needed in Ledgeline (same repo; make them directly)

| #  | Change                                                                                                                                               | Why                                                    |
|----|------------------------------------------------------------------------------------------------------------------------------------------------------|--------------------------------------------------------|
| L1 | `DocumentLine` gains `memo`/extra posting tags (`item:`); `DocumentSpec` gains extra header tags (`document:`, `due:`, `reference:`)                  | Plan 45's builder writes only plan-45 tags             |
| L2 | `DocumentLine` allows **negative** lines (deposit application, discounts to a contra account)                                                          | Today it requires positive magnitudes                  |
| L3 | `Loaded::write_document` takes an expected generation (it passes `expected: None` today); add `find_document(number)`, which finds by `invoice:` and returns `(Tindex, Fingerprint, Transaction)` | Safe re-book; no stable ids                     |
| L4 | `receivables` module: open documents, N:1 partial settlement, aging buckets, DSO; void reversal recognized as settlement                                | The dashboard, the `paid:` mirror, Ledgeline aging     |
| L5 | Move UI prefs (SPECS, merge, save routing, theme pin), `AppDirs`, recents, the security token/Host guard, SPA embed and GUI boot out of `ledgeline-server` into `ledgeline-core` (data) and `ledgeline-shell` (runtime) | Shared prefs and theme; one shell           |
| L6 | Extract `packages/ui` from `web/` (themes, scale, breakpoints, components, API client, native bridge); a root bun workspace                             | The invoicer SPA reuses the look                       |
| L7 | Generalize `scripts/release.sh`, the flake, `ci.yml`, `release.yml` and `package-crates` for a second app                                              | Lock-step release                                      |
| L8 | Relicense to AGPL-3.0-or-later (`LICENSE`, `[workspace.package] license`, `web/package.json`, README badge and section, docs)                                  | MBR is GPL; decision 2026-11-19                        |
| L9 | Ledgeline journal view: a `document:` tag renders as an "Open invoice" link (it opens the invoicer, or MBR's page)                                     | Book → invoice navigation                              |
| L10 | Incremental journal reload: re-parse only the changed include file, or the changed transactions, instead of the whole journal                       | A reload is 73–92 ms on the §5.6 corpus, most of the 100 ms journal-change budget |

## 9. Risks

- **MBR API churn.** MBR's crates say "no API stability guarantees".
  Mitigations:
  - pin an exact crates.io version (a git rev can't build: mbr-core's built
    JS assets aren't in git);
  - keep all MBR calls behind one `invoicer_core::notes` module;
  - get the façade (MBR-CHANGES M2) agreed early.
- **Two vault abstractions.** Ledgeline has its `Vault` and MBR has its own.
  On desktop both are plain filesystem access. On iOS, writes must go through
  one coordinator. We decide that at the iOS phase; until then, every
  invoicer write goes through Ledgeline's `Vault`.
- **Template trust.** A template is code in the user's repo. MBR's template
  override already confines paths, caps sizes and never falls back silently.
  Tera has no filesystem or network access beyond MBR's helpers.
- **Hand-edited tables that don't parse.** The editor opens the invoice
  read-only, showing the parse error and the row. The CLI `check` names the
  line. We never "fix" a table without the user saving.
- **Concurrent writers** (Ledgeline + invoicer + git pull). Journal writes
  are guarded by `ExternalChange`. Markdown writes use a content-hash check,
  and a refused write reloads and retries once, then asks the user.
- **Death by re-parsing.** Fast enough at 50 invoices, slow at 5,000. The
  budgets in §5.6 are gated by benches on a 5,000-invoice corpus from
  Phase 1 onward, before any UI exists to hide the cost.
- **Positional percentages surprise people.** The editor shows each
  adjustment's base ("10% of Senior architect hours") inline, and the books
  panel shows the resulting postings before send.

[Settings files are TOML]: ../docs/settings/settings-files.md
