// The accrual UI's pure helpers (plan 45 Phase 3): the Post-through date, the
// source posting's tags, fresh ids, and specs on the way to the wire.

import {describe, expect, it} from "vitest";
import {
    checkState,
    commentTag,
    dayShort,
    defaultDescription,
    draftToWire,
    endOfMonth,
    entryKey,
    journalMonthHref,
    monthShort,
    newScheduleId,
    postThroughDate,
    sourceTags,
    specToDraft,
    termLabel,
    withCommentTags,
    type ScheduleDraft,
} from "./model";
import type {ScheduleSpec} from "./types";

const draft = (overrides: Partial<ScheduleDraft> = {}): ScheduleDraft => ({
    id: "k7f3a2",
    txnDate: "2026-11-17",
    start: "2026-11-17",
    term: {months: 12},
    proration: "anniversary",
    contract: "new",
    party: {kind: "customer", name: "Customer X"},
    description: "Customer X Recognized",
    lines: [{sourceAccount: "liabilities:deferred-revenue", targetAccount: "revenue:sales", commodity: "$", quantity: "-12000.00"}],
    ...overrides,
});

describe("UNIT accrual model: dates", () => {
    it("ends months, leap years included", () => {
        expect(endOfMonth("2026-11")).toBe("2026-11-30");
        expect(endOfMonth("2028-02")).toBe("2028-02-29");
        expect(endOfMonth("2026-12")).toBe("2026-12-31");
    });

    it("posts through the latest checked month's end, capped at today", () => {
        expect(postThroughDate(["2026-09", "2026-10"], "2026-11-17")).toBe("2026-10-31");
        expect(postThroughDate(["2026-11", "2026-10"], "2026-11-17")).toBe("2026-11-17");
        expect(postThroughDate([], "2026-11-17")).toBeNull();
    });

    it("labels months and days", () => {
        expect(monthShort("2026-11")).toBe("Nov 2026");
        expect(dayShort("2026-11-07")).toBe("Nov 7");
        expect(journalMonthHref("2026-02")).toBe("/?from=2026-02-01&to=2026-02-28&expand=2026-02");
    });
});

describe("UNIT accrual model: checks and keys", () => {
    it("is none, some or all", () => {
        const keys = [entryKey({id: "a", recognized: "2026-10-01"}), entryKey({id: "b", recognized: "2026-10-01"})];
        expect(keys[0]).toBe("a@2026-10-01");
        expect(checkState(keys, new Set())).toBe("none");
        expect(checkState(keys, new Set([keys[0]!]))).toBe("some");
        expect(checkState(keys, new Set(keys))).toBe("all");
    });
});

describe("UNIT accrual model: the source posting's tags", () => {
    it("writes party, id, term, proration and contract, and service-start only when it moves", () => {
        expect(sourceTags(draft())).toEqual([
            ["customer", "Customer X"],
            ["recognition", "k7f3a2"],
            ["term-months", "12"],
            ["contract", "new"],
        ]);
        expect(sourceTags(draft({start: "2026-12-01", term: {through: "2027-11-30"}, proration: "calendar", contract: null, party: null}))).toEqual([
            ["recognition", "k7f3a2"],
            ["service-start", "2026-12-01"],
            ["service-end", "2027-11-30"],
            ["proration", "calendar"],
        ]);
    });

    it("sets tags on a comment, replacing the old schedule tags and keeping everything else", () => {
        const before = "first payment, recognition: old1, term-months: 6, invoice: 1042\nsecond line, contract: renewal";
        const after = withCommentTags(before, sourceTags(draft()));
        expect(after).toBe("first payment, invoice: 1042, customer: Customer X, recognition: k7f3a2, term-months: 12, contract: new\nsecond line");
        expect(commentTag(after, "recognition")).toBe("k7f3a2");
        expect(commentTag(after, "invoice")).toBe("1042");
        expect(commentTag(after, "service-start")).toBeNull();
        expect(withCommentTags("", [["recognition", "x"]])).toBe("recognition: x");
    });
});

describe("UNIT accrual model: ids and specs", () => {
    it("draws six-character ids starting with a letter, avoiding taken ones", () => {
        let n = 0;
        const sequence = [0, 0, 0, 0, 0, 0, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5];
        const random = () => sequence[n++ % sequence.length] ?? 0;
        expect(newScheduleId(new Set(["aaaaaa"]), random)).toBe("nsssss");
        for (let i = 0; i < 50; i += 1) expect(newScheduleId()).toMatch(/^[a-z][a-z0-9]{5}$/);
    });

    it("sends a draft as the save route takes it", () => {
        expect(draftToWire(draft())).toEqual({
            id: "k7f3a2",
            lines: [
                {
                    sourceAccount: "liabilities:deferred-revenue",
                    targetAccount: "revenue:sales",
                    commodity: "$",
                    quantity: "-12000.00",
                    party: {kind: "customer", name: "Customer X"},
                },
            ],
            start: "2026-11-17",
            term: {months: 12},
            proration: "anniversary",
            description: "Customer X Recognized",
        });
        expect(draftToWire(draft({party: {kind: "vendor", name: "  "}, term: {through: "2027-01-31"}})).lines[0]).not.toHaveProperty("party");
    });

    it("reads a served spec back as a draft", () => {
        const spec: ScheduleSpec = {
            id: "cx1217",
            lines: [
                {
                    sourceAccount: "liabilities:deferred-revenue",
                    targetAccount: "revenue:sales",
                    total: {commodity: "$", quantity: {m: -1200000n, p: 2}},
                    party: {kind: "customer", name: "Customer X"},
                },
            ],
            start: "2025-12-17",
            term: {months: 12},
            proration: "anniversary",
            description: "Customer X Recognized",
            headerTags: [],
        };
        const back = specToDraft(spec, "2025-12-17", "renewal");
        expect(back.lines[0]?.quantity).toBe("-12000.00");
        expect(back.party).toEqual({kind: "customer", name: "Customer X"});
        expect(back.contract).toBe("renewal");
    });

    it("labels terms and descriptions", () => {
        expect(termLabel({months: 1})).toBe("1 month");
        expect(termLabel({through: "2027-01-31"})).toBe("through 2027-01-31");
        expect(defaultDescription({kind: "customer", name: "Customer X"}, "Payment")).toBe("Customer X Recognized");
        expect(defaultDescription(null, "Northwind Insurance")).toBe("Northwind Insurance Recognized");
    });
});
