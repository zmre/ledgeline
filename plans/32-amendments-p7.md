# 32 — Phase 7a contract amendments

These belong under "Contract amendments made during implementation" in
`32-transaction-review.md`. Fold them in when the branches meet. Phase 7a is
the engine and the wire for D33–D36. Phases 7b and 7c (the UI) build on them.

1. **Where elision lives (D35).**
   - `model::Posting` gains `elided: bool`. The parser's `finalize_posting`
     sets it from the raw posting (`amount.is_none()`), so a posting whose only
     content after the account is an assertion (`assets:cash  = $99.00`)
     counts too. Postings that are not read from a file (request bodies, the
     QuickBooks builder, forecasts, auto postings) say `false`; a body states
     elision with an empty `amounts`.
   - The source-text scan the July fix used (`parse::posting_line_elides_amount`
     and `edit::elided_postings`) is gone: the model flag is the one notion.
2. **Elision on the wire.**
   - The native row (`view::rows::RowPosting`, so `/api/journal/rows`,
     `/api/transactions/{index}`, every review route and the import's staged
     rows) carries `"elided": true` on an elided posting and nothing otherwise.
     Additive: the inferred `amounts` are still served, and an older client
     reads exactly what it read before.
   - The hledger-shaped `/transactions` keeps `poriginal: null`. Checked
     against `hledger print -O json` (1.52): hledger itself emits `null` for a
     balancer-inferred amount, and the parity suites compare `/transactions`
     key for key with hledger's output. The plan's "`poriginal` instead of the
     hard-coded `None`" is therefore not done.
   - `normalize.ts` (the hledger-wire normaliser) is unchanged. The SPA's
     transactions come from the native rows, so the decoder that keeps the
     marker is `journalViewDecode.ts` `decodePosting` (`Posting.elided?: true`).
3. **`Replace` writes what it is given.** `replace_transaction` and
   `EditOp::Replace` no longer re-blank legs the file elided
   (`restore_elisions`). A posting with no amount is written blank; one with an
   amount is written with it, even on a leg the file used to leave blank. That
   is the D35 rule: an amount the user typed is written.
   - `JournalEditor::with_elisions_restored(index, txn)` keeps the old
     behaviour for a caller holding a fully explicit copy. The QuickBooks
     update path (`edit_api::apply_replacements`) uses it, so it behaves as
     before.
   - **The consequence:** a client that does not read `elided` and sends the
     inferred values back now writes them out. The SPA ships inside the engine,
     so no such client exists; this note is for anyone scripting the API.
4. **An assertion on a blank leg is accepted.**
   - The server's `build_assertion` used to refuse it ("cannot be written on
     the inferred (elided) leg"), from when the formatter dropped it. The
     formatter has written `account  = AMT` since July, and an elided leg with
     an assertion now goes back that way.
   - `validateForm` refuses a blank amount with an assertion only on a row
     that is NOT `elided`, so blanking an explicit asserted posting is still
     refused. `formToBody` sends the assertion on an elided row.
   - `error_surface.rs` `assertion_on_the_elided_leg_is_a_400` became
     `…_is_written`, and the edit-endpoint case moved to
     `an_assertion_on_the_elided_leg_round_trips`.
   - **hledger's reading.** hledger treats an amountless posting with an
     assertion as a *balance assignment*: it computes the amount from the
     account's running balance, while this parser infers it from the
     transaction. The two agree whenever the journal is consistent, which is
     the only case either loads without an error. `removePostingAssertion` on
     such a leg turns an assignment into a plain inferred leg.
5. **Comments keep a leading line break.** `edit_api::comment_string` and
   `formToBody` trim only the end of a comment. A comment written on its own
   `;` line is served as `"\nnote"`, and a re-render used to hoist it onto the
   header or posting line.
