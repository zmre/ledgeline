# 32 — Phase 2 contract amendments

These belong under "Contract amendments made during implementation" in
`32-transaction-review.md`. Fold them in when the branches meet. Amendment 8
in `32-amendments-p1.md` ("NFKC is partial") is superseded by amendment 13
here.

1. **Payees and tags come wrapped.** The routes return `{generation, payees:
   [{payee, count, last}]}` and `{generation, tags: [{name, count, values:
   [{value, count}]}]}`.
   - Both are cached per snapshot.
   - A payee is the description before `|`.
   - Tags are header tags only. Hidden `_` tags are excluded, and values are
     capped at 20.
   - Forecast transactions are skipped.
   - The core functions are `review::payees` and `review::tag_uses`.
2. **`summary.review` has four fields:** `{unknown, unreviewedRecent,
   notEditable, notEditableRecent}`. The two not-editable counts overlap, so
   they're reported separately.
   - `unreviewedRecent` is capped at `reviewLimit`.
   - The params are `today`, `reviewLimit` and `reviewMaxDays`, and the
     summary's ETag now includes the date.
   - When the field is absent, the decoder yields `review: null`.
3. **`setPostingAccount.posting` counts written postings**, the form's rows,
   not the served ones as PATCH does. That leaves generated postings with no
   address.
4. **Fingerprints are opaque tokens** (`Fingerprint::token()`).
   - The server compares them under the editor lock, then passes the current
     fingerprints to `apply_batch`.
   - GET routes read fingerprints under the same lock as the snapshot.
   - A fingerprint is `null` when the server has no editor.
5. **Batch errors.**
   - 409: plain text naming the stale transaction.
   - 400 `transaction #N: …`: an op was refused. A malformed or empty body is
     also a 400.
   - 404: missing index. 501: no editor.
   - Nothing is written on any failure.
   - On 200, the batch returns the edited rows with `flags: []`. The store's
     background revalidation brings the flags back.
6. **Similar takes `today`** and returns `{generation, index, key, total,
   candidates, inQueue, precedent}`. Candidates may be future-dated (D11 says
   "any date"). Applying a rename or category to a scheduled transaction is
   harmless, so they stay in. The queues still exclude future dates.
7. **Editability is `JournalTarget::writable`**, extracted as
   `journals::writable_sources` and shared with `targets_in`. The
   `ReviewIndex` is cached in `ViewCache` and warmed at snapshot build.
8. **Queue items are `/api/journal/rows` rows plus `unknown`, `reviewed`,
   `fingerprint`, `key` and `suggestions`.** Suggestions carry `strong`, so the
   client doesn't re-implement the threshold. `JournalRow` gains:
   - `unknown`: uncategorised and not `ignore`;
   - `reviewed`: any `reviewed` tag, `ignore` included.
9. **TS placement.**
   - The decoders live in `api/reviewDecode.ts`.
   - `formToOps` returns `WireEditOp[]` with `replace` first.
     - A date change forces `replace`.
     - Equal amounts (5.5 vs 5.50) and an elided leg left blank are not a
       split.
   - New helpers in `editMapping.ts`: `ReviewForm`, `reviewFormOf`,
     `commentFreeText`, `cardOps` (writes `payee-key` on a rename and the
     `reviewed` stamp), and `cardChange`/`similarOps`.
10. **Store semantics.**
    - In Unknowns, the optimistic advance skips cards included in the pending
      batch.
    - A failed save rewinds to its card, keeping the draft and the error, and
      cancels the saves queued behind it. Their drafts are kept.
    - Revalidation runs after each save, and when the generation changes.
      - It keeps the run's order, its skips and its saved cards.
      - Departed cards are dropped, except the one on screen.
      - Cards that changed underneath are refreshed and lose their drafts,
        except the one on screen.
      - New queue items are appended.
11. **Generated postings (amendment 14 of p1).**
    - `is_unknown` counts `=` postings on real transactions.
    - Forecast-tagged (`generated-transaction`) transactions are never
      unknown, and `local_findings` now skips them for all four transaction
      rules: pending, uncategorized, missing description and future date.
    - Precedent, the funding account and spend still use written postings
      only.
    - Served postings already carry `generated: {rule, query}`, which the card
      uses to name the rule.
12. **Fixture layout.** The `=` rule lives in `review-2025.journal` (the
    include). In the main file it trips
    `projections::serialize::the_scan_s_blocks…`, whose own-rule filter
    compares an uncanonicalised path. That is a pre-existing bug, noted, not
    fixed.
13. **Full NFKC.** `unicode-normalization` 0.1.26 normalises and then
    lowercases. A superscript or circled digit becomes a digit and is dropped.
    crane builds from `Cargo.lock`, so `flake.nix` needs no change.
