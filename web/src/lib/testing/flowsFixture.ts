// A hand-written `GET /api/reports/incomestatement/flows` response body, in the
// exact wire shape crates/ledgeline-server/src/reports_api.rs serializes.
//
// NOT the wire contract. `fixtures/native/v1/incomestatement-flows.json` is:
// those are bytes a live engine produced, replayed by
// `crates/ledgeline-server/tests/native_wire_golden.rs` and swept key-by-key on
// this side by nativeDecode.test.ts's rename sweep, so a renamed Rust field
// breaks both suites at once (DRY-3). Anything about the SHAPE OF THE WIRE
// belongs there and should be asserted there.
//
// This literal exists for the two DISPLAY cases the golden cannot show, because
// `fixtures/sample.journal` does not contain them:
//
//   * A FOLDED TAIL. The golden's two graphs name exactly 8 distinct accounts
//     between them, which is `CATEGORICAL.length`, so nothing in it folds. This
//     body names 11, three of them past the last slot, and all three feed the
//     SAME statement line, so the three links have to re-aggregate into one.
//   * AN INCOMPLETE GRAPH. `inflows.total` is $300.00 short of its
//     `sectionTotal`, which is what a revenue line that netted negative over the
//     window looks like. Every graph in the golden ties out exactly.
//
// The figures are internally consistent the way the engine's are, and the tests
// lean on that: each node's `total` is the sum of the links touching it, each
// graph's `total` is the sum of its links, and the nodes and links are ordered
// biggest-first, as the engine emits them.

/** `{mantissa, places}`: the wire's exact-decimal encoding. */
const d = (mantissa: string, places: number) => ({mantissa, places});

/** `$n` at cent precision, the shape every figure here takes. */
const usd = (cents: string) => d(cents, 2);

/**
 * Money in: two revenue lines feeding three accounts, $300.00 of the section
 * undrawn.
 */
const INFLOWS = {
    nodes: [
        {key: "g:Salary", label: "Salary", side: "source", account: null, total: usd("500000")},
        {key: "a:assets:bank:checking", label: "Bank: Checking", side: "target", account: "assets:bank:checking", total: usd("400000")},
        {key: "a:assets:bank:savings", label: "Bank: Savings", side: "target", account: "assets:bank:savings", total: usd("120000")},
        {key: "g:Dividends", label: "Dividends", side: "source", account: null, total: usd("70000")},
        {key: "a:assets:broker:taxable:cash", label: "Broker: Taxable: Cash", side: "target", account: "assets:broker:taxable:cash", total: usd("50000")},
    ],
    links: [
        {source: "g:Salary", target: "a:assets:bank:checking", value: usd("400000")},
        {source: "g:Salary", target: "a:assets:bank:savings", value: usd("100000")},
        {source: "g:Dividends", target: "a:assets:broker:taxable:cash", value: usd("50000")},
        {source: "g:Dividends", target: "a:assets:bank:savings", value: usd("20000")},
    ],
    total: usd("570000"),
    sectionTotal: usd("600000"),
};

/**
 * Money out: ten funding accounts against six cost lines. The three smallest
 * accounts fall past the last palette slot, and all three pay Utilities.
 */
