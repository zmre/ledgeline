// The accrual store (plan 45 Phase 3) against a fake engine: the gate read
// once per generation, every accrual write's body, and the re-reads after.

import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import {journal} from "#lib/stores/journal.svelte.js";
import {settings} from "#lib/stores/settings.svelte.js";
import {FAKE_ENGINE} from "#lib/testing/fakeEngine.js";
import {dueEntryWire, dueMonthWire, dueWire, proposalWire, serveAccrual, settlementsWire, sideWire} from "#lib/testing/accrualFixture.js";
import {decodeSettlements} from "./decode";
import {accrual, resetAccrual} from "./store.svelte";

const OCT = dueEntryWire("cx1217", "2026-10-17", 100000);
const NOV = dueEntryWire("cx1217", "2026-11-17", 100000);
const NOV_Y = dueEntryWire("cy0203", "2026-11-03", 83333, "Customer Y Recognized");
const SEP = dueEntryWire("cx1217", "2026-09-17", 100000);

const statusReads = (seen: {url: URL}[]) => seen.filter((r) => r.url.pathname.endsWith("/api/accrual")).length;
const posts = (seen: {method: string; url: URL; body?: unknown}[], route: string) => seen.filter((r) => r.method === "POST" && r.url.pathname.endsWith(route));

beforeEach(() => resetAccrual());
afterEach(() => {
    vi.unstubAllGlobals();
    resetAccrual();
});

describe("COMPONENT accrual store: the gate", () => {
    it("reads the status once per generation and says the helpers are on", async () => {
        const {seen} = await serveAccrual();
        const generation = journal.generation ?? "";
        await accrual.sync(FAKE_ENGINE, generation);
        await accrual.sync(FAKE_ENGINE, generation);
        expect(statusReads(seen)).toBe(1);
        expect(accrual.enabled).toBe(true);
        expect(accrual.dueCount).toBe(3);
        expect(accrual.roleOf("assets:prepaid")).toBe("prepaid");
        expect(accrual.roleOf("assets:checking")).toBeNull();
        await accrual.sync(FAKE_ENGINE, "another");
        expect(statusReads(seen)).toBe(2);
    });

    it("stays off on {enabled: false}, and the overlay will not open", async () => {
        await serveAccrual({status: {enabled: false}});
        await accrual.sync(FAKE_ENGINE, journal.generation ?? "");
        expect(accrual.enabled).toBe(false);
        expect(accrual.status).toBeNull();
        accrual.openOverlay();
        expect(accrual.open).toBe(false);
    });

    it("sends the lookback, and a new lookback reads again", async () => {
        const {seen} = await serveAccrual();
        await accrual.sync(FAKE_ENGINE, journal.generation ?? "");
        await accrual.setNeedsLookback(0);
        const last = seen.filter((r) => r.url.pathname.endsWith("/api/accrual")).at(-1);
        expect(last?.url.searchParams.get("lookback")).toBe("0");
        expect(statusReads(seen)).toBe(2);
    });
});

