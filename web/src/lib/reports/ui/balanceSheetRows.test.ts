// The grouped balance sheet's display model: what is on screen at a given
// collapse state, and how one MixedAmount becomes one figure plus footnotes.
//
// These are the two claims the view rests on and cannot make about itself:
// a collapsed group contributes exactly one row (so `j` has somewhere to go on
// first load, and the cursor never points at an invisible account), and an
// unpriced commodity is demoted rather than dropped.

import {describe, expect, it} from "vitest";
import {decodeBalanceSheetReport} from "$lib/api/nativeDecode";
import {dec, type MixedAmount} from "$lib/domain/money";
import type {AmountStyle} from "$lib/domain/types";
import type {BsSection} from "$lib/reports/types";
import {GROUPED_BALANCE_SHEET, UNBALANCED_BALANCE_SHEET} from "$lib/testing/balanceSheetFixture";
import {amountCell, bsGroupKey, bsSummary, sectionDisplayRows} from "./balanceSheetRows";

const REPORT = decodeBalanceSheetReport(GROUPED_BALANCE_SHEET);
const [ASSETS, , EQUITY] = REPORT.sections;

/** Journal-derived display styles, as `reportStyles` would supply them. */
const STYLES: ReadonlyMap<string, AmountStyle> = new Map<string, AmountStyle>([
    ["$", {side: "L", spaced: false, precision: 2, decimalPoint: ".", digitGroups: [",", [3]]}],
    ["GLD", {side: "R", spaced: true, precision: 1, decimalPoint: ".", digitGroups: null}],
    ["TSLA", {side: "R", spaced: true, precision: 1, decimalPoint: ".", digitGroups: null}],
    ["EUR", {side: "R", spaced: true, precision: 2, decimalPoint: ",", digitGroups: [".", [3]]}],
]);

const NONE = (): boolean => false;
const ALL = (): boolean => true;
const only =
    (...keys: string[]) =>
    (key: string): boolean =>
        keys.includes(key);

describe("UNIT reports/ui/balanceSheetRows — sectionDisplayRows", () => {
    it("shows one row per group when everything is collapsed", () => {
        const rows = sectionDisplayRows(ASSETS, NONE);

        expect(rows.map((r) => r.label)).toEqual(["Cash and cash equivalents", "Investments"]);
        expect(rows.every((r) => r.kind === "group")).toBe(true);
        // Collapsed is the DEFAULT, and a collapsed group still has to be
        // cursorable — otherwise `j` on a freshly-loaded report does nothing at
        // all, because no account row exists yet.
        expect(rows).toHaveLength(2);
    });

    it("carries each group's own subtotal, not a row's balance", () => {
        const [cash] = sectionDisplayRows(ASSETS, NONE);
        expect(cash.amount.get("$")).toEqual({m: 4245024n, p: 2});
        expect(cash.account).toBeNull(); // a group heading is not an account
    });

    it("expands one group without expanding its neighbours", () => {
        const rows = sectionDisplayRows(ASSETS, only(bsGroupKey("assets", "Cash and cash equivalents")));

        // Labels are relative to the displayed PARENT, as in every other report
        // table: the group's own root keeps its full path, its children are named
        // by the segments below it. `wise:eur` is one row, not two: the engine
        // is asked for an unclamped report now, and `assets:bank:wise` has no
        // postings of its own, so `compressSectionRows` folds the chain.
        expect(rows.map((r) => r.label)).toEqual(["Cash and cash equivalents", "assets:bank", "checking", "savings", "wise:eur", "Investments"]);
        expect(rows[0].expanded).toBe(true);
        expect(rows[5].expanded).toBe(false);
        expect(rows[5].kind).toBe("group"); // the neighbour is still just its heading
    });

    it("compresses single-child chains exactly as every other report table does", () => {
        // `assets:broker` has one child and nothing of its own, so the pair reads
        // as one row rather than two lines carrying the identical balance.
        const rows = sectionDisplayRows(ASSETS, only(bsGroupKey("assets", "Investments")));
        const accounts = rows.filter((r) => r.kind === "account");

        expect(accounts.map((r) => r.account)).toEqual(["assets:broker:taxable"]);
        expect(accounts[0].label).toBe("assets:broker:taxable");
    });

    it("indents account rows beneath their group", () => {
        const rows = sectionDisplayRows(ASSETS, ALL);

        expect(rows.filter((r) => r.kind === "group").every((r) => r.indent === 0)).toBe(true);
        expect(rows.filter((r) => r.kind === "account").every((r) => r.indent >= 1)).toBe(true);
    });

    it("gives every row a key unique across the whole report", () => {
        const keys = REPORT.sections.flatMap((section) => sectionDisplayRows(section, ALL)).map((r) => r.key);
        expect(new Set(keys).size).toBe(keys.length);
    });

    it("marks a computed group unexpandable so it renders without a dead triangle", () => {
        // "Retained earnings" summarizes accounts that are not on the balance
        // sheet at all: a total and no rows. A disclosure that opens onto
        // nothing is worse than no disclosure.
        const rows = sectionDisplayRows(EQUITY, ALL);
        const retained = rows.find((r) => r.label === "Retained earnings");

        expect(retained?.expandable).toBe(false);
        expect(retained?.expanded).toBe(false);
        // …and asking for it to be open adds no rows at all, so a stray key in
        // the collapse set cannot produce a group that claims to be expanded.
        const asked = sectionDisplayRows(EQUITY, only(bsGroupKey("equity", "Retained earnings")));
        expect(asked).toHaveLength(sectionDisplayRows(EQUITY, NONE).length);
        expect(asked.every((r) => r.expanded === false)).toBe(true);
    });

    it("renders a section with no groups as an empty list, not a throw", () => {
        const empty: BsSection = {kind: "liabilities", title: "Liabilities", groups: [], total: new Map()};
        expect(sectionDisplayRows(empty, ALL)).toEqual([]);
    });
});

