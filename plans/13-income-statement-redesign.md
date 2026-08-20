# 13 — Income statement redesign (grouped, valued, adaptive GAAP)

Replaces the `hledger is`-lookalike table with grouped, market-valued boxes and a
GAAP subtotal ladder that only appears when the journal has asked for it. Driven
by the README TODO "p&l report ui improvements", and shaped by
[`12-balance-sheet-redesign.md`](12-balance-sheet-redesign.md) — read that first;
almost every rule here is the same rule.

## The complaint being fixed

> "totals show up at the top and the bottom of a group. so 'revenue' has a total,
> then each thing under it has a total, then there's a 'Total Revenues' which is
> the same as the 'revenue' line."

That is `build_section`'s roll-up showing an `income` row (the depth-1 ancestor,
inclusive) immediately above its own children and immediately below the section
footer that repeats it. Groups replace the ancestor rows outright: a section's
lines are groups, a group's total is summed over members, and the account rows
only exist inside an expanded disclosure. Nothing is printed twice.

## Decisions (locked with Patrick 2026-08-19)

1. **Adaptive shape, driven by tags.** Untagged journals get two boxes — Revenue,
   Expenses — and a single Net income figure. The GAAP ladder (Gross profit,
   EBITDA, Operating income, Income before taxes) materialises line by line, each
   one appearing only when the sections it needs exist. No mode switch, no empty
   boxes, no jargon a personal journal never asked for.
2. **Two tags, mirroring `type:` + `bsgroup:`.** `issection:` is a closed, coded
   vocabulary that picks the box; `isgroup:` is free text that names the line
   inside it. Both inherit down the account tree.
3. **Market value in base currency**, exactly as the balance sheet. One number per
   line; unpriced commodities surfaced, never dropped.
4. **Two comparison columns: prior period and % of revenue.**
5. **No depth control on this tab**, for the reason it left the balance sheet:
   groups are the reading, and the accounts inside one are a drill-down.

## Why `issection:` is a code and `isgroup:` is prose

This is the `type:` / `bsgroup:` split, and it exists for the reason recorded in
[[account-type-not-name]]: a classification that decides *membership* must never
match English words, because the failure mode is a section that reads **zero**,
and because a chart of accounts may be in any language or use roots like `cogs:`.

- `issection:` decides membership → closed vocabulary of seven ASCII codes.
- `isgroup:` decides only which line an account is printed on, within a box it is
  already in → free text, any language, no table to match against.

The rejected alternative was a single `isgroup:` whose value was matched against
known English line names ("Cost of goods sold" → COGS). It would have put
`Coûts des marchandises vendues` in no section at all.

```journal
account cogs                  ; type: X, issection: cogs
account cogs:hosting          ; type: X, isgroup: Cloud hosting
account cogs:payments         ; type: X, isgroup: Payment processing
account expenses:salaries     ; type: X, issection: opex
account expenses:depreciation  ; type: X, issection: depreciation
account expenses:interest     ; type: X, issection: interest
account expenses:taxes:income  ; type: X, issection: tax
account income:grants         ; type: R, issection: other
```

`issection:` values, and nothing else: `revenue`, `cogs`, `opex`,
`depreciation`, `interest`, `tax`, `other`. An unrecognised value is a **hard
error** surfaced in Problems, not a silent `None` — precisely the mistake
`parse_account_type_tag` already had to be corrected for
(`account_types.rs:99-108`: `type: expenses` on a `cogs:` account declared
nothing and the account vanished from the income statement).

> **Correction (2026-08-19, as built).** "Surfaced in Problems" is not reachable
> from the engine alone: `wire::WireDiagnostic` anchors every entry to a
> `txnIndex`, and an `account` *directive* has no transaction to anchor to, so it
> would also need an allow-list entry in the SPA's `normalize.ts`. Landed instead
> as `ReportError::UnknownIsSection` → `AppError::BadRequest` → a 400 naming the
> account, the value and the seven codes. The property that mattered is kept —
> the tag is never silently dropped, so no account can go missing — but the cost
> is that one typo takes the whole tab down rather than annotating it. Routing it
> into Problems is a follow-up, not a redesign: one wire field and one allow-list
> entry.