describe("COMPONENT accrual store: posting", () => {
    it("posts exactly the entries asked for, through the latest month's end, and reads everything again", async () => {
        const {seen, state} = await serveAccrual({
            due: dueWire(
                [dueMonthWire("2026-10", [{party: "Customer X", entries: [OCT]}]), dueMonthWire("2026-11", [{party: null, entries: [NOV, NOV_Y]}])],
                [SEP]
            ),
        });
        await accrual.sync(FAKE_ENGINE, journal.generation ?? "");
        accrual.openOverlay();
        await vi.waitFor(() => expect(accrual.due).not.toBeNull());
        const listed = accrual.due!;
        const october = listed.months[0]!.groups[0]!.entries[0]!;
        const outcome = await accrual.post([october], true);
        expect(outcome.ok).toBe(true);
        const sent = posts(seen, "/api/recognition/post")[0]?.body as Record<string, unknown>;
        expect(sent).toMatchObject({
            generation: "g1",
            through: "2026-10-31",
            entries: [{id: "cx1217", recognized: "2026-10-17"}],
            includeClosed: true,
        });
        expect(typeof sent.today).toBe("string");
        expect(state.generation).toBe(2);
        // The journal, the status and the due list were read again at the new generation.
        await vi.waitFor(() => expect(accrual.due?.generation).toBe("g2"));
        expect(accrual.status?.generation).toBe("g2");
        expect(accrual.notice?.message).toContain("Posted 1 recognition entry through 2026-10-31 and 1 closed-period adjustment");
        expect(accrual.notice?.month).toBe("2026-10");
    });

    it("says nothing was written when the engine posted nothing", async () => {
        await serveAccrual({due: dueWire([dueMonthWire("2026-11", [{party: null, entries: [NOV]}])])});
        await accrual.sync(FAKE_ENGINE, journal.generation ?? "");
        accrual.openOverlay();
        await vi.waitFor(() => expect(accrual.due).not.toBeNull());
        await accrual.post([]);
        expect(accrual.notice?.message).toBe("Nothing to post: those entries are already in the journal.");
    });

    it("answers a 409 as a conflict", async () => {
        const {state} = await serveAccrual({due: dueWire([dueMonthWire("2026-11", [{party: null, entries: [NOV]}])])});
        await accrual.sync(FAKE_ENGINE, journal.generation ?? "");
        accrual.openOverlay();
        await vi.waitFor(() => expect(accrual.due).not.toBeNull());
        state.failNext = {status: 409, error: "the journal changed since this was listed"};
        const outcome = await accrual.post(accrual.due!.months[0]!.groups[0]!.entries);
        expect(outcome).toMatchObject({ok: false, failure: {kind: "conflict"}});
    });
});

describe("COMPONENT accrual store: settlements and books", () => {
    const SHORT = proposalWire(sideWire(1), sideWire(2, {account: "assets:receivable", amount: {commodity: "$", quantity: {m: "97000", p: 2}}}), {
        fit: {kind: "short", difference: {m: "3000", p: 2}},
        clear: false,
    });

    it("links one proposal with its fee account and the tolerance", async () => {
        settings.settlementToleranceBps = 250;
        const {seen} = await serveAccrual({settlements: settlementsWire([SHORT])});
        const proposal = decodeSettlements(settlementsWire([SHORT]))!.proposals[0]!;
        const outcome = await accrual.link(proposal, " expenses:fees ");
        expect(outcome.ok).toBe(true);
        expect(posts(seen, "/api/settlements/link")[0]?.body).toEqual({
            invoice: {index: 1, posting: 1, fingerprint: "fp-1"},
            payment: {index: 2, posting: 1, fingerprint: "fp-2"},
            feeAccount: "expenses:fees",
            tolerance: 250,
        });
        settings.settlementToleranceBps = 300;
    });

    it("refuses a side with no fingerprint without a request", async () => {
        const {seen} = await serveAccrual();
        const proposal = decodeSettlements(settlementsWire([proposalWire(sideWire(1, {fingerprint: undefined}), sideWire(2))]))!.proposals[0]!;
        const outcome = await accrual.link(proposal);
        expect(outcome).toMatchObject({ok: false, failure: {kind: "unavailable"}});
        expect(posts(seen, "/api/settlements/link")).toHaveLength(0);
    });

    it("links all clear against the listed generation", async () => {
        settings.settlementLookbackDays = 90;
        const {seen} = await serveAccrual({settlements: settlementsWire([proposalWire(sideWire(1), sideWire(2))])});
        accrual.setPageOpen(true);
        await vi.waitFor(() => expect(accrual.settlements).not.toBeNull());
        const outcome = await accrual.linkAll();
        expect(outcome.ok).toBe(true);
        expect(posts(seen, "/api/settlements/link-all")[0]?.body).toMatchObject({generation: "g1", lookback: 90, tolerance: 300});
        expect(accrual.notice?.message).toBe("Linked 1 invoice to its payment.");
        // The page's list was read again after the write.
        await vi.waitFor(() => expect(accrual.settlements?.generation).toBe("g2"));
        accrual.setPageOpen(false);
        settings.settlementLookbackDays = 180;
    });

    it("sets and clears closed-through, and creates the file", async () => {
        const {seen} = await serveAccrual();
        expect(await accrual.setClosedThrough("2026-10-31")).toMatchObject({ok: true, value: {closedThrough: "2026-10-31"}});
        expect(await accrual.setClosedThrough(null)).toMatchObject({ok: true, value: {closedThrough: null}});
        expect(posts(seen, "/api/books/closed-through").map((r) => r.body)).toEqual([{date: "2026-10-31"}, {date: null}]);
        expect(await accrual.createFile()).toMatchObject({ok: true, value: {path: "recognition.journal"}});
    });
});

