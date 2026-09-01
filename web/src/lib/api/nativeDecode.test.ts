import {readFileSync} from "node:fs";
import {describe, expect, it} from "vitest";
import {CLASSIFIED_BALANCE_SHEET, GROUPED_BALANCE_SHEET, UNBALANCED_BALANCE_SHEET} from "$lib/testing/balanceSheetFixture";
import {GROUPED_INCOME_STATEMENT, MULTI_STEP_INCOME_STATEMENT, UNCOMPARED_INCOME_STATEMENT} from "$lib/testing/incomeStatementFixture";
import {ApiShapeError} from "./client";
import {
    decodeAccountReference,
    decodeBalanceSheetReport,
    decodeBudgetListing,
    decodeBudgetReport,
    decodeFlowReport,
    decodeHoldingsReport,
    decodeHoldingsSeries,
    decodeIncomeStatementReport,
    decodeInsightsReport,
    decodeJournalInfo,
    decodeOtherHoldingsReport,
    decodePeriodReport,
    decodeRulesDoc,
    decodeRulesIndex,
    decodeRulesPreview,
    decodeSectionedReport,
    decodeSubscriptionsReport,
} from "./nativeDecode";

// The native wire has no codegen: these decoders mirror 28 `Wire*` structs in
// crates/ledgeline-server/src/reports_api.rs BY HAND (CLEANUP.md DRY-3).
//
// So the primary samples here are not literals — they are the SAME committed
// response bodies the Rust side asserts against, `fixtures/native/v1/*.json`,
// produced by `just snapshot-native` and byte-checked by
// crates/ledgeline-server/tests/native_wire_golden.rs. Renaming a Rust field
// therefore breaks the Rust golden AND these decode assertions at once, instead
// of quietly rendering $0.00 in the SPA.
//
// Hand-written literals are kept ONLY for the cases fixtures/sample.journal
// cannot produce (a mantissa past Number.MAX_SAFE_INTEGER, a malformed mantissa,
// an unknown enum member, net worth's `meta.unpriced`, refused holdings totals,
// a budget goal of `{}`). Each says which.

const dec = (mantissa: number, places: number) => ({mantissa: String(mantissa), places});

/** A copy of `obj` with one key removed — for asserting a decoder NOTICES an absent field rather than defaulting it. */
function without<T extends object>(obj: T, key: keyof T): Partial<T> {
    const copy: Partial<T> = {...obj};
    delete copy[key];
    return copy;
}

function golden(name: string): unknown {
    return JSON.parse(readFileSync(new URL(`../../../../fixtures/native/v1/${name}.json`, import.meta.url), "utf8"));
}

/**
 * The import-rules goldens, which live in their OWN directory.
 *
 * They cannot join `fixtures/native/v1/`: that manifest is replayed against
 * `fixtures/sample.journal` and guarded by a rule that every pinned URI must fix
 * its own dates, and a rules response has no dates in it at all. These are
 * served over `fixtures/rules/tree/main.journal` and byte-checked by
 * `crates/ledgeline-server/tests/rules_endpoints.rs`, so a renamed Rust field
 * fails there and here at once — the same DRY-3 guard, one directory over.
 */
function rulesGolden(name: string): unknown {
    return JSON.parse(readFileSync(new URL(`../../../../fixtures/rules/golden/${name}.json`, import.meta.url), "utf8"));
}

describe("UNIT nativeDecode — SectionedReport over the balancesheet golden", () => {
    const raw = golden("balancesheet");

    it("decodes sections, rows, and exact Dec/MixedAmount, preserving asOf", () => {
        const report = decodeSectionedReport(raw);
        expect(report.asOf).toBe("2026-07-08");
        expect(report.from).toBeUndefined();
        expect(report.sections.map((s) => s.title)).toEqual(["Assets", "Liabilities"]);

        const assets = report.sections[0];
        expect(assets.rows[0]).toMatchObject({account: "assets", depth: 1});
        // {mantissa, places} → Dec {m: bigint, p: number}
        expect(assets.rows[0].inclusive.get("$")).toEqual({m: 6890256n, p: 2});
        expect(assets.rows[0].inclusive.get("AAPL")).toEqual({m: 195n, p: 1});
        // places 0 stays places 0, and a negative mantissa survives
        expect(assets.rows[0].inclusive.get("TSLA")).toEqual({m: -2n, p: 0});
        // This report is UNVALUED, so the home stays the unit it is written as —
        // which is exactly why the Other tab shows "1 HOME" beside a dollar value.
        expect(assets.rows[0].inclusive.get("HOME")).toEqual({m: 1n, p: 0});
        // a parent account's OWN amount is an empty object → an empty Map
        expect(assets.rows[0].own.size).toBe(0);
        expect(assets.rows[1]).toMatchObject({account: "assets:bank", depth: 2});
        expect(assets.total.get("$")).toEqual({m: 6890256n, p: 2});

        // $531.15 of credit card, plus the $336,000 mortgage that funds the home.
        expect(report.sections[1].total.get("$")).toEqual({m: 33653115n, p: 2});
        expect(report.grandTotal.get("$")).toEqual({m: -26762859n, p: 2});
    });

    it("carries from/to on a range report (the incomestatement golden)", () => {
        const report = decodeSectionedReport(golden("incomestatement"));
        expect(report.from).toBe("2026-01-01");
        expect(report.to).toBe("2026-07-08");
        expect(report.asOf).toBeUndefined();

        const revenues = report.sections[0];
        expect(revenues.title).toBe("Revenues");
        expect(revenues.total.get("$")).toEqual({m: 3401000n, p: 2});
        // a LEAF row carries a non-empty `own` — the field the balance sheet golden
        // only ever shows empty.
        expect(revenues.rows[2]).toMatchObject({account: "income:salary", depth: 2});
        expect(revenues.rows[2].own.get("$")).toEqual({m: 3396000n, p: 2});
        expect(report.grandTotal.get("EUR")).toEqual({m: -22875n, p: 2});
    });

    it("throws ApiShapeError when sections is missing", () => {
        expect(() => decodeSectionedReport({grandTotal: {}})).toThrow(ApiShapeError);
    });

    // Literal: no realistic journal produces this, but a computed marketValue can.
    it("decodes a mantissa far beyond the JS safe-integer range (the marketValue overflow fix)", () => {
        // 9625405560255625000 > Number.MAX_SAFE_INTEGER; string-encoded on the wire
        // so it round-trips exactly via BigInt (a JSON number would have lost it).
        const big = {
            sections: [{title: "X", rows: [{account: "a", depth: 1, own: {}, inclusive: {$: {mantissa: "9625405560255625000", places: 15}}}], total: {}}],
            grandTotal: {},
        };
        const report = decodeSectionedReport(big);
        expect(report.sections[0].rows[0].inclusive.get("$")).toEqual({m: 9625405560255625000n, p: 15});
    });

    it("rejects a non-integer mantissa string", () => {
        const bad = {sections: [], grandTotal: {$: {mantissa: "1.5", places: 2}}};
        expect(() => decodeSectionedReport(bad)).toThrow(/not an integer/);
    });

    it("rejects a negative places value", () => {
        const bad = {sections: [], grandTotal: {$: {mantissa: "100", places: -1}}};
        expect(() => decodeSectionedReport(bad)).toThrow(/invalid places/);
    });
});

