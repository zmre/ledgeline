// Hand-written `GET /api/reports/incomestatement/grouped` response bodies, in
// the exact wire shape plans/13-income-statement-redesign.md specifies.
//
// NOT the wire contract — `fixtures/native/v1/incomestatement-grouped.json` is.
// Those are bytes a live engine produced, replayed by
// `crates/ledgeline-server/tests/native_wire_golden.rs` and swept key-by-key on
// this side by nativeDecode.test.ts's rename sweep, so a renamed Rust field
// breaks both suites at once (DRY-3). These literals were written before that
// endpoint existed and are deliberately KEPT, because the golden can only ever
// show what `fixtures/sample.journal` produces — one simple, comparing report:
//
//   * MULTI_STEP_INCOME_STATEMENT is the whole GAAP ladder, all seven sections
//     and a mixed `other`. sample.journal carries no `issection:` tags at all.
//   * UNCOMPARED_INCOME_STATEMENT is the `compare=none` shape, which the
//     snapshot request does not ask for.
//
// So: anything about the SHAPE OF THE WIRE belongs in the golden and should be
// asserted there. These stay for the display cases the journal cannot express.
// If sample.journal ever grows section tags, delete the multi-step literal
// rather than letting the two drift.
//
// PROVENANCE. Every figure in GROUPED_INCOME_STATEMENT was read out of hledger
// 1.52 against `fixtures/sample.journal`, not out of our own output (CLI `-e` is
// exclusive, hence the +1 day on each end):
//
//   hledger is -V -b 2026-01-01 -e 2026-07-09 --depth 2
//       income:salary $33,960.00   income:dividends     $50.00   → $34,010.00
//       expenses:depreciation $3,500.00
//       expenses:food  $1,654.38   expenses:housing $13,125.00
//       expenses:taxes $8,760.00   expenses:transport  $186.54
//       expenses:travel  $656.40   expenses:unknown     $75.00
//       expenses:utilities $669.16                              → $28,626.48
//       Net                                                     →  $5,383.52
//
//   hledger is -V -b 2025-06-26 -e 2026-01-01 --depth 2     (the prior window)
//       income:salary $39,360.00   income:dividends     $37.50   → $39,397.50
//       expenses:depreciation $4,000.00
//       expenses:food  $1,546.35   expenses:housing $11,175.00
//       expenses:taxes $10,110.00  expenses:transport  $182.15
//       expenses:travel  $749.10   expenses:utilities  $754.11   → $28,516.71
//       Net                                                     → $10,880.79
//
// `expenses:depreciation` is the vehicle write-down plans/14 added to the
// journal: $4,000 on 2025-06-30 (inside the prior window, which opens 2025-06-26)
// and $3,500 on 2026-06-30. It is a leaf at depth 2, so its group has one row.
//
//   hledger bal income expenses -V -b … --tree              (the account rows)
//
// The prior window is the immediately preceding window of EQUAL LENGTH:
// 2026-01-01 minus one day is 2025-12-31, less the 188-day span, is 2025-06-26.
//
// TWO fixture properties are load-bearing and were not arranged, they are what
// the journal actually contains:
//
//   * `expenses:unknown` exists in the current window and NOT in the prior one.
//   * `expenses:travel:flights` is current-only; `expenses:travel:activities` is
//     prior-only.
//
// Both are the union-join case — "a line present in only one period still
// appears with a zero on the other side" — which is the rule most likely to
// silently drop a row, and the reason the join is done in Rust rather than here.
// The absent side arrives as `{}`, an explicit empty amount, never as a missing
// key.
//
// Groups follow the shared-prefix rule: every member of the revenue section
// starts `income:` and every member of the expense section starts `expenses:`,
// so the group is the SECOND segment, humanized — which is exactly what
// `--depth 2` prints above, and why that flag is the right ground truth for
// this chart.

/** `{mantissa, places}` — the wire's exact-decimal encoding. */
const d = (mantissa: string, places: number) => ({mantissa, places});