6. **`SetTxnComment` covers the whole comment.** The transaction comment is
   the header's same-line comment plus the `;` lines above the first posting.
   Its free text is, per line, the comma segments that are not tags, joined
   `", "`; lines with none are not part of it (`edit::comment_free_text`, TS
   `commentFreeText`, which now returns every line joined by `\n`).
   - **In:** the text, one line per line. `\r\n` counts as a line break (CRLF
     input). Each line is trimmed and its commas are normalised (`a,b` is
     written `a, b`). Blank lines are dropped. A line that reads as a tag, or
     a bare `\r`, is `Unsupported`.
   - **Out:**
     - the lines that have free text are rewritten in order, each keeping its
       tags after the text, its `;` and the spacing after the `;`;
     - a line whose free text already reads as asked is left byte for byte;
     - extra lines become new `; text` lines after the last comment line,
       indented like the other `;` lines (or the first posting), with the
       file's own terminator;
     - a line whose text is no longer wanted keeps only its tags, and with
       none it goes (a continuation line is deleted, the header drops `;` and
       the gap);
     - with no free text anywhere yet, line 1 joins the header comment ahead
       of its tags.
   - The single-line behaviour of Phase 1 is the one-line case, and its tests
     pass unchanged.
   - Over HTTP the comment may hold `\n` and `\t` (`check_text`'s `COMMENT`
     set), but not `\r`, as for every other comment field. The core accepts
     `\r\n` for callers in-process.
7. **`SetPostingComment { posting, comment: Option }` is verbatim and
   positional.** Tags are allowed (a `date:` tag sets the posting date). Line 0
   is the same-line comment (empty: none); each later line is one `;` line
   under the posting (an empty middle line is a bare `;`). Trailing blank
   lines are dropped, and `None` or only blank lines removes it all.
   - Existing lines are rewritten in place, keeping the indentation and the
     spacing after the `;`. A line that already reads as asked is untouched.
   - New lines go after the last one, indented like the first continuation
     line, else like the posting.
   - A posting that is not there is `PostingNotFound`, a `404` over HTTP, as
     for `setPostingAccount`.
8. **The header ops share one rewrite.** `rewrite_description` became
   `rewrite_header(text, model, token)`, used by `SetDescription`, `SetDate`,
   `SetDate2`, `SetStatus` and `SetCode`.
   - The part before the comment is rebuilt as `set_status` rebuilds it
     (canonical single spaces). The comment and the gap before it are kept
     byte for byte.
   - The date token is the line's own bytes unless a date op changes it.
   - `SetDate`/`SetDate2` write ISO digits in the separator the header's
     primary date uses (`/`, `.` or `-`). A yearless date becomes a full one.
   - The standalone `set_status` and `set_description` (the PATCH route) are
     unchanged and still re-render the comment as `"  ; "`. D34 asked for
     `SetStatus` "wrapping `set_status`"; it shares the rebuild instead,
     because a batch op cannot commit on its own.
9. **What the header ops refuse (`Unsupported`, `400 transaction #N: …`).**
   - A date that is not exactly `YYYY-MM-DD` and a real calendar day
     (`parse::is_iso_date`, PARSE-6's check).
   - A code that is empty, padded with spaces, or holds `)`, `;` or a line
     break. Checked against hledger 1.52: a code may hold spaces and `(`
     (`(a b)`, `(a(b)`), ends at the first `)`, and `( x )` keeps its spaces.
     hledger also accepts `(a;b)`, but this parser cuts the header comment at
     the first `;`, so it is refused.
   - Removing a secondary date or a code that is not there is a no-op. So is
     `removePostingAssertion` on a posting with no assertion.
   - The read-back guard still applies. Removing the code from
     `(1) (note) Payee` would re-read `note` as the code, and it is refused as
     `RoundTripMismatch`.
10. **The surgical read-back is stricter.** `Expected::check` now compares
    the header (date, date2, status, code, description) against the MODEL the
    ops built, not the original. Per posting it compares the account, type,
    status, amounts, `elided`, comment and assertion (by value). Before, it
    ignored assertions.
11. **`suggest_for_description` is a method.** It is
    `ReviewIndex::suggest_for_description(&self, journal, classes,
    description)`, like `suggest` and `suggest_staged` (p1 amendment 1),
    rather than a free function.
    - Sources 1 and 3 run as for `suggest`, with nothing excluded. They were
      extracted as `ReviewIndex::precedent`, which `suggest_inner` now calls.
    - Source 4 can't match by amount and day before there is either. It
      matches the detected subscriptions whose `payee` has the description's
      similarity key, at the same flat 60% (split).
    - Each such suggestion carries `recurring: Option<RecurringFill>
      {quantity, commodity, account}`:
      - the quantity is the subscription's `typical_amount`;
      - the commodity is the one the newest such charge spent in;
      - the account is the newest charge's funding account among
        `sub.funding`.
    - `rank` keeps a fill when another source wins the account. Usually the
      exact key does, at 100%.
    - `spend` now returns the commodity with the total.
12. **`GET /api/review/suggest?description&today`** answers `{generation,
    key, suggestions}`.
    - `key` is the description's similarity key, or `null`, and then there are
      no suggestions.
    - A suggestion is the queue item's shape, plus `recurring: {amount:
      {commodity, quantity: {mantissa, places}}, account}` when a recurring
      charge backs it. The field is left off otherwise, so queue bodies are
      byte-identical to before.
    - A description over 1 KB is a `400`. `today` is validated and keys the
      ETag, as on the other review GETs; nothing else reads it.
13. **Adding a reviewed transaction is already one write (D36).** `POST
    /api/transactions` carries the stamp in its `comment` (`"family plan,
    reviewed:2026-10-30"`), which the header writes as `  ; …`. Nothing was
    added; `review_endpoints.rs` `adding_a_reviewed_transaction_is_one_post`
    pins it. 7c composes the comment.