// The grouped balance sheet (plans/12). Sourced from a hand-written literal
// rather than `fixtures/native/v1/`, because the endpoint it mirrors is being
// built in parallel — `balanceSheetFixture.ts` says what has to happen to this
// suite the moment `just snapshot-native` can produce the real bytes.
describe("UNIT nativeDecode — BalanceSheetReport (grouped)", () => {
    const report = decodeBalanceSheetReport(GROUPED_BALANCE_SHEET);

    it("tags the report so it can be told apart from a SectionedReport", () => {
        // Both carry a `sections` array. `kind` is added by the decoder, never
        // sent — it is the whole reason the page can pick the right renderer
        // and the right workbook builder (FE-1, one shape further on).
        expect(report.kind).toBe("balanceSheet");
        expect((GROUPED_BALANCE_SHEET as Record<string, unknown>).kind).toBeUndefined();
    });

    it("decodes the three sections in order with their kinds and totals", () => {
        expect(report.sections.map((s) => s.kind)).toEqual(["assets", "liabilities", "equity"]);
        expect(report.sections.map((s) => s.title)).toEqual(["Assets", "Liabilities", "Equity"]);
        expect(report.asOf).toBe("2026-07-08");
        expect(report.base).toBe("$");
        expect(report.value).toBe("market");

        // Assets at market, full engine precision — $59,612.615, not a rounded $59,612.62.
        expect(report.sections[0].total.get("$")).toEqual({m: 59612615n, p: 3});
        expect(report.sections[0].total.get("GLD")).toEqual({m: 5n, p: 0});
        expect(report.sections[0].total.get("TSLA")).toEqual({m: -2n, p: 0});
        expect(report.sections[1].total.get("$")).toEqual({m: 53115n, p: 2});
    });

    it("decodes groups with their resolution source and member rows", () => {
        const [cash, investments] = report.sections[0].groups;
        expect([cash.name, cash.source]).toEqual(["Cash and cash equivalents", "type"]);
        expect(cash.total.get("$")).toEqual({m: 4245024n, p: 2});
        expect(cash.rows.map((r) => r.account)).toEqual([
            "assets:bank",
            "assets:bank:checking",
            "assets:bank:savings",
            "assets:bank:wise",
            "assets:bank:wise:eur",
        ]);
        expect(cash.rows[0].own.size).toBe(0); // a parent's own amount is `{}` → an empty Map
        expect(cash.rows[1].inclusive.get("$")).toEqual({m: 2829281n, p: 2});
        expect(investments.source).toBe("commodity");
    });

    // The current/non-current axis. `term` and `subsections` are both DEMANDED
    // even though their untagged values (null and []) are the common case: the
    // two fields describe an absence, so an absent field renders as the very
    // thing a present one would say, and nothing downstream could tell.
    describe("current / non-current bands", () => {
        it("reads an untagged journal as classified-by-nothing, explicitly", () => {
            expect(report.sections.map((s) => s.subsections)).toEqual([[], [], []]);
            expect(report.sections.flatMap((s) => s.groups).every((g) => g.term === null)).toBe(true);
        });

        it("decodes the bands with their term, prose and subtotal", () => {
            const classified = decodeBalanceSheetReport(CLASSIFIED_BALANCE_SHEET);
            const [assets, liabilities, equity] = classified.sections;

            expect(assets.subsections.map((s) => [s.term, s.heading, s.label])).toEqual([
                ["current", "Current", "Total current assets"],
                ["noncurrent", "Non-current", "Total non-current assets"],
            ]);
            expect(assets.subsections[0].total.get("$")).toEqual({m: 6250000n, p: 2});
            expect(assets.groups.map((g) => g.term)).toEqual(["current", "current", "noncurrent", "noncurrent"]);
            // The prose is the ENGINE's, per section — "assets" and "liabilities"
            // are not interchangeable in a subtotal label, and nothing on this
            // side may compose one.
            expect(liabilities.subsections.map((s) => s.label)).toEqual(["Total current liabilities", "Total non-current liabilities"]);
            // Equity is never split, so it arrives banded by nothing even here.
            expect(equity.subsections).toEqual([]);
            expect(equity.groups.every((g) => g.term === null)).toBe(true);
        });

        it("freezes the bands with the rest of the tree", () => {
            const classified = decodeBalanceSheetReport(CLASSIFIED_BALANCE_SHEET);
            expect(Object.isFrozen(classified.sections[0].subsections)).toBe(true);
            expect(Object.isFrozen(classified.sections[0].subsections[0])).toBe(true);
        });

        it("throws when a section omits `subsections` rather than reading it as unclassified", () => {
            const patch = {sections: [{kind: "assets", title: "X", groups: [], total: {}}]};
            expect(() => decodeBalanceSheetReport({...(GROUPED_BALANCE_SHEET as object), ...patch})).toThrow(ApiShapeError);
            expect(() => decodeBalanceSheetReport({...(GROUPED_BALANCE_SHEET as object), ...patch})).toThrow(/missing title\/groups\/subsections/);
        });

        it("throws when a group omits `term` — null is how 'unclassified' is said", () => {
            const patch = {sections: [{kind: "assets", title: "X", groups: [{name: "G", source: "segment", rows: [], total: {}}], subsections: [], total: {}}]};
            expect(() => decodeBalanceSheetReport({...(GROUPED_BALANCE_SHEET as object), ...patch})).toThrow(
                /term: expected one of current\/noncurrent or null/
            );
        });

        it.each([
            ["a group's term", {groups: [{name: "G", source: "segment", term: "shortish", rows: [], total: {}}], subsections: []}],
            ["a band's term", {groups: [], subsections: [{term: "later", heading: "Later", label: "Total later assets", total: {}}]}],
        ])("throws on an unknown %s rather than guessing one", (_what, section) => {
            const patch = {sections: [{kind: "assets", title: "X", total: {}, ...section}]};
            expect(() => decodeBalanceSheetReport({...(GROUPED_BALANCE_SHEET as object), ...patch})).toThrow(ApiShapeError);
        });

        it("throws when a band omits the prose it exists to carry", () => {
            const patch = {sections: [{kind: "assets", title: "X", groups: [], subsections: [{term: "current", heading: "Current", total: {}}], total: {}}]};
            expect(() => decodeBalanceSheetReport({...(GROUPED_BALANCE_SHEET as object), ...patch})).toThrow(/missing heading\/label/);
        });
    });

    it("keeps a computed group's empty row list as a fact, not as missing data", () => {
        const retained = report.sections[2].groups.find((g) => g.name === "Retained earnings");
        expect(retained?.source).toBe("computed");
        expect(retained?.rows).toEqual([]);
        expect(retained?.total.get("EUR")).toEqual({m: -93325n, p: 2});
    });

    it("decodes netWorth, an empty check, and the unpriced list", () => {
        expect(report.netWorth.get("$")).toEqual({m: 59081465n, p: 3});
        expect(report.check.size).toBe(0); // `{}` — the journal balances
        expect(report.balanced).toBe(true);
        expect(report.meta?.unpriced).toEqual(["GLD", "TSLA"]);
    });

    it("carries `balanced` through independently of `check`", () => {
        // Two different facts, and the client may not infer either from the
        // other: a journal holding fractional lots leaves unavoidable sub-cent
        // dust in `check` and is still balanced (the engine decides that from
        // the precisions the journal writes, which the client cannot see).
        const dusty = decodeBalanceSheetReport({...(GROUPED_BALANCE_SHEET as object), check: {$: {mantissa: "22797", places: 7}}, balanced: true});
        expect(dusty.check.get("$")).toEqual({m: 22797n, p: 7});
        expect(dusty.balanced).toBe(true);
        expect(decodeBalanceSheetReport(UNBALANCED_BALANCE_SHEET).balanced).toBe(false);
    });

    it("freezes the whole tree, as every other decoder does", () => {
        expect(Object.isFrozen(report)).toBe(true);
        expect(Object.isFrozen(report.sections)).toBe(true);
        expect(Object.isFrozen(report.sections[0])).toBe(true);
        expect(Object.isFrozen(report.sections[0].groups[0])).toBe(true);
        expect(Object.isFrozen(report.sections[0].groups[0].rows[0])).toBe(true);
    });

    it("decodes a non-zero check at sub-cent precision", () => {
        // Half a cent: invisible at the 2-decimal display cap, so this is exactly
        // the imbalance a check computed from rendered strings would miss.
        const unbalanced = decodeBalanceSheetReport(UNBALANCED_BALANCE_SHEET);
        expect(unbalanced.check.get("$")).toEqual({m: 5n, p: 3});
    });

    it("keeps a null base as null rather than inventing a commodity", () => {
        const noBase = decodeBalanceSheetReport({...(GROUPED_BALANCE_SHEET as object), base: null});
        expect(noBase.base).toBeNull();
    });

    describe("refuses a body it cannot trust", () => {
        const mutate = (patch: Record<string, unknown>): unknown => ({...(GROUPED_BALANCE_SHEET as object), ...patch});
        const without = (key: string): unknown => {
            const copy = {...(GROUPED_BALANCE_SHEET as Record<string, unknown>)};
            delete copy[key];
            return copy;
        };

        // `base` is `Option<Commodity>` with no `skip_serializing_if`, so the
        // key is ALWAYS on the wire and `null` is the real "no base commodity"
        // answer. An absent key must throw rather than collapse into it —
        // under version skew a renamed `base` would otherwise silently render
        // the no-base presentation, exactly the loss `term`/`check`/`balanced`
        // all throw on (see decodeNullableStr).
        it("throws when `base` is absent — null is how 'no base commodity' is said", () => {
            expect(() => decodeBalanceSheetReport(without("base"))).toThrow(ApiShapeError);
            expect(() => decodeBalanceSheetReport(without("base"))).toThrow(/base: expected a string or null, got nothing/);
        });

        it("throws when `base` is present but is not a string or null", () => {
            expect(() => decodeBalanceSheetReport(mutate({base: 7}))).toThrow(/base: expected a string or null/);
        });

        // A MISSING check must never read as "balanced" — that is the single
        // wrong answer this field is capable of giving (DRY-3).
        it("throws when `check` is absent instead of defaulting to balanced", () => {
            expect(() => decodeBalanceSheetReport(without("check"))).toThrow(ApiShapeError);
            expect(() => decodeBalanceSheetReport(without("check"))).toThrow(/check: missing amount/);
        });

        it("throws when `netWorth` is absent", () => {
            expect(() => decodeBalanceSheetReport(without("netWorth"))).toThrow(/netWorth: missing amount/);
        });

        // Same reasoning as `check`, one step further along: `balanced` is the
        // field every consumer renders its verdict from, so a missing one may
        // not silently become `true`.
        it("throws when `balanced` is absent or is not a boolean", () => {
            expect(() => decodeBalanceSheetReport(without("balanced"))).toThrow(ApiShapeError);
            expect(() => decodeBalanceSheetReport(without("balanced"))).toThrow(/balanced: expected a boolean/);
            expect(() => decodeBalanceSheetReport(mutate({balanced: "yes"}))).toThrow(/balanced: expected a boolean/);
        });

        it.each([
            ["section kind", {sections: [{kind: "profits", title: "X", groups: [], subsections: [], total: {}}]}],
            [
                "group source",
                {sections: [{kind: "assets", title: "X", groups: [{name: "G", source: "vibes", term: null, rows: [], total: {}}], subsections: [], total: {}}]},
            ],
            ["valuation", {value: "guessed"}],
        ])("throws on an unknown %s rather than guessing one", (_what, patch) => {
            expect(() => decodeBalanceSheetReport(mutate(patch))).toThrow(ApiShapeError);
        });

        it("throws when a group omits `rows` — absent and empty are different facts", () => {
            const patch = {
                sections: [{kind: "assets", title: "X", groups: [{name: "G", source: "computed", term: null, total: {}}], subsections: [], total: {}}],
            };
            expect(() => decodeBalanceSheetReport(mutate(patch))).toThrow(/missing name\/rows/);
        });

        it("throws when asOf or sections are missing", () => {
            expect(() => decodeBalanceSheetReport({sections: []})).toThrow(/expected asOf and a sections array/);
            expect(() => decodeBalanceSheetReport({asOf: "2026-07-08"})).toThrow(/expected asOf and a sections array/);
        });
    });
});

