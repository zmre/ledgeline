// The grouped income statement's display model: what is on screen at a given
// collapse state, and how two exact decimals become "-13.7%".
//
// These are the claims the view rests on and cannot make about itself: a
// collapsed group contributes exactly one row (so `j` has somewhere to go on
// first load, and the cursor never points at an invisible account), the ladder
// only carries rungs the engine sent, and the percentage column is derived from
// the engine's exact `Dec` values rather than from the rounded strings beside it.

import {readFileSync} from "node:fs";
import {describe, expect, it} from "vitest";
import {decodeIncomeStatementReport} from "#lib/api/nativeDecode.js";
import {dec, type MixedAmount} from "#lib/domain/money.js";
import {fmt} from "#lib/format/amounts.js";
import type {IncomeStatementReport, IsSection} from "#lib/reports/types.js";
import {GROUPED_INCOME_STATEMENT, MULTI_STEP_INCOME_STATEMENT, UNCOMPARED_INCOME_STATEMENT} from "#lib/testing/incomeStatementFixture.js";
import {amountsPct, fmtPct, isCursorRows, isDisplayModel, isGroupKey, isSummary, pctChange, sectionDisplayRows} from "./incomeStatementRows";

const REPORT = decodeIncomeStatementReport(GROUPED_INCOME_STATEMENT);
const MULTI = decodeIncomeStatementReport(MULTI_STEP_INCOME_STATEMENT);
const [REVENUE, EXPENSES] = REPORT.sections;

/** A committed engine golden, decoded. */
const golden = (name: string): IncomeStatementReport =>
    decodeIncomeStatementReport(JSON.parse(readFileSync(new URL(`../../../../../fixtures/native/v1/${name}.json`, import.meta.url), "utf8")));

const NONE = (): boolean => false;
const ALL = (): boolean => true;
const only =
    (...keys: string[]) =>
    (key: string): boolean =>
        keys.includes(key);

/** One section's rows at a given collapse state, in this report's base commodity. */
const rowsOf = (section: IsSection, expanded: (key: string) => boolean, report: IncomeStatementReport = REPORT) =>
    sectionDisplayRows(section, expanded, report.base);

