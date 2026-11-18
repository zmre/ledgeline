// Settings → Books (plan 45 D-settings): nothing but a note with the helpers
// off; with them on, the autoRecognize toggle, Close books through… with its
// confirmation (the one write to `[books] closed-through`), Reopen, and the
// invoice matcher's lookback and tolerance.

import {fireEvent, render, screen} from "@testing-library/svelte";
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";

import {accrual, resetAccrual} from "#lib/accrual/store.svelte.js";
import {settings} from "#lib/stores/settings.svelte.js";
import {connectFakeEngine, FAKE_ENGINE, journalInfoBody} from "#lib/testing/fakeEngine.js";
import AccrualSettingsPanel from "./AccrualSettingsPanel.svelte";

/** A `/api/accrual` body with the helpers on (synthetic). */
function accrualBody(closedThrough: string | null): Record<string, unknown> {
    return {
        enabled: true,
        generation: "g1",
        basis: {value: "accrual", source: "inferred"},
        reason: {kind: "declared", account: "liabilities:deferred-revenue", role: "deferred-revenue"},
        roles: {"liabilities:deferred-revenue": {role: "deferred-revenue", declared: true}},
        file: {path: "recognition.journal", exists: true, included: false},
        ...(closedThrough === null ? {} : {closedThrough}),
        counts: {due: 3, dueClosed: 0, needsSchedule: 0, invoices: 0},
        warnings: [],
    };
}

let posted: unknown[] = [];
let closed: string | null = null;
let enabled = true;

async function serve(): Promise<void> {
    await connectFakeEngine();
    posted = [];
    vi.stubGlobal("fetch", (input: RequestInfo | URL, init?: RequestInit) => {
        const url = new URL(String(input));
        const json = (value: unknown, status = 200) => Promise.resolve(new Response(JSON.stringify(value), {status}));
        const path = url.pathname;
        if (path.endsWith("/version")) return Promise.resolve(new Response("1.52"));
        // Every write moves the generation, which is what re-reads the status.
        if (path.endsWith("/api/journal")) return json(journalInfoBody({generation: `g${posted.length + 1}`}));
        if (path.endsWith("/api/diagnostics")) return json({count: 0, maxSeverity: null, diagnostics: []});
        if (path.endsWith("/api/accrual")) return json(enabled ? accrualBody(closed) : {enabled: false});
        if (path.endsWith("/api/books/closed-through") && init?.method === "POST") {
            const body = JSON.parse(String(init.body)) as {date: string | null};
            posted.push(body);
            closed = body.date;
            return json({generation: `g${posted.length + 1}`, closedThrough: closed});
        }
        return Promise.resolve(new Response("", {status: 404}));
    });
    await accrual.sync(FAKE_ENGINE, "g1");
}

beforeEach(() => {
    resetAccrual();
    closed = null;
    enabled = true;
});

afterEach(() => {
    vi.unstubAllGlobals();
    settings.autoRecognize = false;
    settings.settlementToleranceBps = 300;
    settings.settlementLookbackDays = 180;
});

describe("COMPONENT AccrualSettingsPanel", () => {
    it("only explains itself while the helpers are off", async () => {
        enabled = false;
        await serve();
        render(AccrualSettingsPanel);
        expect(screen.getByText(/appear when the ledger has a deferred-revenue, prepaid, receivable or payable account/)).toBeTruthy();
        expect(screen.queryByTestId("books-auto-recognize")).toBeNull();
        expect(screen.queryByTestId("books-close")).toBeNull();
    });

    it("toggles automatic posting and sets the matcher's lookback and tolerance, clamped", async () => {
        await serve();
        render(AccrualSettingsPanel);
        expect(screen.getByTestId("books-file").textContent).toContain("recognition.journal");
        await fireEvent.click(screen.getByTestId("books-auto-recognize"));
        expect(settings.autoRecognize).toBe(true);

        const tolerance = screen.getByTestId("books-tolerance") as HTMLInputElement;
        expect(tolerance.value).toBe("3.0");
        await fireEvent.change(tolerance, {target: {value: "2.5"}});
        expect(settings.settlementToleranceBps).toBe(250);
        expect(tolerance.value).toBe("2.5");

        const lookback = screen.getByTestId("books-lookback") as HTMLInputElement;
        await fireEvent.change(lookback, {target: {value: "99999"}});
        expect(settings.settlementLookbackDays).toBe(3650);
    });

    it("closes the books only after saying what it writes, then offers Reopen", async () => {
        await serve();
        render(AccrualSettingsPanel);
        expect(screen.getByTestId("books-closed-through").textContent).toContain("The books are open");
        const close = screen.getByTestId("books-close") as HTMLButtonElement;
        expect(close.disabled).toBe(true);

        const date = screen.getByLabelText("Close books through") as HTMLInputElement;
        await fireEvent.input(date, {target: {value: "2026-09-30"}});
        expect(close.disabled).toBe(false);
        await fireEvent.click(close);
        const confirm = screen.getByTestId("books-confirm");
        expect(confirm.textContent?.replace(/\s+/g, " ")).toContain('Ledgeline writes closed-through = "2026-09-30" to this ledger\'s ledgeline.toml');
        expect(posted).toEqual([]);
        await fireEvent.click(screen.getByTestId("books-confirm-go"));

        await vi.waitFor(() => expect(posted).toEqual([{date: "2026-09-30"}]));
        await vi.waitFor(() => expect(screen.getByTestId("books-closed-through").textContent).toContain("Closed through 2026-09-30"));

        await fireEvent.click(screen.getByTestId("books-reopen"));
        expect(screen.getByTestId("books-confirm").textContent).toContain("Reopen every period?");
        await fireEvent.click(screen.getByTestId("books-confirm-go"));
        await vi.waitFor(() => expect(posted).toEqual([{date: "2026-09-30"}, {date: null}]));
    });
});