// The grouped income statement (plans/13). Sourced from hand-written literals
// rather than `fixtures/native/v1/`, because the endpoint it mirrors is being
// built in parallel — `incomeStatementFixture.ts` says what has to happen to
// this suite the moment `just snapshot-native` can produce the real bytes.
describe("UNIT nativeDecode — IncomeStatementReport (grouped)", () => {
    const report = decodeIncomeStatementReport(GROUPED_INCOME_STATEMENT);

    it("tags the report so it can be told apart from the other two `sections` shapes", () => {
        // SectionedReport, BalanceSheetReport and this all carry a `sections`
        // array. `kind` is added by the decoder, never sent — it is the whole
        // reason the page can pick the right renderer and the right workbook
        // builder (FE-1, two shapes further on).
        expect(report.kind).toBe("incomeStatement");
        expect((GROUPED_INCOME_STATEMENT as Record<string, unknown>).kind).toBeUndefined();
    });

    it("decodes the sections in ladder order with their kinds, titles and totals", () => {
        expect(report.sections.map((s) => s.kind)).toEqual(["revenue", "opex"]);
        expect(report.sections.map((s) => s.title)).toEqual(["Revenue", "Expenses"]);
        expect([report.from, report.to]).toEqual(["2026-01-01", "2026-07-08"]);
        expect(report.base).toBe("$");
        expect(report.value).toBe("market");
        expect(report.multiStep).toBe(false);

        // hledger 1.52, `is -V -b 2026-01-01 -e 2026-07-09`.
        expect(report.sections[0].total.current.get("$")).toEqual({m: 3401000n, p: 2});
        expect(report.sections[1].total.current.get("$")).toEqual({m: 2862648n, p: 2});
        expect(report.netIncome.current.get("$")).toEqual({m: 538352n, p: 2});
    });

    it("decodes the prior window and every figure inside it", () => {
        // The immediately preceding window of EQUAL length: 2026-01-01 minus one
        // day, less the 188-day span.
        expect(report.prior).toEqual({from: "2025-06-26", to: "2025-12-31"});
        expect(report.sections[0].total.prior?.get("$")).toEqual({m: 3939750n, p: 2});
        expect(report.netIncome.prior?.get("$")).toEqual({m: 1088079n, p: 2});
    });

    it("decodes groups with their resolution source and member rows", () => {
        // Groups arrive sorted by name, so Dividends precedes Salary — which is
        // NOT the order `hledger is` prints them in.
        const [dividends, salary] = report.sections[0].groups;
        expect([salary.name, salary.source]).toEqual(["Salary", "segment"]);
        expect(salary.total.current.get("$")).toEqual({m: 3396000n, p: 2});
        expect(salary.rows.map((r) => r.account)).toEqual(["income:salary"]);
        expect(salary.rows[0].depth).toBe(2);
        expect(dividends.name).toBe("Dividends");
    });

    it("keeps a line that exists in only one period, with an EXPLICIT empty amount on the other side", () => {
        // `expenses:unknown` had no postings in the prior window. The engine joins
        // over the union of keys precisely so the group does not disappear, and
        // `{}` is how "nothing here" arrives — never a missing key, which would
        // be indistinguishable from a renamed field.
        const unknown = report.sections[1].groups.find((g) => g.name === "Unknown");
        expect(unknown?.total.current.get("$")).toEqual({m: 7500n, p: 2});
        expect(unknown?.total.prior).toEqual(new Map());
        expect(unknown?.total.prior).not.toBeUndefined();
    });

    it("decodes the ladder, attached to the sections it follows", () => {
        const multi = decodeIncomeStatementReport(MULTI_STEP_INCOME_STATEMENT);

        expect(multi.multiStep).toBe(true);
        expect(multi.sections.map((s) => s.kind)).toEqual(["revenue", "cogs", "opex", "depreciation", "other", "interest", "tax"]);
        expect(multi.sections.map((s) => s.trailing.map((t) => t.kind))).toEqual([
            [],
            ["grossProfit"],
            ["ebitda"],
            ["operatingIncome"],
            [],
            ["pretaxIncome"],
            [],
        ]);
        expect(multi.sections[1].trailing[0].label).toBe("Gross profit");
        expect(multi.sections[1].trailing[0].total.current.get("$")).toEqual({m: 51740000n, p: 2});
        // `other` is the one section allowed to be negative — it is a net
        // contribution to income, not a magnitude.
        expect(multi.sections[4].total.current.get("$")).toEqual({m: -1500000n, p: 2});
    });

    it("carries a `tag`-sourced group, which is how two unrelated accounts share a line", () => {
        const multi = decodeIncomeStatementReport(MULTI_STEP_INCOME_STATEMENT);
        const cloud = multi.sections[1].groups[0];

        expect([cloud.name, cloud.source]).toEqual(["Cloud infrastructure", "tag"]);
        expect(cloud.rows.map((r) => r.account)).toEqual(["cogs:cdn", "cogs:hosting"]);
    });

    it("omits `prior` entirely — never nulls it, never zeroes it — when not comparing", () => {
        const uncompared = decodeIncomeStatementReport(UNCOMPARED_INCOME_STATEMENT);

        expect(uncompared.prior).toBeNull();
        expect(uncompared.netIncome.prior).toBeUndefined();
        expect(uncompared.sections[0].total.prior).toBeUndefined();
        expect(uncompared.sections[0].groups[0].rows[0].amounts.prior).toBeUndefined();
        // A zero would be a claim about a period that was never computed.
        expect(uncompared.sections[0].total.current.get("$")).toEqual({m: 3401000n, p: 2});
    });

    it("freezes the whole tree, as every other decoder does", () => {
        expect(Object.isFrozen(report)).toBe(true);
        expect(Object.isFrozen(report.sections)).toBe(true);
        expect(Object.isFrozen(report.sections[0])).toBe(true);
        expect(Object.isFrozen(report.sections[0].groups[0])).toBe(true);
        expect(Object.isFrozen(report.sections[0].groups[0].rows[0])).toBe(true);
        expect(Object.isFrozen(report.sections[0].groups[0].rows[0].amounts)).toBe(true);
        expect(Object.isFrozen(report.prior)).toBe(true);
    });

    it("keeps a null base as null rather than inventing a commodity", () => {
        expect(decodeIncomeStatementReport({...(GROUPED_INCOME_STATEMENT as object), base: null}).base).toBeNull();
    });

    describe("refuses a body it cannot trust", () => {
        const mutate = (patch: Record<string, unknown>): unknown => ({...(GROUPED_INCOME_STATEMENT as object), ...patch});
        const without = (key: string): unknown => {
            const copy = {...(GROUPED_INCOME_STATEMENT as Record<string, unknown>)};
            delete copy[key];
            return copy;
        };
        /** The comparing fixture's first section, with one field patched. */
        const section = (patch: Record<string, unknown>): unknown =>
            mutate({sections: [{...((GROUPED_INCOME_STATEMENT as {sections: object[]}).sections[0] as object), ...patch}]});

        it("throws when a figure is absent instead of decoding it to $0.00", () => {
            expect(() => decodeIncomeStatementReport(without("netIncome"))).toThrow(ApiShapeError);
            expect(() => decodeIncomeStatementReport(without("netIncome"))).toThrow(/netIncome: missing amounts/);
            expect(() => decodeIncomeStatementReport(mutate({netIncome: {}}))).toThrow(/netIncome current: missing amount/);
        });

        // The cross-check that turns an optional key into a checked one. A
        // renamed `prior` field would otherwise just stop rendering the column,
        // which reads as "this report has no comparison" rather than as a bug.
        it("throws when the report says it compares but a figure carries no prior", () => {
            const noPrior = section({total: {current: {$: {mantissa: "3401000", places: 2}}}});
            expect(() => decodeIncomeStatementReport(noPrior)).toThrow(/prior: missing amount/);
        });

        it("throws when a figure carries a prior the report has no window for", () => {
            // There would be no dates to label the column with, so the figure
            // cannot be rendered and must not be silently held.
            const stray = {...(UNCOMPARED_INCOME_STATEMENT as object), netIncome: {current: {}, prior: {$: {mantissa: "1", places: 2}}}};
            expect(() => decodeIncomeStatementReport(stray)).toThrow(/no prior period/);
        });

        it("tolerates a null prior as absent, which is what serde emits for a bare Option", () => {
            const nulled = {...(UNCOMPARED_INCOME_STATEMENT as object), prior: null, netIncome: {current: {}, prior: null}};
            expect(decodeIncomeStatementReport(nulled).prior).toBeNull();
            expect(decodeIncomeStatementReport(nulled).netIncome.prior).toBeUndefined();
        });

        // Same strictness as the balance sheet's `base`, and for the same
        // reason: the Rust struct always sends the key, and null already says
        // "no base commodity" explicitly.
        it("throws when `base` is absent — null is how 'no base commodity' is said", () => {
            expect(() => decodeIncomeStatementReport(without("base"))).toThrow(ApiShapeError);
            expect(() => decodeIncomeStatementReport(without("base"))).toThrow(/base: expected a string or null, got nothing/);
            expect(() => decodeIncomeStatementReport(mutate({base: 7}))).toThrow(/base: expected a string or null/);
        });

        it("throws when `multiStep` is absent or is not a boolean", () => {
            // It decides whether `opex` reads "Expenses" or "Operating expenses",
            // so a missing one may not silently relabel a box.
            expect(() => decodeIncomeStatementReport(without("multiStep"))).toThrow(/multiStep: expected a boolean/);
            expect(() => decodeIncomeStatementReport(mutate({multiStep: "yes"}))).toThrow(/multiStep: expected a boolean/);
        });

        it.each([
            ["section kind", {sections: [{kind: "profits", title: "X", groups: [], total: {current: {}, prior: {}}, trailing: []}]}],
            [
                "group source",
                {
                    sections: [
                        {
                            kind: "revenue",
                            title: "X",
                            groups: [{name: "G", source: "vibes", rows: [], total: {current: {}, prior: {}}}],
                            total: {current: {}, prior: {}},
                            trailing: [],
                        },
                    ],
                },
            ],
            [
                "subtotal kind",
                {
                    sections: [
                        {
                            kind: "revenue",
                            title: "X",
                            groups: [],
                            total: {current: {}, prior: {}},
                            trailing: [{kind: "vibes", label: "L", total: {current: {}, prior: {}}}],
                        },
                    ],
                },
            ],
            ["valuation", {value: "guessed"}],
        ])("throws on an unknown %s rather than guessing one", (_what, patch) => {
            expect(() => decodeIncomeStatementReport(mutate(patch))).toThrow(ApiShapeError);
        });

        it("throws when a section omits `trailing` — an empty ladder is a computed answer", () => {
            const patch = {sections: [{kind: "revenue", title: "X", groups: [], total: {current: {}, prior: {}}}]};
            expect(() => decodeIncomeStatementReport(mutate(patch))).toThrow(/missing title\/groups\/trailing/);
        });

        it("throws when a group omits `rows` — absent and empty are different facts", () => {
            const patch = {
                sections: [
                    {
                        kind: "revenue",
                        title: "X",
                        groups: [{name: "G", source: "segment", total: {current: {}, prior: {}}}],
                        total: {current: {}, prior: {}},
                        trailing: [],
                    },
                ],
            };
            expect(() => decodeIncomeStatementReport(mutate(patch))).toThrow(/missing name\/rows/);
        });

        it("throws when a prior window is half-written", () => {
            expect(() => decodeIncomeStatementReport(mutate({prior: {from: "2025-06-26"}}))).toThrow(/expected \{from, to\} ISO dates/);
        });

        it("throws when from, to or sections are missing", () => {
            expect(() => decodeIncomeStatementReport({to: "2026-07-08", sections: []})).toThrow(/expected from, to and a sections array/);
            expect(() => decodeIncomeStatementReport({from: "2026-01-01", to: "2026-07-08"})).toThrow(/expected from, to and a sections array/);
        });
    });
});

describe("UNIT nativeDecode — PeriodReport over the cashflow / networth goldens", () => {
    it("decodes buckets, per-bucket MixedAmounts, and totals", () => {
        const report = decodePeriodReport(golden("cashflow"));
        expect(report.buckets).toEqual(["2026-05", "2026-06", "2026-07"]);
        expect(report.rows.map((r) => r.account)).toEqual(["assets", "assets:bank", "assets:broker"]);
        expect(report.rows[0].values[0].get("$")).toEqual({m: 138878n, p: 2});
        expect(report.rows[0].values[0].get("EUR")).toEqual({m: 50000n, p: 2});
        expect(report.rows[0].values[2].get("$")).toEqual({m: -187500n, p: 2});
        // places 0 stays places 0
        expect(report.rows[2].values[1].get("$")).toEqual({m: 630n, p: 0});
        // a bucket with no activity is an empty object → an empty Map
        expect(report.rows[2].values[2].size).toBe(0);
        expect(report.totals[1].get("$")).toEqual({m: 242512n, p: 2});
        // sample.journal prices every commodity, so the server omits `meta` entirely
        // (reports_golden.rs asserts the same on the Rust side).
        expect(report.meta).toBeUndefined();
    });

    it("decodes market-valued net worth at full computed precision", () => {
        const report = decodePeriodReport(golden("networth"));
        expect(report.rows[0]).toMatchObject({account: "assets", depth: 1});
        // 6 decimal places: a computed value, not a parsed one.
        // 2026-05-31: the stock portfolio plus the home at its then-current
        // $445,000 price and the car at $24,000 — the Other tab's first trend point.
        expect(report.rows[0].values[0].get("$")).toEqual({m: 528729101000n, p: 6});
        expect(report.totals[2].get("$")).toEqual({m: 2119514650n, p: 4});
    });

    // Literal: sample.journal leaves nothing unpriced, so `meta` never appears in
    // the goldens — but the field exists and the SPA renders it.
    it("decodes the net-worth meta.unpriced list", () => {
        const report = decodePeriodReport({buckets: ["2026-07"], rows: [], totals: [{$: dec(1, 0)}], meta: {unpriced: ["GLD", "TSLA"]}});
        expect(report.meta?.unpriced).toEqual(["GLD", "TSLA"]);
    });

    it("throws ApiShapeError when totals is missing", () => {
        expect(() => decodePeriodReport({buckets: [], rows: []})).toThrow(ApiShapeError);
    });
});