Both tags carry hledger's own tag-parsing gotchas, already documented for
`bsgroup:` in `docs/balance-sheet.md`: **a tag value ends at the next comma**, and
**the tag name is the last word before the colon**.

## Section resolution

First match wins, mirroring `AccountGroups::resolve`:

1. `issection:` on the account itself.
2. `issection:` on the nearest declared **ancestor**.
3. Effective account type is `Revenue` → **`revenue`**.
4. Effective account type is `Expense` → **`opex`**.
5. Otherwise the account is not on this statement at all.

There is deliberately **no inference for `cogs`, `tax`, `interest` or
`depreciation`**. Every rule that could produce them from an untagged journal
would be a name match. A journal rooted at `cogs:` with `type: X` lands in
Operating expenses and reads correctly; splitting it out is one tag.

## Group resolution — drop the shared prefix

Within a section, first match wins:

1. `isgroup:` on the account itself.
2. `isgroup:` on the nearest declared **ancestor**.
3. Fallback: **the first account segment after the prefix every member of that
   section shares**, humanized.

Rule 3 generalises the balance sheet's "second path segment" instead of
contradicting it. Let `common` be the longest leading segment sequence shared by
every member of the section, and `min_segs` the fewest segments any member has;
the group is the segment at index `prefix = min(common, min_segs − 1)`.

| Section members | shared prefix | group segment | groups |
|---|---|---|---|
| `income:salary`, `income:dividends` | `income` | 2nd | Salary, Dividends |
| `expenses:food:groceries`, `expenses:housing:rent` | `expenses` | 2nd | Food, Housing |
| `cogs:materials`, `expenses:rent` | *(none)* | 1st | Cogs, Expenses |
| `expenses`, `expenses:food:groceries` | *(capped to 0)* | 1st | Expenses |

The cap is what makes the rule total: it guarantees at least one segment remains
for the shortest member, so a direct posting to a section root still gets a line
rather than an empty name. On every single-rooted chart — which is what
`assets:`/`liabilities:`/`equity:` always are — this is *exactly* the balance
sheet's existing behaviour, so the two statements group alike.

Reuse `account_groups.rs`'s humanization and its cosmetic alias table verbatim
(DRY, and the aliases are already membership-neutral by construction).

## The subtotal ladder

Sections render in this fixed order; a section with no members is **omitted
entirely**, and each subtotal is emitted only when its guard holds. Subtotals
attach to the section they follow (`trailing: Vec<IsSubtotal>`), so a subtotal can
never float free of a box.

```
Revenue                        [box]
Cost of revenue                [box]    iff cogs
    → Gross profit                      iff cogs
Operating expenses             [box]
    → EBITDA                            iff depreciation
Depreciation & amortization    [box]    iff depreciation
    → Operating income                  iff multi_step
Other income & expense         [box]    iff other
Interest                       [box]    iff interest
    → Income before taxes               iff tax
Income taxes                   [box]    iff tax
    (Net income lives in the summary, below the boxes)
```

`multi_step = any member resolves to a section other than revenue or opex`.

EBITDA sits **above** D&A and Operating income **below** it, which is the order a
real statement uses and which makes each subtotal a running total of everything
printed above it — no line is ever the sum of things both above and below it.
EBITDA is suppressed without a D&A section because it would then be numerically
identical to Operating income, which is the duplicate-total complaint this whole
redesign exists to fix.

In simple form the ladder is empty: two boxes, then Net income. That is the
entire personal-finance experience, and it requires no tags.

### Titles depend on the shape

`opex` is titled **"Expenses"** in simple form and **"Operating expenses"** in
multi-step form. It is the same section either way; only the label moves, so no
account changes box when a journal grows its first `cogs:` tag.

## Signs

