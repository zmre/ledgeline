// A hand-written `GET /api/reports/balancesheet/grouped` response body, in the
// exact wire shape plans/12-balance-sheet-redesign.md specifies.
//
// TEMPORARY, and it says so on purpose. Every other native decoder is tested
// against `fixtures/native/v1/*.json` — bytes a live engine actually produced,
// replayed by `crates/ledgeline-server/tests/native_wire_golden.rs` so a renamed
// Rust field breaks both suites at once (DRY-3). That endpoint does not exist
// yet, so this literal stands in for it. **When the engine lands, add
// `balancesheet-grouped` to `fixtures/native/v1/requests.tsv`, regenerate with
// `just snapshot-native`, and move this suite onto the golden** — a hand-written
// mirror of a wire contract is exactly the thing that drifts. Keep whichever
// literals below still cover a shape the journal cannot produce; each says which.
//
// `CLASSIFIED_BALANCE_SHEET` is the current/non-current shape. sample.journal
// DOES tag three accounts `bsterm: noncurrent`, so the golden covers the wire
// shape of the bands — this literal is for the DISPLAY tests, whose subject is
// where a heading opens and a subtotal closes rather than what the engine
// computed, and whose figures are round on purpose so a band subtotal is
// checkable by eye. Delete it when this suite moves onto the golden, after
// checking the golden still exercises the two things chosen for here: a band
// holding MORE THAN ONE group (only a heading over several groups can show the
// heading is not per-group), and an unbanded equity section standing beside two
// banded ones in the same report.
//
// PROVENANCE. Every figure below was read out of hledger 1.52 against
// `fixtures/sample.journal` at as-of 2026-07-08 (CLI `-e` is exclusive, hence
// `-e 2026-07-09`), not out of our own output:
//
//   hledger bal assets -V -e 2026-07-09 --depth 3 -c '$1000000.00000000'
//       checking $28292.81  savings $13500.00  wise $657.43
//       broker:taxable $17162.375, 5.0 GLD, -2.0 TSLA
//       total          $59612.615, 5.0 GLD, -2.0 TSLA
//   hledger bs -e 2026-07-09 --depth 3          liabilities:cc:visa $531.15
//   hledger bal equity -B -e 2026-07-09         opening $14550.00, transfers 5.0 GLD
//   hledger is -B -e 2026-07-09                 Net $42998.91, -933,25 EUR
//   hledger bse -B -e 2026-07-09                assets at cost $58080.06, -933,25 EUR, 5.0 GLD
//
// The synthetic equity lines follow from those:
//   Retained earnings    = the `is -B` net.
//   Valuation adjustment = assets at market − assets at cost
//                        = $1532.555, +933,25 EUR, −2.0 TSLA.
// (Not "unrealized gains": the EUR and TSLA parts are currency revaluation and
// an unpriced holding, neither of which is a holding gain.)
//
// Those choices are what make the identity hold, and the fixture is built so it
// does — `check` is `{}` here because A − L − E is exactly zero in every
// commodity. See the note in the plan review about `Equity, declared`: the
// AT-COST equity ($14,550.00 + 5 GLD) is what balances; the unvalued $15,550.00
// figure the plan's table quotes does not.
//
// Decimal places are the engine's, not a display convention: unrounded products
// keep their full precision (`$17162.375` is places 3), and GLD/TSLA arrive as
// places 0 exactly as they do in the committed `balancesheet.json` golden.

/** `{mantissa, places}` — the wire's exact-decimal encoding. */
const d = (mantissa: string, places: number) => ({mantissa, places});

const CHECKING = d("2829281", 2);
const SAVINGS = d("1350000", 2);
const WISE = d("65743", 2);
const CASH_TOTAL = d("4245024", 2); // 28292.81 + 13500.00 + 657.43
const BROKER = {$: d("17162375", 3), GLD: d("5", 0), TSLA: d("-2", 0)};
const ASSETS_TOTAL = {$: d("59612615", 3), GLD: d("5", 0), TSLA: d("-2", 0)};
const VISA = d("53115", 2);
const NET_WORTH = {$: d("59081465", 3), GLD: d("5", 0), TSLA: d("-2", 0)}; // assets − liabilities