describe("UNIT nativeDecode — BudgetReport over the budget golden", () => {
    it("decodes buckets, cells, and the actual/goal pair", () => {
        const report = decodeBudgetReport(golden("budget"));
        expect(report.kind).toBe("budget");
        expect(report.buckets).toEqual(["2026-05", "2026-06", "2026-07"]);

        // sample.journal declares no periodic transactions, so everything lands in
        // the <unbudgeted> catch-all, whose goal is always null.
        const unbudgeted = report.rows[0];
        expect(unbudgeted).toMatchObject({account: "<unbudgeted>", depth: 1});
        expect(unbudgeted.cells[0].actual.get("$")).toEqual({m: -57100n, p: 2});
        expect(unbudgeted.cells[0].actual.get("EUR")).toEqual({m: 50000n, p: 2});
        expect(unbudgeted.cells[0].goal).toBeNull();
        expect(unbudgeted.cells[2].actual.size).toBe(0);
        expect(report.totals[1].actual.get("TSLA")).toEqual({m: -2n, p: 0});
        expect(report.totals[1].goal).toBeNull();
    });

    // Literal: needs a `~` periodic transaction, which fixtures/sample.journal has
    // none of (the budget goldens live in fixtures/budget/ and are exercised by
    // crates/ledgeline-server/tests/report_endpoints.rs).
    it("keeps null goal (unbudgeted) distinct from an empty-object goal (budgeted-but-zero)", () => {
        const report = decodeBudgetReport({
            buckets: ["2026-01", "2026-02"],
            rows: [
                {account: "<unbudgeted>", depth: 1, cells: [{actual: {$: dec(-375, 0)}, goal: null}]},
                // budgeted-but-zero-this-bucket: goal is {} (empty object), NOT null.
                {
                    account: "expenses:gifts",
                    depth: 2,
                    cells: [
                        {actual: {}, goal: {}},
                        {actual: {$: dec(50, 0)}, goal: {$: dec(25, 0)}},
                    ],
                },
            ],
            totals: [{actual: {$: dec(-23, 0)}, goal: {$: dec(400, 0)}}],
        });
        expect(report.rows[0].cells[0].goal).toBeNull();
        const gifts = report.rows[1];
        expect(gifts.cells[0].goal).not.toBeNull();
        expect(gifts.cells[0].goal?.size).toBe(0);
        expect(gifts.cells[1].goal?.get("$")).toEqual({m: 25n, p: 0});
        expect(report.totals[0].goal?.get("$")).toEqual({m: 400n, p: 0});
    });

    it("throws ApiShapeError when totals is missing", () => {
        expect(() => decodeBudgetReport({buckets: [], rows: []})).toThrow(ApiShapeError);
    });

    it("throws ApiShapeError when a row lacks cells", () => {
        expect(() => decodeBudgetReport({buckets: ["2026-01"], rows: [{account: "a", depth: 1}], totals: []})).toThrow(ApiShapeError);
    });
});

describe("UNIT nativeDecode — HoldingsReport over the holdings golden", () => {
    const raw = golden("holdings");

    it("decodes priced and tainted holdings, keeping nulls", () => {
        const report = decodeHoldingsReport(raw);
        expect(report.asOf).toBe("2026-07-08");
        expect(report.base).toBe("$");
        expect(report.holdings.map((h) => h.symbol)).toEqual(["VTI", "AAPL", "TSLA", "GLD"]);

        const aapl = report.holdings[1];
        expect(aapl.name).toBe("Apple Inc.");
        expect(aapl.accounts).toEqual(["assets:broker:taxable:aapl"]);
        expect(aapl.shares).toEqual({m: 195n, p: 1});
        expect(aapl.basis).toEqual({m: 4346100n, p: 3});
        expect(aapl.firstBasisDate).toBe("2024-09-16");
        expect(aapl.price).toEqual({qty: {m: 27025n, p: 2}, date: "2026-06-30", source: "directive"});
        expect(aapl.marketValue).toEqual({m: 5269875n, p: 3});
        expect(aapl.gain).toEqual({m: 923775n, p: 3});
        expect(aapl.gainPct).toBeCloseTo(21.2552633, 6);

        // A short position priced from a cost annotation rather than a P directive.
        const tsla = report.holdings[2];
        expect(tsla.shares).toEqual({m: -2n, p: 0});
        expect(tsla.price?.source).toBe("cost");
        expect(tsla.basis).toBeNull();
        expect(tsla.firstBasisDate).toBeNull();

        // Held but unpriced: every derived field stays null rather than becoming 0.
        const gld = report.holdings[3];
        expect(gld.price).toBeNull();
        expect(gld.marketValue).toBeNull();
        expect(gld.gain).toBeNull();
        expect(gld.gainPct).toBeNull();
    });

    it("decodes portfolio totals and the gainer/loser lists", () => {
        const report = decodeHoldingsReport(raw);
        expect(report.totals.marketValue).toEqual({m: 9922625n, p: 3});
        expect(report.totals.basis).toEqual({m: 9039460n, p: 3});
        expect(report.totals.gain).toEqual({m: 1513165n, p: 3});
        expect(report.totals.gainPct).toBeCloseTo(16.7395508, 6);
        expect(report.topGainers.map((h) => h.symbol)).toEqual(["AAPL", "VTI"]);
        expect(report.topLosers).toEqual([]);
    });

    it("decodes the warning union", () => {
        const report = decodeHoldingsReport(raw);
        expect(report.warnings.map((w) => [w.symbol, w.kind])).toEqual([
            ["GLD", "unpriced"],
            ["GLD", "missing-basis"],
            ["TSLA", "negative-shares"],
        ]);
        expect(report.warnings[0].message).toMatch(/no market price/);
    });

    // Literal: sample.journal's portfolio is priced well enough to produce real
    // totals, so the "refused totals" branch has no golden.
    it("keeps null portfolio totals (honest-totals rule) with a present market value", () => {
        const report = decodeHoldingsReport({
            asOf: "2026-07-08",
            base: "$",
            holdings: [],
            accounts: [],
            totals: {marketValue: dec(10552625, 3), basis: null, gain: null, gainPct: null},
        });
        expect(report.totals.marketValue).toEqual({m: 10552625n, p: 3});
        expect(report.totals.basis).toBeNull();
        expect(report.totals.gain).toBeNull();
        expect(report.totals.gainPct).toBeNull();
    });

    it("rejects an unknown warning kind", () => {
        expect(() => decodeHoldingsReport({...(raw as object), warnings: [{symbol: "X", kind: "surprise", message: "?"}]})).toThrow(/unknown warning kind/);
    });

    it("rejects an unknown price source", () => {
        const holdings = [{symbol: "X", name: "X", shares: dec(1, 0), price: {qty: dec(1, 0), date: "2026-01-01", source: "guess"}}];
        expect(() => decodeHoldingsReport({...(raw as object), holdings})).toThrow(/unknown price source/);
    });

    it("throws ApiShapeError when base or the candidate account list is missing", () => {
        expect(() => decodeHoldingsReport({asOf: "2026-07-08", holdings: [], accounts: [], totals: {marketValue: dec(0, 0)}})).toThrow(ApiShapeError);
        // The scope chooser's option list, demanded for the reason `accounts` is
        // demanded on the Other report: an absent key would empty the picker.
        expect(() => decodeHoldingsReport(without(raw as object as Record<string, unknown>, "accounts"))).toThrow(ApiShapeError);
    });
});