/** `$n` at cent precision, the shape every figure in this fixture takes. */
const usd = (cents: string) => ({$: d(cents, 2)});

/** Current + prior, the wire's `Amounts`. */
const both = (current: string, prior: string) => ({current: usd(current), prior: usd(prior)});

/**
 * One plan 50 outline row: `{kind, label, level, account, depth, amounts}`.
 * `depth` is the account's own segment count, as the engine sends it.
 */
const row = (kind: "heading" | "account" | "other" | "total", label: string, level: number, account: string, amounts: unknown) => ({
    kind,
    label,
    level,
    account,
    depth: account.split(":").length,
    amounts,
});
/** The commonest row: an account directly under its group heading. */
const leaf = (label: string, account: string, amounts: unknown) => row("account", label, 0, account, amounts);

/** A line that exists in only ONE window: `{}` on the other side, never a missing key. */
const currentOnly = (current: string) => ({current: usd(current), prior: {}});
const priorOnly = (prior: string) => ({current: {}, prior: usd(prior)});

/**
 * The simple (personal-journal) response: two boxes, no ladder, comparing.
 *
 * Typed `unknown` so a test has to go through `decodeIncomeStatementReport` to
 * get anything out of it — the same posture the golden-file decoders take, and
 * the reason a wrong field name fails loudly instead of type-checking against a
 * convenient interface.
 */
export const GROUPED_INCOME_STATEMENT: unknown = {
    from: "2026-01-01",
    to: "2026-07-08",
    prior: {from: "2025-06-26", to: "2025-12-31"},
    base: "$",
    value: "market",
    // `false`: every member resolves to revenue or opex, so no rung of the
    // ladder has anything to stand on and `opex` is titled plain "Expenses".
    multiStep: false,
    maxDepth: 3,
    sections: [
        {
            kind: "revenue",
            title: "Revenue",
            // Groups are sorted by NAME, so Dividends precedes Salary — which is
            // not the order `hledger is` prints them in. Taken from the committed
            // golden rather than from the CLI: the engine's ordering is what the
            // screen shows, and this literal drifting from it is exactly the kind
            // of divergence a hand-written wire mirror is prone to.
            groups: [
                {
                    name: "Dividends",
                    source: "segment",
                    total: both("5000", "3750"),
                    rows: [leaf("Dividends", "income:dividends", both("5000", "3750"))],
                },
                {
                    name: "Salary",
                    source: "segment",
                    total: both("3396000", "3936000"),
                    rows: [leaf("Salary", "income:salary", both("3396000", "3936000"))],
                },
            ],
            total: both("3401000", "3939750"),
            trailing: [],
        },
        {
            kind: "opex",
            // "Expenses", not "Operating expenses": the same section either way,
            // and only the label moves when a journal grows its first `cogs:` tag.
            title: "Expenses",
            groups: [
                {
                    // Sorted by name, so Depreciation leads. A single leaf at
                    // depth 2: the group and its only row carry the same figure.
                    name: "Depreciation",
                    source: "segment",
                    total: both("350000", "400000"),
                    rows: [leaf("Depreciation", "expenses:depreciation", both("350000", "400000"))],
                },
                {
                    name: "Food",
                    source: "segment",
                    total: both("165438", "154635"),
                    rows: [
                        leaf("Groceries", "expenses:food:groceries", both("127250", "129520")),
                        leaf("Restaurants", "expenses:food:restaurants", both("38188", "25115")),
                    ],
                },
                {
                    name: "Housing",
                    source: "segment",
                    total: both("1312500", "1117500"),
                    // A lone child: the engine's outline (plan 50 D1) prints it as
                    // one `Rent` row, never a `Housing` heading over it.
                    rows: [leaf("Rent", "expenses:housing:rent", both("1312500", "1117500"))],
                },
                {
                    name: "Taxes",
                    source: "segment",
                    total: both("876000", "1011000"),
                    rows: [
                        leaf("Federal", "expenses:taxes:federal", both("690000", "795000")),
                        leaf("State", "expenses:taxes:state", both("186000", "216000")),
                    ],
                },
                {
                    name: "Transport",
                    source: "segment",
                    total: both("18654", "18215"),
                    rows: [leaf("Fuel", "expenses:transport:fuel", both("18654", "18215"))],
                },
                {
                    name: "Travel",
                    source: "segment",
                    total: both("65640", "74910"),
                    // The union join at ACCOUNT level: `activities` only ran in the
                    // prior window, `flights` only in the current one. Both rows
                    // exist in both periods, one side explicitly empty.
                    rows: [
                        leaf("Activities", "expenses:travel:activities", priorOnly("3960")),
                        leaf("Flights", "expenses:travel:flights", currentOnly("41280")),
                        leaf("Lodging", "expenses:travel:lodging", both("24360", "70950")),
                    ],
                },
                {
                    // The union join at GROUP level: nothing landed in
                    // `expenses:unknown` during the prior window.
                    name: "Unknown",
                    source: "segment",
                    total: currentOnly("7500"),
                    rows: [leaf("Unknown", "expenses:unknown", currentOnly("7500"))],
                },
                {
                    name: "Utilities",
                    source: "segment",
                    total: both("66916", "75411"),
                    rows: [leaf("Electric", "expenses:utilities:electric", both("66916", "75411"))],
                },
            ],
            total: both("2862648", "2851671"),
            trailing: [],
        },
    ],
    netIncome: both("538352", "1088079"),
    meta: {unpriced: []},
};

