# 12 — Balance sheet redesign (grouped, valued, three boxes)

Replaces the current `hledger bs`-lookalike table with a grouped, market-valued,
spreadsheet-style balance sheet. Driven by the README TODO "balance sheet ui
improvements".

## Decisions (locked with Patrick 2026-08-18)

1. **Market value in base currency.** Every line is one number. Commodities are
   valued via the existing `reports/prices.rs` path. Raw share counts remain
   available on the expanded rows. Unpriced commodities are surfaced, never
   silently dropped.
2. **Three boxes + balance check.** Assets / Liabilities / Equity, where Equity
   includes a computed **Retained earnings** line, so `A = L + E` actually holds.
3. **Built-in default groups + `bsgroup:` tag override.**
4. **Groups collapsed by default, expandable** to account detail at `depth`
   (default depth becomes **3**).

   > **Correction (2026-08-19).** The depth control is GONE from this tab, and
   > the report is requested unclamped. Depth was already nearly invisible here
   > — groups are the reading, the accounts inside one are a drill-down, and the
   > engine exempts a group's roots from the clamp anyway (see "Roots outrank the
   > depth clamp") — so all a clamp did was hide accounts the reader had no
   > remaining control to ask for. `sample.journal`'s `assets:bank:wise:eur` sits
   > four segments down: at depth 3 the row read `assets:bank:wise` and stood for
   > an account nothing on screen could reach. `TAB_CONTROLS.bs.depth` is now
   > `false`; is/cf/nw/budget keep the slider and the shared default of 3.

## The accounting identity this rests on

Verified against hledger 1.52 on `fixtures/sample.journal` (2026-07-08):

```
hledger bse -B  Net:  $42,998.91, -933,25 EUR
hledger is  -B  Net:  $42,998.91, -933,25 EUR      # identical, per commodity
```

So **at cost**, per commodity:

```
Assets − Liabilities − Equity(declared) − RetainedEarnings ≡ 0
```

This is exact, not approximate — it is just "every posting sums to zero,
regrouped". Therefore:

- The **check line** is a real journal-integrity signal. Non-zero means a genuine
  problem, so it must be displayed, not swallowed.
- Valuing assets at **market** breaks the identity by exactly the unbooked
  revaluation. We restore it with a synthetic Equity line **Valuation
  adjustment** `= (assets at market) − (assets at cost)`. Then `A = L + E` holds
  in market terms too, and the check stays a pure error detector.

  > **Correction (2026-08-18).** Earlier drafts called this line *Unrealized
  > gains*, and shipped it under that name. It is wrong: the same subtraction
  > absorbs currency revaluation and unpriced holdings, so on `sample.journal`
  > the line reads `$532.56 · 933.25 EUR · 5 GLD · -2 TSLA` — "933,25 EUR of
  > unrealized gains" is not a sentence a balance sheet can say. Renamed to
  > **Valuation adjustment**; the arithmetic is untouched.