/**
 * The response body. Typed `unknown` so a test has to go through
 * `decodeBalanceSheetReport` to get anything out of it — the same posture the
 * golden-file decoders take, and the reason a wrong field name fails loudly
 * instead of type-checking against a convenient interface.
 *
 * Every group here has `term: null` and every section an empty `subsections`:
 * this is the UNCLASSIFIED shape, deliberately, and it is what pins "a journal
 * that classifies nothing renders exactly what it rendered before the
 * current/non-current axis existed". `sample.journal` has since been tagged, so
 * the golden can no longer stand in for this case at all — which is the one
 * reason this literal is worth more than the bytes it mirrors. The classified
 * shape is `CLASSIFIED_BALANCE_SHEET` below.
 */
export const GROUPED_BALANCE_SHEET: unknown = {
    asOf: "2026-07-08",
    base: "$",
    value: "market",
    sections: [
        {
            kind: "assets",
            title: "Assets",
            groups: [
                {
                    name: "Cash and cash equivalents",
                    source: "type", // every member resolves to the declared `type: C`
                    term: null, // no `bsterm:` anywhere in this journal
                    total: {$: CASH_TOTAL},
                    rows: [
                        {account: "assets:bank", depth: 2, own: {}, inclusive: {$: CASH_TOTAL}},
                        {account: "assets:bank:checking", depth: 3, own: {$: CHECKING}, inclusive: {$: CHECKING}},
                        {account: "assets:bank:savings", depth: 3, own: {$: SAVINGS}, inclusive: {$: SAVINGS}},
                        // Another single-child chain, and the reason the tab asks
                        // the engine for an UNCLAMPED report: `assets:bank:wise`
                        // holds nothing itself, so stopping at depth 3 showed a
                        // row that stood for an account the reader could not see.
                        // `compressSectionRows` renders the pair as one
                        // `wise:eur` row.
                        {account: "assets:bank:wise", depth: 3, own: {}, inclusive: {$: WISE}},
                        {account: "assets:bank:wise:eur", depth: 4, own: {$: WISE}, inclusive: {$: WISE}},
                    ],
                },
                {
                    name: "Investments",
                    source: "commodity", // holds non-base commodities (AAPL/VTI/GLD/TSLA)
                    term: null,
                    total: BROKER,
                    rows: [
                        // A single-child chain: `compressSectionRows` renders these as one
                        // `broker:taxable` row on screen and in the workbook.
                        {account: "assets:broker", depth: 2, own: {}, inclusive: BROKER},
                        {account: "assets:broker:taxable", depth: 3, own: BROKER, inclusive: BROKER},
                    ],
                },
            ],
            // Empty, not absent: the engine always sends the key, and an absent
            // one would be indistinguishable from this — an unclassified journal.
            subsections: [],
            total: ASSETS_TOTAL,
        },
        {
            kind: "liabilities",
            title: "Liabilities",
            groups: [
                {
                    name: "Credit cards",
                    source: "segment", // second segment `cc`, prettified by the alias table
                    term: null,
                    total: {$: VISA},
                    rows: [
                        {account: "liabilities:cc", depth: 2, own: {}, inclusive: {$: VISA}},
                        {account: "liabilities:cc:visa", depth: 3, own: {$: VISA}, inclusive: {$: VISA}},
                    ],
                },
            ],
            subsections: [],
            total: {$: VISA},
        },
        {
            kind: "equity",
            title: "Equity",
            groups: [
                {
                    name: "Opening",
                    source: "segment",
                    // Null on every equity group in every journal: equity is not
                    // split by term, tagged or not.
                    term: null,
                    total: {$: d("1455000", 2)},
                    rows: [{account: "equity:opening", depth: 2, own: {$: d("1455000", 2)}, inclusive: {$: d("1455000", 2)}}],
                },
                {
                    name: "Transfers",
                    source: "segment",
                    term: null,
                    total: {GLD: d("5", 0)},
                    rows: [{account: "equity:transfers", depth: 2, own: {GLD: d("5", 0)}, inclusive: {GLD: d("5", 0)}}],
                },
                // The two computed lines carry a total and NO rows: they summarize
                // accounts that are not on the balance sheet at all. Nothing to
                // expand, so the UI must render them without a disclosure triangle.
                {
                    name: "Retained earnings",
                    source: "computed",
                    term: null,
                    total: {$: d("4299891", 2), EUR: d("-93325", 2)},
                    rows: [],
                },
                {
                    name: "Valuation adjustment",
                    source: "computed",
                    term: null,
                    total: {$: d("1532555", 3), EUR: d("93325", 2), TSLA: d("-2", 0)},
                    rows: [],
                },
            ],
            // Always empty on equity, in every journal — there is no such thing
            // as a current or non-current share of capital.
            subsections: [],
            // EUR nets to zero across the two computed lines and is therefore
            // ABSENT here — the wire drops zero commodities.
            total: {$: d("59081465", 3), GLD: d("5", 0), TSLA: d("-2", 0)},
        },
    ],
    netWorth: NET_WORTH,
    // A − L − E is exactly zero in $, GLD, EUR and TSLA.
    check: {},
    // Sent on every response, and the ONLY thing a client may render a verdict
    // from — `check` being empty is not the same question (see the engine's
    // `is_balanced`).
    balanced: true,
    // GLD and TSLA have no `P` directive anywhere in the fixture journal, so the
    // valuation genuinely could not convert them.
    meta: {unpriced: ["GLD", "TSLA"]},
};