/** Recursively delete every `prior` key — the top-level window and each `Amounts`. */
function stripPrior(value: unknown): unknown {
    if (Array.isArray(value)) return value.map(stripPrior);
    if (typeof value !== "object" || value === null) return value;
    return Object.fromEntries(
        Object.entries(value as Record<string, unknown>)
            .filter(([key]) => key !== "prior")
            .map(([key, inner]) => [key, stripPrior(inner)])
    );
}

/**
 * The same body with every figure's `prior` key removed and the report's own
 * `prior` set to `null` — what `compare=none` returns (the engine always writes
 * the report-level key; it is the switch the decoder reads).
 *
 * Derived rather than written out again, so the two variants cannot drift: the
 * comparing and non-comparing shapes must agree about every CURRENT figure, and
 * a second literal is how they would stop doing so. It stays `unknown`, so a
 * test still has to go through the decoder.
 */
export const UNCOMPARED_INCOME_STATEMENT: unknown = {...(stripPrior(GROUPED_INCOME_STATEMENT) as object), prior: null};

/**
 * A tagged business journal: all seven sections, the full ladder, a mixed
 * `other`, and an `isgroup:` that merges two unrelated accounts.
 *
 * SYNTHETIC, and unlike the fixture above it makes no claim about hledger — it
 * is arithmetic, chosen so every rung of the ladder lands on a figure that is
 * checkable by hand:
 *
 *   Revenue                                                 620,000.00
 *   Cost of revenue                                        −102,600.00
 *     → Gross profit                                        517,400.00   83.5%
 *   Operating expenses                                     −356,500.00
 *     → EBITDA                                              160,900.00   26.0%
 *   Depreciation & amortization                             −24,000.00
 *     → Operating income                                    136,900.00   22.1%
 *   Other income & expense                                  −15,000.00   (net)
 *   Interest                                                −12,400.00
 *     → Income before taxes                                 109,500.00   17.7%
 *   Income taxes                                            −23,000.00
 *   Net income                                               86,500.00   14.0%
 *
 * The percentages are of the $620,000.00 revenue total and are what
 * `pctOfRevenue` must produce; 26.0% and 14.0% are there deliberately, because a
 * formatter that dropped the trailing zero would pass on every other line.
 *
 * `other` nets NEGATIVE ($30,000.00 of grants against a $45,000.00 settlement)
 * — the one section allowed to print below zero, because it is presented as a
 * net contribution to income rather than as a magnitude.
 *
 * "Cloud infrastructure" is the merged-`isgroup:` case: `cogs:hosting` and
 * `cogs:cdn` share no path prefix beyond `cogs` and are on one line only because
 * both accounts carry the tag. Its `source` is `tag`, which is the visible
 * difference from every group above.
 */
