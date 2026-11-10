# 35 — Transfer matching: link the two halves of a transfer

Each side of a transfer is imported from its own bank feed, and each lands on a
**clearing account** (`transfers:internal-bank`, `transfers:creditcard-payment`).
Ledgeline finds the pairs, links them with a posting tag, and shows each linked
pair in the journal as one row: *Checking → Amex*. **Nothing is merged or
rewritten.** Both transactions stay in the file as they were. The only edit is
one `match:` tag on each clearing posting.

## The ask (Patrick, 2026-11-09)

From `TODO.md` ("Matching"):

> QuickBooks is pretty good at matching and proposing matches when dollar
> amounts are the same and something is a payment or a transfer … hledger
> doesn't do this so right now I have a lot of rules that send things to
> escrow-like accounts like transfers:internal-bank or
> transfers:creditcard-payment so that independent rules in each place can
> work.
>
> Ideally in our journal we'd have a button to search for matches. Maybe we
> have some settings that control this on things like how far it looks back
> and whether its filtered to those escrow accounts or not. Essentially we
> could import with the escrows but then the matcher can find when
> `$X -> account1 -> escrow` matches with a `escrow -> $X -> account2`.

And in conversation:

> I'm not an accountant and I want to be careful about rewriting history and
> losing details or changing reports. That said, it's much easier for me to
> understand a bank transfer or credit card payment that goes from checking to
> savings or from checking to amex than it is to tease that apart when it
> happens in two parts.

Decisions, given after the research below:

1. **Link only.** Don't rewrite files.
2. Clearing accounts are marked with an **account directive tag**.
3. The match window is **10 days**.
4. Collapsed transfer rows are **always on**. Expanding a row shows both halves.

Follow-ups to the first draft (same day):

5. **Remember rejections** so the same wrong pair isn't proposed again, using
   comments on the transactions (tags are acceptable) — D12. Patrick noted it
   could get noisy.
6. A transfer row shows the **original description**, which the user can edit,
   including **from the matcher card** — D9, D11.
7. **No matching across clearing accounts** — D4.

## Research summary

| Tool                | Match criteria                                                     | What a match does                                                    |
|---------------------|--------------------------------------------------------------------|----------------------------------------------------------------------|
| QuickBooks Online   | Exact amount; record dated −90 to +20 days from the bank line       | Links the bank-feed line to one transfer record; Unmatch unlinks     |
| Xero                | Suggested match (criteria not published)                           | One transfer object; the other statement line links to it            |
| Actual Budget       | Exact opposite amounts, different accounts                         | Two transactions linked; dates and cleared flags stay independent    |
| YNAB                | Amounts agree, within a few days, **bank category agrees**          | One payment; a "Matched" filter lets you review the pairs            |
| GnuCash             | Scored; 4 days "likely", 14 days "unlikely"                        | Matches an import against splits already in the book                 |
| beancount `zerosum` | Opposite amount ± tolerance, N-day window                          | Renames both legs to `…-Matched` at load time; the file is untouched |
| hledger             | None. The FAQ suggests `skip` one side, or "an imaginary third account" | n/a                                                             |

What this plan takes from it:

- **Every serious tool keeps both bank lines as source records.** Even
  QuickBooks links feed lines to a record instead of merging them. Its most
  common user error is a duplicate transfer from pressing "Add" on both feeds.
- **Clearing accounts are standard practice.** Manager.io documents them, and
  hledger-flow uses `assets:transfers:bank1bank2`. A clearing account at zero
  means every transfer has landed. A nonzero balance means money in transit, a
  missing import or a miscoded side. A transfer sent Dec 31 and received Jan 2
  is correctly a Dec 31 asset (cash in transit). Linking keeps that correct;
  merging would lose it.
- **zerosum is the closest model.** It pairs postings on a zero-sum account
  and leaves unmatched ones there as the to-do list. Its flaw is **greedy
  first-fit in file order**, which can pair a recurring $500 transfer with the
  wrong month. This plan assigns pairs by smallest date gap across the whole
  candidate set (D6).
- **YNAB's extra signal** (the bank's own category must agree) catches a refund
  and an unrelated payment that share an amount. Ledgeline has no bank
  category, but the **types** of the counter-accounts play the same role (D7).

## What is true today

Verified against hledger 1.52 with a scratch journal (`/tmp/m35`, 2026-11-09):

- **Account directive tags are inherited by subaccounts.** With
  `account assets:transfers ; clearing:`, `hledger accounts tag:clearing`
  lists `assets:transfers` and `assets:transfers:cc`.
- **Posting tags query as expected.** `hledger bal tag:clearing not:tag:match`
  gives exactly the unlinked balance. `hledger reg tag:match=7f3a` lists both
  halves of one pair.
- **The in-transit balance is right at period end.** `bal -e 2027-01-01
  tag:clearing` shows the $500 sent Dec 31 and received Jan 2.

In the code:

- **Inheritance is already implemented.**
  `wire.rs:288 inherited_account_tags` walks the ancestors, and
  `wire.rs posting_tags` computes hledger's `ptags`. The query engine accepts
  the inherited tags (`query.rs:120`). But `model.rs:785 Journal::account_tags`
  is an **exact-name** lookup. The matcher must use the inheriting walk.