/**
 * The same report with a deliberate `$0.005` imbalance, for the check-line path.
 *
 * A half CENT on purpose: it is invisible at the 2-decimal display cap, so a
 * verdict computed from rendered strings would call this balanced. It is a real
 * imbalance for a journal that writes dollars to three places — one unit of
 * `$0.001` is what the engine measures it against — which is exactly the pairing
 * the wire carries: a residual the client must not judge for itself, next to the
 * `balanced: false` that judges it.
 */
export const UNBALANCED_BALANCE_SHEET: unknown = {
    ...(GROUPED_BALANCE_SHEET as Record<string, unknown>),
    check: {$: d("5", 3)},
    balanced: false,
};

/**
 * A journal that classifies its accounts: Assets and Liabilities banded into
 * current and non-current, Equity untouched.
 *
 * SYNTHETIC, and unlike the fixture above it makes no claim about hledger — it
 * is round arithmetic, chosen so every band subtotal is checkable by eye:
 *
 *   Assets      current      50,000 cash + 12,500 receivable   =  62,500
 *               non-current 450,000 property + 87,500 IRA      = 537,500
 *                                                       Total  = 600,000
 *   Liabilities current       4,200 cards + 7,800 payable      =  12,000
 *               non-current 288,000 mortgage                   = 288,000
 *                                                       Total  = 300,000
 *   Equity      250,000 opening + 50,000 retained              = 300,000
 *
 * so `A = L + E` (600,000 = 300,000 + 300,000) and net worth is 300,000.
 *
 * Four properties here are load-bearing rather than decorative:
 *
 *   * Bands are CONTIGUOUS and current-first, which is the engine's ordering
 *     invariant and the reason one pass over the groups can decide where a
 *     heading opens and a subtotal closes.
 *   * `heading` and `label` are the engine's prose. Nothing on the TS side may
 *     compose "Total non-current assets" — see `BsSubsection`.
 *   * Equity carries `subsections: []` and `term: null` throughout, next to two
 *     banded sections, so "equity is never split" is exercised in the one report
 *     where splitting is otherwise happening.
 *   * `Property` is a single-child chain (`assets:property` holds nothing
 *     itself), so chain compression still has to work inside a band.
 *
 * `meta` is omitted: every commodity here is the base one, so nothing is
 * unpriced and the engine sends no meta at all.
 */
