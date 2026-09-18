# 21 — Every period expression hledger accepts, and the comment that carries the rest

Ledgeline's parser understands five period expressions. hledger understands
dozens, and a journal using any of the others does not merely lose a rule — it
fails to open at all. This plan closes that gap and, in the same pass, stops
discarding the one thing a `~` rule's header carries that nothing else can: its
comment. The grammar is a hard prerequisite for
[`22-projections.md`](22-projections.md), whose file format is built from
bounded and single-date rules; the comment is a correctness fix worth making
while the code is open, since postings already keep theirs.

## The ask

This one is not from `TODO.md` directly. It fell out of designing the
projections file format, which needs bounded rules (`~ monthly from 2027-04-01`)
to express a step change and rule-level tags to carry a growth rate. The
user's framing of the scope question:

> "Extending the parser is unavoidable. How far should it go while we're in
> there?" — answered: **accept everything hledger accepts**.

## What is true today

`crates/ledgeline-core/src/parse.rs:1345-1354`:

```rust
fn parse_period_expr(expr: &str) -> Result<PeriodExpr, ParseError> {
    match expr {
        "daily" => Ok(PeriodExpr::Daily),
        "weekly" => Ok(PeriodExpr::Weekly),
        "monthly" => Ok(PeriodExpr::Monthly),
        "quarterly" => Ok(PeriodExpr::Quarterly),
        "yearly" => Ok(PeriodExpr::Yearly),
        other => Err(ParseError::UnsupportedPeriodExpr(other.to_string())),
    }
}
```

That `Err` propagates out of the whole-journal parse (`parse.rs:104`). A user
whose journal contains `~ every 2 weeks  paycheck` cannot open their ledger in
Ledgeline. Tests at `parse.rs:3156-3182` pin this behaviour deliberately, and
they are wrong to; they become the tests that the rule is *kept and locked*.

`PeriodExpr` (`model.rs:331-343`) is a payload-free five-variant enum, so a
bound, a multiplier and a day-of-month anchor have nowhere to live even if they
parsed. And `parse.rs:1279`:

```rust
let (main, _comment) = split_comment(after_tilde);
```

throws the rule's comment away — the note at `parse.rs:1294-1298` says why
("`PeriodicTransaction` has no comment field"). Posting-level comments and tags
*are* kept (`parse.rs:1300`), so the asymmetry is an omission, not a policy.

## What hledger 1.52 actually accepts

Verified against the `hledger` binary in the dev shell, driving `print
--forecast` over scratch journals. Accepted:

| Form | Notes |
|---|---|
| `~ monthly` · `~ every month` | synonyms |
| `~ every 2 weeks` · `~ biweekly` | synonyms; 63 occurrences over 29 months |
| `~ every 15th day of month` | anchored |
| `~ every 3rd tuesday of month` | anchored, weekday |
| `~ every tuesday` · `~ every 2nd day of week` | weekday |
| `~ every 12/25` | annual on a month/day |
| `~ quarterly from 2027` · `~ yearly from 2027` | open-ended bound |
| `~ every 2 months from 2027-01` | multiplier + bound |
| `~ monthly from 2026 to 2028` | **`to` is EXCLUSIVE** — 24 occurrences, 2026-01…2027-12 |
| `~ 2027-03-01` | a single occurrence |
| `~ from 2027-03-01 to 2027-04-01` | equivalent single occurrence |

Rejected by hledger itself: `~ every last day of month`,
`~ every 15th,last day of month`, `~ every jan`. We reject exactly what hledger
rejects, and the parser refusing something hledger refuses is not a bug.

Two hard rules the parser must honour, both of which hledger enforces with its
own error messages: **two spaces** separate a period expression from the
description, and `to` is exclusive.

## Decisions (locked with Patrick 2026-09-18)

1. **Parse everything hledger parses; lock what we cannot rewrite.** The
   editor already has a vocabulary for this — `BlockLock` (`periodic.rs:172`)
   renders a rule read-only with the engine's own sentence, and
   `docs/budget.md` already documents "its period is not one of hledger's five
   fixed intervals" as a lock reason. That machinery is right; it was simply
   unreachable, because the journal died before it could be used.
2. **An expression we fail to parse degrades to a locked rule, never to a
   failed journal.** A period expression is one line of one rule. Refusing to
   open a 40,000-line ledger over it is not a proportionate response, and it is
   the difference between Ledgeline being usable on a real journal and not.
3. **`PeriodicTransaction.period` becomes a `PeriodSpec`, not a second field
   beside `PeriodExpr`.** Two fields that can disagree about the same fact are
   a bug waiting for a maintainer. `PeriodSpec` answers `interval()` with
   `Some` only for a bare fixed interval, which is exactly the question every
   current caller is really asking.
4. **The raw expression is preserved verbatim.** `PeriodSpec::raw` holds the
   bytes between `~` and the double space. The editor round-trips headers by
   never touching them (`periodic.rs:576-596` splices by span), and a lock
   message that can quote what the user actually wrote is worth the field.