- **The posting model has what the matcher needs** (`model.rs:205-240`): its
  own `tags`, a posting `date` from `; date:`, `status`, `ptype`,
  `balance_assertion`, and `generated` for postings from auto rules.
- **Edits.** `edit.rs:1775 set_txn_tag` is surgical, keeps elisions and
  verifies the result by re-parsing, but it only tags **transactions**. The one
  posting-level comment op, `EditOp` `SetPostingComment`, replaces the whole
  comment verbatim. **No surgical posting-tag op exists.**
- **Batch writes.** `POST /api/transactions/batch` (`review_api.rs:1181`)
  applies `{index, fingerprint, ops}` edits atomically under one lock, at most
  `MAX_BATCH` transactions at a time. Linking a pair is a two-transaction batch.
- **The journal list is windowed by the engine.** `GET /api/journal/rows`
  (`journal_api.rs:516`) serves `{generation, total, offset, rows}` in
  200-row blocks, and the client never holds the whole journal (plan 32,
  "What is true today"). So collapsing two transactions into one row has to
  happen **in the engine**, where `total`, offsets and positions are computed.
  The client can't do it.
- **Problems.** `view/checks.rs` produces `Finding`s (`local_findings` :259)
  that feed the Problems drawer and the per-row severity dot.
- **Review overlay** (`web/src/lib/review/ui/ReviewOverlay.svelte`) is the
  one-card-at-a-time pattern this plan reuses. Its toolbar buttons sit next to
  Add Transaction in `journal/TransactionTable.svelte`.
- **Settings.** Per-browser numeric settings live in
  `stores/settings.svelte.ts` (`reviewMaxDays`, `REVIEW_DAYS_RANGE`), edited in
  `ReviewSettings.svelte`.
- **Reimport dedup** (`reimport.rs`) reads a single transaction-level `id:`
  tag. Linking adds a posting tag and doesn't touch `id:`, so dedup is
  unaffected. (A merge would have broken it. That's one reason linking is
  safer.)

## Decisions

### D1. A link is one posting tag on each clearing posting

```journal
2026-12-31 * ACH AMEX EPAYMENT  ; id:FIT1
    assets:checking             $-500
    assets:transfers:cc          $500  ; match:k7f3a2

2027-01-02 * PAYMENT THANK YOU  ; id:FIT2
    liabilities:amex             $500
    assets:transfers:cc         $-500  ; match:k7f3a2
```

- **It goes on the posting, not the transaction.** One transaction can hold
  two clearing postings (a split transfer, or a fee), and `hledger reg
  tag:match=…` then shows exactly the two legs that cancel.
- **The value is a fresh 6-character base-36 id**, generated to be unique
  across the journal at link time. It means nothing on its own. It's
  readable enough to search for, and short enough to keep the line tidy.
- **Unlinking removes both tags.** The files then read byte for byte as they
  did before the link.
- **`match:ignore`** marks a clearing posting as never to be proposed and
  takes it off the in-transit list. It's for a real one-sided item, such as
  an opening balance moved through the clearing account. It mirrors
  `reviewed:ignore`.
- **The tag names `match` and `nomatch` (D12) are fixed** in v1, like
  `reviewed` in plan 32. A journal that already uses `match:` for something
  else gets a Problems finding (D10), not silent misbehaviour.

### D2. Clearing accounts come from an account directive tag

```journal
account transfers  ; clearing:
```

- The tag is inherited by every subaccount, as in hledger. A posting is a
  **clearing posting** when its inherited account tags include `clearing`.
- This lives in the accounts file, so it's plain hledger, survives without
  Ledgeline, and follows the "classify by declared type/tag, not by name"
  rule. No settings list.
- **The Accounts editor (plan 18) gains a "Clearing account" checkbox.** It
  writes or removes the tag on the directive. Inherited status shows on
  subaccounts as read-only ("inherited from `transfers`").
- **With no clearing account declared, Find Matches is disabled.** Its
  tooltip says how to declare one, and links to the docs. General matching
  without clearing accounts is deferred (see Scope).

### D3. Which postings are candidates

A posting is a candidate when all of these are true:

- It is a clearing posting (D2).
- It is a real posting: not virtual, and not `generated` by an auto rule.
- It has no `match` tag of its own, including `match:ignore`.
- It has exactly one amount, with no cost (`@`/`@@`) and no balance
  assertion.
