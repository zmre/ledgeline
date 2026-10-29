# 31 — Contract amendments made during Phase C1

These belong under "Contract amendments made during implementation" in
`plans/31-projections-editor-and-chart-tweaks.md`. They are kept here because
the plan file lives in another worktree. Fold them in when the branches meet.

1. **Existing tests whose meaning changed.** The plan asked for these to be
   listed here.
   - "Comments between rules stay where they were." With a layout, an
     attached comment moves with its rule
     (`a_moved_unchanged_block_keeps_its_bytes_and_its_attached_comment`).
     Without one, `legacy_layout` keeps it with the block's slot, as before
     (`rows_are_written_in_scenario_order_and_the_bytes_between_blocks_stay`,
     unchanged).
   - "Rules and events fill separate slot sets" now holds only under
     `legacy_layout`. Its test is unchanged.
   - "A moved chunk is re-rendered" is gone on both paths. Claims are
     content-checked (C9 pass 2), so a moved, unchanged block keeps its bytes
     even under `legacy_layout`.
   - In the serialize unit tests, `read()` now means "the scenario as
     today's wire carries it": the active entries, with no layout, comments,
     fixed text or `=` keys. `read_full()` is the whole read. The tests that
     compared whole scenarios go through `read()`.
     `editing_rules_through_forms_rewrites_only_their_blocks`
     (`tests/projection_files.rs`) clears the layout, because it edits
     `auto_rules` by position the way a layout-less client does.
   - Removing a `~` block now takes its gap with it, as removing an `=`
     block already did. A deletion no longer leaves a double blank line.
     No test asserted the old bytes.

2. **A divider needs three marks.** A divider is a comment line that, with
   its whitespace removed, has 3 or more characters, all from
   `- = _ ~ * # ;`. So a lone `;` or `;;` is NOT a divider. The plan's
   wording ("empty after the comment characters are stripped") would make
   `;` a divider. Then an attached comment with a blank `;` line between two
   paragraphs would read as a section and come off its rule.

3. **`layout::scan(text, header_end, confirm) -> Scan { lead, pieces }`.**
   The scanner stays pure: the caller passes a `confirm` callback that
   parses each disabled candidate and gets its stripped text. A rejected
   candidate stays in its comment run. With a two-phase "scan, then confirm"
   it would have to be merged back in afterwards.

4. **A disabled candidate's body uses the header line's exact prefix.**
   That prefix is the comment character plus an optional space. A
   following line that would extend the body under a *different* prefix
   means the run is mixed, and then nothing in it is a candidate. That is
   the plan's "a mixed `;`/`#` run is never sliced". It is stated this way
   so that `;~ monthly` / `;    posting` reads correctly: by its own prefix,
   the body line's prefix would be `; `.

5. **Fixed text a layout leaves out is kept, not refused.** The writer puts
   each fixed piece the layout does not name back where it is in the file
   (`ProjectionDoc::keep_fixed`). It still refuses a layout that reorders
   two fixed pieces, or that names fixed text the file does not have. The
   reason: the first save of an older file writes `account … ; type: E` and
   `commodity …` declarations under the header. On the next read those are
   a fixed piece, and a client holding its pre-save layout (C10's
   `markSaved` path) does not know about them. Refusing would block every
   second save of such a file. A client cannot delete fixed text anyway.
   For the same reason, `confirm_round_trip` expects the header's new
   declarations to read back as a leading fixed piece.

