// The Holdings tab's "Update prices" button, mounted against a real
// `pricesStore` fed by a stubbed `fetch` — the `FAKE_ENGINE` convention
// (see `EditRulesPanel.svelte.test.ts`): the store is real, only the bytes it
// reads are fake, so what breaks is what the button does with the response, not
// whatever a hand-mocked store was told to say.

import {fireEvent, render, screen} from "@testing-library/svelte";
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import {connectFakeEngine, FAKE_ENGINE} from "$lib/testing/fakeEngine";
import {pricesStore} from "$lib/stores/prices.svelte";
import UpdatePricesButton from "./UpdatePricesButton.svelte";

const format = (qty: {m: bigint; p: number}): string => `$${(Number(qty.m) / 10 ** qty.p).toFixed(2)}`;

const STATUS_ONE_SYMBOL = {
    editable: true,
    quoteCommodity: "$",
    symbols: [{symbol: "AAPL", yahooTicker: "AAPL"}],
    defaultTarget: "prices.journal",
    canCreateFile: false,
    createFileName: "prices.journal",
    files: [{journalId: "prices.journal", label: "prices.journal", writable: true, priceCount: 3}],
};

const STATUS_NEEDS_FILE = {
    ...STATUS_ONE_SYMBOL,
    defaultTarget: null,
    canCreateFile: true,
    files: [],
};

/**
 * The shape a fresh single-file journal answers with: nothing anywhere holds a
 * `P` directive (`canCreateFile`), and so the engine's `defaultTarget` is its
 * fallback guess — the MAIN journal, the file holding every transaction.
 */
const STATUS_FALLS_BACK_TO_MAIN = {
    ...STATUS_ONE_SYMBOL,
    defaultTarget: "main.journal",
    canCreateFile: true,
    files: [{journalId: "main.journal", label: "main.journal", writable: true, priceCount: 0}],
};

/** Prices already on record, in a file this user did not call `prices.journal`. */
const STATUS_EXISTING_PRICE_FILE = {
    ...STATUS_ONE_SYMBOL,
    defaultTarget: "history.journal",
    canCreateFile: false,
    files: [{journalId: "history.journal", label: "history.journal", writable: true, priceCount: 12}],
};

const STATUS_NO_SYMBOLS = {...STATUS_ONE_SYMBOL, symbols: []};
const STATUS_READ_ONLY = {...STATUS_ONE_SYMBOL, editable: false};

const UPDATED_RESPONSE = {
    file: {journalId: "prices.journal", label: "prices.journal", writable: true, priceCount: 4},
    results: [{symbol: "AAPL", yahooTicker: "AAPL", outcome: "updated", date: "2026-06-30", price: {mantissa: "22800", places: 2}}],
};

let calls: string[] = [];
/** The JSON body each POSTed route was sent, by route — "which file did it write to?". */
let payloads: Record<string, string> = {};

async function stub(table: Record<string, unknown>): Promise<void> {
    calls = [];
    payloads = {};
    await connectFakeEngine();
    vi.stubGlobal("fetch", (input: unknown, init?: RequestInit) => {
        const url = String(input);
        const method = init?.method ?? "GET";
        const route = url.replace(FAKE_ENGINE, "");
        if (init?.body !== undefined && init.body !== null) payloads[route] = String(init.body);
        calls.push(`${method} ${route}`);
        if (url.endsWith("/version")) return Promise.resolve(new Response(JSON.stringify("1.52"), {status: 200}));
        const key = Object.keys(table).find((route) => url.endsWith(route));
        const body = key === undefined ? undefined : table[key];
        if (body === undefined) return Promise.resolve(new Response(`no route for ${url}`, {status: 404}));
        if (body instanceof Response) return Promise.resolve(body);
        return Promise.resolve(new Response(JSON.stringify(body), {status: 200, headers: {"Content-Type": "application/json"}}));
    });
}

beforeEach(() => {
    calls = [];
    payloads = {};
});

afterEach(() => vi.unstubAllGlobals());

