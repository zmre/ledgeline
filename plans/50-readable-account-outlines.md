# 50 — Readable account outlines in reports, P&L depth, budget column

Decided with Patrick 2026-11-27. Branch `2026-11-26-reports-improvements`.
Built 2026-11-27: WP1–WP6 merged on the branch (not yet pushed).

## Goal

Statements should read like a bookkeeper's report, not a dump of account paths:

```
Travel
    Airfare                     1,200
    Ground Transport              340
    Lodging
        Hotels                    900
        Short-term Rentals        410
    Total Travel                2,850
```

Each level is shown once. Children are indented under it. Headings inside a
group carry no amount; the group's Total line captures everything. The full account path appears on
hover, never as the row label. The same outline is used on screen and in the
xlsx export, and in the P&L, Cash Flow (sections plus Beginning/Ending cash) and
the Balance Sheet.

Second goal: an optional **Budget** column (italic, beside the Total column) so
stakeholders can compare each line against plan at a glance.

## Today (as traced 2026-11-27)

- **Cash Flow depth "does nothing".** The value does reach the engine
  (`cash_flow_statement.rs:640`, clamp to depth). It looks dead because:
  - a line holding one account prints that account's full leaf path
    (`cashFlowRows.ts:84`);
  - expanded rows are always leaves, with no intermediate levels;
  - the slider's max is the journal's deepest account (`+page.svelte:182`), but
    cash-flow lines are 2–3-segment balance-sheet accounts, so every setting
    from 3 up renders the same;
  - `cfgroup:` tags override depth;
  - `[cashflow] --depth` in hledger.conf re-applies on each tab activation.
- **P&L indentation is inconsistent.** The engine sends only group *member*
  accounts. `compressRows` nests a row only under a parent that is itself a
  row. `revenue:sales` has direct postings, so it is a member and
  `hybrid-cloud` nests under it. `expenses:travel` has none, so `airfare`,
  `ground-transport` and the rest each print their full path. The Balance Sheet
  doesn't have this problem because it synthesizes covering ancestors
  (`balance_sheet.rs:1390`).
- **No P&L depth.** Plan 13 decision 5. The engine already supports
  `IsOpts.depth`; only Projections uses it.
- **No display names in reports.** `name:` on accounts is read only by
  Holdings › Other, and there it is inherited from the nearest ancestor.
- **No budget vs actual on any statement.** `budget.rs` already does hledger
  `--budget` semantics, including moving a child's actuals onto its nearest
  budgeted ancestor (`remap_account`).
- **Labels are right.** "P&L" and "income statement" are two names for the
  same statement. "Cash Flow" is a true indirect-method statement of cash
  flows. Nothing called cash flow shows income or expense rows. Keep the tab
  name "P&L"; the xlsx title stays "Income Statement" (the formal name).

## Decisions

