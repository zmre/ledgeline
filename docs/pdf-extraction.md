# Extracting tables from PDF statements (deferred — research notes)

PDF import was deliberately cut from WP-11 (`plans/11-enhanced-import.md`). This document
exists so whoever picks it up does not repeat the survey. Everything here was measured in
August 2026 — versions, closure sizes and the capability claims were compiled and run, not
read off a README. Re-check the version numbers before trusting them; the conclusions should
age better than the releases.

**The headline: it is feasible in pure Rust, and the extractor is not the hard part.**
Getting positioned glyphs out of a PDF took about 60 lines. Turning positioned glyphs into
correct rows — across wrapped descriptions, page breaks and right-aligned amount columns —
is where the real work is.

## Why it was deferred

Not because it is impossible, but because bank PDF layouts vary far more than OFX or CSV do,
and a wrong extraction in a bookkeeping app is worse than no extraction. The other formats in
WP-11 have a *correct* answer we can validate; a PDF gives a plausible answer that needs a
human to check every row. That is a different UX and a different amount of work, and mixing
it into the same milestone risked swallowing it.

## Recommendation

Use **`pdf-extract`** (0.12.0, MIT, active, ~595★). Contrary to the common claim that no pure
Rust crate exposes glyph positions, it does — via the public `OutputDev` trait:

```rust
fn output_character(&mut self, trm: &Transform, width: f64, spacing: f64,
                    font_size: f64, char: &str);
fn begin_word(&mut self);  fn end_word(&mut self);
fn begin_page(&mut self, page_num: u32, media_box: &MediaBox, art_box: Option<...>);
fn stroke(&mut self, ctm: &Transform, colorspace: &ColorSpace, style: &..., path: &Path);
fn fill(&mut self, ...);
```

`trm.m31` / `trm.m32` are the glyph's x and y. `begin_word`/`end_word` give you word
segmentation for free. `stroke`/`fill` with a `Path` give you the **ruling lines**, which
means both stream-mode (whitespace-based) and lattice-mode (line-based) table detection are
available without leaving Rust.

A working probe against a text-layer bank statement produced:

```
positioned WORDS: 31   ruling-line subpaths: 0
["Date", "Description", "Debit", "Credit", "Balance"]
["01/02/2026", "COFFEE ROASTERS #114",  "4.50",         "1,995.50"]
["01/03/2026", "ACME HARDWARE & SUPPLY","128.99",       "1,866.51"]
```

### Two traps that cost real time

1. **Partition by page in `begin_page`.** Otherwise every page collapses into one coordinate
   space and rows from page 2 interleave with page 1 at the same y.
2. **Naive left-edge clustering fails on right-aligned amount columns.** The first pass put
   `3,200.00` in Debit instead of Credit, because both columns' *left* edges vary with the
   number's width. The fix: seed column boundaries from the **header row**, then locate
   numeric cells by their **right** edge and non-numeric cells by their left. That took the
   probe from 6/7 to 7/7 rows minus one text-overrun case.

### Known risks in `pdf-extract`

- There is an open text-extraction regression in the 0.12 line (a font cache persisting
  across pages). **Consider pinning 0.10.0** and re-testing before adopting 0.12.
- **It panics on malformed input** rather than returning an error. Wrap it in
  `catch_unwind`, or better, run it out-of-process — a corrupt PDF must not take down the
  desktop app.

## The alternative, if you want to defer again

**`pdftotext -tsv`** from poppler-utils — note `-tsv`, **not** `-layout`. It emits
Tesseract-style TSV with per-word `left/top/width/height`, which the `csv` crate reads
directly with `.delimiter(b'\t')`. `-layout` looks excellent on clean files and then puts
amounts at inconsistent character offsets on real ones; do not build on it.

Cost: the poppler-utils closure measured **154.9 MiB across 55 store paths**, reducible to
roughly 100 MiB with `poppler_min.override { utils = true; }`. It is GPL-2+, which is
defensible as a **separate process** and not as a linked library. That is a big dependency
for a project whose selling point is a single binary, which is the other reason `pdf-extract`
wins.

## Ruled out, with reasons

| Option | Why not |
| --- | --- |
| `pdfium-render` (0.9.3, MIT/Apache) | Works, but nixpkgs `pdfium-binaries` ships **only** `lib/libpdfium.dylib` — no static `.a`. You would ship and codesign a dylib, which breaks the single-binary property. Closure is otherwise excellent (6.8 MiB, 1 path, no deps). Keep as the fallback if pure Rust stalls. |
| `mupdf-rs` (0.8.0) | **AGPL-3.0.** Artifex actively litigates. Disqualified on licence alone. |
| `poppler-rs` (0.26.0) | Doesn't bind `poppler_page_get_text_layout()`, so it cannot give you per-character rectangles — the one thing you need. |
| `pdf` / pdf-rs (0.10.0) | Exposes raw content-stream operators only; you would reimplement text-state tracking yourself. Sporadic maintenance (a two-year gap between 0.9.0 and 0.9.1). |
| `lopdf` (0.44.0) | Very active, but it is a document-structure library with no text positioning. It is already a transitive dependency of `pdf-extract`. |
| `tabula-java` | Needs a JVM — ~887 MiB closure. |
| Python `camelot` / `pdfplumber` | Needs Python + Ghostscript (AGPL); ~1.14 GiB closure. |

**No mature Rust table-detection crate exists.** Of ~50 reverse-dependencies of
`pdf-extract`, zero add table extraction. This part you write yourself either way — which is
the real argument for staying in Rust rather than paying a 155 MiB dependency to still have
to write the column logic.

## If you build it

- Architect behind a `PositionedWord { x, y, w, h, text }` interface now, so `pdf-extract`
  and `pdftotext -tsv` are swappable without touching the table logic. WP-11's `convert`
  module already has the right shape for this: a PDF backend just needs to produce
  `Tabular`.
- **Scanned PDFs have no text layer at all.** Detect zero positioned words and say so
  plainly ("this looks like a scan — OCR is not supported") rather than returning an empty
  table. OCR is a separate project.
- Reuse WP-11's arithmetic validation: assert `previous_balance + amount == balance` per row
  and `opening + Σ == closing` per statement. For PDF this matters more than anywhere else,
  because it is the only cheap way to catch a column that silently shifted. Treat it as the
  primary correctness gate, not a nicety.
- Budget realistically: ~2–3 weeks to something trustworthy, with most of it in wrapped
  descriptions and page continuations, not in extraction.

## Related: QuickBooks (also deferred, also different)

Worth recording here because it is the other format on the roadmap and it does **not** belong
in this pipeline. A QuickBooks export is not a bank statement — it already carries both sides
of every transaction, in QuickBooks' own chart of accounts. So:

- There is **no intermediate CSV and no `hledger import`**. Going through a rules file would
  mean flattening two-sided data into one-sided rows and then guessing the other side back —
  losing information to recover it badly.
- What it needs instead is a persistent **account mapping** (QuickBooks account → hledger
  account), stored per-source and reused across imports, plus a UI for resolving unmapped
  accounts on first sight.
- That mapping store, not the parser, is the actual design work. `.qbo` files are just OFX
  and are already handled by WP-11's OFX path; it is IIF/CSV/XML *exports* that carry the
  chart of accounts and need this treatment.

It should get its own WP, sharing WP-11's drop target, git safety net and journal-target
selection, but none of its rules-file machinery.
