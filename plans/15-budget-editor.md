# 15 — Budget editor, and Budget as a top-level tab

Makes the budget writable. Ledgeline could already *report* against `~` periodic
rules (`reports/budget.rs`, WP-06); this adds the other half — creating and
editing those rules from the GUI — and moves Budget out of the Reports tab strip
onto a route of its own. Driven by the TODO entry "feat: edit / create budget".

## The ask

> "figure out where budget rules already exist and that's where we'll store new
> lines and update existing ones. if they don't exist, make a budget.journal file
> and include it from the main file (with a button press by user first). when
> adding or editing a budget line, we need to say if it's weekly/monthly/annual,
> pick a category and it should show us recent values for that time period for
> that account… Then I set a dollar amount. Or edit a dollar amount. Move Budget
> from Reports to its own top-level tab."

## Decisions (locked with Patrick 2026-08-30)

1. **One block per interval.** A new goal joins the existing `~` rule of its
   period, or creates `~ monthly  monthly budget` if there is none. The UI never
   mentions blocks; you pick weekly/monthly/quarterly/annual. A journal that
   already has two monthly blocks keeps both — we append to the first and leave
   the second alone.
2. **Magnitude in, sign out.** You type `1200` for an income goal and the journal
   gets `$-1200`, because the account is income-typed. The reference figures are
   shown in the same orientation. Signs never appear in the UI unless you type one.
3. **Reference figures are subaccount-INCLUSIVE.** `expenses:food` reports food
   plus `food:dining` plus `food:groceries` — matching how the budget report
   already aggregates a parent's goal from its children, so the reference and the
   bar it informs cannot disagree.
4. **The strip shows four complete periods, the running one, and an average of
   the complete ones.** Four rather than three because three points is the fewest
   a pattern can be claimed from at all. The average excludes the running period
   — folding in a part-month would make the mean swing with the calendar rather
   than with spending — and reports its own coverage (`averagedPeriods`), so
   "nothing has finished yet" is distinguishable from "an average of zero".
   `Dec::div_int` is the one division the engine has: money over a *count* is
   money, at the same scale, rounding half away from zero.
5. **Balanced rules are edited too, not locked out.** When every amount in a rule
   is written down, changing one rewrites the counter-leg by the same delta. An
   ambiguous counter-leg is a refusal, not a guess — see § Which leg.

## Why the core module is `periodic`, not `budget`

A `~ PERIODEXPR  DESCRIPTION` block is hledger's *one* recurring-entry construct.
`hledger balance --budget` reads those blocks as goals; `hledger --forecast` reads
the very same blocks as future transactions. The projections work in `TODO.md`
("Planning / modeling") is going to want to write forecast scenarios, and it will
want this document rather than a second one that has to agree with it. So the
model is "a periodic rule", and "a budget goal" is a reading of one.

One consequence worth recording for that later work: our parser **rejects** every
period expression outside the five fixed intervals — `~ every 2 weeks`,
`~ monthly from 2026-01` — and the rejection fails the *whole journal* parse, not
just the rule. Forecasts need those forms. Fixing it is not in this plan; it is
noted in `TODO.md` under Planning / modeling.

## The document model

`crates/ledgeline-core/src/periodic.rs`, built to the discipline `aliases.rs`
states for a file of the same value:

> An edit rewrites bytes only inside the spans it names, and every other byte of
> the file comes out the `&str` slice it went in as.

`PeriodicDoc::apply` splices, and the only spans it will splice are one posting
line's **amount** extent, one posting line's **whole line** (a delete), one
block's **whole extent** (deleting its last goal), or an insertion point between
lines. It never touches an account name, a `~` header, or the whitespace that
aligns a column — which is why a tidy block stays tidy with no alignment code
anywhere, exactly as `rules.rs` needs none.

`PeriodicDoc::verify` then refuses rather than trusting that: it re-renders the
plan, requires the bytes to match, re-scans, and requires every unedited line back
byte-identical.

### What the document deliberately does not do

**It does not parse amounts.** That is the parser's job and not a job worth doing
twice — only `parse.rs` knows whether `1.234,56` in this journal is a thousand or
a one. So the document is a text-shape model, and two checks live above it:

- `periodic::plan` is handed the file's already-parsed `PeriodicTransaction`s and
  does the counter-leg arithmetic in exact `Dec`;
