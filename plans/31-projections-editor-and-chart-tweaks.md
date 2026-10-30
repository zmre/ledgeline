# 31 — Projections editor redesign, and two chart tweaks

Three changes, shipped in this order:

- **Part A (small).** The cash flow statement's section colours match the
  chart's.
- **Part B (small).** The P&L Sankey's labels stop overlapping, and hovering
  shows a real tooltip.
- **Part C (big).** The Projections what-if table becomes one list in **file
  order**. Each entry reads like the text it is, and editing happens in a
  dialog. Comment blocks become entries you can see, edit and move, and any
  entry can be turned off by commenting it out in the file.

Parts A and B touch nothing in Part C, and each ships as its own PR first.
Part C is an engine change (an ordered piece model of the scenario file, with
comment and disabled entries), then a wire change, then the UI.

## The ask (Patrick, 2026-10-29)

> **A.** The cash flow report's section colours in the statement don't match
> the chart. Pin one colour per activity, taken from the chart palette, and use
> it in both places.
>
> **B.** The P&L Sankey's labels overlap on a ledger with many accounts.
> Declutter them, and add hover.
>
> **C.** Redesign the projections editor. One list in the file's order, with
> each entry displayed like text (like the Journal / Rules editor) and an Edit
> button that opens a dialog. Comment blocks are first-class entries. Entries
> can be disabled and enabled. Add "Duplicate" and "Duplicate as alternative".
> This covers every kind: recurring flows, one-offs, assets and automatic
> postings. It should look really nice, with keyboard navigation and
> accessibility.

Decisions Patrick made before this plan was written are marked **(P)** below.

## What is true today

### Part A

- **The statement's accents are daisyUI theme colours.**
  `CashFlowStatementView.svelte:101-105` holds `ACCENT` (operating → `info`,
  investing → `secondary`, financing → `accent`). It is used only by the
  section `<h3>` at :210-212.
  - There are three section kinds (`CfSectionKind`, `reports/types.ts:714`).
  - "Exchange rates" is a ruled line, not a section (:236-237), and has no
    accent.
- **The chart assigns colours by size.**
  - `CashFlowChart.svelte:18` → `cashFlowChartReport` (`cashFlowRows.ts:214-224`)
    → `PeriodStackView.svelte:63` → `periodStack()` (`periodStack.ts:219-245`).
  - Colours are picked BIGGEST FIRST (`slotsBySize`, :203-211; rule 4 in the
    header, :32-40).
  - The rows carry the section *title* ("Operating"), not the kind. So a
    quarter where financing dwarfs operating paints financing indigo. The
    colours share no source with the statement and change with the data.
- **Net Worth uses the same path** (`NetWorthChart.svelte:31`). Its colours are
  pinned by existing tests (`periodStack.test.ts:254-263`, :285-385), and they
  must not move.
- **The palette.** `palette.ts:92-113` holds 14 CVD-validated slots as CSS vars
  `--chart-1..14` (`app.css:34-50`). `chartColors` resolves them per theme
  (`chartColors.svelte.ts:43-99`). Dark is the only theme.

### Part B

- **Rendering.** `SankeyPanel.svelte` draws with layerchart's `Sankey`
  (:143, `nodePadding={12}`).
  - Each node has a name line at `middle - 6` and an amount line at
    `middle + 6` (:173-190). Nothing stops two neighbours' labels from
    colliding.
  - Height is `min(900, max(200, n×30))` (:98-101).
  - Hover is SVG `<title>` only (:146, :163), so it is slow, unstyled and
    can't be reached by keyboard.
  - The legend lists every account with its amount (:201), which is the
    accessible fallback.
- **Tooltips elsewhere.** No shared tooltip component exists. Charts use
  layerchart's `Tooltip.Root` inline (`PeriodStackChart.svelte:303-329`).

### Part C

- **One wall of forms.**
  - `ProjectionsTable.svelte` (1330 lines) splits the scenario into five
    sections:
    - Income / Expenses (:910-978)
    - Assets (:984-1108)
    - One-off events (:1110-1256)
    - Automatic postings (:1265-1329)
  - Each section shows an input for every field. One `~` block with a revenue
    and an expense posting appears in **two** sections.
  - A section "hold" (`holdSection` :306) exists only to stop rows jumping
    between sections while you type.
- **Order is per kind, and comments are invisible.**
  - The writer fills "slots" separately for rules and events
    (`serialize.rs:1077` `assign_to`, :996-1014) and for `=` rules
    (`auto_splices` :669).
  - Comments, transactions and directives between blocks are copied through
    where they were (module docs :14-29). The table never shows them.
  - A chunk moved into another slot is **re-rendered** rather than carrying its
    own bytes (:592).
- **Identity.**
  - A `~` block is group `rule:K` (K = its index among the file's `~` blocks,
    :398).
  - A single-date block is a `ScenarioEvent` with id `rule:K` (:317-376).
  - A `=` rule is `ScenarioAutoRule { block }` (`projections.rs:465`; plan 30
    amendment 12).
  - A step change is two `~` blocks whose postings share a `; line:` tag, so
    the two lines share an `id` but not a `group` (`projections.rs:343-360`).
  - `PeriodicDoc` (`periodic.rs:1508` `scan`) locates `~` blocks. It skips
    `comment`/`end comment`, and it includes indented comment lines in a block.
  - `header_span` (`serialize.rs:1701`) covers only the recognised header lines
    (banner, `projection:`, `created:`, `updated:`, `cash:`).
- **Inline comments are lost on edit.**
  - An edit re-renders the block from `rule_header(period, description)` and
    `line_tags` (:1420, :1454). Those write only `line:`, `growth:`, `opening:`
    and `gains:`.
  - So a header comment (`PeriodicTransaction.comment`, `model.rs:589`) or a
    free-text posting comment in an *edited* block is dropped today. An
    unedited block keeps its bytes (`current` comparison, :586-596).
- **The save path.**
  - `PUT /api/projections/{*id}` → `DecodedScenario::signed`
    (`projections_api.rs:1103-1180`) → `upgrade_legacy_counters` → `update_scenario`
    (:2049).
  - That calls `write_scenario` (`serialize.rs:1760`) =
    `apply` + `confirm_round_trip` (:1822) + `confirm_auto_rules` (:1788).
  - The response is the re-read `WireScenarioFile` (:2127-2134). The page
    calls `markSaved()`, not `adopt()` (`+page.svelte:202-216`), so after a
    save the client's `rule:K`/`block` identities can be stale. The writer
    survives that because its claims are content-checked (amendment 12).
- **Engine consumers of the scenario.** `forecast::expand` (`forecast.rs:891`)
  and `expand_from` (used by `real_side.rs:493` `project_cached`) are the only
  readers of `lines`, `events` and `auto_rules` (`forecast.rs:298, 817, 1008,
  1012, 1030`).
- **The page.**
  - It registers no keys (`routes/projections/+page.svelte`).
  - Read-only state lives in `ScenarioFileBar.svelte:109-130`. You can still
    edit a non-writable file and Save As.
  - A seed has `name: ""` and no `fileStore.currentId`.
- **Pieces to reuse.**
  - The journal modal shell: `journal/edit/TransactionModal.svelte:169-201` and
    `keys/dismissible.ts`.
  - `AccountInput.svelte`, `components/DateInput.svelte`, `RowMenu.svelte`.
  - `listCursor` (`ui/listCursor.svelte.ts:45`) and `registerKeys`
    (`keys/keymap.svelte.ts:58`).
  - The RulesList keymap: j/k, J/K move, `g g`/`G`, Enter
    (`imports/ui/RulesList.svelte:187-211`).
  - The read-only print layout of `ForecastJournalView.svelte:101-128`, and
    `AmountCell.svelte`.
  - `AutoRuleEditor` has no save/cancel of its own (host-owned, :29-53).
  - The raw/form fallback in `AutoRulesPanel.svelte:240-280`.

> `scenarioModel.ts` contains NUL bytes, so plain `grep` misses its matches.
> Use `grep -a` (memory: grep-binary-ts-files).

## Decisions

Decisions marked **(P)** are Patrick's. The rest are this plan's. Each has a
reason, and any of them can be overridden.

### Part A

**A1. The statement's colours are the source; the chart follows them. (P)**
- The headings stay exactly as they are: `ACCENT`
  (`CashFlowStatementView.svelte:101-105`) keeps operating → `info`,
  investing → `secondary`, financing → `accent`. Not one heading class
  changes.
- The chart pins those same three theme colours as the activity series
  colours. "Exchange rates" is pinned to the palette's `other` grey: it is a
  residual, not an activity, and the statement gives it no accent.
- Every row without a pin (Net Worth, and anything else on `PeriodStackView`)
  stays biggest-first over the categorical slots, unchanged.
