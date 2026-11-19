// The accrual decoders (plan 45 Phase 3) against Phase 2's wire shapes:
// `{enabled: false}` is null (the helpers are off), every field is checked,
// and exact decimals stay exact.

import {describe, expect, it} from "vitest";
import {ApiShapeError} from "#lib/api/client.js";
import {
    accrualStatusWire,
    dueEntryWire,
    dueMonthWire,
    dueWire,
    needsWire,
    proposalWire,
    settlementsWire,
    sideWire,
    tieWire,
} from "#lib/testing/accrualFixture.js";
import {
    decodeAccrualStatus,
    decodeClosedThrough,
    decodeCreatedFile,
    decodeDue,
    decodeNeeds,
    decodePostResult,
    decodeSavePlan,
    decodeSettleLink,
    decodeSettleLinkAll,
    decodeSettlements,
    decodeTransactionSchedules,
} from "./decode";

const OFF = {enabled: false};

const specWire = (id: string) => ({
    id,
    lines: [
        {
            sourceAccount: "liabilities:deferred-revenue",
            targetAccount: "revenue:sales",
            total: {commodity: "$", quantity: {m: "-1200000", p: 2}},
            party: {kind: "customer", name: "Customer X"},
        },
    ],
    start: "2025-12-17",
    term: {months: 12},
    proration: "anniversary",
    description: "Customer X Recognized",
    headerTags: [["renewal-date", "2026-12-17"]],
});

const scheduleWire = (id: string, status?: string) => ({
    spec: specWire(id),
    periods: [{date: "2025-12-17", part: "regular", amounts: [{m: "-100000", p: 2}], ...(status === undefined ? {} : {status, entry: 7})}],
    rules: [{role: "main", period: "~ monthly from 2025-12-17 to 2026-12-17", text: "~ monthly from 2025-12-17 to 2026-12-17  Customer X Recognized\n"}],
    renewalDate: "2026-12-17",
});

describe("UNIT accrual decode: the gate", () => {
    it("reads every route's {enabled: false} as null", () => {
        expect(decodeAccrualStatus(OFF)).toBeNull();
        expect(decodeDue(OFF)).toBeNull();
        expect(decodeNeeds(OFF)).toBeNull();
        expect(decodeSettlements(OFF)).toBeNull();
        expect(decodeTransactionSchedules(OFF)).toBeNull();
        expect(decodeSavePlan(OFF)).toBeNull();
    });

    it("decodes the status: roles as a map, the file, closed-through and counts", () => {
        const status = decodeAccrualStatus(accrualStatusWire());
        expect(status?.roles.get("assets:prepaid")).toEqual({role: "prepaid", declared: true});
        expect(status?.file).toEqual({path: "recognition.journal", exists: true, included: false});
        expect(status?.closedThrough).toBe("2026-09-30");
        expect(status?.counts).toEqual({due: 3, dueClosed: 1, needsSchedule: 1, invoices: 2, invoicesCheck: 1});
        expect(status?.reason).toEqual({kind: "declared", account: "liabilities:deferred-revenue", role: "deferred-revenue"});
        expect(status?.problem).toBeNull();
    });

    it("refuses an unknown role, a missing count and a non-boolean enabled", () => {
        expect(() => decodeAccrualStatus(accrualStatusWire({roles: {x: {role: "loan", declared: true}}}))).toThrow(ApiShapeError);
        expect(() => decodeAccrualStatus(accrualStatusWire({counts: {due: 1}}))).toThrow(ApiShapeError);
        expect(() => decodeAccrualStatus({enabled: "yes"})).toThrow(ApiShapeError);
    });
});

describe("UNIT accrual decode: due and post", () => {
    it("decodes months, groups, ties and the closed section exactly", () => {
        const due = decodeDue(
            dueWire(
                [dueMonthWire("2026-10", [{party: "Customer X", entries: [dueEntryWire("cx1217", "2026-10-17", 100000)]}], [tieWire(true)])],
                [dueEntryWire("cx1217", "2026-09-17", 83333)]
            )
        );
        const month = due?.months[0];
        expect(month?.month).toBe("2026-10");
        expect(month?.groups[0]?.party).toEqual({kind: "customer", name: "Customer X"});
        expect(month?.groups[0]?.entries[0]?.postings[0]?.amounts[0]?.quantity).toEqual({m: 100000n, p: 2});
        expect(month?.ties[0]?.ties).toBe(true);
        expect(due?.closed.count).toBe(1);
        expect(due?.closed.entries[0]?.recognized).toBe("2026-09-17");
    });

    it("refuses a month that isn't YYYY-MM", () => {
        expect(() => decodeDue(dueWire([dueMonthWire("Oct 2026", [])]))).toThrow(/YYYY-MM/);
    });

    it("decodes what a post wrote", () => {
        expect(decodePostResult({enabled: true, generation: "g2", posted: [{id: "a", recognized: "2026-11-01"}], adjustments: ["b"]})).toEqual({
            generation: "g2",
            posted: [{id: "a", recognized: "2026-11-01"}],
            adjustments: ["b"],
        });
        expect(() => decodePostResult({generation: "g2", posted: [{id: 1}], adjustments: []})).toThrow(ApiShapeError);
    });
});