export const MULTI_STEP_INCOME_STATEMENT: unknown = {
    from: "2026-01-01",
    to: "2026-12-31",
    prior: null,
    base: "$",
    value: "market",
    multiStep: true,
    maxDepth: 3,
    sections: [
        {
            kind: "revenue",
            title: "Revenue",
            groups: [
                {
                    name: "Product",
                    source: "tag",
                    total: {current: usd("50000000")},
                    rows: [leaf("Product", "income:product", {current: usd("50000000")})],
                },
                {
                    name: "Services",
                    source: "tag",
                    total: {current: usd("12000000")},
                    rows: [leaf("Services", "income:services", {current: usd("12000000")})],
                },
            ],
            total: {current: usd("62000000")},
            trailing: [],
        },
        {
            kind: "cogs",
            title: "Cost of revenue",
            groups: [
                {
                    name: "Cloud infrastructure",
                    source: "tag",
                    total: {current: usd("8400000")},
                    // Two accounts with no shared prefix below `cogs`, on one line
                    // only because both carry `isgroup: Cloud infrastructure`.
                    rows: [leaf("Cdn", "cogs:cdn", {current: usd("1400000")}), leaf("Hosting", "cogs:hosting", {current: usd("7000000")})],
                },
                {
                    name: "Payment processing",
                    source: "tag",
                    total: {current: usd("1860000")},
                    rows: [leaf("Payments", "cogs:payments", {current: usd("1860000")})],
                },
            ],
            total: {current: usd("10260000")},
            trailing: [{kind: "grossProfit", label: "Gross profit", total: {current: usd("51740000")}}],
        },
        {
            kind: "opex",
            // "Operating expenses" here, plain "Expenses" in the simple fixture:
            // same section, and only the label moves.
            title: "Operating expenses",
            groups: [
                {
                    name: "Salaries",
                    source: "segment",
                    total: {current: usd("31000000")},
                    rows: [leaf("Salaries", "expenses:salaries", {current: usd("31000000")})],
                },
                {
                    name: "Marketing",
                    source: "segment",
                    total: {current: usd("4650000")},
                    rows: [leaf("Marketing", "expenses:marketing", {current: usd("4650000")})],
                },
            ],
            total: {current: usd("35650000")},
            trailing: [{kind: "ebitda", label: "EBITDA", total: {current: usd("16090000")}}],
        },
        {
            kind: "depreciation",
            title: "Depreciation & amortization",
            groups: [
                {
                    name: "Depreciation",
                    source: "segment",
                    total: {current: usd("2400000")},
                    rows: [leaf("Depreciation", "expenses:depreciation", {current: usd("2400000")})],
                },
            ],
            total: {current: usd("2400000")},
            trailing: [{kind: "operatingIncome", label: "Operating income", total: {current: usd("13690000")}}],
        },
        {
            kind: "other",
            title: "Other income & expense",
            groups: [
                {
                    name: "Grants",
                    source: "tag",
                    total: {current: usd("3000000")},
                    rows: [leaf("Grants", "income:grants", {current: usd("3000000")})],
                },
                {
                    // Negative: this box is a net contribution to income, so a cost
                    // inside it is written below zero rather than as a magnitude.
                    name: "Legal settlement",
                    source: "tag",
                    total: {current: usd("-4500000")},
                    rows: [leaf("Settlement", "expenses:legal:settlement", {current: usd("-4500000")})],
                },
            ],
            total: {current: usd("-1500000")},
            trailing: [],
        },
        {
            kind: "interest",
            title: "Interest",
            groups: [
                {
                    name: "Interest expense",
                    source: "tag",
                    total: {current: usd("1240000")},
                    rows: [leaf("Interest", "expenses:interest", {current: usd("1240000")})],
                },
            ],
            total: {current: usd("1240000")},
            trailing: [{kind: "pretaxIncome", label: "Income before taxes", total: {current: usd("10950000")}}],
        },
        {
            kind: "tax",
            title: "Income taxes",
            groups: [
                {
                    name: "Income taxes",
                    source: "tag",
                    total: {current: usd("2300000")},
                    rows: [leaf("Income", "expenses:taxes:income", {current: usd("2300000")})],
                },
            ],
            total: {current: usd("2300000")},
            trailing: [],
        },
    ],
    netIncome: {current: usd("8650000")},
    meta: {unpriced: []},
};