- Its effective date (the posting's `date:` if present, else the
  transaction's date) falls inside the lookback window (D8).

### D4. What makes a pair

Two candidate postings `a` and `b` can pair when:

- they are in **different transactions**,
- they use the **same commodity** and their amounts are **exactly opposite**
  (`a.qty == -b.qty`, compared as decimals, so there's no tolerance),
- their effective dates are **at most 10 days apart** (the window setting,
  D8),
- they are on the **same clearing account** (decision 7: there is no setting
  to match across clearing accounts; each escrow holds both halves of its own
  transfers), and
- they **don't share a `nomatch` value** (D12).

Exact amounts only: transfers between your own accounts don't carry fees on
one side, and a tolerance is how a refund pairs with the wrong payment. If a
real case appears, revisit it then.

### D5. Undo, and what can never be lost

- Linking writes two tags and nothing else. Every other byte of both
  transactions stays as it is, elided amounts included. It uses the new
  surgical posting-tag op (Phase 1), never PUT.
- Unlinking removes both tags, from the expanded row or the card.
- Editing either half later (amount, account, delete) can break the link.
  The engine never guesses: a `match` id that no longer names exactly two
  clearing postings that cancel becomes a Problems finding (D10), with
  **Unlink** as its fix.

### D6. Assignment: smallest date gap across the whole candidate set

zerosum goes through the file in order and takes the first fit. Ledgeline
instead:

1. Builds every legal pair (D4). It indexes candidates by
   `(clearing account, commodity, |qty|)` so this stays near-linear on a
   200k-transaction journal.
2. Sorts the pairs by `(date gap, -score)` (score in D7).
3. Takes pairs in that order, skipping any that use a posting already taken.

A pair is **ambiguous** when one of its postings had another legal partner
with the same gap and score. Ambiguous pairs are always shown with their
alternatives (D9) and are never part of "Link all clear matches".

A recurring same-amount transfer (say $500 to savings on the 1st of every
month) is the false positive this ordering prevents. The 10-day window is
shorter than the recurrence, so each month's halves pair with each other.

### D7. Score: a tie-breaker, not a gate

The date gap decides the order. The score breaks ties and sets the confidence
label on the card. It's built from the transactions' **other** postings (the
real accounts on each side):

- **+** The two sides' real accounts are different accounts, and each has a
  declared type of asset or liability (cash→card, cash→cash). Account types
  play the role YNAB gives to the bank category.
- **−** Both sides' real accounts are the same account. That looks like a
  reversal or refund, not a transfer.
- **−** Either side's real account has an income or expense type. A
  clearing account shouldn't be reached from one; it's usually a miscoded
  rule.
- **+** Either description names the other side's account (the last segment
  of its name, or a declared `alias`). For example, "AMEX EPAYMENT" next to
  `liabilities:amex`.

A pair is labelled **clear** when it's unambiguous and its score isn't
negative. Everything else is **check**.

### D8. Settings

Per browser, next to the review settings (`ReviewSettings.svelte` becomes the
home for both):

| Setting                       | Default | Range                    |
|-------------------------------|---------|--------------------------|
| Match window (days)           | 10      | 1–60                     |
| Lookback (days)               | 365     | 0 (no limit)–36500       |
| Flag in-transit after (days)  | 10      | 1–365 (D10)              |

The lookback limits which postings are **proposed**. It doesn't limit the
display: a link from three years ago still collapses (D11).

### D9. Find Matches: the overlay

- **Button.** **Find Matches** goes in the journal toolbar with Review Unknowns
  and Review Recents, with the same disabled-when-empty rule and a count badge
  for the number of proposed pairs.
- **Card.** It reuses the review overlay's frame, keys and motion. One pair per
  card:
  - A header naming the transfer: **Checking → Amex · $500.00**.
  - Both halves side by side (date, description, real account, status, file),
    with the date gap between them ("2 days").
  - A confidence label: **clear** or **check**.
  - An ambiguous pair lists its alternatives as radio choices, each with its
    date and description. The nearest is selected by default.
  - **Each half's description is editable** in place, with the review card's
    `DescriptionInput` (payee autocomplete included). The outgoing half's
    description is what the transfer row will show (D11), so it's the one
    the user is most likely to tidy up here.
- **Footer**, following plan 32:
  - **✓ Link** (green, bottom left).
  - **✕ Not a match** and **Skip →** (grey, bottom right). Not a match is
    remembered in the file (D12); Skip lasts for the session only.
  - Once a description changes, **✓ Link** becomes **✓ Link and Save**, and
    **✕ Not a match** saves the edits along with the rejection. A **Revert**
    link restores the originals.
  - A **⋯** menu holds **Ignore this side** (writes `match:ignore` on one
    half, after confirmation).
  - Every write from one card is a single batch: tags and descriptions
    together, or none of them.
- **Bulk.** The overlay's first screen shows the counts ("212 clear, 9 to
  check") and a **Link all 212 clear matches** button, which asks for
  confirmation and then writes the links in `MAX_BATCH`-sized batches. This is
  for adoption: an existing journal with years of clearing postings would
  otherwise need hundreds of clicks. The pairs to check are always reviewed
  one at a time.
- **Skip lasts for the session only.** A skipped pair comes back next time
  unless something better takes one of its halves. **Not a match** is the
  permanent answer (D12).
- **Import.** After **Write changes**, when the import added clearing
  postings, the result screen offers **Find Matches** scoped to pairs where at
  least one half is new. It's the same overlay.

### D10. Problems: in transit, broken links

New `local_findings` (`view/checks.rs`):

| Finding                  | Severity | When                                                                                  | Fix offered        |
|--------------------------|----------|---------------------------------------------------------------------------------------|--------------------|
| In transit               | info     | A candidate (D3) older than the flag setting (default 10 days) has no proposed partner | Find Matches; Ignore |
| Broken match             | warning  | A `match` id is on fewer or more than two postings, or on two that don't cancel, or on a non-clearing posting | Unlink             |
| Unusual clearing posting | info     | A clearing posting has a cost or assertion, so it can never be matched                 | none               |

"In transit" is often the most useful part: a stale clearing posting usually
means a missing import or a wrong rule.

### D11. The journal shows a linked pair as one row

This is the readability Patrick asked for, delivered without editing the
transactions.

- **Where it happens.** The engine's row model (`view/rows.rs`, served by
  `/api/journal/rows`) collapses each linked pair into one **transfer row**.
  `total`, offsets and `/api/journal/position` count the collapsed rows, so
  windowing, the cursor and scroll stay consistent.
