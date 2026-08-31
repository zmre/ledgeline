import {readFileSync} from "node:fs";
import {describe, expect, it, vi} from "vitest";
import {ApiShapeError} from "./client";
import {lastSkippedAccountCount, normalizeAccounts, normalizeDiagnostics, normalizePrices, normalizeTransactions} from "./normalize";

// Hand-rolled wire samples (fixtures/api snapshots are WP-09).
// "Modern" shape verified against a live hledger 1.52: acost/asdecimalmark/UnitCost.
// "Legacy" shape per the drift table: aprice/asdecimalpoint/UnitPrice.

const usdStyleModern = {
    ascommodityside: "L",
    ascommodityspaced: false,
    asdecimalmark: ".",
    asdigitgroups: [",", [3]],
    asprecision: 2,
    asrounding: "NoRounding",
};

const modernTxn = {
    tindex: 2,
    tdate: "2026-01-10",
    tdate2: null,
    tstatus: "Pending",
    tdescription: "Grocery run",
    tcode: "CHK42",
    tcomment: "type: food\n",
    ttags: [["type", "food"]],
    tprecedingcomment: "",
    tsourcepos: [],
    tpostings: [
        {
            paccount: "expenses:food:groceries",
            pstatus: "Unmarked",
            pcomment: "organic:\n",
            ptags: [["organic", ""]],
            pdate: null,
            pdate2: null,
            pbalanceassertion: null,
            ptype: "RegularPosting",
            poriginal: null,
            ptransaction_: "2",
            pamount: [
                {
                    acommodity: "$",
                    aquantity: {decimalMantissa: 8720, decimalPlaces: 2, floatingPoint: 87.2},
                    astyle: usdStyleModern,
                    acost: null,
                    acostbasis: null,
                },
            ],
        },
        {
            paccount: "assets:broker:aapl",
            pstatus: "Cleared",
            pcomment: "",
            ptags: [],
            pdate: "2026-01-11",
            pdate2: null,
            pbalanceassertion: null,
            ptype: "RegularPosting",
            poriginal: null,
            ptransaction_: "2",
            pamount: [
                {
                    acommodity: "AAPL",
                    aquantity: {decimalMantissa: 3, decimalPlaces: 0, floatingPoint: 3},
                    astyle: {
                        ascommodityside: "R",
                        ascommodityspaced: true,
                        asdecimalmark: null,
                        asdigitgroups: null,
                        asprecision: 0,
                        asrounding: "NoRounding",
                    },
                    acost: {
                        tag: "UnitCost",
                        contents: {
                            acommodity: "$",
                            aquantity: {decimalMantissa: 22850, decimalPlaces: 2, floatingPoint: 228.5},
                            astyle: usdStyleModern,
                            acost: null,
                            acostbasis: null,
                        },
                    },
                    acostbasis: null,
                },
            ],
        },
    ],
};

const legacyTxn = {
    tindex: 1,
    tdate: "2025-12-31",
    tdate2: "2026-01-02",
    tstatus: "Cleared",
    tdescription: "Euro dinner",
    tcode: "",
    tcomment: "",
    ttags: [],
    tpostings: [
        {
            paccount: "expenses:travel:food",
            pstatus: "Unmarked",
            pcomment: "",
            ptags: [],
            pdate: null,
            pamount: [
                {
                    acommodity: "EUR",
                    aquantity: {decimalMantissa: 4500, decimalPlaces: 2, floatingPoint: 45},
                    astyle: {
                        ascommodityside: "R",
                        ascommodityspaced: true,
                        asdecimalpoint: ",",
                        asdigitgroups: [".", [3]],
                        asprecision: "NaturalPrecision",
                    },
                    aprice: {
                        tag: "TotalPrice",
                        contents: {
                            acommodity: "$",
                            aquantity: {decimalMantissa: 4860, decimalPlaces: 2, floatingPoint: 48.6},
                            astyle: {ascommodityside: "L", ascommodityspaced: false, asdecimalpoint: ".", asdigitgroups: null, asprecision: 2},
                        },
                    },
                    aismultiplier: false,
                },
            ],
        },
        {
            paccount: "liabilities:card",
            pstatus: "Unmarked",
            pcomment: "",
            ptags: [],
            pdate: null,
            pamount: [
                {
                    acommodity: "$",
                    aquantity: {decimalMantissa: -4860, decimalPlaces: 2, floatingPoint: -48.6},
                    astyle: {ascommodityside: "L", ascommodityspaced: false, asdecimalpoint: ".", asdigitgroups: [",", [3]], asprecision: 2},
                    aprice: null,
                    aismultiplier: false,
                },
            ],
        },
    ],
};