5. **Rule-level comments and tags are parsed and kept**, using the same
   `append_comment_line` the postings already use.
6. **Nothing here changes what `budget_report` counts.** A rule that was
   already legible keeps producing identical goals; a rule that previously
   killed the parse now contributes goals it should always have contributed.
   The committed budget goldens must not move, and the test for that is that
   they do not.

## Scope

In: the period grammar, `PeriodSpec`, rule comments/tags, the downstream call
sites those two changes touch, the editor's lock reasons, and the wire field
that names a rule's period.

Out of scope, on purpose: generating forecast transactions (that is plan 22);
`--forecast` as an hledger invocation (`hledger_conf.rs:33` and
`hledger.rs:59` strip a user's config-file `--forecast` precisely so it cannot
perturb results, and that stays); editing a non-simple rule's header, which
remains locked.

---

## Phase 1 — The model

### `crates/ledgeline-core/src/model.rs`

```rust
/// One of hledger's period-expression shapes, as written.
///
/// `raw` is the bytes between `~` and the double space, preserved so the editor
/// can quote a rule it will not rewrite and so a rewrite that does not touch the
/// header is provably byte-identical.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeriodSpec {
    pub raw: String,
    pub kind: PeriodKind,
    /// Inclusive first date the rule may fire on (`from`), if stated.
    pub start: Option<String>,
    /// EXCLUSIVE end (`to`), as hledger writes it, if stated.
    pub end: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PeriodKind {
    /// `daily` … `yearly`, optionally with a multiplier (`every 2 weeks`).
    Every { unit: PeriodExpr, multiplier: u32 },
    /// `every 15th day of month`, `every 3rd tuesday of month`, `every tuesday`.
    Anchored { unit: PeriodExpr, anchor: Anchor },
    /// `every 12/25` — one occurrence a year on a fixed month/day.
    Annual { month: u32, day: u32 },
    /// `~ 2027-03-01` or `~ from D to D` spanning one period: fires once.
    Once,
    /// Syntactically well-formed to hledger but not modelled here. Kept whole.
    Unsupported,
}

impl PeriodSpec {
    /// The report `Interval` this rule steps by, or `None` when it is not a
    /// bare fixed interval. Every caller that used to match on `PeriodExpr`
    /// asks this.
    pub fn interval(&self) -> Option<PeriodExpr>;
    /// True when the editor can add to / rewrite goals in this rule.
    pub fn is_simple(&self) -> bool;
}
```

`PeriodicTransaction` (`model.rs:352-374`) gains `comment: String` and
`tags: Vec<Tag>` and its `period` field changes type. `PeriodExpr`
(`model.rs:331-343`) is untouched — it keeps meaning "one of the five units".

### `crates/ledgeline-core/src/parse.rs`

- `parse_period_expr` becomes `parse_period_spec(raw) -> PeriodSpec`,
  **infallible**. `ParseError::UnsupportedPeriodExpr` is removed from the error
  enum along with its `parse.rs:104` arm; the same information now travels as
  `PeriodKind::Unsupported`.
- The grammar, in the order hledger resolves it: optional `every`, optional
  multiplier, unit or anchor, optional `from DATE`, optional `to DATE`. A bare
  ISO date is `Once`. Anything left over makes the whole spec `Unsupported`
  while keeping `raw`.
- `parse.rs:1279`: stop discarding the comment. Feed it through
  `append_comment_line` into the new `comment`/`tags` fields, matching the
  posting path at `:1300` exactly, so tag syntax means the same thing in both
  places.
- The double-space requirement: hledger errors with "a double space is required
  between period expression and description/comment". We match that — a single
  space means the rest is part of the period expression, which is the reading
  that makes `~ monthly rent` an error rather than a rule named "rent".

### Tests

- Replace the rejection tests at `parse.rs:3156-3182` with acceptance tests
  covering every accepted row of the table above, asserting `kind`, `start`,
  `end`, and that `raw` is byte-identical to the input.
- The three forms hledger itself rejects parse to `Unsupported` with `raw`
  intact and **do not** fail the journal.
- A rule with `~ monthly  rent  ; growth: 3%/yr, scenario: base` yields both
  tags, and its `comment` is the text after `; `.

---

## Phase 2 — Downstream

Three call sites read `PeriodExpr` today and must ask `PeriodSpec::interval()`
instead:

| Site | Change |
|---|---|
| `reports/budget.rs:107-113` `period_interval` | takes `&PeriodSpec`; `None` ⇒ the rule contributes no goals to the bars, which is what it does today by virtue of not parsing |
| `reports/budget.rs:160-174` `occurrences` | honours `spec.start`/`spec.end` by intersecting them with the report span, and handles `Once` and `Annual` |
| `periodic.rs:1347-1356` `period_of` | its doc says it must stay in lockstep with `parse_period_expr`; it now maps a `PeriodSpec` back to the editor's period vocabulary |