// The golden covers the happy path (below); these literals cover what
// `fixtures/sample.journal` cannot produce — an all-null row, an unknown warning
// kind, an absent key, a mantissa past the safe-integer range. Same division of
// labour every other decoder here uses, and each case says which.
describe("UNIT nativeDecode — OtherHoldingsReport (plans/14)", () => {
    const HOUSE = {
        account: "assets:property:house",
        name: "Family home",
        commodities: {HOUSE: dec(1, 0)},
        value: dec(17500000, 2),
        cost: dec(15000000, 2),
        change: dec(2500000, 2),
        changePct: 16.67,
    };
    const RAW = {
        asOf: "2026-07-08",
        base: "$",
        holdings: [HOUSE],
        accounts: ["assets:partners:acme", "assets:property:house", "assets:vehicles:van"],
        totals: {value: dec(17500000, 2), cost: dec(15000000, 2), change: dec(2500000, 2), changePct: 16.67},
        warnings: [{account: "assets:foreign", kind: "unpriced", message: "no market price for FOO"}],
    };

    it("decodes the committed golden: a commodity-booked home and a dollar-booked car", () => {
        const report = decodeOtherHoldingsReport(golden("holdings-other"));

        expect(report.asOf).toBe("2026-07-08");
        expect(report.base).toBe("$");
        expect(report.holdings.map((h) => h.account)).toEqual(["assets:property:home", "assets:vehicles:car"]);

        // `1 HOME @ $420,000.00`, revalued by `P 2026-06-30 HOME $468,000.00`.
        const home = report.holdings[0];
        expect(home.name).toBe("Family home"); // the `name:` tag, not the last segment
        expect(home.commodities.get("HOME")).toEqual({m: 1n, p: 0});
        expect(home.value).toEqual({m: 46800000n, p: 2});
        expect(home.change).toEqual({m: 4800000n, p: 2});
        expect(home.changePct).toBeCloseTo(11.4285714, 6);

        // Dollar-booked, and depreciated through a SIBLING account tagged
        // `valuation: depreciation`. The two roll into one row, so cost stays
        // gross at $28,000.00 while value is net at $20,500.00 — which is the
        // only way this row can report the loss at all. Posted straight at the
        // asset, the two moved together and the change read $0.00.
        const car = report.holdings[1];
        expect(car.commodities.get("$")).toEqual({m: 2050000n, p: 2});
        expect(car.value).toEqual({m: 2050000n, p: 2});
        expect(car.cost).toEqual({m: 2800000n, p: 2});
        expect(car.change).toEqual({m: -750000n, p: 2});
        expect(car.changePct).toBeCloseTo(-26.7857142, 6);

        expect(report.totals.value).toEqual({m: 48850000n, p: 2});
        expect(report.totals.cost).toEqual({m: 44800000n, p: 2});
        expect(report.totals.changePct).toBeCloseTo(9.0401785, 6);
        // Everything in scope is priced, so nothing is refused.
        expect(report.warnings).toEqual([]);
    });

    it("keeps cost at the precision the engine sent, rather than normalizing it to cents", () => {
        // `cost` for the home arrives as {mantissa: "420000", places: 0} — a
        // whole-dollar cost from a `@ $420,000.00` annotation. Exact is exact;
        // display rounding happens at format time and nowhere else.
        const report = decodeOtherHoldingsReport(golden("holdings-other"));
        expect(report.holdings[0].cost).toEqual({m: 420000n, p: 0});
    });

    it("decodes the Other trend with the STOCK series decoder, which is the contract", () => {
        // Byte-identical wire shape, so there is deliberately no second decoder.
        const series = decodeHoldingsSeries(golden("holdings-other-series"));

        expect(series.base).toBe("$");
        expect(series.hasBasis).toBe(true);
        expect(series.points.map((p) => p.bucket)).toEqual(["2026-05", "2026-06", "2026-07"]);
        // The June revaluation ($445,000 → $468,000) is visible as a step.
        expect(series.points[0].marketValue).toEqual({m: 46900000n, p: 2});
        expect(series.points[1].marketValue).toEqual({m: 48850000n, p: 2});
    });

    it("decodes an account-keyed row, its as-written commodities, and the warning", () => {
        const report = decodeOtherHoldingsReport(RAW);
        expect(report.asOf).toBe("2026-07-08");
        expect(report.base).toBe("$");
        expect(report.holdings.map((h) => h.account)).toEqual(["assets:property:house"]);
        const house = report.holdings[0];
        expect(house.name).toBe("Family home");
        // MixedAmount, exact: "1 HOUSE" is what makes the row revalue at all.
        expect(house.commodities.get("HOUSE")).toEqual({m: 1n, p: 0});
        expect(house.value).toEqual({m: 17500000n, p: 2});
        expect(house.cost).toEqual({m: 15000000n, p: 2});
        expect(house.change).toEqual({m: 2500000n, p: 2});
        expect(house.changePct).toBeCloseTo(16.67, 6);
        expect(report.totals.value).toEqual({m: 17500000n, p: 2});
        expect(report.warnings).toEqual([{account: "assets:foreign", kind: "unpriced", message: "no market price for FOO"}]);
    });

    it("keeps every nullable field null rather than folding it to zero (DRY-3)", () => {
        const report = decodeOtherHoldingsReport({
            ...RAW,
            holdings: [{...HOUSE, value: null, cost: null, change: null, changePct: null}],
            totals: {value: dec(0, 0), cost: null, change: null, changePct: null},
        });
        expect(report.holdings[0].value).toBeNull();
        expect(report.holdings[0].cost).toBeNull();
        expect(report.holdings[0].change).toBeNull();
        expect(report.holdings[0].changePct).toBeNull();
        expect(report.totals.cost).toBeNull();
        expect(report.totals.change).toBeNull();
        expect(report.totals.changePct).toBeNull();
        // `value` is the one unconditional total: an all-unpriced report sums to
        // zero, which is a different fact from "no total".
        expect(report.totals.value).toEqual({m: 0n, p: 0});
    });

    it("decodes the scope chooser's candidate account list, which is wider than the rows", () => {
        const report = decodeOtherHoldingsReport(RAW);

        // Scope- and date-independent by contract, so it names accounts that are
        // NOT rows of this particular report — that is the point of it.
        expect(report.accounts).toEqual(["assets:partners:acme", "assets:property:house", "assets:vehicles:van"]);
        expect(report.holdings.map((h) => h.account)).toEqual(["assets:property:house"]);
    });

    it("throws rather than defaulting an absent candidate list to an empty picker", () => {
        expect(() => decodeOtherHoldingsReport(without(RAW, "accounts"))).toThrow(ApiShapeError);
        // And a non-string member is a broken contract, not something to coerce.
        expect(() => decodeOtherHoldingsReport({...RAW, accounts: ["ok", 7]})).toThrow(ApiShapeError);
    });

    it("freezes what it emits, like its neighbours", () => {
        const report = decodeOtherHoldingsReport(RAW);
        expect(Object.isFrozen(report)).toBe(true);
        expect(Object.isFrozen(report.holdings[0])).toBe(true);
        expect(Object.isFrozen(report.accounts)).toBe(true);
        expect(Object.isFrozen(report.totals)).toBe(true);
        expect(Object.isFrozen(report.warnings[0])).toBe(true);
    });

    it("throws rather than defaulting an absent balance to an empty amount", () => {
        // An account with no balance is not a row at all (membership demands a
        // non-zero one), so an absent `commodities` means a broken contract —
        // and the Holding cell would silently read as a currency-only asset.
        expect(() => decodeOtherHoldingsReport({...RAW, holdings: [{account: "a", name: "a", value: dec(1, 0)}]})).toThrow(ApiShapeError);
    });

    it("rejects an unknown warning kind", () => {
        expect(() => decodeOtherHoldingsReport({...RAW, warnings: [{account: "a", kind: "surprise", message: "?"}]})).toThrow(/unknown warning kind/);
    });

    it("accepts the engine's unpriced-cost warning kind", () => {
        // The wire's second kind (a stranded at-cost basis under a priced
        // value); rejecting it would ApiShapeError the whole report.
        const warning = {account: "a", kind: "unpriced-cost", message: "cost unknown"};
        expect(decodeOtherHoldingsReport({...RAW, warnings: [warning]}).warnings[0]).toEqual(warning);
    });

    it("throws ApiShapeError when any unconditional key is missing", () => {
        const ok = {asOf: "2026-07-08", base: "$", holdings: [], accounts: [], totals: {value: dec(0, 0)}, warnings: []};
        expect(decodeOtherHoldingsReport(ok).holdings).toEqual([]);
        for (const key of ["asOf", "base", "holdings", "accounts", "totals", "warnings"] as const) {
            expect(() => decodeOtherHoldingsReport(without(ok, key)), `dropping ${key} was absorbed`).toThrow(ApiShapeError);
        }
    });

    it("guards the mantissa past Number.MAX_SAFE_INTEGER, like every other Dec on this wire", () => {
        const huge = {mantissa: "90071992547409910000", places: 2};
        const report = decodeOtherHoldingsReport({...RAW, holdings: [{...HOUSE, value: huge}]});
        expect(report.holdings[0].value).toEqual({m: 90071992547409910000n, p: 2});
    });
});

describe("UNIT nativeDecode — HoldingsSeries over the holdings-series golden", () => {
    it("decodes points with labels and basis", () => {
        const series = decodeHoldingsSeries(golden("holdings-series"));
        expect(series.base).toBe("$");
        expect(series.hasBasis).toBe(true);
        expect(series.points).toHaveLength(3);
        expect(series.points[0]).toMatchObject({date: "2026-05-31", bucket: "2026-05", label: "May 2026"});
        expect(series.points[0].marketValue).toEqual({m: 10045300n, p: 3});
        expect(series.points[0].basis).toEqual({m: 9039460n, p: 3});
        expect(series.points[2].date).toBe("2026-07-08");
    });

    // Literal: every point in the golden has a basis.
    it("keeps a null basis null", () => {
        const series = decodeHoldingsSeries({
            base: "$",
            points: [{date: "2026-05-31", bucket: "2026-05", label: "May 2026", marketValue: dec(10045300, 3), basis: null}],
            hasBasis: false,
        });
        expect(series.points[0].basis).toBeNull();
        expect(series.hasBasis).toBe(false);
    });

    it("throws ApiShapeError when points is missing", () => {
        expect(() => decodeHoldingsSeries({base: "$"})).toThrow(ApiShapeError);
    });
});

describe("UNIT nativeDecode — InsightsReport over the insights golden", () => {
    const raw = golden("insights");

    it("decodes the comparison period and the metric deltas", () => {
        const report = decodeInsightsReport(raw);
        expect(report.base).toBe("$");
        expect(report.journalStart).toBe("2024-07-01");
        expect(report.period).toEqual({
            start: "2025-07-01",
            mid: "2026-01-03",
            end: "2026-07-08",
            prevStart: "2025-07-01",
            prevEnd: "2026-01-03",
            currStart: "2026-01-04",
            currEnd: "2026-07-08",
        });

        expect(report.revenue.current.get("$")).toEqual({m: 3401000n, p: 2});
        expect(report.revenue.previous.get("$")).toEqual({m: 3399750n, p: 2});
        expect(report.revenue.delta.get("$")).toEqual({m: 1250n, p: 2});
        expect(report.revenue.pct).toBeCloseTo(0.0367674, 6);
        // a multi-commodity metric keeps every commodity
        expect(report.expenses.delta.get("EUR")).toEqual({m: -47575n, p: 2});
        expect(report.netWorth.current.get("$")).toEqual({m: 2119514650n, p: 4});
        expect(report.cashBalance.delta.get("$")).toEqual({m: 1523496n, p: 2});
    });

    it("decodes cost of living and investment performance", () => {
        const report = decodeInsightsReport(raw);
        // +$3,500 on the current side alone: the 2026-06-30 vehicle depreciation
        // is an expense, and the 2025-06-30 one falls before the previous window.
        expect(report.costOfLiving.currentTotal.get("$")).toEqual({m: 1772613n, p: 2});
        expect(report.costOfLiving.previousTotal.get("$")).toEqual({m: 1550676n, p: 2});
        expect(report.costOfLiving.monthsCurrent).toBe(6);
        expect(report.costOfLiving.monthsPrevious).toBe(6);

        expect(report.investment.current.gain).toEqual({m: 562675n, p: 3});
        expect(report.investment.current.gainPct).toBeCloseTo(5.119905, 6);
        expect(report.investment.previous.gain).toEqual({m: 124425n, p: 2});
    });

    it("decodes change rows, movers, and top transactions", () => {
        const report = decodeInsightsReport(raw);
        const rent = report.expenseChanges[0];
        expect(rent.account).toBe("expenses:housing:rent");
        expect(rent.current).toEqual({m: 1125000n, p: 2});
        expect(rent.previous).toEqual({m: 1305000n, p: 2});
        expect(rent.delta).toEqual({m: -180000n, p: 2});
        expect(rent.pct).toBeCloseTo(-13.7931034, 6);
        expect(rent.kind).toBe("changed");
        expect(report.revenueChanges.map((r) => r.account)).toEqual(["income:dividends", "income:salary"]);

        expect(report.movers.map((m) => m.symbol)).toEqual(["AAPL", "VTI", "GLD"]);
        expect(report.movers[0].name).toBe("Apple Inc.");
        expect(report.movers[0].gain).toEqual({m: 327525n, p: 3});
        expect(report.movers[0].startEstimated).toBe(false);
        // GLD's window start fell back to purchase cost — the UI caveats this row.
        expect(report.movers[2].startEstimated).toBe(true);

        // The index moved 141 → 144 when sample.journal gained three earlier
        // entries (two 2024-07-01 opening positions and the 2025-06-30
        // depreciation); the transaction itself is the same one.
        const top = report.topTxns[0];
        expect(top).toMatchObject({index: 144, date: "2026-01-27", description: "Acme Corp | January salary"});
        expect(top.amount).toEqual({m: 566000n, p: 2});
    });

    // Literal: sample.journal produces only `changed` rows.
    it("decodes the `ended` change kind and rejects anything else", () => {
        const base = raw as {expenseChanges: unknown[]};
        const row = {account: "expenses:gone", current: dec(0, 2), previous: dec(5000, 2), delta: dec(-5000, 2), pct: -100, kind: "ended"};
        const report = decodeInsightsReport({...(raw as object), expenseChanges: [row]});
        expect(report.expenseChanges[0].kind).toBe("ended");
        expect(base.expenseChanges.length).toBeGreaterThan(0);

        expect(() => decodeInsightsReport({...(raw as object), expenseChanges: [{...row, kind: "new"}]})).toThrow(/unknown change kind/);
    });

    it("throws ApiShapeError when base is missing", () => {
        expect(() => decodeInsightsReport({period: {}})).toThrow(ApiShapeError);
    });
});

