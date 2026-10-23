// The statement of cash flows' display model (plans/27): what folds, what
// starts open, and that the chart draws exactly the statement's section totals.

import {describe, expect, it} from "vitest";
import {CASH_FLOW, CASH_FLOW_TOTAL, flow} from "#lib/testing/cashFlowFixture.js";
import {cashFlowChartReport, cfCursorRows, cfDisplayModel, cfFoldKey, type IsOpen} from "./cashFlowRows";

const defaults: IsOpen = (_, startsOpen) => startsOpen;
const labels = (rows: {label: string}[]): string[] => rows.map((row) => row.label);

describe("UNIT cfDisplayModel — sections", () => {
    it("heads Operating with net income, then its lines", () => {
        const model = cfDisplayModel(CASH_FLOW, defaults);
        expect(model.boxes.map((box) => box.kind)).toEqual(["operating", "investing", "financing"]);
        expect(labels(model.boxes[0].rows)).toEqual([
            "Net income",
            "assets:equipment:accum-depreciation",
            "assets:prepaid",
            "assets:receivable",
            "Payables",
            "liabilities:deferred-revenue",
        ]);
        expect(model.boxes[0].totalLabel).toBe("Net cash from operating activities");
    });

    it("folds a group line shut, and opens it to its accounts", () => {
        const shut = cfDisplayModel(CASH_FLOW, defaults).boxes[0].rows.find((row) => row.label === "Payables");
        expect(shut).toMatchObject({expandable: true, expanded: false, account: null});

        const model = cfDisplayModel(CASH_FLOW, (key, startsOpen) => key === "operating/group/Payables" || startsOpen);
        const rows = model.boxes[0].rows;
        const at = rows.findIndex((row) => row.label === "Payables");
        expect(rows.slice(at + 1, at + 3).map((row) => [row.label, row.account, row.indent])).toEqual([
            ["liabilities:card", "liabilities:card", 1],
            ["liabilities:payable", "liabilities:payable", 1],
        ]);
    });

    it("does not offer to open a line that is exactly one account, and drills straight to it", () => {
        const row = cfDisplayModel(CASH_FLOW, defaults).boxes[0].rows.find((r) => r.label === "assets:prepaid");
        expect(row).toMatchObject({expandable: false, account: "assets:prepaid"});
    });

    it("shows the effect of exchange rates only when it is not zero", () => {
        expect(cfDisplayModel(CASH_FLOW, defaults).exchange).toBeNull();
        const withFx = {...CASH_FLOW, exchange: flow([0, 10, 0])};
        expect(cfDisplayModel(withFx, defaults).exchange).not.toBeNull();
    });

    it("has interval columns only when there is more than one", () => {
        expect(cfDisplayModel(CASH_FLOW, defaults).buckets).toHaveLength(3);
        expect(cfDisplayModel(CASH_FLOW_TOTAL, defaults).buckets).toBeNull();
    });
});

describe("UNIT cfDisplayModel — the cash balances", () => {
    it("starts with the cash detail open, under both Beginning and Ending cash", () => {
        const model = cfDisplayModel(CASH_FLOW, defaults);
        expect(labels(model.beginning.rows)).toEqual(["Operating cash", "assets:bank:checking", "assets:bank:savings", "assets:petty-cash"]);
        expect(labels(model.ending.rows)).toEqual(labels(model.beginning.rows));
        expect(model.ending.rows[0].expanded).toBe(true);
    });

    it("shares one fold between the two blocks, so they always line up", () => {
        const model = cfDisplayModel(CASH_FLOW, defaults);
        expect(cfFoldKey(model.beginning.rows[0])).toBe(cfFoldKey(model.ending.rows[0]));
        const shut = cfDisplayModel(CASH_FLOW, (key, startsOpen) => (key === cfFoldKey(model.beginning.rows[0]) ? false : startsOpen));
        expect(labels(shut.beginning.rows)).toEqual(["Operating cash", "assets:petty-cash"]);
        expect(labels(shut.ending.rows)).toEqual(["Operating cash", "assets:petty-cash"]);
    });

    it("reads a cash account's own name relative to its line, and its figures as balances", () => {
        const model = cfDisplayModel(CASH_FLOW, defaults);
        const checking = model.ending.rows[1];
        expect(checking.account).toBe("assets:bank:checking");
        expect(checking.amounts.total.get("$")).toEqual({m: 1145000n, p: 2});
    });
});

describe("UNIT cfCursorRows", () => {
    it("walks every visible row: sections, then beginning cash, then ending cash", () => {
        const model = cfDisplayModel(CASH_FLOW, defaults);
        const rows = cfCursorRows(model);
        expect(rows[0].label).toBe("Net income");
        expect(rows).toHaveLength(model.boxes.flatMap((box) => box.rows).length + model.beginning.rows.length + model.ending.rows.length);
        expect(new Set(rows.map((row) => row.key)).size).toBe(rows.length);
    });
});

describe("UNIT cashFlowChartReport", () => {
    it("is one series per activity, with net change in cash as the net", () => {
        const chart = cashFlowChartReport(CASH_FLOW);
        expect(chart.rows.map((row) => row.account)).toEqual(["Operating", "Investing", "Financing"]);
        expect(chart.rows[1].values.map((value) => value.get("$")?.m ?? 0n)).toEqual([0n, -300000n, 0n]);
        expect(chart.totals).toBe(CASH_FLOW.netChange.values);
        expect(chart.periods.map((period) => period.label)).toEqual(["Jan 2026", "Feb 2026", "Mar 2026"]);
    });
});

describe("UNIT cfDisplayModel — a rolled-up line that is one account", () => {
    // The reported case: depth 3 rolls `assets:fidelity:ira:cash` up to
    // `assets:fidelity:ira`, and nothing else is under it. It printed as a
    // pre-opened disclosure hiding exactly one row.
    const lone = {
        ...CASH_FLOW,
        cash: [
            {
                name: "assets:fidelity:ira",
                source: "account" as const,
                beginning: CASH_FLOW.cash[1].beginning,
                ending: CASH_FLOW.cash[1].ending,
                rows: [{account: "assets:fidelity:ira:cash", beginning: CASH_FLOW.cash[1].beginning, ending: CASH_FLOW.cash[1].ending}],
            },
        ],
        sections: [
            {
                ...CASH_FLOW.sections[0],
                lines: [
                    {
                        name: "assets:fixed",
                        source: "account" as const,
                        total: flow([0, 100, 0]),
                        rows: [{account: "assets:fixed:accum-depreciation", amounts: flow([0, 100, 0])}],
                    },
                ],
            },
        ],
    };

    it("prints the whole account, with nothing to open, under the cash balances", () => {
        const model = cfDisplayModel(lone, defaults);
        expect(model.beginning.rows.map((row) => [row.label, row.expandable, row.account])).toEqual([
            ["assets:fidelity:ira:cash", false, "assets:fidelity:ira:cash"],
        ]);
        expect(model.ending.rows).toHaveLength(1);
    });

    it("does the same in a section", () => {
        const row = cfDisplayModel(lone, defaults).boxes[0].rows[1];
        expect([row.label, row.expandable, row.account]).toEqual(["assets:fixed:accum-depreciation", false, "assets:fixed:accum-depreciation"]);
    });

    it("keeps a cfgroup line's own name and arrow even with one account behind it", () => {
        const grouped = {...lone, cash: [{...lone.cash[0], name: "Retirement", source: "group" as const}]};
        const [head] = cfDisplayModel(grouped, defaults).beginning.rows;
        expect([head.label, head.expandable]).toEqual(["Retirement", true]);
    });
});