- The mapping lives in one module, `web/src/lib/reports/cashFlowColors.ts`:
  `CF_PIN: Record<CfSectionKind, ThemePin> = {operating: "info", investing:
  "secondary", financing: "accent"}` and `CF_EXCHANGE_PIN = "other"`.
  `ACCENT` stays a separate static map, because Tailwind's scanner needs the
  literal classes. A unit test asserts that the two maps name the same
  daisyUI colour per kind (for example `text-info` ↔ `"info"`), so they cannot
  drift.

**A2. A pin is a theme colour TOKEN, resolved the way the chart tokens are.**
- `ChartPalette` (`palette.ts:58-69`) gains
  `theme: {info: string; secondary: string; accent: string}`.
  - `resolvePalette` (`palette.ts:163-178`) reads them with the same
    `pick(token, backup)` it uses for `--chart-other`, from daisyUI 5's
    `--color-info`, `--color-secondary` and `--color-accent`.
  - `ReactiveChartColors` (`chartColors.svelte.ts:43-91`) re-resolves on a
    `data-theme` change, so a theme switch recolours the chart live with no
    new code.
  - `samePalette` (`palette.ts:181`) compares the new fields.
  - `DEFAULT_PALETTE` gains daisy dark's three values as the fallback. They
    are oklch strings, copied from `node_modules/daisyui`'s dark theme at
    implementation time, and test-pinned in the way `palette.test.ts` already
    pins `app.css`.
- `PeriodRow` (`reports/types.ts:516`) gains
  `pin?: "info" | "secondary" | "accent" | "other"`. It is a token name,
  never a colour string, so the report model stays theme-free.
- `stackSegments` carries `pin` onto the segment the way it carries `kind`
  (`periodStack.ts:160`). An `(own)` remainder inherits its row's pin.
- In `periodStack` (:219-245), a pinned segment's colour is
  `palette.theme[pin]` (or `palette.other`). Unpinned segments keep
  `colorAt(palette, slotsBySize(...))` over the full categorical palette.
  - The two sets are independent: pins are not palette slots, so no slot is
    reserved or skipped.
  - With no pins, the output is identical to today's, so Net Worth cannot
    move.
  - Rule 4 in the header (:32-40) gains one sentence on pins.
- SVG `fill` and `stroke` accept the oklch strings directly. WebKit has
  supported `oklch()` since 15.4, and the desktop app is WebKit (verify in
  the component test by asserting the attribute value is the token's
  resolved string).

**A3. Will the colours read apart on the chart?** On daisy dark, `info` is a
sky blue, `secondary` a pink-magenta and `accent` a teal. Against them sit the
net-change line (`flowNet`, near-white `#ecf9ff`) and `other` (grey
`#81878d`).
- The three hues are far apart for normal vision, and all three are clearly
  separate from the near-white line and the mid grey.
- The weak pair is **info vs accent** (blue vs teal) under tritanopia. In the
  bar chart this is mitigated by fixed order and the legend: activities always
  stack Operating, Investing, Financing, in table order (`cashFlowRows.ts:215`),
  and the legend names each. Accepted, not changed, because the headings are
  fixed by decision.
- A test computes CIEDE2000 between the three resolved defaults, `flowNet` and
  `other`. It asserts ΔE ≥ 20 for every pair, so a future daisyUI theme change
  that collapses two activities fails CI rather than shipping. The threshold
  sits below the expected info/accent value, to be measured when the test is
  written. It uses the same ΔE helper as `palette.test.ts` if one exists;
  otherwise a small one goes in the test.

### Part B

**B1. Declutter with a pure layout function.**
- New file: `web/src/lib/reports/ui/sankeyLabels.ts`.
- Signature: `layoutLabels(items: {key, anchor: number, size: number}[],
  opts: {top, bottom, twoLine: number, oneLine: number}) → {key, y, twoLine:
  boolean, displaced: boolean}[]`.
- One call per column, in node order. The algorithm:
  1. **Forward pass:** `y[i] = max(anchor[i], y[i-1] + gap(i-1, i))`, where
     `gap` is `twoLine` (≈ 26px: 11px name + 10px amount + 5px air) or
     `oneLine` (≈ 14px).
  2. **Backward pass** to clamp to `bottom`, then a forward clamp to `top`.
  3. **If the column still doesn't fit**, demote labels to one line, smallest
     node `size` first (ties by later index), and re-run, until it fits or
     every label is one line.
  4. **If even one-line labels don't fit**, the panel grows the chart, below.
- Order is always preserved. A label is never reordered past a neighbour.
- `displaced` means `|y − anchor| > 2px`.

**B2. Growing the chart.**
- `heightFor` becomes `clamp(200, max(colCount × oneLine + padding, nodes ×
  30), 1600)`.
- The 900px cap rises to 1600 only when one-line labels need the room. A
  normal P&L keeps today's height.

**B3. Leader lines.** For a displaced label, draw a 1px line in
`base-content/25` from the bar edge (`x0` or `x1`, at the node middle) to the
label anchor, with a short horizontal elbow.

**B4. Instant hover, and keyboard.**
- Native `<title>`s are removed, because they double up with the tooltip.
- Nodes and ribbons get `onpointerenter` / `onpointerleave`, and nodes also
  get `onfocus` / `onblur`.
- They feed one small, absolutely positioned tooltip `<div>` in the panel
  (`role="tooltip"`, `pointer-events-none`), styled like
  `PeriodStackChart`'s tooltip content.
  - Why not layerchart's `Tooltip.Root`: its pointer model is the chart's
    data grid, not SVG paths.
- Tooltip content:
  - **node:** label, amount, and share of its side's total.
  - **ribbon:** `source → target`, amount, and share of the source.
- Each node `<g>` gets `tabindex="0"`, `role="img"` and
  `aria-label="{label}: {amount}, {share}"`. Ribbons stay pointer-only;
  the legend already lists them.
- Hovering a node dims the unrelated ribbons to 0.1 opacity.

**B5. What `sankeyModel` gains.** `SankeyLinkView` gains `share` (of its
source). The tooltip strings are built in the model, not the component, so
they are unit-tested.

### Part C

**C1. Layout = file order. (P)**
- One list mirrors the scenario file top to bottom. Each list entry is one
  **piece** of the file:

  | piece                        | in the file                                                       | list entry                                                        |
  |------------------------------|-------------------------------------------------------------------|-------------------------------------------------------------------|
  | recurring rule               | a `~ PERIOD` block (any number of postings)                       | one card; badges for every section its postings fall in           |
  | one-off                      | a `~ YYYY-MM-DD` block                                            | one card, badge `one-off`                                         |
  | auto rule                    | a `= QUERY` block                                                 | one card, badge `auto`, readable summary                          |
  | comment                      | a free-standing run of column-1 comment lines                     | a note, or a **section heading** when it contains a divider line  |
  | hledger comment block        | `comment` … `end comment`                                         | a note, shown verbatim                                            |
  | disabled rule/one-off/auto   | a commented-out block that parses as exactly one `~` or `=` rule  | the same card, dimmed, with a `disabled` badge                    |
  | other journal text           | `account`, `commodity`, transactions, `P`, `include`, `alias` …   | a muted, read-only, NOT draggable "file text" row                 |

- **The header is not a piece.** That means the recognised header lines
  (`projection:`, `created:`, `updated:`, `cash:`, the banner). Its `cash:`
  stays on the existing "Default cash account" control above the list.

**C2. One card per `~` block, so step changes show as two cards.**
- A step change is two blocks, so it is two cards. Each shows a
  `step 1 of 2 · payroll` chip, and the chip links to (focuses) the sibling.
- Hovering one card outlines its siblings. When the siblings are adjacent,
  a thin rail joins them in the gutter.
- They **move independently**. hledger doesn't care about their order, and
  "the file's order" is now the contract.
- The card menu offers **Add a step change…**. It splits the block at a date
  you enter, inserts the new segment directly below, and opens it in the
  editor. That is `splitStep` reused, and the group key of the new half is
  fresh.
