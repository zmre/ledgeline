# 18 — Account List Editor, and a Settings tab

Detects (or creates) the file that declares the chart of accounts and lets
each account's `type:`/tags/comment be edited from the GUI, under a new
top-level Settings surface that also picks up Account Aliases (moved out of
Imports). Driven by the TODO entry "Account List Editor".

## The ask

> "Most financial apps allow editing of the chart of accounts. We should
> detect where they live and allow editing. If there aren't any, we should
> create an accounts.journal and include it from the main file. For each
> account, we should provide an editor for comments/notes, type, tags in
> general, and our special tags used in various reports. Let's put this under
> a Settings top level tab or gear icon […] we should probably move aliases to
> here under 'settings' too."

## Decisions (locked with Patrick 2026-09-04)

1. **This pass covers the account-directive editor, the Settings route, and
   moving the Aliases UI in.** Editing `commodity`/`decimal-mark`/`D` (no
   write path exists today, and rewriting one after amounts have already
   parsed under it can silently reinterpret them — the hazard the removed
   `restyle.rs` feature hit) and a dedicated tag-glossary/help UI for the
   report-consumed tags are deferred to a follow-up plan. Both stay
   read-only/parsed-only, exactly as before this change.
2. **Every account name the journal knows is listed, declared or not.** The
   merge is the same list `AccountInput`/`AccountTreeSelect` already read
   (`journal.accountNames`, from postings) joined against the engine's
   declared set. Selecting an undeclared name and saving sends a `declare`
   edit rather than requiring a separate "add account" flow.
3. **A gear icon in the nav bar, not a text tab.** Settings is configuration,
   not a data view, and the corner (beside the connection status and the
   refresh button) is where this app already puts icon-only controls.
4. **Aliases move by nav entry, not by file.** `web/src/lib/imports/` still
   owns `aliasModel.ts` / `aliasStore.svelte.ts` / `AliasPanel.svelte` /
   `AliasEffectPanel.svelte` unchanged — the last of those is also used by the
   import dry-run preview, so relocating the library code would be a larger,
   riskier diff than the ask, which is about where the tab lives.

## Why `accounts.rs` needs no lock, unlike `aliases.rs`

`aliases.rs` states the discipline every editable-directive module in this
codebase follows:

> An edit rewrites bytes only inside the spans it names, and every other byte
> of the file comes out the `&str` slice it went in as.

`aliases.rs` needs per-line locks (`AliasLock`) because an edit there only
splices the pattern/replacement *inside* otherwise-preserved formatting, so a
line whose existing shape is ambiguous (a stray `;`, an unescaped `/`) cannot
be touched at all without guessing what its author meant.

`account NAME  ; tags…`'s comment is different: the model
(`AccountDeclaration.tags`) is already *derived* from the raw comment by
`parse::parse_tags` classifying each comma segment as a tag or not, so an edit
can simply replace the **entire** comment — from the separator that ends the
name to the end of the line — with this module's own deterministic rendering
of `{tags, note}` (every tag as `key: value`, comma-joined, then the note as
the trailing segment). There is no ambiguous existing shape to preserve, only
a candidate new string to validate — so every declared account is editable,
and the risk moves from "can this line be read safely" to "would the rendered
replacement read back as something other than what was asked", which is a
write-time check (`check_tags`, `check_note`), not a per-line lock.
`check_note` in particular rejects a note whose own comma segments would
themselves classify as a tag (`"ping bob, re: taxes"` → `re: taxes` reads back
as a tag named `re`) — the exact predicate `parse::tag_in_segment` applies to
a real comment, refactored out of `parse_tags` so there is one definition of
"is this a tag", not two that can silently disagree (`parse_account_directive`'s
own doc comment already records one instance of that exact class of bug, for
the account/comment split itself).

`AccountDoc::verify` still re-renders, byte-compares, and re-parses the
result to confirm every unedited line is byte-identical and every edited one
reads back as exactly what was asked — the same four-step discipline
`AliasDoc::verify` uses.

## The one model change: `AccountDeclaration.source_file`

Added alongside `Transaction::source_file` / `AliasDirective::source_file`
(`model.rs`), populated at parse time the same way `alias`'s is
(`parse_account_directive` now takes the resolved `source_file`). Without it
there was no way to know which `include`d file a declaration lives in, which
"detect where they live" and "write the edit back" both need.