/**
 * The uncompared statement with its Travel group replaced by a deeper outline
 * (plan 50): a leaf, then a `Lodging` HEADING over its own `(other)` postings
 * and a `Hotels` child — the shape sample.journal is too shallow to produce.
 * Synthetic arithmetic, kept equal to the group's total: 412.80 + 43.60 +
 * 200.00 = 656.40. A heading's `amounts` is its subtree (243.60), which the
 * screen and the workbook must NOT print.
 */
export const OUTLINED_INCOME_STATEMENT: unknown = (() => {
    const base = UNCOMPARED_INCOME_STATEMENT as {sections: {kind: string; groups: {name: string; rows: unknown[]}[]}[]};
    const current = (cents: string) => ({current: usd(cents)});
    const travel = [
        leaf("Flights", "expenses:travel:flights", current("41280")),
        row("heading", "Lodging", 0, "expenses:travel:lodging", current("24360")),
        row("other", "Lodging (other)", 1, "expenses:travel:lodging", current("4360")),
        row("account", "Hotels", 1, "expenses:travel:lodging:hotels", current("20000")),
    ];
    return {
        ...base,
        maxDepth: 4,
        sections: base.sections.map((section) => ({
            ...section,
            groups: section.groups.map((group) => (group.name === "Travel" ? {...group, rows: travel} : group)),
        })),
    };
})();

/**
 * OUTLINED_INCOME_STATEMENT with the Budget column on (plan 50 WP5, `budget=1`):
 * every row, group, section, subtotal and the net carry a `budget` key, `null`
 * where no goal sits. Synthetic goals: Flights $500.00, Lodging $300.00 (a
 * heading, so a `total` row closes its children), and a $200.00 goal on
 * `expenses:people` that spans groups and is footnoted under Expenses
 * (D14), counted only in its total: $1,000.00. Revenue has no goal.
 */
export const BUDGETED_INCOME_STATEMENT: unknown = (() => {
    type Raw = Record<string, unknown>;
    const base = OUTLINED_INCOME_STATEMENT as {sections: (Raw & {kind: string; trailing: Raw[]; groups: (Raw & {name: string; rows: Raw[]})[]})[]};
    const goals: Record<string, unknown> = {
        Flights: usd("50000"),
        Lodging: usd("30000"),
    };
    const travel = (rows: Raw[]): Raw[] => [
        ...rows.map((r) => ({...r, budget: goals[r.label as string] ?? null})),
        {...row("total", "Total Lodging", 0, "expenses:travel:lodging", {current: usd("24360")}), budget: usd("30000")},
    ];
    return {
        ...base,
        netIncomeBudget: usd("-100000"),
        sections: base.sections.map((section) => {
            const opex = section.kind === "opex";
            return {
                ...section,
                budget: opex ? usd("100000") : null,
                budgetUnplaced: opex ? [{account: "expenses:people", label: "People"}] : [],
                trailing: section.trailing.map((subtotal) => ({...subtotal, budget: null})),
                groups: section.groups.map((group) =>
                    group.name === "Travel"
                        ? {...group, budget: usd("80000"), rows: travel(group.rows)}
                        : {...group, budget: null, rows: group.rows.map((r) => ({...r, budget: null}))}
                ),
            };
        }),
    };
})();