14. **TS, as built.**
    - `WireEditOp` gains `setDate`, `setDate2` (`null` removes), `setStatus`,
      `setCode` (`null` removes), `setPostingComment` (`null` removes) and
      `removePostingAssertion`.
    - `ReviewForm` gains `date2` (`""` none), `status` and `code` (`""` none).
      Its `comment` is the multi-line free text. `reviewFormOf` fills them from
      the transaction.
    - `PostingForm` gains optional `elided` and `inferred`:
      - `txnToForm` leaves an elided leg's `amount` `""`;
      - `inferred` is the served value as input text, for a placeholder (`""`
        for a mixed amount);
      - an engine without the marker still prefills the amount.
    - `formToOps` writes one op per changed field: `setDate`, `setDate2`,
      `setStatus`, `setCode`, `setDescription`, then per row
      `setPostingAccount`, `setPostingComment`, `removePostingAssertion`, then
      tags and `setTxnComment`.
    - `isSplit` (when to `replace`) is now: an amount, a commodity, a
      posting's status, type or cost, an assertion added or changed (not
      removed), or a posting added or removed. A date, a posting comment or a
      removed assertion is no longer a split, so similar-apply still offers
      the account change.
    - A `replace` body takes the FORM's date2, status and code, with the
      original comment. The tag and comment ops follow it.
    - New: `decodeDescriptionSuggestions` (`reviewDecode.ts`), the
      `Suggestion.recurring?` / `RecurringFill` / `DescriptionSuggestions`
      types, `LedgelineApi.reviewSuggest(description, today)`, and the
      exported `nativeDecode.decodeDec`.
15. **The import's override logic knows the new ops.** `imports/reviewModel.ts`
    (`slot`, `isNoOp`, `differences`) and `imports/stagedReview.ts`
    (`describeOps`) switch over every op, so they had to.
    - Date, date2, status, code, posting comment and a removed assertion are
      now slots of their own.
    - An assertion added or changed is still `other`.
16. **Goldens.**
    - No route in `fixtures/native/v1/` serializes a posting, so no native
      golden byte changed.
    - The rows golden (`fixtures/journal-view/rows.json`) was captured from
      the old TypeScript and says "do not regenerate from Rust".
      `journal_view_golden.rs` now takes `elided` off each posting before
      comparing, asserts it matches `Posting::elided`, and requires more than
      100 elided postings across the corpus.
17. **For 7b: the card's comment box must become the textarea before 7b
    ships.** `ReviewFields.svelte` still binds `form.comment` to an
    `<input type="text">`.
    - A browser text input strips line breaks from its value. On a
      transaction whose comment has free text on two lines, an edit in that
      box would merge them and delete the second line.
    - D33's growing textarea fixes this. Until then it can only hit a comment
      with free text on a `;` continuation line. The fixtures' continuation
      lines hold tags only.
    - 7a changed only the `ReviewForm` literal in `ReviewOverlay.svelte`, and
      built no UI.
18. **For 7b/7c: the elided leg now shows blank** in both the modal and the
    card, with no hint of the inferred value. `PostingForm.inferred` is there
    to bind a placeholder.

# Phase 7b — the complete card

