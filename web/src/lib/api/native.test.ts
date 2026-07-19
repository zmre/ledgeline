import {afterEach, describe, expect, it, vi} from "vitest";
import {ApiUnreachableError} from "./client";
import {
    ConflictError,
    LedgelineApi,
    NATIVE_UNAVAILABLE_MESSAGE,
    NativeApiUnavailableError,
    NotFoundError,
    ValidationError,
    type AddTransactionBody,
} from "./native";

const jsonResponse = (body: unknown): Response => new Response(JSON.stringify(body), {status: 200, headers: {"Content-Type": "application/json"}});

/** Last URL fetch() was called with. */
function lastUrl(fetchMock: ReturnType<typeof vi.fn>): string {
    return fetchMock.mock.calls[fetchMock.mock.calls.length - 1][0] as string;
}

/** The RequestInit fetch() was last called with. */
function lastInit(fetchMock: ReturnType<typeof vi.fn>): RequestInit {
    return fetchMock.mock.calls[fetchMock.mock.calls.length - 1][1] as RequestInit;
}

/** A plain-text error response (the write endpoints answer with text bodies). */
const textResponse = (body: string, status: number): Response => new Response(body, {status, statusText: body});

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

    it("omits gainSince for the all-time window (no param when undefined)", async () => {
        const fetchMock = vi.fn().mockResolvedValue(jsonResponse({asOf: "x", base: "$", holdings: [], totals: {marketValue: {mantissa: "0", places: 0}}}));
        vi.stubGlobal("fetch", fetchMock);
        await new LedgelineApi("http://127.0.0.1:5000").holdings({asOf: "2026-07-08", accounts: "", mode: "include", gainSince: undefined});
        expect(lastUrl(fetchMock)).toBe("http://127.0.0.1:5000/api/holdings?asOf=2026-07-08&mode=include");
    });

    it("appends gainSince when a window start is set", async () => {
        const fetchMock = vi.fn().mockResolvedValue(jsonResponse({asOf: "x", base: "$", holdings: [], totals: {marketValue: {mantissa: "0", places: 0}}}));
        vi.stubGlobal("fetch", fetchMock);
        await new LedgelineApi("http://127.0.0.1:5000").holdings({asOf: "2026-07-08", accounts: "", mode: "include", gainSince: "2026-01-01"});
        expect(lastUrl(fetchMock)).toBe("http://127.0.0.1:5000/api/holdings?asOf=2026-07-08&mode=include&gainSince=2026-01-01");
    });

    it("does NOT window the series endpoint even when a gainSince is passed", async () => {
        const fetchMock = vi.fn().mockResolvedValue(jsonResponse({base: "$", points: [], hasBasis: false}));
        vi.stubGlobal("fetch", fetchMock);
        await new LedgelineApi("http://127.0.0.1:5000").holdingsSeries({
            asOf: "2026-07-08",
            mode: "include",
            interval: "monthly",
            count: 12,
            gainSince: "2026-01-01",
        });
        expect(lastUrl(fetchMock)).toBe("http://127.0.0.1:5000/api/holdings/series?asOf=2026-07-08&mode=include&interval=monthly&count=12");
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

// ===========================================================================
// Write path (edit endpoints)
// ===========================================================================

const ADD_BODY: AddTransactionBody = {
    date: "2026-07-20",
    status: "cleared",
    description: "Safeway | groceries",
    postings: [{account: "expenses:food:groceries", amount: {commodity: "$", quantity: {mantissa: "5624", places: 2}}}, {account: "liabilities:cc:visa"}],
};

/** A 201/200 mutation response body (the added/edited transaction + its index). */
const mutationResponse = (index: number): Response =>
    new Response(
        JSON.stringify({index, transaction: {index, date: "2026-07-20", status: "cleared", code: "", description: "Safeway | groceries", postings: []}}),
        {
            status: index === 0 ? 201 : 200,
            headers: {"Content-Type": "application/json"},
        }
    );

describe("UNIT LedgelineApi — write requests", () => {
    afterEach(() => vi.unstubAllGlobals());

    it("POSTs the add body as JSON to /api/transactions and returns the parsed 201", async () => {
        const fetchMock = vi.fn().mockResolvedValue(new Response(JSON.stringify({index: 7, transaction: {index: 7}}), {status: 201}));
        vi.stubGlobal("fetch", fetchMock);
        const result = await new LedgelineApi("http://127.0.0.1:5000/").addTransaction(ADD_BODY);
        expect(lastUrl(fetchMock)).toBe("http://127.0.0.1:5000/api/transactions");
        const init = lastInit(fetchMock);
        expect(init.method).toBe("POST");
        expect((init.headers as Record<string, string>)["Content-Type"]).toBe("application/json");
        expect(JSON.parse(init.body as string)).toEqual(ADD_BODY);
        expect(result.index).toBe(7);
    });

    it("PUTs the replace body to /api/transactions/{index}", async () => {
        const fetchMock = vi.fn().mockResolvedValue(mutationResponse(3));
        vi.stubGlobal("fetch", fetchMock);
        await new LedgelineApi("http://127.0.0.1:5000").replaceTransaction(3, ADD_BODY);
        expect(lastUrl(fetchMock)).toBe("http://127.0.0.1:5000/api/transactions/3");
        expect(lastInit(fetchMock).method).toBe("PUT");
    });

    it("PATCHes only the changed fields to /api/transactions/{index}", async () => {
        const fetchMock = vi.fn().mockResolvedValue(mutationResponse(3));
        vi.stubGlobal("fetch", fetchMock);
        await new LedgelineApi("http://127.0.0.1:5000").patchTransaction(3, {postings: [{index: 0, account: "expenses:dining"}]});
        expect(lastUrl(fetchMock)).toBe("http://127.0.0.1:5000/api/transactions/3");
        const init = lastInit(fetchMock);
        expect(init.method).toBe("PATCH");
        expect(JSON.parse(init.body as string)).toEqual({postings: [{index: 0, account: "expenses:dining"}]});
    });

    it("DELETEs /api/transactions/{index} (no body) and returns the parsed result", async () => {
        const fetchMock = vi.fn().mockResolvedValue(new Response(JSON.stringify({deletedIndex: 2, remaining: 5}), {status: 200}));
        vi.stubGlobal("fetch", fetchMock);
        const result = await new LedgelineApi("http://127.0.0.1:5000").deleteTransaction(2);
        expect(lastUrl(fetchMock)).toBe("http://127.0.0.1:5000/api/transactions/2");
        const init = lastInit(fetchMock);
        expect(init.method).toBe("DELETE");
        expect(init.body).toBeUndefined();
        expect(result).toEqual({deletedIndex: 2, remaining: 5});
    });
});

describe("UNIT LedgelineApi — write error taxonomy", () => {
    afterEach(() => vi.unstubAllGlobals());

    it("maps 400 to ValidationError carrying the server's plain-text message", async () => {
        vi.stubGlobal("fetch", vi.fn().mockResolvedValue(textResponse("transaction is unbalanced", 400)));
        const promise = new LedgelineApi("http://127.0.0.1:5000").addTransaction(ADD_BODY);
        await expect(promise).rejects.toBeInstanceOf(ValidationError);
        await expect(promise).rejects.toThrow("transaction is unbalanced");
    });

    it("maps 409 to ConflictError", async () => {
        vi.stubGlobal("fetch", vi.fn().mockResolvedValue(textResponse("the journal changed on disk", 409)));
        await expect(new LedgelineApi("http://127.0.0.1:5000").deleteTransaction(2)).rejects.toBeInstanceOf(ConflictError);
    });

    it("maps 404 to NotFoundError", async () => {
        vi.stubGlobal("fetch", vi.fn().mockResolvedValue(textResponse("transaction 99 not found", 404)));
        await expect(new LedgelineApi("http://127.0.0.1:5000").patchTransaction(99, {description: "x"})).rejects.toBeInstanceOf(NotFoundError);
    });

    it("maps 501 (editing disabled) to NativeApiUnavailableError", async () => {
        vi.stubGlobal("fetch", vi.fn().mockResolvedValue(textResponse("editing is not enabled", 501)));
        await expect(new LedgelineApi("http://127.0.0.1:5000").addTransaction(ADD_BODY)).rejects.toBeInstanceOf(NativeApiUnavailableError);
    });

    it("maps a network failure to ApiUnreachableError", async () => {
        vi.stubGlobal("fetch", vi.fn().mockRejectedValue(new TypeError("Failed to fetch")));
        await expect(new LedgelineApi("http://127.0.0.1:5000").deleteTransaction(1)).rejects.toBeInstanceOf(ApiUnreachableError);
    });

    it("uses the fallback message when the error body is empty", async () => {
        vi.stubGlobal("fetch", vi.fn().mockResolvedValue(textResponse("", 400)));
        await expect(new LedgelineApi("http://127.0.0.1:5000").addTransaction(ADD_BODY)).rejects.toThrow("The transaction is invalid.");
    });
});

describe("UNIT LedgelineApi — editing probe", () => {
    afterEach(() => vi.unstubAllGlobals());

    it("reports available when GET /api/transactions is 405 (route present, method not GET)", async () => {
        const fetchMock = vi.fn().mockResolvedValue(new Response("", {status: 405, statusText: "Method Not Allowed"}));
        vi.stubGlobal("fetch", fetchMock);
        await expect(new LedgelineApi("http://127.0.0.1:5000").probeEditing()).resolves.toBe(true);
        expect(lastUrl(fetchMock)).toBe("http://127.0.0.1:5000/api/transactions");
        expect(lastInit(fetchMock).method).toBe("GET");
    });

    it("reports unavailable when the route 404s (plain hledger-web / SPA fallback)", async () => {
        vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response("", {status: 404})));
        await expect(new LedgelineApi("http://127.0.0.1:5000").probeEditing()).resolves.toBe(false);
    });

    it("rejects (unreachable) so the caller can degrade to read-only", async () => {
        vi.stubGlobal("fetch", vi.fn().mockRejectedValue(new TypeError("Failed to fetch")));
        await expect(new LedgelineApi("http://127.0.0.1:5000").probeEditing()).rejects.toBeInstanceOf(ApiUnreachableError);
    });
});