Do **not** compute the check by comparing displayed/rounded numbers — compute it
from the exact `Dec` values (cf. [[journal-refresh-fingerprint]]'s lesson).

> **Correction (2026-08-19).** "Non-zero means a genuine problem" is false, and
> shipping it warned on Patrick's real journal: *"should be zero, but it is
> $0.00227970"*.
>
> The identity above is exact in ALGEBRA but not in the decimals a journal can
> write. A priced posting is worth `quantity × unit price` at cost, and `Dec`
> multiplication ADDS the scales, so `26.2690 VTI @ $289.7713` costs
> `$7,612.00227970` — five digits finer than the cash leg that paid for it can
> be written to. Nothing can cancel the surplus: there is no way to post
> $0.0022797 to a bank account. **hledger carries the identical residue** (`bal
> -B -c '$1000.00000000'` prints it) and merely hides it behind two-decimal
> display, and `hledger check` passes because its balance test tolerates half a
> unit at the precision the entry was written at — which
> `parse::check_transaction_balances` already reproduced. The report was the one
> place holding the sum to a standard no journal can meet.
>
> Note this is *not* a valuation artefact. The display-basis terms cancel
> exactly whatever the prices do: the three sections partition the very map the
> valuation adjustment sums, and `value_at` multiplies by one rate per commodity
> pair, which distributes over exact addition. A rounded reciprocal or a
> non-terminating cross-rate changes both sides identically — pinned by
> `a_rounded_cross_rate_leaves_no_residue`.
>
> So `check` stays the exact residual (it is what a warning has to quote) and a
> new `balanced: bool` carries the verdict: **balanced iff, for every commodity,
> `|residual|` is strictly under one unit of that commodity's own precision**,
> where the precision is the widest `Dec.places` the journal writes for it over
> the postings in scope — cost annotations excluded, since a price is not a
> posting. Justification: *a residual smaller than the smallest unit the
> commodity can express cannot correspond to any real posting, because no such
> posting can be written.* This cannot weaken the detector, because `Dec`
> addition takes the wider scale — a sum of amounts written at ≤ `p` places is
> itself at ≤ `p` places, so a non-zero one is always ≥ one unit. Only cost
> multiplication makes finer residues, and both failures the check exists to
> catch (an unbalanced transaction, an account in no section) are sums of
> written amounts. Fixtures: `bs-cost-dust.journal` (valid, dusty, balanced) and
> `bs-unbalanced.journal` ($10.00, still flagged).

> **Correction (2026-08-19, later the same day).** The precision rule alone was
> too clever, and it shipped: Patrick's real journal STILL warned, with the same
> `$0.00227970`. His book writes dollars at three and four places — brokerage
> interest, FX conversions and dividend reinvestments all do — so `p($)` was 4,
> the threshold collapsed to `$0.0001`, and the dust sailed past it. The
> tolerance had been made a function of the most finely written posting anywhere
> in the journal, a quantity with no relationship to how large a
> cost-multiplication residue can get.
>
> The rule now carries a **one-hundredth floor**, verbatim from the request
> ("i want the balance sheet to always ignore imbalances below one cent"):
>
> ```text
> tolerance(c) = max(10^-p(c), 0.01)
> balanced     = every commodity's |residual| < tolerance(c)   (strictly)
> ```
>
> Implemented as one unit at `min(p(c), 2)` places, which is the same number and
> makes plain that the floor only ever LOOSENS: whole-unit commodities keep their
> tolerance of 1, four-place dollars rise from `0.0001` to `0.01`. `check` is
> untouched and still exact.
>
> **The safety argument above no longer holds as written**, and pretending
> otherwise would be the same mistake twice. The "a sum of written amounts is ≥
> one written unit" proof stops covering any commodity written finer than two
> places. What holds instead: an account in no section (RPT-1) contributes its
> WHOLE balance, and an account worth under a cent is not the failure mode being
> detected; an unbalanced transaction still fires here when it is worth firing
> (`bs-unbalanced.journal` is $10.00), and a sub-cent one is caught upstream by
> `parse::check_transaction_balances` at hledger's own tolerance — this report is
> the second net for that class, never the only one. The real caveat: 0.01 is a
> unit of the COMMODITY, so on a BTC-denominated book the floor is not a
> negligible sum. New fixture: `bs-cent-floor.journal` — `bs-cost-dust.journal`
> plus the one `$0.0327` interest line that reproduced the bug.

## Group resolution

Resolution order for an account, first match wins — mirrors
`resolve_account_type`'s own → ancestor → inference shape:

1. `bsgroup:` tag on the account itself.
2. `bsgroup:` tag on the nearest declared **ancestor** (inherits down the tree,
   exactly like `type:`).
3. Effective type is `Cash` → **"Cash and cash equivalents"**.
4. Asset account whose balance holds any **non-base commodity** → **"Investments"**.
5. Fallback: the account's **second path segment**, humanized
   (`assets:bank:chase` → "Bank"; `liabilities:cc:visa` → "Cc" → "Credit cards").

### Why the fallback is a segment and not a name match

Per [[account-type-not-name]]: classification must never depend on matching
English account names, because Patrick's chart uses roots like `cogs:` and names
may be non-English. Steps 3 and 4 are **type-driven and commodity-driven** — both
language-neutral. Step 5 groups by tree position, also language-neutral, and
happens to produce exactly the wanted behaviour ("Bank", not "bank account A" vs
"bank account B").

A small alias table (`cc` → "Credit cards", `ar` → "Accounts receivable", `ap` →
"Accounts payable", …) may prettify step 5's **label only**. It must never affect
membership — a cosmetic alias cannot cause the "reports read zero" failure mode.

### Group ordering

Rank table for the known built-in names (current assets first, then non-current;
current liabilities, then long-term), everything else alphabetical after. Synthetic
equity lines sort last: Retained earnings, then Valuation adjustment.

## Engine API — `crates/ledgeline-core/src/reports/balance_sheet.rs`

Keep the existing `balance_sheet()` **exactly as it is**. It backs the hledger
parity golden (`balancesheet_matches_bs_d1_golden`) and
`fixtures/native/v1/balancesheet.json`; both must stay byte-identical. Add
alongside it:

```rust
pub struct BsOpts<'a> {
    pub as_of: &'a str,
    pub depth: Option<usize>,         // None = NO clamp; Some(0) = totals only
    pub value: Valuation,             // Market | Cost | None
    pub value_in: Option<Commodity>,  // default: prices.base_commodity()
}

pub fn balance_sheet_grouped(
    txns: &[Transaction],
    opts: &BsOpts,
    declared: &BTreeMap<String, AccountType>,
    groups: &BTreeMap<String, String>,   // account -> declared bsgroup
) -> Result<BalanceSheetReport, ReportError>;

pub struct BalanceSheetReport {
    pub as_of: String,
    pub base: Option<Commodity>,
    pub sections: Vec<BsSection>,        // always Assets, Liabilities, Equity
    pub net_worth: MixedAmount,          // assets − liabilities
    pub check: MixedAmount,              // A − L − E, EXACT (see the correction below)
    pub balanced: bool,                  // the verdict — never `check.is_zero()`
    pub meta: ReportMeta,                // unpriced
}
pub struct BsSection { pub kind: BsSectionKind, pub title: String,
                       pub groups: Vec<BsGroup>, pub total: MixedAmount }
pub struct BsGroup  { pub name: String, pub source: GroupSource,
                      pub rows: Vec<ReportRow>, pub total: MixedAmount }
pub enum GroupSource { Tag, Type, Commodity, Segment, Computed }
```

Read declared groups off `journal.accounts` tags with a new
`account_groups(journal) -> BTreeMap<String, String>`, mirroring `declared_types`.
**Do not widen `AccountDecl`** — it is constructed in ~25 test literals and
widening it is churn for no gain.

Reuse, do not reimplement: `aggregate::{account_totals, roll_up, at_depth}`,
`prices::{PriceDb, infer_market_prices, value_at, ValuationMeta}`,
`account_types::{resolve_account_type, is_account_type}`.

Preserve the ordering invariant from `sections.rs`: **filter by type first, then
roll up** (RPT-2), and sum section/group totals over *members*, not over displayed
rows, so totals stay depth-independent (RPT-1/RPT-4).

## Server — `crates/ledgeline-server/src/reports_api.rs`

New route, leaving `/api/reports/balancesheet` untouched:

```
GET /api/reports/balancesheet/grouped
      ?asOf=YYYY-MM-DD   (default: today)
      &depth=N           (ABSENT = no clamp; 0 = totals only)
      &value=market|cost|none   (default: market)
      &valueIn=$         (default: prices.base_commodity())
```

Validate `depth` like `count` is validated (the existing `depth` param is
unvalidated — do not copy that). JSON, camelCase, `Dec` as
`{mantissa: string, places: number}` exactly as today:

```jsonc
{
  "asOf": "2026-07-08",
  "base": "$",
  "value": "market",
  "sections": [
    { "kind": "assets", "title": "Assets",
      "groups": [ { "name": "Cash and cash equivalents", "source": "type",
                    "total": {"$": {"mantissa": "4245024", "places": 2}},
                    "rows": [ {"account": "assets:bank", "depth": 2,
                               "own": {}, "inclusive": {…}} ] } ],
      "total": {…} }
  ],
  "netWorth": {…},
  "check": {},
  "meta": {"unpriced": ["GLD", "TSLA"]}
}
```

## Frontend

- New `web/src/lib/reports/ui/BalanceSheetView.svelte` — three boxes. Follow
  `BudgetSummary.svelte:154-190` for the `bg-base-200 rounded-box` colored section
  header and `HoldingsTable.svelte:180-192` for the real `<tfoot>` total.
- Groups collapsed by default with a disclosure triangle; expanded rows are the
  group's accounts at FULL depth, indented, reusing `compressSectionRows` (so a
  single-child chain like `assets:bank:wise` → `…:eur` still reads as one row).