## Declaring a new account

`AccountEdit::Declare` is `AliasEdit::Append`'s analogue: inserted immediately
after the file's last `account` line, or at EOF when it has none. Unlike an
alias, an `account` directive's order never changes what it means (no
scoping, no first-match cascade), so a batch of declares needs no specificity
re-ordering — they land in the order the client sent them.

## File discovery and creation

`account_api::account_files` mirrors `budget_api::budget_lines`'s "worth
listing" rule, not `alias_api::alias_files`'s narrower "declaring, or root" —
because, like budget (and unlike aliases), this module has a create-file
flow, and a freshly created `accounts.journal` declares nothing yet:

- when some file already declares an account, list exactly those files;
- when none do, list every writable file the parse read that holds no
  transactions (a pure directive file — a freshly created `accounts.journal`
  is exactly this shape), so the file `POST /api/accounts/file` just created
  is immediately usable;
- failing both, fall back to the root journal.

`account_api::create_accounts_file` is `budget_api::create_budget_file`
line-for-line: the new file is written FIRST, the `include` line SECOND (an
`include` naming a file that is not there is a journal that does not parse,
so a failed second write leaves only an orphaned file, never a broken
journal), both proved by a whole-journal re-parse before either lands. It
also refuses when the journal already declares an account anywhere — the same
"do not split an existing home across two files" refusal `can_create` makes
for budget rules.

## Where the code is

| Path | What it holds |
| --- | --- |
| `crates/ledgeline-core/src/accounts.rs` | The format-preserving document model: `AccountDoc`, `AccountLine`, `AccountEdit`/`AccountPlan`, `apply`/`verify` |
| `crates/ledgeline-core/src/parse.rs` | `AccountDeclaration.source_file`; `split_account_name` now returns the comment's start offset too; `tag_in_segment` factored out of `parse_tags` |
| `crates/ledgeline-server/src/account_api.rs` | `/api/accounts`, `/api/accounts/{*id}`, `/api/accounts/file` |
| `crates/ledgeline-server/tests/account_endpoints.rs` | The HTTP surface: written bytes, 409, the create-file refusals, the token guard, no absolute paths |
| `web/src/lib/accounts/` | Pure form model (`model.ts`), store (`accountsStore.svelte.ts`), UI (`ui/AccountsPanel.svelte`) |
| `web/src/lib/settings/` | The Settings subnav (`params.ts`, `ui/SettingsTabs.svelte`) |
| `web/src/routes/settings/+page.svelte` | The route: Accounts and Account Aliases tabs |
| `web/src/routes/+layout.svelte` | The gear icon |
| `web/src/lib/imports/params.ts`, `web/src/routes/imports/+page.svelte` | Aliases tab removed; `?tab=aliases` forwards to `/settings?tab=aliases` |

## Testing

| Level | Covers |
| --- | --- |
| `accounts.rs` unit tests | scan spans, an isolated edit leaves every other byte alone, clearing tags+note leaves a bare name, declare's insertion point (after the last line / at EOF / missing terminator), the note-reads-as-a-tag refusal, CRLF, a comment block hiding a declaration, tag/note classification matching the parser's own |
| `tests/account_endpoints.rs` | written bytes, a stale revision, create-then-declare against the new file, an existing `accounts.journal` never overwritten, the token guard on all three routes, no absolute paths |
| `web/src/lib/accounts/model.test.ts` | the merge, the type/tags/note split, dirty-checking, the edit builder, the default target file |
| `web/src/lib/accounts/ui/AccountsPanel.svelte.test.ts` | mounts without a self-feeding effect (the same latch shape `AliasPanel` once got wrong), lists declared and undeclared accounts, seeds the editor, keeps mid-edit typing across a re-render |
| `web/src/lib/imports/params.test.ts`, `web/src/lib/settings/params.test.ts` | the tab codec on both sides of the move, and `aliasesRedirect` |
| `web/e2e/settings.e2e.ts` | the tab is reachable and shows a real declaration's type; the old `/imports?tab=aliases` URL forwards. Writes nothing — same reasoning `budget.e2e.ts` gives: `fixtures/sample.journal`'s 36 declarations are asserted on by other specs, so the write path is covered by the Rust endpoint test instead. **Unverified in this session** — Chromium cannot launch in this sandbox (see `vite.config.ts`); needs a real run before merge |