`periodic.rs` also gains one thing: `PeriodicBlock` exposes the **span of the
header comment**, so a later feature can rewrite `; growth: 3%/yr` without
touching a byte of the rest. No edit operation uses it yet — plan 22 adds the
`PeriodicEdit` variant that does. Exposing the span now keeps the document
model's knowledge in one place.

`BlockLock` gains a reason for `PeriodKind::Unsupported` that quotes `raw`.

### Wire

`WireBudgetRule.period` (`budget_api.rs:179-194`) is a bare string today
(`"monthly"`). It becomes:

```jsonc
"period": {"raw": "every 2 weeks", "simple": null}     // locked
"period": {"raw": "monthly", "simple": "monthly"}      // editable
```

`simple` is what `web/src/lib/budget/types.ts:24 BudgetPeriod` consumes, so
`isBudgetPeriod:35` and `BUDGET_PERIODS:27` are unchanged; `raw` is what the
lock sentence and the tooltip show. This is a contract change and is meant to
be seen: `just snapshot-native`, review the diff, commit it, and update
`nativeDecode.test.ts` and `budget/types.ts` together.

Frontend sites: `BudgetEditor.svelte` groups on `row.rule.period` (now
`.period.simple`) and already has an `otherRows` bucket (`:otherRows`) for
periods it does not offer — which is precisely where a `~ every 2 weeks` rule
lands, with no new UI needed. `routes/budget/+page.svelte:167 isOffered` reads
`simple`.

---

## Phase 3 — Proving it against hledger

Add `fixtures/budget/period-forms.journal` containing one rule per accepted
form, plus the three hledger rejects, plus a rule carrying tags. Extend
`scripts/gen-budget-golden.sh`'s `cases=()` array with a `bal --budget -M` run
over it at pinned dates, commit `period-forms.budget.json` and its `.txt`, and
add the case to `crates/ledgeline-core/tests/budget_golden.rs` reconciling
hledger's exclusive `-e DATE` against our inclusive `end` the way every existing
case does.

This is the test that matters. The grammar is inferred from a CLI, and the only
defensible proof that our occurrences match hledger's is hledger's own output
over the same file.

`cargo test` stays hermetic — the golden is committed; nothing shells out to
hledger at test time.

## Sequencing

```
Phase 1 (model + parse) ──> Phase 2 (downstream + wire) ──> Phase 3 (golden)
```

One agent, sequentially. Phase 1 alone will not compile the workspace (the
`period` field's type changes), so Phases 1 and 2 land as one commit; Phase 3
may be a second.

**Plan 22 depends on this landing first.** Plan 20 does not.

## Where the code is

| Path | Purpose |
|---|---|
| `crates/ledgeline-core/src/model.rs` | `PeriodSpec`, `PeriodKind`, `Anchor`; `PeriodicTransaction.comment`/`tags` |
| `crates/ledgeline-core/src/parse.rs` | `parse_period_spec`, the kept rule comment, the removed error variant |
| `crates/ledgeline-core/src/periodic.rs` | `period_of`, the header-comment span, the new `BlockLock` reason |
| `crates/ledgeline-core/src/reports/budget.rs` | `period_interval`, `occurrences` honouring bounds |
| `crates/ledgeline-server/src/budget_api.rs` | `WireBudgetRule.period` becomes an object |
| `web/src/lib/budget/types.ts` · `ui/BudgetEditor.svelte` | `period.simple` / `period.raw` |
| `fixtures/budget/period-forms.journal` | one rule per accepted form |
| `docs/budget.md` | § "What Ledgeline will not rewrite" — the lock table gains the `Unsupported` row and stops implying the journal fails |

## Testing

| Level | Covers |
|---|---|
| `parse.rs` unit tests | every accepted form; `raw` preserved; hledger's three rejects degrade rather than fail; rule tags |
| `periodic.rs` unit tests | the header-comment span; the new lock reason |
| `reports/budget.rs` unit tests | `from`/`to` intersected with the report span; `Once`; a multiplier |
| `tests/budget_golden.rs` | `period-forms.journal` matches hledger 1.52 bucket for bucket |
| `tests/budget_endpoints.rs` | the reshaped `period` field |
| `nativeDecode.test.ts` | `period.simple`/`period.raw` decode; an absent `raw` throws |
| `e2e/budget.e2e.ts` | unchanged — it must still pass, which is the regression guard |

Deliberately not tested: editing a non-simple rule. There is no edit path for
one, by design; the test that it stays read-only is the lock assertion in
`periodic.rs`.

## Definition of done

- `just engine-check`, `just engine-test`, `just check`, `just test`,
  `just lint`, `just e2e` green.
- `./scripts/gen-budget-golden.sh` and `just snapshot-native` re-run; both
  diffs reviewed and committed.
- The committed budget goldens that existed before this plan are **byte
  identical** afterwards.
- `docs/budget.md` amended.
- Any contract in this doc that changed during implementation is amended here in
  the same commit, per `plans/00-overview.md` convention #9.
