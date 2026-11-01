# 32 — Transaction review: rip through unknowns and recents

A one-transaction-at-a-time **review overlay**. It opens from two places:

- **The Journal page.** **Review Unknowns** and **Review Recents** sit next to
  **Add Transaction**.
- **The Import flow** (CSV and QuickBooks), as an optional last step before
  **Write changes**.
  There the buttons are **Review Unknowns** and **Review All**.

Each card puts *who* and *how much* front and centre. You can change the
description, tags, comment, amounts and accounts. **Reviewed** stamps a
`reviewed:YYYY-MM-DD` tag. A rename or recategorisation offers to apply the
same change to similar unreviewed transactions. During import it can also save
the change as a rule, so the next import already knows. Everything works from
the keyboard, and every field autocompletes.

Shipped in six phases (engine → wire → journal UI → import → docs, and Phase 6
after user testing). The plan follows the order of plan 31.

## The ask (Patrick, 2026-10-30)

> Make categorizing and describing unknowns faster and easier.
>
> 1. On the journal page, two new buttons next to "Add Transaction": "Review
>    Unknowns" and "Review Recents".
> 2. The same two buttons as a final optional step of the import process
>    before writing the journal, but "Review Recents" becomes "Review All".
>
> One transaction at a time, as beautiful and readable as we can make it. Who
> and how much front and center. An overlay. Footer: bottom left "✓ Reviewed"
> (green), bottom right "Skip →" (gray). If an unknown account is listed and
> not fixed, only Skip is enabled. Reviewed adds a tag like
> `reviewed:2026-10-15`. Also support a never-review-again marker such as
> `reviewed:ignore`, applied manually.
>
> Review Recents grabs N unreviewed items by date (default 50, a setting), going
> back at most M days (default 365, a setting). Disable the buttons when there
> is nothing to review.
>
> The user can change description, tags, comments, amounts and accounts. If the
> description changes, look in the background for similar descriptions (ignoring
> numbers) among previous unreviewed items, and show a section listing them with
> checkboxes (and a check-all header), all unchecked by default, collapsed, with
> a count. On save, the current entry and the checked ones are saved. The others
> are NOT marked reviewed. The same applies to an account change
> (`expenses:unknown` → `expenses:shopping`).
>
> Once anything changes, the footer becomes "Mark Reviewed and Save →" (left)
> and "✕ Revert Changes" (right).
>
> If a single account is split (amount changed and/or postings added), don't
> search or apply anything for now. Maybe later, and only for identical
> amounts.
>
> During import, offer to make a rule that always does this categorization or
> rename (or both), and save it to the rules file. Ideally from the Journal view
> too, but we don't know which rules file was used. Ideas welcome that don't
> involve tagging entries with their source.
>
> It must be smoking fast and keyboard-driven: Tab between fields,
> autocompletion on everything, shortcuts for every button and for jumping
> around, and shortcuts on the Journal and Import pages to launch a review.
>
> Think through other implications, such as a subtle indicator in the journal
> list and editor (e.g. a left border for unreviewed, another colour for
> unknown). Good tests, docs with screenshots.

