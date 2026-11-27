// A literal statement of cash flows for the web tests, built by hand rather
// than through the decoder (the discipline `qbJournalModel.test.ts` states: a
// test cannot pass because a bug in the decoder and one in the code under test
// cancel out).
//
// Its figures are the engine fixture's (`fixtures/reports/cf-statement.journal`,
// Q1 2026 monthly), so the two test suites describe one business.

import {dec, type MixedAmount} from "#lib/domain/money.js";
import type {CashFlowStatement, CfCashRow, CfColumns, CfLine, CfRow} from "#lib/reports/types.js";

export const usd = (n: number): MixedAmount => (n === 0 ? new Map() : new Map([["$", dec(Math.round(n * 100), 2)]]));

/** A flow: the Total is the sum of the months. */
export function flow(values: number[]): CfColumns {
    return {values: values.map(usd), total: usd(values.reduce((a, b) => a + b, 0))};
}

/** A balance: the Total is the first month's (beginning) or the last month's (ending). */
export function balance(values: number[], edge: "first" | "last"): CfColumns {
    return {values: values.map(usd), total: usd(edge === "first" ? values[0] : values[values.length - 1])};
}

/** An outline row as the engine sends it (plan 50): a leaf, at level 0 unless said. */
export function cfRow(account: string, label: string, amounts: CfColumns, level = 0): CfRow {
    return {account, kind: "account", label, level, amounts};
}

function cashRow(account: string, label: string, beginning: CfColumns, ending: CfColumns): CfCashRow {
    return {account, kind: "account", label, level: 0, beginning, ending};
}

/** A line that is one account at the report's depth, labelled as the engine labels it. */
function accountLine(account: string, label: string, values: number[]): CfLine {
    return {name: account, source: "account", label, account, total: flow(values), rows: [cfRow(account, label, flow(values))]};
}

const BUCKETS = [
    {key: "2026-01", label: "Jan 2026", from: "2026-01-01", to: "2026-01-31"},
    {key: "2026-02", label: "Feb 2026", from: "2026-02-01", to: "2026-02-28"},
    {key: "2026-03", label: "Mar 2026", from: "2026-03-01", to: "2026-03-31"},
] as CashFlowStatement["buckets"];

export const CASH_FLOW: CashFlowStatement = {
    kind: "cashFlow",
    from: "2026-01-01",
    to: "2026-03-31",
    buckets: BUCKETS,
    netIncome: flow([3500, -400, 0]),
    sections: [
        {
            kind: "operating",
            title: "Operating activities",
            lines: [
                accountLine("assets:equipment:accum-depreciation", "Accum Depreciation", [0, 100, 0]),
                accountLine("assets:prepaid", "Prepaid", [-1200, 0, 0]),
                accountLine("assets:receivable", "Receivable", [-5000, 5000, 0]),
                {
                    name: "Payables",
                    source: "group",
                    label: "Payables",
                    account: null,
                    total: flow([200, 300, -200]),
                    rows: [cfRow("liabilities:card", "Card", flow([200, 0, -200])), cfRow("liabilities:payable", "Payable", flow([0, 300, 0]))],
                },
                accountLine("liabilities:deferred-revenue", "Deferred Revenue", [2200, 0, 0]),
            ],
            total: flow([-300, 5000, -200]),
        },
        {kind: "investing", title: "Investing activities", lines: [accountLine("assets:equipment", "Equipment", [0, -3000, 0])], total: flow([0, -3000, 0])},
        {kind: "financing", title: "Financing activities", lines: [accountLine("liabilities:note", "Note", [0, 0, 8000])], total: flow([0, 0, 8000])},
    ],
    exchange: flow([0, 0, 0]),
    netChange: flow([-300, 2000, 7800]),
    cash: [
        {
            name: "Operating cash",
            source: "group",
            label: "Operating cash",
            account: null,
            beginning: balance([10000, 9700, 11700], "first"),
            ending: balance([9700, 11700, 19450], "last"),
            rows: [
                cashRow("assets:bank:checking", "Checking", balance([10000, 9700, 11700], "first"), balance([9700, 11700, 11450], "last")),
                cashRow("assets:bank:savings", "Savings", balance([0, 0, 0], "first"), balance([0, 0, 8000], "last")),
            ],
        },
        {
            name: "assets:petty-cash",
            source: "account",
            label: "Petty Cash",
            account: "assets:petty-cash",
            beginning: balance([0, 0, 0], "first"),
            ending: balance([0, 0, 50], "last"),
            rows: [cashRow("assets:petty-cash", "Petty Cash", balance([0, 0, 0], "first"), balance([0, 0, 50], "last"))],
        },
    ],
    beginningCash: balance([10000, 9700, 11700], "first"),
    endingCash: balance([9700, 11700, 19500], "last"),
    reconciles: true,
    maxDepth: 3,
};

/** The same statement as a single Total column (Interval = Total). */
export const CASH_FLOW_TOTAL: CashFlowStatement = {
    ...CASH_FLOW,
    buckets: [{key: "2026-01-01..2026-03-31", label: "2026-01-01 to 2026-03-31", from: "2026-01-01", to: "2026-03-31"}] as CashFlowStatement["buckets"],
    netIncome: flow([3100]),
    sections: CASH_FLOW.sections.map((section) => ({
        ...section,
        lines: section.lines.map((line) => ({
            ...line,
            total: {values: [line.total.total], total: line.total.total},
            rows: line.rows.map((row) => ({...row, amounts: {values: [row.amounts.total], total: row.amounts.total}})),
        })),
        total: {values: [section.total.total], total: section.total.total},
    })),
    exchange: flow([0]),
    netChange: flow([9500]),
    cash: CASH_FLOW.cash.map((line) => ({
        ...line,
        beginning: {values: [line.beginning.total], total: line.beginning.total},
        ending: {values: [line.ending.total], total: line.ending.total},
        rows: line.rows.map((row) => ({
            ...row,
            beginning: {values: [row.beginning.total], total: row.beginning.total},
            ending: {values: [row.ending.total], total: row.ending.total},
        })),
    })),
    beginningCash: balance([10000], "first"),
    endingCash: balance([19500], "last"),
};
