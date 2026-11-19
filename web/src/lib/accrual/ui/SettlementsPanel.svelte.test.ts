// Invoice ↔ payment matching (plan 45 D-settle) against a fake engine: a
// proposal card's fit, the fee account a short payment needs, Link's body,
// Link all clear's confirm, and the open lists.

import {fireEvent, render, screen, within} from "@testing-library/svelte";
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import {journal} from "#lib/stores/journal.svelte.js";
import {settings} from "#lib/stores/settings.svelte.js";
import {FAKE_ENGINE} from "#lib/testing/fakeEngine.js";
import {proposalWire, serveAccrual, settlementsWire, sideWire} from "#lib/testing/accrualFixture.js";
import {accrual, resetAccrual} from "../store.svelte";
import {resetTxnDetails} from "../details.svelte";
import {keymap} from "#lib/keys/keymap.svelte.js";
import {txnFixture} from "#lib/testing/reviewFixture.js";
import {wireRow} from "#lib/testing/journalViewFixture.js";
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

beforeEach(() => {
    resetAccrual();
    resetTxnDetails();
});
afterEach(() => {
    vi.unstubAllGlobals();
    resetAccrual();
    resetTxnDetails();
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

    it("says when a payment came before its invoice, and the lead setting reads the list again", async () => {
        const {seen} = await show([proposalWire(sideWire(1), sideWire(2), {early: true, gapDays: 3})]);
        expect(screen.getByTestId("settlement-gap").textContent).toBe("Paid 3 days before the invoice");
        const lead = screen.getByTestId("settlements-lead") as HTMLInputElement;
        expect(lead.value).toBe(String(settings.settlementLeadDays));
        await fireEvent.change(lead, {target: {value: "6"}});
        expect(settings.settlementLeadDays).toBe(6);
        await vi.waitFor(() =>
            expect(
                seen
                    .filter((r) => r.url.pathname.endsWith("/api/settlements"))
                    .at(-1)
                    ?.url.searchParams.get("lead")
            ).toBe("6")
        );
        settings.settlementLeadDays = 4;
    });

    it("converts percents and basis points", () => {
        expect(bpsToPercent(300)).toBe("3.0");
        expect(percentToBps("2.5%")).toBe(250);
        expect(percentToBps("abc")).toBeNull();
        expect(percentToBps(" ")).toBeNull();
    });

    // -- Phase 3b: full transaction details ----------------------------------

    /**
     * Serve `GET /api/transactions/{index}` on top of the accrual engine,
     * counting the reads; `failing` answers those indices with a 500.
     */
    function serveTransactions(failing: Set<number> = new Set()): Map<number, number> {
        const reads = new Map<number, number>();
        const inner = globalThis.fetch;
        vi.stubGlobal("fetch", (input: RequestInfo | URL, init?: RequestInit) => {
            const url = new URL(String(input));
            const match = /\/api\/transactions\/(\d+)$/.exec(url.pathname);
            if (match !== null && (init?.method ?? "GET") === "GET") {
                const index = Number(match[1]);
                reads.set(index, (reads.get(index) ?? 0) + 1);
                if (failing.has(index)) return Promise.resolve(new Response("the engine fell over", {status: 500}));
                return Promise.resolve(new Response(JSON.stringify(wireRow(txnFixture(index, `Whole transaction ${index}`, "assets:receivable")))));
            }
            return inner(input, init);
        });
        return reads;
    }

    it("Details on either side opens the pair, each read once, its posting highlighted; again closes both", async () => {
        await show([EXACT, proposalWire(sideWire(7), sideWire(8))]);
        const reads = serveTransactions();
        const [card, other] = screen.getAllByTestId("settlement-card") as HTMLElement[];
        const invoice = within(card as HTMLElement).getByTestId("settlement-invoice");
        const payment = within(card as HTMLElement).getByTestId("settlement-payment");
        expect(within(card as HTMLElement).queryByTestId("txn-details")).toBeNull();
        expect(reads.size).toBe(0);
        // The payment side's Details opens the invoice side too.
        await fireEvent.click(within(payment).getByTestId("details-toggle"));
        await vi.waitFor(() => expect(within(invoice).getByTestId("detail-description").textContent).toBe("Whole transaction 1"));
        await vi.waitFor(() => expect(within(payment).getByTestId("detail-description").textContent).toBe("Whole transaction 2"));
        expect([reads.get(1), reads.get(2)]).toEqual([1, 1]);
        // Each side marks its own matched posting (1).
        const rows = within(within(invoice).getByTestId("detail-postings")).getAllByRole("listitem");
        expect(rows.map((row) => row.hasAttribute("data-highlighted"))).toEqual([false, true]);
        // Both toggles say so; the other card is untouched; there is no separate Both.
        expect(
            within(card as HTMLElement)
                .getAllByTestId("details-toggle")
                .map((b) => b.getAttribute("aria-expanded"))
        ).toEqual(["true", "true"]);
        expect(within(other as HTMLElement).queryByTestId("txn-details")).toBeNull();
        expect(screen.queryByTestId("details-both")).toBeNull();
        // Again, from the invoice side: both close. Reopening reads nothing new.
        await fireEvent.click(within(invoice).getByTestId("details-toggle"));
        expect(within(card as HTMLElement).queryByTestId("txn-details")).toBeNull();
        await fireEvent.click(within(invoice).getByTestId("details-toggle"));
        await vi.waitFor(() => expect(within(card as HTMLElement).getAllByTestId("detail-description")).toHaveLength(2));
        expect([reads.get(1), reads.get(2)]).toEqual([1, 1]);
    });

    it("toggles the focused card's pair with d", async () => {
        await show([EXACT, proposalWire(sideWire(7), sideWire(8))]);
        serveTransactions();
        const [first, second] = screen.getAllByTestId("settlement-card") as HTMLElement[];
        const button = within(second as HTMLElement).getAllByTestId("details-toggle")[0] as HTMLElement;
        button.focus();
        await fireEvent.focusIn(button);
        keymap.handle(new KeyboardEvent("keydown", {key: "d", cancelable: true}));
        await vi.waitFor(() => expect(within(second as HTMLElement).getAllByTestId("txn-details")).toHaveLength(2));
        expect(within(first as HTMLElement).queryByTestId("txn-details")).toBeNull();
        keymap.handle(new KeyboardEvent("keydown", {key: "d", cancelable: true}));
        await vi.waitFor(() => expect(within(second as HTMLElement).queryByTestId("txn-details")).toBeNull());
    });

    it("Show details expands every card and every open-list row", async () => {
        await show([EXACT, proposalWire(sideWire(7), sideWire(8))], {invoices: [sideWire(9)], payments: [sideWire(10)]});
        serveTransactions();
        await fireEvent.click(screen.getByTestId("settlements-show-details"));
        await vi.waitFor(() => expect(screen.getAllByTestId("txn-details")).toHaveLength(6));
        for (const row of screen.getAllByTestId("settlements-open-row")) expect(within(row).getByTestId("txn-details")).toBeDefined();
        await fireEvent.click(screen.getByTestId("settlements-show-details"));
        expect(screen.queryAllByTestId("txn-details")).toHaveLength(0);
    });

    it("reads a side again after the journal moves on", async () => {
        const {state} = await show([EXACT]);
        const reads = serveTransactions();
        const invoice = screen.getByTestId("settlement-invoice");
        await fireEvent.click(within(invoice).getByTestId("details-toggle"));
        await vi.waitFor(() => expect(reads.get(1)).toBe(1));
        state.generation += 1;
        await journal.refresh({force: true});
        await vi.waitFor(() => expect(reads.get(1)).toBe(2));
        await vi.waitFor(() => expect(within(invoice).getByTestId("detail-description")).toBeDefined());
    });

    it("says so inline when a fetch fails, and Retry reads it again", async () => {
        await show([EXACT]);
        const failing = new Set([1]);
        const reads = serveTransactions(failing);
        const invoice = screen.getByTestId("settlement-invoice");
        await fireEvent.click(within(invoice).getByTestId("details-toggle"));
        await vi.waitFor(() => expect(within(invoice).getByTestId("txn-details-failed").textContent).toContain("Couldn't load transaction #1"));
        failing.clear();
        await fireEvent.click(within(invoice).getByTestId("txn-details-retry"));
        await vi.waitFor(() => expect(within(invoice).getByTestId("detail-description").textContent).toBe("Whole transaction 1"));
        expect(reads.get(1)).toBe(2);
    });

    it("offers Open in editor only when the host can open one", async () => {
        const onEdit = vi.fn();
        await serveAccrual({settlements: settlementsWire([EXACT])});
        await accrual.sync(FAKE_ENGINE, journal.generation ?? "");
        accrual.setPageOpen(true);
        render(SettlementsPanel, {onEdit});
        serveTransactions();
        const invoice = await vi.waitFor(() => screen.getByTestId("settlement-invoice"));
        await fireEvent.click(within(invoice).getByTestId("details-toggle"));
        await fireEvent.click(await vi.waitFor(() => within(invoice).getByTestId("txn-details-edit")));
        expect(onEdit).toHaveBeenCalledWith(1);
    });
});