Decisions that need Patrick are marked **(Q)** and collected under
[Questions for Patrick](#questions-for-patrick). Every **(Q)** has a default,
so the work can start without waiting for an answer.

## What is true today

### Journal page

- **Route.** `web/src/routes/+page.svelte` holds `FilterBar`, `TransactionTable`
  and one page-level `<TransactionModal />` (:144).
  - **Add Transaction** lives in the toolbar strip of
    `journal/TransactionTable.svelte:271-277`, shown only when
    `editing.canEdit`. The new buttons go here.
- **The list is windowed by hand.** Rows are 40px (104px as cards).
  `rowModel.ts:160 computeWindow` reads 200-row blocks from
  `GET /api/journal/rows`, holds at most 50 blocks, and is built for 200k
  transactions. **The client never holds the whole journal**, so every "find
  similar" or "find unreviewed" query has to run in the engine.
- **Row markers.** The cursor is `bg-primary/25` plus a 3px inset left rule,
  drawn as a `box-shadow` on the first `<td>`. A real border won't work because
  daisyUI tables use `border-collapse:separate` (`TransactionRow.svelte:37-48`).
  Problems show as a severity dot before the description (:97-101). There is no
  reviewed or unknown marker.
- **Keys.** One global keymap (`keys/keymap.svelte.ts:58 registerKeys`), with
  priorities page 0 / widget 10 / transient 20 / overlay 100. A `modal: true`
  layer blocks everything below it.
  - The journal binds `j k ↑ ↓ ctrl+d/u PgUp/PgDn gg G Esc Enter e c s x n`.
  - **`r` and `u` are free.**
  - `KeyGroup` is a closed union (`keys/types.ts:17-19`).
  - **Nothing registers an `overlay` layer today**, including
    `TransactionModal`. With focus on a modal *button*, the journal's j/k/x can
    still fire.

### Editing and writing

- **Editor.** `journal/edit/TransactionModal.svelte` uses `editMapping.ts`
  (`txnToForm` / `formToBody` / `validateForm`).
  - Comment and tags are one raw text input.
  - **Only accounts autocomplete** (`edit/AccountInput.svelte`, ranked by
    `edit/accountMatch.ts:105`). Payees, descriptions and tags have none.
- **Endpoints** (`edit_api.rs`, `lib.rs:1206-1213`):
  - `POST /api/transactions`.
  - `GET` / `PUT` / `PATCH` / `DELETE` on `/api/transactions/{index}`.
  - **PATCH can set description, status and posting accounts, but not
    comments or tags.** Adding a tag today means a PUT, and a PUT re-renders the
    whole transaction (`edit.rs:1524 replace_transaction` →
    `format_transaction` :2369).
  - That PUT also triggers the accepted elision bug (memory: wire
    `poriginal: None`): an auto-balanced amount gets written out explicitly.
    **Review would trigger that on almost every transaction it touches**, so
    Reviewed must not go through PUT.
- **Surgical ops** exist for description (`edit.rs:1311`), status (:1375) and
  posting account (:1449). They rewrite only their own line.
- **Every op re-parses the whole journal** (`validate_with` :662). There is no
  HTTP batch route.
  - `add_and_replace_transactions` (`edit_api.rs:849`) is an internal helper:
    one lock, one save, but still one re-parse per replacement.
- **Locating a transaction.** `Tindex` is its file-order position. It stays
  stable across replace and patch, and shifts on add and delete.
  `edit.rs:190` fingerprints guard against external changes.
- **Tags** exist only inside comment text.
  - The parser is `parse.rs:3546 tag_in_segment` / `:3558 parse_tags`, hledger
    semantics: a tag's name is the word before `:`, and its value runs to the
    next `,`.
  - There is **no tag serializer**.

### Unknown accounts

- `view/checks.rs:137 is_uncategorized` flags any account whose last segment
  is `unknown` or `uncategorized`, plus a bare top-level expense or revenue
  account. That is the definition of "unknown" this plan reuses.
- `rules/matching.rs:895` uses a narrower `ends_with(":unknown")`.
- `rules/generate.rs:110` drafts rules with `expenses:unknown`.

### Import

- **Steps.** Drop → Staged (preview, ranked rules candidates, destination,
  balance) → **Test Import** → `DryRunPanel` (`POST /api/import/dry-run`)
  → **Write changes** (`POST /api/import/commit`) → Result.
- **The dry run is text.** `DryRunOk.entries` is hledger's stdout, shown in a
  `<pre>`. No per-transaction struct exists, and nothing links back to a CSV row.
- **The commit regenerates the entries on the server**
  (`import_api.rs:3907`). "The preview is the bytes"
  (`imports-internals.md:262-281`). Any edit the client makes to the preview
  is thrown away unless the server applies it.
- **Which rules file was used** is known only in client state
  (`selectedRulesId`) during the import. Nothing is persisted, and the rules
  file is not in the import's git commit.
  - Candidates are ranked fresh on every stage (`rank_candidates`
    `import_api.rs:2592`).
- **Rules can already be written programmatically.** Call `GET /api/rules/{id}`
  to get the revision, then `PUT` with `keep` items plus a new `ifBlock`
  (`rules_api.rs:1030-1113`). Client side, `model.ts appendRule` /
  `rulesStore.save` already does it.
  - **Matchers run against raw CSV fields.** The previewed description is
    post-rules.
- `TODO.md:129` already lists "remember categorization → write an `if` rule",
  and `imports-internals.md:21` deferred it on purpose.

### Settings

- **Per-browser:** `stores/settings.svelte.ts` (localStorage, versioned). No
  numeric settings yet, and no settings page; values are toggled where they're
  used.
- **Server:** `prefs.json` (`prefs.rs:88`). PUT replaces the whole object, and
  `WirePrefs` uses `deny_unknown_fields`.
- **The Settings page** edits journal files (accounts, aliases, hledger.conf,
  automatic postings), not app preferences.

### Tests, docs, conventions

- **Rust tests.**
  - Core: `crates/ledgeline-core/tests/*.rs`.
  - Server: `*_endpoints.rs`, which drive the router via `tower::oneshot` over
    a temp-copy journal and assert the response, the GET snapshot and the bytes
    on disk.
- **Vitest.** `*.test.ts` (node) and `*.svelte.test.ts` (jsdom), using
  `testing/fakeEngine.ts`.
  - Live tests: `just test-integration`.
- **Playwright e2e** runs on `fixtures/sample.journal`, which has **one**
  unknown transaction (2026-06-20 Venmo, a deliberate problem record).
  - e2e asserts exact totals, so new review fixtures must not go into
    `sample.journal`.
- **Docs shots.** `web/docs-screenshots/*.docs.ts`, with staged sets on their
  own ports (`stage.ts`), a `harness.ts` `shot()`, and a `MANIFEST.md` row per
  image. No spec presses Save.
- **Modals.** daisyUI `div.modal` plus `use:dismissible` (focus trap, Escape,
  focus restore).
- **Icons** are inline 16×16 SVGs; there is no icon library.
- **`btn-success` is unused anywhere.**

## Decisions

### Review state lives in one tag

**D1. The `reviewed` tag on the transaction header.**

| Value          | Meaning                                                           |
| -------------- | ----------------------------------------------------------------- |
| *(no tag)*     | Unreviewed                                                        |
| `YYYY-MM-DD`   | Reviewed on that day (local date)                                 |
| `ignore`       | Never ask. Hidden from both queues and from the row markers       |
| anything else  | Treated as reviewed, so a hand-typed `reviewed:yes` works         |

- Only the **transaction** comment counts. A `reviewed:` on a posting comment
  is ignored. That keeps the rule easy to explain, and hledger users can run
  `hledger reg tag:reviewed` and `not:tag:reviewed`. The docs show both.
- `reviewed:ignore` can also be set from the overlay: **Ignore** (key `i`),
  tucked in a "⋯" menu rather than the footer so it doesn't get pressed by
  accident. Patrick said "manually"; this is a cheap extra. **(Q1)**

**D2. Writing the tag is a new surgical core op, `set_txn_tag(name, value)`.**
It mirrors `set_description`: it rewrites the header line's comment only and
never re-renders the transaction. That avoids reformatting and the elision bug.

Before → after `set_txn_tag("reviewed", "2026-10-30")`. The examples are a code
block because inline code collapses the double space:

```
2026-10-12 Safeway
2026-10-12 Safeway  ; reviewed:2026-10-30

2026-10-12 Safeway  ; groceries
2026-10-12 Safeway  ; groceries, reviewed:2026-10-30

2026-10-12 Safeway  ; reviewed:2026-01-02, x:1
2026-10-12 Safeway  ; reviewed:2026-10-30, x:1
```

- **Two spaces, then `; `.** A new same-line comment is always separated from
  the description by exactly two spaces (`"  ; "`), the house format already
  used by `edit.rs:2505 push_comment`. An existing comment's spacing is left
  exactly as found. A test pins both.
- The same op with `None` removes the tag, together with its separator.
- Tags on continuation lines are found and replaced where they sit.

### What "unknown" and "unreviewed" mean

**D3. Unknown.** A transaction with any posting that `checks.rs
is_uncategorized` flags. It's the same predicate as the Problems drawer, so
the two counts always agree. **(P, 2026-10-30)**

- **Postings added by `=` auto rules to real journal transactions count, in
  review and in the drawer.**
  - A generated posting can't be edited on the card. The card marks it "from
    automatic posting rule `= …`" and links to Settings → Automatic postings.
  - **Reviewed** stays disabled while that posting is unknown, so the fix is
    to edit the rule.
- **Forecast transactions from `~` periodic rules, including any auto postings
  applied to them, are ignored everywhere** (review and the drawer).

**D4. The two queues.** Both are computed in the engine from the current
snapshot, and both share **one lookback window**: dated within the last
`maxDays` (default 365) up to today. Future-dated transactions are excluded
from both. Nobody gets asked what a three-year-old charge was unless they raise
`maxDays`. **(P)**

- **Unknowns.** Every unknown transaction in the window except
  `reviewed:ignore`, newest first, with no count limit (paged at 100).
  - A `reviewed:<date>` transaction that is still unknown (a hand edit) is
    included. It's still uncategorised. **(P)**
- **Recents.** Transactions in the window with **no `reviewed` tag at all**,
  newest first, capped at `limit` (default 50). An unknown with no tag is
  included. An unknown already tagged reviewed is **not** included. **(P)**
  **(Q2: newest first, or oldest first within the window?)**
- **Not in either queue:**
  - periodic and auto-posting rule definitions;
  - forecast transactions;
  - transactions in files the editor can't write (read-only includes), which
    are counted and reported as "N not editable".
- **Respect the filter? No.** The queues ignore the Journal filter. **(Q3)**
  The tempting alternative ("review what I'm looking at") is easy to add later
  as `?q=`.

**D5. Skip is session-only.** A skipped card leaves this run's queue and comes
back next time. **←** returns to the previous card, so a mis-press costs
nothing.

**D5a. The stack changes as you work. (P)** A save that applies a change to
similar transactions that are also in the queue updates the queue at once,
from the batch result. It doesn't wait for a refetch.

- **Unknowns.** A queued transaction that the save made categorised is removed
  (it no longer meets D3). For example: 10 unknowns, the first card
  recategorises and applies to the 9 related ones, and the run ends right there
  on the summary: "1 reviewed · 9 fixed via similar".
- **Recents.** A queued transaction the save touched stays in the queue,
  because it's still unreviewed. Its card now shows the applied change and an
  "updated from card 1" note, so it's a one-key **Reviewed**.
- **Header.** The position and total recompute ("3 of 41" becomes "3 of 32"),
  and the progress bar animates to the new fraction (D29).
- **After each save** the queue is re-validated against the new generation in
  the background (another tab or an external edit may have changed things).
  The result is merged keeping the run's order, its skips, and the current
  card. A queued transaction that disappeared or changed underneath is dropped
  or refreshed rather than shown stale.

### The card

**D6. Layout** (mockup below).

- **Header.** Position ("12 of 50"), a thin progress bar, the queue name, the
  ⋯ menu (Ignore, Open in full editor, Settings) and ✕.
- **Hero.** The payee in large type (description before `|`, with the note
  after `|` in muted text), the amount in large tabular figures and the date.
  Colours:

  | Amount                        | Colour  |
  | ----------------------------- | ------- |
  | Expense-side (money out)      | neutral |
  | Income-side (money in)        | success |
  | Pure transfer between assets  | info    |

  Under the hero: the funding account ("from **Chase Checking**", using the
  account's declared display name if it has one) and the status.
- **Fields.** Description, then postings (account and amount per row, an
  unknown account outlined in warning colour), then tags as chips with an input,
  then the comment. Each field's label shows its shortcut letter.
- **Similar section** (D10), collapsed, with its count in the header.
- **Footer** (D7).

**D7. Footer states.**

| State                                          | Left                                    | Right                    |
| ---------------------------------------------- | --------------------------------------- | ------------------------ |
| Clean, no unknown                              | `✓ Reviewed` (btn-success)              | `Skip →` (btn-ghost)     |
| Clean, unknown present                         | `✓ Reviewed` **disabled**, tooltip why  | `Skip →`                 |
| Dirty, no unknown                              | `Mark Reviewed and Save →` (btn-success)| `✕ Revert Changes`       |
| Dirty, unknown still present                   | `Save →` (btn-primary, *not* reviewed)  | `✕ Revert Changes`       |

The last row is my addition. Renaming a transaction without knowing its
category yet is a real case: you fix the payee name now so that similar-match
finds its siblings, and come back to the category later. The default is to
allow saving without marking reviewed. **(Q4)**

Saving advances to the next card. It is optimistic: the next card renders
immediately while the write completes (D14).

**D8. Edits allowed.** Description, tags, comment, posting accounts and
amounts, adding and removing postings, and the date (behind the ⋯ menu, since
it's rarely needed). Cost annotations, balance assertions and posting types
are carried through and shown read-only. **Open in full editor** hands off to
`TransactionModal` for anything else.

**D9. How a save writes.**

- **Description, accounts and tags only:** surgical ops in one batch
  (`set_description`, `set_posting_account`, `set_txn_tag`). The text is
  otherwise untouched and elision is preserved.
- **Amounts or postings changed:** `replace_transaction`, which reformats that
  one transaction. That's the existing behaviour, accepted. The tag goes in the
  same batch.
- **Comment changed** (free text, not tags): a new surgical op,
  `set_txn_comment`, which rewrites the header comment while keeping its tags.
  It is the same machinery as D2.

### Similar transactions

**D10. Similarity key.** A pure `similarity_key(description)` in core:

1. Take the payee part (before `|`).
2. Lowercase it and apply Unicode NFKC.
3. Drop every token that contains a digit (`#4412`, `POS0923`, `10/12`,
   `X1234`).
4. Replace punctuation with spaces and collapse whitespace.
5. If nothing is left (e.g. "12345"), the transaction has no key and matches
   nothing.

So `SQ *BLUE BOTTLE 4412 OAKLAND` and `SQ *BLUE BOTTLE 0917 OAKLAND` both key to
`sq blue bottle oakland`. Exact key equality, no fuzzy matching. Fuzzy matching
over a journal invites false positives into a bulk edit, and exact keys can be
indexed. Proptests pin idempotence, digit-insensitivity and case-insensitivity.

**D11. What "similar" returns.** The current transaction's key is computed from
its **original** (pre-edit) description. The engine returns:

- **Candidates:** unreviewed transactions (not `ignore`, not the current one)
  with the same key, at any date, newest first, capped at 500. Any that are also
  in the current queue are marked, so applying to them makes their later cards
  arrive already fixed.
- **Precedent:** among *categorised* transactions with that key (reviewed or
  not), the most common counter-account and how many times it was used. This
  drives the suggestion chip (D13).

What gets applied to each checked candidate:

- **Description changed:** the candidate's description is set to the new one.
- **One account changed from A to B:** every posting in the candidate on
  account A moves to B. A candidate without account A shows the rename only, or
  is hidden if the rename isn't happening.
- **Both changed:** both, per row. Each row shows chips for exactly what it
  will receive.
- **Amounts changed, or postings added or removed (a split):** no section at
  all, as Patrick asked. A note reads: "Splits aren't applied to similar
  transactions."

Applied candidates are **not** marked reviewed, as Patrick asked. Their writes
are surgical (D9), so they cost nothing in fidelity.

**D12. When the query runs.** Similar is fetched **once, as soon as a card is
shown**, keyed on the original description. Every later edit filters that
result client-side, so the section appears the instant you change something
and no request is made per keystroke. The next card's queue entry and its
similar set are prefetched while you look at this one.

**D13. Suggestions, and accepting one in a single keystroke. (P, Q5
answered: yes.)** On an unknown card, the unknown posting row shows the best
suggestion as a chip:

> `⌘⇧↵ expenses:food:coffee` — 14 of 15 times for this payee

- **⌘⇧↵** (Ctrl+Shift+Enter elsewhere, or **Y** in card mode) does three things
  at once: it accepts the top suggestion, marks the card reviewed and saves,
  and advances. Most unknowns become **one keystroke**.
  - The keys work from any mode, including inside a field.
  - With no suggestion on the card, ⌘⇧↵ is plain ⌘↵.
  - Applying to similar transactions still needs the section opened and boxes
    checked. ⌘⇧↵ never bulk-applies anything on its own.
- **Tab** on the unknown account field fills in the selected suggestion
  without saving (pressing it again cycles, see below).
  You can then adjust and press ⌘↵. Accepting counts as an account change, so
  the similar section offers the rest.
- **Alternatives.** Up to two runner-up chips are shown. **Tab** in the
  account field cycles the selected chip (top → alt 1 → alt 2 → top), and
  ⌘⇧↵ accepts whichever chip is selected. Number-key accelerators were
  rejected: with Shift held, a digit reports a layout-dependent key (`!` on
  US keyboards).

**D13a. Where suggestions come from**, best first. They are all computed in the
engine by `review::suggest(tindex)` and returned with the queue item, so
nothing waits:

1. **Exact-key precedent.** The most common counter-account among categorised
   transactions with the same similarity key (D10). The confidence is that
   account's share of the matches.
2. **Rule-file precedent.** During import, the account a rule in the chosen
   rules file assigns to a description that matches. This catches a rule
   another file triggers but this one doesn't.
3. **Payee-prefix precedent.** When the exact key has no history, the longest
   leading run of at least two key tokens shared with categorised transactions
   (`sq blue bottle` from `sq blue bottle berkeley`). Its confidence is
   discounted.
4. **Recurring match.** The same funding account, the same amount within 1%
   and a similar day of month as a known recurring charge (the
   `reports/subscriptions.rs` clusterer, reused). For example, a renamed
   streaming charge.

- Precedent counts **reviewed transactions double**: a human said yes to them.
- Suggestions below 50% confidence are shown as alternatives only, never as
  the ⌘⇧↵ chip, so the one-keystroke path only fires on strong evidence.
- **Description suggestion.** When the key's history shows that people have
  renamed it (reviewed transactions whose original key matches and whose
  description differs), the chip offers the rename as well:
  "→ Blue Bottle Coffee · food:coffee".
  - That needs the original key recorded at rename time, so a rename from the
    overlay also writes `payee-key:<key>`, but only when the description
    actually changed. **(Q12)** Without it, rename precedent can't be found
    once the raw text is gone.
- Recents cards (already categorised) get a suggestion only when their
  category disagrees with a strong precedent. The chip then reads "usually
  food:coffee", which helps catch miscategorised rules.

### Speed

**D14. Fast by construction.**

- The queue endpoint returns the **full transactions** for the whole queue (≤50
  for recents; unknowns are paged at 100). Stepping between cards never waits on
  the network.
- Saves are queued and sent in order. The UI advances immediately and shows a
  small "saving…" dot in the header. A failure stops the run on that card, puts
  it back with the error, and keeps its edits.
- One HTTP call per save: a new **batch route** (`POST /api/transactions/batch`)
  carries the current transaction plus every checked similar one. It takes one
  lock, does **one re-parse** and one save. This needs a core
  `apply_batch(Vec<(Tindex, Fingerprint, Vec<EditOp>)>)` that applies all span
  edits in a single pass and validates once.
- The engine indexes the snapshot lazily: `similarity_key → Vec<Tindex>` and a
  `reviewed` bitmap, both cached in `ViewCache` per generation. Queue and
  similar requests on a 200k journal are then a hash lookup plus a sort of a
  few hundred items.
- After the batch, `journal.refresh({force:true})` runs as today. The overlay
  doesn't wait for it.
- Perf budget, checked by a Rust bench on the 200k perf fixture from plan 26:
  - queue < 20ms
  - similar < 5ms
  - 1-transaction batch < the existing single-PATCH time
  - 50-transaction batch < 2× that

**D15. Autocomplete everywhere.**

- **Accounts:** the existing `AccountInput`, unchanged.
- **Description:** a new `GET /api/payees` returns distinct payees with use
  count and last date, cached per generation. A new `DescriptionInput` (same
  combobox pattern as `AccountInput`) ranks by prefix, then word-start, then
  frequency × recency. Picking a payee that has a precedent also offers its
  category, as in D13.
- **Tags:** a new `GET /api/tags` returns tag names with their most common
  values. The tag chip input completes the name, then after `:` completes the
  value.
- All three lists load once per overlay open and refresh when the generation
  changes.

### Keyboard

**D16. Two modes, so single keys never fight with typing.** The overlay
registers a `PRIORITY.overlay`, `modal: true` layer, the first in the app. It
also fixes the bleed-through noted above for this overlay.
`TransactionModal` gets the same treatment in passing.

- **Card mode** (no text field focused) is where you land on each new card.
- **Field mode** (typing). **Esc** leaves the field and returns to card mode.
  **Esc** in card mode closes the overlay, asking first if there are unsaved
  edits.

| Keys                         | Mode   | Action                                                       |
| ---------------------------- | ------ | ------------------------------------------------------------ |
| <kbd>⌘↵</kbd> / <kbd>Ctrl↵</kbd> | any    | Reviewed, or the dirty-state save (the left footer button) |
| <kbd>⌘⇧↵</kbd> / <kbd>Ctrl⇧↵</kbd> | any  | Accept the top suggestion, mark reviewed, save, next (D13) |
| <kbd>y</kbd> / <kbd>↵</kbd>  | card   | Same as ⌘↵                                                   |
| <kbd>Y</kbd>                 | card   | Same as ⌘⇧↵                                                  |
| <kbd>→</kbd> / <kbd>n</kbd> / <kbd>l</kbd> | card | Skip                                              |
| <kbd>←</kbd> / <kbd>p</kbd> / <kbd>h</kbd> | card | Previous card                                     |
| <kbd>z</kbd>                 | card   | Revert changes                                               |
| <kbd>d</kbd>                 | card   | Edit description                                             |
| <kbd>a</kbd>                 | card   | Edit the first unknown account (or the first posting)        |
| <kbd>1</kbd>–<kbd>9</kbd>    | card   | Edit posting N's account                                     |
| <kbd>$</kbd>                 | card   | Edit the first posting's amount                              |
| <kbd>t</kbd>                 | card   | Add a tag                                                    |
| <kbd>m</kbd>                 | card   | Edit comment (memo)                                          |
| <kbd>s</kbd>                 | card   | Expand or collapse Similar; focus its list                   |
| <kbd>Space</kbd>             | list   | Toggle the focused similar row                               |
| <kbd>⇧A</kbd>                | card/list | Check or uncheck all similar                              |
| <kbd>i</kbd>                 | card   | Mark `reviewed:ignore` and advance                           |
| <kbd>o</kbd>                 | card   | Open in the full editor                                      |
| <kbd>Tab</kbd>               | field  | Accept the completion or suggestion; otherwise next field    |
| <kbd>Esc</kbd>               | field → card → close | Leave field / close the overlay                |

- Every footer button shows its key as a `<kbd>` hint.
- `?` lists everything, grouped under a new `KeyGroup` "Review".
- **⌘⇧↵ capture.** The keymap's chord parser has to accept `meta+shift+Enter`
  / `ctrl+shift+Enter`, plus a platform-neutral `mod+shift+Enter` alias, which
  is added if it isn't already there.
  - `chord.ts` already resolves `mod+` per platform and keeps `shift+` on named
    keys such as Enter, so `mod+shift+Enter` should parse as it is. A unit test
    confirms it.
  - The browser must not swallow the combination first. Neither Safari, Chrome
    nor the desktop app's WKWebView (wry) binds ⌘⇧↵. An e2e test pins it.
  - It fires inside fields because the binding is `mod`-qualified, the same
    rule ⌘↵ follows in `TransactionModal`.

**D17. Launch keys.**

| Keys        | Where                                 | Action                          |
| ----------- | ------------------------------------- | ------------------------------- |
| <kbd>u</kbd> | Journal                              | Review Unknowns                 |
| <kbd>r</kbd> | Journal                              | Review Recents                  |
| <kbd>u</kbd> | Import, once the dry run is ready    | Review Unknowns (staged)        |
| <kbd>r</kbd> | Import, once the dry run is ready    | Review All (staged)             |

The keys are disabled exactly when their button is.

### Motion

**D29. Fast animations that feel like knocking things out. (P)** They're all
CSS transforms and opacity, so they run on the compositor. They're all short,
and **none of them blocks input**: the next card is already in the DOM, and
keys act on it the moment they're pressed. A keystroke during an animation
finishes that animation instantly and starts the next one, so holding ⌘⇧↵
through ten cards never queues up motion.

| Moment                         | Motion                                                                                         | Duration          |
| ------------------------------ | ---------------------------------------------------------------------------------------------- | ----------------- |
| Overlay opens                  | Backdrop fades in. Card scales 0.96 → 1 and rises 8px, ease-out                                | 140ms             |
| Overlay closes                 | Reverse, slightly faster                                                                       | 100ms             |
| **Reviewed / saved**           | A green ✓ stamp pops on the card (scale 0.6 → 1.1 → 1). The card flies up-left (−40px, −6°, fades). The next card slides in from the right | 90ms stamp, 160ms swap |
| ⌘⇧↵ (suggestion accepted)      | Same as Reviewed. The suggestion chip first flashes into the account field (40ms)              | 200ms total       |
| Skip                           | The card slides left and fades, grey with no stamp. The next one slides in from the right      | 140ms             |
| Back (←)                       | Mirror of Skip: the card exits right, the previous one enters from the left                    | 140ms             |
| Ignore                         | The card sinks 12px and fades                                                                  | 140ms             |
| Similar applied, queue shrinks | The counter ticks down (number roll), and the progress bar eases to the new fraction           | 300ms, off the critical path |
| Run finished                   | Summary card scales in, with one brief pulse of the ✓ glyph                        | 200ms             |
| Footer swap (clean ↔ dirty)    | Buttons cross-fade in place, with no layout shift (fixed footer height)                        | 80ms              |
| Similar section expand         | Height animates through `grid-template-rows: 0fr → 1fr`                                        | 120ms             |

- **`prefers-reduced-motion`.** Every transform is replaced by a 60ms
  cross-fade, and the ✓ stamp is a static colour flash on the card border.
- **Implementation.** Svelte `in:`/`out:` transitions, with custom
  `stampOut`/`slideIn` functions in `review/ui/motion.ts` (pure, so durations
  and easings can be unit-tested), and `{#key card.index}` around the card.
- **Performance check.** A Playwright trace on a 50-card run must show no
  long tasks over 50ms during card swaps. Every swap from keypress to
  next-card-interactive must take under 16ms. This is a manual check, noted in
  the PR, not a CI gate.

### Buttons and counts

**D18. Button states.**

- Both Journal buttons show a count badge ("Review Unknowns **3**").
- Each is disabled, with a tooltip, at zero.
- The counts come from a new `review` block in `GET /api/journal/summary`:
  `{unknown, unreviewedRecent, notEditable}`. That endpoint is already
  computed per snapshot and per `today`, so the counts stay correct after
  every save and every 5s poll.
- The buttons only appear when `editing.canEdit`, like **Add Transaction**.

### Settings

**D19. Two numbers, stored per browser.** `reviewRecentLimit` (50) and
`reviewMaxDays` (365) go in `settings.svelte.ts`.

- There is no app-preferences page. They're edited from the overlay's ⋯ →
  **Settings** popover, and from a small ▾ on the **Review Recents** button.
  That keeps them where they're used, the same convention as the column
  toggles.
- `maxDays` = 0 means "no limit".
- Per browser, not per ledger. **(Q6)**

A third setting, `reviewMarkers` (on by default), turns D20 off.

### Indicators in the journal

**D20. Row markers.** A 3px left stripe on the first `<td>`, drawn as another
`box-shadow` layer so it stacks with the cursor rule. The cursor's rule sits
inside it.

| Row state                                         | Stripe                         |
| ------------------------------------------------- | ------------------------------ |
| Unknown (D3), not `ignore`                        | `warning` at 80%               |
| Unreviewed and within `reviewMaxDays`             | `base-content` at 15%, barely there |
| Reviewed, ignored, or older than the window       | none                           |

- Old history stays clean. The faint stripe marks exactly what **Review
  Recents** would pick up.
- The stripe needs two booleans per row, which `JournalRow` gets from the
  engine: `unknown` and `reviewed`.
- The window test runs client-side against the row date, so changing the
  setting needs no refetch.
- Card mode (narrow screens) uses the same stripe on the card's left edge.

**D21. Editor indicator.** `TransactionModal`'s title row gets a small badge:
"Reviewed Oct 30", "Ignored" or "Unreviewed" (ghost). It's informational. The
tag itself is still edited in the comment field.

**D22. Filter hooks.** The free-text filter already finds `reviewed:` as a
substring. Two filter chips, "Unreviewed" and "Unknown", would make the
stripes actionable. They are out of scope here. **(Q7)**

### Import

**D23. Review before writing works by overrides, and keeps "the preview is the
bytes".**

- The dry run gains a structured view. The server parses hledger's dry-run
  stdout with our own parser and returns `entries` (text, as today) plus
  `txns: WireTxn[]`, each with its ordinal and the fingerprint of its
  generated text.
- Review edits during import become **overrides**: `{ordinal, fingerprint,
  ops}`, using the same `EditOp` list as the batch route.
- **Dry run** and **commit** both accept `overrides`. The server regenerates the
  entries, checks each override's fingerprint against the regenerated
  transaction at that ordinal, applies the ops, and that result is what gets
  shown and appended.
  - A fingerprint mismatch (a rules file changed underneath) is a 409 naming the
    transaction. The UI re-runs the dry run and replays the overrides by
    original text.
- So the `<pre>` preview always shows exactly what will be written, edits
  included.
- CLI parity (`CliRun`) now differs when overrides exist. The "equivalent
  command" panel says so ("plus N reviewed edits"). **(Q8)**
- **Reviewed** during import puts `reviewed:<today>` on that staged transaction
  through an override. Unreviewed staged transactions are written untagged.

**D24. Similar during import has two sources, shown as two groups.**

- **In this import:** staged siblings with the same key. Checking them adds
  overrides.
- **Already in the journal:** unreviewed journal transactions (D11). These are
  real edits to existing entries. They're **sent with the commit** and applied
  under the same lock, in the same git commit as the append, so cancelling the
  import leaves the journal untouched. Commit gains `journalEdits` (batch ops,
  with fingerprints).

**D25. "Remember this" makes a rule.** On an import card whose description or
account changed (not a split), a toggle reads **Remember as a rule** (key
`R`). It expands into a compact rule builder:

- **Match.** Prefilled with a pattern built from the **raw CSV field** that
  produced the description.
  - To get that field, the dry run also returns, per transaction, the raw CSV
    record it came from. The server already runs the rules against the CSV, and
    hledger keeps record order, so ordinal *n* is record *n* after skips. This
    is verified by a test, with a fallback to whole-record matching when counts
    don't line up.
  - The pattern is the similarity key's tokens, regex-escaped, joined with
    `.*`, case-insensitive (hledger matchers already are).
  - The scope is the field named `description` in the `fields` list when there
    is one, otherwise the whole record. The user can edit it.
- **Then set.** `account2` (or the account slot that held the unknown) and/or
  `description`, prefilled from the edit. Each is checkable.
- **Live check:** "Matches **7** of 112 rows in this file (3 already
  categorised by other rules)."
  - Computed by a new `POST /api/import/rule-preview`, which runs the dry run
    with the candidate rule appended in a temp copy of the rules file. It is
    debounced and cancellable.
  - Rows that would change are listed, so a too-broad pattern is obvious before
    saving.
- **Saving the card** appends the rule through the existing rules PUT
  (revision-pinned, appended at the end, since the last match wins) and re-runs
  the dry run. Overrides the rule now covers are dropped automatically (their
  regenerated text already equals the overridden text). Others replay by
  original text.
- **The rules file joins the import's git commit** (`Plan::targets`) when it was
  edited during this import. Today it isn't. Committing it is right: the rule
  and the transactions it produced belong together.

Rules only get written when you ask (the toggle defaults off). **(Q9)**

**D26. Review is optional and never blocks.** In the import flow the review
buttons sit in `DryRunPanel` above **Write changes**, with counts ("Review
Unknowns **9** · Review All **112**"). Closing the overlay keeps every edit as
an override. **Write changes** is always available. Unknowns left unreviewed
are written as they are, which is today's behaviour.

**D30. QuickBooks imports get review too. (P)** An alias can map a QuickBooks
account to `expenses:unknown`, and not everything in QuickBooks is categorised.

- `QbJournalPanel` gets the same **Review Unknowns** / **Review All** buttons
  and keys once a preview is ready, with no unmapped accounts.
- Like the CSV path, the QuickBooks commit rebuilds transactions on the
  server from the stage (`WireQbCommitRequest` is just `{stageId,
  updateChanged}`, `qb_journal_api.rs:291`), and the preview only sends a
  `sample`. So:
  - the preview gains the full `txns`;
  - the commit gains `overrides`, the same `EditOp` shape as D23. They are
    keyed by **QuickBooks Trans # `id:`** instead of by ordinal (it is stable,
    and already the dedupe key), plus the fingerprint;
  - it gains `journalEdits` for similar existing transactions, as in D24.
- **Remember as a rule** is hidden here (see Scope).

### Rules from the Journal view

**D27. Rules from the Journal view: infer the rules file, don't record it.
Approved as Phase 6, after user testing of Phases 1–5. (P)**
You asked for ideas that don't tag entries. Three signals already in the
ledger identify the rules file for most imported transactions, with no new
metadata:

1. **The funding account.** Every rules file names its bank account (`account1`,
   or the `account1` assignment in a top-level block). An unknown transaction's
   other posting (`assets:checking:chase`) maps to the rules files whose
   `account1` is that account. In a typical setup that is exactly one file. The
   engine already scans and parses every `*.rules` under the journal
   (`discovery.rs`).
2. **The FITID convention.** Transactions written with `id:%fitid` came from a
   rules file that has `comment id:%fitid`, which narrows ties.
3. **hledger's `.latest.NAME` markers.** Each one names its CSV, and so its
   rules file. Its date bounds which transactions it could have produced.

With one match, the journal overlay offers **Remember as a rule** exactly as in
import, with the match pattern built from the journal description. That works
because an *unknown* transaction by definition matched no categorising rule,
so its description is usually the raw CSV text, unless a description-only rule
renamed it, which the builder detects by testing the pattern against the file's
rules. With several matches, a picker. With none, the toggle doesn't appear.

The live check (D25) needs CSV rows to test against. From the journal it can use
the most recent CSV in the rules file's `source` folder when there is one.
Otherwise it falls back to "matches N similar journal transactions" (the
similarity index).

**Several rules files for one account. (P)** A common layout is one rules file
per year (`citibank-2025.csv.rules`, `citibank-2026.csv.rules`), all with the
same `account1`. Inference groups candidates into **families**: the same name
once a year or date token is removed (`citibank-.csv.rules`), in the same
directory or sibling year directories (`2025/citibank.csv.rules`,
`2026/citibank.csv.rules`). Within a family:

1. If the names carry years, pick the file whose year equals the
   **transaction's** year. Failing that, the newest year at or before it. A
   2026 transaction goes to the 2026 file, and a 2025 one to the 2025 file.
2. Otherwise, pick the newest by `.latest.NAME` date, then by mtime.
3. The builder states the choice and offers a dropdown of the rest: "Saving to
   **citibank-2026.csv.rules** (newest of 2 for Citi Card)".

When a family has a year for the transaction's year but the user is clearly
importing ahead (the newest file is a later year), the newest file is offered
as the second option.

Different families for the same account are a genuine tie, so the builder shows
a picker. The family logic is shared with the import's own candidate ranking
so the two never disagree. Phase 6 also covers whatever user testing turns
up.

## Scope

**In:**

- Review overlay with both journal queues.
- Similar-apply.
- Suggestions and payee/tag autocomplete.
- Batch and surgical tag/comment ops.
- Row and editor indicators.
- Settings.
- Import review with overrides, journal edits in the commit, and the rule
  builder.
- Docs and screenshots.

**Out (for now):**

- Similar-apply for splits. A later version could apply only when amounts are
  identical, as Patrick suggested.
- Filter chips (D22).
- Rules from the journal (D27, Phase 6 after user testing).
- Undo beyond ← and git.
- Rule-making for QuickBooks imports. Its "rules" are account aliases, which
  map a whole QuickBooks account rather than a payee, so "remember this" has
  nothing sensible to write. Review itself **is** in scope (D30).

## Phase 1 — Engine: review model, similarity, surgical tag and comment ops, batch (core only, PR 1)

**Code**

- `crates/ledgeline-core/src/review.rs` (new):
  - `ReviewState::{Unreviewed, Reviewed(date), Ignored, ReviewedOther}` from a
    transaction's header tags;
  - `is_unknown(txn)`, reusing `checks.rs is_uncategorized` (make it `pub(crate)`
    rather than duplicating it);
  - `similarity_key(&str) -> Option<String>`;
  - `ReviewIndex { by_key: HashMap<String, Vec<Tindex>>, state: Vec<ReviewState> }`,
    built from a `Journal`;
  - `queue(kind, today, limit, max_days) -> QueueResult {items, total, not_editable}`;
  - `similar(tindex) -> Similar {candidates, precedent}`;
  - `suggest(tindex) -> Vec<Suggestion {account, description?, source, confidence}>`
    (D13a), reusing the subscriptions clusterer for the recurring source.
- `edit.rs`:
  - `EditOp::{SetDescription, SetPostingAccount{posting, account}, SetTxnTag{name, value: Option}, SetTxnComment, Replace(body)}`;
  - `set_txn_tag` and `set_txn_comment` as surgical span rewrites, like
    `set_description` (:1311);
  - `apply_batch(&[(Tindex, Fingerprint, Vec<EditOp>)])`. It resolves every
    target under one lock, checks fingerprints, applies span edits
    back-to-front per file so spans stay valid, then makes **one**
    `validate_with` call and one save. It's atomic: any failure writes
    nothing.
- A pure tag-comment helper `upsert_tag(comment: &str, name, value) -> String`
  that keeps hledger's comma semantics. It's shared with D23 overrides.

**Tests** (`tests/review.rs`, `tests/edit_batch.rs`, new fixture
`fixtures/review/review.journal`):

- `ReviewState` for each D1 row, plus posting-level tags being ignored.
- `similarity_key`:
  - table cases (Square/Toast/POS prefixes, dates, store numbers, all-digit →
    None, `|` notes);
  - proptests: idempotent, case- and digit-insensitive.
- Queue:
  - ordering, limit, `max_days` boundary (inclusive) for **both** queues,
    future dates, `ignore` excluded;
  - unknown + reviewed-date stays in Unknowns but is **not** in Recents;
  - an untagged unknown is in both;
  - generated postings don't count;
  - read-only include counted as not editable.
- Similar: key equality, excludes self/reviewed/ignored, 500 cap, precedent
  picks the modal counter-account.
- Suggest: each D13a source alone, ranking between sources, reviewed
  transactions weighted double, the 50% threshold for the top chip, and none
  for an all-digit description.
- `set_txn_tag`: every D2 example byte-exact (new comments get exactly
  `"  ; "`, and existing spacing is preserved), removal, continuation-line tags,
  CRLF files, a tag inside a free-text comment.
- **Elision survives:** an elided posting stays elided after tagging and after
  an account change.
- `apply_batch`:
  - 50 transactions across two included files;
  - stale fingerprint → nothing written;
  - one invalid op → nothing written;
  - only one re-parse (counted through a test hook).
- Bench (`benches/review.rs`) on the plan-26 200k fixture, against the D14
  budget.

## Phase 2 — Wire (server and TS types; no UI yet, PR 2)

**Code** (`crates/ledgeline-server/src/review_api.rs`, registered above the auth
`route_layer` like the others):

- `GET /api/review/queue?kind=unknowns|recents&today&limit&maxDays&offset`
  returns the full `WireTxn`s plus `total` and `notEditable`.
- `GET /api/review/similar/{index}` returns
  `{candidates: WireTxn[], inQueue: index[], precedent: {account, count}|null}`.
- `GET /api/payees` returns `[{payee, count, last}]`; `GET /api/tags` returns
  `[{name, count, values: [{value, count}]}]`. Both are cached per generation.
- `POST /api/transactions/batch` takes `{edits: [{index, fingerprint, ops}]}`
  and returns 200 with the new generation, or 409 naming the stale index.
- `GET /api/journal/summary` gains `review: {unknown, unreviewedRecent,
  notEditable}` (with `limit`/`maxDays` query params).
- `JournalRow` gains `unknown: bool` and `reviewed: bool`.
- `ViewCache` holds the `ReviewIndex` per generation.
- TS: `review/types.ts`, decoders in `api/nativeDecode.ts`, client functions in
  `api/native.ts`, and `stores/review.svelte.ts` (queue, cursor, prefetch, save
  queue).
- `editMapping.ts` gets `formToOps(original, form) -> EditOp[] | {replace}`,
  which picks surgical ops when only description, accounts, tags or comment
  changed (D9). It also gets `isSplit(original, form)`.

**Tests**

- `review_endpoints.rs` (response, GET snapshot and bytes on disk, the
  `edit_endpoints.rs` pattern), plus a `native_wire_golden.rs` update.
- Vitest:
  - decoders;
  - `formToOps` table tests (every field alone, combinations, split detection,
    the elided posting left alone);
  - the review store: prefetch, optimistic advance, failure rewinds to the
    failed card with edits intact, generation change refetches.
- A `reviewLive.integration.test.ts` for `just test-integration` (memory: run
  it after any /api shape change).

## Phase 3 — Journal UI: overlay, buttons, keys, markers (PR 3)

**Code** (`web/src/lib/review/ui/`):

- `ReviewOverlay.svelte`: daisyUI `.modal` with `use:dismissible` and its
  overlay key layer. It's full-height on narrow screens and `max-w-2xl`
  otherwise.
- `ReviewHero.svelte`, `ReviewFields.svelte`, `PostingRows.svelte` (reusing
  `AccountInput`), `DescriptionInput.svelte` (new combobox),
  `TagChips.svelte`, `SimilarSection.svelte` (collapsible, tri-state check-all
  header, rows showing date, original description, amount and the chips they'll
  receive), `ReviewFooter.svelte`, and `ReviewSettings.svelte` (popover).
- Icons: inline SVG check, arrow-right, arrow-left, x, ellipsis, in the house
  style.
- `TransactionTable.svelte`: the two buttons with counts and the ▾ settings,
  plus `u`/`r` bindings.
- `TransactionRow.svelte`: the D20 stripe.
- `TransactionModal.svelte`: the D21 badge, and an overlay-priority modal key
  layer.
- `keys/types.ts`: the "Review" group.
- `settings.svelte.ts`: `reviewRecentLimit`, `reviewMaxDays`,
  `reviewMarkers`.

**UX requirements** (each is a component test, `*.svelte.test.ts` via
`fakeEngine`):

- Reviewed is disabled while any posting is unknown. It enables the moment the
  account changes.
- The footer swaps on the first change and swaps back after Revert or after
  undoing the change by hand (dirty is computed by value, not by touch).
- The similar section is absent when nothing matches. Its collapsed header
  shows the count. All rows start unchecked, the header checkbox is tri-state,
  and splits hide the section and show the note.
- The save sends one batch: the current transaction with the reviewed tag, and
  checked candidates without it.
- Keyboard: every D16 row. In particular, single keys do nothing while a field
  is focused; Esc goes field → card → close; ⌘↵ works from inside a field; the
  `j` journal binding does not fire under the overlay.
- Focus lands on the card (not a field) when each card appears, and focus is
  restored to the journal row on close, with the journal cursor on the last
  reviewed transaction.
- Suggestion chip: Tab fills it in and cycles the alternatives. ⌘⇧↵ accepts
  the selected chip, saves with the reviewed tag, and advances, from inside a
  field too. With no chip, ⌘⇧↵ behaves as ⌘↵.
- Dynamic stack (D5a): in Unknowns, applying to 9 queued siblings ends the run
  with "1 reviewed · 9 fixed via similar". In Recents, those siblings stay,
  showing the applied change. The total recomputes. A background revalidation
  drops a queued transaction deleted elsewhere.
- Motion (D29): each transition's class or keyframe is applied for its
  moment. A keypress mid-animation acts on the next card immediately. Reduced
  motion swaps transforms for a fade. `motion.ts` has unit tests.
- The empty queue shows "Nothing to review" with a close button. Reaching the
  end shows a summary ("12 reviewed, 3 skipped, 41 similar updated") and closes
  on ↵.
- The buttons are disabled at zero counts, and `u`/`r` are disabled with them.
- Row stripes: the colour for each D20 row, turned off by the setting, and the
  window boundary.

**e2e** (`web/e2e/review.e2e.ts`, which also covers ⌘⇧↵ reaching the page
unswallowed): a second engine on
`fixtures/review/review.journal` (port 5098), added to
`playwright.config.ts`, so `sample.journal` totals don't move. One flow:

1. open with `u`;
2. Tab-accept a suggestion;
3. expand similar, check all, ⌘↵;
4. skip one, go back with ←, mark reviewed with `y`;
5. assert the bytes on disk via a GET of the raw transactions.

## Phase 4 — Import review, overrides, rule builder (PR 4)

**Code**

- `import_api.rs`:
  - the dry run returns `txns` plus raw `records` (D23/D25);
  - `overrides` on dry-run and commit, with the fingerprint check;
  - `journalEdits` on commit, applied in the same lock and git commit through
    `apply_batch`;
  - `POST /api/import/rule-preview`;
  - the edited rules file added to `Plan::targets`;
  - `WireCommitRequest` keeps `deny_unknown_fields`, with the new fields added
    explicitly.
- `importStore.svelte.ts`: an overrides map keyed by ordinal and original text,
  replayed after every dry run.
- `review.svelte.ts` gains a "staged" source; the overlay is the same component.
- `DryRunPanel.svelte`: the buttons, counts, `u`/`r`, and an "N reviewed edits"
  line by **Write changes**.
- `review/ui/RuleBuilder.svelte`: built on `imports/model.ts appendRule` and
  `rulesStore.save`.
- `CliRun` notes the overrides (D23).
- QuickBooks (D30): `qb_journal_api.rs` preview gains `txns`; the commit gains
  `overrides` keyed by Trans # and `journalEdits`. `QbJournalPanel.svelte`
  gets the buttons and `u`/`r`. Rule-making is hidden for this source.

**Tests**

- `import_endpoints.rs`:
  - an override changes the appended bytes, and the preview equals those bytes;
  - a stale override → 409;
  - `journalEdits` and the append land in one git commit;
  - a cancelled import writes nothing;
  - rule-preview counts on `fixtures/rules/simple/checking.csv.rules`;
  - an ordinal↔record alignment test, including skipped header lines and
    `skip` rules;
  - a misalignment falls back to whole-record matching.
- Vitest: override replay after a rules save (covered → dropped, uncovered →
  kept, changed → kept and flagged), and RuleBuilder pattern generation.
- e2e (`imports.e2e.ts`):
  1. stage the docs `checking.csv`;
  2. dry run, then `u`;
  3. recategorise and make a rule, checking the live count;
  4. Write changes;
  5. assert the rule exists in the rules file and the journal has the edited,
     reviewed transaction.
- QuickBooks: a `qb_journal_endpoints` test where an alias maps to
  `expenses:unknown`, an override keyed by Trans # recategorises it, and a
  stale fingerprint → 409. Fixture: a new export under
  `fixtures/import/qb-journal/` (memory: `git add -f`, because the global
  `*-journal` ignore hides it).

## Phase 5 — Docs and screenshots (PR 4 or a follow-up)

- `docs/journal/review.md` (new):
  - why review, and the two queues;
  - the card;
  - Reviewed vs Skip;
  - similar transactions;
  - suggestions;
  - the `reviewed:` tag and `reviewed:ignore`, with
    `hledger reg not:tag:reviewed`;
  - settings;
  - row stripes.
- `docs/journal/index.md` links to it, with a sentence on stripes.
- `docs/journal/keyboard.md` gets a "Review" section and adds `u`/`r` to the
  Journal table.
- `docs/imports/new-transactions.md`: a "Review before writing" section and
  **Remember as a rule**. `create-rules.md` links back.
- Shots (`web/docs-screenshots/review.docs.ts`, new staged set `review` on port
  5214 using `fixtures/review/review.journal`, plus the `sample` import set),
  each with a `MANIFEST.md` row:
  - `journal/images/review-buttons.png`: the toolbar with counts, and two rows
    showing stripes.
  - `journal/images/review-card.png`: a clean unknown card with a suggestion
    chip.
  - `journal/images/review-card-dirty.png`: a changed account, similar
    expanded, two rows checked, and the dirty footer.
  - `journal/images/review-settings.png`: the popover.
  - `imports/images/review-before-write.png`: the DryRunPanel buttons.
  - `imports/images/review-rule.png`: the rule builder with its live count.
- No spec presses Save.
- `just docs-build` link check.

## Phase 6 — Rules from the Journal view (D27; after user testing)

- Rules-file inference: the `account1` index, the FITID convention, the
  `.latest.NAME` window, and **families with year selection**.
- The journal overlay shows **Remember as a rule**, with the chosen file and
  its alternatives.
- The live check against the source folder's newest CSV, or similar-count as
  the fallback.
- Changes from user testing of Phases 1–5.

It is sized after Phases 1–5 land and have been used for real.

## Mockups

### Clean card (an unknown, with a suggestion)

```
┌──────────────────────────────────────────────────────────────────────┐
│ Review Unknowns        3 of 9   ▰▰▰▱▱▱▱▱▱            ⋯   ✕         │
├──────────────────────────────────────────────────────────────────────┤
│                                                                      │
│   Blue Bottle Coffee                                     −$6.75     │
│   Oct 12, 2026 · from Chase Sapphire · cleared                       │
│                                                                      │
│  ─────────────────────────────────────────────────────────────────── │
│  d  Description  [ SQ *BLUE BOTTLE 4412 OAKLAND                    ] │
│                                                                      │
│     Postings                                                         │
│  1  [ expenses:unknown            ]⚠                    $6.75        │
│        ↹ expenses:food:coffee  — used 14× for this payee             │
│  2  [ liabilities:chase:sapphire  ]                    −$6.75        │
│                                                                      │
│  t  Tags      ( id:20261012-0042 ×)  [+ tag ]                        │
│  m  Comment   [                                                    ] │
│                                                                      │
│  ▸ Similar unreviewed  ·  6                                          │
├──────────────────────────────────────────────────────────────────────┤
│ [ ✓ Reviewed  ⌘↵ ]  (disabled: categorise first)      [ Skip  → ]   │
└──────────────────────────────────────────────────────────────────────┘
```

### Dirty card (account changed, similar expanded)

```
├──────────────────────────────────────────────────────────────────────┤
│  1  [ expenses:food:coffee        ]                     $6.75        │
│  ...                                                                 │
│  ▾ Similar unreviewed  ·  6        apply: account                    │
│    [–] all                                                           │
│    [✓] Oct 03  SQ *BLUE BOTTLE 0917 OAKLAND   $5.50  → food:coffee   │
│    [✓] Sep 28  SQ *BLUE BOTTLE 0233 OAKLAND   $7.25  → food:coffee   │
│    [ ] Sep 14  SQ *BLUE BOTTLE 8812 SF        $4.00  → food:coffee   │
│    ... 3 more                                                        │
├──────────────────────────────────────────────────────────────────────┤
│ [ Mark Reviewed and Save  →  ⌘↵ ]                [ ✕ Revert  z ]    │
└──────────────────────────────────────────────────────────────────────┘
```

### Import card extra

```
│  ☑ Remember as a rule  (R)                                           │
│     Match   [description ▾]  [ blue.*bottle.*oakland       ]  (i)    │
│     Set     ☑ account2 → expenses:food:coffee                        │
│             ☐ description → Blue Bottle Coffee                       │
│     Matches 7 of 112 rows in checking.csv · 0 already categorised    │
```

### Journal toolbar

```
 [ + Add Transaction ]  [ Review Unknowns 9 ]  [ Review Recents 50 ▾ ]
 ▌ 2026-10-12  SQ *BLUE BOTTLE 4412   expenses:unknown        $6.75   ← amber stripe
 ▏ 2026-10-11  Safeway                expenses:food:groceries $82.10  ← faint stripe
   2026-10-10  Rent                   expenses:housing        $2,400  ← reviewed: none
```

## Sequencing

1. PR 1 (engine) → PR 2 (wire) → PR 3 (journal UI) → PR 4 (import + docs).
   Each one runs `just pre-push` before merging.
2. PR 2's wire is additive. The old UI keeps working, as in plan 31's C2.
3. Phase 3 can start against `fakeEngine` once PR 2's TS types are fixed.
4. Worktree notes from memory apply: create worktrees from HEAD, agents never
   run Playwright, and `git add -f` anything under a `*-journal` path.
5. Per-phase contract amendments go in `plans/32-amendments-pN.md` and are
   folded in when the branches meet.

## Risks

| Risk                                                         | Mitigation                                                                                  |
| ------------------------------------------------------------ | ------------------------------------------------------------------------------------------- |
| Bulk apply edits the wrong transactions                      | Exact keys only, all unchecked by default, each row shows its exact change, atomic batch, git autocommit as undo |
| Batch spans shift mid-apply                                  | Back-to-front per file, one validate, all-or-nothing; a test across two included files       |
| Optimistic advance hides a failed save                       | The save queue is serial; on failure the run stops and rewinds to that card with edits and the error |
| Tag write mangles a comment with free text and commas        | `upsert_tag` byte-exact table tests; hledger itself parses the result in a test              |
| Override/record alignment wrong for exotic rules (`skip`, `end`, multi-line) | Alignment check and fallback to whole-record matching; the rule builder's live count makes a bad pattern visible |
| Rules save between dry run and commit (another tab)          | Override fingerprints → 409 → re-run and replay                                              |
| Stripes add visual noise                                     | Faint stripe only inside the window, nothing on reviewed rows, a setting to turn it off      |
| 200k journals                                                | Engine-side index per generation; benchmarks against the D14 budget                         |

## Questions for Patrick

1. **(Q1) Ignore from the overlay.** Add an **Ignore** action (⋯ menu, key `i`)
   that writes `reviewed:ignore` and advances? *Default: yes.*
2. **(Q2) Recents order.** Newest first (catch up on what just happened), or
   oldest first within the window (work forward chronologically)? *Default:
   newest first.*
3. **(Q3) Journal filter.** Should the queues respect the current Journal
   filter (e.g. review only Chase Sapphire), or always cover the whole journal?
   *Default: whole journal; a filtered review could come later.*
4. **(Q4) Saving while still unknown.** If you rename but leave
   `expenses:unknown`, allow **Save** (not reviewed), or force Skip/Revert?
   *Default: allow Save.*
6. **(Q6) Settings scope.** Per browser (localStorage), or per ledger? *Default:
   per browser*, because there's no per-ledger settings store yet.
7. **(Q7) Filter chips.** Want "Unreviewed" / "Unknown" quick filters on the
   Journal now, or later? *Default: later.*
8. **(Q8) CLI parity during import.** With review edits, the import is no
   longer exactly `hledger import`. Is it OK that the "equivalent command"
   panel says "plus N reviewed edits"? *Default: OK.*
9. **(Q9) The reviewed tag on imported transactions.** When you press Reviewed
   during import, the transaction is written with `reviewed:<today>`. Should
   transactions a *rule* fixed during this import also be stamped reviewed?
   *Default: no*, matching "applied to others → not reviewed".
11. **(Q11) Tag date.** `reviewed:2026-10-30` uses the browser's local date. A
    real hledger *date tag* (`date:`) is different and is not involved. OK?
    *Default: yes.*
12. **(Q12) `payee-key:` tag on renames.** To suggest a rename later (D13a),
    renaming from the overlay records the original similarity key as
    `payee-key:sq blue bottle oakland`. It's one more tag, written only when a
    description actually changes. Worth it, or should rename suggestions be
    skipped? *Default: write it.*

## Resolved with Patrick (2026-10-30)

1. **Header comments use two spaces before `;`.** The engine already writes
   `"  ; "` this way. The plan's examples are now in a code block so the
   double space shows. (D2)
2. **Motion.** Quick, non-blocking animations for opening the overlay and for
   every card exit, with a ✓ stamp on Reviewed. (D29)
3. **The queues share the lookback window.** Unknowns are limited to
   `maxDays` too. Recents excludes anything tagged reviewed, unknown or not.
   Unknowns includes everything except `ignore`. (D4)
4. **Suggestions (Q5): yes, and smarter.** Several sources are ranked, and
   ⌘⇧↵ / Ctrl⇧↵ / `Y` accepts the selected suggestion, marks it reviewed,
   saves and advances in one keystroke. (D13, D13a)
5. **The stack shrinks as you go.** Fixing queued siblings through Similar
   removes them from Unknowns immediately. (D5a)
6. **Rules from the Journal (Q10): yes, as Phase 6 after user testing**, with
   year-split rules-file families handled. (D27)
7. **QuickBooks gets review**, because aliases can map to unknown. (D30)

## Status

- Plan written 2026-10-30 and revised the same day after Patrick's review.
- **Phase 1: committed 2026-10-30** (`8131198`). Amendments are in `32-amendments-p1.md`.
- **Phase 2: committed 2026-10-30** (`936f6f5`). Amendments are in `32-amendments-p2.md`.
- **Phase 3: committed 2026-10-30** (`c02538d`), after two rounds of UI review
  with Patrick, which produced the card deck and the visual tweaks. Amendments are in
  `32-amendments-p3.md`.
- Also on this branch: `721edb0`, the Projections scroll fix (not part of plan 32).
- **Phase 4: committed 2026-10-31** (`a4bc8ad`), as 4a (server and wire) and 4b (UI). The same commit carries the
  off-screen 409 fix, the git-integration removal (Patrick, 2026-10-31) and "Test Import" with `use:reveal`.
  Amendments are in `32-amendments-p4.md`.
- **Phase 5: committed 2026-10-31.** Docs pages, the `review` docs-screenshot set on :5214 with
  `review.docs.ts`, two import-review shots in `imports.docs.ts`, and the RuleBuilder's `%` warning. The PNGs were generated and reviewed.
  Amendments are in `32-amendments-p5.md`.
- **Phase 6** (rules from the Journal view, D27) waits for user testing of Phases 1–5.

## Contract amendments made during implementation

(none yet)