describe("COMPONENT accrual store: the badge, the first tab, and stale lists", () => {
    it("badges due + needs a schedule + clear matches, and breaks the total down", async () => {
        const {seen} = await serveAccrual();
        await accrual.sync(FAKE_ENGINE, journal.generation ?? "");
        // accrualStatusWire: 3 due, 1 needs a schedule, 2 clear (1 to check).
        expect(accrual.badgeCount).toBe(6);
        expect(accrual.badgeBreakdown).toBe("3 due · 1 needs a schedule · 2 ready to link");
        // The counts are the user's: the matcher's lookback and tolerance ride along.
        const read = seen.filter((r) => r.url.pathname.endsWith("/api/accrual")).at(-1);
        expect(read?.url.searchParams.get("settlementLookback")).toBe(String(settings.settlementLookbackDays));
        expect(read?.url.searchParams.get("settlementTolerance")).toBe(String(settings.settlementToleranceBps));
    });

    it("opens on the first tab with something in it: Due, then Needs a schedule, then Invoices", async () => {
        const {state} = await serveAccrual();
        const counts = (due: number, needsSchedule: number, invoices: number) => ({due, dueClosed: 0, needsSchedule, invoices, invoicesCheck: 0});
        const cases: [ReturnType<typeof counts>, string][] = [
            [counts(2, 5, 5), "due"],
            [counts(0, 11, 6), "needs"],
            [counts(0, 0, 6), "invoices"],
            [counts(0, 0, 0), "due"],
        ];
        for (const [wire, tab] of cases) {
            resetAccrual();
            state.status = {...state.status, counts: wire};
            state.generation += 1;
            await accrual.sync(FAKE_ENGINE, `g${state.generation}`);
            accrual.openOverlay();
            expect(accrual.tab).toBe(tab);
            accrual.close();
        }
    });

    it("re-reads the showing list when the journal moves on, so its fingerprints are the editor's again", async () => {
        const {seen, state} = await serveAccrual({settlements: settlementsWire([proposalWire(sideWire(1, {fingerprint: undefined}), sideWire(2))])});
        await accrual.sync(FAKE_ENGINE, journal.generation ?? "");
        accrual.setPageOpen(true);
        await vi.waitFor(() => expect(accrual.settlements?.proposals[0]?.invoice.fingerprint).toBeNull());
        const reads = seen.filter((r) => r.url.pathname.endsWith("/api/settlements")).length;
        // The watcher (or anyone) published a new generation.
        state.generation += 1;
        state.settlements = settlementsWire([proposalWire(sideWire(1), sideWire(2))]);
        await accrual.sync(FAKE_ENGINE, `g${state.generation}`);
        await vi.waitFor(() => expect(seen.filter((r) => r.url.pathname.endsWith("/api/settlements")).length).toBe(reads + 1));
        await vi.waitFor(() => expect(accrual.settlements?.proposals[0]?.invoice.fingerprint).toBe("fp-1"));
        accrual.setPageOpen(false);
    });
});
