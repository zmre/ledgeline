# 32 — Phase 4a contract amendments

These belong under "Contract amendments made during implementation" in
`32-transaction-review.md`. Fold them in when the branches meet. Phase 4a is
the server and the wire. Phase 4b (the UI) builds on them.

1. **Staged rows.**
   - The dry run's `txns[]` use the review queue item's shape: the flattened
     journal row, plus `unknown`, `reviewed`, `fingerprint`, `key` and
     `suggestions`. Each also carries `ordinal` (0-based in `entries`) and
     `overridden`.
   - `index` is `ordinal + 1`, the proposal's own file order, so the review
     store can key cards by it.
   - The row is the CURRENT transaction, with overrides applied.
   - `fingerprint` and `key` always describe the GENERATED text. The
     fingerprint is what an override echoes. The key follows D11's "original
     description".
   - Fingerprints come from the engine's own editor, held over a memory vault
     that contains the proposal (`import_review::StagedText`). They depend
     only on a transaction's own text, so the dry run and the commit agree on
     them.
2. **Override rules.**
   - Each request carries at most 10,000 overrides.
   - An ordinal past the end of the regenerated proposal is a `409`, the same
     as a stale fingerprint, because the proposal changed underneath.
   - The same ordinal twice is a `400`, and so is an override with no ops.
   - Ops are checked as the batch route checks them. A `replace` op takes the
     open journal's amount styles.
   - With no overrides, `entries` is hledger's text byte for byte, as before.
3. **A stale review edit writes nothing, not even the CSV.**
   - A commit that carries overrides first regenerates the proposal from a
     staged copy (one extra preview) and applies the overrides to it.
   - A commit that carries `journalEdits` checks their fingerprints under the
     editor lock.
   - Both checks happen before the CSV is written. They are repeated where the
     write happens. Other failures after the CSV write still leave the CSV in
     place, as before.
4. **How `journalEdits` land.**
   - They are applied with `review_api::apply_edits` and saved under the
     editor lock BEFORE the append. That lock is held through the append and
     the catch-up.
   - If either of those fails, every edited file is put back to its pre-edit
     bytes. The restore is pinned to what the save wrote, so it never
     overwrites a later edit. The error says the edits were undone, or names
     the files that could not be put back.
   - The edited files join both the git-blocked check and the commit set.
   - The commit message reads `import N transactions from X, and edit M
     existing transaction(s)`.
   - The response gains `overrides` and `journalEdited`.
   - New core method: `JournalEditor::dirty_sources()`.
5. **Detecting a rules file edited during the import.** The commit compares
   content against the stage; there is no client flag and no mtime check.
   - At stage time, `Stage` records an id → revision token for every
     discovered rules file.
   - The commit adds the import's rules file to the commit set (never to the
     blocked check) when its current bytes differ from that baseline, or when
     it was not in the baseline.
   - Why this way:
     - Every way the file can change counts: a "Remember as a rule" save, the
       rules tab, another tab.
     - A client cannot forget to say so.
     - A file already dirty before staging is not swept into the commit
       unless it also changed during the import.
     - mtime is ruled out by DL-3.
   - Two edge cases:
     - A rules file made with the Create panel re-stages, so it is in the
       new baseline and is not committed. That is unchanged behaviour.
     - A file that was dirty before staging AND edited during the import is
       committed whole.
6. **CLI parity (Q8).**
   - `CliRun` gains `reviewed_edits`.
   - The dry run gains `cliNote`: `"plus N reviewed edit(s)"`, or `null`.
   - `cliCommand` is unchanged, because it has to round-trip through clap.
7. **Raw records.**
   - Shape: `records: {aligned, header, fields, rows}`.
     - `rows[i]` belongs to `txns[i]` when aligned, and is `[]` otherwise.
     - `header` is the converted CSV's header row.
     - `fields` is the rules file's `fields` names, or `null`.
     - Cells are clipped like the stage preview's (64 columns, 200
       characters).
   - Alignment costs one extra `hledger print -O json` per dry run. It reads
     each printed transaction's `tsourcepos` line. In hledger 1.52 that line
     is the transaction's position among those the file PRODUCED, in file
     order (checked on a newest-first file).
   - So the run counts as aligned only when as many transactions print as
     there are data records after the rules file's own `skip`. The proposal,
     which `.latest` and the id match have filtered, is then matched in order
     by date, description and accounts.
   - Anything that does not fit gives `aligned: false`: a `skip`/`end` rule, a
     header read as data, an unmatched transaction, or a proposal our parser
     cannot read.