export const CLASSIFIED_BALANCE_SHEET: unknown = {
    asOf: "2026-07-08",
    base: "$",
    value: "market",
    sections: [
        {
            kind: "assets",
            title: "Assets",
            groups: [
                {
                    name: "Cash and cash equivalents",
                    source: "type",
                    term: "current",
                    total: {$: d("5000000", 2)},
                    rows: [
                        {account: "assets:bank", depth: 2, own: {}, inclusive: {$: d("5000000", 2)}},
                        {account: "assets:bank:checking", depth: 3, own: {$: d("3000000", 2)}, inclusive: {$: d("3000000", 2)}},
                        {account: "assets:bank:savings", depth: 3, own: {$: d("2000000", 2)}, inclusive: {$: d("2000000", 2)}},
                    ],
                },
                {
                    name: "Accounts receivable",
                    source: "segment", // second segment `ar`, prettified by the alias table
                    term: "current",
                    total: {$: d("1250000", 2)},
                    rows: [{account: "assets:ar", depth: 2, own: {$: d("1250000", 2)}, inclusive: {$: d("1250000", 2)}}],
                },
                {
                    name: "Property",
                    source: "tag",
                    term: "noncurrent",
                    total: {$: d("45000000", 2)},
                    // A single-child chain: `compressSectionRows` renders these as
                    // one `assets:property:house` row, inside the band.
                    rows: [
                        {account: "assets:property", depth: 2, own: {}, inclusive: {$: d("45000000", 2)}},
                        {account: "assets:property:house", depth: 3, own: {$: d("45000000", 2)}, inclusive: {$: d("45000000", 2)}},
                    ],
                },
                {
                    name: "Long-term investments",
                    source: "tag",
                    term: "noncurrent",
                    total: {$: d("8750000", 2)},
                    rows: [{account: "assets:broker:ira", depth: 3, own: {$: d("8750000", 2)}, inclusive: {$: d("8750000", 2)}}],
                },
            ],
            subsections: [
                {term: "current", heading: "Current", label: "Total current assets", total: {$: d("6250000", 2)}},
                {term: "noncurrent", heading: "Non-current", label: "Total non-current assets", total: {$: d("53750000", 2)}},
            ],
            total: {$: d("60000000", 2)},
        },
        {
            kind: "liabilities",
            title: "Liabilities",
            groups: [
                {
                    name: "Credit cards",
                    source: "segment",
                    term: "current",
                    total: {$: d("420000", 2)},
                    rows: [
                        {account: "liabilities:cc", depth: 2, own: {}, inclusive: {$: d("420000", 2)}},
                        {account: "liabilities:cc:visa", depth: 3, own: {$: d("420000", 2)}, inclusive: {$: d("420000", 2)}},
                    ],
                },
                {
                    name: "Accounts payable",
                    source: "segment",
                    term: "current",
                    total: {$: d("780000", 2)},
                    rows: [{account: "liabilities:ap", depth: 2, own: {$: d("780000", 2)}, inclusive: {$: d("780000", 2)}}],
                },
                {
                    name: "Long-term debt",
                    source: "tag",
                    term: "noncurrent",
                    total: {$: d("28800000", 2)},
                    rows: [{account: "liabilities:mortgage", depth: 2, own: {$: d("28800000", 2)}, inclusive: {$: d("28800000", 2)}}],
                },
            ],
            subsections: [
                {term: "current", heading: "Current", label: "Total current liabilities", total: {$: d("1200000", 2)}},
                {term: "noncurrent", heading: "Non-current", label: "Total non-current liabilities", total: {$: d("28800000", 2)}},
            ],
            total: {$: d("30000000", 2)},
        },
        {
            kind: "equity",
            title: "Equity",
            groups: [
                {
                    name: "Opening",
                    source: "segment",
                    term: null,
                    total: {$: d("25000000", 2)},
                    rows: [{account: "equity:opening", depth: 2, own: {$: d("25000000", 2)}, inclusive: {$: d("25000000", 2)}}],
                },
                {
                    name: "Retained earnings",
                    source: "computed",
                    term: null,
                    total: {$: d("5000000", 2)},
                    rows: [],
                },
            ],
            subsections: [],
            total: {$: d("30000000", 2)},
        },
    ],
    netWorth: {$: d("30000000", 2)}, // assets − liabilities
    check: {},
    balanced: true,
};

/**
 * The classified sheet with one group NAME on both sides of the axis.
 *
 * The engine keys groups by `(term, name)`, so a single `bsgroup:` whose
 * accounts are partly current and partly not prints as two lines under two
 * subheadings — a receivable due this year and one due in five are two lines on
 * a real statement, not a defect.
 *
 * DERIVED by renaming one group rather than written out a third time: a rename
 * moves no money, so every figure and every subtotal above still holds, and the
 * two fixtures cannot drift into disagreeing about the arithmetic.
 *
 * It exists because it is the case that breaks a display key made of section and
 * group name alone — two rows with one key share a collapse state, share a
 * cursor stop, and are a duplicate `{#each}` key.
 */
export const STRADDLING_BALANCE_SHEET: unknown = JSON.parse(JSON.stringify(CLASSIFIED_BALANCE_SHEET).replace(/Long-term investments/g, "Accounts receivable"));