describe("UNIT reports/ui/balanceSheetRows — amountCell", () => {
    const usd = (m: number, p: number): MixedAmount => new Map([["$", dec(m, p)]]);

    it("promotes the base commodity to the one figure on the line", () => {
        expect(amountCell(usd(4245024, 2), "$", STYLES)).toEqual({text: "$42,450.24", negative: false, extras: []});
    });

    it("rounds half away from zero, matching every other money surface", () => {
        // $59,612.615 — the engine's exact assets total. hledger's CLI prints
        // .61 here because Haskell's `round` is half-to-EVEN; `formatDec` is
        // half-away-from-zero everywhere in this app, so .62 is the number the
        // screen and the workbook both show.
        expect(amountCell(usd(59612615, 3), "$", STYLES).text).toBe("$59,612.62");
    });

    it("demotes what the valuation could not convert to a secondary line", () => {
        const cell = amountCell(REPORT.sections[0].total, "$", STYLES);

        expect(cell.text).toBe("$59,612.62");
        // Sorted, so the footnote never depends on Map insertion order.
        expect(cell.extras).toEqual(["5.0 GLD", "-2.0 TSLA"]);
    });

    it("flags a negative base figure for the caller to paint", () => {
        expect(amountCell(usd(-53115, 2), "$", STYLES)).toMatchObject({text: "$-531.15", negative: true});
    });

    it("shows a real formatted zero when the amount has no base part", () => {
        // "Transfers" is 5 GLD and no dollars. A blank cell would read as "no
        // data"; `$0.00` with the GLD footnote reads as what it is.
        const transfers = REPORT.sections[2].groups.find((g) => g.name === "Transfers");
        expect(amountCell(transfers?.total ?? new Map(), "$", STYLES)).toEqual({text: "$0.00", negative: false, extras: ["5.0 GLD"]});
    });

    it("drops zero commodities from the secondary line", () => {
        const withZero: MixedAmount = new Map([
            ["$", dec(100, 2)],
            ["GLD", dec(0, 0)],
        ]);
        expect(amountCell(withZero, "$", STYLES).extras).toEqual([]);
    });

    describe("a journal with no base commodity", () => {
        // `base` is `Option<Commodity>` on the wire and arrives null. There is
        // then nothing to promote, so the first commodity leads and the rest
        // stay footnotes — deterministic, and honest that nothing was converted.
        it("leads with the first commodity in sort order", () => {
            const mixed: MixedAmount = new Map([
                ["GLD", dec(5, 0)],
                ["EUR", dec(56675, 2)],
            ]);
            expect(amountCell(mixed, null, STYLES)).toEqual({text: "566,75 EUR", negative: false, extras: ["5.0 GLD"]});
        });

        it("renders a bare 0 for an empty amount rather than a currency it does not have", () => {
            expect(amountCell(new Map(), null, STYLES)).toEqual({text: "0", negative: false, extras: []});
        });
    });
});