- `budget_api` re-parses the whole journal with the edited text in memory and
  requires the goal to read back as the number that was asked for. That is the
  check that would catch an amount rendered with the wrong decimal mark for a
  `1.234,56 EUR` journal — written, read back as a different number, and committed
  with a `200`.

The ordinal correspondence both rely on — `doc.blocks()[i]` ↔ the file's `i`-th
parsed rule, `block.lines[k]` ↔ `rule.postings[k]` — holds because the scan and
the parser skip the same `comment` blocks and consume the same body lines. It is
checked rather than assumed (`aligned_rule`), and pinned per fixture in
`tests/periodic_edit.rs`.

## Locating a rule: the one model change

`PeriodicTransaction` carried no position. A rule with no position can be
reported but never edited — the editor has to say *which* `~` block in *which*
file a goal came from before it will rewrite a byte of it. So the model gained
`source_span` and `source_file`, on exactly the convention `Transaction` uses
(end-exclusive, relative to the file the rule was parsed from). Everything
downstream is additive.

## Balance: three shapes, told apart structurally

No account-name heuristics, no type declarations — the shape of the block decides.

| Shape                                                | An edit…                                                        |
|------------------------------------------------------|-----------------------------------------------------------------|
| `Free` — every posting is unbalanced-virtual `(a)`   | rewrites one number and stops                                   |
| `Inferred` — exactly one real posting has no amount  | rewrites one number and stops; hledger re-derives the leg        |
| `Explicit` — every real amount is written            | rewrites one number **and** the counter-leg, by the same delta   |

`Inferred` is the case where doing less is doing it right. Given

```journal
~ monthly  budget
    expenses:food   $400
    assets:checking
```

nothing needs writing to `assets:checking` at all — which is also why that line is
presented read-only (`GoalLock::Inferred`): it has no amount extent to splice, and
setting one would change the block's shape from inferred to explicit, a bigger
edit than was asked for.

### Which leg

**The unique real posting, other than the one being changed, whose amount is
signed opposite to the change.** Not exactly one is a refusal
(`PeriodicError::AmbiguousCounterparty`).

That rule is worth stating in the concrete, because the refusals are as deliberate
as the successes. Given

```journal
~ monthly  budget
    expenses:food      $400
    expenses:rent     $1500
    assets:checking  $-1900
```

raising food to `$450` finds exactly one opposite-signed leg and takes checking to
`$-1950`. Editing `assets:checking` *itself* finds two, which is genuinely
ambiguous — no fact in the file says whether food or rent absorbs the difference —
so it is refused with a sentence. A user who wants that edits their journal.

The arithmetic is `counter -= delta`, where `delta` is the change in the sum of
every other real posting. Exact for a set, a delete and an append alike, which is
why all three share one code path.

## Creating `budget.journal`

Two writes, and the **order is the whole safety argument**: the new file first,
the `include` line second. An `include` naming a file that is not there is a
journal that does not parse, so if the second write fails the worst outcome is an
unreferenced file nobody reads. The other way round, a failed second write leaves
the user's journal broken. Both are proved before either happens — the whole
journal is re-parsed with both texts in memory.

The `include` goes at **EOF** of the main journal, for the reason
`aliases::insertion_point` gives at length: it is the one position provably unable
to change the meaning of anything already in the file. An `include` is the
directive that matters most for — one placed mid-file changes which directives are
in force for everything after it.

