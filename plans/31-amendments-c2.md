# 31 — Contract amendments made during Phase C2

These belong under "Contract amendments made during implementation" in
`plans/31-projections-editor-and-chart-tweaks.md`, after Phase C1's
(`plans/31-amendments-c1.md`). Fold them in when the branches meet.

1. **The old table is kept safe by the store, not the decoder.** The
   decoders are faithful. `decodeScenario` returns every entry, and a
   disabled one stays in `lines`/`events`/`autoRules` with its flag on its
   layout item. That is the shape Phase C3 consumes, unchanged. The table
   from before plan 31 is protected in two places:
   - `scenarioStore.adopt`, the one point where a scenario enters it, holds
     `activeScenario(next)`. That is the TypeScript mirror of
     `Scenario::active`: every disabled entry and its layout item dropped.
   - `projectable()` applies `activeScenario` too, so a run body never
     carries one (C7 asks for this anyway).

   `scenarioToWire` sends no `layout`, `comments` or `fixed`. The table's
   save therefore takes `legacy_layout`, which restores every disabled
   entry, comment and piece of file text from the file. C3 removes the one
   `activeScenario` call in `adopt`.

2. **A seed is sent with a derived layout.** `WireScenario::new` gives a
   layout-less scenario the layout a save would derive for it
   (`legacy_layout` over `ProjectionDoc::empty`): every entry new
   (`origin: null`, `raw: ""`), in seed order. Keyless `=` rules get
   `auto:in:N` there. Every scenario the server sends now carries a complete
   layout, so C3 never has to derive one itself.

3. **One-off comments travel per posting.** The plan's "events gain the
   same" is spelled like this:
   - The event's header comment is `WireScenarioEvent.comment`.
   - Each posting's comment is `WireEventPosting.comment`, on the first wire
     posting of a flattened multi-amount posting, which is where a save
     writes it.
   - Inbound, `EventPostingIn.comment` builds `posting_comments` index for
     index, with trailing empty ones dropped as a read drops them.
   - In TypeScript these are `ScenarioEvent.comment` and
     `EventPosting.comment`.

