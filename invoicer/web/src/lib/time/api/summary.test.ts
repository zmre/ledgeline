import {describe, expect, it} from "vitest";
import {isSummary, summaryQueryString} from "./summary.js";

const qty = {seconds: 60, hours: "0.0166", text: "1m"};

describe("UNIT time summary API", () => {
    it("writes the request in a fixed order, booleans as the server reads them", () => {
        expect(
            summaryQueryString({
                from: "2026-11-01",
                to: "2026-11-30",
                person: "",
                customer: "Acme Corp",
                billable: false,
                account: "clients",
                group: ["account", "day"],
                depth: 2,
                top: 10,
                series: "week",
            })
        ).toBe("from=2026-11-01&to=2026-11-30&person=&customer=Acme+Corp&billable=false&account=clients&group=account%2Cday&depth=2&top=10&series=week");
        expect(summaryQueryString({group: []})).toBe("");
    });

    it("accepts a summary, with or without series, and refuses the tree view or a bad row", () => {
        const ok = {
            rows: [{key: ["a"], quantity: qty, entries: 1, series: [["2026-11", qty]]}],
            other: null,
            total: qty,
            buckets: ["2026-11"],
            file: "time.journal",
        };
        expect(isSummary(ok)).toBe(true);
        expect(isSummary({...ok, other: {key: ["other"], quantity: qty, entries: 3}})).toBe(true);
        expect(isSummary({tree: []})).toBe(false);
        expect(isSummary({...ok, rows: [{key: [1], quantity: qty, entries: 1}]})).toBe(false);
        expect(isSummary({...ok, file: undefined})).toBe(false);
    });
});