Two refusals: an existing `budget.journal` is never written over (a file we did
not create is a file whose contents are somebody's), and a journal that already
declares `~` rules is not given a second home for them.

## Which files the listing shows, and why not by name

Every file that declares a `~` rule — the parser records which file each rule came
from, so there is no guessing.

When *no* file declares one, a first goal still needs somewhere to go, so the
fallback is every writable file the parse read that holds **no transactions**, and
the root journal when there are none of those. A transaction-free file is what
`journals::targets` already calls a pure directive file, identified from its
CONTENT — which is what makes a freshly created `budget.journal` an offered home
without this module ever asking what a file is called. `journals.rs` states at
length why no filename is ever inspected, and creating a file under a name we
chose is not a licence to start recognising it later: someone whose budget file is
called `plan.hledger` gets the same behaviour.

## Signs, and where the flip lives

Exactly one place: `budget_api::inverted`. Whether to invert is a function of the
account's *type*, which comes from the journal's `account` declarations and
hledger's own inference — a fact neither the engine's editor nor the browser has
any business re-deriving. The wire carries both numbers (`amount` as the file
writes it, `entry` as the user types it) and a flag saying whether they differ, so
no component ever negates anything.

## Why Budget is its own tab

The second half of this page is a **write** surface, and a write surface does not
belong behind a strip labelled "Reports". The bars and the goals are the same
subject read two ways, so they share a page and a refresh: a save reloads both,
because a screen whose top half disagrees with its bottom half about the budget is
worse than one that is briefly blank.

`/reports?tab=budget` is forwarded to `/budget`, carrying its range. A bookmark
from before the move must not quietly land on Insights, which is where an unknown
`tab` value otherwise falls back to.

## A regression this uncovered: the account popup was never portalled

The goal modal reuses `journal/edit/AccountInput.svelte`, and briefly opening its
list on focus is what made the failure obvious: the popup did not appear inside a
`.modal-box` at all.

`anchoredPopup.ts` had documented, since it was written, that the popup is
"`position: fixed` and portalled to `<body>`" — and the component never performed
the portal. That was survivable until daisyUI 5.7.19 (bumped in dfe9619, a commit
that described itself as "no styling behaviour"), whose `.modal-box` carries

```css
.modal-box { scale: .95; translate: 0; transition: translate, scale, … }
```

A non-`none` `scale`/`translate` makes an element the containing block for its
fixed-position descendants, and `.modal-box` is also `overflow-y: auto`, so it
clips them too. The popup was therefore offset by the modal's own position and
hidden behind its edge — on screen, "the autocomplete stopped working". The
inline category editor kept working throughout, because its scroller has no
transform and `fixed` still meant the viewport there, which is why the breakage
looked specific to the transaction popup.

The fix is a four-line `portal` action doing what the comment always claimed.
Two component tests pin it — the popup's parent IS `document.body`, and it is
gone after unmount — because the property is invisible to every other kind of
test: jsdom has no layout, so nothing but an assertion on parentage can tell a
portalled popup from a clipped one.

### And why the list still opens only on typing

The `openOnFocus` option that surfaced the portal bug did not survive it. The
goal modal autofocuses its account field, so the list opened *during mount* —
while `.modal-box` is still at `scale: .95; opacity: 0` and mid-transition. The
rect measured there is not where the field ends up, so the popup was pinned to a
position the modal then animated away from, arriving off screen before the user
had done anything. Repositioning on a later frame would only have narrowed the
window, since the transition runs 300ms.

So the rule is the one the component always had, now stated as a test: the popup
opens on typing and never on focus. By the time anyone types, the dialog has
settled and the rect is real — and the discovery this was meant to serve is one
keystroke away.

## Testing

| Level                                | Covers                                                                                  |
|--------------------------------------|-----------------------------------------------------------------------------------------|
| `periodic.rs` unit tests             | the splice mechanics, each lock, each refusal, CRLF, a missing final newline             |
| `tests/periodic_edit.rs`             | every committed `fixtures/budget/*.journal`, edited every way, with two invariants: nothing else moved, and it still parses and means what was asked |
| `tests/budget_endpoints.rs`          | the HTTP surface — written BYTES, the counter-leg, the sign round trip, 409, the token guard, no absolute paths |
| `budget/params.test.ts`              | the presets, `budgetSpan`, the URL codec                                                 |
| `ReferenceStrip.svelte.test.ts`      | the average's coverage label, and the absence-vs-zero distinction                        |
| `AccountInput.svelte.test.ts`        | the popup is portalled to `<body>` and cleaned up; it never opens on focus                |
| `nativeDecode.test.ts`               | the `amount`/`entry` pairing, which is the whole sign contract                           |
| `e2e/budget.e2e.ts`                  | the tab is reachable, the old URL forwards, the empty state offers the right next step   |

The e2e deliberately **writes nothing**. The e2e engine is launched over
`fixtures/sample.journal` with editing enabled, and a budget goal lives IN that
journal — there is no scratch file to redirect a save into, the way
`imports.e2e.ts` redirects a rules-file save. A spec that added a goal would
rewrite a committed fixture five other specs assert exact numbers from. The write
path is covered by `budget_endpoints.rs` against a temp journal, asserting the
bytes, which is the stronger check anyway.