## Deferred (see Decisions §1)

- Editing `commodity` / `decimal-mark` / `D` directives.

## Follow-up (2026-09-04, same day): add/remove, special tags, list layout

Four requests against the first pass, none touching the deferred item above.

1. **`AccountEdit::Delete`, added to `accounts.rs`.** Removes one declaration
   line, terminator and all — `AliasEdit::Delete`'s analogue, over
   `AccountLine::full` (renamed from the `full_end: usize` the insertion point
   alone had needed). `verify` gained the same `deleted`/`survivors` split
   `AliasDoc::verify` already uses, since a plan can now shrink the line count
   as well as grow it. Wired through `WireAccountEdit::Delete` and
   `SaveAccountEdit`'s `"delete"` variant. Deliberately does not, and cannot,
   touch a posting that still names the account — removing the declaration is
   metadata-only.
2. **Every report-consumed tag now behaves like `type:`.** `model.ts`'s
   `SPECIAL_TAGS` registry (restating, by hand, the exact accepted vocabulary
   of `account_types.rs::parse_account_type_tag`,
   `income_statement.rs::parse_is_section_tag`,
   `account_groups.rs::parse_bs_term_tag`, and
   `holdings/classify.rs::parse_{holdings,valuation}_tag`) gives `type`,
   `issection`, `bsterm`, `holdings` and `valuation` a dropdown with a
   `(none)` default, and `bsgroup`/`isgroup` (free labels, not a closed
   vocabulary) a text field with suggestions instead. Each field carries
   inline help (a small "ⓘ" `tooltip`, `AccountsPanel.svelte`'s `help`
   snippet). An existing value that matches none of a closed tag's options is
   kept as an ordinary tag row rather than discarded — the dropdown reads
   `(none)`, but a save that touches nothing else still preserves it. A
   generic tag row named after a special tag is refused by
   `validateForm` ("use the … field above instead") rather than silently
   producing two tags of the same name.
3. **Add and remove, in the panel.** "Add account" (a name field + button)
   opens the same undeclared-account editor a click on an unknown row would —
   there was already a `Declare` path, just no way to reach it for a name
   `journal.accountNames` had never heard of. "Remove account" stages a
   deletion (dims the form, shows a warning) rather than deleting immediately
   on click, matching `AliasPanel`'s own row-level Delete/Keep toggle instead
   of introducing a native `confirm()` this codebase uses nowhere else;
   committing it is the same Save button as everything else.
4. **List column: wider, no tag-count badge, real abbreviation.** `md:w-96`
   (was `w-72`); dropped the "N tags" badge (kept "undeclared"); each row now
   renders through `AccountLabel.svelte` (`fitAccount`/`abbreviateAccount`
   from `domain/accounts.ts`) instead of a plain `truncate` span, so a long
   name shortens its ANCESTOR segments (`expenses:household:repairs:plumbing`
   → `exp:household:repairs:plumbing`) rather than losing its right (most
   identifying) end to an ellipsis.
5. **`AliasPanel`'s default file, fixed.** The seeding effect's fallback chain
   was `files.find(selectedId) ?? files[0]` — on a journal whose aliases live
   in an `include`d file, `files[0]` (main.journal, root, always offered) has
   none, so the tab opened looking empty. Now:
   `files.find(selectedId) ?? files.find(file => file.aliases.length > 0) ?? files[0]`.
   main.journal is still shown (and still the fallback when nothing anywhere
   declares an alias yet) — it just doesn't win by sort position alone.

Tests added: `accounts.rs` (delete removes exactly one line; a declare can
still land in the same batch as a delete of the file's last line; the same
index cannot be named by both a replace and a delete);
`account_endpoints.rs::deleting_a_declaration_removes_exactly_one_line`;
`model.test.ts` (the `SPECIAL_TAGS` registry's own shape, the closed/free
split, the unrecognised-value-preserved case, `validateName`,
`deleteSaveRequest`); `AccountsPanel.svelte.test.ts` (every special tag
renders with its own label and help button, add-a-new-name, stage-then-untage
a removal, the special-tag-collision refusal); `AliasPanel.svelte.test.ts`
(a two-file listing where only the second has aliases seeds from the second).
`web/e2e/settings.e2e.ts` needed no change — its one assertion on the `Type`
field checks the `<select>`'s VALUE ("C"), not an option's display label, and
that vocabulary is unchanged. Still unverified in this session for the same
Chromium-sandbox reason as the first pass.