19. **The comment boxes are growing textareas** (`review/ui/GrowingTextarea.svelte`),
    fixing amendment 17.
    - It starts one row tall, at the matching input's height, grows to six
      lines and then scrolls. Enter types a newline. ⌘↵ saves and Esc leaves
      the field, because the overlay catches both on its root.
    - **The posting comment is a textarea too.** It is the posting's whole
      comment (amendment 7), so a comment over `;` lines under the posting
      would have hit the same data-loss bug in an input.
    - `focusField` now finds a textarea as well as an input, so `m` still
      works.
20. **The details row lives in the hero** (`ReviewDetails.svelte`, and a
    `details` snippet plus `detailsBlock` on `ReviewHero`).
    - **Collapsed:** the hero's date line on the top card is the summary
      button, "Oct 12, 2026 · secondary Oct 14, 2026 · cleared · #1042 ▾",
      then the funding account. It replaces the plain date and status the
      hero showed, so a queue card gains no line. The summary reads the FORM,
      so an edit shows. `card.ts` `detailsSummary` builds it.
    - **Expanded:** the funding account keeps the date line, and the editor
      gets a line of its own under it: `D` date, `!` status pill, `#` code,
      then the secondary date (with ×) or "+ secondary date", and a ▴ to fold
      it back.
    - **Opening and closing.** Click, `D` (date focused), `#` (code focused)
      and `!` open it. It stays open while `detailsDirty` (date, date2, status
      or code changed), and ▴ is offered only when nothing in it changed. It
      folds back on every new card and on Revert.
    - **The ⋯ menu.** "Edit date" is gone. The menu has "Add secondary date"
      instead, shown while the card has none. It opens the row with the field
      focused.
    - **Fields.** Both dates are `DateInput` (WebKit's empty-date bug). The
      status pill cycles with `journal/rowModel.ts` `nextStatus`, the
      Journal's own cycle.
    - Peek heroes are unchanged.
21. **Keys.** These are card mode only, in the "Review" group of the `?`
    sheet:
    - `D`: "Edit the date (and the other details)";
    - `!`: "Cycle the status: unmarked, pending, cleared";
    - `#`: "Edit the code".

    None collides with the card's bindings. `d`, `1`…`9` and `$` are
    distinct keys under chord.ts's case-sensitive printable rule. `$` still
    lands on the amount, not the new commodity field in front of it: the
    amount input carries `data-review-focus`, which `focusField` prefers.
    `docs/journal/keyboard.md` lists the three keys.
22. **Commodity** (`CommodityInput.svelte`, `complete.ts` `rankCommodities`).
    - It is a borderless prefix field inside the amount box, as wide as its
      text, using the same combobox pattern as `DescriptionInput`.
    - **The list.** The SPA has no commodity list of its own, so the overlay
      passes `journal.dominantCommodity`, then the card's own commodities,
      then the keys of `journal.reportStyles`, which holds every posted
      commodity. Ranking puts prefix matches first, then substring matches,
      ignoring case.
    - The amount box grew from `w-32` to `w-40`.
    - **Inert on a blank leg.** `isSplit` compares a commodity only on a row
      with an amount, so changing the commodity of a leg left blank sends
      nothing. That is 7a's rule, kept.
23. **Assertion chip.** The chip is `== $500.00` with a ×.
    - `assertionLabel` moved from `TransactionModal.svelte` to
      `editMapping.ts`. The modal imports it until 7c deletes it.
    - The rows keep a `w-32` slot for the chip whenever the transaction has
      an assertion, so the amounts stay in one column and removing one
      doesn't shift the row.
    - The "kept as written" note under a row now names only cost and posting
      type.
24. **Posting comment.** ⋯ is a single button ("Add a comment to posting N"),
    not a menu. It is invisible, though it keeps its space, on a row that
    shows a comment line.
    - The line is a muted `;` and a ghost textarea under the row.
    - Once focused it stays up even if cleared, so clearing a comment doesn't
      pull the field away mid-edit.
25. **The blank leg's placeholder goes beyond amendment 18**
    (`card.ts` `balancingHint`).
    - A served `inferred` goes stale as soon as the other amount is edited.
      So the hint is worked out from the form when it can be:
      - the blank row is the only blank row with an account;
      - every other row is a plain regular amount in the blank row's
        commodity.
    - The hint keeps at least the precision the engine showed (`7.5` against
      a `$6.75` leg reads `-7.50`).
    - When the form can't work it out (a cost, mixed commodities), the hint
      falls back to the served `inferred`, but only while no other row's
      amount or commodity has changed. Otherwise it is plain "auto".
    - It applies to any blank row, an added one included. The field itself
      always stays blank.
26. **`formToOps` needed no new wiring.** Each D33 field maps to its own op
    as 7a built it. The parity matrix pins the exact batch for each one;
    Replace happens only for an amount, a commodity, or a posting added or
    removed.
27. **Docs.** `docs/journal/review.md` described the removed ⋯ → Edit date
    and said assertions were only "kept as written". Both sentences are
    corrected, and the card's new fields are described in brief. 7d still
    owns the rewrite and the screenshots.

# Phase 7c — single mode, add, delete, and the switch

28. **A new route, `GET /api/review/item/{index}?today` → `{generation, item}`.**
    D31 said the card opens on one `GET /api/transactions/{index}` and loads
    the suggestions after it. That route has no fingerprint, and a single
    card's save is a batch, which needs one. The new route answers the queue
    item's shape (row, fingerprint, key and `review::suggest` suggestions),
    whether or not a queue holds the transaction, read under the editor lock
    like the queue (`queue_items` is shared with it). A missing index is a
    `404`.
    - **So the suggestions come with the card, not after it.** They are the
      same ones a queue card gets (D38: "the chips show"), computed in the
      same request. The similar set and rules-for still load after the card
      lands, and the card is editable before they do.
    - Tests: three in `review_endpoints.rs`, one live
      (`reviewLive.integration.test.ts`), TS `decodeReviewItem` and
      `LedgelineApi.reviewItem`.
29. **Single runs live in the review store** (`review.openEdit(index)`,
    `review.openAdd(blank)`). `Run.single` is `"edit" | "add" | null`, and
    `review.kind` is the mode for a single run. Nothing pages, revalidates or
    follows the generation. A single save is not optimistic: `saveSingle`,
    `addSingle` and `deleteSingle` mark the card `saving`, write through
    `editing` (new `editing.batch(edits)`; `editing.add` now answers the new
    index), and answer the transaction's index afterwards.
    - **Conflicts.** Going through `editing` is what keeps D39's 409
      behaviour: `editing.conflict` is set, the journal refreshes, and the
      page's banner shows. The card closes, as the old modal did. Any other
      failure stays on the card as `review-failure`, edits intact.
30. **The page mounts the overlay for the editor.** `modalState` keeps
    `openAdd()` / `openEdit(index)` / `close()` and `open`, `mode`,
    `targetIndex`; `target` and `loadError` are gone (the store fetches). A
    `nonce` keys the mount. `routes/+page.svelte` renders `ReviewOverlay
    kind={txnModal.mode} index={…}` when no review run is open. The card's
    close calls `txnModal.close()`.
31. **The cursor after a write.** The old modal's add did NOT move the
    journal's cursor: it closed, and the table's position-anchored cursor
    stayed on whatever row it was on. The card now hands the new index to
    `reviewLaunch.returnTo` after an add, and the edited one after an edit,
    so the table scrolls to it and puts its cursor there, as a review run's
    close does. A delete hands nothing back, so the cursor stays at its
    position, as after the table's `x`.
32. **The checkbox is a view over the form's tag chips** (`card.ts`
    `reviewedBox`, `toggleReviewed`). Checking adds `reviewed:<today>`, or
    puts back the tag the transaction had (its date, or `ignore`), so
    unchecking and checking again is clean. A save writes whatever the chips
    say: `cardOps` is called with `reviewedOn: null`, so ⌘⇧↵ never forces
    the stamp. A reviewed card's `reviewed` chip stays visible among the tag
    chips; both edit the same tag.
    - **Tooltip.** The queue card's "Categorise the unknown posting first — or
      Skip it for now" lost its second half: single mode has no Skip.
