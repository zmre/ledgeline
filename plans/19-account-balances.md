# 19 — Account Balances, a second view inside the Insights box

A **Balances** tab beside the journal tab's existing P&L insights, showing cash
on hand, short-term liabilities and net cash, then the leaf accounts behind
them with the date each was last touched. Driven by the TODO entry "account
balances".

## The ask

> "One of the most basic and important things a finance app does is help the
> user understand cash balances. We implicitly have this under our balance
> sheet report, but we need a quick view. […] I'm only interested in cash
> accounts and liabilities. In short, just the things under cash and cash
> equivalents on the balance sheet and liabilities on the balance sheet. If
> accounts are tagged as current/non-current for assets and liabilities, then
> filter down further to current. […] There are three pieces of information:
> leaf accounts with cash balances (or liabilities, but don't intermingle), the
> amount, and an as-of date."

## Decisions (locked with Patrick 2026-09-13)

1. **Always as of today; the journal's date filter is ignored.** A balance is
   cumulative, so the filter bar's 90-day default would understate every
   figure — and the question the box answers is "what do I have right now",
   which must not move when you scrub the dates. Future-dated postings are
   excluded for the same reason: a scheduled entry is not money you have.
2. **The account filter is ignored too.** Filtering the journal to `expenses:`
   must not blank the cash panel. Balances is a standalone snapshot, not a view
   of the current selection.
3. **Computed client-side, behind one swappable function.** The whole journal
   is already in memory (`journal.txns`), so this is instant and needs no
   fetch, no loading state and no refresh coordination — matching how the rest
   of the insights box works. `cashViewClassifier` is the single seam; see
   "Where this could drift" below for what it costs and how to buy it back.

## What counts

Two declared tags decide membership:

| Question             | Answer                                            |
| -------------------- | ------------------------------------------------- |
| Is it cash?          | effective `type:` is hledger's Cash subtype (`C`) |
| Is it a liability?   | effective `type:` is `L`                          |
| Does it still count? | effective `bsterm:` is not `noncurrent`           |

Both tags inherit to sub-accounts, resolved by walking up the path — the same
rule `type:` already follows everywhere else in this app. `resolveAccountType`
was already there; `resolveBsTerm` is new, in `lib/domain/accountTerms.ts`, and
is the TypeScript mirror of `reports::account_groups::parse_bs_term_tag`
spelling for spelling, including every synonym (`short`, `long-term`, …) and
the refusal to guess at anything outside that closed vocabulary.

**The current/non-current filter is adaptive for free.** A journal that
declares no `bsterm:` anywhere has nothing resolving to non-current, so nothing
is excluded — which is exactly the behaviour the balance sheet's own split has,
arrived at without a second code path.

**Getting the tag to the browser needed one small change.** `normalizeAccounts`
read the `type:` tag off `/accounts` and threw every other tag away, so
`AccountDecl` gained a `bsterm` field and the normalizer now parses it. It is
optional on the interface so the many `{name, type}` fixtures still compile.

## The one place a NAME gets a vote

Added 2026-09-13 at Patrick's request: an untagged `liabilities:mortgage`
should not sit in "short-term liabilities". So `inferBsTerm` guesses a term
from the account's name — and it is deliberately the narrowest thing that
answers the ask.

It runs **last**, after the account and every ancestor have been asked for a
`bsterm:` tag, which is exactly where hledger puts its own name heuristic
inside `resolve_account_type`. A tag always wins, in both directions: `bsterm:
current` on a home-equity line brings it back. The Rust balance sheet does
**not** do this, so an untagged mortgage is out of this view and still under
`Current` on the Balance Sheet tab — the one visible inconsistency this change
introduces, and the reason a tag is still the right answer.

### Why the list is so short

The words come from a survey of 108 public plain-text-accounting repositories
(2,057 distinct account paths) plus the GnuCash, QuickBooks and Xero default
charts. The selection rule was **precision, not coverage**, because the two
errors are not symmetric: a miss leaves a long-term debt in the short-term
figure, where the reader can see it and tag it; a false positive removes a real
debt from the screen entirely.

| Kind | Words |
|------------|--------------------------------------------------------------|
| either | `noncurrent`, `non-current`, `longterm`, `long-term` |
| liability | `mortgage`, `mortage`, `heloc`, `student`, `pension` |

`mortage` is in there because it is the observed misspelling. `student` rather
than `studentloan` because the corpus never once spelled it as one word — real
paths are `liabilities:loans:student` and `Liabilities:Loan:Student`.

**Rejected, all of which are usually long-term:** `loan` (a personal loan may
be due this month), `car` / `auto` / `vehicle`, `note`, `bond`, `lease`,
`deposit` (a security deposit is non-current; a bank deposit could not be more
current), `investment` (`assets:investments:vanguard:cash-plus` is the literal
plaintextaccounting.org example), `savings`, `property`, `home` / `house`,
`capital`, `fixed`, `principal`, `escrow`, `treasury`, `brokerage`, `hsa`,
`529`, `payable` / `receivable`, and every short abbreviation — `ltd` is a
company suffix ("Wilson Ltd"), `sep` is September, `re` only ever appeared as a
substring of `retained-earnings`, `cd` and `mtg` and `ppe` likewise. A car loan
really will be missed.

**One phrase inverts the reading and is vetoed before anything else.**
"Long-Term Debt, Current Maturities" is a real FASB line item and a *current*
liability — the slice due inside a year. A plain search for `long-term` gets it
exactly backwards, so `current-maturities`, `current-portion` and `due-within`
stand the guess down entirely.

### Asset-side words were researched and deliberately not wired up

The survey also produced a defensible asset list — `401k`, `403b`, `457b`,
`rrsp`, `ira` (exact segment only; catastrophic as a substring, cf. `lira`),
`roth`, `intangible`, `goodwill`, `land`, `buildings`, and the bigrams
`accumulated depreciation` / `accumulated amortization`. None of it is used
here, and the reason is structural rather than cautious: **every asset in this
view is an account its owner declared `type: C`**, and overruling an explicit
"this is cash" on the strength of a word in the name is the move the whole
name-matching rule exists to forbid. The list is recorded here for whoever
mirrors this into the Rust balance sheet, where plain assets do appear.

### The guess is never silent

An excluded account is invisible by definition, which is what makes a wrong
guess dangerous. So `cashViewClassifier` returns `"guessed-long-term"` as a
distinct verdict from `null`, `guessedLongTerm()` collects those accounts off
the same memoized pass, and the view prints them under the lists with the tag
that overrides them. Accounts excluded by their own `bsterm:` tag are **not**
listed — the owner wrote that tag, and repeating their decision back at them is
noise, not disclosure.

## Where this could drift, and the seam that fixes it

The Balance Sheet tab resolves its "Cash and cash equivalents" line through the
Rust engine's five-step `AccountGroups::resolve`, whose *first* two steps are an
explicit `bsgroup:` tag on the account or on an ancestor. This view does not
reimplement those steps, so an account deliberately moved **onto** the Cash line
by `bsgroup: Cash and cash equivalents` without being typed `C` — or moved
**off** it onto a line of its own — is classified differently in the two places.

That is a deliberate, bounded trade, not an oversight. Steps 1–2 are the rare
case; step 3 (effective type is Cash) is how essentially every journal gets its
cash line, and reimplementing the group engine in TypeScript to catch the
remainder would be the duplication this codebase has removed twice already
(the stock pools, the period math).

The buy-back is cheap and pre-planned: `cashViewClassifier` in
`lib/reports/cashBalances.ts` is the only place membership is decided.
`GET /api/reports/balancesheet/grouped` already returns per-account rows tagged
with their group and their term, so swapping that one function for a call to it
changes nothing else in the module — the as-of dates are computed from the
in-memory postings either way, since no endpoint carries them.

## The as-of date

Per account, the effective date (`posting.date ?? txn.date`) of its most recent
counted posting, in any commodity — "when did this account last move", which is
the question a staleness figure answers. A balance check imported into the
journal is a posting like any other, so it dates the account exactly as the ask
describes; when the *latest* posting carried a balance assertion the row also
says **checked**, because a figure that was verified against a statement is
worth distinguishing from one that was merely accumulated.

An older assertion does not confer "checked" on a newer, unverified figure.

## Signs, and the one that is a problem

No sign is applied anywhere. hledger already records a liability you owe as a
negative balance and cash you hold as a positive one, which is exactly the
display the ask asks for — so "liabilities show as negative" cost no code, and
the numbers agree with `hledger bal` by construction.

Which leaves the interesting case: a cash account whose balance is **negative**
is overdrawn, or the journal is wrong. Those rows are red, and they are also
reported in Problems by a new `negative-cash` rule. The rule is anchored to the
ACCOUNT rather than to a transaction — no single entry is at fault, the running
total is — which makes it the second such finding after the engine's
`account-tag`, a shape the drawer already renders.

Liabilities are not checked: a negative balance is what owing money looks like.
The **Short-term liabilities** tile is muted rather than red for the same
reason. Only an individual overdrawn cash account, and net cash going negative,
earn the alarm colour.

## One pass, two consumers

The rule and the view need the same whole-journal accumulation, and the view is
mounted while Problems is recomputing on every journal swap. `accountBalances`
is therefore memoized on the identity of `(txns, decls, asOf)` — the same
`WeakMap`-of-`WeakMap` shape, and the same immutability assumption, as
`signConventions` in `lib/insights/series.ts`. Weak keys so an entry dies with
the journal it describes instead of pinning a 150k-transaction array for the
life of the process.

## Layout, and the height budget

The journal page is `height: calc(100dvh - 7rem)` with only the transaction
table scrolling, so every pixel this box takes comes straight out of the table.
Hence:

- The account lists live in one `max-h-48 sm:max-h-56 overflow-y-auto`
  container, with sticky "Cash" / "Liabilities" subheads inside it. Both lists
  scroll together, so the two never intermingle but also never each get a
  scrollbar.
- The three tiles are `stats-horizontal` at **every** width, unlike the
  activity tab's, which stack on mobile. Three short money figures fit across a
  phone, and stacking them cost ~140px on exactly the screens with least to
  spare.
- The cash pie is `hidden lg:block`. It is the ask's "if we have space", and on
  a phone there is none.

### Account names use the width the panel actually has

The first cut rendered `<AccountLabel name={…} />` with nothing else, which
falls through to its unmeasured fallback: a **thirty-character** budget tuned
for the journal's accounts column. In a panel several hundred pixels wider,
`liabilities:creditcards:chase-sapphire` came out as
`lia:creditcards:chase-sapphire` for no reason at all.

`AccountLabel`'s own notes are emphatic that characters are not a unit of width
in a proportional font and that measuring is the design, so each row now
measures its own name cell (`AccountBalanceRow.svelte`) and hands the label the
room it really has. Two details make that safe:

- **Per row, not per list.** `justify-between` gives the name cell whatever the
  amount beside it did not take, and amounts differ in width, so one number for
  the list would be wrong for most of it. The cost is bounded — this list is
  short and does not virtualize, which is exactly why the journal table shares
  a single observer on its column header instead.
- **It cannot oscillate.** The name cell is `flex-1 min-w-0` (`flex-basis: 0%`),
  so its width comes from the container and the amount beside it, never from
  its own content. A longer label cannot widen the box that decided how long
  the label may be — the same argument `accountColumn.svelte.ts` makes from
  `table-layout: fixed`.

`textWidth.ts` grew a font size parameter for this. It measured at 12px only
(daisyUI `badge-sm`), and these names render at `text-sm`; borrowing the 12px
context would have answered about a sixth narrow — the exact silent wrongness
that module refuses elsewhere. Contexts and width caches are now keyed by size,
and `chipMeasurer()` is `textMeasurer(12)`.

The pie draws positive cash only, biggest first, tail folded into `(other)` at
the same six-group cap the activity chart uses. Liabilities are excluded
because a pie divides a whole by area and a debt has none — the same reasoning
already written down for the activity pie's negative slices.

## Why the tab strip is inside the collapse body

daisyUI's `collapse` lays its toggle checkbox over the whole title row, so a tab
button beside the heading would have swallowed its own click and shut the box.
The strip is therefore the first row of `collapse-content`. The header keeps
showing the active view's headline figure while collapsed — `Net` for activity,
`Net cash` for balances — which is the point of a box that collapses rather than
one that hides.

Both the active tab and the "hide zero balances" checkbox persist in
`settings`, validated against their union on load so a stale blob naming a tab
that no longer exists cannot render an empty box with no way back.