- One number per line via the `fmtBase` + `extras` pattern
  (`insights/format.ts:21-31`) — **not** `formatTotals`' stacked `<div>`s.
- Keep `data-account`, `scroll-mt-10` and the `listCursor` j/k wiring, or
  `ReportTable.svelte:55`'s `scrollIntoView` equivalent breaks.
- Unpriced banner must now fire for the balance sheet —
  `routes/reports/+page.svelte:137` currently gates it on `PeriodReport` only.
- `params.ts`: the shared default depth stays **3** for is/cf/nw/budget;
  `TAB_CONTROLS.bs.depth` is `false` (2026-08-19), so this tab renders no slider,
  writes no `depth` to the URL, and sends none to the engine.
- Purity rule: pure logic under `lib/reports/*.ts`, presentation under
  `lib/reports/ui/` (guarded by `reports/purity.test.ts`).
- **Tie-out, not a net-worth panel (added 2026-08-18).** `Total equity ≡
  A − L ≡ Net worth` by construction, so a panel showing net worth alone
  restated a figure already on screen. Close the statement with the classic
  spreadsheet tie-out instead — the three section totals, then
  `Liabilities + equity` set against `Total assets`, which is what carries the
  ✓/✗ — and show net worth below it as its own prominent figure. Derive
  `Liabilities + equity` from the exact `Dec`s (`bsSummary` in
  `balanceSheetRows.ts`) and take the verdict from `report.balanced`; a
  display-rounded tie-out would call a real half-cent imbalance balanced, and
  `maIsZero(report.check)` would do the opposite — see the 2026-08-19 correction
  above.