describe("UNIT reports/ui/balanceSheetRows — bsSummary", () => {
    it("adds liabilities and equity on the exact Decs, per commodity", () => {
        const summary = bsSummary(REPORT);

        // $531.15 (places 2) + $59,081.465 (places 3) = $59,612.615, carried at
        // full precision — the half-cent survives the addition. Adding the
        // DISPLAYED $531.15 and $59,081.47 instead gives $59,612.62, which is
        // right here by luck and wrong the moment either side rounds the other
        // way.
        expect(summary.liabilitiesPlusEquity.get("$")).toEqual({m: 59612615n, p: 3});
        // The unpriced holdings tie out too, and must not vanish from the line.
        expect(summary.liabilitiesPlusEquity.get("GLD")).toEqual({m: 5n, p: 0});
        expect(summary.liabilitiesPlusEquity.get("TSLA")).toEqual({m: -2n, p: 0});
        expect(summary.liabilitiesPlusEquity).toEqual(summary.assets);
    });

    it("takes the verdict from the engine, not from the tie-out it displays", () => {
        expect(bsSummary(REPORT).balanced).toBe(true);

        // Same sections, so `liabilitiesPlusEquity` still equals `assets` to the
        // last decimal — the imbalance is only in `check`. Deriving the verdict
        // by comparing the two displayed figures would report this as balanced.
        const unbalanced = bsSummary(decodeBalanceSheetReport(UNBALANCED_BALANCE_SHEET));
        expect(unbalanced.liabilitiesPlusEquity).toEqual(unbalanced.assets);
        expect(unbalanced.balanced).toBe(false);
    });

    it("does not re-derive the verdict from `check`, which is dust on a real journal", () => {
        // A journal holding fractional lots leaves sub-cent residue in `check`
        // with nothing wrong: `26.2690 VTI @ $289.7713` costs $7,612.00227970
        // and no cash posting can carry the surplus digits. hledger accepts such
        // a journal; so does the engine, which is why it sends `balanced: true`
        // beside a non-empty `check`. `maIsZero(check)` here is what made a
        // valid journal warn "should be zero, but it is $0.00227970".
        const dusty = decodeBalanceSheetReport({...(GROUPED_BALANCE_SHEET as object), check: {$: {mantissa: "22797", places: 7}}, balanced: true});
        expect(dusty.check.size).toBe(1);
        expect(bsSummary(dusty).balanced).toBe(true);
    });

    it("finds each section by kind, so a reordered report cannot mislabel a figure", () => {
        const reversed = {...REPORT, sections: [...REPORT.sections].reverse()};

        expect(bsSummary(reversed)).toEqual(bsSummary(REPORT));
    });

    it("treats a missing section as zero rather than reading the wrong one", () => {
        const noEquity = {...REPORT, sections: REPORT.sections.filter((s) => s.kind !== "equity")};
        const summary = bsSummary(noEquity);

        expect(summary.equity).toEqual(new Map());
        expect(summary.liabilitiesPlusEquity).toEqual(summary.liabilities);
    });
});
