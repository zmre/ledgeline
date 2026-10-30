# 31 — Contract amendments made during Phase C3

These belong under "Contract amendments made during implementation" in
`plans/31-projections-editor-and-chart-tweaks.md`, after Phase C2's
(`plans/31-amendments-c2.md`). Fold them in when the branches meet.

1. **Amounts line up down the whole list, not just within a card.** The
   plan asked for one amount column per card. Each card's postings grid
   now uses the same fixed tracks (`POSTING_GRID` in `ui/list/frame.ts`:
   account `1fr`, amount `9.5rem`, annotations `19rem`). With an `auto`
   track, each card put its amounts wherever its own annotations left room,
   so the column moved from card to card. Long annotations wrap inside
   their track.

2. **Done waits for an in-flight render instead of being disabled during
   it.** Done is disabled while the draft has problems
   (`draftProblems`) or the server refused the render (a 400). While a
   render is pending, pressing Done queues it. The dialog commits when the
   render answers, or stands down if the answer is a refusal. The reason:
   the blur that commits the last field is what starts the render, so a
   strictly disabled button would swallow the click that ended the edit.

3. **Only a refusal blocks Done.** A render that fails for any other reason
   (engine unreachable, an engine without the endpoint) shows "Couldn't
   preview the text: …" in the warning colour, and Done stays usable. The
   save is still the backstop: the writer's round-trip check runs on save.

4. **"Move posting to a new rule" commits at once.** The button sits on each
   posting row (`⇲`, named "Move {account} to a new rule"). It applies the
   draft and the split as ONE commit (one revision) and closes the dialog,
   with the cursor on the new rule. Holding a second, split-off entry
   inside one draft would have needed a two-entry draft model for one rare
   action. It is offered on existing rules only, not on a new draft.

5. **Step changes split the whole block.** "Add a step change…" opens a
   small date prompt under the card. It defaults to the page's step date
   and says why when it cannot split there (`stepProblem`). It then opens
   the new segment's draft (`stepDraft`). The split is `entries.splitBlock`:
   every posting of the block is bounded `to` the date, and every posting is
   copied `from` it into a new group, each copy keeping its posting's id, so
   the pair is a step. `splitStep` split one LINE, which would leave a
   multi-posting block with two spans, and the writer refuses that.
   `splitStep`/`mergeStep` stay in `scenarioModel` and are tested.
   `entries.mergeSteps` merges per posting id with them. The earliest
   segment survives, in its own place in the file. As before, an asset rule
   offers no step change.

6. **Step siblings: an outline, not a gutter rail.** Hovering a step's
   card gives its siblings a dashed primary border, and the chip moves the
   cursor to the next segment. The joining rail for adjacent siblings was
   not built.

7. **One namespace of keys.** The list addresses entries by `key` alone.
   `takenIds` now covers line ids and groups, one-off ids, `=` keys, comment
   and fixed keys, and layout keys, so a fresh key never collides across
   kinds. The engine's own keys never do either (`rule:K`/`off:K` share one
   counter, and `auto:`/`comment:`/`fixed:` are prefixed).

8. **The client derives a layout when it has none** (`withLayout` in
   `scenarioModel`), in the server's legacy order: rules by first
   appearance, one-offs, `=` rules, comments, file text. A keyless `=` rule
   gets `auto:new-N`. The engine always sends a layout (amendment C2-2), so
   this only covers scenarios built here and older bodies. `scenarioStore.
   adopt` applies it, and so do `scenarioToWire` and `entriesOf`.

9. **`projectable` prunes the layout with the entries it drops.** A run
   body's layout must name exactly the entries it holds (`check_layout`). A
   half-typed line or a disabled entry dropped from a run takes its layout
   item with it.

10. **UI-only fields on the wire types.** `LayoutItem.attachedText?` holds an
    edited attached comment (sent as `attachedText`, else `attached`).
    `ScenarioComment.edited?` marks a comment whose text or style changed
    (sent as `{key, style, text, previous}`, else `{key, raw}`). A new
    comment has `raw: ""` and is sent without `previous`, so the engine
    writes the house style. `ScenarioLine.section` (the hold) and the
    `HeldSection` type are gone.

11. **After Done, the item's `raw` is the rendered preview.** The text the
    dialog showed when Done was pressed becomes the layout item's `raw`, so
    the card's "rule text" disclosure and the next dialog's "Writes:" show
    the entry's current text, not the file's old bytes. It is exact, since a
    save renders the entry with the same functions. It is replaced by the
    file's own bytes on the post-save adopt (C10).