## XLSX

Rewrite the sectioned path in `web/src/lib/export/xlsx.ts` (`addSectioned`,
:143-162) as `addBalanceSheet`: colored section header rows matching the on-screen
boxes, bold group rows, indented account rows, ruled subtotals, section totals, and
the same tie-out the screen closes with (sharing `bsSummary`, so the workbook cannot
claim a different `Liabilities + equity` or a different verdict from the page it was
exported from). Because everything is valued to one commodity, cells are finally
**real numbers with a number format** rather than the comma-joined text fallback at
`xlsx.ts:106-108`. Freeze the header rows.

## Test expectations (hledger 1.52 ground truth, `fixtures/sample.journal`, 2026-07-08)

| Quantity | Value |
|--------------------------------|--------------------------------------------|
| Assets, unvalued | `$48,402.56` + 19.5 AAPL, 566,75 EUR, 5 GLD, −2 TSLA, 17 VTI |
| Assets, at market (`bs -V`) | exactly `$59,612.61500` + 5 GLD, −2 TSLA (both unpriced) |
| Assets, at cost | `$58,080.06`, −933,25 EUR, 5 GLD |
| Liabilities | `$531.15` |
| **Equity at cost** | **`$14,550.00` + 5 GLD** |
| Equity, unvalued | `$15,550.00` — NOT the figure that balances |
| Retained earnings (`is -B` Net)| `$42,998.91`, −933,25 EUR |
| Check, at cost | zero, per commodity |

> **Correction (2026-08-18).** An earlier draft of this table listed equity as
> `$15,550.00`. That is `hledger bal type:E` *unvalued*; the figure the identity
> needs is the **at-cost** equity, `$14,550.00 + 5 GLD` (`equity:transfers` is
> 5 GLD at cost, $1,000.00 unvalued). Using the unvalued number throws the check
> off by exactly $1,000 and 5 GLD. Verify: `58,080.06 − 531.15 − 14,550.00 =
> 42,998.91` ✓, EUR `−933,25` ✓, GLD `5 − 5 = 0` ✓, TSLA `0` ✓.