describe("UNIT nativeDecode — SubscriptionsReport over the subscriptions golden", () => {
    const raw = golden("subscriptions");

    it("decodes the detected monthly charges", () => {
        const report = decodeSubscriptionsReport(raw);
        expect(report.asOf).toBe("2026-07-08");
        expect(report.lookbackStart).toBe("2024-07-08");

        const rent = report.monthly[0];
        expect(rent.payee).toBe("Oakview Properties");
        expect(rent.cadence).toBe("monthly");
        expect(rent.typicalAmount).toEqual({m: 187500n, p: 2});
        expect(rent.annualizedCost).toEqual({m: 22500n, p: 0});
        expect(rent.occurrences).toBe(24);
        expect(rent.firstSeen).toBe("2024-08-01");
        expect(rent.lastSeen).toBe("2026-07-01");
        expect(rent.nextExpected).toBe("2026-08-01");
        expect(rent.accounts).toEqual(["expenses:housing:rent"]);
        expect(rent.manual).toBe(false);
    });

    it("finds no annual subscription in the golden, and that is deliberate", () => {
        const report = decodeSubscriptionsReport(raw);

        // This briefly listed `annual vehicle depreciation`. plans/14 gave
        // sample.journal two vehicle write-downs a year apart, and while they
        // shared one description they were a twice-seen, same-account expense —
        // exactly the shape the cadence detector matches. The detector was not
        // wrong; the FIXTURE was, because a non-cash depreciation entry listed
        // under Subscriptions is the demo journal teaching something false.
        //
        // The two entries are now described distinctly (`vehicle depreciation
        // FY2025` / `FY2026`), which is also just better bookkeeping. This
        // assertion is the guard: if a future fixture edit makes the demo
        // journal claim a depreciation charge is a subscription again, it fails
        // here rather than being noticed by a user browsing the demo.
        expect(report.annual).toEqual([]);
    });

    // Literal: sample.journal carries no `subscription:true` tag, so the manual
    // flag is `false` everywhere in the golden.
    it("decodes a manually-tagged subscription", () => {
        const report = decodeSubscriptionsReport({
            asOf: "2026-07-08",
            lookbackStart: "2024-07-08",
            monthly: [],
            annual: [
                {
                    payee: "Domain Registrar",
                    cadence: "annual",
                    typicalAmount: dec(1800, 2),
                    annualizedCost: dec(1800, 2),
                    occurrences: 2,
                    firstSeen: "2024-09-01",
                    lastSeen: "2025-09-01",
                    nextExpected: "2026-09-01",
                    accounts: ["expenses:online"],
                    manual: true,
                },
            ],
        });
        expect(report.annual[0].cadence).toBe("annual");
        expect(report.annual[0].manual).toBe(true);
    });

    it("rejects an unknown cadence", () => {
        const bad = {asOf: "2026-07-08", lookbackStart: "2024-07-08", monthly: [{...(raw as {monthly: object[]}).monthly[0], cadence: "weekly"}]};
        expect(() => decodeSubscriptionsReport(bad)).toThrow(/unknown cadence/);
    });

    it("throws ApiShapeError when lookbackStart is missing", () => {
        expect(() => decodeSubscriptionsReport({asOf: "2026-07-08"})).toThrow(ApiShapeError);
    });
});

// ---------------------------------------------------------------------------
// DRY-3: a missing amount is an error, never a zero
// ---------------------------------------------------------------------------

describe("UNIT nativeDecode — a missing MixedAmount throws instead of rendering zero", () => {
    // decodeMixed used to return an empty Map for undefined, so a renamed or
    // dropped Rust field became `$0.00` (format.ts fmtBase) / `0`
    // (ReportTable.svelte) with nothing raising on either side of the wire.
    const row = (extra: object) => ({sections: [{title: "Assets", rows: [{account: "assets", depth: 1, ...extra}], total: {}}], grandTotal: {}});

    it("a report missing `inclusive` throws rather than rendering zero", () => {
        expect(() => decodeSectionedReport(row({own: {}}))).toThrow(ApiShapeError);
        expect(() => decodeSectionedReport(row({own: {}}))).toThrow(/inclusive: missing amount/);
    });

    it("a report missing `own` throws", () => {
        expect(() => decodeSectionedReport(row({inclusive: {}}))).toThrow(/own: missing amount/);
    });

    it("a section missing `total` throws", () => {
        expect(() => decodeSectionedReport({sections: [{title: "Assets", rows: []}], grandTotal: {}})).toThrow(/total: missing amount/);
    });

    it("a report missing `grandTotal` throws", () => {
        expect(() => decodeSectionedReport({sections: []})).toThrow(/grandTotal: missing amount/);
    });

    it("a period report missing a bucket value throws", () => {
        const bad = {buckets: ["2026-07"], rows: [{account: "a", depth: 1, values: [undefined]}], totals: [{}]};
        expect(() => decodePeriodReport(bad)).toThrow(/values\[0\]: missing amount/);
    });

    it("a budget cell missing `actual` throws", () => {
        const bad = {buckets: ["2026-07"], rows: [{account: "a", depth: 1, cells: [{goal: null}]}], totals: []};
        expect(() => decodeBudgetReport(bad)).toThrow(/actual: missing amount/);
    });

    it("an insights metric missing `delta` throws", () => {
        const bad = {...(golden("insights") as object), revenue: {current: {}, previous: {}, pct: 0}};
        expect(() => decodeInsightsReport(bad)).toThrow(/delta: missing amount/);
    });

    it("still decodes an EMPTY amount `{}` as an empty Map — absent and empty are different facts", () => {
        const report = decodeSectionedReport(row({own: {}, inclusive: {}}));
        expect(report.sections[0].rows[0].inclusive.size).toBe(0);
        expect(report.grandTotal.size).toBe(0);
    });
});

// ---------------------------------------------------------------------------
// DRY-3: the cross-language rename guard
// ---------------------------------------------------------------------------

/** Deep-clone `node`, renaming every key whose collapsed path equals `target`. */
function renameKeyAt(node: unknown, path: string, target: string): unknown {
    if (Array.isArray(node)) return node.map((child) => renameKeyAt(child, `${path}[]`, target));
    if (node !== null && typeof node === "object") {
        const out: Record<string, unknown> = {};
        for (const [key, value] of Object.entries(node)) {
            const child = `${path}.${key}`;
            out[child === target ? `${key}Renamed` : key] = renameKeyAt(value, child, target);
        }
        return out;
    }
    return node;
}

/** Every key path in `node`, with array indices collapsed to `[]`. */
function keyPaths(node: unknown, path: string, out: Set<string>): void {
    if (Array.isArray(node)) {
        for (const child of node) keyPaths(child, `${path}[]`, out);
    } else if (node !== null && typeof node === "object") {
        for (const [key, value] of Object.entries(node)) {
            out.add(`${path}.${key}`);
            keyPaths(value, `${path}.${key}`, out);
        }
    }
}

/** Structural snapshot of a decoded report — Maps and BigInts made comparable. */
function shape(value: unknown): string {
    return JSON.stringify(value, (_key, item) => (typeof item === "bigint" ? `${item}n` : item instanceof Map ? [...item.entries()] : (item as unknown)));
}

describe("UNIT nativeDecode — RulesIndex over the rules-index golden", () => {
    it("decodes the discovery listing, including the fields a summary needs", () => {
        const index = decodeRulesIndex(rulesGolden("rules-index"));
        expect(index.rootLabel).toBe("tree");
        expect(index.editable).toBe(true);
        expect(index.truncated).toBe(false);
        expect(index.warnings).toEqual([]);
        expect(index.files).toHaveLength(1);
        expect(index.files[0]).toEqual({
            id: "import/2026/bank.csv.rules",
            label: "bank",
            revision: "807-cdb071b43e7abbba",
            sizeBytes: 2055,
            parsed: true,
            account1: "assets:bank:checking",
            account2: "expenses:unknown",
            ifBlockCount: 5,
            editableBlockCount: 4,
            opaqueItemCount: 1,
            warnings: [],
        });
    });

    // `account1`/`account2` are `skip_serializing_if = "Option::is_none"`, so an
    // absent key means "this file declares neither" — a different fact from an
    // empty string, and one the file list renders differently.
    it("reads an absent account1/account2 as null rather than as an empty string", () => {
        const index = decodeRulesIndex({
            rootLabel: "fixtures",
            editable: false,
            truncated: false,
            files: [{id: "a.rules", label: "a", revision: "0-0", sizeBytes: 0, parsed: false, ifBlockCount: 0, editableBlockCount: 0, opaqueItemCount: 0}],
            warnings: ["one file was skipped"],
        });
        expect(index.files[0]).toMatchObject({account1: null, account2: null, parsed: false, warnings: []});
        expect(index.warnings).toEqual(["one file was skipped"]);
    });

    it("throws ApiShapeError when the files array is missing", () => {
        expect(() => decodeRulesIndex({rootLabel: "x", editable: true})).toThrow(ApiShapeError);
    });
});