33. **Footer** (`SingleFooter.svelte`, `data-state="single"`): Save (Add on
    an add card), disabled until the card is dirty; the checkbox with its
    `v`; Revert once dirty. The delete confirm and the dirty-close bar take
    the footer's place, as the close bar already did.
34. **Validation.** `validateForm` runs before any save, queue cards and
    imports included (`review-errors`, under the fields). Before this a
    queue card would save "five" in an amount as an elided leg. The messages
    follow the edits once a save has been tried.
35. **Keys (D40).** Single mode registers `v` and, on an edit, `X`; it does
    not register `y`, `Y`, `→ n l`, `← p h` or `i`. `o` and "Open in full
    editor" are gone from queue runs too. `?` shows the single-mode labels
    ("Save", "Accept the suggestion and save", "Mark reviewed, or not",
    "Delete the transaction (press again to confirm)"). `X` works while its
    confirm is up, so a second `X` deletes; Escape disarms it.
36. **Menu.** Single mode's ⋯ has "Add secondary date" and, on an edit,
    "Delete transaction ⇧X". Ignore and Settings are queue-only.
37. **Add mode (D36).**
    - The card is `blankItem(today)`, a transaction with index -1 and no
      fingerprint; the form is `blankReviewForm` (two rows in the journal's
      main commodity), and dirtiness is against it (`addDirty`).
    - The hero follows the description as it is typed; its amount reads "—".
    - `DescriptionInput` has an `onPick`. Picking a payee asks
      `GET /api/review/suggest` (`review.suggestFor`, latest request wins)
      and the strong chip is selected. Chips target row 1 (`addChips`).
    - Accepting a chip (`acceptAddChip`) sets row 1's account and, with a
      `recurring` fill, its amount and commodity, plus the funding account on
      the first other row with no account. Only blank fields are filled. The
      chip reads "streaming · $15.49 from liabilities:visa"
      (`recurringLabel`).
    - The comment is composed by `editMapping.addComment`: the first free-text
      line, then every tag chip, on the header line; later lines after it.
      So a checked box is `reviewed:<today>` in the one POST.
