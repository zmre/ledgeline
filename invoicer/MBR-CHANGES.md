# MBR changes needed by Ledgeline Invoicer

This is a hand-off spec for implementing changes in
[zmre/mbr-markdown-browser](https://github.com/zmre/mbr-markdown-browser).
Context is in [ARCHITECTURE.md](ARCHITECTURE.md). In short, the invoicer
links MBR as a library to:

- render invoice notes with per-file Tera templates;
- export them to PDF;
- read the note index: customers are `type: person|organization` notes,
  invoices are `type: invoice`;
- show backlinks.

The invoicer's axum server merges `mbr_server::server::Server.router` at its
root, and serves its own SPA under `/app` and its API under `/api`.

The state these requests start from was surveyed on 2026-11-19:

- main is at 56d3316;
- PDF export (#386) and per-note templates (#387) are both merged;
- neither is released yet: crates.io has 0.7.3.

Paths below are relative to the MBR repo.

Each item has a priority:

- **P0** blocks the invoicer's first usable release;
- **P1** is needed for the full GUI;
- **P2** is nice to have.

## M0 (P0). One release, after the other P0 and P1 changes

The invoicer depends on what #386 and #387 added:

- `template:` as a `/`-rooted `.html` path;
- the lossless `frontmatter` template variable;
- `url()`;
- `Templates::render_page_template`;
- `standalone::render_note`;
- `pdf_export`.

**Release order.** Land the other P0 and P1 items (M1–M6, M8) first, then
publish a single crates.io release containing all of them. Until then the
invoicer pins a git rev of main. It needs the release only before its own
first release, because `cargo publish` rejects git dependencies. After the
release, the invoicer pins that exact version.

The invoicer's template convention is `template: /invoices/tpl/<name>.html`.
`invoices/tpl/` is not in `ignore_dirs`, so the watcher reloads templates
there without further changes. Please confirm that, and add a test that
editing a page template triggers a reload of the notes that use it.

## M1 (P0). Resolve wikilinks in any frontmatter value

**Today.** `customer: "[[Customer X]]"` reaches templates and `site.json` as
a literal string. It isn't linked when rendered, and it isn't a backlink.
Only `relationships` and a person's `company` are resolved.

**Want.**

- **Find the links.** A frontmatter string value whose entire content is
  `[[Target]]` or `[[Target|Label]]`, either at the top level or as an
  element of a list of strings, is a frontmatter link.
- **Paths to files are links too.** A string value that is a `/`-rooted or
  `./`/`../`-relative path to an existing file in the repo is also a
  frontmatter link, whatever the file type. The common case is an executed
  contract:
  `contracts: ["[[Customer X MSA 2026]]", "/customers/contracts/Customer X SOW 3 renewal (signed).pdf"]`.
  It renders as a link to MBR's URL for that file. A path that doesn't exist
  is reported in `errors.json` like a broken body link, rather than silently
  staying text. Only values that start with `/`, `./` or `../` count, so
  ordinary strings are never mistaken for paths.
- **Wikilinks to non-notes.** If the wikilink index learns non-markdown files
  (`[[Customer X SOW 3 renewal (signed).pdf]]`, as Obsidian allows), the
  same rule covers them. That is optional: paths are enough for the
  invoicer.
- **Make them backlinks.** Frontmatter links count as outbound links, with
  `kind: "frontmatter"` and `key: "<field>"`. That makes them inbound links
  on the target, so the target's "Linked from N notes" chip lists every
  invoice for a customer.
- **Show them in `links.json` and `site.json`.** `links.json` gets
  `inbound[].kind` and `inbound[].key`. `site.json` gets a resolved form, for
  example `frontmatter_links: {customer: {url, title}}`, next to the raw
  value.
- **Template context.** Templates get
  `frontmatter_links.<key> = {url, title, raw}`, so a template can render
  `<a href="{{ frontmatter_links.customer.url }}">`.
- **Grep fallback.** The fallback in `crates/mbr-server/src/link_grep.rs`
  must agree with the index, whichever way it is decided: either it includes
  frontmatter links too, or both exclude them.
- **API.** Something like
  `mbr_core::markdown::frontmatter_wikilinks(&Yaml, &WikilinkIndex) -> Vec<FrontmatterLink { key, raw, resolved: Option<ResolvedLink> }>`.

## M2 (P0). A library façade in `mbr-core`

**Today.** Assembling `site.json` (`SiteJson`, `render_site_json`) and
building the inbound-link index are private to `mbr-server`. The lossless
YAML→JSON converter is private. `SimpleMetadata` is lossy: it flattens maps,
drops lists of maps, and turns floats into strings.

**Want.** A small, documented, semver-conscious façade:

```rust
pub struct Site { /* Arc<Repo>, indexes */ }
impl Site {
    pub fn open(root: &Path, config: Config) -> Result<Site, MbrError>;   // scan + indexes
    pub fn refresh(&self, changed: &[PathBuf]) -> Result<(), MbrError>;   // incremental
    pub fn notes(&self) -> impl Iterator<Item = NoteSummary>;              // path, url, title, type, tags
    pub fn notes_of_type(&self, ty: &str) -> Vec<NoteSummary>;             // case-insensitive
    pub fn note(&self, rel: &Path) -> Result<NoteData, MbrError>;          // lossless frontmatter (serde_json::Value), Contact, body source
    pub fn field_values(&self, key: &str) -> Vec<(serde_json::Value, usize)>; // distinct values + counts, for autocomplete
    pub fn backlinks(&self, rel: &Path) -> Vec<InboundLink>;               // including M1 frontmatter links
    pub fn resolve_wikilink(&self, from: &Path, target: &str) -> Option<ResolvedLink>;
    pub fn search(&self, q: &SearchQuery) -> SearchResults;
}
```

Requirements:

- **No contact stripping in-process.** `NoteData` includes the contact fields
  (`emails`, `addresses`, …), which `serialize_public_frontmatter` strips from
  `site.json`. That stripping is right for published sites; an in-process
  library caller needs the full data.
- **Lossless frontmatter.** The YAML→JSON converter that per-note templates
  (#387) use for its `frontmatter` variable is made public.
- **HTTP equivalents.** `mbr-server` gains `GET /.mbr/fields/{key}/values`
  and `GET /.mbr/notes?type=person&fields=a,b`. In GUI or loopback mode with
  the edit token, `fields` may name private contact fields. The invoicer can
  do without these by using the Rust façade, but the SPA benefits.

## M3 (P0). Render a page from an in-memory string

**Today.** `standalone::render_note(config, abs, rel)` reads the note from
disk.

**Want.** `standalone::render_note_source(config, rel, source: &str) -> Result<String>`.
It renders exactly as the on-disk path would, applying the per-file template
and resolving links relative to `rel`. The invoicer uses it for live preview
of unsaved edits, at roughly 10 renders per second while typing, so it must
not rescan the repo.

## M4 (P0, small). PDF export from a host app: keep it working

**Today this already works.** `mbr_markdown_browser::pdf_export::ExportSession::start(&EventLoopWindowTarget<T>, url, PdfOptions, PendingOutput, ExportSink)`
is public. The invoicer passes its own tao event loop and the invoice's URL
on its own server (which has MBR's router merged in), and routes
`ExportEvent`s back through `ExportSession::handle`. No new crate is needed.

**Want: only guarantees, no new API.**

- **A lean build.** `mbr-markdown-browser = { default-features = false, features = ["gui"] }`
  must keep building as a library with no ffmpeg or pdfium. Today `gui`
  implies `server`, which is fine because the invoicer mounts the server
  anyway. A CI check for that feature set would stop it regressing.
- **Matching wry and tao.** The host's `EventLoopWindowTarget` must be the
  same tao type, so the two repos move wry and tao together. Both are on wry
  0.57 and tao 0.37 today. A note in MBR's `Cargo.toml` next to those pins
  is enough.
- **`@page` from the template.** For a template that sets its own `@page`
  size and margin, those should win over `PdfOptions`' defaults. Today the
  export injects `#mbr-export-page` with the configured margin, and the
  native print settings take paper from `PdfOptions`. Either a
  `PdfOptions { paper: None, margin: None }` meaning "use the page's
  `@page`", or documenting that a template's own `@page` rule wins, is
  fine.
- **Platform matrix.** macOS and Linux are needed for the invoicer. iOS goes
  through `mbr-ffi` later, using WKWebView `createPDF`.

## M5 (P0). Readiness contract for pages that use custom templates

**Today.** PDF export waits for `mbr:export-ready`, which is posted by
`export-ready.ts` in MBR's main bundle. A chrome-free per-file template that
doesn't load the main bundle never signals ready, so the export waits out the
30 s timeout.

**Want.**

- **A tiny partial.** Ship `/.mbr/_export_ready.html` (or a small
  `export-ready.js`) that a template includes. It waits for images, fonts,
  mermaid, KaTeX, and anything else present, then signals ready.
- **Document it** in `docs/features/custom-page-templates.md` and
  `docs/features/pdf-export.md`.
- **Fallback.** As a fallback, an export option `ready = "load"` signals ready
  on `window.load` plus `document.fonts.ready`.

## M6 (P1). Embeddable markdown editor component

**Today.** `<mbr-editor>` (Crepe 7.23 with a frontmatter form, in
`components/src/mbr-editor.ts`) depends on page globals: `window.frontmatter`,
`shared.js`, `site.json`, the edit token and live reload. It edits whole
files.

**Want.** A standalone web component published from `components/`, usable in
another SvelteKit app:

```html
<mbr-markdown-field value="…markdown…" wikilinks-url="/.mbr/site.json" placeholder="Notes above the table"></mbr-markdown-field>
```

- **Properties:** `value`, `readonly`, and an optional `catalogUrl` for
  wikilink autocomplete.
- **Events:** `change` with `{value}`, and `blur`.
- **No file I/O.** The host owns saving. MBR's remark plugins (wikilinks,
  alerts, math, definition lists) stay enabled. HTML output stays disabled,
  as in #383.
- **Packaging.** It must be loadable as an ES module from MBR's assets
  (`/.mbr/…`), or published as an npm package. Either works.

## M7 (P2). Column identity on rendered table cells

**Want.** When rendering GFM tables, add `data-col="<slug of header text>"`
to every `th` and `td`. For example, `Account` gives `data-col="account"`,
and an italic `*Account*` header gives the same slug. Optionally also add
`data-row-kind="total"` when any cell's sole content is bold.

The invoicer hides its books column with a portable `:has()` rule that doesn't
need this. With it, templates can style columns without positional
selectors. There is no change to the markdown syntax.

## M8 (P1). Theme injection by the host

**Want.** `ServerConfig.extra_head_html: Option<String>`, or a
`theme_vars: BTreeMap<String, String>` that is emitted as `:root { --pico-…: …; --mbr-…: … }`
into every page's head. The invoicer can then push the user's Ledgeline
theme (light/dark variant colors) into MBR's chrome in browse mode, without
writing files into the user's repo.

## M9 (P2). The frontmatter scan cap

**Today.** `FRONTMATTER_MAX_BYTES` (8 KB, in `markdown.rs`) silently
truncates large frontmatter in the index.

**Want.** When the closing fence isn't found within the cap, fall back to a
full read, or at least report a frontmatter error instead of truncating.
Invoices with many custom fields, or a customer with many contacts, can pass
8 KB.

## Not needed (decided against)

- **A base-path mount for `Server.router`.** The invoicer mounts MBR at the
  root and moves its own SPA to `/app`.
- **Bare-name template lookup** (`template: myco-default`). The invoicer
  writes full `/`-rooted paths.
- **A separate `?embed=1` mode.** Invoice templates are chrome-free already,
  and browse mode wants the chrome.