Internally revenues are negative and expenses positive, and net income is
`−sum(all members)`. Each section is therefore displayed *flipped* or not, and
**the flip determines the sign of its contribution** — one field, not two:

| Section | displayed | contributes |
|---|---|---|
| `revenue` | `−sum` (positive) | `+displayed` |
| `cogs`, `opex`, `depreciation`, `interest`, `tax` | `+sum` (positive) | `−displayed` |
| `other` | `−sum` (**signed**) | `+displayed` |

`other` is the one section that is genuinely mixed — a grant and a lawsuit
settlement can share it — so it is presented as a net contribution to income and
is allowed to print negative, in the parenthesised style a real statement uses.
Everything else prints as a positive magnitude.

## Comparison columns

**Prior period** is the immediately preceding window of **equal length**:
`prior_to = from − 1 day`, `prior_from = prior_to − (to − from)`. No calendar
special-casing — a full calendar year already yields the prior calendar year
(`2026-01-01..2026-12-31` → `2025-01-01..2025-12-31`), and every other range gets
an honest apples-to-apples duration with its dates in the column header.

> **Correction (2026-08-19, as built).** That parenthetical holds only when the
> preceding year is the same length. `2025-01-01..2025-12-31` maps to
> `2024-01-02..2024-12-31`, because 2024 is a leap year and the rule is equal
> *duration*, not equal *calendar unit*. The plain arithmetic was kept anyway —
> the rule forbids calendar special-casing, the column header always states the
> dates, and one day out of 366 is a smaller lie than a column silently comparing
> 365 days against 366. Pinned by a test so it cannot drift into a surprise.

