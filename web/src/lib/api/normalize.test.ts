import {readFileSync} from "node:fs";
import {describe, expect, it} from "vitest";
import {ApiShapeError} from "./client";
import {normalizePrices, normalizeTransactions} from "./normalize";

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
        expect(txns).toHaveLength(185);
        expect(txns.reduce((n, t) => n + t.postings.length, 0)).toBe(420);
    });

    it("preserves the status distribution", () => {
        const counts = {cleared: 0, pending: 0, unmarked: 0};
        for (const txn of txns) counts[txn.status] += 1;
        expect(counts).toEqual({cleared: 171, pending: 1, unmarked: 13});
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
        const flight = txns.find((t) => t.index === 184);
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
