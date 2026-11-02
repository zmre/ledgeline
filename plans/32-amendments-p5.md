# 32 — Phase 5 contract amendments

These belong under "Contract amendments made during implementation" in
`32-transaction-review.md`. Fold them in when the branches meet. Phase 5 is the
docs, the screenshots, and the `%` gap from amendment 26 of
`32-amendments-p4.md`.

1. **`%` in a rule's value: no escape exists, so the builder warns.**
   - Checked against hledger 1.52 on a one-row rules file plus CSV:
     - `%desc` → the field; `%nosuch`, `%off`, `%Ü`, `%_x`, `%-x` → nothing;
     - `%%desc` → `%` followed by the field, so `%%` is no escape;
     - `\%desc` → `\` followed by the field, so a backslash is no escape;
     - `50% off`, `100%`, `%.x`, `%(desc)`, `%{desc}` → kept literally.
   - So a reference is `%` followed by a letter, digit, `_` or `-` (Unicode
     letters included). `ruleBuilder.ts` gains `fieldReferences(value)`.
   - `RuleBuilder.svelte` shows `review-rule-percent` under "Then set" while a
     TICKED line holds a reference. It doesn't block the save. An unticked line
     (the description beside an account change starts unticked) warns nothing.
   - The live suite (`importReviewLive.integration.test.ts`) previews rules
     against hledger and pins the substitution, including `%%`. If hledger
     ever gains an escape, that test fails, and the builder should escape
     instead of warning.
   - `docs/imports/new-transactions.md` documents it.
2. **The docs harness takes a clock.** `harness.ts` gains a `now` option
   (default FIXED_NOW, 2026-07-08), and the Projections date pin follows it.
   `review.docs.ts` sets 2026-10-30, the review fixture's TODAY.
3. **`finishMotion(page)` in the harness.** It finishes every finite animation
   over three frames, then waits until no `[data-review-card][inert]` (an
   outgoing card) is left. It is the page-side twin of the overlay's own
   `finishMotion`, which the bundle doesn't expose.
4. **Shot list as built.**
   - `review-editor-badge.png` is built (the brief marked it optional). It is
     the top 150px of the dialog for #17, "Reviewed Sep 2".
   - The card shots use a 1280×1300 viewport, so the card with Similar open
     never scrolls inside itself. The import shots use 1280×1600.
   - The review shots clip the top bar plus the deck, extended 34px down,
     because the peeks' 14px and 28px offsets are transforms the deck's box
     doesn't include.
   - `review-before-write.png` shows the strip before any review edit, so it
     has no edit-count line. Getting one would mean saving a staged card. That
     writes nothing to disk, but it is a Save, and the rule is that no spec
     presses one.
5. **Existing shots change.** `journal.docs.ts` runs on the sample set, whose
   rows now carry stripes and whose toolbar has the review buttons, and the `?`
   sheet lists `u`/`r`. `overview.png` and `keyboard-help.png` will differ the
   next time the pipeline runs. That is Phase 3's UI, not a regression.
6. **Doc placement.** `docs/journal/review.md` has `order: 15`, between the
   journal page and Problems. The keyboard page's Review table is the
   overlay's `registerKeys`, row for row, plus Tab (handled in the account
   field, not the keymap). `u`/`r` are added to the Journal and Imports tables.

7. **A Recents run never grows** (Patrick, 2026-10-31: a 50-card run
    showed "60 of 90"). This corrects p2 amendment 10's "new queue items are
    appended", which now applies to Unknowns only.
    - **Why it grew.** The background re-validation after each save re-read
      the server's top N unreviewed transactions. The one just reviewed had
      left that list, so the next one moved into it and was appended: one
      extra card per save.
    - **Now.** Recents is fixed when the run opens. Re-validation still
      refreshes changed cards and drops departed ones. It re-reads N plus the
      run's length, so newer arrivals can't push a held card out and make it
      look gone.
    - Unknowns keeps appending, because that queue has no count limit.
    - **Test.** `review.save.svelte.test.ts`: "never grows a Recents run".