describe("UNIT reports/ui/incomeStatementRows — sectionDisplayRows", () => {
    it("shows one row per group when everything is collapsed", () => {
        const rows = rowsOf(EXPENSES, NONE);

        expect(rows.map((r) => r.label)).toEqual(["Depreciation", "Food", "Housing", "Taxes", "Transport", "Travel", "Unknown", "Utilities"]);
        expect(rows.every((r) => r.kind === "group")).toBe(true);
        // Collapsed is the DEFAULT, and a collapsed group still has to be
        // cursorable — otherwise `j` on a freshly-loaded report does nothing at
        // all, because no account row exists yet.
        expect(rows).toHaveLength(8);
    });

    it("carries each group's own subtotal, not a row's balance", () => {
        // By label, not by position: Depreciation now sorts ahead of Food, and
        // the claim here is about Food's subtotal rather than about the first row.
        const food = rowsOf(EXPENSES, NONE).find((r) => r.label === "Food");

        expect(food?.amounts.current.get("$")).toEqual({m: 165438n, p: 2});
        expect(food?.amounts.prior?.get("$")).toEqual({m: 154635n, p: 2});
        expect(food?.account).toBeNull(); // a group heading is not an account
    });

    it("prints no ancestor roll-up above its own children — the duplicate this redesign removes", () => {
        // The old table printed the depth-1 `income` row, then each account under
        // it, then a "Total Revenues" footer repeating the same figure. Nothing
        // between the group heading and the section total may be that figure
        // again.
        const rows = rowsOf(REVENUE, ALL);

        // Groups sort by name, so Dividends leads — see the fixture's note.
        expect(rows.map((r) => r.account)).toEqual([null, "income:dividends", null, "income:salary"]);
        expect(rows.filter((r) => r.amounts.current.get("$")?.m === 3401000n)).toEqual([]);
    });

    it("expands one group without expanding its neighbours", () => {
        const rows = rowsOf(EXPENSES, only(isGroupKey("opex", "Food")));

        // Labels are relative to the displayed PARENT, as in every other report
        // table: the group's own root keeps its full path, its children are named
        // by the segments below it.
        expect(rows.map((r) => r.label)).toEqual([
            "Depreciation",
            "Food",
            "expenses:food",
            "groceries",
            "restaurants",
            "Housing",
            "Taxes",
            "Transport",
            "Travel",
            "Unknown",
            "Utilities",
        ]);
        // Index 1 is the expanded Food group; 0 and 5 are untouched neighbours on
        // either side of it, which is the claim — expanding one opens exactly one.
        expect(rows[1].expanded).toBe(true);
        expect(rows[0].expanded).toBe(false);
        expect(rows[5].expanded).toBe(false);
        expect(rows[5].kind).toBe("group"); // the neighbour is still just its heading
    });

    it("compresses single-child chains, testing BOTH periods before folding one", () => {
        // `expenses:housing` has one child and the same figure in both windows, so
        // the pair reads as one row rather than two lines carrying an identical
        // balance. Folding on the current period alone would drop a parent that
        // had postings of its own in the prior one.
        const rows = rowsOf(EXPENSES, only(isGroupKey("opex", "Housing")));
        const accounts = rows.filter((r) => r.kind === "account");

        expect(accounts.map((r) => r.account)).toEqual(["expenses:housing:rent"]);
        expect(accounts[0].label).toBe("expenses:housing:rent");
    });

    it("keeps a row that exists in only one period, with an explicit zero on the other side", () => {
        // `expenses:travel:flights` ran only in the current window and
        // `expenses:travel:activities` only in the prior one. The engine joins
        // over the union of keys precisely so neither disappears; an empty
        // `MixedAmount` is what "nothing here" looks like on the wire.
        const accounts = rowsOf(EXPENSES, only(isGroupKey("opex", "Travel"))).filter((r) => r.kind === "account");

        expect(accounts.map((r) => r.account)).toEqual(["expenses:travel", "expenses:travel:activities", "expenses:travel:flights", "expenses:travel:lodging"]);
        const flights = accounts.find((r) => r.account === "expenses:travel:flights");
        expect(flights?.amounts.current.get("$")).toEqual({m: 41280n, p: 2});
        expect(flights?.amounts.prior?.size).toBe(0);
    });

    it("indents account rows beneath their group", () => {
        const rows = rowsOf(EXPENSES, ALL);

        expect(rows.filter((r) => r.kind === "group").every((r) => r.indent === 0)).toBe(true);
        expect(rows.filter((r) => r.kind === "account").every((r) => r.indent >= 1)).toBe(true);
    });

    it("gives every row a key unique across the whole report", () => {
        const keys = REPORT.sections.flatMap((section) => rowsOf(section, ALL)).map((r) => r.key);
        expect(new Set(keys).size).toBe(keys.length);
    });

    it("marks a group with no rows unexpandable so it renders without a dead triangle", () => {
        const rowless: IsSection = {
            kind: "other",
            title: "Other income & expense",
            groups: [{name: "Rounding", source: "computed", rows: [], total: {current: new Map()}}],
            total: {current: new Map()},
            trailing: [],
        };
        const [row] = rowsOf(rowless, ALL);

        expect(row.expandable).toBe(false);
        expect(row.expanded).toBe(false);
    });

    it("renders a section with no groups as an empty list, not a throw", () => {
        const empty: IsSection = {kind: "tax", title: "Income taxes", groups: [], total: {current: new Map()}, trailing: []};
        expect(rowsOf(empty, ALL)).toEqual([]);
    });
});