- **Merge with the other step** is `mergeStep`.
- `moveRow`'s "a step moves as one" and "postings swap inside a rule"
  semantics are **retired**.
  - Postings are reordered inside the editor, with ↑/↓ on a posting row.
  - `withCounter`'s "a line whose counter differs moves to a fresh group" is
    retired too. The counter-account is a **block** field in the editor (it
    is the block's amountless posting), so there is nothing to split.
  - To give one posting a different counter, use **Move posting to a new
    rule** in the editor. That splits it out into a new block directly below.

**C3. The edit dialog. (P)**
- Every editable card has an **Edit** button. Enter or `e` on the cursor
  entry, or a double-click, opens a modal dialog.
- The dialog is built on the journal modal's pattern: daisyUI `modal` +
  `dismissible` with `trap`, Cmd/Ctrl+Enter saves, and Escape cancels
  (`TransactionModal.svelte:136-201`).
- **Edits apply on Done. (P)**
  - The dialog edits a **draft**: a deep copy of the entry's lines, event or
    rule, its layout item and its attached comment, taken on open.
  - Nothing in `scenarioStore.scenario` changes while the dialog is open, so
    no run fires and the readings below stay as they were.
  - **Done** validates the draft (the existing `projectable` checks for that
    entry), commits it to the scenario in one mutation, and calls
    `scenarioStore.touch()` once. That gives exactly one debounced run.
  - **Cancel**, Escape or the backdrop discard the draft.
  - The draft lives in `projections/entryDraft.svelte.ts` (pure helpers plus
    a small rune class), so commit and discard are unit-tested without the
    component.
- **"Writes:" is exact, from the server. (P)**
  - The dialog's footer shows the block the save would write. It comes from
    `POST /api/projections/render` (C15), called with the draft, debounced
    300ms after each committed field.
  - It shows the server's text verbatim in a `<pre>`. If the server refuses,
    it shows the writer's own sentence (for example "every line of one rule
    must share its recurrence") in `text-error`, and **Done** is disabled
    until it renders.
  - While a request is in flight, the previous text stays, dimmed.
- Fields per kind:
  - **Every kind:** an "Enabled" toggle and "Comment above this entry".
    The comment is the attached comment (C5), as a textarea without `; `
    prefixes.
  - **Recurring rule:**
    - the period, either an interval select plus From and To
      (`DateInput`), or, for a period the simple controls can't state
      (`boundsLockedReason`), the raw expression in a monospace input,
      validated server-side by the run;
    - the description;
    - a postings grid. Each posting has an account (`AccountInput`), an
      amount (a magnitude, signed by account type as today's `entry` is —
      the grid shows the signed value the file will hold), a growth rate and
      unit, and an asset toggle. An asset posting also shows opening
      (override, with "use the journal's balance" and the journal figure)
      and gains account;
    - the counter-account, which is block-level;
    - a "comment" text field per posting and one for the header (C6);
    - step info: "step 2 of 2 of `payroll` — the other segment runs to
      2027-04-01", with a jump link.
  - **One-off:** the date, the description, postings (account, *signed*
    amount — amendment 19 of plan 22 stays), and the counter-account.
  - **Auto rule:** hosts `AutoRuleEditor` unchanged. If `form === null`, it
    shows a locked monospace view of `raw` with the reason. If the rule is
    unscoped, it shows the "Limit to forecasts in the file" button
    (ProjectionsTable :1290-1299 moves here).
  - **Comment:** a textarea plus a "Section heading / Note" switch.
- **Not editable:** "file text" pieces have no Edit button. Their row says
  "Edit this in the file".

**C4. Comment blocks are first-class entries. (P)** Attribution follows
these rules, applied to the file's top-level lines:

1. **Header first.** The leading recognised header lines are the header
   (`header_span`, unchanged). Everything after it is pieces.
2. **A comment run** is a maximal run of consecutive column-1 lines that
   start with `;`, `#` or `*`. These are hledger's three top-level comment
   characters (`parse.rs:916`). The run must contain no blank line.
3. **Disabled blocks are carved out of runs first** (C7). What is left of a
   run, before and after each carved block, is a smaller run.
4. **Section heading.** A run that contains a **divider line** is a
   free-standing comment piece with style `section`. A divider is a line
   which, after its comment characters are stripped, is empty or consists of
   three or more of `- = _ ~ * # ;` and spaces. `;;;;;;;;` and
   `; ---------` are dividers. Such a run is never attached to a rule.
5. **Attached comment.** A run with no divider, directly above a `~` or `=`
   block (or a disabled one) with no blank line between, is that entry's
   **attached comment**. It moves, duplicates, disables and deletes with it.
   This is how the docs' own example reads: "; A STEP: payroll jumps…"
   sits directly above its rule (`docs/projections/file-format.md`).
6. **Free note.** Any other run is a free-standing piece with style `note`.
7. `comment` … `end comment` is a free-standing piece with style `block`,
   verbatim to EOF if unterminated (the parser's own behaviour,
   `periodic.rs:1488-1497`). Its contents are never interpreted.
8. **Inside a block** — the header's `; …` and indented comment lines — the
   text belongs to the rule (C6). It is never a piece.

- **Writing a NEW section heading** always uses Patrick's own hand style
  (P): a line of exactly 22 semicolons, then `;; Title`. That is two lines,
  with no closing divider:

  ```journal
  ;;;;;;;;;;;;;;;;;;;;;;
  ;; Operational expenses
  ```

  **Existing headings keep their bytes and their style,** whatever framing
  they use. A heading that is not edited is never re-rendered (C5).

- **Editing a heading** rewrites only its non-divider lines. **A note** is
  written `; ` + text per line.
- **Separation.** A free-standing piece is always followed by a blank line
  when anything follows it. Without that, the next read would attach it to
  the entry below and the round-trip check would refuse the save.

**C5. A piece model, written by reordering whole pieces.** This replaces
today's per-kind slot filling with one rule that covers everything:

- **The parse.** `ProjectionDoc::parse` produces
  `header`, `lead` (blank lines after the header), and `pieces: Vec<Piece>`.
  - Each piece has a `span` (its content lines, attached comment included)
    and a `gap` (the blank lines that follow it, up to the next piece).
  - It also has the `kind`, the `disabled` state, and its identity (C8).
  - For a rule, event or auto piece it records what the piece states, as
    `chunk_print` / `raw` prints it.
- **The write.** `apply(scenario)` emits:
  1. the header (as today);
  2. the original `lead`;
  3. each `scenario.layout` item in order: its **body**, then its
     **separator**.
- **Body.**
  - **A claimed piece whose content is unchanged** is written with its own
    original bytes, **even when it has moved**. That is better than today
    (:592).
  - **If only its disabled state changed**, those bytes are commented or
    uncommented line by line (C7). Its attached comment is untouched.
  - **Otherwise it is rendered**, with its attached comment written from
    the original bytes if unchanged, else rendered `; ` + line.
- **Separator.**
  - A claimed piece keeps its own original `gap`.
  - If its next neighbour in the output is not the piece that followed it in
    the file, it gets at least one blank line when its gap had none, so
    nothing glues together. One exception: a "file text" piece that
    originally had no gap keeps none.
  - A new piece gets one blank line.
  - The output ends with exactly the file's original final-newline
    convention, in the file's `Newline`.
- **Removal.** A piece not in the layout is dropped with its gap.
- **Fixed pieces stay put.** "File text" pieces are in the layout as `fixed`
  items. The writer refuses (`Invalid`) a layout that reorders two `fixed`
  items relative to each other, or drops one. The client never drags them,
  but other entries can be dragged past them, and the list shows exactly
  where they will land.
  - If a move across a scoping directive (`alias`, `apply account`, `Y`,
    `decimal-mark`) changes what a rule means, `confirm_round_trip` catches
    it. The save is refused with "moving this entry past `alias …` changes
    its accounts".
- **The guarantees,** each a test:
  1. **Identity.** `apply(read(t))` with the read layout gives back `t`,
     byte for byte, apart from the header stamp. This holds for every file
     the reader accepts, CRLF included.
  2. **Locality.** Every byte of an unchanged piece appears verbatim in the
     output, whatever was moved, edited, added or deleted around it.
  3. **Round trip.** Re-reading the output yields the layout asked for:
     kinds, order, disabled flags, comment texts and each entry's print.
     `confirm_round_trip` is extended to check exactly this, in **layout
     order**. Today it checks rules-then-events (`prints`, :1861).
  4. **Toggle involution.** Disabling then enabling an entry restores its
     bytes exactly.
  5. **hledger blindness.** The scenario re-read with disabled pieces
     removed equals the projection's active scenario.

**C6. Comments inside a block survive an edit.** Today they don't. This
lands with C5, because "the file's text, readable" is the point of the
redesign.
- `ScenarioLine` gains `comment: String`. It holds the posting's comment with
  Ledgeline's own tags (`line`, `growth`, `opening`, `gains`) removed.
  Continuation comment lines are kept, since the parser already folds
  indented comment lines into the preceding posting's comment.
- The rule gains a header comment, carried as `ScenarioLine.rule_comment`
  (group-level, like `note`). Events gain `comment` and per-posting
  `comments`.
- `line_tags` writes Ledgeline's tags first and then the user's text, after
  `, ` when that text has no tag of its own, and on a continuation comment
  line otherwise. That keeps hledger's tag parsing from ever reading the
  user's text as a Ledgeline tag value. This is verified by parity (C14).
- `chunk_print` includes the comments, so changing only a comment counts as
  a change.

**C7. Disable = comment out the whole entry. (P)**
- **Writing a disabled entry:** prefix every line of the block with `; `, and
  leave the attached comment as it is. Example:

  ```journal
  ; Big new customer — the "with" alternative
  ; ~ monthly from 2027-01-01  acme contract
  ;     revenues:acme        $-40000
  ;     assets:checking
  ```

- **Reading one.** In a comment run, a candidate starts at a line whose
  stripped text (one comment character, then at most one space, removed)
  begins with `~` or `=`. Its body is the following lines of the run whose
  stripped text is indented and non-blank. The candidate is a **disabled
  entry** only if:
  - every line of it carries the **same** prefix (comment character plus
    optional space), so a mixed `;`/`#` run is never sliced;
  - its body has at least one line that is not itself a comment, i.e. a
    posting;
  - the stripped text, parsed **alone** with
    `parse_journal_with_styles` (same inherited styles), yields exactly one
    `~` or exactly one `=` rule, no error, and nothing else.

  Anything else is prose. So `; ~ monthly is how often` (no body) and
  `; = Totals =` (no posting) stay comments.
- **The cost of a false positive is zero.** A prose comment that *is* a
  complete, parseable rule shows as a disabled entry. It still round-trips
  byte for byte, because unchanged pieces keep their bytes.
- **hledger's `comment` / `end comment` is NOT read as disabled.**
  Ledgeline never writes it, its contents may be arbitrary, and a block
  might hold several rules. It is a `block` comment piece (C4.7).
- **Disabled entries reach the engine as data, flagged.** A disabled `~`
  block's lines go in `scenario.lines` (group `off:K`) and a disabled
  one-off in `scenario.events` (id `off:K`), where K is its index among the
  file's disabled `~` blocks. A disabled `=` rule goes in
  `scenario.auto_rules`. The flag lives on the **layout item** (C8), the
  one place it is stated.
  - `Scenario::active(&self) -> Scenario` drops every disabled group,
    event and auto rule.
  - `forecast::expand` and `forecast::expand_from` both start with it. They
    are the only engine readers, so a disabled entry cannot leak into
    numbers.
  - The SPA's `projectable()` drops them too, to keep run bodies small.
  - A property test asserts that disabling equals deleting, for the numbers.
- Unrelated keys: `off:K` contains a colon, which no written `line:` id can
  (file-format.md, "line:"), so it can never be joined to a step.

**C8. Layout on the model and on the wire.**

```rust
// projections.rs
pub struct Scenario {
    /* existing fields … */
    pub comments: Vec<ScenarioComment>,
    pub fixed: Vec<FixedPiece>,
    /// The file's pieces in order. Empty = "derive it" (legacy body, seed).
    pub layout: Vec<LayoutItem>,
}
pub enum ItemKind { Rule, Event, Auto, Comment, Fixed }
pub struct LayoutItem {
    pub kind: ItemKind,
    /// Rule: the group. Event: its id. Auto: `ScenarioAutoRule::key`.
    /// Comment: `ScenarioComment::key`. Fixed: `FixedPiece::key`.
    pub key: String,
    /// The piece of the file this item was read from (its index in
    /// `ProjectionDoc::pieces`), or None for a new one.
    pub origin: Option<usize>,
    pub disabled: bool,           // Rule | Event | Auto only
    /// The attached comment, as the file states it (prefixes included),
    /// or "" for none. Edited text comes in as `attached_text`.
    pub attached: String,
}
pub struct ScenarioComment { pub key: String, pub style: CommentStyle, pub raw: String }
pub enum CommentStyle { Note, Section, Block }
pub struct FixedPiece { pub key: String, pub raw: String }
// ScenarioAutoRule gains `key: String` ("auto:B" on read, a fresh id for new).
```

**Wire.**
- `WireScenario` gains:
  - `layout: [{kind, key, origin, disabled, attached}]`
  - `comments: [{key, style, raw, text}]` — `text` is the display form:
    prefixes stripped, and for a section the divider lines dropped
  - `fixed: [{key, raw}]`
- `ScenarioIn` takes:
  - `layout` (optional), each item possibly with `attachedText` in place of
    `attached` when edited;
  - `comments`, each `{key, raw}` unchanged or `{key, style, text}` edited.
    This mirrors `AutoRuleIn`'s raw/form duality (`projections_api.rs:916`).
  - `fixed` as `{key}` only. The server takes the bytes from the file and
    never from the client.
- `WireScenarioLine` gains `comment` and `ruleComment`. Events gain the same.
- **Server-side validation:**
  - every layout key resolves exactly once;
  - every line, event and auto rule is referenced;
  - `fixed` order is preserved;
  - limits `MAX_COMMENTS = 1000` and `MAX_COMMENT_BYTES = 64 KiB`, with
    the existing control-character rules (`check_inline`);
  - all → 400 with a sentence.
- **An empty or missing `layout`** (a pre-31 client, a seed, the FFI) is
  derived by `legacy_layout(doc, scenario)`. It reproduces today's semantics
  exactly: per-kind slot filling via the existing `assign_to` and claim
  passes, every comment, disabled and fixed piece kept where it is, and new
  entries appended. **That function is the only home of the old algorithm.**
  It keeps the FFI and older SPAs correct, and it is deleted only when
  nothing sends a layout-less body.

**C9. Claiming a piece.** This is amendment 12's three passes, generalised to
every kind:
1. Origin plus same content (by print or raw).
2. Any unclaimed piece of the same kind with the same content. This covers
   stale origins.
3. Origin alone (an edited entry).

Kinds must match, with disabled and active of one kind counted as the same
kind. A claimed piece's bytes are reused only per C5's body rule. So a stale
or wrong claim can cost byte preservation, never correctness, because
`confirm_round_trip` stays the backstop.

**C10. After a save, the client adopts the echo when it's safe.**
- If `scenarioStore`'s revision did not change while the PUT was in flight,
  the page **adopts** the response's scenario. That makes origins and keys
  fresh, and the cursor is restored by layout index.
- Otherwise it keeps `markSaved()` (today's rationale, `+page.svelte:207-209`),
  and C9's pass 2 keeps the next save byte-faithful.
- This replaces the stale-identity reasoning that amendment 12 had to
  tolerate.

**C11. Duplicate, and Duplicate as alternative. (P)** Both go through C3's
draft model, so neither touches the scenario until **Done**.
- **Duplicate** opens the dialog on a **new-entry draft**. The draft is a
  copy of the entry with an insertion point directly below the original.
  - The copy gets fresh keys: a new group via `freshId`, a new event id or a
    new auto key.
  - It has `origin: None`, so it is always rendered.
  - It has **no `line:` ids** (a copy is never a step of the original), and
    it copies the attached comment.
  - While the dialog is open, the list shows a dashed placeholder card
    ("New copy — editing") below the original, so you can see where it will
    land.
  - **Done** inserts the copy in one mutation, giving one run, and the
    cursor moves to it.
  - **Cancel** discards the draft. The copy never existed, so "Cancel removes
    the fresh duplicate" holds by construction.
- **Duplicate as alternative** does the same. The draft also records
  `disableOriginal: true`, and the original shows a "will be turned off"
  hint while the dialog is open.
  - **Done** inserts the copy **and** sets the original's layout item
    `disabled = true`, in the same single mutation.
  - **Cancel** discards both. The original was never disabled, so it stays
    enabled: "Cancel re-enables the original" holds without any undo step.
  - After Done, the list's live region says "Original turned off; copy
    added".
- Both actions are available on rule, one-off and auto entries. Comments get
  plain Duplicate.
- The same draft-insertion path serves **Add** (from the Add menu) and **Add a
  step change…** (C2). The new entry exists only after Done.

**C12. Kinds, badges and filters.**
- A rule card's badges come from `line_section` per posting, which the
  server already computes as `resolved.section`. Sections are
  income / expense / asset / one-off. So a mixed block shows both `income`
  and `expense`.
- Other badges: `auto`, `note`/`section` for comments, and `disabled`.
- **Filter chips** (All · Income · Expense · Asset · One-off · Auto ·
  Disabled · Notes) show counts. Choosing a chip hides non-matching entries.
  Section headings stay visible, so the context survives.
- **While a filter is on,** drag and J/K are disabled, with a hint: "Clear
  the filter to reorder." Moving relative to hidden entries would be
  ambiguous.
- The chosen filter is kept in the URL (`?show=`), like the other tabs'
  view state.

**C13. Keyboard.** One layer:
`registerKeys({id: "projection-list", priority: PRIORITY.widget, group:
"Projections"})`.

| keys              | action                                                           |
|-------------------|------------------------------------------------------------------|
| `j` / `↓`         | next entry                                                       |
| `k` / `↑`         | previous entry                                                   |
| `g g` / `G`       | first / last                                                     |
| `Enter` / `e`     | edit (view, for file text)                                       |
| `J` / `K`         | move entry down / up (not on fixed; not while filtered)          |
| `c`               | duplicate                                                        |
| `C`               | duplicate as alternative                                         |
| `t`               | turn off / turn on                                               |
| `x` then `x`      | delete (arm, confirm — the journal's idiom)                      |
| `n`               | open the "Add" menu                                              |
| `Escape`          | disarm, else clear the cursor                                    |

- These are the RulesList conventions plus the journal's `x x`.
- The cursor is visual (`listCursor`, keyed by layout key), with
  `aria-current` on the entry and `scrollIntoView({block: "nearest"})`.
- The digit bindings of `ProjectionReports` (`:113-122`, `PRIORITY.page`)
  don't collide.
- Every binding also exists as a menu item. Drag handles stay
  `aria-hidden`, and the menu is the accessible path, as today
  (`ProjectionsTable.svelte:716-719`).

**C14. hledger compatibility is a test, not a hope (memory:
projections-hledger-compatible).**
- Disabled entries and notes are comments, and hledger ignores them.
- Attached comments are comments.
- Block comments (C6) use hledger's own tag syntax.
- The parity suite gains a fixture that runs `hledger print --forecast` over
  the written file and over the same file with disabled blocks deleted, and
  requires identical output.

**C15. One entry's exact text: `POST /api/projections/render`. (P)**
- **Purpose.** It feeds the dialog's "Writes:" preview (C3). It renders one
  entry exactly as a save would write it as a new or edited piece. It never
  reads or writes a file.
- **Core:**
  `serialize::render_entry(scenario: &Scenario, key: &str, declared,
  inherited) -> Result<String, SerializeError>`.
  - It builds `ProjectionDoc::empty(declared).inheriting(inherited)` and
    renders the one layout item through the **same** body path `apply` uses
    for a non-claimed piece:
    - the attached comment;
    - `render_chunk` / auto `render` / the comment renderer;
    - the C7 `; ` prefix when disabled;
    - C6 comments.
  - It then re-reads the snippet with the reader and requires it to state
    the entry (a one-item `confirm_round_trip`).
  - No second renderer exists, so the preview cannot disagree with the save.
  - LF line endings: the preview is display, and the file's own `Newline`
    is applied only at save.
- **Server:** `projections_api.rs`, a new handler `render`.
  - **Body:** `{scenario: ScenarioIn, key: string}`. `scenario` carries only
    what the entry needs: that entry's lines, event, auto rule or comment,
    its one layout item with `attachedText`, `cash` and `accountTypes`.
  - It is decoded by the existing `DecodedScenario` and `signed` (the main
    journal's declared types and `JournalStyles`, inside `compute`), so the
    signs and styles are the save's.
  - **Response:** `{text: string}` with `no_store`, or 400 with the writer's
    sentence.
  - **Limits:** the existing body limits, at most one layout item, and
    `MAX_LINES` applied as for a run.
  - **No `import_writes` lock and no discovery.** This handler touches no
    file.
- **The original text travels too.** Outbound `LayoutItem` gains `raw`: the
  piece's exact text, `\n`-normalised, or `""` for a seed's or new entry.
  - The dialog shows `raw` until the first edit, because an unchanged piece
    is written with its own bytes (C5). After that it shows the render.
  - The card's "show rule text" disclosure uses the same `raw`.

## Scope

**In:** Parts A, B and C as decided, docs, screenshots, and test migration.

**Out:**
- editing "file text" pieces;
- reading `comment`/`end comment` as disabled;
- an expanded growth export;
- Save As copying "file text" pieces. Today Save As writes only the
  scenario, and that stays true. Their absence from a copy is called out in
  the Save As dialog when the file has any (see Risks);
- multi-select;
- undo beyond Cancel.

## Phase A — Cash flow colours (PR 1)

**Code**
- `format/palette.ts`:
  - `ChartPalette.theme`;
  - `THEME_TOKENS = {info: "--color-info", secondary: "--color-secondary",
    accent: "--color-accent"}`;
  - `resolvePalette` reads them, `samePalette` compares them, and
    `DEFAULT_PALETTE.theme` holds daisy dark's values;
  - `pinColor(palette, pin)`.
- `format/chartColors.svelte.ts`: no logic change. It re-resolves the whole
  palette on a theme switch already. It exposes `theme` beside `other`.
- `reports/cashFlowColors.ts` (new): `CF_PIN`, `CF_EXCHANGE_PIN`.
- `reports/types.ts:516`: `PeriodRow.pin?: "info" | "secondary" | "accent" | "other"`.
- `cashFlowRows.ts:214-224`: set `pin` from `section.kind`, and `"other"` on
  the exchange row.
- `periodStack.ts`:
  - `StackSegment.pin`;
  - in `periodStack`, a pinned segment takes `pinColor(palette, pin)` and an
    unpinned one keeps `colorAt(palette, slots[i])` from `slotsBySize` over
    the unpinned segments only;
  - one sentence on pins in rule 4.
- `CashFlowStatementView.svelte`: **unchanged.**

**Tests (vitest)**
- `palette.test.ts` and `chartColors.svelte.test.ts`:
  - `resolvePalette` reads the three `--color-*` tokens and falls back per
    token when one is empty;
  - a `data-theme` change re-resolves them through the injected `read`;
  - `samePalette` sees a change in `theme`;
  - `DEFAULT_PALETTE.theme` equals daisy dark's values.
- `periodStack.test.ts`:
  - a pinned row gets its token's colour whatever its size;
  - unpinned rows are coloured biggest-first among themselves;
  - with no pins, colours are identical to today's. The existing :254-263
    and :285-385 run unchanged, and that is the Net Worth guarantee.
- `cashFlowRows.test.ts`: each series carries its kind's pin, and exchange
  carries `"other"`.
- `cashFlowColors.test.ts`:
  - `CF_PIN` and `ACCENT` name the same daisyUI colour per kind (A1);
  - the ΔE distinguishability check of A3.
- `ReportCharts.svelte.test.ts`: in a CF chart where financing is the
  biggest, the financing bars and legend swatch still use the resolved
  `accent` colour, and exchange uses `other`.
- `CashFlowStatementView.svelte.test.ts`: unchanged, as a guard that the
  headings didn't move.

## Phase B — Sankey labels and hover (PR 2)

**Code**
- `reports/ui/sankeyLabels.ts` (new, pure): `layoutLabels` per B1.
- `sankeyModel.ts`: `SankeyLinkView.share`, plus `nodeTooltip(node)` and
  `linkTooltip(link)` returning `{title, lines[]}`.
- `SankeyPanel.svelte`:
  - compute label positions per column from the layerchart node geometry,
    in a `$derived` over `nodes`;
  - draw the leader lines, the one-line demotion and the tooltip `<div>`;
  - `heightFor` per B2;
  - remove the `<title>`s;
  - nodes become focusable.

**Tests**
- `sankeyLabels.test.ts` (table-driven plus a fast-check-style randomised
  loop, using the vitest seed):
  - order is preserved;
  - spacing ≥ the gap of each pair;
  - every label within `[top, bottom]` whenever `n × oneLine ≤ height`;
  - no label moves when there is no overlap;
  - demotion picks the smallest first;
  - `displaced` is correct.
- `sankeyModel.test.ts`: ribbon share, tooltip strings.
- `SankeyPanel.svelte.test.ts`:
  - 40 accounts give no two label boxes closer than the gap (read from the
    `y` attributes);
  - a leader line exists for each displaced label;
  - hover or focus on a node shows `role=tooltip` with its amount and share;
  - Escape and blur hide it;
  - no `<title>` remains.
- Manual: `just dev` against `fixtures/corpus` (many accounts).

## Phase C1 — The piece model and the writer (core only, PR 3)

**Code** (`crates/ledgeline-core`)
- **`projections/layout.rs` (new, pure):**
  `scan(text, header_end) -> Vec<RawPiece>`. It implements C4 and C7's
  candidate detection. Disabled candidates are confirmed by the caller,
  which parses them.
  - Its `~` spans must equal `PeriodicDoc`'s, and its `=` spans must equal
    `auto_edit::block_spans`. Both are asserted in tests over every fixture
    and the generator below.
- **`projections.rs`:**
  - `Scenario.{comments, fixed, layout}`, `LayoutItem`, `ItemKind`,
    `CommentStyle`, `ScenarioComment`, `FixedPiece`;
  - `ScenarioAutoRule.key`;
  - `ScenarioLine.{comment, rule_comment}`, `ScenarioEvent.{comment,
    posting_comments}`;
  - `Scenario::active()`.
- **`forecast.rs:891`** and `expand_from` call `active()` first.
- **`serialize.rs`:**
  - **reading:** `read_scenario` builds the layout from `layout::scan` plus
    the journal parse. Disabled candidates are parsed alone and given
    `off:K` keys.
  - **`ProjectionDoc`:** replace `blocks` / `current` / `auto_blocks` with
    `pieces: Vec<Piece>`.
  - **`apply`:** C5. **`legacy_layout`:** C8. The old `assign_to`,
    `auto_splices` and `removal` logic moves here and is used only to
    *derive* a layout.
  - **`block_order`:** derived from the layout, so same-day forecast order
    follows the file.
  - **`confirm_round_trip`:** layout order, plus disabled flags, comments
    and attached comments. `confirm_auto_rules` folds into it.
  - **`line_tags` / `rule_header`:** C6.
  - **Module docs:** rewrite the "discipline" section and the mapping
    table, and record this plan as the source.
- **`seed_scenario`:** unchanged (layout empty → derived).

**Tests**
- **Unit (`serialize.rs` / `layout.rs`):**
  - every C4 rule, including: a divider run directly above a rule (a
    section, not attached); `#` and `*` runs; a mixed-prefix run; CRLF; a
    file ending without a newline; a comment between a rule and EOF; an
    unterminated `comment` block;
  - every C7 rule: prose look-alikes (`; ~ monthly is…`, `; = Totals =`);
    a disabled one-off; a disabled auto rule; a disabled block with an
    attached comment;
  - a candidate whose stripped text has a parse error (it stays prose).
- **Existing tests whose meaning changes:**
  - "comments between rules stay where they were" becomes "attached
    comments move with their rule; free comments move only when moved";
  - "rules and events fill separate slot sets" holds only under
    `legacy_layout`;
  - "a moved chunk is re-rendered" becomes "a moved unchanged block keeps
    its bytes".

  List each in amendment 1 when changed.
- **Property tests (`tests/projection_pieces.rs`, proptest, already a
  dev-dependency):** a generator of scenario files from pieces:
  - rules with 1–4 postings, growth, opening and gains;
  - events and auto rules;
  - notes, sections and block comments;
  - disabled versions of each;
  - fixed `account` directives;
  - random gaps of 0–2 blank lines (never 0 after a free comment);
  - LF or CRLF.

  Properties: C5 guarantees 1–5, plus "a random permutation of the
  non-fixed items reads back in that order", "delete k random items" and
  "edit one item's amount changes only that piece's bytes".
- **Fixtures:** `fixtures/projections-parity/comments.journal`. It has
  Patrick-style `;;;;` sections, notes, attached comments, a disabled rule,
  a disabled one-off, a disabled auto rule, a `comment` block and a step
  pair.
- **Parity (`tests/projection_files.rs`, `LEDGELINE_HLEDGER_PROJECTION_CHECK=1`
  via `just hledger-checks`):**
  - Written files load in hledger 1.52.
  - `print --forecast` of the comments fixture equals the print with the
    disabled blocks removed.
  - `bs` and `is` parity on it, as the existing harness does
    (`projection_parity.rs`).
  - A file written with a header comment and posting comments (C6) reads
    the same tags in hledger: `print --forecast tag:growth` selects the same
    postings.
- **Disabling equals deleting:**
  `project(s with item i disabled) == project(s without item i)` over the
  generator. This is a property test in `projection_cache.rs`'s style, and
  `project_cached` must give the same answer.

## Phase C2 — The wire (server and TS types; old UI still works, PR 4)

**Code**
- **`projections_api.rs`:**
  - `WireScenario` / `ScenarioIn` / `WireAutoRule` / `AutoRuleIn` per C8,
    `deny_unknown_fields` kept;
  - `DecodedScenario` carries the layout, comments and attached texts;
  - the C8 validation, with limits;
  - the save path otherwise unchanged.
  - `run`: disabled items are excluded by the engine (C7). A run body may
    omit the layout.
  - outbound `LayoutItem.raw` (C15).
- **The render endpoint (C15):**
  - core `serialize::render_entry`, factored out of `apply`'s
    non-claimed-piece body path so both call one function;
  - the `render` handler;
  - the route `POST /api/projections/render`, registered beside
    `/api/projections/run` (`lib.rs:1139`). It is a literal segment like
    `run`, so it cannot be shadowed by `{*id}`, and ids must end in
    `.journal` anyway;
  - TS `LedgelineApi.renderProjectionEntry(body): Promise<{text: string}>`
    next to `runProjection` (`api/native.ts:1124`), with a decoder in
    `nativeDecode.ts`.
- **TS:**
  - `projections/types.ts`: `LayoutItem`, `ScenarioComment`, `FixedPiece`,
    `AutoRule.key`, the line and event comment fields;
  - `api/native.ts:762` `WireScenarioIn`;
  - `api/nativeDecode.ts:2507` `decodeScenario` and `decodeAutoRules`, with
    shape tests in `nativeDecode.test.ts:2168+`.
- **`scenarioModel.ts` `scenarioToWire` (:873):** in THIS phase it sends
  `comments`, `fixed` and `layout` **only when the scenario has not been
  reordered by the old table**. Simplest: it never sends `layout` in C2, so
  the server's `legacy_layout` keeps comments, disabled and fixed pieces in
  place. It does send the line and event comments (C6), so the old table
  stops losing them.
- **Goldens:** `fixtures/native/v1/projections-*.json` regenerate (they gain
  the fields).
- **Integration:** run `just test-integration` after the shape change
  (memory: live-integration-tests-skip).

**Tests**
- Server: a round trip through `GET` → `PUT` of the comments fixture is
  byte-identical except `updated:`.
- Server: a 400 for each validation.
- Legacy body: a body without `layout` behaves exactly as before. Port three
  existing save tests to assert it.
- TS: decoder shape tests, and `scenarioToWire` echoing comments.
- **Render, core (`serialize.rs` tests):**
  - `render_entry` equals the bytes `apply` writes for the same entry
    appended to an empty document (header excluded). That is asserted over
    the C1 property generator, so the preview and the save cannot diverge;
  - a disabled entry renders prefixed;
  - an attached comment and C6 comments render;
  - a refused field returns the same sentence `write_scenario` gives.
- **Render, server:**
  - 200 with the text for a rule, a one-off, an asset row, an auto form and
    a comment;
  - 400 for two layout items, an unknown key and an unwritable field;
  - a magnitude is signed by the main journal's declared types, the same
    figure a save of the same draft writes (asserted by saving it to a
    scratch file and comparing the block);
  - no file is created or touched (the scratch directory listing is
    unchanged).
- **Render, TS:** decoder tests, plus a `nativeDecode` shape test for `{text}`.
- **Live integration:** add a render case to the live-engine integration
  suite and run `just test-integration` (memory:
  live-integration-tests-skip). If `fixtures/native/v1/requests.tsv` lists
  POST routes by then, add a `projections-render` golden. Today it lists
  only GETs (:34-46), so the integration test is the cross-language check.

## Phase C3 — The list and the dialog (PR 5)

The commits, in order, inside one PR (ProjectionsTable can only go once
every kind has an editor):

1. **`projections/entries.ts` (new, pure; tested in `entries.test.ts`).**
   - `entriesOf(scenario, declared): Entry[]`. `Entry` is a tagged union
     `{kind: "rule"|"event"|"auto"|"comment"|"fixed", key, item:
     LayoutItem, badges, step?: {id, index, of, siblings}, …}`. A scenario
     with an empty layout derives one in the server's legacy order: rules
     by first appearance, then events, then auto rules.
   - Mutations on the scenario, by key:
     - `moveEntry(s, key, toIndex)`, which refuses to reorder fixed items
       relative to each other;
     - `insertAfter(s, key, entry)`, `removeEntry`, `toggleDisabled`;
     - `duplicateEntry(s, key) → newKey`;
     - `duplicateAsAlternative(s, key) → newKey`;
     - `addEntry(s, kind, afterKey)`;
     - `addStep(s, key, date)` (wraps `splitStep`), `mergeStep`;
     - `movePostingToNewRule(s, group, lineIndex)`;
     - `setAttached(s, key, text)`, `setComment(s, key, text, style)`.
   - Filtering: `entryMatches(entry, filter)`.
   - Kept from `scenarioModel.ts`: magnitudes, periods, ids, `splitStep` and
     `mergeStep`, growth, notes, `scenarioToWire`, `projectable` (which now
     drops disabled), `runBody`, names.
   - **Deleted:** `resolvedSection`'s hold logic (`sectionOfLine`,
     `HeldSection`), `groupLines`/`logicalRows`/`rowNames`, `moveRow`,
     `moveRowTo`, `moveEvent`, `moveEventTo`, `gatherRules`, and
     `withCounter`'s split. Move their tests or delete them.
2. **Display components** (`projections/ui/list/`):
   - `EntryFrame.svelte`: the grip, kind badges, a disabled overlay
     (`opacity-55` plus a struck header line, with a `disabled` badge
     announced to screen readers), the cursor ring
     (`ring-2 ring-primary`, as in `RulesList`), hover and focus-within
     affordances, the Edit button and `RowMenu`.
   - `RuleEntry.svelte`, `EventEntry.svelte`, `AutoEntry.svelte`,
     `CommentEntry.svelte` (note / section heading / block), `FixedEntry.svelte`.
   - The text-like body is a CSS grid: `[gutter] [account 1fr] [amount
     auto] [annotations auto]`, `font-mono text-sm tabular-nums`, with the
     amounts right-aligned in one column per card. It reuses
     `AmountCell`'s formatting (`journal/AmountCell.svelte:9`) and the
     journal's `text-error` for negatives.
3. **`ScenarioList.svelte`:**
   - the filter chips, the Add menu (Recurring flow · Asset · One-off ·
     Automatic posting · Note · Section heading), and empty states
     (C-UX below);
   - the cursor and the keymap (C13);
   - native HTML5 drag, anywhere to anywhere, with a drop line between
     cards, reusing the `dragStart`/`dragOver`/`drop` pattern of
     `ProjectionsTable.svelte:659-689` without zones;
   - the live region.
4. **`projections/entryDraft.svelte.ts`** (C3 draft: `draftOf`, `commitDraft`,
   new-entry drafts) and **`EntryDialog.svelte`** (the modal shell, the
   draft, and the debounced `renderProjectionEntry` preview; C3, C15) and
   `editors/RuleEditor.svelte`, `EventEditor.svelte`,
   `AutoEntryEditor.svelte` (wraps `AutoRuleEditor`, plus the raw fallback
   from `AutoRulesPanel.svelte:240-280`), and `CommentEditor.svelte`.
5. **`+page.svelte`:** mount `ScenarioList` in place of `ProjectionsTable`
   (:422-446), and adopt after save per C10.
   - The read-only notice moves up beside the list header: "Read-only
     here: the main journal includes this file. Edit freely and Save As a
     copy." The text comes from `ScenarioFileBar.svelte:124-130`.
6. **Deleted:**
   - `ProjectionsTable.svelte` and its test;
   - `NoteButton.svelte` and its test (its only user was ProjectionsTable;
     the description is edited in the dialog and shown in full on the card);
   - the `section-move-notice` region.
   - `RowMenu.svelte` stays.

**UX requirements** (each is a component test):
- **Scannable.** One card per entry, with a 4px gap between cards in a
  run and 16px above a section heading. Section headings are `text-sm
  font-semibold uppercase tracking-wide` with a rule, and they are sticky
  under the page header while their run scrolls.
- **Amounts align** within a card. The counter-account is a last line
  `→ account` in `text-base-content/60`. Growth is a `↗ 3%/yr` chip. An
  asset posting shows `bal $650,000 (journal)` or `bal $700,000 (override)`
  and `gains → …`.
- **The step chip** links to and outlines its siblings. An `estimated` badge
  marks `source: unbudgeted` lines, carrying today's seeded-row distinction,
  and a stale description shows a dot (`noteIsStale`).
- **Auto rule summary.** It reads as a sentence: "When `revenue:sales` is
  projected: `expenses:…:sales-commission` 3% of it → `assets:mercury:checking`".
  This is `rulePreview` (`ruleForm.ts:167`) plus the form. The raw text is
  shown in a disclosure, and a locked rule shows a lock and its raw text.
- **Empty states:**
  - a new, empty scenario: "No entries yet", with the four Add buttons;
  - a seed: "Seeded from your budget and the last 12 months — nothing is
    saved until you Save As";
  - filtered to nothing: "No {kind} entries. Clear filter".
- **Focus.** Opening the dialog focuses its first field. Closing returns
  focus to the card's Edit button (`dismissible` does this). The cursor
  follows a moved or duplicated entry.
- **Reduced motion.** Moves animate with `animate:flip` (120ms), and not at
  all under `prefers-reduced-motion`.
- **Accessibility:**
  - each card is an `article` with `aria-labelledby` its header line;
  - the list is a `role="list"`;
  - kind badges have text (not colour only);
  - disabled is announced ("turned off");
  - all actions are reachable from the menu and the keyboard.
  - axe runs in the e2e test (`@axe-core/playwright` if already present;
    otherwise a manual pass, noted).

**Tests**
- **Ported from `ProjectionsTable.svelte.test.ts`,** by behaviour, into
  `ScenarioList.svelte.test.ts` and `EntryDialog.svelte.test.ts`:
  - seeded scenario;
  - editing marks dirty;
  - bounded periods stay editable (in the dialog);
  - step add/remove (now split/merge);
  - assets and the balance column (in the dialog);
  - one-off events;
  - counters, gains and cash;
  - description;
  - reordering (now: any entry anywhere, fixed pinned, disabled while
    filtered);
  - auto postings.
  - The "SECTION-STABILITY RULE" and "hold" tests are deleted on purpose:
    the hold is gone because nothing moves when you type.
- **New:**
  - j/k/J/K/Enter/e/c/C/t/x x/n/Escape;
  - Duplicate opens the dialog on a draft copy with a placeholder card.
    Done inserts it directly below and runs once. Cancel leaves the
    scenario deep-equal to before, with no run;
  - Duplicate as alternative: Done inserts the copy and disables the
    original in one mutation, with one run. Cancel leaves the original
    enabled and no copy;
  - toggling disabled dims the card and changes the run body;
  - comment notes and sections can be created, edited, moved and deleted;
  - the filter chips and their counts;
  - empty states;
  - a read-only file;
  - **draft semantics:**
    - editing fields in the dialog leaves `scenarioStore.scenario`
      deep-equal and its revision unchanged, and fires no run (spy on
      `schedule`);
    - Done applies every field and calls `touch()` exactly once;
    - Cancel, Escape and the backdrop discard;
    - Done is disabled while the draft is invalid or the render refused;
  - `entryDraft.test.ts`: `draftOf(scenario, key)` deep-copies (mutating
    the draft never reaches the scenario), `commitDraft` replaces exactly
    the entry's lines, event, rule, layout item and attached comment, and
    the new-entry drafts (Add, Duplicate, alternative, Add a step change)
    insert at their recorded point;
  - **the "Writes:" preview:**
    - it shows `raw` before the first edit;
    - after an edit it calls the render API once per debounce window, with
      the draft (mocked API), and shows the returned text verbatim;
    - a 400 shows the sentence and disables Done;
    - a stale response (an older request resolving last) is ignored.
- **`scenarioModel.test.ts`:** delete the tests of the deleted functions,
  and keep the rest.
- **`web/e2e/projections.e2e.ts`:**
  - These rely on the table's selectors and labels and are rewritten
    against the list: :58 (seeds), :79 (assets empty), :97 (auto rule),
    :125 (asset row), :311 (counter greyed), :340 (edit marks dirty), :364
    (reload).
  - New `data-testid`s: `projection-list`, `projection-entry`
    (`data-kind`, `data-key`, `data-disabled`), `entry-edit`,
    `entry-dialog`, `entry-dialog-done`, `entry-dialog-cancel`,
    `projection-filter-*`, `projection-add-menu`.
  - Field labels in the dialog keep today's accessible names where they
    exist ("Amount for income:salary", "Counter-account for …", "Balance
    for …"), so most locators only gain a dialog scope.
  - New e2e: open the comments fixture copy, disable a rule, save, reload,
    and see it disabled.
  - Run Playwright on the host only. Agents never run it (memory:
    worktree-agents-branch-from-main; ledgeline-playwright-nix-drift).

## Phase C4 — Docs and screenshots (PR 5 or a follow-up)

- **`docs/projections/file-format.md`:**
  - rewrite "Descriptions and order": order is the list's, attached comments
    travel, and free notes and section headings are entries;
  - add "Comments and section headings" (the C4 rules, in user words);
  - add "Turning an entry off" (C7, with the `; ` example and what does and
    doesn't count as disabled);
  - add a "Duplicate as alternative" recipe (with or without a big
    customer);
  - say that comments inside a rule survive edits (C6).
- **`docs/projections/index.md`:** the editor section (the list, the dialog,
  keys).
- **`asset-rows.md` and `cash.md`:** update the UI references (rows → cards,
  columns → dialog fields).
- **Screenshots** (`web/docs-screenshots/projections.docs.ts`, `MANIFEST.md`):
  - re-shoot `seeded`, `counter-account`, `description` and `asset-rows`;
  - add `list.png` (sections, a disabled alternative, a step pair, an auto
    rule) and `editor.png` (the rule dialog with an asset posting);
  - `asset-scenario` is updated.
  - The selectors in `projections.docs.ts:13-116` are rewritten to the new
    test ids.
- **The mbr site:** no `[!INFO]` callouts, and run `docs/.mbr` locally
  (memory: ledgeline-docs-site-mbr).

## Mockups

### The list

```
 Projections ▸ Series A with hiring ramp           [file ▾] [Save] [Save as]  ● edited
 Default cash account [assets:checking        ]
 ┌──────────────────────────────────────────────────────────────────────────────────┐
 │ [All 14] [Income 2] [Expense 6] [Asset 2] [One-off 2] [Auto 2] [Off 1] [Notes 3]   │
 │                                                                      [+ Add ▾]    │
 ├──────────────────────────────────────────────────────────────────────────────────┤
 │ ▤ account equity:unrealized gains  ; type: E                 file text · read-only │
 │                                                                                    │
 │ OPERATIONS ──────────────────────────────────────────────────────────── section ⋯ │
 │                                                                                    │
 │⠿ INCOME EXPENSE  ~ monthly                         projection          [Edit] ⋯   │
 │   ; Every rule ends with its counter-account …                                     │
 │     revenues:salary                          $-12,000.00   ↗ 3%/yr                  │
 │     revenues:consulting                       $-2,500.00                           │
 │     expenses:rent                              $4,200.00   ↗ 2%/yr                  │
 │     expenses:software                            $900.00                           │
 │     → assets:checking                                                              │
 │⠿ EXPENSE  ~ monthly to 2027-04-01  projection         ⛓ step 1 of 2 · payroll  ⋯ │
 │ ┃   expenses:payroll                          $50,000.00                           │
 │ ┃   → assets:checking                                                              │
 │⠿ EXPENSE  ~ monthly from 2027-04-01  projection       ⛓ step 2 of 2 · payroll  ⋯ │
 │     expenses:payroll                         $120,000.00   ↗ 5%/yr                  │
 │     → assets:checking                                                              │
 │                                                                                    │
 │ ; Assumes no price rises before 2027.                                    note ⋯   │
 │                                                                                    │
 │ CAPITAL ─────────────────────────────────────────────────────────────── section ⋯ │
 │⠿ ONE-OFF  ~ 2027-03-01                        Series A                 [Edit] ⋯   │
 │     assets:checking                        $2,000,000.00                           │
 │     → equity:preferred                                                             │
 │⠿ OFF INCOME  ~ monthly from 2027-01-01   acme contract   (turned off)  [Edit] ⋯   │   dimmed,
 │   ; Big new customer — the "with" alternative                                      │   header
 │     revenues:acme                            $-40,000.00                           │   struck
 │     → assets:checking                                                              │
 │⠿ ASSET  ~ monthly                             investing                [Edit] ⋯   │
 │     assets:house      bal $650,000 (journal)       $0.00   ↗ 4%/yr  gains → equity:… │
 │     assets:savings    bal $41,200 (journal)    $2,000.00   ↗ 4%/yr                  │
 │     → assets:checking                                                              │
 │⠿ AUTO  = when revenue:sales is projected        sales commission       [Edit] ⋯   │
 │     expenses:people-costs:sales-commission     3% of it                            │
 │     → assets:mercury:checking                                ▸ show rule text       │
 └──────────────────────────────────────────────────────────────────────────────────┘
   ⠿ grip (drag)   ⋯ menu: Edit · Duplicate · Duplicate as alternative · Turn off ·
                          Add a step change… · Move up · Move down · Delete
```

### The editor dialog (recurring rule with an asset posting)

```
 ┌─ Edit recurring rule ─────────────────────────────────────────────── ✕ ─┐
 │ [✓] Enabled                                                              │
 │ Comment above this entry                                                 │
 │ ┌──────────────────────────────────────────────────────────────────────┐ │
 │ │ Savings plan agreed with Alex, 2026-09                               │ │
 │ └──────────────────────────────────────────────────────────────────────┘ │
 │ Repeats  [monthly ▾]   From [2027-01-01]   To [ mm/dd/yyyy ]             │
 │ Description [investing                                              ]    │
 │                                                                          │
 │ Postings                                              amount   growth     │
 │ ↕ [assets:house          ▾] (●) asset  [      0.00]  [4  ]%/[yr▾]  ✕    │
 │      Balance [ 700,000.00 ] ✕ use journal's ($650,000.00)                 │
 │      Gains account [equity:unrealized gains ▾]                            │
 │      Comment [appraised Sept 2026                         ]              │
 │ ↕ [assets:savings        ▾] (●) asset  [  2,000.00]  [4  ]%/[yr▾]  ✕    │
 │ [+ Add a posting]                                                         │
 │ Counter-account [assets:checking ▾]   (the posting with no amount)        │
 │ Header comment  [                                                ]       │
 │                                                                          │
 │ Writes (exact, from the server):                                         │
 │ ┌──────────────────────────────────────────────────────────────────────┐ │
 │ │ ; Savings plan agreed with Alex, 2026-09                             │ │
 │ │ ~ monthly from 2027-01-01  investing                                 │ │
 │ │     assets:house      $0  ; growth: 4%/yr, opening: 700000, gains: … │ │
 │ │     assets:savings $2000  ; growth: 4%/yr                            │ │
 │ │     assets:checking                                                  │ │
 │ └──────────────────────────────────────────────────────────────────────┘ │
 │ ───────────────────────────────────────────────────────────────────────── │
 │ Changes apply when you press Done.                                       │
 │                       [Move posting to a new rule]   [Cancel]  [Done ⌘↵]  │
 └──────────────────────────────────────────────────────────────────────────┘
```

The "Writes:" box shows the piece's own text until the first edit. After
that it shows `POST /api/projections/render`'s output for the draft (C15),
which comes from the same rendering functions a save uses. The readings
below the list do not change until Done (C3).

## Sequencing

1. **PR 1:** Phase A. **PR 2:** Phase B. They are independent and can go in
   parallel.
2. **PR 3:** C1, core only. It has no visible change, and every existing
   save takes the legacy-layout path.
3. **PR 4:** C2, the wire. The old table still works, and stops losing
   comments inside blocks.
4. **PR 5:** C3 (+ C4 docs). This is the UI switch-over.

Worktree agents branch from `main`. Create worktrees from HEAD and warm
`target/` with `/bin/cp -Rc` (memory: worktree-agents-branch-from-main).
Run `just pre-push` after each merge.

## Risks

- **False-positive disabled detection.** A prose comment that is a complete,
  parseable rule shows as "turned off". Mitigations: the strict C7 test
  (same prefix, at least one posting, parses alone to exactly one rule),
  and bytes are preserved unless edited. Accepted.
- **Scoping directives.** `alias`, `apply account`, `Y` and `decimal-mark`
  in a scenario file make the order matter beyond hledger's rule order.
  Mitigation: `confirm_round_trip` refuses the save with a sentence naming
  the directive. These are rare in projection files.
- **C6 tag interplay.** A user comment containing `name: value` is a tag to
  hledger. Writing Ledgeline's tags first and the user's text after keeps
  the user's tags as tags. Mitigation: a parity test with a user `owner:
  alex` tag beside `growth:`.
- **Volume of UI churn.** 1330 lines of table plus about 1270 lines of tests
  are replaced. Mitigation: the behaviour list above is ported test by test.
  e2e labels are kept wherever the field survives.
- **Render traffic.** The preview calls the server about once per 300ms of
  editing. A render is one block, with no file I/O and no projection, so it
  is cheap. Stale responses are dropped client-side.
- **Save As drops "file text" pieces,** as it does today. A copied
  `account x ; type: L` declaration would otherwise change classification
  in the copy. Mitigation: the Save As dialog lists the pieces it won't
  copy. Copying them is out of scope (`include` paths are relative).
- **The FFI and older clients.** These send no layout. `legacy_layout`
  keeps them byte-correct. Keep it until the FFI sends layouts.

## Resolved with Patrick (2026-10-29)

1. **Cash flow colours.** The statement's headings stay as they are (daisyUI
   `info` / `secondary` / `accent` for operating / investing / financing),
   and the chart matches them. The three theme colours are resolved live
   from `--color-*` and pinned as series colours. Exchange is `other`.
   Everything unpinned stays biggest-first (A1–A3, Phase A).
2. **The dialog edits a draft.** Done commits it, which gives one run.
   Cancel discards it. Cancel on a fresh duplicate leaves no copy, and
   Cancel on "Duplicate as alternative" leaves the original enabled (C3,
   C11).
3. **"Writes:" is exact, from the server,** via
   `POST /api/projections/render`, which uses the writer's own rendering
   (C15, built in Phase C2).
4. **New section headings use Patrick's hand style:** 22 semicolons, then
   `;; Title`, with no closing line. Existing headings keep their bytes and
   style (C4).

## Status

- Not started.

## Contract amendments made during implementation

(none yet)