describe("UNIT normalizeTransactions", () => {
    it("normalizes the modern (1.52/2.0-preview) shape: acost, asdecimalmark", () => {
        const [txn] = normalizeTransactions([modernTxn]);
        expect(txn.index).toBe(2);
        expect(txn.date).toBe("2026-01-10");
        expect(txn.date2).toBeUndefined(); // null on the wire → absent
        expect(txn.status).toBe("pending");
        expect(txn.description).toBe("Grocery run");
        expect(txn.code).toBe("CHK42");
        expect(txn.comment).toBe("type: food");
        expect(txn.tags).toEqual([["type", "food"]]);
        expect(txn.postings).toHaveLength(2);

        const [groceries, broker] = txn.postings;
        expect(groceries.account).toBe("expenses:food:groceries");
        expect(groceries.status).toBe("unmarked");
        expect(groceries.tags).toEqual([["organic", ""]]);
        expect(groceries.date).toBeUndefined();
        expect(groceries.amounts[0].qty).toEqual({m: 8720n, p: 2});
        expect(groceries.amounts[0].style).toEqual({side: "L", spaced: false, precision: 2, decimalPoint: ".", digitGroups: [",", [3]]});

        expect(broker.status).toBe("cleared");
        expect(broker.date).toBe("2026-01-11");
        const aapl = broker.amounts[0];
        expect(aapl.commodity).toBe("AAPL");
        expect(aapl.qty).toEqual({m: 3n, p: 0});
        expect(aapl.style.side).toBe("R");
        expect(aapl.style.spaced).toBe(true);
        expect(aapl.cost).toEqual({commodity: "$", qty: {m: 22850n, p: 2}, per: true});
    });

    it("normalizes the legacy shape: aprice, asdecimalpoint, NaturalPrecision", () => {
        const [txn] = normalizeTransactions([legacyTxn]);
        expect(txn.status).toBe("cleared");
        expect(txn.date2).toBe("2026-01-02");

        const eur = txn.postings[0].amounts[0];
        expect(eur.qty).toEqual({m: 4500n, p: 2});
        // NaturalPrecision falls back to the quantity's own decimal places
        expect(eur.style).toEqual({side: "R", spaced: true, precision: 2, decimalPoint: ",", digitGroups: [".", [3]]});
        // TotalPrice → cost with per=false
        expect(eur.cost).toEqual({commodity: "$", qty: {m: 4860n, p: 2}, per: false});

        expect(txn.postings[1].amounts[0].cost).toBeUndefined();
    });

    it("builds a lowercase haystack from desc, comments, accounts, amounts, commodities", () => {
        const [txn] = normalizeTransactions([modernTxn]);
        expect(txn.haystack).toContain("grocery run");
        expect(txn.haystack).toContain("expenses:food:groceries");
        expect(txn.haystack).toContain("organic");
        expect(txn.haystack).toContain("$87.20"); // formatted amount
        expect(txn.haystack).toContain("3 aapl"); // spaced right-side commodity
        expect(txn.haystack).toContain("type: food"); // txn comment
        expect(txn.haystack).not.toMatch(/[A-Z]/);
    });

    it("freezes transactions, postings, and amounts", () => {
        "use strict";
        const [txn] = normalizeTransactions([modernTxn]);
        expect(Object.isFrozen(txn)).toBe(true);
        expect(Object.isFrozen(txn.postings)).toBe(true);
        expect(Object.isFrozen(txn.postings[0])).toBe(true);
        expect(Object.isFrozen(txn.postings[0].amounts[0])).toBe(true);
        expect(Object.isFrozen(txn.postings[0].amounts[0].style)).toBe(true);
        expect(Object.isFrozen(txn.postings[0].amounts[0].qty)).toBe(true);
        expect(() => {
            (txn as {description: string}).description = "mutated";
        }).toThrow(TypeError);
    });

    it("throws ApiShapeError naming the transaction when decimalMantissa is unsafe", () => {
        const unsafe = {
            ...modernTxn,
            tindex: 7,
            tdescription: "Huge amount",
            tpostings: [
                {
                    paccount: "assets:whale",
                    pstatus: "Unmarked",
                    pcomment: "",
                    ptags: [],
                    pdate: null,
                    pamount: [
                        {acommodity: "$", aquantity: {decimalMantissa: 2 ** 53, decimalPlaces: 2, floatingPoint: 9e15}, astyle: usdStyleModern, acost: null},
                    ],
                },
            ],
        };
        expect(() => normalizeTransactions([unsafe])).toThrow(ApiShapeError);
        expect(() => normalizeTransactions([unsafe])).toThrow(/transaction #7 "Huge amount".*safe integer/);
    });

    it("throws ApiShapeError on non-array input and missing tindex/tdate", () => {
        expect(() => normalizeTransactions({})).toThrow(ApiShapeError);
        expect(() => normalizeTransactions([{tdescription: "no index"}])).toThrow(ApiShapeError);
    });

    it("normalizes a legacy-shaped (aprice/asdecimalpoint) sample identically to its modern equivalent", () => {
        expect(normalizeTransactions([toLegacyShape(modernTxn)])).toEqual(normalizeTransactions([modernTxn]));
    });

    it("canonicalizes a 1.52 signed @@ total cost (sell) to its unsigned magnitude", () => {
        // Verified empirically: hledger 1.52 emits TotalCost aquantity SIGNED on
        // sells (-4.5 AAPL @@ $-1,117.35 on the wire) — the domain contract is
        // an unsigned cost.qty with the sign carried by the posting amount.
        const signedSell = {
            tindex: 9,
            tdate: "2026-04-01",
            tdate2: null,
            tstatus: "Cleared",
            tdescription: "Sell AAPL",
            tcode: "",
            tcomment: "",
            ttags: [],
            tprecedingcomment: "",
            tsourcepos: [],
            tpostings: [
                {
                    paccount: "assets:broker:aapl",
                    pstatus: "Unmarked",
                    pcomment: "",
                    ptags: [],
                    pdate: null,
                    pdate2: null,
                    pbalanceassertion: null,
                    ptype: "RegularPosting",
                    poriginal: null,
                    ptransaction_: "9",
                    pamount: [
                        {
                            acommodity: "AAPL",
                            aquantity: {decimalMantissa: -45000, decimalPlaces: 4, floatingPoint: -4.5},
                            astyle: {
                                ascommodityside: "R",
                                ascommodityspaced: true,
                                asdecimalmark: ".",
                                asdigitgroups: null,
                                asprecision: 4,
                                asrounding: "NoRounding",
                            },
                            acost: {
                                tag: "TotalCost",
                                contents: {
                                    acommodity: "$",
                                    aquantity: {decimalMantissa: -111735, decimalPlaces: 2, floatingPoint: -1117.35},
                                    astyle: usdStyleModern,
                                    acost: null,
                                    acostbasis: null,
                                },
                            },
                            acostbasis: null,
                        },
                    ],
                },
                {
                    paccount: "assets:broker:cash",
                    pstatus: "Unmarked",
                    pcomment: "",
                    ptags: [],
                    pdate: null,
                    pdate2: null,
                    pbalanceassertion: null,
                    ptype: "RegularPosting",
                    poriginal: null,
                    ptransaction_: "9",
                    pamount: [
                        {
                            acommodity: "$",
                            aquantity: {decimalMantissa: 111735, decimalPlaces: 2, floatingPoint: 1117.35},
                            astyle: usdStyleModern,
                            acost: null,
                            acostbasis: null,
                        },
                    ],
                },
            ],
        };
        const [txn] = normalizeTransactions([signedSell]);
        const aapl = txn.postings[0].amounts[0];
        expect(aapl.qty).toEqual({m: -45000n, p: 4}); // posting amount keeps its sign
        expect(aapl.cost).toEqual({commodity: "$", qty: {m: 111735n, p: 2}, per: false}); // cost magnitude comes out positive
        expect(Object.isFrozen(aapl.cost!.qty)).toBe(true);
    });
});

/** Deep-rewrite a modern (1.52/2.0) wire object into its pre-1.5x spelling per the plans/00 drift table. */
function toLegacyShape(value: unknown): unknown {
    if (Array.isArray(value)) return value.map(toLegacyShape);
    if (typeof value !== "object" || value === null) return value;
    const out: Record<string, unknown> = {};
    for (const [key, v] of Object.entries(value)) {
        if (key === "asrounding" || key === "acostbasis") continue; // did not exist pre-1.5x
        if (key === "asdecimalmark") out.asdecimalpoint = toLegacyShape(v);
        else if (key === "acost") out.aprice = toLegacyShape(v);
        else if (key === "tag" && v === "UnitCost") out.tag = "UnitPrice";
        else if (key === "tag" && v === "TotalCost") out.tag = "TotalPrice";
        else out[key] = toLegacyShape(v);
    }
    if ("aprice" in out) out.aismultiplier = false; // it's an Amount — legacy carried this flag
    return out;
}

// Regression net over the RAW committed API snapshot (WP-09) — the same bytes
// a live hledger-web 1.52 serves. Counts/statuses verified against
// `hledger -f fixtures/sample.journal stats` and the fixture journal itself.
describe("UNIT normalizeTransactions over the fixtures/api/v1.52 snapshot", () => {
    const raw: unknown = JSON.parse(readFileSync(new URL("../../../../fixtures/api/v1.52/transactions.json", import.meta.url), "utf8"));
    const txns = normalizeTransactions(raw);

    it("normalizes every transaction and posting", () => {
        // 185 → 189 and 420 → 429 when plans/14 added the home and car opening
        // positions and two depreciation entries to sample.journal.
        expect(txns).toHaveLength(189);
        expect(txns.reduce((n, t) => n + t.postings.length, 0)).toBe(429);
    });

    it("preserves the status distribution", () => {
        const counts = {cleared: 0, pending: 0, unmarked: 0};
        for (const txn of txns) counts[txn.status] += 1;
        // All four new entries are `*` cleared, so only that count moved.
        expect(counts).toEqual({cleared: 175, pending: 1, unmarked: 13});
    });

    it("carries exact Dec quantities (opening checking balance)", () => {
        const opening = txns[0];
        expect(opening.index).toBe(1);
        expect(opening.date).toBe("2024-07-01");
        expect(opening.description).toBe("Opening balances");
        const checking = opening.postings.find((p) => p.account === "assets:bank:checking");
        expect(checking?.amounts[0].qty).toEqual({m: 500000n, p: 2});
    });

    it("builds lowercase haystacks (the pending flight)", () => {
        // Located by description rather than by index: it is the journal's one
        // `!` transaction, and pinning the index here meant every later fixture
        // edit failed as `expected undefined` instead of naming what moved. The
        // index is still asserted, one line down.
        const flight = txns.find((t) => t.description.startsWith("Delta Airlines"));
        expect(flight?.index).toBe(188);
        expect(flight?.status).toBe("pending");
        expect(flight?.date).toBe("2026-07-02");
        expect(flight?.haystack).toContain("delta airlines");
        expect(flight?.haystack).toContain("expenses:travel:flights");
        expect(flight?.haystack).toContain("$412.80");
        expect(flight?.haystack).not.toMatch(/[A-Z]/);
    });

    it("freezes normalized snapshot objects", () => {
        expect(Object.isFrozen(txns[0])).toBe(true);
        expect(Object.isFrozen(txns[0].postings[0])).toBe(true);
        expect(Object.isFrozen(txns[0].postings[0].amounts[0])).toBe(true);
        expect(Object.isFrozen(txns[0].postings[0].amounts[0].qty)).toBe(true);
    });
});

describe("UNIT normalizePrices", () => {
    it("normalizes 1.52 MarketPrice records (mp* fields)", () => {
        const raw = [{mpdate: "2026-01-15", mpfrom: "EUR", mprate: {decimalMantissa: 108, decimalPlaces: 2, floatingPoint: 1.08}, mpto: "$"}];
        const [price] = normalizePrices(raw);
        expect(price.date).toBe("2026-01-15");
        expect(price.commodity).toBe("EUR");
        expect(price.price.commodity).toBe("$");
        expect(price.price.qty).toEqual({m: 108n, p: 2});
        expect(Object.isFrozen(price)).toBe(true);
    });

    it("normalizes full price-directive records (pd* fields)", () => {
        const raw = [
            {
                pddate: "2026-02-01",
                pdcommodity: "AAPL",
                pdamount: {
                    acommodity: "$",
                    aquantity: {decimalMantissa: 23000, decimalPlaces: 2, floatingPoint: 230},
                    astyle: usdStyleModern,
                    acost: null,
                },
            },
        ];
        const [price] = normalizePrices(raw);
        expect(price.commodity).toBe("AAPL");
        expect(price.price.qty).toEqual({m: 23000n, p: 2});
        expect(price.price.style.digitGroups).toEqual([",", [3]]);
    });

    it("throws ApiShapeError on unrecognized shapes", () => {
        expect(() => normalizePrices("nope")).toThrow(ApiShapeError);
        expect(() => normalizePrices([{bogus: true}])).toThrow(ApiShapeError);
    });
});

// ---------------------------------------------------------------------------
// Engine diagnostics (unbalanced / balance assertion). Advisory findings on a
// journal the engine DID open, so nothing here may throw — a junk entry costs
// that one finding, never the journal load.
// ---------------------------------------------------------------------------

/** Three transactions whose 1-based tindex deliberately differs from their 0-based position. */
const diagTxns = normalizeTransactions([
    {...modernTxn, tindex: 1},
    {...modernTxn, tindex: 2},
    {...modernTxn, tindex: 3},
]);

const unbalancedDiag = {
    txnIndex: 0,
    rule: "unbalanced",
    severity: "error",
    message: "This transaction is unbalanced. The real postings' sum should be 0 but is: $-1.00",
};
const assertionDiag = {
    txnIndex: 2,
    rule: "assertion",
    severity: "error",
    message: "balance assertion failed in assets:bank\n  expected: $10.00\n  actual:   $12.00",
};

/** Every malformed shape returns [] rather than throwing. */
const expectNoDiagnostics = (raw: unknown): void => {
    expect(normalizeDiagnostics(raw, diagTxns)).toEqual([]);
};

describe("UNIT normalizeDiagnostics", () => {
    it("decodes a well-formed array, translating the 0-based wire position to the txn's own index", () => {
        const decoded = normalizeDiagnostics({diagnostics: [unbalancedDiag, assertionDiag]}, diagTxns);
        expect(decoded).toEqual([
            {txnIndex: 1, rule: "unbalanced", severity: "error", message: unbalancedDiag.message},
            {txnIndex: 3, rule: "assertion", severity: "error", message: assertionDiag.message},
        ]);
    });

    it("preserves a multi-line message verbatim", () => {
        const [decoded] = normalizeDiagnostics({diagnostics: [assertionDiag]}, diagTxns);
        expect(decoded.message).toBe(assertionDiag.message);
        expect(decoded.message.split("\n")).toHaveLength(3);
    });

    it("treats an empty array as clean", () => {
        expectNoDiagnostics({diagnostics: []});
    });

    it("treats a missing field, null, or a non-array as no diagnostics", () => {
        expectNoDiagnostics({transactions: []}); // field absent — an older engine build
        expectNoDiagnostics({diagnostics: null});
        expectNoDiagnostics({diagnostics: undefined});
        expectNoDiagnostics({diagnostics: 42});
        expectNoDiagnostics({diagnostics: "unbalanced"});
        expectNoDiagnostics({diagnostics: {txnIndex: 0}});
        expectNoDiagnostics(null); // whole payload null
        expectNoDiagnostics(undefined);
        expectNoDiagnostics("nope");
        expectNoDiagnostics([]); // a bare (pre-diagnostics) transactions array
    });

    it("skips an entry with a bad severity", () => {
        expectNoDiagnostics({diagnostics: [{...unbalancedDiag, severity: "critical"}]});
        expectNoDiagnostics({diagnostics: [{...unbalancedDiag, severity: "ERROR"}]});
        expectNoDiagnostics({diagnostics: [{...unbalancedDiag, severity: 2}]});
        expectNoDiagnostics({diagnostics: [{...unbalancedDiag, severity: null}]});
        expectNoDiagnostics({diagnostics: [{...unbalancedDiag, severity: undefined}]});
    });

    it("skips an entry with a non-integer or out-of-range txnIndex", () => {
        expectNoDiagnostics({diagnostics: [{...unbalancedDiag, txnIndex: 1.5}]});
        expectNoDiagnostics({diagnostics: [{...unbalancedDiag, txnIndex: "0"}]});
        expectNoDiagnostics({diagnostics: [{...unbalancedDiag, txnIndex: -1}]});
        expectNoDiagnostics({diagnostics: [{...unbalancedDiag, txnIndex: NaN}]});
        expectNoDiagnostics({diagnostics: [{...unbalancedDiag, txnIndex: undefined}]});
        expectNoDiagnostics({diagnostics: [{...unbalancedDiag, txnIndex: null}]});
        // Past the end of the served array: unanchorable to any row, so dropped.
        expectNoDiagnostics({diagnostics: [{...unbalancedDiag, txnIndex: 3}]});
    });

    // An `account`-anchored finding: the `account-tag` rule, whose subject is an
    // `account` DIRECTIVE and so has no transaction to point at. It is the first
    // diagnostic to arrive without a `txnIndex`, and the decoder used to drop
    // exactly that shape.
    // The engine's real sentence, both halves (`journal_to_tag_diagnostics` in
    // wire.rs): the accepted codes, then what ignoring the tag cost. Kept whole
    // because the decoder passes the message through verbatim, and a truncated
    // fixture would let a decoder that mangled the tail still pass.
    const tagDiag = {
        account: "assets:property:house",
        rule: "account-tag",
        severity: "warning",
        message:
            "account 'assets:property:house' declares `holdings: real-estate`, which is not one of stocks, other, none; " +
            "the tag is ignored and the account is classified mechanically (does it hold a non-currency commodity?)",
    };

    it("keeps an account-anchored entry, with a null txnIndex", () => {
        expect(normalizeDiagnostics({diagnostics: [tagDiag]}, diagTxns)).toEqual([
            {txnIndex: null, account: "assets:property:house", rule: "account-tag", severity: "warning", message: tagDiag.message},
        ]);
    });

    it("drops an entry with NEITHER anchor", () => {
        // No txnIndex and no account: renderable, but nothing could say what it
        // was about — worse than losing the one finding.
        expectNoDiagnostics({diagnostics: [{rule: "account-tag", severity: "warning", message: "something is wrong somewhere"}]});
        expectNoDiagnostics({diagnostics: [{...tagDiag, account: ""}]});
        expectNoDiagnostics({diagnostics: [{...tagDiag, account: "   "}]});
        expectNoDiagnostics({diagnostics: [{...tagDiag, account: 42}]});
        expectNoDiagnostics({diagnostics: [{...tagDiag, account: null}]});
    });

    it("still refuses a junk txnIndex rather than demoting it to an account anchor", () => {
        // An entry carrying BOTH a bad txnIndex and a good account is a bug on
        // the engine side, not a finding to salvage: the anchors are exclusive,
        // so falling back would hide the contradiction.
        expectNoDiagnostics({diagnostics: [{...tagDiag, txnIndex: 99}]});
        expectNoDiagnostics({diagnostics: [{...tagDiag, txnIndex: 1.5}]});
        expectNoDiagnostics({diagnostics: [{...tagDiag, txnIndex: null}]});
    });

    it("decodes both anchor kinds side by side", () => {
        const decoded = normalizeDiagnostics({diagnostics: [unbalancedDiag, tagDiag, assertionDiag]}, diagTxns);
        expect(decoded.map((p) => p.txnIndex)).toEqual([1, null, 3]);
        expect(decoded.map((p) => p.account)).toEqual([undefined, "assets:property:house", undefined]);
    });

    it("skips an entry with a missing or empty message", () => {
        expectNoDiagnostics({diagnostics: [{txnIndex: 0, rule: "unbalanced", severity: "error"}]});
        expectNoDiagnostics({diagnostics: [{...unbalancedDiag, message: ""}]});
        expectNoDiagnostics({diagnostics: [{...unbalancedDiag, message: "   "}]});
        expectNoDiagnostics({diagnostics: [{...unbalancedDiag, message: 7}]});
    });

    it("skips an entry with an unknown or missing rule, and non-object entries", () => {
        expectNoDiagnostics({diagnostics: [{...unbalancedDiag, rule: "made-up"}]});
        expectNoDiagnostics({diagnostics: [{...unbalancedDiag, rule: undefined}]});
        expectNoDiagnostics({diagnostics: [null, undefined, 3, "x", []]});
    });

    it("keeps the good entries when a bad one sits between them", () => {
        const decoded = normalizeDiagnostics({diagnostics: [unbalancedDiag, {rule: "assertion"}, null, assertionDiag]}, diagTxns);
        expect(decoded.map((p) => p.txnIndex)).toEqual([1, 3]);
    });

    it("collapses exact duplicates (the drawer keys its list by txnIndex + message)", () => {
        const decoded = normalizeDiagnostics({diagnostics: [unbalancedDiag, {...unbalancedDiag}, assertionDiag]}, diagTxns);
        expect(decoded).toHaveLength(2);
    });

    it("accepts a bare diagnostics array as well as the payload envelope", () => {
        expect(normalizeDiagnostics([unbalancedDiag], diagTxns)).toHaveLength(1);
    });

    it("returns [] for any diagnostics when no transactions were served", () => {
        expect(normalizeDiagnostics({diagnostics: [unbalancedDiag]}, [])).toEqual([]);
    });
});

// DL-2: the write path can only preserve a posting's type and balance assertion
// if the READ path decodes them. `/transactions` has always served both; nothing
// decoded them, so the edit popup could not round-trip what it never saw.
describe("UNIT normalizeTransactions — posting type and balance assertion", () => {
    /** A posting in the hledger-web wire shape, with `ptype`/`pbalanceassertion` overridable. */
    function rawPosting(account: string, mantissa: number, extra: Record<string, unknown> = {}): unknown {
        return {
            paccount: account,
            pstatus: "Unmarked",
            pcomment: "",
            ptags: [],
            pdate: null,
            pdate2: null,
            pbalanceassertion: null,
            ptype: "RegularPosting",
            pamount: [
                {
                    acommodity: "$",
                    aquantity: {decimalMantissa: mantissa, decimalPlaces: 2, floatingPoint: mantissa / 100},
                    astyle: usdStyleModern,
                    acost: null,
                },
            ],
            ...extra,
        };
    }

    const assertionWire = {
        baamount: {acommodity: "$", aquantity: {decimalMantissa: 9900, decimalPlaces: 2, floatingPoint: 99}, astyle: usdStyleModern, acost: null},
        bainclusive: false,
        batotal: false,
        baposition: {sourceColumn: 1, sourceLine: 3, sourceName: "x.journal"},
    };

    const wireTxn = {
        tindex: 2,
        tdate: "2026-01-01",
        tdate2: null,
        tstatus: "Unmarked",
        tdescription: "A",
        tcode: "",
        tcomment: "",
        ttags: [],
        tpostings: [
            rawPosting("expenses:a", 100),
            rawPosting("assets:cash", -100, {pbalanceassertion: assertionWire}),
            rawPosting("budget:env", 100, {ptype: "BalancedVirtualPosting"}),
            rawPosting("tracking:note", 700, {ptype: "VirtualPosting"}),
        ],
    };

    it("decodes ptype, leaving an ordinary posting's type absent", () => {
        const [txn] = normalizeTransactions([wireTxn]);
        expect(txn.postings[0].type).toBeUndefined(); // RegularPosting → absent
        expect(txn.postings[2].type).toBe("balancedVirtual");
        expect(txn.postings[3].type).toBe("virtual");
    });

    it("decodes a balance assertion, and leaves it absent when there is none", () => {
        const [txn] = normalizeTransactions([wireTxn]);
        expect(txn.postings[0].balanceAssertion).toBeUndefined();
        expect(txn.postings[1].balanceAssertion).toEqual({
            amount: {commodity: "$", qty: {m: 9900n, p: 2}, style: {side: "L", spaced: false, precision: 2, decimalPoint: ".", digitGroups: [",", [3]]}},
            inclusive: false,
            total: false,
        });
    });

    it("reads the == (total) and =* (inclusive) flags", () => {
        const totalInclusive = {
            ...wireTxn,
            tpostings: [rawPosting("assets:cash", -100, {pbalanceassertion: {...assertionWire, batotal: true, bainclusive: true}})],
        };
        const [txn] = normalizeTransactions([totalInclusive]);
        expect(txn.postings[0].balanceAssertion?.total).toBe(true);
        expect(txn.postings[0].balanceAssertion?.inclusive).toBe(true);
    });

    it("tolerates an unknown ptype and an assertion record with no amount", () => {
        const junk = {
            ...wireTxn,
            tpostings: [rawPosting("a:b", 100, {ptype: "SomeFuturePosting", pbalanceassertion: {bainclusive: true}})],
        };
        const [txn] = normalizeTransactions([junk]);
        // An unrecognized type reads as regular (what an unbracketed account means)…
        expect(txn.postings[0].type).toBeUndefined();
        // …and an unusable assertion is dropped rather than sinking the load.
        expect(txn.postings[0].balanceAssertion).toBeUndefined();
    });
});

describe("UNIT normalizeTransactions journal payload envelope", () => {
    it("accepts a {transactions, diagnostics} envelope as well as a bare array", () => {
        const enveloped = normalizeTransactions({transactions: [modernTxn], diagnostics: [unbalancedDiag]});
        expect(enveloped).toHaveLength(1);
        expect(enveloped[0].index).toBe(2);
    });

    it("still throws ApiShapeError when the payload is neither", () => {
        expect(() => normalizeTransactions({nope: true})).toThrow(ApiShapeError);
        expect(() => normalizeTransactions("nope")).toThrow(ApiShapeError);
    });
});

describe("UNIT normalizeAccounts", () => {
    it("extracts the declared `type:` tag; missing/absent declaration → null", () => {
        const raw = [
            {aname: "assets:bank:checking", adeclarationinfo: {adicomment: "type: C\n", aditags: [["type", "C"]]}},
            {aname: "assets:wallet", adeclarationinfo: {aditags: [["type", "Cash"]]}},
            {aname: "expenses:food", adeclarationinfo: {aditags: []}}, // declared without a type
            {aname: "assets:broker", adeclarationinfo: null}, // never declared (tree-only)
        ];
        expect(normalizeAccounts(raw)).toEqual([
            {name: "assets:bank:checking", type: "cash"},
            {name: "assets:wallet", type: "cash"},
            {name: "expenses:food", type: null},
            {name: "assets:broker", type: null},
        ]);
    });

    it("skips the empty root account and unknown type letters", () => {
        const raw = [
            {aname: "", adeclarationinfo: null},
            {aname: "assets", adeclarationinfo: {aditags: [["type", "Z"]]}}, // unrecognized → null, not dropped
        ];
        expect(normalizeAccounts(raw)).toEqual([{name: "assets", type: null}]);
    });

    it("throws ApiShapeError when the payload is not an array", () => {
        expect(() => normalizeAccounts({nope: true})).toThrow(ApiShapeError);
    });

    it("counts and reports malformed entries instead of dropping them in silence (FE-5g)", () => {
        // A dropped declaration is not a small loss: reports classify accounts
        // by their DECLARED type, so losing one re-buckets a whole subtree and
        // makes its totals read zero — a wrong answer that looks like a right
        // one. Skipping it quietly left nothing to notice.
        const warn = vi.spyOn(console, "warn").mockImplementation(() => undefined);
        try {
            const raw = [{aname: "assets", adeclarationinfo: {aditags: [["type", "A"]]}}, {aname: 42}, {nothing: true}];
            expect(normalizeAccounts(raw)).toEqual([{name: "assets", type: "asset"}]);
            expect(lastSkippedAccountCount()).toBe(2);
            expect(warn).toHaveBeenCalledTimes(1);
            expect(String(warn.mock.calls[0][0])).toMatch(/skipped 2 malformed entries/);
        } finally {
            warn.mockRestore();
        }
    });

    it("stays silent about the empty root account, which every healthy payload has", () => {
        const warn = vi.spyOn(console, "warn").mockImplementation(() => undefined);
        try {
            expect(normalizeAccounts([{aname: "", adeclarationinfo: null}, {aname: "assets"}])).toEqual([{name: "assets", type: null}]);
            expect(lastSkippedAccountCount()).toBe(0);
            expect(warn).not.toHaveBeenCalled();
        } finally {
            warn.mockRestore();
        }
    });
});