describe("UNIT reports/ui/incomeStatementRows — pctChange", () => {
    const usd = (m: number, p: number): MixedAmount => new Map([["$", dec(m, p)]]);

    it("divides the exact Decs, never the rounded figures beside them", () => {
        // Expenses $28,626.48 against $28,516.71 = +0.3849%; net income $5,383.52
        // against $10,880.79 = −50.5228%. Both operands are exact here, and that
        // is the point: a percentage taken from the DISPLAYED strings agrees by
        // luck on a two-decimal journal and disagrees the moment the engine sends
        // a third place, which it does for anything valued at a price.
        expect(pctChange(usd(2862648, 2), usd(2851671, 2), "$")).toBe(0.4);
        expect(pctChange(usd(538352, 2), usd(1088079, 2), "$")).toBe(-50.5);
    });

    it("is unaffected by the operands' decimal places", () => {
        // The same values written to three and four places, as valued figures arrive.
        expect(pctChange(usd(28626480, 3), usd(2851671, 2), "$")).toBe(0.4);
        expect(pctChange(usd(2862648, 2), usd(285167100, 4), "$")).toBe(0.4);
    });

    it("rounds half away from zero, matching every money figure on the same row", () => {
        // ±5.25% is exactly ±52.5 tenths → ±5.3%, not the 5.2% a
        // round-half-to-even would give. The amounts beside it round the same way.
        expect(pctChange(usd(105250, 2), usd(100000, 2), "$")).toBe(5.3);
        expect(pctChange(usd(94750, 2), usd(100000, 2), "$")).toBe(-5.3);
    });

    it("keeps one decimal, including a trailing zero", () => {
        // $1,020.00 against $1,000.00 is 2.0%, not "2%"; no movement is 0.0%.
        expect(fmtPct(pctChange(usd(102000, 2), usd(100000, 2), "$"))).toBe("2.0%");
        expect(fmtPct(pctChange(usd(100000, 2), usd(100000, 2), "$"))).toBe("0.0%");
        // A line that went to zero fell by all of it.
        expect(fmtPct(pctChange(new Map(), usd(100000, 2), "$"))).toBe("-100.0%");
    });

    it("has NO percentage when the prior was zero, rather than a division artefact", () => {
        // Not 0%, not ∞, not NaN — growth from nothing has no ratio, and `—` is
        // the honest cell. Absent and an explicit zero are the same case.
        expect(pctChange(usd(10000, 2), new Map(), "$")).toBeNull();
        expect(pctChange(usd(10000, 2), usd(0, 2), "$")).toBeNull();
        expect(fmtPct(null)).toBe("—");
    });

    it("has none when there is no prior period at all", () => {
        // Compare off, or interval mode: `prior` is absent from every figure.
        expect(pctChange(usd(10000, 2), undefined, "$")).toBeNull();
        expect(amountsPct({current: usd(10000, 2)}, "$")).toBeNull();
    });

    it("has none for a journal with no base commodity, rather than picking one", () => {
        expect(pctChange(usd(10000, 2), usd(5000, 2), null)).toBeNull();
    });

    it("divides by the prior's MAGNITUDE, so the sign says which way the line moved", () => {
        // An `other` box going from a $100 net cost to a $50 one improved by half.
        expect(pctChange(usd(-5000, 2), usd(-10000, 2), "$")).toBe(50);
        expect(pctChange(usd(-15000, 2), usd(-10000, 2), "$")).toBe(-50);
    });

    it("signs a negative share with the SAME minus character money uses, and never a bare +", () => {
        // These two land in one row — `$-15,000.00 | -7.3%` — so asked against
        // `fmt` rather than against a literal: whichever side someone changes,
        // this fails. U+2212 here put two different minus characters an inch
        // apart on the same line.
        const moneyMinus = fmt("$", dec(-100, 2), new Map()).replace(/[$\d.,]/g, "");
        expect(fmtPct(-2.4)).toBe(`${moneyMinus}2.4%`);
        expect(fmtPct(-2.4)).toBe("-2.4%");
        // No forced `+` on growth, as in the QuickBooks % Change column.
        expect(fmtPct(73.9)).toBe("73.9%");
    });

    it("gives each line its own change against its own prior", () => {
        const pct = (label: string) => fmtPct(rowsOf(EXPENSES, NONE).find((r) => r.label === label)?.pct ?? null);
        expect(pct("Food")).toBe("7.0%"); // $1,654.38 against $1,546.35
        expect(pct("Utilities")).toBe("-11.3%"); // $669.16 against $754.11
        // `expenses:unknown` has no activity in the prior window: no change to state.
        expect(pct("Unknown")).toBe("—");
    });
});