Each period is **valued at its own period end**, matching `hledger is -V` run over
that range. The alternative (both at the report's `to`, constant currency) makes a
cleaner change column but means the prior column disagrees with the report you
actually ran last year. Parity with hledger wins; the caveat goes in the docs.

The prior figures are merged **in Rust**, over the union of section/group/account
keys, so a line present in only one period still appears with a zero on the other
side. Doing this join in TypeScript was rejected: it is exactly the kind of
key-matching that silently drops rows, and it would be untested twice over.

**% of revenue** is computed in `incomeStatementRows.ts` from the decoded exact
`Dec` values — never from formatted strings ([[journal-refresh-fingerprint]]) —
and rendered to one decimal. It is `Option`-shaped: with zero revenue there is no
percentage, and `—` is the honest cell. The denominator is the Revenue section
total, not net income.

## Engine API — `crates/ledgeline-core/src/reports/income_statement.rs`

Keep `income_statement()` **exactly as it is**: it backs
`income_statement_depth_2_matches_golden` (`fixtures/golden/is-d2.json`),
`fixtures/native/v1/incomestatement.json`, and the
`income_statement_net_equals_balance_sheet_delta_over_a_clean_window` invariant.
All must stay byte-identical. Add alongside it:

```rust
pub struct IsOpts<'a> {
    pub from: &'a str,
    pub to: &'a str,                  // both INCLUSIVE, as today
    pub value: Valuation,             // reuse balance_sheet::Valuation
    pub value_in: Option<Commodity>,  // default: prices.base_commodity()
    pub compare: bool,                // prior equal-length window
}

pub fn income_statement_grouped(
    txns: &[Transaction],
    explicit_prices: &[PriceDirective],
    opts: &IsOpts,
    declared: &BTreeMap<String, AccountType>,
    sections: &BTreeMap<String, IsSectionKind>,  // account -> declared issection
    groups: &BTreeMap<String, String>,           // account -> declared isgroup
) -> Result<IncomeStatementReport, ReportError>;

pub enum IsSectionKind { Revenue, Cogs, Opex, Depreciation, Interest, Tax, Other }
pub enum IsSubtotalKind { GrossProfit, Ebitda, OperatingIncome, PretaxIncome }

pub struct IncomeStatementReport {
    pub from: String,
    pub to: String,
    pub prior: Option<DateRange>,     // the window the `prior` figures cover
    pub base: Option<Commodity>,
    pub sections: Vec<IsSection>,     // non-empty sections only, ladder order
    pub net_income: Amounts,
    pub multi_step: bool,
    pub meta: ReportMeta,             // unpriced
}
pub struct IsSection { pub kind: IsSectionKind, pub title: String,
                       pub groups: Vec<IsGroup>, pub total: Amounts,
                       pub trailing: Vec<IsSubtotal> }
pub struct IsGroup   { pub name: String, pub source: GroupSource,
                       pub rows: Vec<IsRow>, pub total: Amounts }
pub struct IsRow     { pub account: String, pub depth: usize, pub amounts: Amounts }
pub struct IsSubtotal{ pub kind: IsSubtotalKind, pub label: String, pub total: Amounts }

/// Current-period figure plus the prior window's, when comparing.
pub struct Amounts { pub current: MixedAmount, pub prior: Option<MixedAmount> }
```

Reuse, do not reimplement: `aggregate::{account_totals, roll_up}`,
`prices::{PriceDb, infer_market_prices, value_at}`,
`account_types::{resolve_account_type, is_account_type}`, and from
`account_groups.rs` the humanizer, the alias table and `GroupSource`.

Read the two tag maps off `journal.accounts` with
`account_sections(journal) -> BTreeMap<String, IsSectionKind>` and by widening
`account_groups.rs` to read a configurable tag name, so `bsgroup:` and `isgroup:`
share one implementation rather than growing a copy. **Do not widen
`AccountDecl`** — same reason as last time, ~25 test literals construct it.

Preserve the invariants from `sections.rs`: **filter by section first, then roll
up** (RPT-2); sum section and group totals over **members**, never over displayed
rows (RPT-1/RPT-4), so a collapsed group and an expanded one report the same
number.

> **Corrections (2026-08-19, as built).** Two clauses above did not survive
> contact:
>
> - **Group rows are members at their DIRECT totals, with no roll-up.** This
>   report has no depth clamp, so a rolled-up ancestor row set against `IsRow`'s
>   single `amounts` field would double-count on screen. RPT-2 is honoured where
>   it actually bites — membership is decided on the direct per-account totals,
>   before any valuation — so a shared ancestor like `expenses` can never net
>   children belonging to different boxes.
> - **`multi_step` is read off the sections that RENDER, not raw membership.**
>   The two differ only for a tagged account that is zero in both windows, and
>   the literal rule would then retitle every box and print an "Operating income"
>   line beneath a D&A box that had been omitted for being empty — contradicting
>   this plan's own "a section with no members is omitted entirely".
>
> Also: `account_sections` returns `Result`, not the plain `BTreeMap` in the
> signature above. That return type is what makes a bad `issection:` a hard error
> at all.

## Server — `crates/ledgeline-server/src/reports_api.rs`

New route, leaving `/api/reports/incomestatement` untouched:

```
GET /api/reports/incomestatement/grouped
      ?from=YYYY-MM-DD   (default: Jan 1 of the current year, as today)
      &to=YYYY-MM-DD     (default: today)
      &value=market|cost|none   (default: market)
      &valueIn=$         (default: prices.base_commodity())
      &compare=previous|none    (default: previous)
```

No `depth` param — this report has no depth. Validate `value`/`compare` with the
existing `parse_valuation` shape; reject unknown values rather than defaulting.

```jsonc
{
  "from": "2026-01-01", "to": "2026-07-08",
  "prior": {"from": "2025-06-26", "to": "2025-12-31"},
  "base": "$", "value": "market", "multiStep": false,
  "sections": [
    { "kind": "revenue", "title": "Revenue",
      "groups": [ { "name": "Salary", "source": "segment",
                    "total": {"current": {"$": {"mantissa": "3396000", "places": 2}},
                              "prior":   {"$": {"mantissa": "3936000", "places": 2}}},
                    "rows": [ {"account": "income:salary", "depth": 2,
                               "amounts": {"current": {…}, "prior": {…}}} ] } ],
      "total": {"current": {…}, "prior": {…}},
      "trailing": [] }
  ],
  "netIncome": {"current": {…}, "prior": {…}},
  "meta": {"unpriced": []}
}
```

`Dec` stays `{mantissa: string, places: number}`; enums are lowercase
`&'static str`; `prior` keys are absent (not null) when `compare=none`.

## Frontend

- `web/src/lib/reports/ui/IncomeStatementView.svelte` — boxes, mirroring
  `BalanceSheetView.svelte` beat for beat: local `open` state, groups collapsed by
  default, disclosure triangle, expanded rows at full depth through
  `compressSectionRows`, `<tfoot>` section total, `data-account`, `scroll-mt-10`
  and the `listCursor` j/k wiring. Subtotal lines render **between** boxes, ruled,
  not inside them.
- `web/src/lib/reports/ui/incomeStatementRows.ts` — the display model, one
  function feeding both the template and the cursor list, plus `isSummary()`
  shared with `xlsx.ts` so the workbook cannot disagree with the screen. This is
  where % of revenue is computed.
- Amount cells reuse the `fmtBase` + `extras` pattern from `balanceSheetRows.ts`'s
  `amountCell`; extract it rather than copying it.
- `params.ts`: `TAB_CONTROLS.is.depth = false` (the slider goes, as it did for
  `bs`); `range` stays true.
- `+page.svelte`: new `kind: "incomeStatement"` branch, an `exportInfo` case, and
  the unpriced banner must fire for this report too.
- `types.ts` / `nativeDecode.ts`: decoder-applied `kind` discriminator (three
  reports now carry `sections`), strict `decodeEnum` for both enums.
- xlsx: `addIncomeStatement` alongside `addBalanceSheet` — coloured section
  headers, bold group rows, indented accounts, ruled subtotals, real numbers with
  a number format, groups always written expanded, prior and % columns included.

## Tests

New: `crates/ledgeline-core/tests/income_statement_grouped.rs`,
`web/src/lib/reports/ui/incomeStatementRows.test.ts`,
`IncomeStatementView.svelte.test.ts`,
`web/src/lib/testing/incomeStatementFixture.ts`, and a tagged business journal
`fixtures/reports/is-sections.journal` exercising all seven sections, the full
ladder, a mixed `other`, and an `isgroup:` that merges two unrelated accounts.

Ground truth (hledger 1.52, `fixtures/sample.journal`, `is -V --depth 2`, which is
what the shared-prefix rule reproduces on this chart):

| Range | Revenue | Expenses | Net |
|---|---|---|---|
| `2026-01-01..2026-07-08` | `$34,010.00` | `$25,126.48` | `$8,883.52` |
| prior `2025-06-26..2025-12-31` | `$39,397.50` | `$24,516.71` | `$14,880.79` |
| `2024-07-01..2026-07-08` | `$132,851.25` | `$90,934.91` | `$41,916.34` |

Groups for the current range: Salary `$33,960.00`, Dividends `$50.00`; Food
`$1,654.38`, Housing `$13,125.00`, Taxes `$8,760.00`, Transport `$186.54`, Travel
`$656.40`, Unknown `$75.00`, Utilities `$669.16`. Verify every number against the
`hledger` binary in the dev shell, never against our own output.

Note the valued net income (`$41,916.34`) is **not** the balance sheet's at-cost
Retained earnings (`$42,998.91`): the difference is exactly what the Valuation
adjustment line already absorbs. Say so in the docs before someone files it as a
bug.

Existing tests that will need updating:
- `web/e2e/smoke.e2e.ts:128` — asserts the depth slider *reappears* on the P&L
  tab. It no longer does.
- `web/src/lib/reports/ui/params.test.ts:44` — pins `depth` in the `is` query.

## Docs

`docs/income-statement.md`, mirroring `docs/balance-sheet.md`: the two tags, the
seven codes, the ladder and its guards, the shared-prefix grouping rule, valuation
and the prior-period caveat, and the net-income-vs-retained-earnings note. README
gets a link beside the balance sheet one, and the TODO entry comes out.