const OUTFLOWS = {
    nodes: [
        {key: "a:assets:bank:checking", label: "Bank: Checking", side: "source", account: "assets:bank:checking", total: usd("500000")},
        {key: "g:Housing", label: "Housing", side: "target", account: null, total: usd("400000")},
        {key: "a:liabilities:cc:visa", label: "Credit cards: Visa", side: "source", account: "liabilities:cc:visa", total: usd("300000")},
        {key: "g:Food", label: "Food", side: "target", account: null, total: usd("270000")},
        {key: "g:Utilities", label: "Utilities", side: "target", account: null, total: usd("130000")},
        {key: "g:Taxes", label: "Taxes", side: "target", account: null, total: usd("115000")},
        {key: "g:Transport", label: "Transport", side: "target", account: null, total: usd("100000")},
        {key: "a:assets:bank:savings", label: "Bank: Savings", side: "source", account: "assets:bank:savings", total: usd("90000")},
        {key: "a:assets:bank:wise:eur", label: "Bank: Wise: Eur", side: "source", account: "assets:bank:wise:eur", total: usd("40000")},
        {key: "a:liabilities:cc:amex", label: "Credit cards: Amex", side: "source", account: "liabilities:cc:amex", total: usd("30000")},
        {key: "a:assets:cash:wallet", label: "Cash: Wallet", side: "source", account: "assets:cash:wallet", total: usd("25000")},
        {
            key: "a:assets:vehicles:car:depreciation",
            label: "Vehicles: Car: Depreciation",
            side: "source",
            account: "assets:vehicles:car:depreciation",
            total: usd("20000"),
        },
        {key: "g:Depreciation", label: "Depreciation", side: "target", account: null, total: usd("20000")},
        // The tail: three accounts past `CATEGORICAL.length`, all paying Utilities.
        {key: "a:liabilities:loan:auto", label: "Loan: Auto", side: "source", account: "liabilities:loan:auto", total: usd("15000")},
        {key: "a:assets:bank:joint", label: "Bank: Joint", side: "source", account: "assets:bank:joint", total: usd("10000")},
        {key: "a:assets:prepaid:transit", label: "Prepaid: Transit", side: "source", account: "assets:prepaid:transit", total: usd("5000")},
    ],
    links: [
        {source: "a:assets:bank:checking", target: "g:Housing", value: usd("400000")},
        {source: "a:liabilities:cc:visa", target: "g:Food", value: usd("200000")},
        {source: "a:assets:bank:checking", target: "g:Utilities", value: usd("100000")},
        {source: "a:liabilities:cc:visa", target: "g:Transport", value: usd("100000")},
        {source: "a:assets:bank:savings", target: "g:Taxes", value: usd("90000")},
        {source: "a:assets:bank:wise:eur", target: "g:Food", value: usd("40000")},
        {source: "a:liabilities:cc:amex", target: "g:Food", value: usd("30000")},
        {source: "a:assets:cash:wallet", target: "g:Taxes", value: usd("25000")},
        {source: "a:assets:vehicles:car:depreciation", target: "g:Depreciation", value: usd("20000")},
        {source: "a:liabilities:loan:auto", target: "g:Utilities", value: usd("15000")},
        {source: "a:assets:bank:joint", target: "g:Utilities", value: usd("10000")},
        {source: "a:assets:prepaid:transit", target: "g:Utilities", value: usd("5000")},
    ],
    total: usd("1035000"),
    sectionTotal: usd("1035000"),
};

export const FLOW_REPORT: unknown = {
    from: "2026-01-01",
    to: "2026-07-08",
    base: "$",
    inflows: INFLOWS,
    outflows: OUTFLOWS,
    meta: {unpriced: []},
};

/**
 * The no-base answer: several commodities and nothing pricing them against each
 * other, so the engine sends two empty graphs rather than widths in a unit it
 * had to invent. The panel names that reason specifically.
 */
export const UNPRICEABLE_FLOW_REPORT: unknown = {
    from: "2026-01-01",
    to: "2026-07-08",
    base: null,
    inflows: {nodes: [], links: [], total: d("0", 0), sectionTotal: d("0", 0)},
    outflows: {nodes: [], links: [], total: d("0", 0), sectionTotal: d("0", 0)},
    meta: {unpriced: ["BTC", "GLD"]},
};

/** A base commodity, but nothing in the window: the OTHER empty state. */
export const EMPTY_FLOW_REPORT: unknown = {
    from: "2026-01-01",
    to: "2026-01-02",
    base: "$",
    inflows: {nodes: [], links: [], total: usd("0"), sectionTotal: usd("0")},
    outflows: {nodes: [], links: [], total: usd("0"), sectionTotal: usd("0")},
    meta: {unpriced: []},
};