describe("UNIT nativeDecode — RulesDocument over the rules-doc golden", () => {
    const raw = rulesGolden("rules-doc");

    it("decodes the document header and every settings entry with its item id", () => {
        const doc = decodeRulesDoc(raw);
        expect(doc.id).toBe("import/2026/bank.csv.rules");
        expect(doc.label).toBe("bank");
        expect(doc.revision).toBe("807-cdb071b43e7abbba");
        expect(doc.editable).toBe(true);
        expect(doc.newline).toBe("lf");
        expect(doc.warnings).toEqual([]);

        expect(doc.settings.dateFormat).toEqual({value: "%Y-%m-%d", itemId: 3});
        expect(doc.settings.skip).toEqual({value: 1, itemId: 1});
        expect(doc.settings.account1).toEqual({value: "assets:bank:checking", itemId: 5});
        expect(doc.settings.account2).toEqual({value: "expenses:unknown", itemId: 6});
        expect(doc.settings.currency).toEqual({value: "$", itemId: 4});
        expect(doc.settings.fields).toEqual({names: ["date", "description", "amount"], itemId: 2});
        // Absent settings are "the file does not say", which is NOT hledger's
        // default for them — choosing a default is a rendering decision.
        expect(doc.settings.separator).toBeNull();
        expect(doc.settings.source).toBeNull();
        expect(doc.settings.newestFirst).toBeNull();
    });

    it("decodes every item kind the wire can carry, in order", () => {
        const doc = decodeRulesDoc(raw);
        expect(doc.items.map((item) => item.kind)).toEqual([
            "trivia",
            "directive",
            "fields",
            "directive",
            "assignment",
            "assignment",
            "assignment",
            "ifBlock",
            "ifBlock",
            "ifBlock",
            "ifBlock",
            "opaque",
        ]);
        expect(doc.items.map((item) => item.id)).toEqual([0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]);

        const trivia = doc.items[0];
        expect(trivia).toMatchObject({kind: "trivia", line: 1, lines: 13, truncated: false});
        expect(trivia?.kind === "trivia" && trivia.text).toContain("discovery MUST find");

        expect(doc.items[1]).toMatchObject({kind: "directive", name: "skip", value: "1"});
        expect(doc.items[2]).toMatchObject({kind: "fields", names: ["date", "description", "amount"]});
        expect(doc.items[5]).toMatchObject({kind: "assignment", field: "account1", value: "assets:bank:checking"});
    });

    it("reads a whole-record matcher's absent `field` as null, and a scoped one by name", () => {
        const doc = decodeRulesDoc(raw);
        expect(doc.items[7]).toMatchObject({
            kind: "ifBlock",
            layout: "inline",
            groups: [{matchers: [{field: null, pattern: "COFFEE"}]}],
            assignments: [{field: "account2", value: "expenses:food:coffee"}],
        });
        // A plain OR list is ONE MATCHER PER GROUP, not one group holding both:
        // the two readings differ in which rows the file matches, and only the
        // nesting says which one this file means.
        expect(doc.items[9]).toMatchObject({
            kind: "ifBlock",
            layout: "stacked",
            groups: [{matchers: [{field: "description", pattern: "SUPERMARKET"}]}, {matchers: [{field: "description", pattern: "GROCER"}]}],
            assignments: [
                {field: "account2", value: "expenses:food:groceries"},
                {field: "comment", value: "weekly shop"},
            ],
        });
    });

    // The AND is carried by NESTING, and this is the item that pins it: two
    // matchers inside ONE group, which is a different rule from the two-group
    // OR above and would decode identically under a flattened reading.
    it("reads an AND-group as two matchers inside one group", () => {
        expect(decodeRulesDoc(raw).items[10]).toMatchObject({
            kind: "ifBlock",
            layout: "stacked",
            groups: [
                {
                    matchers: [
                        {field: "description", pattern: "AIRLINE"},
                        {field: "amount", pattern: "^-"},
                    ],
                },
            ],
            assignments: [{field: "account2", value: "expenses:travel:airfare"}],
        });
    });

    // VERBATIM from the contract in `rules_api.rs` (`WireItemBody::IfBlock` /
    // `WireMatcherGroup`), not round-tripped through our own encoder — the same
    // discipline `importModel.test.ts` documents. A literal that was generated
    // from this decoder's own idea of the shape would agree with it by
    // construction and could never catch the engine renaming a key.
    it("decodes a multi-group block from the wire's own JSON", () => {
        const doc = decodeRulesDoc({
            id: "x.rules",
            label: "x",
            revision: "1-0",
            editable: true,
            newline: "lf",
            settings: {},
            items: [
                {
                    id: 0,
                    line: 1,
                    lines: 5,
                    kind: "ifBlock",
                    layout: "stacked",
                    groups: [
                        {
                            matchers: [
                                {field: "description", pattern: "AMAZON"},
                                {field: "card", pattern: "personal"},
                            ],
                        },
                        {matchers: [{pattern: "COSTCO"}]},
                    ],
                    assignments: [{field: "account2", value: "expenses:shopping:online"}],
                },
            ],
            warnings: [],
        });
        const block = doc.items[0];
        if (block?.kind !== "ifBlock") throw new Error("expected an ifBlock");
        expect(block.groups.map((group) => group.matchers.map((matcher) => [matcher.field, matcher.pattern]))).toEqual([
            [
                ["description", "AMAZON"],
                ["card", "personal"],
            ],
            [[null, "COSTCO"]],
        ]);
    });

    // VERBATIM from `WireItemBody::IfBlock` again, for the key the engine only
    // emits sometimes. Both halves matter: `"skip"` has to arrive as `"skip"`,
    // and an ABSENT key has to become `null` rather than `undefined` — the
    // engine omits it with `skip_serializing_if`, exactly as it omits a
    // whole-record matcher's `field`, and every rules file that predates this
    // feature sends blocks without it.
    it("decodes an if-block control word, and an absent one as null", () => {
        const wire = (item: Record<string, unknown>) => ({
            id: "x.rules",
            label: "x",
            revision: "1-0",
            editable: true,
            newline: "lf",
            settings: {},
            items: [
                {
                    id: 0,
                    line: 1,
                    lines: 2,
                    kind: "ifBlock",
                    layout: "inline",
                    groups: [{matchers: [{field: "description", pattern: "PENDING"}]}],
                    assignments: [],
                    ...item,
                },
            ],
            warnings: [],
        });

        expect(decodeRulesDoc(wire({control: "skip"})).items[0]).toMatchObject({kind: "ifBlock", control: "skip"});
        expect(decodeRulesDoc(wire({control: "end"})).items[0]).toMatchObject({kind: "ifBlock", control: "end"});
        expect(decodeRulesDoc(wire({})).items[0]).toMatchObject({kind: "ifBlock", control: null});

        // A third control word would mean the engine grew one. Rendering that
        // block as "imports as usual" would hide an instruction to drop rows,
        // so it is refused instead.
        expect(() => decodeRulesDoc(wire({control: "halt"}))).toThrow(ApiShapeError);
    });

    // Flattening `groups` into a matcher list would turn an AND into an OR —
    // the rule would start matching either condition instead of both — so a
    // body without the nesting is refused rather than read the old way.
    it("refuses an ifBlock that sends a flat matcher list instead of groups", () => {
        const items = (raw as {items: Record<string, unknown>[]}).items;
        const flattened = {...items[7], groups: undefined, matchers: [{pattern: "COFFEE"}]};
        expect(() => decodeRulesDoc({...(raw as object), items: [flattened]})).toThrow(ApiShapeError);
        expect(() => decodeRulesDoc({...(raw as object), items: [{...items[7], groups: [{}]}]})).toThrow(ApiShapeError);
    });

    it("carries an opaque construct's reason, label and raw text", () => {
        const opaque = decodeRulesDoc(raw).items[11];
        expect(opaque).toMatchObject({kind: "opaque", reason: "ifTable", label: "if,account2,comment", truncated: false});
        expect(opaque?.kind === "opaque" && opaque.text).toContain("ATM WITHDRAWAL,assets:cash,cash out");
    });

    // An item whose shape the decoder invented would be echoed back to the
    // engine under an id it has no right to claim, so refusing to open the
    // document is the honest failure.
    it("throws on an unknown item kind rather than inventing a carry-through", () => {
        const doc = raw as {items: unknown[]};
        const mutated = {...doc, items: [...doc.items, {id: 12, line: 60, lines: 1, kind: "somethingNew"}]};
        expect(() => decodeRulesDoc(mutated)).toThrow(ApiShapeError);
    });

    it("throws on an unknown opaque reason, layout or newline", () => {
        expect(() => decodeRulesDoc({...(raw as object), newline: "cr"})).toThrow(ApiShapeError);
        const items = (raw as {items: Record<string, unknown>[]}).items;
        expect(() => decodeRulesDoc({...(raw as object), items: [{...items[11], reason: "somethingNew"}]})).toThrow(ApiShapeError);
        expect(() => decodeRulesDoc({...(raw as object), items: [{...items[7], layout: "table"}]})).toThrow(ApiShapeError);
    });

    it("decodes an anchored warning, and one about the file as a whole", () => {
        const doc = decodeRulesDoc({
            ...(raw as object),
            warnings: [
                {itemId: 2, line: 15, message: "hledger requires at least two comma-separated field names"},
                {line: 0, message: "this file has no date field"},
            ],
        });
        expect(doc.warnings[0]).toEqual({itemId: 2, line: 15, message: "hledger requires at least two comma-separated field names"});
        expect(doc.warnings[1]).toEqual({itemId: null, line: 0, message: "this file has no date field"});
    });
});

describe("UNIT nativeDecode — RulesPreview", () => {
    // Literal, not a golden: the preview route has no committed body (its rows
    // come from a CSV, not from the journal). These are the exact bytes the
    // engine answered for fixtures/rules/tree/import/2026/bank.csv.
    it("decodes an available preview, header and sample rows included", () => {
        const preview = decodeRulesPreview({
            available: true,
            dataLabel: "bank.csv",
            separator: ",",
            header: ["Date", "Description", "Amount"],
            rows: [["2026-01-03", "COFFEE HOUSE", "-6.45"]],
            columns: 3,
            truncated: false,
        });
        expect(preview.available).toBe(true);
        expect(preview.reason).toBeNull();
        expect(preview.dataLabel).toBe("bank.csv");
        expect(preview.header).toEqual(["Date", "Description", "Amount"]);
        expect(preview.rows).toEqual([["2026-01-03", "COFFEE HOUSE", "-6.45"]]);
        expect(preview.columns).toBe(3);
    });

    // A refusal is a VALUE, not an error: "your `source` is a shell command we
    // will not run" is information the mapping panel shows.
    it("decodes a refusal with its typed reason and no header", () => {
        const preview = decodeRulesPreview({
            available: false,
            reason: "sourceIsCommand",
            dataLabel: "piped",
            separator: ",",
            rows: [],
            columns: 0,
            truncated: false,
        });
        expect(preview).toMatchObject({available: false, reason: "sourceIsCommand", header: null, rows: [], columns: 0});
    });

    it("throws on a reason it does not know rather than reporting `no reason`", () => {
        expect(() => decodeRulesPreview({available: false, reason: "somethingNew", separator: ",", rows: [], columns: 0})).toThrow(ApiShapeError);
    });

    it("throws when rows is missing", () => {
        expect(() => decodeRulesPreview({available: true, separator: ","})).toThrow(ApiShapeError);
    });
});

describe("UNIT nativeDecode — journal identity (which ledger is on screen)", () => {
    it("decodes the engine's title and the bare journal file name", () => {
        expect(decodeJournalInfo({title: "Acme Books", file: "2026.journal"})).toEqual({title: "Acme Books", file: "2026.journal"});
    });

    it("reads null and absent alike as `the engine derived none`", () => {
        // The app bar's fallback (name the server URL instead) is driven off
        // null, so an older engine that omits a key entirely must land in the
        // same branch as one that sends it as null — not in a decode failure.
        expect(decodeJournalInfo({title: null, file: null})).toEqual({title: null, file: null});
        expect(decodeJournalInfo({})).toEqual({title: null, file: null});
        expect(decodeJournalInfo({title: "Acme Books"})).toEqual({title: "Acme Books", file: null});
    });

    it("throws rather than labelling the screen with whatever a non-string stringifies to", () => {
        // The whole feature is "say WHICH ledger this is", so a title of
        // `"[object Object]"` or `"7"` would be worse than no title at all: the
        // caller's null branch shows the URL, which at least does not claim.
        expect(() => decodeJournalInfo({title: {name: "Acme"}, file: "2026.journal"})).toThrow(ApiShapeError);
        expect(() => decodeJournalInfo({title: "Acme Books", file: 7})).toThrow(ApiShapeError);
    });

    it("throws on a body that is not an object at all", () => {
        // What a non-engine server on the same port answers with — an HTML page
        // is caught earlier as `NativeApiUnavailableError`, but a bare JSON
        // string or array reaches here.
        expect(() => decodeJournalInfo(null)).toThrow(ApiShapeError);
        expect(() => decodeJournalInfo("Acme Books")).toThrow(ApiShapeError);
        // `typeof [] === "object"`, and `[].title` is undefined — an array used
        // to be absorbed as {title: null, file: null}, an answer no engine gave.
        expect(() => decodeJournalInfo([])).toThrow(ApiShapeError);
        expect(() => decodeJournalInfo(["Acme Books"])).toThrow(ApiShapeError);
    });
});

