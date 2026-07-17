import {afterEach, describe, expect, it, vi} from "vitest";
import {ApiUnreachableError} from "./client";
import {LedgelineApi, NATIVE_UNAVAILABLE_MESSAGE, NativeApiUnavailableError} from "./native";

const jsonResponse = (body: unknown): Response => new Response(JSON.stringify(body), {status: 200, headers: {"Content-Type": "application/json"}});

/** Last URL fetch() was called with. */
function lastUrl(fetchMock: ReturnType<typeof vi.fn>): string {
    return fetchMock.mock.calls[fetchMock.mock.calls.length - 1][0] as string;
}

describe("UNIT LedgelineApi — query building", () => {
    afterEach(() => vi.unstubAllGlobals());

    it("strips trailing slashes and builds the balance-sheet query", async () => {
        const fetchMock = vi.fn().mockResolvedValue(jsonResponse({sections: [], grandTotal: {}}));
        vi.stubGlobal("fetch", fetchMock);
        await new LedgelineApi("http://127.0.0.1:5000/").balanceSheet({asOf: "2026-07-08", depth: 2});
        expect(lastUrl(fetchMock)).toBe("http://127.0.0.1:5000/api/reports/balancesheet?asOf=2026-07-08&depth=2");
    });

    it("omits undefined and empty params", async () => {
        const fetchMock = vi.fn().mockResolvedValue(jsonResponse({sections: [], grandTotal: {}}));
        vi.stubGlobal("fetch", fetchMock);
        await new LedgelineApi("http://127.0.0.1:5000").balanceSheet({});
        expect(lastUrl(fetchMock)).toBe("http://127.0.0.1:5000/api/reports/balancesheet");
    });

    it("builds the holdings query, dropping an empty accounts set but keeping mode", async () => {
        const fetchMock = vi.fn().mockResolvedValue(jsonResponse({asOf: "x", base: "$", holdings: [], totals: {marketValue: {mantissa: "0", places: 0}}}));
        vi.stubGlobal("fetch", fetchMock);
        await new LedgelineApi("http://127.0.0.1:5000").holdings({asOf: "2026-07-08", accounts: "", mode: "exclude"});
        expect(lastUrl(fetchMock)).toBe("http://127.0.0.1:5000/api/holdings?asOf=2026-07-08&mode=exclude");
    });

    it("comma-joins subtree roots and adds the series window", async () => {
        const fetchMock = vi.fn().mockResolvedValue(jsonResponse({base: "$", points: [], hasBasis: false}));
        vi.stubGlobal("fetch", fetchMock);
        await new LedgelineApi("http://127.0.0.1:5000").holdingsSeries({accounts: "assets:broker,assets:ira", mode: "include", interval: "monthly", count: 12});
        expect(lastUrl(fetchMock)).toBe(
            "http://127.0.0.1:5000/api/holdings/series?accounts=assets%3Abroker%2Cassets%3Aira&mode=include&interval=monthly&count=12"
        );
    });
});

describe("UNIT LedgelineApi — error taxonomy", () => {
    afterEach(() => vi.unstubAllGlobals());

    it("maps a 404 (plain hledger-web, no /api/*) to NativeApiUnavailableError", async () => {
        vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response("not found", {status: 404, statusText: "Not Found"})));
        const promise = new LedgelineApi("http://127.0.0.1:5000").balanceSheet();
        await expect(promise).rejects.toBeInstanceOf(NativeApiUnavailableError);
        await expect(promise).rejects.toThrow(NATIVE_UNAVAILABLE_MESSAGE);
    });

    it("maps a 200 non-JSON body to NativeApiUnavailableError", async () => {
        vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response("<html></html>", {status: 200})));
        await expect(new LedgelineApi("http://127.0.0.1:5000").holdings()).rejects.toBeInstanceOf(NativeApiUnavailableError);
    });

    it("maps a network failure to ApiUnreachableError", async () => {
        vi.stubGlobal("fetch", vi.fn().mockRejectedValue(new TypeError("Failed to fetch")));
        await expect(new LedgelineApi("http://127.0.0.1:5000").netWorth()).rejects.toBeInstanceOf(ApiUnreachableError);
    });

    it("maps other non-2xx (e.g. 500) to ApiUnreachableError", async () => {
        vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response("boom", {status: 500, statusText: "Internal Server Error"})));
        await expect(new LedgelineApi("http://127.0.0.1:5000").cashFlow()).rejects.toBeInstanceOf(ApiUnreachableError);
    });
});
