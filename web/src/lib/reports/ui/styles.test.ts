import {describe, expect, it} from "vitest";
import {dec} from "$lib/domain/money";
import type {AmountStyle, Transaction} from "$lib/domain/types";
import {reportStyles} from "./styles";

const style = (precision: number): AmountStyle => ({side: "R", spaced: true, precision, decimalPoint: ".", digitGroups: null});

function txn(index: number, amounts: {commodity: string; p: number; stylePrecision?: number}[]): Transaction {
    return {
        index,
        date: "2026-01-01",
        status: "unmarked",
        description: "t",
        code: "",
        comment: "",
        tags: [],
        haystack: "",
        postings: amounts.map(({commodity, p, stylePrecision}) => ({
            account: "assets:x",
            amounts: [{commodity, qty: dec(105n, p), style: style(stylePrecision ?? p)}],
            status: "unmarked" as const,
            comment: "",
            tags: [],
        })),
    };
}

describe("UNIT reports/ui/styles", () => {
    it("raises first-seen precision to the max actual decimal places seen", () => {
        const txns = [txn(1, [{commodity: "AAPL", p: 0}]), txn(2, [{commodity: "AAPL", p: 1}])];
        expect(reportStyles(txns).get("AAPL")?.precision).toBe(1);
    });

    it("uses actual decimal places even when the declared style precision is lower", () => {
        const txns = [txn(1, [{commodity: "X", p: 4, stylePrecision: 0}])];
        expect(reportStyles(txns).get("X")?.precision).toBe(4);
    });

    it("ignores inflated style precisions on later amounts (wire quirk of cost postings)", () => {
        const txns = [txn(1, [{commodity: "$", p: 2}]), txn(2, [{commodity: "$", p: 0, stylePrecision: 4}])];
        expect(reportStyles(txns).get("$")?.precision).toBe(2);
    });

    it("never lowers the first-seen style precision", () => {
        const txns = [txn(1, [{commodity: "$", p: 0, stylePrecision: 2}])];
        expect(reportStyles(txns).get("$")?.precision).toBe(2);
    });

    it("keeps the first-seen style object when nothing needs raising", () => {
        const txns = [txn(1, [{commodity: "$", p: 2}]), txn(2, [{commodity: "$", p: 2}])];
        expect(reportStyles(txns).get("$")).toEqual(style(2));
    });
});