describe("UNIT reports/ui/incomeStatementRows — the ladder", () => {
    it("has no rungs at all in simple form", () => {
        // Two boxes and a net income figure: the entire personal-finance
        // experience, and it requires no tags. An empty ladder is the default,
        // not a degraded state.
        const model = isDisplayModel(REPORT, NONE);

        expect(model.boxes.map((b) => b.kind)).toEqual(["revenue", "opex"]);
        expect(model.boxes.flatMap((b) => b.trailing)).toEqual([]);
        expect(REPORT.multiStep).toBe(false);
    });

    it("attaches each rung to the box it follows, in ladder order", () => {
        const model = isDisplayModel(MULTI, NONE);

        expect(model.boxes.map((b) => b.kind)).toEqual(["revenue", "cogs", "opex", "depreciation", "other", "interest", "tax"]);
        // A subtotal hangs off its section rather than floating in a list of its
        // own, so it can never be orphaned from the box it summarizes.
        expect(model.boxes.map((b) => b.trailing.map((s) => s.kind))).toEqual([[], ["grossProfit"], ["ebitda"], ["operatingIncome"], [], ["pretaxIncome"], []]);
    });

    it("makes every rung a running total of everything printed above it", () => {
        const model = isDisplayModel(MULTI, NONE);
        const rung = (kind: string) => model.boxes.flatMap((b) => b.trailing).find((s) => s.kind === kind);

        // EBITDA sits ABOVE D&A and Operating income below it, so no line is ever
        // the sum of things both above and below it.
        expect(rung("grossProfit")?.amounts.current.get("$")).toEqual({m: 51740000n, p: 2}); // 620,000 − 102,600
        expect(rung("ebitda")?.amounts.current.get("$")).toEqual({m: 16090000n, p: 2}); // − 356,500
        expect(rung("operatingIncome")?.amounts.current.get("$")).toEqual({m: 13690000n, p: 2}); // − 24,000
        expect(rung("pretaxIncome")?.amounts.current.get("$")).toEqual({m: 10950000n, p: 2}); // − 15,000 − 12,400
        expect(MULTI.netIncome.current.get("$")).toEqual({m: 8650000n, p: 2}); // − 23,000
    });

    it("states no change on any rung of a statement with no prior window", () => {
        // MULTI is uncompared, so there is nothing for any line to change FROM.
        const model = isDisplayModel(MULTI, NONE);
        const pcts = model.boxes.flatMap((b) => b.trailing).map((s) => fmtPct(s.pct));

        expect(pcts).toEqual(["—", "—", "—", "—"]);
    });

    it("lets the `other` box print a negative total, alone among the sections", () => {
        const model = isDisplayModel(MULTI, NONE);
        const other = model.boxes.find((b) => b.kind === "other");

        // A grant and a lawsuit settlement can share this box, so it is presented
        // as a net contribution to income rather than as a magnitude. Nothing
        // here negates it: the engine sent it negative and it prints negative.
        expect(other?.total.current.get("$")).toEqual({m: -1500000n, p: 2});
        // Every other box prints a positive magnitude, whichever way it moves
        // the bottom line.
        expect(model.boxes.filter((b) => (b.total.current.get("$")?.m ?? 0n) < 0n).map((b) => b.kind)).toEqual(["other"]);
    });

    it("titles opex by the shape of the statement, not by a rule of its own", () => {
        // The same section either way; only the label moves, so no account
        // changes box when a journal grows its first `cogs:` tag.
        expect(REPORT.sections.find((s) => s.kind === "opex")?.title).toBe("Expenses");
        expect(MULTI.sections.find((s) => s.kind === "opex")?.title).toBe("Operating expenses");
    });
});