- **Where it sits.** At the earliest half that matches the current filter.
  With no account filter, that's the outgoing date. Filtered to Amex, it's
  the Amex half's date, so the Amex register reads in Amex's own order.
- **What it shows.**
  - Date (the anchoring half's date, with "→ Jan 2" if the other differs).
  - Description: the **anchoring half's own description**, unchanged
    (decision 6). With no filter, that's the outgoing half. In the accounts
    column, the row shows the two real accounts (**assets:checking →
    liabilities:amex**) through the same account cell as every other row,
    so they're shortened the same way when space is tight. That's where the
    readability comes from. (First built with last-segment labels; changed
    2026-11-09 so the row lines up with every other row's display.)
  - Editing the description from the row edits the anchoring half's
    description (`SetDescription`, the existing surgical op). The other half
    is edited from the expanded view.
  - Amount: the transferred amount. Filtered to one side, it's that side's
    signed amount, as any row would show.
  - A small ⇄ link icon.
  - Direction comes from the signs: the side whose real posting is negative
    is the source.
- **Expand.** It shows both transactions as ordinary rows, with every
  description, comment, tag, status, posting and file intact. Each half opens
  in the review card for editing, as any row does.
- **Partial filters.** If only one half passes the filter, the row still
  appears as a transfer row anchored on that half, and it still expands to
  both.
- **Totals and charts are unchanged.** The clearing legs cancel within a
  pair, and every report runs on postings, never on rows. The journal
  summary's transaction count counts rows as displayed, labelled "rows" when
  any pair is collapsed. That needs to be checked against the summary's
  current wording.
- **Always on.** No toggle (decision 4).
- **Search.** A transfer row's haystack is both halves' haystacks, so
  searching either description finds it.

### D12. Rejections are remembered as a shared `nomatch` tag

**Not a match** writes the same fresh id to both clearing postings:

```journal
2026-12-31 * TRANSFER TO SAVINGS
    assets:checking             $-500
    assets:transfers:bank        $500  ; nomatch:r4x9

2027-01-04 * ONLINE TRANSFER FROM CHK
    assets:savings               $500
    assets:transfers:bank       $-500  ; nomatch:r4x9
```

- **A pair is rejected when its two postings share a `nomatch` value.** The
  id names the pair, not either posting, so it doesn't depend on a bank
  `id:` (many transactions have none) or on file positions (they shift).
  It's symmetric, and it survives any edit that keeps the posting.
- **A posting can carry several**: `; nomatch:r4x9, nomatch:b2c1`. Verified
  against hledger 1.52: repeated tags on one posting read back, and
  `reg tag:nomatch=r4x9` lists exactly the two halves of the rejected pair.
- **Values are generated like `match` ids** (D1), from a shared id space, so a
  `match` and a `nomatch` never collide.
- **Undo.** The expanded row and the posting's tag chips offer **Forget
  rejection**, which removes that value from both postings.

**Keeping it quiet.** Patrick's concern is noise. Four rules bound it:

1. **Only an explicit "Not a match" writes one.** Skip writes nothing, bulk
   link writes no rejections, and the matcher never writes one on its own.
2. **One value per rejected pair, on the clearing posting's own comment
   line**, not a new comment line on the transaction. The visible cost is a
   short tag on a line that's usually empty.
3. **A rejection is cleaned up when it stops mattering.** Once either posting
   in a rejected pair is linked (or marked `match:ignore`), it can never be
   proposed again, so the rejection is dead weight. The link batch removes
   every `nomatch` value the newly linked postings carry, from them **and**
   from each posting on the other side of those rejections. A pair that's
   been sorted out ends with only its `match` tags.
4. **A rejection only exists where the matcher proposed a pair.** It's
   limited by the window (10 days) and same-account rule, so a posting
   rarely collects more than one or two.

What's left over time is a `nomatch` tag on postings that are still genuinely
in transit, which is the list the user is working through anyway.

A `nomatch` value found on exactly one posting (its partner was deleted or
edited off the clearing account) is a stale rejection. It's harmless, gets
removed by the same link-time cleanup, and is **not** a Problems finding,
because it would be noise.

## Scope

In:

- Clearing accounts by account tag, with an Accounts-editor checkbox.
- The matcher (engine), the Find Matches overlay with bulk link, and the
  import entry point.
- The posting-tag edit op, plus link and unlink through the batch endpoint.
- Collapsed transfer rows in the journal.
- Problems: in transit, broken link, unusual clearing posting.
- Docs.

Out (deferred, each needs its own decision):

- **Merging** pairs into one transaction (ruled out by decision 1). If it
  ever comes back, it needs posting-level `id:` tags plus reimport index
  support, posting `date:` instead of `date2`, and a cross-file atomic write.
  It would also give up the in-transit asset at period end.
- **General matching** without clearing accounts, such as checking →
  `expenses:unknown` paired with amex ← `income:unknown`. Linking those as a
  "transfer" would be misleading while the unknown legs carry the amounts, so
  accepting would have to recategorize both legs to a clearing account first.
  That's an account edit, which is a different consent from a tag.
- **Many-to-one** (one transfer out landing as two deposits).
- **Tolerance** on amounts.
- **Matching across clearing accounts** (decision 7: not wanted).

## Phase 1 — Engine (core only, PR 1)

- `JournalEditor::set_posting_tag(index, posting, name, Option<value>)`,
  following `set_txn_tag`. It's surgical: the posting's same-line comment
  gains or loses the tag, nothing else moves, elisions stay elided, and the
  result is re-parsed with a round-trip check. Add a matching
  `EditOp::SetPostingTag`.
- `nomatch` needs **multi-valued** tags, which `set_txn_tag`'s
  replace-all-occurrences semantics can't express. Add two posting ops:
  `AddPostingTagValue(posting, name, value)` (no-op if that exact value is
  already there) and `RemovePostingTagValue(posting, name, value)` (removes
  only that occurrence and its separator). `match` uses the set/remove-all
  form; `nomatch` uses these.
- `matching.rs` (new):
  - `clearing_accounts(journal)`, through `inherited_account_tags`.
  - `candidates(journal, settings)` (D3).
  - `propose(journal, settings, today) -> Vec<Proposal>` (D4, D6, D7), where
    `Proposal { a: PostingRef, b: PostingRef, gap_days, score, clear, alternatives }`.
  - `links(journal) -> LinkIndex`: valid pairs by id, plus the broken ones.
  - `rejections(journal)`: the set of rejected posting pairs (D12).
  - `new_match_id(journal)`, unique across `match` and `nomatch` values.
  - `link_ops(journal, a, b) -> Vec<(Tindex, Vec<EditOp>)>`: the two `match`
    tags plus the D12 cleanup (every `nomatch` value on `a` or `b` removed
    from both of its postings). Built in the engine so the server and any
    future client agree on what a link writes.
- Findings for D10 in `view/checks.rs`.
- Tests:
  - Unit tests: recurring same-amount transfers, ties and ambiguity,
    posting dates crossing the window, different commodities, cost or
    assertion exclusion, `match:ignore`, inherited and exact clearing tags,
    and broken-link detection.
  - Rejections: a rejected pair is not proposed, but each half can still pair
    with someone else; linking cleans up rejections on both sides and leaves
    unrelated ones alone; add/remove of one value among several leaves the
    others byte for byte.
  - A proptest: no posting appears in two proposals, and every proposal
    cancels exactly.
  - An opt-in hledger parity check that `bal tag:clearing not:tag:match`
    agrees with the in-transit total.
  - A criterion bench on the 200k corpus (`benches/corpus.rs` already emits
    transfers; route them through a clearing account).

## Phase 2 — Wire (server, TS types, PR 2)

- `GET /api/matches?window=&lookback=&since=` returns
  `{generation, counts: {clear, check}, proposals}`. Each proposal carries
  both halves as review rows (`ReviewRow`) plus their fingerprints, so a link
  goes through the existing optimistic-concurrency path. `since` scopes to the
  pairs that involve new postings (the import entry point).
- `POST /api/matches/link`, `/reject`, `/unlink` and `/forget` take the two
  postings (index, position, fingerprint) plus optional description edits for
  each half. The server builds the ops with `link_ops` and friends and applies
  them as one batch, so the D12 cleanup can't be skipped by a client, and it
  checks that a link cancels exactly, so a stale client can't create a broken
  link. Bulk link takes a list of pairs and applies them in `MAX_BATCH`-sized
  batches.
- The batch endpoint also gains `EditOpIn::SetPostingTag`,
  `AddPostingTagValue` and `RemovePostingTagValue`, so the tag chips in the
  ordinary review card can edit posting tags.
- Rows: `Row` gains an optional `transfer: { matchId, halves: [Row, Row],
  from, to }`, and the rows, position and summary endpoints count collapsed
  rows (D11).
- Endpoint tests (`*_endpoints.rs`, temp-copy journal): propose, link, the
  bytes on disk, re-propose (the pair is gone), unlink (the bytes are restored
  exactly), reject (the pair isn't proposed again; each half still pairs
  elsewhere), link-with-edits (descriptions and tags in one write), link
  cleanup of `nomatch`, and a stale fingerprint is refused.
- Run `just test-integration` after these shape changes (memory:
  live-integration-tests-skip).

## Phase 3 — Journal UI (PR 3)

- The Find Matches button, overlay, bulk screen and keys (D9), reusing
  `ReviewOverlay` parts (`ReviewFooter`, `ReviewHero`, motion).
- Transfer rows: collapsed rendering, expand/collapse (keyboard: `Enter` or
  `l` on the row, to be confirmed against the keymap's free keys), the ⇄ icon,
  and Unlink in the row menu.
- The "Clearing account" checkbox in the Accounts editor.
- The settings from D8.
- Vitest with `fakeEngine`. The new fixture journal is
  `fixtures/matching.journal`, **not** `sample.journal`, because e2e asserts
  exact totals there.

## Phase 4 — Import entry point and docs (PR 4)

- The import result screen offers Find Matches (D9).
- `docs/journal/transfer-matching.md`:
  - clearing accounts and why they're correct practice,
  - the `clearing:` tag,
  - what a link writes (one tag per side, shown in a journal snippet),
  - reading the in-transit balance at month end,
  - the hledger CLI equivalents (`bal tag:clearing not:tag:match`,
    `reg tag:match=ID`).
- Screenshots through `web/docs-screenshots`, with a `MANIFEST.md` row.
- `TODO.md`: replace the Matching entry with a pointer to this plan.

## Mockups

### Card

```
┌──────────────────────────────────────────────────────────────┐
│  ⇄  Checking → Amex                               $500.00     │
│      clear · 2 days apart                                     │
├───────────────────────────────┬──────────────────────────────┤
│  Dec 31, 2026   ✓ cleared      │  Jan 2, 2027   ✓ cleared     │
│  [ACH AMEX EPAYMENT        ]   │  [PAYMENT THANK YOU       ]  │
│  assets:checking    −$500.00   │  liabilities:amex   $500.00  │
│  via transfers:cc              │  via transfers:cc            │
│  checking.journal              │  amex.journal                │
├───────────────────────────────┴──────────────────────────────┤
│  ✓ Link                       ⋯   ✕ Not a match    Skip →     │
└──────────────────────────────────────────────────────────────┘
```

### Journal rows

```
Dec 31  ⇄ ACH AMEX EPAYMENT  → Jan 2   Checking → Amex      $500.00   ▸
Jan 03    Whole Foods                 expenses:groceries     $84.12
```

Expanded:

```
Dec 31  ⇄ ACH AMEX EPAYMENT  → Jan 2   Checking → Amex      $500.00   ▾
          Dec 31  ACH AMEX EPAYMENT   assets:checking    −$500.00
          Jan 02  PAYMENT THANK YOU   liabilities:amex    $500.00
```

## Risks

- **Row collapsing is the deepest change.** Every endpoint that counts or
  positions rows (rows, position, summary, chart click-through, cursor
  restore after an edit) must agree on what a row is. Build it once in
  `view/rows.rs` and test the pair-across-a-block-boundary case directly.
- **First run on an old journal** may propose thousands of pairs. The bulk
  screen exists for that, but `MAX_BATCH`-sized batches each re-parse the
  journal (plan 32 "Speed"). Measure on the 200k corpus. If it's slow, add a
  bulk tag op that parses once.
- **Two clearing postings in one transaction**, such as a transfer plus a
  fee through the same account. Each posting pairs on its own, so matching
  works, but if both are linked the transaction belongs to two pairs and
  can't collapse into either row without hiding the other. Rule: a
  transaction in more than one link is **not collapsed**. It and its partners
  show as ordinary rows, each with the ⇄ icon. Test it.
- **Rejection noise** (Patrick's concern). D12 bounds it, but measure it on
  the real journal after a first pass: count `nomatch` values left after
  bulk link plus a session of checking. If it's more than a handful per
  hundred clearing postings, revisit (for example, a tighter default window).
- **The `match` tag name could collide** with an existing user tag. D10's
  broken-match finding surfaces it rather than silently pairing nonsense.

## Resolved with Patrick (2026-11-09)

1. **Remember rejections**, in comments on the transactions, tags acceptable,
   while keeping the noise down → D12 (`nomatch`, written only on an explicit
   Not a match, cleaned up at link time).
2. **The transfer row shows the original description**, editable, including
   from the matcher card → D9, D11. The accounts column carries
   "Checking → Amex".
3. **No matching across clearing accounts** → D4; the setting is gone.

## Status

Implemented 2026-11-09, phases 1–4: the engine (`matching.rs`, the posting-tag
ops, D10 findings, collapsed rows), the wire (`match_api.rs`), the journal UI
(Find Matches, transfer rows, the Accounts checkbox, settings), the import
entry point, `docs/journal/transfer-matching.md`, `fixtures/matching.journal`
(pinned by `crates/ledgeline-server/tests/match_fixture.rs`), the docs
screenshots (`web/docs-screenshots/matching.docs.ts`) and a criterion bench
(`crates/ledgeline-core/benches/matching.rs`).

Where the build differs from the plan above:

- **`since` is `JOURNAL_ID:COUNT`**: the last `COUNT` transactions of that
  file, which is what a CSV commit appends (`journalWritten`, `imported`).
  A QuickBooks import inserts by date, possibly into several files, so its
  result screen offers Find Matches over the whole journal instead; so does a
  CSV result once **Re-sort by date** has moved the new transactions.
- **`GET /api/matches` also says `clearing`** (whether any account declares
  `clearing:`), which disables the button and drives its tooltip and **How?**
  link. Proposals are ordered by the earlier half's effective date, then its
  file position, so a journal with one file per feed still reviews in date
  order.
- **Each half carries `file`** (the file name only), for the card's last line.
- **Two more write routes**: `POST /api/matches/ignore` (Ignore this side) and
  `POST /api/matches/link-all` (bulk link, written in `MAX_BATCH` batches
  server-side, one request from the client).
- **`collapse=false` on `/api/journal/rows`** serves transactions, not rows:
  the xlsx export reads every transaction, never the collapsed rows.
- **Keys**: `m` opens Find Matches; `l` expands or collapses a transfer row
  (`Enter` stays "edit the transaction"). In the overlay, `y`/`Enter` link,
  `x` rejects, `→`/`n`/`l` skip, `a` links all clear, `c` cycles the
  alternatives, `1`/`2` edit a description, `z` reverts.
- **Forget rejection** is a small ≠ badge on the row of a posting carrying
  `nomatch:`, not a tag-chip action; **Unlink** is a hover icon on the
  transfer row's description. Both confirm in the toolbar bar, as delete does.
- **Skip lasts one overlay run**, not the browser session: closing the
  overlay forgets the skips.
- **The footer counts "rows"** (instead of "transactions") whenever any pair
  is collapsed (`summary.transfers > 0`).
- **In transit uses the engine's defaults** (10-day window, flagged after 10
  days). The per-browser "In transit after" setting is stored and shown but
  the Problems check does not read it yet: findings are computed per
  generation for every client, not per browser.
- **A value-less account tag is written `clearing:`**, with no trailing space
  (`accounts.rs` `render_suffix`); hledger reads it the same.

Perf on the 200k corpus with its transfers split through a clearing account
is in the bench's module docs and the Phase 4 report.

## Amendment A — Problems you can clear, lookback control, many-to-one (2026-11-09)

### The ask (Patrick, after first use)

> I set my first clearing accounts and suddenly I had 354 problems flagged
> (has been in transit since). Meanwhile on the journal page it shows 54
> possible matches. … I think this might leave the user with problems they
> can't clear. … With the problems tab, it takes me back to something that
> shifts money to a clearance account, but in bringing up the transaction
> editor, there's no indication of a possible match from there. … If a
> transaction is being looked at, particularly in the editor or if it has a
> flag for a problem regarding matching, then there should be a way to pop up
> the matcher for the transaction. "Review Recents" lets you change the filter
> with a dropdown attached to the button. We should do the same for the Find
> Matches button so someone can adjust the filter lookback.

Also: **matching stays limited to clearing accounts** (general matching is
not wanted for now).

### What the real journals showed

A read-only pass over Patrick's two journals (`hledger reg tag:clearing
not:tag:match`, then each unlinked posting's nearest exact opposite on the
same account):

| Journal  | Unlinked | Exact partner ≤ 10 d | No exact partner | Exact partner > 10 d only |
|----------|----------|----------------------|------------------|---------------------------|
| Personal | 216      | 198                  | 18               | 0                         |
| ICL      | 655      | 300                  | 264              | 91                        |

- **The 354 problems are ICL's**, and they are mostly **one bank debit paying
  several bills** through Bill.com: on 2019-02-19 a $2,247.50 debit is a
  $2,000 bill plus a $247.50 bill. One-to-one matching can never clear these.
- **Personal shows the other gap**: pairs older than the 365-day lookback are
  neither proposed nor flagged (the in-transit check finds them a partner), so
  they are invisible. 18 car-loan postings are genuinely one-sided (the loan
  side was never imported).

### Decisions

**A1. Find Matches gets a ▾ dropdown, like Review Recents.** It holds the
window and the lookback (presets 90 days, 1 year, 2 years, All time, plus the
number), and changing them re-reads. `GET /api/matches` also returns
`older`: how many more proposals there would be with no lookback. When it's
above 0 the first screen and the button tooltip say "N more older than the
lookback", with a **Show all** action that sets the lookback to All time.

**A2. A matcher for one transaction.** `GET /api/matches/for/{index}`
returns, for each of that transaction's unlinked clearing postings (not
`match:ignore`):
- **exact partners**: unlinked postings on the same clearing account with the
  exact opposite amount, at **any** date gap (no lookback, no window), not
  rejected with it, nearest first, at most 20;
- **suggested combinations** that include it (A4);
- **nearby**: unlinked opposite-sign postings on the same clearing account
  within ± the window (the ▾ setting, widenable in the focused view), at most
  50, for picking a many-to-one group by hand.

The overlay opens in **focus mode** on that posting: exact partners as radio
choices (nearest selected), then the combinations, then the nearby list with
checkboxes and a running total ("$2,247.50 of $2,247.50 ✓"). **Link** is
enabled only when the selection sums to exactly zero. **Not a match** applies
to a single partner only. **Ignore this side** is always there. With nothing to
offer, it says why ("No unlinked −$506.55 on transfers:car-payment") and
offers Ignore.

Entry points:
- the **transaction editor** (the review card): a **⇄ Find match** button when
  the transaction has an unlinked clearing posting; a linked one shows its
  link and **Unlink** instead;
- the **Problems drawer**: the in-transit finding's fix is **Find match**; the
  broken-match finding's fix stays **Unlink**;
- the journal row: **Find match** in the row's hover actions on a row with an
  unlinked clearing posting, and a key for it while the cursor is on one.

**A3. A link can be a group.** A `match` id on N ≥ 2 postings is valid when
they are on the **same clearing account**, in the **same commodity**, in **N
different transactions**, and sum to **exactly zero**. The broken-match rule
changes to match. Linking writes the same id on every member, and D12's
`nomatch` cleanup runs for each. Unlink removes the id from all of them.
Groups of more than two are **not collapsed**: their rows carry ⇄ like a
transaction in two links. `nomatch` stays pairwise.

**A4. Suggested combinations.** For each unlinked posting that the
one-to-one pass left without a partner, look among opposite-sign unlinked
postings on the same account within ± the window (nearest 20) for a subset of
**2–4** that sums to exactly its negation. Propose a combination only when it
is the **only** such subset (otherwise it would be a guess), take them in
order of the widest gap, and never let one posting into two proposals. They
are always **check**, never part of **Link all**. The card shows the single
posting on one side and the group on the other.

**A5. In-transit stays "no proposed partner"**, but a posting in a suggested
combination now counts as having one. The finding text says "Find match" is
available.

The ▾ settings stay per browser, and the default lookback stays 365 days.

### Card header labels (Patrick, 2026-11-09)

The card's big "checking → amex" header stays short. Each side is its
account's **shortest trailing run of segments that no other account in the
journal ends with** (`matching::AccountLabels`, built once per journal from
declared and posted accounts): a lone `assets:bank:checking:cash` reads
`cash`, two `…:checking` accounts read `bank1:checking` / `bank2:checking`,
and a name that is itself a suffix of another (`checking` beside
`assets:checking`) reads in full. This replaced the first rule (last segment,
full names only when the two sides collided), which still showed `checking →
checking`-style ambiguity whenever the journal had several accounts with the
same last segment. Journal rows show full account names (D11).

### Status (Amendment A, UI)

Implemented 2026-11-09: the ▾ on Find Matches, combination cards, the
focused matcher and its four entry points, docs. Where the build differs from
A1–A5 or adds to them:

- **The ▾ is `ReviewSettings show="matches"`** (window, lookback, in-transit
  age) with lookback presets (90 days / 1 year / 2 years / All time) beside the
  number; the presets also appear in Review Recents' ▾ and the overlay's
  sliders, which render the same component. Find Matches stays enabled when
  nothing is proposed but `older > 0`, so its first screen can offer **Show
  all**.
- **Combination cards**: only the single's description is editable (the group
  is listed read-only with `MatchLine`); Link sends `{members: [a, ...group]}`;
  Not a match and `x` are absent; ⋯ offers Ignore this side for the single only.
  Link all already excluded them (`clear` is false); a test pins it.
- **Focus mode is a view inside the same overlay** (`MatchFocus.svelte`), not
  a second overlay. Its key layer is `PRIORITY.overlay + 1`: mounted together
  with the overlay, a child's `registerKeys` effect runs first, and the
  overlay's modal layer would otherwise blind it. Keys: `y`/`Enter`/`⌘↵` link,
  `c` next radio, `x` Not a match (one exact partner), `i` Ignore (asks), `Esc`.
- **One selection model**: exact radios and combination radios are presets of
  the same picked set the nearby checkboxes edit, so a combination's members
  show ticked below. One partner links as a pair, several as `{members}`
  (focused posting first, at most 16). The running total is `selectionTotal`
  (exact `Dec` sums); it also refuses two postings of one transaction or a
  second commodity.
- **Flags on exact partners** are worked out in the browser: gap > the ▾
  window, or the partner's transaction date before today − lookback.
- **The focused window** starts at the ▾ setting and is changed by a number
  beside "Pick several" (1–60, the server's range); it does not change the
  setting.
- **After a write** the store re-reads `/for` with the proposals. A link or
  ignore that leaves the transaction no open clearing posting closes the
  overlay; otherwise it stays on what is left. Unlink stays.
- **Entry points read the served transaction**, not `/for`: a served
  posting's tags include its account's inherited `clearing` tag
  (`wire::posting_tags`), so `findableClearing(txn)` (clearing, real, no
  `match` tag, one amount, no cost, no assertion) decides whether ⇄ Find match
  is offered, and `linkIdsOf(txn)` lists the links for Unlink.
  - Editor (single edit mode only, not queue cards): a line under the header.
    Find match is disabled while the card is dirty; the page's overlay chain
    swaps the editor out and remounts it, re-read, when the matcher closes.
    Unlink fetches `/for` for the link's members, names all of them, then
    re-opens the editor (`txnModal.openEdit`) on the rewritten transaction.
  - Journal row: a hover ⇄ button, and `⇧M` (free in the journal keymap) on the
    cursored row.
  - Problems drawer: in-transit → **Find match** (jumps to the row, then opens
    the focused matcher); broken-match → **Unlink** (asks; unlinks every
    *broken* link on that transaction via `/for`, naming every member). The
    drawer now labels `in-transit`, `broken-match` (“Broken match”) and
    `unusual-clearing`.
- No new screenshot: the focused view is described in
  `docs/journal/transfer-matching.md` without one.
