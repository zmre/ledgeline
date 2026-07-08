// INTEGRATION check against a live hledger-web (--serve-api). Skipped unless
// LEDGELINE_API_URL is set, e.g.:
//   hledger-web -f FILE --serve-api --cors='*' --allow=view --port 5055
//   LEDGELINE_API_URL=http://127.0.0.1:5055 vitest run
// Covers the setup-modal contract end-to-end at the logic level: verify
// GET /version, persist the URL, then fetch + normalize the whole journal.

import {describe, expect, it} from "vitest";
import {HledgerApi} from "./client";
import {normalizePrices, normalizeTransactions} from "./normalize";

const apiUrl = process.env.LEDGELINE_API_URL;

function stubLocalStorage(): Map<string, string> {
    const backing = new Map<string, string>();
    const stub = {
        getItem: (key: string) => backing.get(key) ?? null,
        setItem: (key: string, value: string) => void backing.set(key, value),
        removeItem: (key: string) => void backing.delete(key),
        clear: () => backing.clear(),
        key: (index: number) => [...backing.keys()][index] ?? null,
        get length() {
            return backing.size;
        },
    };
    globalThis.localStorage = stub as unknown as Storage;
    return backing;
}

describe.runIf(apiUrl !== undefined && apiUrl !== "")("INTEGRATION live hledger-web api", () => {
    const url = apiUrl ?? "";

    it("reports a version string", async () => {
        const version = await new HledgerApi(url).version();
        expect(version).toMatch(/^\d+\.\d+/);
    });

    it("fetches and normalizes transactions, prices, account names, commodities", async () => {
        const api = new HledgerApi(url);
        const txns = normalizeTransactions(await api.transactions());
        expect(txns.length).toBeGreaterThan(0);
        for (const txn of txns) {
            expect(txn.date).toMatch(/^\d{4}-\d{2}-\d{2}$/);
            expect(Object.isFrozen(txn)).toBe(true);
            expect(txn.haystack).toBe(txn.haystack.toLowerCase());
        }
        const prices = normalizePrices(await api.prices());
        for (const price of prices) {
            expect(price.date).toMatch(/^\d{4}-\d{2}-\d{2}$/);
            expect(typeof price.price.qty.m).toBe("bigint");
        }
        const names = await api.accountNames();
        expect(names.length).toBeGreaterThan(0);
        const commodities = await api.commodities();
        expect(commodities.length).toBeGreaterThan(0);
    });

    it("settings.setServerUrl verifies /version and persists under ledgeline.settings.v1", async () => {
        const backing = stubLocalStorage();
        const {settings} = await import("$lib/stores/settings.svelte");
        expect(settings.serverUrl).toBeNull();
        await settings.setServerUrl(`${url}/`);
        expect(settings.serverUrl).toBe(url);
        const persisted = JSON.parse(backing.get("ledgeline.settings.v1") ?? "{}") as {serverUrl?: string};
        expect(persisted.serverUrl).toBe(url);
    });

    it("settings.setServerUrl rejects an unreachable URL without persisting", async () => {
        const backing = stubLocalStorage();
        const {settings} = await import("$lib/stores/settings.svelte");
        await expect(settings.setServerUrl("http://127.0.0.1:59999")).rejects.toThrow();
        expect(backing.has("ledgeline.settings.v1")).toBe(false);
    });
});