| #   | Decision |
| --- | -------- |
| D1  | **Outline rule.** Within one group/line, build the tree of its member accounts. Show an intermediate ancestor as a heading only when **two or more** member accounts sit under it in that group. A lone child never gets a heading; its chain collapses into it. This is how cross-subtree groups work: in a Professional Services group, `expenses:people-costs:payroll-processing-fees` sits beside `expenses:professional-fees:accounting` and `…:legal`. Payroll processing has no siblings in the group, so it prints as one leaf. Accounting and legal share `professional-fees`, so they roll up under a "Professional Fees" heading. Rollups never cross group boundaries. |
| D2  | **Row label** is the account's own `name:` tag if it has one. Otherwise it is the last path segment, prettified only when it looks code-like (all lowercase with `-`/`_` → title case, via the existing `humanized_segment`). Segments already written for people (`People Costs`, `4110 Rental Income`) are shown exactly as written. For labels, `name:` is **not inherited**: a child does not borrow its parent's name. (Holdings › Other keeps its inherited behaviour; it uses the tag differently.) |
| D3  | **Collision guard.** If two sibling labels in an outline come out identical (e.g. two `other` leaves from different parents), prefix each with its parent's label: `Travel · Other`. |
| D4  | **Headings carry no amount and no Total row.** Only the group gets a Total line. **Exception:** when the Budget column is on and a goal is set on a heading's account, that heading gets a `Total <label>` row carrying the subtree's actual and the goal (see D13). |
| D5  | **Parent with its own postings.** When a heading account also has direct postings, those show as a first child row labelled `<label> (other)`, so the children still add up to the group Total. |
| D6  | **Hover** on any label shows the full account path. When the label came from `name:`, the tooltip shows both. Synthesized headings show their path too. In xlsx the path goes in a cell note on the label. |
| D7  | **One outline builder in the Rust engine**, next to `group_rows` (plan 26 direction: logic in Rust). It is shared by the P&L, Cash Flow and Balance Sheet endpoints, which emit ready-to-render outline rows: `{kind: heading\|account\|other\|total, label, account, depth, amounts}`. The UI and xlsx only indent, fold and format; the per-report `compressRows`/`compressIsRows`/`cfDisplayModel` labelling is retired for these three statements. |
| D8  | **Depth on the P&L.** Add the Depth slider to P&L (re-opening plan 13 D5). The request sends `depth`, and the engine clips members with the existing `IsOpts.depth`. Depth runs first, then the D1 outline is built over the clipped members, so lowering depth folds leaves into their ancestor rows. Default: unlimited (today's behaviour). |
| D9  | **Cash Flow depth made visible.** The slider max becomes the deepest account *in the cash-flow report* (engine returns `maxDepth`). Lines use the D1 outline, so depth visibly folds and unfolds levels. A single-account line shows its D2 label, not its full path. hledger.conf `--depth` seeds the slider once per conf revision and does not reset it on each tab switch. |
| D10 | **Export depth follows the screen**, as plan 24 says ("XLSX follows the screen"). The params line records the depth. There is no separate export-only depth setting. |
| D11 | **Balance Sheet and cash blocks.** The same outline applies. Two banks with checking and savings each become `Bank A › Checking / Savings` and `Bank B › Checking / Savings`. Bank names come from `name:` when the path segment is a code. |
| D12 | **Budget column, P&L only.** It is an opt-in checkbox on P&L, saved with the other report params in the URL. When on, the P&L adds an italic `Budget` column immediately after `Total`, covering the same From–To range. It also appears in the xlsx export, in italics. There are no per-interval budget columns. Cash Flow and Balance Sheet get no budget column. |
| D13 | **Budget placement.** A goal on account G attaches to the outline row whose subtree is exactly G within the group: G's leaf row, the `Total G` row when G is a heading (D4 exception), or the group Total when G's subtree is exactly the group. If G was folded away by depth or collapsed by D1, the goal attaches to the nearest shown ancestor row that contains it. Group Total, section Total and Net rows show the sum of goals placed beneath them. Actuals never move. Only the Budget cell is placed. |
| D14 | **Budget split across groups.** If G's members are spread over several groups (e.g. a goal on `expenses:people-costs` while payroll processing sits in Professional Services), no single row's actual matches the goal. The goal shows **only** on the section Total, plus a footnote under the section: "Budget for People Costs spans groups: shown in section total only." |
| D15 | **Budget numbers come from `budget.rs`.** That means the same goals, periodic-rule parsing and `remap_account` semantics as the Budget page. No second implementation. |

## Wire contract (sketch)

- `GET /api/reports/incomestatement/grouped`: new `depth` param (absent =
  unlimited), new `budget=1` param. Each group's `rows` become outline rows
  (D7). With `budget=1`, rows and totals gain a `budget` amount (D13), and each
  section gains `budgetUnplaced: [{account, label}]` for the D14 footnote.
- `GET /api/reports/cashflow`: section lines and cash lines carry outline rows;
  the response gains `maxDepth` (deepest account in the report).
- `GET /api/reports/balancesheet`: group rows become outline rows.
- Labels (D2/D3) and `name:` lookup happen in the engine; the wire carries the
  final `label` and the full `account` for hover.
- After the shape change, run `just test-integration`; `test:unit` skips the
  live-engine tests.

## Work packages

1. **WP1 Outline builder (Rust).** `reports/outline.rs` covering D1–D5: own
   `name:` lookup, humanized fallback, collision guard, `(other)` rows, depth
   fold. Unit tests: the cross-group case, a parent with postings, collisions,
   QuickBooks-numbered segments, depth fold, name tag not inherited.
2. **WP2 P&L.** Engine emits outline rows plus `depth` (D8); UI renders them
   with hover (D6) and the Depth slider; xlsx indents from the rows and adds
   path notes. Add `name` to the Accounts editor's `SPECIAL_TAGS`.
3. **WP3 Cash Flow** (D9, D11 cash blocks): outline rows, `maxDepth`,
   conf-seed fix.
4. **WP4 Balance Sheet outline** (D11).
5. **WP5 Budget column** (D4 exception, D12–D15): engine `budget=1` via
   `budget.rs`, UI checkbox, italic column, xlsx italics, footnote.
6. **WP6 Docs**: reports docs pages and the `name:` tag in the account-tags
   reference.

WP1 comes first. WP2 through WP4 can then run in parallel. WP5 depends on WP2.

## Resolved 2026-11-27

1. Totals: group only, except a budgeted heading when budgets are shown (D4).
2. Parent's own postings: `(other)` (D5).
3. Outline lives in the Rust engine (D7).
4. Export follows screen depth (D10).
5. Budget column on P&L only (D12).
6. Leading account numbers stay as written; no strip setting for now (D2).
7. Naming: P&L = income statement; keep "P&L" on the tab.