12. **A new "Recurring flow" opens with an empty account.** It could be
    income or expense, so there is no prefix to choose. "Asset" opens on the
    journal's asset root (`sectionPrefix("asset", …)`). `sectionPrefix`
    stays for all three sections.

13. **The Add menu inserts after the cursor's entry**, or at the end when
    there is no cursor. The empty state's four buttons append.

14. **File text: Enter/e expands it.** File text has no dialog, no menu and
    no grip. Enter or `e` on its card expands text clipped to three lines
    and collapses it again (the plan's "view, for file text"). The card says
    "file text · read-only", with "Edit this in the file" as its tooltip.

15. **The dialog focuses its first focusable control** (the Enabled toggle
    in the header, for a rule, one-off or `=` rule), which is what
    `dismissible`'s trap does. The plan said "its first field". Putting
    Enabled first matches the mockup's order.

16. **Comments in the dialog.** A section heading's title is a single-line
    input, or a textarea when the heading already has several lines. A note
    is a textarea. The Section heading / Note switch is a radio group. A
    `comment … end comment` block is edited verbatim and stays a block.
    Comments get Duplicate but not "Duplicate as alternative" (they cannot be
    turned off).

17. **Filter chips with a zero count are hidden**, except All and the chip
    in use, so the chip row lists only what the file has. The URL key is
    `show` (`params.ts`), as the plan named it.

18. **The read-only notice moved, and Save explains itself.** The notice is
    now beside the list (`projection-read-only`, same test id). The file
    bar's disabled Save button carries the reason as its title.
    `SaveScenarioDialog.svelte.test.ts`'s read-only case asserts that title
    instead.

19. **`RowMenu` gained `text`, `triggerClass`, `testid` and an exported
    `openMenu()`.** It serves as the "+ Add ▾" menu, which `n` opens.

20. **Draft tests live in `entryDraft.svelte.test.ts`** (the `components`
    project), not `entryDraft.test.ts`, because `EntryPreview` holds
    `$state`. The list's component tests mount through
    `#lib/testing/ScenarioListHarness.svelte`, which wires the list to the
    real store as the page does: `commit` replaces the scenario, and a
    test's `render()` props are not reactive.

21. **`componentSetup.ts` stubs the Web Animations API**
    (`getAnimations`/`animate`, an animation that is already finished).
    jsdom has none, and `animate:flip` calls both on every move. This is the
    same kind of honest stub as the existing `scrollIntoView`, `matchMedia`
    and `ResizeObserver` ones.

22. **One e2e test writes, to a scratch copy only.** "an entry turned off is
    saved commented out, and reloads turned off" copies
    `projections-parity/comments.journal` to
    `fixtures/projection-e2e-scratch-comments.journal`, saves there, checks
    the bytes and the reload, and deletes the copy in `finally`. The spec's
    rule is "nothing here writes a committed fixture", and that rule holds.
    The plan asked for this test.

23. **Accessibility: a manual pass, not axe.** `@axe-core/playwright` is
    not a dependency. What is in place: each card is an `article` labelled
    by its header line; the list is `role="list"` with one `listitem` per
    entry; kind badges are text; "turned off" is in the off badge; every
    Edit button and menu has a unique name (`entriesOf` names entries,
    adding "(entry N)" for duplicates); one polite live region announces
    every action; the drag grip is `aria-hidden`, with the menu and J/K as
    the accessible path.

24. **Live integration cases added** (`nativeLive.integration.test.ts`): a
    run carrying a served file's layout back, and a new section heading
    rendered as 22 semicolons then `;; Capital`. Together they are the
    cross-language check for the two new body shapes every C3 save sends.

25. **Tests deleted on purpose.** These went with the table:
    `ProjectionsTable.svelte.test.ts`'s section-stability and hold tests,
    and `scenarioModel.test.ts`'s tests of `groupLines`, `logicalRows`,
    `rowNames`, `sectionOfLine`, the hold, `moveRow(To)`, `moveEvent(To)`,
    `gatherRules`, `withCounter`, `withGains`, `withNote`/`ruleSize`,
    `removeSegment` and `duplicateRow`. Their behaviours are ported to
    `entries.test.ts`, `entryDraft.svelte.test.ts`,
    `ScenarioList.svelte.test.ts` and `EntryDialog.svelte.test.ts`.
    Exceptions: section membership is now `resolvedSection` (asserted where
    it was `sectionOfLine`), and the counter is a block field (no split).
    `NoteButton.svelte` had no test of its own.