4. **The inbound comment piece has `previous`, and accepts its own echo.**
   - Edited: `{key, style, text, previous?}`. `previous` is the `raw` the
     comment was served with. An edited section keeps that text's dividers
     and framing (`comment_raw`'s `previous`). The server takes it from the
     body rather than looking it up in the file, so a save and a render
     write the same text.
   - Unchanged: `{key, raw, style?}`. A `style` given with `raw` must be the
     style `raw` reads as.
   - The outbound `{key, style, raw, text}` handed straight back counts as
     unchanged when `text` is exactly what `raw` displays as. Any other mix
     of `raw` and `text` is refused, because it cannot say which one the
     client means.

5. **Echoed derived fields are accepted and dropped.**
   - `LayoutItemIn` takes `raw`, and `FixedIn` takes `raw`, and both drop it:
     what a save writes is never the client's to say.
   - `attached` and `attachedText` are mutually exclusive.
   - Every new inbound struct keeps `deny_unknown_fields`.

6. **`=` rules are unchanged: exactly one of `raw` and `form`, plus
   `key`.** The outbound `WireAutoRule` carries both `raw` and `form`, so it
   still cannot be handed back verbatim. A client sends
   `{block, key, raw}` or `{block, key, form}`, as `scenarioToWire` always
   has, now with the key. Plan 30's contract stands. Everything else in a
   served scenario can go back as served.

7. **File text's bytes come from the file.** The server fills each
   `FixedPiece.raw` from the file by key (`ProjectionDoc::fixed()`, new)
   before writing. A client's `raw` is ignored, and the writer writes the
   claimed piece's own bytes in any case. A test pins that an echoed
   `include /etc/passwd` never reaches the file.

8. **Where the C8 validation lives, and its limits.**
   - The check is core's new `check_layout`: every key names an entry
     exactly once, every line group, one-off, `=` rule and comment is named,
     and only entries are turned off or carry an attached comment. It runs
     on `run` and `save` bodies after decoding, and its sentences are the
     writer's own.
   - The fixed-text order needs the file, so only a save checks it.
   - Limits, all `400` with a sentence:
     - `MAX_COMMENTS = 1000` comment pieces.
     - `MAX_COMMENT_BYTES = 64 KiB` per comment string: a piece's
       `raw`/`text`/`previous`, `attached`/`attachedText`, and a line's,
       rule's, one-off's or posting's comment.
     - No control character in any of them other than `\n`, `\r` and `\t`.
   - Also added, beyond the plan: `MAX_FIXED = 1000`, and
     `MAX_LAYOUT_ITEMS` (the sum of every per-kind maximum), so a layout
     cannot outgrow what it could ever name.

9. **What `render_entry` takes.** Its signature is the plan's. Beyond that:
   - The scenario may hold other entries, which are not rendered. A step
     change's `line:` value is decided over every line the body holds, as a
     save decides it over the whole scenario. So a client sends the whole
     scenario plus the entry's one layout item (`renderBody` does). With the
     entry alone, a step segment renders without its tag, which is exactly
     what a save of that entry alone would write.
   - With no layout item, the kind comes from the key, and the entry
     renders enabled and with no attached comment.
   - File text is refused, since a save never renders it.
   - The body is `ProjectionDoc::body(item, entry, print, None, tags)`.
     That is `apply`'s path for a piece it does not claim, so nothing had to
     be factored out of `apply`. `Entries::of` was split into `index` and
     `check_named`, so a one-item render need not name every entry.

10. **A render prepares the body exactly as a save does,** through the
    shared `writable()` step: signed, forms rendered, then plan 28's
    `upgrade_legacy_counters`. So a pre-plan-28 rule that does not balance
    previews with its default counter-account and plain postings, which is
    what its save writes.
    - A refusal's `400` body is the writer's sentence alone. A save prefixes
      it with "`<id>` cannot be written as you asked:", and a render has no
      id.
    - `NotRoundTripped` is a `500`, as for a save.

11. **`upgrade_legacy_counters` skips disabled entries** (core,
    `forecast.rs`). A disabled entry posts nothing, so it has no default to
    make explicit, and upgrading it would rewrite a commented-out block the
    user never touched once C3 sends layouts.

12. **A run's typo warnings are over the active scenario.**
    `auto_rule_account_warnings` gets `scenario.active()`, so a disabled `=`
    rule's accounts raise no "unknown account" warning. This was a leak that
    surfaced only once disabled rules travelled.

13. **The TypeScript API follows the codebase's conventions.**
    - `renderProjectionEntry(body): Promise<unknown>`, decoded by
      `decodeRenderedEntry` into `RenderedEntry {text}`, as every other call
      here is (the plan said `Promise<{text}>`). Its `404` is
      `NativeApiUnavailableError`, as for `runProjection`, and its `400` is a
      `ValidationError` carrying the writer's sentence.
    - Added to `scenarioModel`, for C3 and the live test:
      - `activeScenario`;
      - `renderBody(scenario, key, item?)`, where `item` is a draft's
        `WireLayoutItemIn` overriding the scenario's own.

14. **Core API added for the wire.**
    - `read_scenario_file -> ScenarioRead {scenario, own_types,
      piece_texts}`: one parse gives a `GET` its scenario and every item's
      `raw`.
    - `check_layout`, `comment_style`, `render_entry`.
    - `ProjectionDoc::fixed`, `ItemKind::parse`.

15. **Tests whose bodies changed.**
    - `a_save_with_an_edited_form_rewrites_only_that_rule_s_block` and
      `a_save_deletes_and_appends_rules_by_block` edit `autoRules` by
      position with no keys, so they now send the legacy body (no
      `layout`/`comments`/`fixed`).
    - `a_save_rewrites_only_the_rule_it_changed` runs twice: with the legacy
      body and with the served layout.

    Those are the plan's "port three save tests". In addition:
    - The PAYE test hands the layout back, so its edited rule now names its
      key.
    - `fixtures/native/v1/requests.tsv` still lists only GETs, so there is
      no `projections-render` golden. The live integration case is the
      cross-language check.
    - `nativeDecode.test.ts`'s `SCENARIO_FILE` literal gained a disabled
      item, a section, a piece of file text and the comment fields, because
      no golden carries them and the rename sweep must reach their keys.

16. **One line in `ProjectionsTable.svelte`.** "Add a posting" builds its
    posting with `comment: ""`, because `EventPosting.comment` is now
    required. Its behaviour is unchanged.