## Follow-up (2026-09-07): the list+editor layout was flex, and it showed

Opening the editor made the account list collapse to near-nothing, the
special-tag fields left a large gap on the right instead of filling the row,
and the "Remove account" button read as its own column, floating far from the
account name. All three traced to one cause: the list+editor split was plain
`flex` (`flex flex-col gap-2 md:flex-row md:gap-4` around a `<ul>` sized only
by `md:w-96` and an editor `<div>` sized only by `grow`). Neither side had a
flex-basis or a `min-width` floor, so the browser's shrink/grow arithmetic was
free to reallocate width however the (wide, multi-field) editor's content
happened to demand it — starving the list.

Fixed by switching to the same sidebar+content **grid** shape
`EditRulesPanel.svelte:317` already uses for its own rules-file list+editor
split: `grid grid-cols-1 gap-3 lg:grid-cols-[20rem_minmax(0,1fr)] lg:items-start`.
A `minmax(0, 1fr)` track has an explicit zero floor, so the editor column can
never expand into the list's fixed `20rem` track the way an un-floored flex
item could. The special-tag fields moved from a `flex-wrap` row (whose
*max-content* width is the SUM of every field's `max-w-xs` cap, which is what
left the unfilled gap once the row could no longer claim that much space) to
`grid grid-cols-1 gap-3 sm:grid-cols-2 xl:grid-cols-3` with `w-full` on each
`<select>`/`<input>` — the same 2-column form-grid `PreferencesPanel.svelte`
already uses, just carried to 3 columns at `xl` since there are seven fields.
`xl` rather than `lg` so the two breakpoint changes (list becoming a fixed
column, fields becoming 3-wide) don't both land at once and squeeze each
other. The "Remove account" button lost its `ml-auto`, which had been the
literal cause of the "own column" look — pinning it to the far right of
whatever width the (previously miscalculated) editor column happened to have.
It now just sits beside the name.

**Independent scroll**: the account list also needed to scroll on its own, so
picking a name far down a long list doesn't require the edit form to be
off-screen. Surveyed the codebase for precedent first — `ReportTable.svelte`,
`DryRunPanel.svelte`, and `AccountTreeSelect.svelte` all cap a scrollable
region with a self-contained `max-h-[…] overflow-y-auto` (no `sticky`, no
viewport-height chain to an ancestor); nothing in the app uses `position:
sticky` for a sidebar-stays-put pattern. Adopted the same self-contained
shape — `max-h-[70vh] overflow-y-auto` on the list `<ul>`, the identical
`70vh` value `ReportTable.svelte` uses — rather than inventing a new one. No
`+layout.svelte` change needed: at the `lg` breakpoint the list and editor are
already side-by-side grid columns, not stacked, so the editor was never at
risk of scrolling off screen in the first place; the cap mainly helps the
single-column (narrow-viewport) case, where a long list no longer pushes the
editor an arbitrary distance down the page.

## Follow-up (2026-09-07, same day): the height cap turned the list into columns

The `max-h-[70vh] overflow-y-auto` above did not produce a scrollbar — it
produced a list that filled ~70% of the viewport height in its first "column"
and then started a SECOND column to the right, overflowing the viewport
edge. Cause: `<ul class="menu …">` — daisyUI's `.menu` component class sets
`flex-flow: column wrap` (confirmed in `node_modules/daisyui/daisyui.css`).
`ReportTable.svelte`/`DryRunPanel.svelte`/`AccountTreeSelect.svelte`, the
precedents the `max-h`/`overflow-y-auto` pairing was borrowed from, wrap a
plain `<table>` or a bare `<ul>` with no `menu` class and therefore no
column-wrapping behavior to fight; this is the one list in the app that
needed the `menu`/`menu-active` styling AND a height cap together, and
nothing else hit the combination before. Fixed by adding `flex-nowrap` (and
an explicit, self-documenting `flex-col`) to the same element, overriding
just the wrap half of daisyUI's `flex-flow` shorthand and leaving its
`flex-direction: column` intact — so height overflow now does what
`overflow-y-auto` actually promises: a vertical scrollbar, one column, no
horizontal growth.
