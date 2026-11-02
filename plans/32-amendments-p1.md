# 32 — Phase 1 contract amendments

These belong under "Contract amendments made during implementation" in
`32-transaction-review.md`. Fold them in when the branches meet.

1. **Review queries take the journal as an argument.** `ReviewIndex` holds
   only derived data (the key map and per-transaction state), so it can sit in
   `ViewCache` next to the server's `Arc<Journal>` and `Arc<Classes>`. Its
   `queue`, `similar` and `suggest` methods take `&Journal` (plus `&Classes`
   where needed).
2. **`queue` takes a `QueueOpts`** (`kind, today, limit, max_days, offset`).
   `QueueOpts::new(kind, today)` returns the D4 defaults: a 365-day window,
   Recents capped at 50, and Unknowns paged at 100. `offset` exists for
   Unknowns paging.
3. **Editability is a predicate passed to `ReviewIndex::build`.** Core has no
   notion of a read-only file. A transaction the predicate refuses counts
   toward `not_editable`, and is left out of the items, the `total` and the
   similar candidates. Forecast-generated transactions are excluded too.
4. **`Similar` has a `total`, and in-queue marking moved to the wire.** The
   engine doesn't know the client's queue, so Phase 2 marks which candidates
   are queued.
5. **Confidence is integer basis points** (`STRONG = 5000`, `CERTAIN =
   10000`), so the 50% chip threshold is an exact comparison. `top_chip()`
   applies it.
6. **Suggestion scoring.**
   - One suggestion per account, ranked by confidence, then source order, then
     name, at most 3.
   - The payee prefix is used only when the exact key has no categorised
     history. It needs at least 2 shared tokens, and its confidence is share ×
     80% × shared ÷ key tokens.
   - A recurring match is worth a flat 60%, split when several subscriptions
     match. It requires the same funding account, an amount within 1% (exact
     `Dec`), and a day of month within 4.
   - Subscriptions are detected once per index, as of the newest transaction.
     This added `Subscription.funding`.
   - Source 2 (rules file) exists as an enum variant but isn't produced. It
     arrives in Phase 4.
7. **What counts as precedent.**
   - Counter-accounts are a transaction's written postings to expense or
     revenue accounts that aren't themselves uncategorized. Transfers between
     two asset accounts are never precedent.
   - Reviewed transactions count double. Ignored ones count once.
   - The pool for a key includes renamed transactions whose `payee-key:` names
     that key. The most common rename is attached to the matching exact-key
     suggestion.
8. **NFKC is partial.** Only the fullwidth ASCII block and the ideographic
   space are folded, because no unicode-normalization crate is in the lockfile.
   Adding one is a dependency decision for Patrick.
9. **`apply_batch` is in-memory, and the caller saves once.** This matches
   every other editor op and the server's `save_and_publish`.
   - A failure is `EditError::BulkTransaction { index: <position in batch>,
     source }`.
   - A stale fingerprint is `BulkTransaction { .., ExternalChange }`, which
     Phase 2 maps to a 409 naming the transaction.
   - No new `EditError` variant was added, because the server matches the enum
     exhaustively.
10. **The per-transaction fingerprint is the existing `Fingerprint`** over the
    transaction's span text. `transaction_fingerprints(&[Tindex])` computes
    many with one scan per file. `transaction_source` re-scans the whole file
    on every call, so Phase 2 must use the plural form.
11. **Op semantics.**
    - `Replace` must be an entry's first op.
    - `SetPostingAccount.posting` counts written postings, and generated
      postings are refused.
    - A transaction may appear at most once per batch.
    - `SetTxnTag(None)` removes every occurrence. `Some` sets the first. A
      continuation line left empty is deleted.
    - Refused as `Unsupported`:
      - tag names containing whitespace, `:`, `,` or `;`;
      - values containing a comma, a newline, or leading or trailing spaces.
    - `SetTxnComment` rewrites only the free text of the header's same-line
      comment. Text that would parse as a tag is refused.
12. **`SetDescription` inside a batch keeps the header comment byte-for-byte.**
    The standalone `set_description` re-renders the comment as `"  ; "`, so the
    two differ on a header like `Payee ;note`.
13. **Open: generated postings and the Problems drawer.** D3 excludes generated
    (`=` rule) postings from "unknown", but `checks.rs`'s `uncategorized` check
    doesn't. With an auto rule that posts to an unknown account, the review
    count and the drawer's count will differ. Decide in Phase 2.
14. **Amendment 13 is resolved (Patrick, 2026-10-30), in Phase 2.**
    - `=` auto postings on real journal transactions count as unknown, in
      review and in the drawer. `is_unknown` loses its generated-posting
      exclusion.
    - Forecast transactions from `~` periodic rules, and auto postings applied
      to them, are ignored everywhere, including the drawer.
15. **Full NFKC (Patrick, 2026-10-30), in Phase 2.** `unicode-normalization`
    is added and replaces the partial fold from amendment 8.