describe("UNIT nativeDecode — renaming any wire key is detected, not absorbed", () => {
    const DECODERS: [string, (raw: unknown) => unknown][] = [
        ["balancesheet", decodeSectionedReport],
        ["balancesheet-grouped", decodeBalanceSheetReport],
        ["incomestatement", decodeSectionedReport],
        ["incomestatement-grouped", decodeIncomeStatementReport],
        ["incomestatement-flows", decodeFlowReport],
        ["cashflow", decodePeriodReport],
        ["networth", decodePeriodReport],
        ["budget", decodeBudgetReport],
        ["insights", decodeInsightsReport],
        ["subscriptions", decodeSubscriptionsReport],
        ["holdings", decodeHoldingsReport],
        ["holdings-series", decodeHoldingsSeries],
        ["holdings-other", decodeOtherHoldingsReport],
        // Same decoder as the stock series, because the engine reuses
        // `WireHoldingsSeries` for it byte for byte. Swept separately anyway: the
        // two goldens have different VALUES, and a rename this one absorbs is not
        // necessarily one the other absorbs.
        ["holdings-other-series", decodeHoldingsSeries],
    ];

    // Renames these decoders CANNOT currently notice. Every one is a gap in the
    // FIXTURE, not in the decoder: the field's value in fixtures/sample.journal
    // happens to equal the value the decoder falls back to, so dropping the key
    // is indistinguishable from keeping it. Closing them needs journal data
    // sample.journal does not have (a `~` periodic transaction, a
    // `subscription:true` tag, an annual charge, a losing position).
    //
    // This list must only ever SHRINK. A new entry means a newly-added field can
    // be renamed in Rust and silently absorbed here — the DRY-3 bug, returning.
    const TOLERATED = new Set([
        "budget $.rows[].cells[].goal", // every goal in the golden is already null
        "budget $.totals[].goal", // ditto
        "subscriptions $.monthly[].manual", // no `subscription:true` tag in sample.journal
        // The whole array, because sample.journal has no annual subscription and
        // the golden's `annual` is `[]`.
        //
        // It briefly did: plans/14's two vehicle-depreciation entries shared one
        // description a year apart, which is exactly the shape the cadence
        // detector matches, and that shrank this list to `$.annual[].manual`.
        // The entries have since been described distinctly, because a non-cash
        // write-down listed under Subscriptions is the demo journal teaching
        // something false — a worse cost than this gap. So this is the same
        // entry that was here before plans/14, not a new one; the list did not
        // grow. Closing it honestly needs a real annual charge in
        // sample.journal (an insurance premium, a domain renewal).
        "subscriptions $.annual",
        "holdings $.topLosers", // nothing is down as of 2026-07-08 → [] either way
        // `incomestatement-grouped $.meta.unpriced` used to sit here: no income
        // or expense account in sample.journal holds an unpriced commodity at
        // ANY range, so the list is `[]` on both sides of that rename. It is
        // closed now, and by the decoder rather than by the fixture:
        // `decodeReportMeta` DEMANDS `unpriced` inside a meta block that is
        // present, because all four `WireReportMeta` sites are one plain
        // `Vec<String>` that serde always writes.
    ]);

    /** Rename every key of every golden in turn; report the ones nothing noticed. */
    function sweep(): {tolerated: string[]; checked: number} {
        const tolerated: string[] = [];
        let checked = 0;
        for (const [name, decode] of DECODERS) {
            const raw = golden(name);
            const baseline = shape(decode(raw));
            const paths = new Set<string>();
            keyPaths(raw, "$", paths);

            for (const path of paths) {
                checked += 1;
                let absorbed: boolean;
                try {
                    absorbed = shape(decode(renameKeyAt(raw, "$", path))) === baseline;
                } catch {
                    absorbed = false; // threw — the rename was caught, which is the point
                }
                if (absorbed) tolerated.push(`${name} ${path}`);
            }
        }
        return {tolerated, checked};
    }

    it("every key in every golden is load-bearing, except the documented fixture gaps", () => {
        expect(new Set(sweep().tolerated)).toEqual(TOLERATED);
    });

    it("checks a meaningful number of keys (the sweep is actually running)", () => {
        expect(sweep().checked).toBeGreaterThan(300);
    });
});

describe("UNIT nativeDecode — the budget editor's listing", () => {
    /** One file, one monthly rule, two goals: an expense and an income one. */
    const LISTING = {
        editable: true,
        defaultTarget: "budget.journal",
        canCreateFile: false,
        createFileName: "budget.journal",
        files: [
            {
                journalId: "budget.journal",
                label: "budget.journal",
                revision: "abc123",
                writable: true,
                rules: [
                    {
                        block: 0,
                        line: 3,
                        period: "monthly",
                        description: "household budget",
                        lines: [
                            {
                                index: 0,
                                line: 4,
                                account: "expenses:food",
                                unbalanced: true,
                                amount: {$: {mantissa: "40000", places: 2}},
                                entry: {commodity: "$", value: {mantissa: "40000", places: 2}},
                                inverted: false,
                            },
                            {
                                index: 1,
                                line: 5,
                                account: "income:interest",
                                unbalanced: true,
                                amount: {$: {mantissa: "-120000", places: 2}},
                                entry: {commodity: "$", value: {mantissa: "120000", places: 2}},
                                inverted: true,
                            },
                        ],
                    },
                ],
            },
        ],
    };

    it("carries the file's amount and the user's magnitude as two separate facts", () => {
        // This is the whole sign contract. `amount` is what hledger reads;
        // `entry` is what goes in the number box. Collapsing them — or deriving
        // one from the other in the browser — is how an income budget comes to
        // be written with the wrong sign, silently, and only for some accounts.
        const listing = decodeBudgetListing(LISTING);
        const [food, interest] = listing.files[0].rules[0].goals;

        expect(food.inverted).toBe(false);
        expect(food.amount?.get("$")).toEqual({m: 40000n, p: 2});
        expect(food.entry).toEqual({commodity: "$", value: {m: 40000n, p: 2}});

        expect(interest.inverted).toBe(true);
        expect(interest.amount?.get("$")).toEqual({m: -120000n, p: 2});
        expect(interest.entry).toEqual({commodity: "$", value: {m: 120000n, p: 2}});
    });

    it("reads the wire's `lines` as the domain's `goals`, keeping rule and file identity", () => {
        const listing = decodeBudgetListing(LISTING);
        expect(listing.editable).toBe(true);
        expect(listing.defaultTarget).toBe("budget.journal");
        expect(listing.files[0].revision).toBe("abc123");
        const rule = listing.files[0].rules[0];
        expect(rule.period).toBe("monthly");
        expect(rule.description).toBe("household budget");
        expect(rule.locked).toBeNull();
        expect(rule.goals.map((goal) => goal.account)).toEqual(["expenses:food", "income:interest"]);
    });

    it("keeps a lock's sentence, which is the only thing that makes a read-only row actionable", () => {
        const locked = {
            ...LISTING,
            files: [
                {
                    ...LISTING.files[0],
                    rules: [
                        {
                            ...LISTING.files[0].rules[0],
                            locked: "its period is not one of daily, weekly, monthly, quarterly or yearly",
                            lines: [{...LISTING.files[0].rules[0].lines[0], locked: "it has no written amount", amount: undefined, entry: undefined}],
                        },
                    ],
                },
            ],
        };
        const listing = decodeBudgetListing(locked);
        expect(listing.files[0].rules[0].locked).toContain("not one of daily");
        const goal = listing.files[0].rules[0].goals[0];
        expect(goal.locked).toContain("no written amount");
        // Absent together: a line with no written amount has no box to type in.
        expect(goal.amount).toBeNull();
        expect(goal.entry).toBeNull();
    });

    it("refuses a body that is not a listing at all", () => {
        expect(() => decodeBudgetListing({editable: true})).toThrow(ApiShapeError);
        expect(() => decodeBudgetListing(null)).toThrow(ApiShapeError);
    });
});

describe("UNIT nativeDecode — the budget editor's reference strip", () => {
    const REFERENCE = {
        account: "expenses:food",
        interval: "monthly",
        inverted: false,
        periods: [
            {key: "2026-06", label: "Jun 2026", start: "2026-06-01", end: "2026-06-30", complete: true, total: {$: {mantissa: "61200", places: 2}}},
            {key: "2026-07", label: "Jul 2026", start: "2026-07-01", end: "2026-07-15", complete: false, total: {$: {mantissa: "38900", places: 2}}},
        ],
        average: {$: {mantissa: "61200", places: 2}},
        averagedPeriods: 1,
    };

    it("keeps the running period's flag and its clamped end", () => {
        const reference = decodeAccountReference(REFERENCE);
        expect(reference.periods.map((p) => p.complete)).toEqual([true, false]);
        expect(reference.periods[1].end).toBe("2026-07-15");
        expect(reference.periods[0].total.get("$")).toEqual({m: 61200n, p: 2});
    });

    it("reads an absent `complete` as finished, not as still running", () => {
        // The quiet reading: an engine that does not say must not put a "so far"
        // caveat on what is actually a whole month.
        const reference = decodeAccountReference({...REFERENCE, periods: [{...REFERENCE.periods[0], complete: undefined}]});
        expect(reference.periods[0].complete).toBe(true);
    });

    it("carries the average and how many periods it covers", () => {
        const reference = decodeAccountReference(REFERENCE);
        // The mean of the COMPLETE periods only — June, not June-and-part-of-July.
        expect(reference.average.get("$")).toEqual({m: 61200n, p: 2});
        expect(reference.averagedPeriods).toBe(1);
    });

    it("reads a missing average as ABSENT, never as zero", () => {
        // `averagedPeriods: 0` is "no complete period yet", which is a different
        // fact from an average of nil — the strip prints a number for one and
        // nothing for the other.
        const none = decodeAccountReference({...REFERENCE, average: undefined, averagedPeriods: undefined});
        expect(none.averagedPeriods).toBe(0);
        expect(none.average.size).toBe(0);
        // And an empty average WITH a count is a real answer: "you spent nothing,
        // twice". The two fields are decoded independently so that survives.
        const nothing = decodeAccountReference({...REFERENCE, average: {}, averagedPeriods: 2});
        expect(nothing.averagedPeriods).toBe(2);
        expect(nothing.average.size).toBe(0);
    });

    it("refuses a body with no periods array", () => {
        expect(() => decodeAccountReference({account: "a", interval: "monthly"})).toThrow(ApiShapeError);
    });
});