describe("UNIT reports/ui/incomeStatementRows — isDisplayModel and the cursor list", () => {
    it("feeds the template and the cursor from the very same arrays", () => {
        const model = isDisplayModel(REPORT, only(isGroupKey("opex", "Food")));
        const cursorable = isCursorRows(model);

        // Not merely equal — identical. Two structurally-equal lists would drift
        // the first time one of them was rebuilt and the other was not.
        expect(cursorable).toHaveLength(model.boxes[0].rows.length + model.boxes[1].rows.length);
        expect(cursorable[0]).toBe(model.boxes[0].rows[0]);
        expect(cursorable.at(-1)).toBe(model.boxes[1].rows.at(-1));
    });

    it("leaves the ladder out of the cursor list", () => {
        // There is nothing to expand or drill into on a subtotal, so landing on
        // one with `j` would be a stop that does nothing on Enter.
        const model = isDisplayModel(MULTI, NONE);
        const labels = isCursorRows(model).map((r) => r.label);

        expect(model.boxes.flatMap((b) => b.trailing)).not.toHaveLength(0);
        expect(labels).not.toContain("Gross profit");
        expect(labels).not.toContain("EBITDA");
    });

    it("grows the cursor list as groups open", () => {
        expect(isCursorRows(isDisplayModel(REPORT, NONE))).toHaveLength(10); // 2 revenue + 8 expense groups
        expect(isCursorRows(isDisplayModel(REPORT, ALL)).length).toBeGreaterThan(10);
    });
});

describe("UNIT reports/ui/incomeStatementRows — isSummary", () => {
    it("carries the bottom line and its margin, and nothing else", () => {
        const summary = isSummary(REPORT);

        expect(summary.netIncome.current.get("$")).toEqual({m: 538352n, p: 2});
        expect(summary.netIncome.prior?.get("$")).toEqual({m: 1088079n, p: 2});
        expect(fmtPct(summary.netPct)).toBe("-50.5%");
        // The per-section restatement is gone on purpose. Every section total is
        // already in a box footer and every intermediate figure is already a rung
        // of the ladder, so repeating them was the duplicate-totals complaint
        // this redesign exists to fix, one panel further down.
        expect(Object.keys(summary).sort()).toEqual(["netIncome", "netPct"]);
    });

    it("takes net income's change against its OWN prior figure", () => {
        // The one arithmetic decision the screen and the workbook have to agree
        // on, and the reason this stayed a shared function rather than being
        // inlined at both call sites.
        expect(isSummary(REPORT).netPct).toBe(pctChange(REPORT.netIncome.current, REPORT.netIncome.prior, "$"));
    });

    it("leaves the change absent when the report is not comparing", () => {
        expect(isSummary(MULTI).netPct).toBeNull();
        expect(fmtPct(isSummary(MULTI).netPct)).toBe("—");
    });

    it("has no prior figure at all when the report is not comparing", () => {
        const uncompared = decodeIncomeStatementReport(UNCOMPARED_INCOME_STATEMENT);

        expect(uncompared.prior).toBeNull();
        expect(isDisplayModel(uncompared, NONE).comparing).toBe(false);
        expect(isSummary(uncompared).netIncome.prior).toBeUndefined();
        // Absent, never a zero: a zero would be a claim about a period that was
        // never computed.
        expect(isCursorRows(isDisplayModel(uncompared, ALL)).every((r) => r.amounts.prior === undefined)).toBe(true);
    });

    it("hands the model the very same object, so the panel and the workbook cannot diverge", () => {
        const model = isDisplayModel(MULTI, NONE);

        expect(model.summary.netIncome).toBe(MULTI.netIncome);
        expect(model.summary).toEqual(isSummary(MULTI));
    });
});

