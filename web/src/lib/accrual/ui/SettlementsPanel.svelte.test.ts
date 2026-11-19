// Invoice ↔ payment matching (plan 45 D-settle) against a fake engine: a
// proposal card's fit, the fee account a short payment needs, Link's body,
// Link all clear's confirm, and the open lists.

import {fireEvent, render, screen, within} from "@testing-library/svelte";
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import {journal} from "#lib/stores/journal.svelte.js";
import {FAKE_ENGINE} from "#lib/testing/fakeEngine.js";
import {proposalWire, serveAccrual, settlementsWire, sideWire} from "#lib/testing/accrualFixture.js";
import {accrual, resetAccrual} from "../store.svelte";
import {bpsToPercent, percentToBps} from "./format";
import SettlementsPanel from "./SettlementsPanel.svelte";

const EXACT = proposalWire(sideWire(1), sideWire(2, {description: "Fabrikam payment", invoice: undefined}));
const SHORT = proposalWire(sideWire(3), sideWire(4, {amount: {commodity: "$", quantity: {m: "97000", p: 2}}}), {
    fit: {kind: "short", difference: {m: "3000", p: 2}},
    clear: false,
});
const OVER = proposalWire(sideWire(5), sideWire(6), {fit: {kind: "over", difference: {m: "500", p: 2}}, clear: false});

async function show(proposals: Record<string, unknown>[], open: {invoices?: unknown[]; payments?: unknown[]} = {}) {
    const served = await serveAccrual({settlements: settlementsWire(proposals, open)});
    await accrual.sync(FAKE_ENGINE, journal.generation ?? "");
    accrual.setPageOpen(true);
    render(SettlementsPanel);
    await vi.waitFor(() => expect(screen.getByTestId("settlements-counts")).toBeDefined());
    return served;
}

const links = (seen: {method: string; url: URL; body?: unknown}[], route: string) =>
    seen.filter((r) => r.method === "POST" && r.url.pathname.endsWith(route)).map((r) => r.body as Record<string, unknown>);

beforeEach(() => resetAccrual());
afterEach(() => {
    vi.unstubAllGlobals();
    resetAccrual();
});

