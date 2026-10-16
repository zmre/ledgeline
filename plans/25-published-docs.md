# 25 — Published docs: structure, links, frontmatter, screenshots, mbr site

Turn `docs/` from a folder of maintainer notes into a browsable user manual,
published as a static site with [mbr](https://github.com/zmre/mbr-markdown-browser)
on every release and on demand.

## The ask (Patrick, 2026-10-16)

- References between docs are filenames in code spans. Make them real
  markdown links.
- Make sure the docs are current and complete. The import docs get a callout
  explaining how to export the Journal from QuickBooks (Reports → Journal →
  pick a period, where "All dates" is fine because duplicates won't
  re-import → Export to XLSX), then how to import it. Document creating
  equivalent accounts and the mappings.
- Publish with mbr the way mbr publishes its own docs. Add YAML frontmatter to
  every page, and tags where they help. Publish on every release, or manually.
- Add screenshots and screenshot excerpts with circles and arrows wherever
  they help, without overdoing it.
- Split very long pages into subfolders.

## Constraints from mbr (researched 2026-10-16 against mbr v0.6.2)

- **Site root:** mbr looks upward for `.mbr/`, so `docs/.mbr/config.toml` is
  required. Without it, `mbr -b docs` publishes the whole repo.
- **Folder index:** the folder index is `index.md`. A `README.md` is just an
  ordinary page.
- **Ordering:** pages sort by `[[sort]]`. We sort on the `order` field
  (numeric) first, then `title`. A folder's position comes from its
  `index.md` frontmatter.
- **Frontmatter keys:** `title`, `description`, `tags`, `order`. Invalid YAML
  silently drops the whole block.
- **Callouts:** only `[!NOTE]`, `[!TIP]`, `[!IMPORTANT]`, `[!WARNING]` and
  `[!CAUTION]`. **`[!INFO]` is not supported** (GitHub doesn't support it
  either), so the QuickBooks callout uses `> [!NOTE]`.
- **Links:** `[x](other.md)` is rewritten to the page URL and anchors are
  kept. A link that leaves `docs/` (`../plans/…`, `../README.md`, `../src/…`)
  is a broken "escapes the site root" link. Those become absolute
  `https://github.com/zmre/ledgeline/blob/main/…` URLs.
- **Ignore lists:** the default ignores drop `*.sh`, `*.ts`, `*.css`, and
  directories named `build`, `templates` and so on. Don't keep pages or
  images under those names.
- **Images:** images go next to the pages that use them, in `docs/**/images/`.
  Relative paths only.
- **Building:** `mbr -b --fail-on-broken-links --output <dir> docs` is the
  gate. CI builds mbr with
  `nix build --accept-flake-config github:zmre/mbr-markdown-browser/v0.6.2#mbr`
  because the release binary links into `/nix/store` and won't run on a
  stock runner.
- **Deploying:** the output must be tarred by hand with `--dereference`,
  because mbr symlinks assets and `upload-pages-artifact` drops the `.mbr/`
  dot-dir that search needs. Copy mbr's own `docs.yml`.

## Target structure

```
docs/
  .mbr/config.toml          title_prefix "Ledgeline | ", sidebar single, sort order→title
  index.md                  home: what Ledgeline is, install, open a journal, map of the manual   (order 0)
  journal/                  NEW content (order 10)
    index.md                journal view, columns, totals footer, editing transactions
    filtering.md            filter bar: accounts, dates, search, URL sharing
    problems.md             background checks: each rule, what fixes it
    keyboard.md             shortcut table, `?` help
    export.md               journal → QuickBooks-style XLSX
  imports/                  split from imports.md (order 20)
    index.md                overview, supported formats
    new-transactions.md     drop, match, preview, commit, re-sort, re-downloads
    create-rules.md         Create Rules wizard
    rules-editor.md         Edit Rules: editable vs opaque, grammar facts
    quickbooks.md           export from QuickBooks (NOTE callout), import, accounts and mapping
  reports/                  (order 30)
    index.md                tabs overview, shared controls (dates, interval, depth), export
    insights.md             NEW
    balance-sheet.md        moved
    income-statement.md     moved (updated for plan 24 afterwards)
    cash-flow-and-net-worth.md  moved (updated for plan 24 afterwards)
    subscriptions.md        NEW
  holdings/                 split from holdings.md (order 40)
  budget.md                 (order 50)
  projections/              split from projections.md (order 60)
  settings/                 (order 70)
    index.md
    accounts.md             NEW: account list editor, declaring accounts, account types
    account-aliases.md      moved out of imports.md (the UI lives in Settings)
  development/              maintainer docs, last (order 90)
    index.md, building.md (was development.md), releasing.md, imports-internals.md,
    perf-baseline.md, pdf-extraction.md, nix-binary-cache-playbook.md, docs-site.md
```

The writer can adjust the split where a page's content suggests a better
cut. The fixed parts are subfolders for imports, holdings, projections,
reports and development.

## Frontmatter and tags

Every page gets:

```yaml
---
title: Importing from QuickBooks
description: Export the Journal report from QuickBooks Online and bring it into your hledger journal.
order: 40
tags: [imports, quickbooks]
---
```

Tags earn their place where a topic cuts across folders. A small fixed
vocabulary:

- `imports`, `reports`, `holdings`, `budget`, `projections`
- `account-tags`: every page that explains an hledger account tag Ledgeline
  reads, such as `bsgroup:`, `isgroup:`, `holdings:` and `valuation:`. This
  is the most useful tag, because those tags are documented in four places.
- `export`, `quickbooks`, `prices`, `development`

## QuickBooks section requirements

1. A `> [!NOTE]` callout with the export steps: in QuickBooks Online, go to
   **Reports**, search for **Journal**, set the report period (**All dates**
   is a good choice, because re-importing skips transactions already imported,
   matched by `Trans #`/`id:`), run it, then **Export → Export to Excel
   (XLSX)**.
2. Importing: Imports → New Transactions, drop the file, and the QuickBooks
   panel takes over.
3. **Creating equivalent accounts:** declare the accounts you want in
   Settings → Accounts, using hledger account types and names, before mapping.
   Mapping to an undeclared name works, but the account is then only implicit.
4. **Mappings:** the "Map QuickBooks accounts" table. Each mapping is saved as
   a plain hledger `alias QB name = your:account` in the root journal, visible
   and editable in Settings → Account Aliases. A mapping on a parent covers
   its QuickBooks sub-accounts. Import stays disabled until every account is
   mapped. Imported transactions carry `id:`, `class:`, `customer:` and
   `vendor:` tags.

## Screenshots

- **Harness:** `web/docs-screenshots/` with its own Playwright config. It
  reuses the e2e engine and preview servers and the pinned clock (2026-07-08),
  and runs via `just docs-screenshots`. Output goes to `docs/**/images/*.png`
  and is committed.
- **Annotations:** done in the page, not with ImageMagick. A helper
  (`annotate(page, [{target: locator, kind: 'circle'|'arrow'|'box', label?}])`)
  injects a fixed-position SVG overlay drawn from element bounding boxes,
  then the shot is taken with `clip` for excerpts. This is reproducible with
  no new dev-shell dependency, and the shots regenerate when the UI changes.
- **Size:** `deviceScaleFactor: 2`, a viewport of about 1280×800, and
  excerpts clipped to the region being discussed.
- **Fixtures:** `fixtures/sample.journal` by default. Budget uses
  `fixtures/budget/basic.journal`, Subscriptions uses
  `fixtures/subscriptions/basic.journal`, asset growth uses
  `fixtures/asset-growth-scenario.journal`, and QuickBooks uses
  `fixtures/import/qb-journal/report.xlsx`.
- **Budget:** about 15–25 images across the manual. One overview per major
  tab, plus annotated excerpts where the text explains a specific control:
  filter bar, problems drawer, QuickBooks mapping table, Interval/Total
  controls, tag effects on the balance sheet, holdings price fetch, and
  budget bars.

## Publishing

`.github/workflows/docs.yml`:

- **Triggers:** `push: tags: ["v*"]` (a release; tags pushed by a person do
  trigger, unlike releases created with `GITHUB_TOKEN`), `workflow_dispatch`,
  and `pull_request` on `docs/**` paths. PRs build and link-check only.
- **Steps:** build mbr via nix, then
  `mbr -b --fail-on-broken-links --output docs-build docs`, then tar with
  `--dereference`, `upload-artifact` named `github-pages`, and
  `deploy-pages`. Deploy only for tags and dispatch.
- **Actions:** pinned by SHA, matching the pins already used in `ci.yml`
  and `release.yml`.
- **One-time manual step:** enable GitHub Pages with source "GitHub Actions"
  in repo settings.

## Work split (parallel)

| Agent        | Work                                                                                       | Starts after   |
|--------------|--------------------------------------------------------------------------------------------|----------------|
| Writer       | restructure, links, frontmatter, tags, gap pages, QuickBooks section, `.mbr/config.toml`   | now            |
| Engineer B   | screenshot harness + annotation helper + all non-report-interval shots                     | now            |
| Iris         | `docs.yml` workflow, `just docs` / `just docs-check` recipes, README links                 | now            |
| Final pass   | embed screenshots in pages, update P&L / Cash Flow docs and shots for plan 24              | Writer, B, A   |