6. **`legacy_layout(doc, scenario) -> Scenario`.** It returns the whole
   derived scenario, not just a `Vec<LayoutItem>`, because it also
   restores, from the file, what a layout-less client never sent:
   comments, fixed text and disabled entries. Some further details:
   - It inherits in-block comments. A `~` rule or one-off whose lines state
     no comment at all (no `comment` and no `rule_comment` on any line)
     takes its own block's comments: line by line by id for rules, and by
     position for one-offs when the counts match. Otherwise today's table,
     which cannot edit comments, would strip them from every block it
     rewrites, and would rewrite every commented block on every save,
     because a comment is part of a block's print (C6). Consequence for C2:
     a client that sends comments but no layout cannot *clear* every
     comment of a rule. Clearing needs a layout (C3).
   - It makes keys unique. Keyless `=` rules (today's wire) get
     `auto:in:N`, and a duplicate event id gets a `#N` suffix.

7. **Keys not fixed by the plan.** A disabled `=` rule is
   `key: "auto:off:K"`, `block: None`, where K is its index among the
   file's disabled `=` blocks. Comments are `comment:N` and fixed pieces
   `fixed:N`. `auto:B`'s B counts only active `=` blocks, so `block`
   keeps today's meaning.

8. **Server changes in C1** (the plan said core only):
   - compile fixes: the new fields default to empty in the four struct
     literals of `projections_api.rs`;
   - one behavioural line: `WireScenario::new` sends `scenario.active()`.
     Before plan 31 a commented-out rule was just a comment. Without this,
     today's UI would receive disabled entries as live rows, run them and
     save them uncommented. With it, the UI never sees them, and its
     layout-less save keeps them where they are (`legacy_layout`).
     C2 replaces this with the flagged layout on the wire.

9. **`Scenario::active()` also orders `=` rules by layout position.**
   hledger applies them in file order, and the layout *is* the file order.
   The engine calls it through a `Cow`, so a scenario with nothing
   disabled and its rules already in order is not cloned.

10. **Identity's exceptions.** `apply(read(t)) == t` holds byte for byte
    apart from the header (always re-rendered, as before) and one
    pre-existing normalization: a `; line:` tag on a single segment, which
    is not a step, is still dropped on save
    (`a_legacy_gap_tag_still_loads_and_is_dropped_on_the_next_save`).
    A file without a final newline keeps that after any edit or move.

11. **The C6 comment model.**
    - Reading: `comment` / `rule_comment` / `ScenarioEvent::comment` /
      `posting_comments` are the parser's comment with every comma segment
      that is a `line`, `growth`, `opening` or `gains` tag taken out. Lines
      are trimmed, empty lines dropped, and the result joined by `\n`. A
      comment with no Ledgeline tag comes back exactly as written.
    - Writing refuses a comment that states one of those four tags (it
      could never read back), and any control character other than a tab.
    - A posting: Ledgeline's tags, then the user's first line after `, ` if
      that line holds no tag, else on continuation lines. Continuation
      lines are indented eight spaces.
    - A rule header: the first line goes on the `~` line after `  ; `, and
      the rest go as `    ; ` lines before the first posting. The parser
      folds both into the rule's comment.
    - Not modelled: a comment on the counter (amountless) posting. An
      unchanged block keeps it, but a rendered block drops it.
    - `posting_comments` drops trailing empty strings on read, so a one-off
      with no comments has `[]`.

12. **Disabled candidates parse with the inherited styles only**, as the
    plan says, not with the file's own `commodity` directives as well. A
    disabled amount that depends on a style declared only in the scenario
    file can read differently while disabled. The round-trip check is the
    backstop.

13. **Reading refuses a file whose blocks cannot be lined up.** If the
    scan's count of `~` or `=` blocks differs from what the parser read for
    this file, the read fails with `Unreadable`. Before, the writer quietly
    stopped preserving bytes in that case. The fixture sweep and the
    generator assert that the two always agree.

14. **API added beyond the plan's list, for C2:**
    - `serialize::comment_text(style, raw)`: the wire's display `text`.
    - `serialize::comment_raw(style, text, previous)`: an edited
      comment's raw. A new section uses the house style. An edited one
      keeps its own dividers and framing.
    - `serialize::attached_raw(text)`.
    - `ProjectionDoc::{pieces, piece_text}` and `PieceView`, for C15's
      `raw`.

15. **Where the tests live.** "Disabling equals deleting", with `project`
    and `project_cached`, is in `tests/projection_pieces.rs` beside the
    generator it draws from, not in `projection_cache.rs`.
    `projection_cache.rs`'s cached-vs-reference sweep gained
    `comments.journal`. The span-equality check over every fixture is a
    unit test, because the scanner is crate-private. Over the generator it
    goes through `ProjectionDoc::pieces`.