describe("COMPONENT SettlementsPanel", () => {
    it("shows counts, each card's fit, and the open lists", async () => {
        await show([EXACT, SHORT, OVER], {invoices: [sideWire(9, {description: "Contoso Invoice 2009"})], payments: []});
        expect(screen.getByTestId("settlements-counts").textContent?.replace(/\s+/g, " ")).toContain("1 clear, 2 to check");
        const fits = screen.getAllByTestId("settlement-fit").map((node) => node.textContent);
        expect(fits).toEqual(["Exact amount", "Short by $30.00 (a fee)", "Over by $5.00: check only, can't link"]);
        const cards = screen.getAllByTestId("settlement-card");
        expect((within(cards[2] as HTMLElement).getByTestId("settlement-link") as HTMLButtonElement).disabled).toBe(true);
        expect(screen.getByTestId("settlements-open-invoices").textContent).toContain("Contoso Invoice 2009");
        expect(screen.getByTestId("settlements-open-payments").textContent).toContain("None.");
    });

    it("requires a fee account for a short payment, then links with it", async () => {
        const {seen} = await show([SHORT]);
        await fireEvent.click(screen.getByTestId("settlement-link"));
        expect(screen.getByTestId("settlements-failure").textContent).toContain("Name the account the fee goes to.");
        expect(links(seen, "/api/settlements/link")).toHaveLength(0);
        await fireEvent.input(screen.getByTestId("settlement-fee"), {target: {value: "expenses:fees:card"}});
        await fireEvent.click(screen.getByTestId("settlement-link"));
        await vi.waitFor(() => expect(links(seen, "/api/settlements/link")).toHaveLength(1));
        expect(links(seen, "/api/settlements/link")[0]).toEqual({
            invoice: {index: 3, posting: 1, fingerprint: "fp-3"},
            payment: {index: 4, posting: 1, fingerprint: "fp-4"},
            feeAccount: "expenses:fees:card",
            tolerance: 300,
        });
    });

    it("prefills the fee account the engine suggests", async () => {
        const {seen} = await show([{...SHORT, feeAccount: "expenses:fees:stripe"}]);
        expect((screen.getByTestId("settlement-fee") as HTMLInputElement).value).toBe("expenses:fees:stripe");
        await fireEvent.click(screen.getByTestId("settlement-link"));
        await vi.waitFor(() => expect(links(seen, "/api/settlements/link")[0]?.feeAccount).toBe("expenses:fees:stripe"));
    });

    it("links an exact pair with no fee account", async () => {
        const {seen} = await show([EXACT]);
        await fireEvent.click(screen.getByTestId("settlement-link"));
        await vi.waitFor(() => expect(links(seen, "/api/settlements/link")).toHaveLength(1));
        expect(links(seen, "/api/settlements/link")[0]).not.toHaveProperty("feeAccount");
    });

    it("asks before linking all clear, then links in one request", async () => {
        const {seen} = await show([EXACT, SHORT]);
        expect(screen.getByTestId("settlements-link-all").textContent).toContain("Link all clear (1)");
        await fireEvent.click(screen.getByTestId("settlements-link-all"));
        expect(links(seen, "/api/settlements/link-all")).toHaveLength(0);
        expect(screen.getByTestId("settlements-link-all").textContent).toContain("Link them");
        await fireEvent.click(screen.getByTestId("settlements-link-all"));
        await vi.waitFor(() => expect(screen.getByTestId("settlements-linked-all").textContent).toBe("Linked 1 pair."));
        expect(links(seen, "/api/settlements/link-all")[0]).toMatchObject({generation: "g1"});
    });

    it("writes the tolerance as basis points and reads the list again", async () => {
        const {seen} = await show([EXACT]);
        const before = seen.filter((r) => r.url.pathname.endsWith("/api/settlements")).length;
        const input = screen.getByTestId("settlements-tolerance") as HTMLInputElement;
        await fireEvent.input(input, {target: {value: "2.5"}});
        await fireEvent.change(input);
        await vi.waitFor(() => expect(seen.filter((r) => r.url.pathname.endsWith("/api/settlements")).length).toBe(before + 1));
        expect(
            seen
                .filter((r) => r.url.pathname.endsWith("/api/settlements"))
                .at(-1)
                ?.url.searchParams.get("tolerance")
        ).toBe("250");
        await fireEvent.input(input, {target: {value: "3"}});
        await fireEvent.change(input);
    });

    it("says why Link is held back when the engine served no fingerprints, and refreshes on asking", async () => {
        // Patrick, 2026-11-18: after one Link every other Link stayed grey with no
        // word why. A side with no fingerprint can't be linked; the card says so.
        const stale = proposalWire(sideWire(1, {fingerprint: undefined}), sideWire(2));
        const {seen, state} = await show([stale]);
        const card = screen.getByTestId("settlement-card");
        expect((within(card).getByTestId("settlement-link") as HTMLButtonElement).disabled).toBe(true);
        expect(within(card).getByTestId("settlement-stale").textContent).toContain("Can't link right now: the journal needs a refresh.");
        const reads = seen.filter((r) => r.url.pathname.endsWith("/api/settlements")).length;
        state.settlements = settlementsWire([proposalWire(sideWire(1), sideWire(2))]);
        await fireEvent.click(within(card).getByTestId("settlement-refresh"));
        await vi.waitFor(() => expect(seen.filter((r) => r.url.pathname.endsWith("/api/settlements")).length).toBe(reads + 1));
        await vi.waitFor(() => expect(screen.queryByTestId("settlement-stale")).toBeNull());
        expect((screen.getByTestId("settlement-link") as HTMLButtonElement).disabled).toBe(false);
    });

    it("shows Linking… on the clicked card and asks the others to wait, then frees them all", async () => {
        const other = proposalWire(sideWire(7), sideWire(8));
        const {state} = await show([EXACT, other]);
        // Hold the write open so the in-between state can be seen.
        let release: (() => void) | undefined;
        const held = new Promise<void>((resolve) => (release = resolve));
        const realFetch = globalThis.fetch;
        vi.stubGlobal("fetch", async (input: RequestInfo | URL, init?: RequestInit) => {
            if (String(input).includes("/api/settlements/link")) await held;
            return realFetch(input, init);
        });
        const [first, second] = screen.getAllByTestId("settlement-card") as HTMLElement[];
        await fireEvent.click(within(first as HTMLElement).getByTestId("settlement-link"));
        await vi.waitFor(() => expect(within(first as HTMLElement).getByTestId("settlement-linking").textContent).toContain("Linking…"));
        expect(within(second as HTMLElement).getByTestId("settlement-wait").textContent).toContain("One moment");
        expect((within(second as HTMLElement).getByTestId("settlement-link") as HTMLButtonElement).disabled).toBe(true);
        // The engine links it; the re-read lists only the other pair, still linkable.
        state.settlements = settlementsWire([other]);
        release?.();
        await vi.waitFor(() => expect(screen.getAllByTestId("settlement-card")).toHaveLength(1));
        await vi.waitFor(() => expect((screen.getByTestId("settlement-link") as HTMLButtonElement).disabled).toBe(false));
        expect(screen.queryByTestId("settlement-wait")).toBeNull();
        expect(screen.queryByTestId("settlement-linking")).toBeNull();
    });

    it("converts percents and basis points", () => {
        expect(bpsToPercent(300)).toBe("3.0");
        expect(percentToBps("2.5%")).toBe(250);
        expect(percentToBps("abc")).toBeNull();
        expect(percentToBps(" ")).toBeNull();
    });
});
