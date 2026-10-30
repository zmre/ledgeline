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
   **Removed on Patrick's request (2026-10-29), see 28.**

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
    `show` (`params.ts`), as the plan named it. The chips themselves changed
    on 2026-10-29, see 26.

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

## Patrick's 2026-10-29 feedback

26. **Every posting is badged by its account's TYPE, not the rule by its
    main account.** The plan's C12 badged a rule card's header with the
    `line_section` of each posting (income/expense/asset), which reads a
    rule off the posting that is not its counter: Patrick's deferred-revenue
    rule (`liabilities:deferred-revenue` up, `revenue:sales` down) showed
    only "expense". Now each posting line, on every kind of card, carries a
    badge for `accountTypeOf(account)`: asset, cash, liability, equity,
    income (revenue), expense, gain, conversion. The counter-account line is
    badged too, explicit or default, and a default's badge is dimmed like
    its text. For an `=` rule, the matched account (in the sentence) and
    each generated posting's account are badged. An account with no type
    gets a neutral `?` whose title says how to declare one. A blank account
    gets nothing. The badge sits in a fixed slot (`TYPE_SLOT`,
    `PostingAccount.svelte`), so names line up down the list and the
    amount column (amendment 1) still lines up. Tints: income `--chart-in`,
    expense `--chart-out`, asset info, cash primary, liability warning,
    equity secondary, gain and conversion neutral. Each is text as well.
    - The header keeps only KIND badges: one-off, auto, off, estimated
      (`Badge` is `"oneoff" | "auto"`). A single-date rule is still a one-off.
      An asset rule has no kind badge; its balance annotation marks it.
    - The filter chips follow: All, one chip per account type present, then
      One-off, Auto, Off, Notes. An entry matches a type if ANY of its
      accounts has it, counter included (`Entry.types`), so the
      deferred-revenue rule is under both Income and Liability. A type's URL
      value is its badge word (`?show=liability`). The old `income`,
      `expense` and `asset` values still parse.
    - The types are the journal's with the scenario file's own `type:`
      declarations on top (`scenarioClasses`), as the engine reads a
      scenario (`scenario_account_types`). Before this, the list read only
      the journal's.
    - The dialog's posting rows (rule and one-off) show the same badges, live
      as the account is typed. Each badge leads the row's second line, before
      the posting comment, because in the first line it pushed a rule's row
      past the dialog's width. The `=` rule editor is the shared rule form
      and has none.
    - `resolvedSection` is gone, with its tests. The engine still serves each
      line's `section`, and `ScenarioLine.resolved` still decodes it, so the
      wire key stays checked (`nativeDecode.test.ts`'s TOLERATED list only
      shrinks). Nothing reads it. Dropping it from the engine's wire is a
      follow-up.

27. **A rule or one-off whose postings balance has no default
    counter-account.** Patrick's capex rule (`~ every 6 months`, -3000
    `assets:checking`, +3000 `assets:fixed:computers`) showed a default
    counter it never uses. The engine's `ForecastDefaults::of_under` now
    skips a counter-less group or event whose amounts sum to zero per
    commodity, as `upgrade_legacy_counters` already did. Both call one rule
    (`lines_balance` / `event_balances` over `sums_to_zero`), and the
    expansion still adds no counter posting for them. On the card,
    `Entry.counter` is null when the counter is null and either the postings
    balance as written (`linesBalance` / `eventBalances` in
    `scenarioModel`, signed as the file will hold them) or the engine ran
    the enabled entry and named no default. Then no `→` line is drawn.
    Before the first run, an unbalanced rule's line reads "the default cash
    account". In the dialog, a balanced draft's Counter-account box has the
    placeholder "none needed" and says "None needed: the postings balance on
    their own." Done is not blocked, and a save writes no counter: the
    upgrade skips a balanced group. Once an edit unbalances the postings,
    the box offers the default again, and the server's "Writes:" render
    shows what is written.

28. **"Move posting to a new rule" is removed** (amendment 4, plan C3's
    "To give one posting a different counter, use Move posting to a new
    rule", and the dialog mockup's button). Patrick found it confusing. The
    `⇲` button, the dialog's `movePosting`, `entries.movePostingToNewRule`
    and their tests are deleted. A posting that needs a different
    counter-account goes in a rule of its own, made with **+ Add** or
    **Duplicate**. `docs/projections/cash.md` says so, in place of the old
    table's automatic split.

29. **Amounts are shown and typed as the file writes them** (Patrick,
    2026-10-29). This replaces the plan's C3 rule (and plan 26 B6 on this
    path) that a recurring or asset posting takes a positive magnitude and
    the engine signs it by account type. Revenue reads `−$12,000.00` on the
    card and `-12000.00` in the dialog. The capex rule reads
    `assets:checking −$3,000.00` / `assets:fixed:computers $3,000.00`. The
    bug that prompted it: both capex accounts are assets, so editing the
    checking amount sent `entry: 3000`, the engine signed it positive, and
    the rule stopped balancing.
    - The rule dialog's amount, contribution and balance boxes show the
      signed figure and accept a leading `-`, or the `−` the list draws
      (`parseSignedInput`). The boxes drop `inputmode="decimal"`, whose
      phone keypad has no minus. A typed figure becomes the line's
      `amount.quantity` (`withQuantity`), and the wire carries it with no
      `entry`, so the engine writes exactly what was typed. Changing a
      posting's account leaves its amount alone. One-offs already took
      signed amounts. Both editors now say "Amounts [per period,] as the
      journal writes them: a minus moves money out of an account."
    - Removed: `magnitudeOf`, `withMagnitude`, `withAccount`,
      `signedAmount`, `isRevenueAccount`, `ScenarioLine.entry` and
      `WireScenarioLineIn.entry`. `linesBalance` and `lineAmountText` take
      no account types any more. `ScenarioLine.served` (client-only, never
      sent) keeps the engine's figure once a line is edited, so
      `noteIsStale` still flags a seeded "average over …" description whose
      amount has changed.
    - The server still accepts `entry` and signs it (`ScenarioLineIn.entry`,
      `signed_entry`). No web code sends it now, but it is a documented
      wire field and other clients (FFI, older SPAs) may. Nothing in Rust
      changed.
    - Seeded lines need no conversion: the seed's averages already carry
      the journal's sign.
    - The list draws a negative amount in the neutral text colour, not
      `text-error`. A minus is a direction here, not a warning. The minus
      glyph is still U+2212.
    - Unchanged: the automatic-posting (`=` rule) form keeps its own rule.
      A percentage there means "this account grows by this much", signed
      by the accounts' types (`docs/projections/file-format.md`, "Type
      positive figures").