38. **Motion.** `motion.ts` `dialogIn` (scale 0.96 → 1 and a fade, 140ms;
    reduced motion is the usual 120ms fade). After a save, a ✓ badge
    (`review-flash`, `stampPop`) shows for `DURATIONS.flash` (260ms), then the
    layer closes with the existing `layerOut`. Keys are off meanwhile.
39. **Elided commodity (7b gap, amendment 22).** On an elided leg whose
    amount is blank the commodity field is disabled, with the title "Type an
    amount to set its commodity" (`CommodityInput` gained `title`). New rows
    are not elided, so their commodity stays editable before an amount.
40. **Deleted.** `TransactionModal.svelte` and its three test files, and
    `keys/ModalKeyLayer.svelte`, which only the modal used. Their behaviour
    tests moved to `ReviewOverlay.single.svelte.test.ts`: focus on open (now
    the description, D36), ⌘↵ saves, Escape in an account combobox closes
    the list and not the editor, the next Escapes walk out, generated
    postings read-only with the rule link, the review badge (now the
    checkbox's states), the modal key layer blinding the journal's keys, and
    validation. Two modal tests had no card equivalent and were dropped:
    "submits on Enter via a native `<form>`" and "every other button is
    `type=button`". The card has no `<form>`; Enter in card mode saves
    through the keymap.
41. **The close button** is labelled "Close the editor" in single mode.
42. **Docs.** `journal/index.md` "The transaction dialog" became "The
    transaction editor" (with "Mark reviewed" and "Suggestions"), and its
    delete section names ⇧X. `keyboard.md` has a "Transaction editor"
    section and lost `o`. `review.md` "Row stripes and the editor badge" is
    now "Row stripes and Mark reviewed"; links moved with it, and
    `automatic-postings.md` points at the editor. Prettier is not run over
    `docs/`: the compact tables are kept.

43. **Two e2e fixes made when 7c's specs were first run.**
    - The editor test presses `!` and `#` as characters. Playwright's
      `"Shift+1"` sends key `1` with Shift held, never `!`, so the
      status binding never fired. A real keyboard sends `!`.
    - The Journal-rules test waits for `review-rule` before `Shift+R` on
      the blue card. `R` exists only once the card's rules-for answer has
      landed, and 7c's single-mode work made that race lose. The Amex step
      already waited this way.
    - The app needed no change for either. Result: `just e2e` 118 passed;
      the docs screenshots were re-shot and reviewed.