describe("UNIT accrual decode: settlements and needs", () => {
    it("decodes proposals with their fit and the open sides", () => {
        const view = decodeSettlements(
            settlementsWire(
                [
                    proposalWire(sideWire(1), sideWire(2, {account: "assets:receivable", invoice: undefined}), {
                        fit: {kind: "short", difference: {m: "3000", p: 2}},
                        clear: false,
                        feeAccount: "expenses:fees",
                    }),
                ],
                {
                    invoices: [sideWire(5, {fingerprint: undefined, party: undefined})],
                }
            )
        );
        const proposal = view?.proposals[0];
        expect(proposal?.fit).toEqual({kind: "short", difference: {m: 3000n, p: 2}});
        expect(proposal?.payment.invoice).toBeNull();
        expect(proposal?.feeAccount).toBe("expenses:fees");
        // `early` is additive: absent reads as false, present must be a boolean.
        expect(proposal?.early).toBe(false);
        const early = decodeSettlements(settlementsWire([proposalWire(sideWire(1), sideWire(2), {early: true, gapDays: 3})]));
        expect(early?.proposals[0]?.early).toBe(true);
        expect(() => decodeSettlements(settlementsWire([proposalWire(sideWire(1), sideWire(2), {early: "yes"})]))).toThrow(ApiShapeError);
        expect(view?.counts).toEqual({clear: 0, check: 1});
        expect(view?.open.invoices[0]).toMatchObject({index: 5, fingerprint: null, party: null});
    });

    it("refuses an unknown fit and an index below 1", () => {
        expect(() => decodeSettlements(settlementsWire([proposalWire(sideWire(1), sideWire(2), {fit: {kind: "maybe"}})]))).toThrow(ApiShapeError);
        expect(() => decodeNeeds(needsWire([sideWire(0)]))).toThrow(/invalid index/);
    });

    it("decodes Needs a schedule with its lookback", () => {
        expect(decodeNeeds(needsWire([sideWire(9)], 30))).toMatchObject({lookback: 30, items: [{index: 9, invoice: "2009"}]});
    });
});

describe("UNIT accrual decode: schedules and plans", () => {
    it("decodes a transaction's schedules with period status, renewal and a party suggestion", () => {
        const view = decodeTransactionSchedules({
            enabled: true,
            generation: "g1",
            schedules: [scheduleWire("cx1217", "posted")],
            unscheduled: [2],
            renewal: {prefill: specWire("cx1116"), contract: "renewal"},
            partySuggestion: {kind: "customer", name: "Customer X"},
        });
        expect(view?.schedules[0]?.periods[0]).toMatchObject({status: "posted", entry: 7, part: "regular"});
        expect(view?.schedules[0]?.spec.term).toEqual({months: 12});
        expect(view?.schedules[0]?.spec.headerTags).toEqual([["renewal-date", "2026-12-17"]]);
        expect(view?.renewal?.contract).toBe("renewal");
        expect(view?.unscheduled).toEqual([2]);
    });

    it("refuses a term that is both or neither", () => {
        const bad = {...scheduleWire("x"), spec: {...specWire("x"), term: {months: 12, through: "2027-01-01"}}};
        expect(() => decodeTransactionSchedules({enabled: true, generation: "g", schedules: [bad], unscheduled: []})).toThrow(/months\} or \{through/);
    });

    it("decodes a save plan with a restatement", () => {
        const plan = decodeSavePlan({
            enabled: true,
            accrualGeneration: "a1",
            appended: [scheduleWire("new1")],
            restatements: [
                {
                    id: "cx1217",
                    ruleChange: {before: "~ old", after: "~ new"},
                    rows: [{index: 4, date: "2026-01-17", recognized: "2026-01-17", closed: true, before: [{m: "100000", p: 2}], action: "kept"}],
                    deletes: [],
                    adjustment: {date: "2026-11-17", description: "Customer X Restated", tags: [["restated", "2026-11-17"]], postings: []},
                    totalAfter: [{m: "-1140000", p: 2}],
                },
            ],
            createsFile: false,
        });
        expect(plan?.appended[0]?.periods[0]?.status).toBeNull();
        expect(plan?.restatements[0]?.rows[0]).toMatchObject({action: "kept", after: null, recognizedAfter: null, closed: true});
        expect(plan?.restatements[0]?.adjustment?.tags).toEqual([["restated", "2026-11-17"]]);
        expect(plan?.generation).toBeNull();
    });

    it("decodes the small write answers", () => {
        expect(decodeCreatedFile({enabled: true, generation: "g2", path: "recognition.journal"})).toEqual({generation: "g2", path: "recognition.journal"});
        expect(decodeClosedThrough({generation: "g2", closedThrough: null})).toEqual({generation: "g2", closedThrough: null});
        expect(decodeSettleLink({generation: "g2", matchId: "m1"})).toEqual({generation: "g2", matchId: "m1"});
        expect(decodeSettleLinkAll({generation: "g2", linked: 2, matchIds: ["a", "b"]})).toEqual({generation: "g2", linked: 2, matchIds: ["a", "b"]});
        expect(() => decodeSettleLinkAll({generation: "g2", linked: -1, matchIds: []})).toThrow(ApiShapeError);
    });
});
