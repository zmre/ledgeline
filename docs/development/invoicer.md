---
title: Invoicer code layout
description: "Where Ledgeline Invoicer's code lives: the invoicer-core and ledgeline-invoicer crates, the invoicer/web SPA, the shared packages/ui, the fixtures, and the just recipes that build and test them."
order: 15
tags: [development, invoicer]
---

# Invoicer code layout

Ledgeline Invoicer lives under `invoicer/` in the same Cargo and bun workspaces as Ledgeline, so it can move to its own repository later. It shares Ledgeline's version and release. The user manual for it is the [Invoicer](../invoicer/index.md) section.

The design documents are in the repository:

- [`invoicer/ARCHITECTURE.md`](https://github.com/zmre/ledgeline/blob/main/invoicer/ARCHITECTURE.md): what the app is, the file formats and the accounting;
- [`invoicer/PLAN.md`](https://github.com/zmre/ledgeline/blob/main/invoicer/PLAN.md): the phases, and every decision taken while building them;
- [`invoicer/MBR-CHANGES.md`](https://github.com/zmre/ledgeline/blob/main/invoicer/MBR-CHANGES.md): what the invoicer needed from mbr.

```text
invoicer/
  crates/
    invoicer-core/        lib: documents, the bill table, booking, reconciliation, views
    ledgeline-invoicer/   bin: the CLI, the HTTP API, the desktop window, PDF export
  web/                    the SvelteKit SPA, served under /app
  fixtures/               accrual, cash and empty books folders for tests
packages/
  ui/                     themes, scale, components, keymap and API client shared with web/
crates/
  ledgeline-core/         the journal engine both apps book through
  ledgeline-shell/        the desktop and server shell both apps boot through
```

## `invoicer/crates/invoicer-core`

Everything except UI and platform code. It follows the same rules as `ledgeline-core`: no globals, `today` passed in rather than read from the clock, all file access through a Ledgeline `Vault`, no platform `cfg`. All journal writes go through `ledgeline-core` (plan 45's `accrual::ops` and its `ExternalChange` guard); the invoicer writes no journal text of its own.

| Module                                                           | Holds                                                                                                                                          |
|------------------------------------------------------------------|------------------------------------------------------------------------------------------------------------------------------------------------|
| `table`                                                          | The bill-table engine: parse with spans, recognize columns and rows, compute, check, write back aligned. `table::words` is `[invoicer.words]`. |
| `booking`                                                        | The pure split of a computed table into one booking line per account and deferral.                                                             |
| `frontmatter`                                                    | Lossless frontmatter reading and the by-key editor behind `set`.                                                                               |
| `notes`                                                          | Discovery of notes by type and tag, behind an API shaped like mbr's `Site`.                                                                    |
| `doc`, `customers`, `catalog`, `snippets`, `templates`           | The document types and their validation.                                                                                                       |
| `tera_scan`                                                      | A light scan of Tera tags: blocks, `extends`, the name line.                                                                                   |
| `customer_edit`, `catalog_edit`, `snippet_edit`, `template_edit` | The library editors' writes, shared by the API and the CLI.                                                                                    |
| `numbering`                                                      | The next number over the files and the books.                                                                                                  |
| `index`                                                          | The in-memory document index, refreshed incrementally.                                                                                         |
| `draft`, `edit`, `editor`                                        | Creating drafts, the CLI's writes (`new`, `set`, `recalc`), and the app editor's form and save.                                                |
| `session`                                                        | An open books folder: vault, settings, journal styles, index.                                                                                  |
| `settings`                                                       | `ledgeline.toml [invoicer]`.                                                                                                                   |
| `books`                                                          | The books bridge: send, re-book, void, deposits, cash-books Apply payment and its undo, recognition.                                           |
| `reconcile`                                                      | The books' payment state, the `paid:` mirror and the books-versus-files checks.                                                                |
| `receivables`                                                    | Open items, aging, DSO and days to pay.                                                                                                        |
| `views`                                                          | The serializable shapes the read commands and the API return, the dashboard among them.                                                        |
| `problem`                                                        | What `check` reports.                                                                                                                          |

Tests are in `tests/` (accrual and cash books flows, goldens, proptests, the dashboard). The benches in `benches/` run on a generated 5,000-invoice corpus; `just invoicer-bench-gate` fails when one is slower than `benches/baseline.json` allows.

## `invoicer/crates/ledgeline-invoicer`

The binary. `main.rs` parses the command line with clap, opens the books and prints; each command is one `invoicer-core` call. `human.rs` formats the text output. The [CLI reference](../invoicer/cli.md) documents every command.

Cargo features decide what is built:

| Build                   | Has                                                     |
|-------------------------|---------------------------------------------------------|
| default (`gui`)         | the CLI, the server, and the desktop window around it   |
| `--features server`     | the CLI and the headless server (`--server`), no window |
| `--no-default-features` | the CLI alone: every command but `pdf`                  |

The app, `src/app/`, is one axum server, headless or in-process behind the window, booted through `ledgeline-shell` as Ledgeline is:

| Path                     | Served by                                                                  |
|--------------------------|----------------------------------------------------------------------------|
| `/api/*`                 | the invoicer API (`app::api` and its route modules), bearer token required |
| `/app/*`                 | the embedded SPA                                                           |
| `/.ledgeline-invoicer/*` | export links and the preview page                                          |
| everything else          | mbr's router, unmodified, behind a session cookie                          |

mbr sits at the root so it needs no base path; the SPA is under `/app`. `app::watch` follows file changes and runs the reconciler on every journal or markdown change. `app::pdf` and `pdf.rs` drive mbr's PDF export through the shell's event loop.

`starters.rs` embeds the starter templates and the invoice base from `templates/` (`classic.html`, `modern.html`, `classic-logo.html`, `modern-logo.html`, `_invoice_base.v1.html`); the base's comment header is its own documentation, mirrored for users on [Templates](../invoicer/templates.md). The fixtures carry a byte-identical copy of the base, which a test checks. `app/preview.rs` renders every preview with `invoicer_preview: true` in the note's frontmatter (never saved), so a template can show something in the app's preview alone, as the "with logo" starters' "Your logo" box.

`tests/cli.rs` runs the CLI on copies of the fixtures under `target/` and compares its JSON with `tests/snapshots/`. Those snapshots are worked examples of every `--json` shape.

## `invoicer/web`

A SvelteKit SPA with `paths.base = "/app"`, built to static files and embedded in the binary. `just build-invoicer` builds it; `build.rs` mirrors the build into the crate, or writes a placeholder page when there is none.

- `src/routes/`: the dashboard (`+page.svelte`), invoices, customers, catalog, snippets, templates and the new-invoice page.
- `src/lib/components/`: the pages' parts, each with its component test beside it.
- `src/lib/keys/`: the invoicer's key bindings, registered with the shared keymap. `groups.ts` sets the order of the `?` sheet.
- `e2e/`: Playwright specs, run by `just invoicer-e2e` against five servers (ports 5195–5199), each over a scratch copy of a fixture under `target/`. `playwright.config.ts` lists them.

## `packages/ui`

The parts both SPAs import as `#ui/…`: themes, UI scale and container-query breakpoints, shared components such as `DateInput`, the API client and token, the native bridge, and the keymap engine in `keys/` (chords, layers, the typing guard, the `?` sheet). Each app keeps its own bindings and passes its section order to the keymap.

## Fixtures

`invoicer/fixtures/` holds three synthetic books folders, described in its [README](https://github.com/zmre/ledgeline/blob/main/invoicer/fixtures/README.md):

| Folder     | Books                                                                                                             |
|------------|-------------------------------------------------------------------------------------------------------------------|
| `accrual/` | Invoices 101–108 covering paid, partly paid, void, deposit, final, overdue and draft, on plan 45's accrual books. |
| `cash/`    | Invoices 201–207 on cash books, with payments applied and a deposit income on receipt.                            |
| `empty/`   | A first day: `[invoicer]` set up, nothing billed yet.                                                             |

Tests copy them before writing. Never point a write at the committed folders.

## Recipes

| Recipe                         | Does                                                                                   |
|--------------------------------|----------------------------------------------------------------------------------------|
| `just build-invoicer`          | Build the SPA into `invoicer/web/build`.                                               |
| `just invoicer-server JOURNAL` | Run the headless server on port 5199, for `vite dev` on 4273. Use a copy of a fixture. |
| `just invoicer-e2e`            | Build the SPA and the binary, then run the Playwright suite.                           |
| `just invoicer-bench-gate`     | Run the benches and compare them with the baseline.                                    |
| `just test`                    | The vitest suites of `web/`, `packages/ui` and `invoicer/web`.                         |

`cargo test -p invoicer-core -p ledgeline-invoicer` runs the Rust tests. CI lints `invoicer/web` with prettier and eslint like the other web folders.
