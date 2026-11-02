# 32 — Phase 6a contract amendments

These belong under "Contract amendments made during implementation" in
`32-transaction-review.md`. Fold them in when the branches meet. Phase 6a is
the engine, the server and the wire for D27 ("Remember as a rule" from the
Journal view). Phase 6b (the UI) builds on them.

1. **Ambiguity is explicit: `needsChoice`** (Patrick, 2026-10-31: "if after
   our rules there are still multiple possible rules files, let the user
   decide").
   - `recommended: true` goes on a candidate only when the D27 rules leave
     exactly one winner: one family left after the FITID and `.latest`
     signals, whose year rule picks one file.
   - Anything else — two families, or a family whose pick only the id would
     separate (same stamp, same `.latest` date, same mtime) — marks none,
     and the response says `needsChoice: true`. 6b shows the picker with
     nothing preselected and saves no rule until the user picks.
   - The importing-ahead alternative is only a listed second option. It does
     not make the choice ambiguous.
   - Core: `rules::families::pick_is_decided`, `RulesInference::needs_choice`.
     The TS decoder refuses a body whose `needsChoice` disagrees with its
     recommendations, or that recommends two.
   - Tests: `rules_infer.rs` (chase, citi, venmo, schwab decided; amex and
     the twin-file index undecided), `review_rules_endpoints.rs`,
     `reviewDecode.test.ts`, `journalRules.test.ts`.

2. **Where the inference lives.** `ledgeline_core::rules::infer`, not
   `review`: it reads rules documents and discovery. The API is
   `RulesIndex::build(&Discovery)` (every parsed rules file and its marker,
   read once) and `rules_for(txn, &Classes, &RulesIndex) -> RulesInference
   {candidates, needs_choice}`. `review::funding_accounts` and a new
   `review::matcher_regex` became `pub(crate)` so the two agree on what a
   funding account and a matcher are.

3. **How the signals combine.**
   - Candidates are the files whose effective `account1` is one of the
     transaction's funding accounts (written postings that are neither a
     category nor uncategorised). No match: no candidates, and the toggle is
     hidden.
   - Candidates are grouped into families. FITID narrows first, then
     `.latest`, each at the family level (a family passes when ANY member
     agrees), and neither may narrow to nothing.
     - FITID agrees when the transaction has an `id:` tag and the file writes
       one, or has none and the file writes none. So the convention breaks a
       tie in both directions (`wells` vs `wells-ofx`).
     - `.latest` agrees when the marker's date is on or after the
       transaction's, or there is no marker.
   - Every candidate is still listed, and the picker offers all of them. The
     narrowing decides only the recommendation.
   - Order: the live families first, in family-label order, then the rest.
     Within a family, `order_family`'s order. The recommended file is first
     when there is one.
   - `reasons` are plain sentences: the account, the FITID agreement, the
     marker's date, the family placement ("Named for 2026, this
     transaction's year", "...in case you are importing ahead"), and "Another
     rules file for this account fits as well" on each tied pick.

4. **Effective `account1`.**
   - It is the `account1` of the last `if` block that matches every record,
     else the top-level one. A block "matches every record" when one of its
     OR-groups holds only patterns like `.` or `.*`: patterns that match every
     probe string. The probes include `""` for a field-scoped matcher only.
   - A value with a `%` reference names no single account, so it counts as
     none.
   - **Gaps, left as they are.**
     - An `include`d `account1` is not followed. `paypal.csv.rules` in the
       fixture has none, and the file it includes is the candidate instead.
     - Account aliases are not applied to `account1`. A journal that aliases
       the rules file's account name won't match it, and the toggle stays
       hidden.

5. **Families and year tokens** (`rules::families`).
   - A token is the first maximal digit run that reads as `YYYY`, `YYYYMM` or
     `YYYYMMDD`, with years 1900–2099 and a valid month and day. A `YYYY` may
     be followed by `-`, `_` or `.` and a two-digit month (and day).
   - The same is applied to directory components that are wholly a token.
   - The family key is the id with each token replaced by a NUL, so
     `citi-2025.csv.rules` ≡ `citi-2026.csv.rules` and `2025/citi.csv.rules`
     ≡ `2026/citi.csv.rules` (sibling year directories only).
   - The label is the stripped name (`citi`), or the nearest non-year
     directory for `2026.csv.rules`.
   - **Within a family** (`order_family`):
     1. The file for the date's year. For monthly files, the newest one
        started on or before the date.
     2. Else the newest year before it.
     3. Else, a date older than every file: the EARLIEST file. D27 doesn't
        say what happens here.
     4. With no date or no stamps: newest stamp, then `.latest`, then mtime.
     5. Ties last by id.