8. **Rule-file suggestions (D13a source 2):
   `review::rule_file_suggestions(doc, description, classes)`.**
   - **When a block counts:** an `if` block counts when ANY of its matchers,
     read as a case-insensitive regex, matches the GENERATED description.
     Field scope and `&` partners are deliberately not required: hledger has
     already decided the rule did not fire.
   - **What a block can suggest:** only `account2` and higher count. Values
     with a `%` reference and uncategorized accounts don't.
     - The block's `description` assignment is the rename.
     - Blocks with `skip`/`end`, and opaque blocks, never count.
   - **Confidence:** 70%, split among the distinct accounts that match, so a
     single matching rule is strong.
   - Suggestions are produced for every staged transaction. The overlay's
     chip logic already shows them only on unknowns, or as "usually X" when
     they disagree.
   - **New core API:**
     - `ReviewIndex::suggest_staged(journal, classes, txn, rule_file)` ranks
       them with sources 1, 3 and 4. Nothing is excluded, because the
       transaction is in no journal.
     - `ReviewIndex::similar_by_key`.
     - `Similar: Default`.
9. **New route `GET /api/review/similar-key?key&today`** (D24, "already in
   the journal", for a staged card).
   - It returns the similar body with `index: 0` and `inQueue: []`.
   - The key is normalised again, so a raw description works too.
   - Keys over 1 KB are a `400`.
10. **The rule preview request carries all four dry-run handles:** `stageId`,
    `rulesId`, `csvPath` and `journalId`, plus `rule`.
    - The destination's `.latest` and the target journal decide which rows
      are in THIS import, so without them the ordinals would not match the
      dry run's.
    - Matchers are OR-ed: one group each, hledger's stacked `if`.
    - `total` is the import's rows (after dedup and the id match), not the
      file's rows.
    - Response: `{ok, matched, total, alreadyCategorised, changed:
      [{ordinal, before, after}], changedTotal}`. `changed` is capped at 50.
    - Errors:
      - hledger refusing the rules file (a regex it rejects) is a `200` with
        `{ok: false, stderr}`, like the dry run's.
      - A malformed rule, or one the rules model refuses (an unescaped `(`, an
        unknown field), is a `400`.
      - A rules file that `include`s another is not previewed (`ok: false`),
        because the temporary copy would resolve the include from the wrong
        directory.
11. **How the rule preview works, and what it costs.**
    - It runs three `import --dry-run`s in parallel: the rules file as it is,
      with the candidate appended, and with a PROBE appended.
    - The probe has the same matchers and assigns only `code
      ledgeline-rule-preview-probe`. `matched` comes from the probe, so it
      counts rows another rule already categorised.
    - Both copies are rendered with `RulesDoc::apply` and `verify` from an
      inserted `ItemBody::IfBlock`. No rules text is built by hand.
    - Each preview gets its own run directory (`Stage::preview_slot`, then
      `discard`).
    - Measured on the fixtures: about 100 ms wall clock (the dry run is about
      190 ms).
    - Once started, it cannot be cancelled on the server. The client aborts
      the fetch, and a superseded answer is dropped.
12. **QuickBooks preview.**
    - The preview's `txns` lists every built transaction, keyed by `id`
      (Trans #), with `classification` (`new` / `unchanged` / `changed`). It
      is empty while any account is unmapped.
    - Fingerprints and rows come from the transactions rendered with
      `format_transaction`, exactly as the editor writes them, and held as one
      staged text.
13. **QuickBooks commit.**
    - An override must name an id this commit WRITES: a `new` one, or a
      `changed` one under `updateChanged`. Anything else is a `400`. A stale
      fingerprint is a `409` naming the Trans #.
    - An edited transaction is re-parsed from the staged text and written
      like any built one.
    - `journalEdits` go through `add_and_replace_transactions`, which gains a
      first parameter, `edits`. They are applied first, while their indices
      still hold, and share the lock and the single save.
    - A commit that carries only `journalEdits` still writes.
    - The commit message gains `, and edit M existing transaction(s)`. The
      response gains `overrides` and `journalEdited`.
14. **QuickBooks fixture.**
    - New: `fixtures/import/qb-journal/uncategorized.xlsx`, with its builder
      in `generate.py` and a README row.
    - It contains QuickBooks' own `6999 Uncategorized Expense`. The TEST
      aliases that account to `expenses:unknown`; the fixture does not.
    - The global `*-journal` ignore hides the file, so it needs `git add -f`.
15. **Redaction.**
    - Row and record strings are the statement's own text and are not run
      through the redactor. `entries` still is.
    - The rule preview's stderr hides the temporary copies' paths behind the
      rules id.
16. **Balances.** The dry run's balance check reads the overridden text. The
    commit's assertion preflight still reads hledger's raw proposal, as it did
    before. `write_assertion` re-verifies against the bytes that land.
17. **TS, as built.**
    - Empty `overrides` and `journalEdits` are left out of request bodies.
    - Decoders read an absent `txns`, `records`, `overrides`, `cliNote` or
      `journalEdited` as empty, unaligned, 0 or null.
    - Replay always runs against a no-override dry run:
      - a dry run that returns `409` re-runs without overrides, replays, then
        runs again with what is left;
      - an override is flagged only when the text changed in a way its pruned
        ops don't explain, so "recategorised and reviewed, then saved as a
        rule" keeps just the reviewed stamp, unflagged.
    - A QuickBooks commit that returns `409` re-fetches the preview and
      replays.
    - In a staged Unknowns run, a sibling given `setPostingAccount` or
      `replace` leaves the run and counts in `fixed`. Otherwise it gets
      `updatedFrom`.
    - Staged runs cache their similar sets for the whole run, and do no
      paging, revalidation or generation sync.

## Phase 4b (the UI)

18. **How an import opens the overlay.**
    - `reviewLaunch.openStaged(launch)` takes a `StagedLaunch`:
      `{kind, items, sink, rules, writeLabel?, onClose?}`. `rules` is the rule
      builder's view of a CSV import, or null for QuickBooks. `writeLabel` is
      the host's write button ("Write changes", or "Import").
    - `NewTransactionsPanel` mounts `ReviewOverlay` while
      `reviewLaunch.staged` is set. It sits outside `DryRunPanel` and
      `QbJournalPanel` because a rule saved from inside the overlay re-runs the
      dry run, and the dry-run panel unmounts while that loads. Leaving the tab
      closes the run.
    - `ReviewOverlay` gains a `staged` prop and branches on mount
      (`openStaged` or `open`). Its `kind` is `QueueKind | StagedKind`.
    - Closing a staged run calls `onClose(saved)`, not the journal's
      `returnTo`. The CSV panel re-runs the dry run when anything was saved, so
      the preview shows the bytes the edits will write.
19. **The buttons.**
    - `REVIEW_BUTTON` moved to `review/ui/buttons.ts`, shared by the Journal
      toolbar and the new `imports/ui/StagedReviewButtons.svelte`.
    - The strip says review is optional and names the write button.
      `DryRunPanel` shows it above Write changes when the run is `ok` and has
      `txns`. `QbJournalPanel` shows it once no account is unmapped and there
      are `txns`.
    - `u`/`r` live in the strip's own layer (`PRIORITY.widget`, group
      "Imports"), so they exist exactly while it is mounted. Each is disabled
      when its button is.
    - **Counts:** Review Unknowns leaves out an unknown whose held override
      already recategorises it (`stagedRunItems`). A QuickBooks preview takes
      no overrides, so without this a fixed unknown would come back in the next
      run. It changes nothing for CSV, whose rows already show their overrides.
20. **The edits line.**
    - `StagedEditsNote` sits directly under the write button, not on the same
      row, so the flagged list can open full width. It reads "N reviewed edits ·
      M existing transactions edited — applied by this write".
    - Flagged edits show as "K need another look". That expands to a list of
      date, description and what each edit does (`describeOps`), with a Discard
      on each row and "Discard all".
    - New: `StagedEdits.discard(ref)`, `importStore.discardOverrides(refs)`
      (which re-runs the dry run) and `qbJournalStore.discardOverrides(refs)`.
    - `cliNote` renders under the command as "{cliNote} — the command line
      imports without them."
21. **The overlay over an import.**
    - Titles are "Review Unknowns" and "Review All".
    - The ⋯ menu drops **Open in full editor** (and `o`), because there is no
      file to open yet. It also drops **Settings**, whose window and limit only
      shape the journal's queues. **Ignore** and **Edit date** stay, and both
      become overrides.
    - The empty state and the summary have their own wording. The summary says
      "Nothing is written yet. These edits are applied when you press
      **{writeLabel}**."
22. **Two Similar groups.**
    - `SimilarSection` takes `title` and `testid`. A staged card shows
      "Similar in this import" (`review-similar-staged-*`) and "Similar already
      in the journal" (`review-similar-journal-*`). A journal run keeps
      `review-similar-*`.
    - The rows list carries `data-similar-rows`. The check-all box's label is
      now "Check all: <title>".
    - Each group has its own open state and checked list. Staged rows are keyed
      by `ordinal + 1` and journal rows by tindex, so the lists must stay
      separate.
    - `s` opens or closes both groups. ⇧A checks both, or unchecks both once
      everything is checked. Space works while either is open.
    - The import group shows no "in this run" badge: every row in it is in the
      run.
23. **Rule builder choices.**
    - **Pattern.** The key's tokens are used when they appear, in order, in the
      raw cell (`records.rows[ordinal][fields.indexOf(field)]`). If they don't,
      because a description-only rule renamed the row, the raw cell's own
      tokens are used. A whole-record match takes its prefill from the
      description cell.
    - **Match field.** The select lists the rules file's `fields` names plus
      "whole record". Switching field re-prefills the pattern only if the user
      hasn't typed in it.
    - **Then set.** There is one line per moved account, as `account{N}`, where
      N is the written posting's position plus one. hledger writes posting N
      from `accountN`. The description line follows the accounts.
      - Accounts start checked. The description starts checked only when it is
        the whole change, which matches the mockup (☐ description beside a
        ☑ account).
      - The values follow the card's edit live. They can't be edited in the
        builder.
    - **Live check.** It runs only while the toggle is on, 250ms after the rule
      settles (`RULE_PREVIEW_DEBOUNCE_MS` in `imports/ruleBuilder.ts`). It is
      keyed on the rule's JSON, so editing a tag doesn't re-run it.
      - A newer check aborts the older one, and a late answer is dropped.
      - The last answer stays on screen, dimmed, while the next one loads.
      - Zero matches gets a warning.
      - The list of changed rows shows 20, then "…and N more".
    - **Changed rows.** Each row is a diff with spaces collapsed first.
      Verified on the live engine: hledger re-aligns the amounts when an
      account name gets longer, so without collapsing, the bank line showed up
      as changed.
24. **Saving with the toggle on.**
    - The order is: the rule (`RuleSource.save` →
      `importStore.saveReviewRule`), then the card into the sink, then
      `refresh()`.
    - If the rule is refused, the card stays with every edit and the reason
      shows under the rule.
    - Card keys are off while the rule saves, and the footer is busy.
    - The toggle starts off on every card.
    - `R` is bound in the overlay's own layer and is enabled only while the
      builder is shown, which is also when the `?` sheet lists it.
    - `importStore.saveReviewRule(rule)` reads the rules document fresh, then
      does `toForm` → `appendRule` (a `blankRule` with one group per matcher) →
      `toSaveRequest` → `rulesStore.save`. The revision is pinned to that read.
      `importStore.ruleSource()` builds the `RuleSource`.
    - `RuleSource` (in `reviewTypes.ts`) is `{records, fileLabel, rulesLabel,
      preview, save, refresh}`.
25. **The e2e engine.**
    - The import-review test runs on `:5097`, over
      `target/e2e-import-review/`, which is restaged on every run. It holds an
      empty `main.journal`, `imports/checking.csv.rules`, and a
      `LEDGELINE_CONFIG_DIR` whose `prefs.json` says `gitAutocommit: false`.
    - Git is off because the copy sits inside this repository's working tree.
      With git on, the import would try to commit gitignored paths into the
      real repository, and a run from a hook would inherit `GIT_DIR`.
    - The test therefore can't assert that the rule and the import share one
      git commit. `import_endpoints.rs` covers that.
    - The flow was checked by hand against the debug engine with curl: stage,
      dry run, rule preview (`matched 1 of 5`), rules PUT, a stale override →
      409, replay, and a commit with `git: null`.
26. **Known gaps, left as they are.**
    - **Review All shows stale fields after a sibling apply.** A sibling
      changed through "Similar in this import" keeps showing its generated text
      until the next dry run. It carries "updated from card N". Saving it
      merges its ops into the held override, so nothing is lost. But a
      recategorised sibling still shows its unknown, and **Reviewed** stays
      disabled until it is categorised again.
    - **QuickBooks cards always show the export's own text.** The QuickBooks
      preview takes no overrides, so Review All cards read as the export,
      whatever earlier runs saved.
    - **No blank line before a new rule.** The engine writes the `if` block
      right after the previous block, which is cosmetic. hledger reads it.
    - **`%` in a description value.** It is written verbatim, and hledger
      reads `%name` in an assignment as a field reference.

27. **The post-rule 409 is resolved off-screen, and real failures show on
    top of the deck** (Patrick's review, 2026-10-31).
    - **The cause.** A rule saved from a review card always changes the text
      the card's own override was fingerprinted against. The next dry run
      therefore answered 409, and `runDryRun` loaded that into the visible
      `dryRun` resource before replaying. The conflict flashed as an error in
      the dry-run panel underneath the blurred backdrop.
    - **The fix.**
      - `Resource.run(task)` lets one load settle on a different query than
        it began with, without publishing intermediate failures.
      - `runDryRun` does try, then 409, then bare run, replay and re-run
        inside one `run`.
      - `RuleSource.refresh` now resolves to the regenerated import's failure
        (the engine's message or hledger's stderr), or null.
      - The overlay shows that failure as a dismissible alert above the deck
        (`review-import-notice`).
    - **Tests.** A store test pins that no request after the 409 sees a
      published error. A component test pins the notice.

28. **Git integration removed** (Patrick, 2026-10-31).
    - **Why.** "Right now, our git support is very limited and creates more
      problems than it solves. Maybe we'll make a more complete git
      integration later, but right now, please remove any git checks or
      commits from the code."
    - **What is gone.**
      - `crates/ledgeline-server/src/git.rs` and `tests/git_commit.rs`.
        `hledger.rs` is now the only module that spawns an external tool.
      - The pre-write "uncommitted changes" refusal (`blocked_by_git`, the
        409) on the CSV commit, save-csv and QuickBooks commit.
      - The post-write commits: import, save-csv, sort and QuickBooks
        (`commit_targets`, the generated commit messages).
      - The plan-32 additions that only fed the commit: the edited files
        joining `Plan::targets` (amendment 4), and the rules-file baseline
        on `Stage` with `Plan::rules_edited` (amendment 5).
      - `GitPolicy` and `ledgeline import --no-git`. Old scripts that pass
        it now get clap's unknown-argument error.
      - **Wire fields:** `git` on capabilities (`{available, autocommit}`);
        `blockedByGit` on the dry run; `git` on the commit, save-csv, sort
        and QuickBooks commit results; `gitAutocommit` on `/api/prefs`.
      - **Web:** the git-blocked alert in the dry-run panel and the "Git
        commit failed" line in the result panel. Also the "Committed …" and
        "Not committed …" result lines, and the "re-sorted but git did not
        commit" warning in both stores. `canWrite` no longer checks git.
    - **Prefs.** `Prefs`, `WirePrefs` and the TS `Prefs` hold only
      `hledgerPath`.
      - The file-side struct never denied unknown fields, so an old
        `prefs.json` with `gitAutocommit` still loads. The key is dropped on
        the next store. `tests/prefs.rs` and `import_endpoints.rs` pin this.
      - `WirePrefs` is `deny_unknown_fields`, so a client that sends
        `gitAutocommit` gets a 400. The SPA never sends it: `decodePrefs`
        keeps only `hledgerPath`, and `saveHledgerPath` sends only that.
    - **D24/D25.** "The same git commit" now means **the same write**.
      Journal edits and the append still land under one lock, and a failure
      still writes nothing. The tests now assert the set of files that
      changed on disk, not the contents of a commit.
    - **The e2e engine (amendment 25).** `:5097` no longer gets its own
      `LEDGELINE_CONFIG_DIR`/`prefs.json`. The scratch copy under
      `target/e2e-import-review/` is still restaged on every run.