### Known rounding divergence — do not "fix" it inside this work

Net worth is exactly `$59,081.465`. hledger's CLI prints `$59,081.46` (Haskell
`round` is half-to-even); our `formatDec` is half-away-from-zero app-wide and
prints `$59,081.47`. It is display-only and only bites on computed/valued figures
with a trailing half-cent. Changing global money rounding is out of scope for a
balance-sheet task — record the divergence in the e2e comment and raise it
separately.

Relatedly, displayed group subtotals need not visibly add to a displayed section
total (`49,059.99 + 10,552.62` reads as `59,612.61` against a true
`59,612.615` → `59,612.62`). Always sum from exact `Dec` values; never re-sum
rounded display strings.

GLD and TSLA have **no `P` directive** — the fixture genuinely exercises the
unpriced path. They must appear in `meta.unpriced` and be visible in the UI, not
dropped.

Existing tests that will need updating (they assert the old shape/numbers):
- `web/e2e/smoke.e2e.ts:81-90` — `Total Assets` `$48,402.56`, `Net` `$47,871.41`.
- `web/e2e/insights.e2e.ts:125-135` — `rowheader` "assets" / "Total Assets".
- `web/src/lib/reports/ui/params.test.ts:39` — pins `depth=2`. (From 2026-08-19
  it also pins the bs query string, which no longer carries `depth` at all.)

Update them to the new correct values; verify every new number against the
`hledger` binary in the dev shell rather than against our own output.

---

# Added 2026-08-19 — current vs non-current (`bsterm:`)

Standard practice, and asked for directly: "do we have a way to flag current vs.
long term assets and liabilities? this is standard and would be subheaders in
their respective assets/liabilities boxes."

We did not. The sheet was two levels — box → group → accounts — with no notion of
term. What existed was `BUILT_IN_ORDER`, whose comment claimed "current assets
before non-current, current liabilities before long-term" while only ordering
five hardcoded names; every custom group tied at one rank and sorted
alphabetically, so a chart with Property and Mortgage got whatever the alphabet
gave it. The comment described an intention, not a feature.

## Decisions (locked with Patrick 2026-08-19)

1. **A third tag, `bsterm: current | noncurrent`.** Not a value of `bsgroup:`.
   `type:` picks the box, `bsterm:` the half, `bsgroup:` the line — three
   questions, three tags, and this is the same separation the income statement
   makes with `issection:`/`isgroup:`. Closed vocabulary, refused by name when
   misspelt: a term that silently falls back files a balance into the wrong
   subtotal and leaves a plausible statement behind.
2. **Adaptive.** No `bsterm:` anywhere → the report is byte-identical to the one
   this feature does not exist for: `subsections` empty, every `term` null, same
   groups, same order, same totals. The income statement's rule from plans/13,
   and pinned by `an_untagged_journal_is_completely_unchanged` plus an assertion
   on the committed golden that removing the two new keys reproduces the previous
   bytes exactly.
3. **Defaults once it is on**: the built-in `Investments` group is non-current,
   everything else untagged is current — you tag the house and the mortgage and
   leave the everyday accounts alone. `sample.journal` needed exactly three tags.
4. **Equity is never split.** The question — when does this become cash, when
   does this come due — is not asked of capital.

## Shape

`BsGroup` gains `term: Option<BsTerm>`; `BsSection` gains
`subsections: Vec<BsSubsection>`, each carrying its own `heading`, `label` and
`total`. Groups are keyed by **(term, name)** and ordered term-first, so one
`bsgroup:` straddling both halves prints as two lines under two subheadings —
correct, not a defect: a receivable due this year and one due in five are two
lines on a real statement.

`heading` and `label` are engine-supplied strings rather than something the SPA
derives from `term`, because that mapping would then live in both the view and
the xlsx exporter — the two-copies shape DRY-3 exists to prevent.

Subsection totals are summed over group totals, which are themselves summed over
members, so they stay depth-independent like every other total here and add to
the section total by construction rather than by luck.