describe("COMPONENT UpdatePricesButton", () => {
    it("is absent when the journal is read-only", async () => {
        await stub({"/api/prices/status": STATUS_READ_ONLY});
        await pricesStore.reloadStatus(FAKE_ENGINE);
        render(UpdatePricesButton, {format});
        expect(screen.queryByTestId("update-prices")).toBeNull();
    });

    it("is absent when no currently-held symbol needs a quote", async () => {
        await stub({"/api/prices/status": STATUS_NO_SYMBOLS});
        await pricesStore.reloadStatus(FAKE_ENGINE);
        render(UpdatePricesButton, {format});
        expect(screen.queryByTestId("update-prices")).toBeNull();
    });

    it("renders when a target file already exists, and updates it on click", async () => {
        await stub({"/api/prices/status": STATUS_ONE_SYMBOL, "/api/prices/update": UPDATED_RESPONSE});
        await pricesStore.reloadStatus(FAKE_ENGINE);
        render(UpdatePricesButton, {format});

        await fireEvent.click(screen.getByTestId("update-prices"));
        await vi.waitFor(() => expect(screen.getByTestId("update-prices-results")).toBeTruthy());

        expect(screen.getByTestId("update-prices-results").textContent).toContain("1 price updated");
        // No file was created — the target already existed.
        expect(calls).not.toContain("POST /api/prices/file");
        expect(calls).toContain("POST /api/prices/update");
    });

    it("creates prices.journal first when nothing can hold a price yet, then updates it", async () => {
        await stub({
            "/api/prices/status": STATUS_NEEDS_FILE,
            "/api/prices/file": {journalId: "prices.journal", label: "prices.journal", includedAs: "include prices.journal", mainJournalId: "main.journal"},
            "/api/prices/update": UPDATED_RESPONSE,
        });
        await pricesStore.reloadStatus(FAKE_ENGINE);
        render(UpdatePricesButton, {format});

        await fireEvent.click(screen.getByTestId("update-prices"));
        await vi.waitFor(() => expect(screen.getByTestId("update-prices-results")).toBeTruthy());

        // Create-then-update, in that order.
        expect(calls.indexOf("POST /api/prices/file")).toBeGreaterThanOrEqual(0);
        expect(calls.indexOf("POST /api/prices/update")).toBeGreaterThan(calls.indexOf("POST /api/prices/file"));
    });

    it("creates prices.journal rather than writing into the main journal", async () => {
        // The case a fresh single-file journal actually presents: nothing
        // anywhere holds a `P` directive, so `defaultTarget` is the engine's
        // fallback — the main journal, where every transaction lives. Taking it
        // would append price lines into the user's own book. `canCreateFile` is
        // the engine's "no file prices anything yet", and it wins.
        await stub({
            "/api/prices/status": STATUS_FALLS_BACK_TO_MAIN,
            "/api/prices/file": {journalId: "prices.journal", label: "prices.journal", includedAs: "include prices.journal", mainJournalId: "main.journal"},
            "/api/prices/update": UPDATED_RESPONSE,
        });
        await pricesStore.reloadStatus(FAKE_ENGINE);
        render(UpdatePricesButton, {format});

        await fireEvent.click(screen.getByTestId("update-prices"));
        await vi.waitFor(() => expect(screen.getByTestId("update-prices-results")).toBeTruthy());

        expect(calls).toContain("POST /api/prices/file");
        expect(payloads["/api/prices/update"]).toBe(JSON.stringify({journalId: "prices.journal"}));
    });

    it("updates the file that already holds prices, whatever it is called", async () => {
        // The other half of the same rule: `history.journal` (or `kurse.journal`,
        // or anything else) already prices things, so no second prices file is
        // invented and the update goes where the prices already are.
        await stub({"/api/prices/status": STATUS_EXISTING_PRICE_FILE, "/api/prices/update": UPDATED_RESPONSE});
        await pricesStore.reloadStatus(FAKE_ENGINE);
        render(UpdatePricesButton, {format});

        await fireEvent.click(screen.getByTestId("update-prices"));
        await vi.waitFor(() => expect(screen.getByTestId("update-prices-results")).toBeTruthy());

        expect(calls).not.toContain("POST /api/prices/file");
        expect(payloads["/api/prices/update"]).toBe(JSON.stringify({journalId: "history.journal"}));
    });

    it("re-reads the holdings report after a successful update", async () => {
        // The market values under the table are the whole point of the button;
        // the page's own load effect is keyed on (server, nonce, scope) and a
        // price write moves none of them. See `pricesStore.afterWrite`.
        await stub({"/api/prices/status": STATUS_ONE_SYMBOL, "/api/prices/update": UPDATED_RESPONSE});
        await pricesStore.reloadStatus(FAKE_ENGINE);
        render(UpdatePricesButton, {format});
        calls = [];

        await fireEvent.click(screen.getByTestId("update-prices"));
        await vi.waitFor(() => expect(screen.getByTestId("update-prices-results")).toBeTruthy());

        expect(calls.some((call) => call.includes("/api/holdings?"))).toBe(true);
        expect(calls.some((call) => call.includes("/api/holdings/series"))).toBe(true);
        expect(calls).toContain("GET /api/prices/status");
    });

    it("shows an error toast, not a crash, when the update fails", async () => {
        await stub({"/api/prices/status": STATUS_ONE_SYMBOL, "/api/prices/update": new Response("boom", {status: 500})});
        await pricesStore.reloadStatus(FAKE_ENGINE);
        render(UpdatePricesButton, {format});

        await fireEvent.click(screen.getByTestId("update-prices"));
        await vi.waitFor(() => expect(screen.queryByText(/Updating prices failed/)).toBeTruthy());
        expect(screen.queryByTestId("update-prices-results")).toBeNull();
    });
});
