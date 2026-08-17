// The New Transactions flow's wire fixtures — the literal JSON the engine sends,
// shared by the store test (node project) and the component tests.
//
// One copy on purpose. These describe a CONTRACT with the Rust half, and the two
// projects assert different things about the same responses: `importStore.test.ts`
// checks what the store makes of them, the `*.svelte.test.ts` files check what the
// screen makes of that. Two copies would let one project keep passing against a
// shape the engine no longer sends.
//
// (The `routes`/`json` fetch stub in `fakeEngine.ts` is the same idea for the
// transport. Several older node tests still carry a private copy of it; those
// predate this module and are left alone rather than swept up here.)

/** `GET /api/import/capabilities` — a server that can do everything, so nothing is hidden by a missing capability. */
export const CAPABILITIES = {
    hledger: {available: true, version: "1.52"},
    formats: ["csv", "ofx"],
    journals: [
        {id: "2026/2026.journal", label: "2026.journal", txnCount: 412, lastTxnDate: "2026-08-01", isRoot: false, writable: true},
        {id: "2025/2025.journal", label: "2025.journal", txnCount: 900, lastTxnDate: "2025-12-31", isRoot: false, writable: true},
    ],
    git: {available: true, autocommit: true},
    editable: true,
};

/** `POST /api/import/stage` — one converted file, two ranked rules candidates. */
export const STAGE = {
    stageId: "stage-1",
    format: "csv",
    preview: {header: ["date"], rows: [["2026-06-24"]], rowCount: 1, truncated: false},
    statement: {ledgerBalance: "-3238.65"},
    notes: [],
    candidates: [
        {
            id: "import/2026/bank.csv.rules",
            label: "bank",
            score: 0.98,
            // The account every posting this rules file makes lands in, and so
            // the only account its statement balance could be asserted against.
            account1: "assets:bank:checking",
            signals: {txns: 1, postings: 2, amountlessPostings: 0, bareCommodityAmounts: 0, unknownAccounts: 0},
            sample: [],
        },
        {
            id: "import/2026/card.csv.rules",
            label: "card",
            score: 0.4,
            account1: "liabilities:card:visa",
            signals: {txns: 1, postings: 2, amountlessPostings: 0, bareCommodityAmounts: 0, unknownAccounts: 0},
            sample: [],
        },
    ],
    defaults: {csvPath: "import/2026/whatever.csv", journalId: "2026/2026.journal"},
};

/** A `File` to offer the store. Node's and jsdom's are both enough for `.name` and `.arrayBuffer()`. */
export const upload = (name: string): File => new File(["date\n2026-06-24\n"], name, {type: "text/csv"});