// The display model over the REAL engine bytes, not a hand-written mirror of
// them. `fixtures/native/v1/incomestatement-grouped.json` is what a live engine
// answered for `fixtures/sample.journal`, byte-pinned on the Rust side by
// native_wire_golden.rs.
//
// It buys one thing the literals above cannot: MIXED PRECISION. The engine sends
// revenue at `places: 2` and expenses and net income at `places: 4`, because a
// valued figure keeps the precision the multiplication produced. Every hand
// fixture in this file writes cents on both sides of every ratio, so the
// percentage column is only ever exercised at equal precision here — which is
// the one case where a places-blind implementation would also pass.
describe("UNIT reports/ui/incomeStatementRows — over the committed engine golden", () => {
    const GOLDEN = golden("incomestatement-grouped");

    it("takes the change at the precision the engine actually sends", () => {
        const expenses = GOLDEN.sections.find((s) => s.kind === "opex")?.total;

        // $28,626.4800 against $28,516.7100, both at places 4 — the shapes the
        // engine sends, not the cents-on-both-sides the literals use.
        expect(expenses?.current.get("$")?.p).toBe(4);
        expect(fmtPct(expenses === undefined ? null : amountsPct(expenses, "$"))).toBe("0.4%");
        expect(fmtPct(isSummary(GOLDEN).netPct)).toBe("-50.5%");
    });

    it("renders the same figures the hand-written fixture claims", () => {
        // If these ever disagree, the literals above have drifted from the wire
        // and it is the literals that are wrong.
        const model = isDisplayModel(GOLDEN, NONE);
        const hand = isDisplayModel(REPORT, NONE);

        expect(model.boxes.map((b) => [b.kind, b.title, b.totalPct])).toEqual(hand.boxes.map((b) => [b.kind, b.title, b.totalPct]));
        expect(model.boxes.flatMap((b) => b.rows.map((r) => r.label))).toEqual(hand.boxes.flatMap((b) => b.rows.map((r) => r.label)));
        expect(isSummary(GOLDEN).netPct).toBe(isSummary(REPORT).netPct);
    });

    it("agrees with the hledger CLI on every figure the fixture header quotes", () => {
        // The provenance chain, closed: hledger printed these, the fixture header
        // records them, the engine produced the golden, and this is the display
        // model reading the golden. Verified against hledger 1.52,
        // `is -V -b 2026-01-01 -e 2026-07-09 --depth 2`.
        const box = (kind: string) => isDisplayModel(GOLDEN, NONE).boxes.find((b) => b.kind === kind);

        expect(box("revenue")?.total.current.get("$")).toMatchObject({m: 3401000n}); // $34,010.00
        expect(box("opex")?.rows.map((r) => r.label)).toEqual(["Depreciation", "Food", "Housing", "Taxes", "Transport", "Travel", "Unknown", "Utilities"]);
        expect(fmtPct(box("revenue")?.totalPct ?? null)).toBe("-13.7%"); // against $39,397.50
        expect(fmtPct(box("opex")?.totalPct ?? null)).toBe("0.4%");
    });
});

// Interval mode (plans/24), over the engine's own monthly golden:
// `?from=2026-01-01&to=2026-07-08&interval=monthly&compare=none`.
describe("UNIT reports/ui/incomeStatementRows — interval mode", () => {
    const MONTHLY = golden("incomestatement-grouped-monthly");

    it("hands the view its buckets and no comparison", () => {
        const model = isDisplayModel(MONTHLY, NONE);
        expect(model.buckets?.map((b) => b.key)).toEqual(["2026-01", "2026-02", "2026-03", "2026-04", "2026-05", "2026-06", "2026-07"]);
        // The last column stops at the report's `to`, not at the month's end.
        expect(model.buckets?.at(-1)).toMatchObject({from: "2026-07-01", to: "2026-07-08"});
        expect(model.comparing).toBe(false);
    });

    it("has no % change anywhere, because there is no prior period", () => {
        const model = isDisplayModel(MONTHLY, ALL);
        expect(isCursorRows(model).every((r) => r.pct === null)).toBe(true);
        expect(model.boxes.every((b) => b.totalPct === null)).toBe(true);
        expect(model.summary.netPct).toBeNull();
    });

    it("gives every line one figure per bucket, its Total the same net income as the Total statement", () => {
        const model = isDisplayModel(MONTHLY, ALL);
        expect(isCursorRows(model).every((r) => r.amounts.values?.length === 7)).toBe(true);
        // Market-valued, but nothing on this journal is revalued between the
        // month ends and the range's end, so the sum of the months IS the
        // whole-range figure: $5,383.52.
        expect(fmt("$", model.summary.netIncome.current.get("$") ?? dec(0, 0), new Map())).toBe(fmt("$", dec(538352, 2), new Map()));
    });
});