6. **`.latest` markers.** `Discovery::latest_date(id)` reads the marker
   beside the file's data file: `.latest.<name>` in the data file's
   directory, where hledger keeps it.
   - With no data file and no `source`, the sibling CSV's name is used,
     because statements are often moved away after an import
     (`.latest.citi-2025.csv`).
   - The value is the newest ISO date among its lines.
   - The read is vault-pinned and capped at 256 KiB. Anything else reads as
     no marker.

7. **The import's ranking shares the family order.**
   `matching::rank_in_families(rankings, date)` runs `rank`, then, inside
   each run of EQUAL score, lets same-family members trade only the slots
   they already held, in `order_family`'s order.
   - `date` is the newest `tdate` any candidate printed from the statement.
   - The import passes no `.latest` dates, so its in-family fallback is mtime,
     as before.
   - Tests in `tests/matching.rs`:
     - the three existing ranking scenarios give identical orders through
       `rank_in_families`, with and without a date;
     - a 2025 statement moves `checking-2025` ahead of `checking-2026` and
       leaves an unrelated equal-score file, and a lower-scored family
       member, in place.

8. **The CSV basis is `hledger print`, not `import --dry-run`.**
   - The statement on disk was usually imported already, so its `.latest`
     (and the journal's ids) would filter every row out. The journal preview
     therefore runs `hledger [--alias…] print -f CSV --rules R` three times:
     as it is, with the candidate, and with the probe.
   - `total` is every row of the file.
   - It reuses the import's pieces, extracted rather than copied:
     `import_review::preview_rules` and `has_include`, then
     `import_api::run_previews` (the parallel three-run helper, now shared by
     the import's preview), `include_refusal`, `print_invocation` (now also
     behind candidate scoring's `print_json`), `csv_aliases`, and
     `import_review::compare_runs`.
   - The CSV is `Discovery::data_file(id)`: the same `source`/sibling
     resolution and guards as the column preview. A `source` glob takes its
     newest match.
   - The temporary rules copies go in `StageArea::scratch()`, a fresh private
     directory under the session root that is removed on drop.
   - hledger's stderr hides the copies' paths behind the rules id and the
     CSV's path behind its file name.

9. **Wire shapes.**
   - Every `200` carries `ok`, so a refusal (`{ok: false, stderr}`) is told
     apart the same way as on the import's preview. The CSV basis is the
     import preview's body plus `basis: "csv"` and `file` (a name, never a
     path). The fallback is `{ok: true, basis: "journal", matched, total}`.
   - `noCsvMatch` is not on the wire. The TS decoder derives it (`basis:
     "csv"` and `matched === 0`).
   - Candidates carry `fields` (the rules file's `fields` list, or null) as
     well as `matchField`, so the builder's field select has its options.
   - Both routes are `no-store` with no ETag. A rules file, a marker or a CSV
     can change without the journal changing. Each request rescans and
     re-reads the rules files. Nothing is cached.
   - `index` must name a transaction (`404` otherwise). On the CSV basis it
     is not otherwise used. `rulesId` need not be one of the transaction's
     candidates; any discovered rules file previews.
   - With no storage (a memory-only journal), rules-for answers no
     candidates.

10. **The journal fallback.**
    - It considers the journal's real (non-forecast) transactions with the
      file's effective `account1` among their funding accounts.
    - `matched` counts those whose description some matcher's pattern
      matches: OR-ed, field scope ignored, read with the same
      case-insensitive Rust regex the rules-file suggestions use (not
      hledger's TDFA).
    - A pattern that does not compile is `{ok: false, stderr}`.
    - A file with no `account1` considers nothing (`0 of 0`). Such a file is
      never a candidate anyway.

11. **No new write route.** Saving is the rules editor's PUT, through
    `imports/ruleSave.ts`'s `appendRuleTo(serverUrl, rulesId, rule)`. It
    reads the document, appends a `blankRule` with one group per matcher,
    and PUTs pinned to that revision. It is extracted from
    `importStore.saveReviewRule`, which now calls it.

12. **TS, as built.**
    - `RuleSource.records` is replaced by `prefill: RulePrefill` —
      `{fields, field, pattern(item, field)}`.
      - `importPrefill(records)` is the import's old behaviour, moved out of
        the builder.
      - `descriptionPrefill(candidate)` builds the pattern from the card's own
        description, scoped to `matchField`.
      - `RuleBuilder.svelte` now asks the source and keeps no CSV logic of
        its own. That is three lines, plus one narrowing that ignores a
        `basis: "journal"` answer until 6b renders it.
    - `RulePreview` = `RulePreviewOk | RulePreviewJournal | RulePreviewFailed`.
      - `RulePreviewOk` gains optional `basis: "csv"`, `file` and
        `noCsvMatch`.
      - The import's own decoder returns the narrower `ImportRulePreview`.
    - New:
      - `RulesCandidate`, `RulesFor` and `RuleChoice` (in `reviewTypes.ts`);
      - `decodeRulesFor` (`reviewDecode.ts`) and `decodeJournalRulePreview`
        (`importReviewDecode.ts`);
      - `LedgelineApi.reviewRulesFor(index)` and
        `reviewRulePreview({index, rulesId, rule})`;
      - `review/journalRules.ts`: `ruleChoice(found, io)` (pure),
        `engineRuleIo(url)` and `loadRuleChoice(index, signal?)`.
    - A journal source's `refresh()` resolves null, and its `fileLabel` is
      the rules file's label. The statement's real name arrives as
      `preview.file`.
    - Caveat for 6b: `getJson` maps any 404 to `NativeApiUnavailableError`,
      so rules-for on a vanished index rejects with that class. The overlay
      should treat a rejection as "no toggle".

13. **Fixture.** `fixtures/review-rules/` (README there).
    - Its rules files use `date-format %Y-%m-%d` on purpose. `fixtures/` is
      `sample.journal`'s discovery root, and the import live tests stage a
      `%m/%d/%Y` statement there. ISO dates keep the new files out of that
      statement's prefilter, and so out of the eight scored slots that mtime
      fills.
    - The live suite (`review/rulesLive.integration.test.ts`) reads it as
      `review-rules/…`.

# Phase 6b — the Journal UI for D27

14. **The builder is a shell and a body.** `RuleBuilder.svelte` is now the
    toggle and the drawer. The rule itself (Match, Then set, the live check)
    moved, unchanged for an import, into `review/ui/RuleBody.svelte`, which the
    shell keys on `source`.
    - 6a suggested keying the whole builder on the picked id in the overlay.
      That would remount the picker too, and the select would lose focus every
      time the pick changed. Keying only the body keeps the toggle and the
      picker mounted. The picker is one `<select>` in both states (unpicked
      and picked), so the first pick keeps focus as well.
    - The file line is a `file` snippet prop, rendered above Match. An import
      passes none.
    - `source` may be null: a `needsChoice` card before the pick. The body is
      then disabled ("Choose a rules file first: what a rule can match depends
      on the file."), `currentRule()` is null, and nothing is checked.
    - `item` is a `QueueItem`.

15. **The picker** (`review/ui/RuleFilePicker.svelte`).
    - Decided: "Saving to [file ▾] (reason)". The select IS the file name, so
      the name isn't printed twice. With a single candidate there is no select,
      just the name. The recommended option reads "(suggested)".
    - `needsChoice`: "Choose a rules file [Choose… ▾] the ledger doesn't say
      which", with nothing selected.
    - Native `<option title>` tooltips don't show in WKWebView, so the reasons
      are behind a "Why?" button. It lists every candidate's full id and its
      reasons as secondary text.
    - The short reason is `journalRules.fileReason`. A family member that is
      newest gets "newest of N for family". An OLDER family member gets its
      LAST reason, not its first: the engine's first reason is always the
      shared account line ("Imports into …"), and the last is its place in the
      family ("Named for 2025, this transaction's year"). A file alone in its
      family gets its first reason.

16. **Then set's slots on a journal card** (a bug 6a's plumbing would have
    shipped). `ruleTargets` numbered `accountN` by the posting's position in
    the transaction. That holds for an import, which is hledger's own output.
    A journal transaction can list its postings in any order: every
    `fixtures/review-rules/main.journal` unknown writes the expense first. So
    the rule would have set `account1` and rebooked the bank account.
    `ruleTargets(original, change, account1?)` now counts slots from the
    posting booked to the rules file's `account1`. That posting is slot 1, and
    the rest follow in order. `RuleSource` gains an optional `account`
    (`candidate.account1`), set only by the journal source. It also feeds the
    journal-basis line. Pinned in `ruleBuilder.test.ts`, the overlay test
    (`account2` saved), and the e2e (`account2` on disk).

17. **Loading, in the overlay.** An effect on the card's index (journal runs
    only) calls `loadRuleChoice(index, signal)`. The cleanup aborts it when
    the card leaves. Answers are cached for the run in a plain, non-reactive
    `Map` by index. A rejection is cached as null, so there is no toggle and
    no retry. An aborted load caches nothing, so coming back asks again. The
    toggle is offered when `change !== null` and the choice is non-null (the
    staged condition is unchanged). `rulePick` resets per card, and the
    recommendation fills in when it is null.

18. **Saving.** `saveWithRule` takes the active source (the import's or the
    journal pick). With no source (`needsChoice`, unpicked) it sets the inline
    error "choose a rules file for it first, or turn Remember as a rule off."
    and saves nothing. Picking clears the error. The order is unchanged:
    rule, card, `refresh()` (null on a journal source, so no notice).

19. **Keys.** `R`'s `enabled` is the new `ruleOffered`, so the `?` sheet lists
    it on journal cards that have a choice and not on others. When `R` turns
    the rule on and the file is still to be picked, focus moves to the picker,
    which counts as a typing target, so <kbd>Esc</kbd> returns to the card. The
    select is in the Tab order after the toggle. `keyboard.md`'s row no longer
    says "CSV imports only".

20. **Live-check lines.**
    - The journal basis reads "Matches N of M transactions from <account>
      (no statement file found to test against)", with a warning at 0.
    - The CSV basis names `preview.file`.
    - `noCsvMatch` replaces the import's "No row in this import matches"
      with the D27 hint.
    - `review-rule-count` carries `data-basis` (`journal`, `csv` or
      `import`).

21. **E2E: a fourth engine.** `fixtures/review/` has no rules files, so the
    review engine can never offer the toggle. `playwright.config.ts` now
    starts `ledgeline` on :5096 over a scratch copy of `fixtures/review-rules/`
    (`target/e2e-review-rules/`, `cp -R` for the `.latest.*` markers). It is
    staged fresh on every run, because the test writes a rule and a card. The
    test is in `e2e/review.e2e.ts`:
    - CORNER STORE has no toggle.
    - AMEX STORE: `⇧R` focuses an empty picker, the save is blocked, a pick
      is made, the journal-basis count shows, and the save puts
      `account2 expenses:auto:fuel` in `amex.csv.rules` (not in
      `amex-business`) and in `main.journal`.
    - BLUE BOTTLE 4412: `citi-2026` is preselected with "(newest of 2 for
      citi)", the CSV-basis count reads "2 of 3 rows in citi-2026.csv", and
      the picker comes after the toggle in the Tab order.

    I read the facts from a real engine over a copy of the fixture: the queue
    order, both rules-for answers, and both previews. The e2e itself has not
    been run.

22. **Two fixes to the Journal-rules e2e test, made when it was first
    run.**
    - It opens `/?preset=all`. The `review-rules` fixture's transactions
      predate the Journal's default 90-day window, so no rows rendered. The
      review queues use their own 365-day lookback and weren't affected.
    - The live-check count assertions wait up to 15s (`LIVE_CHECK_TIMEOUT`).
      The CSV basis runs an `hledger print` subprocess. It takes under a
      second on its own, but overran the default 5s under the full suite's
      parallel engines.
    - Result: two full `just e2e` runs, 117 passed each.

23. **Same-named rules files show their relative path** (Patrick,
    2026-11-01).
    - `rulesFileLabel(rulesId, all)` (`imports/ruleBuilder.ts`) gives the
      file name, or the rules id (the path relative to the journal) when
      another candidate has the same file name.
    - It's used for the picker's options, the single-candidate name, and the
      builder's "added to the end of …" line (`journalRules.ts`
      `rulesLabel`).
    - Unit tests and a `journalRules` test.
