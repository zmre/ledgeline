import {afterEach, describe, expect, it, vi} from "vitest";
import {ApiUnreachableError} from "./client";
import {
    ConflictError,
    LedgelineApi,
    MAX_UPLOAD_BYTES,
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

    it("builds the grouped income-statement query on its own route, with no depth", async () => {
        const fetchMock = vi.fn().mockResolvedValue(jsonResponse({sections: []}));
        vi.stubGlobal("fetch", fetchMock);
        await new LedgelineApi("http://127.0.0.1:5000").incomeStatementGrouped({from: "2026-01-01", to: "2026-07-08"});
        // A SIBLING of `/api/reports/incomestatement`, which still exists and is
        // still byte-checked by the hledger parity golden. And no `depth`: this
        // report has no such control and the endpoint takes no such param.
        expect(lastUrl(fetchMock)).toBe("http://127.0.0.1:5000/api/reports/incomestatement/grouped?from=2026-01-01&to=2026-07-08");
    });

    it("omits value/valueIn/compare so the engine's own defaults apply", async () => {
        const fetchMock = vi.fn().mockResolvedValue(jsonResponse({sections: []}));
        vi.stubGlobal("fetch", fetchMock);
        await new LedgelineApi("http://127.0.0.1:5000").incomeStatementGrouped({});
        // The screen has no control for any of them, and sending them anyway
        // would pin the SPA to a base commodity it had to guess.
        expect(lastUrl(fetchMock)).toBe("http://127.0.0.1:5000/api/reports/incomestatement/grouped");
    });

    it("passes value/valueIn/compare through when a caller does set them", async () => {
        const fetchMock = vi.fn().mockResolvedValue(jsonResponse({sections: []}));
        vi.stubGlobal("fetch", fetchMock);
        await new LedgelineApi("http://127.0.0.1:5000").incomeStatementGrouped({
            from: "2026-01-01",
            to: "2026-07-08",
            value: "cost",
            valueIn: "€",
            compare: "none",
        });
        expect(lastUrl(fetchMock)).toBe(
            "http://127.0.0.1:5000/api/reports/incomestatement/grouped?from=2026-01-01&to=2026-07-08&value=cost&valueIn=%E2%82%AC&compare=none"
        );
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

    it("builds the budget query at the top-level /api/budget route (not under /api/reports/)", async () => {
        const fetchMock = vi.fn().mockResolvedValue(jsonResponse({buckets: [], rows: [], totals: []}));
        vi.stubGlobal("fetch", fetchMock);
        await new LedgelineApi("http://127.0.0.1:5000").budget({end: "2026-07-31", interval: "monthly", count: 7, depth: 2});
        expect(lastUrl(fetchMock)).toBe("http://127.0.0.1:5000/api/budget?end=2026-07-31&interval=monthly&count=7&depth=2");
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

    // The read path used to report the STATUS LINE and drop the body unread,
    // which threw away the only actionable half of a journal-authoring mistake.
    it("carries the engine's own sentence on a 4xx, so a bad `holdings:` tag reaches the user", async () => {
        const sentence = "account 'assets:property:house' declares `holdings: hous`, which is not one of stocks, other, none";
        vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response(sentence, {status: 400, statusText: "Bad Request"})));

        await expect(new LedgelineApi("http://127.0.0.1:5000").otherHoldings({asOf: "2026-07-08"})).rejects.toThrow(sentence);
    });

    it("falls back to the status line when the body is empty", async () => {
        vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response("   ", {status: 503, statusText: "Service Unavailable"})));

        await expect(new LedgelineApi("http://127.0.0.1:5000").otherHoldings()).rejects.toThrow(/responded 503 Service Unavailable/);
    });

    it("refuses an HTML error page from a proxy rather than putting markup in an alert", async () => {
        const page = "<!doctype html><html><body><h1>502 Bad Gateway</h1></body></html>";
        vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response(page, {status: 502, statusText: "Bad Gateway"})));
        const promise = new LedgelineApi("http://127.0.0.1:5000").otherHoldingsSeries();

        await expect(promise).rejects.toThrow(/responded 502 Bad Gateway/);
        await expect(promise).rejects.not.toThrow(/doctype/);
    });

    it("refuses a body too long to read, rather than truncating it into a half sentence", async () => {
        vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response("x".repeat(501), {status: 400, statusText: "Bad Request"})));

        await expect(new LedgelineApi("http://127.0.0.1:5000").otherHoldings()).rejects.toThrow(/responded 400 Bad Request/);
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

    it("PATCHes a status-only body to /api/transactions/{index}", async () => {
        const fetchMock = vi.fn().mockResolvedValue(mutationResponse(3));
        vi.stubGlobal("fetch", fetchMock);
        await new LedgelineApi("http://127.0.0.1:5000").patchTransaction(3, {status: "cleared"});
        const init = lastInit(fetchMock);
        expect(init.method).toBe("PATCH");
        expect(JSON.parse(init.body as string)).toEqual({status: "cleared"});
    });

    it("PUTs a replace body carrying date2, comment/tags, and per-posting status + comment", async () => {
        const fetchMock = vi.fn().mockResolvedValue(mutationResponse(3));
        vi.stubGlobal("fetch", fetchMock);
        const body: AddTransactionBody = {
            date: "2026-07-20",
            date2: "2026-07-22",
            comment: "note, category:food",
            postings: [
                {account: "expenses:food", status: "cleared", comment: "on sale", amount: {commodity: "$", quantity: {mantissa: "500", places: 2}}},
                {account: "assets:cash"},
            ],
        };
        await new LedgelineApi("http://127.0.0.1:5000").replaceTransaction(3, body);
        expect(JSON.parse(lastInit(fetchMock).body as string)).toEqual(body);
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

describe("UNIT LedgelineApi — the import upload path", () => {
    afterEach(() => vi.unstubAllGlobals());

    const bytes = (size: number): Uint8Array => new Uint8Array(size);

    it("POSTs raw bytes with the filename in a header, not a JSON body", async () => {
        // `mutate` sends `JSON.stringify(body)`; a workbook is not JSON, and
        // base64-ing 16 MiB through one would cost a third again in transfer.
        const fetchMock = vi.fn().mockResolvedValue(jsonResponse({stageId: "t"}));
        vi.stubGlobal("fetch", fetchMock);
        await new LedgelineApi("http://127.0.0.1:5000").stageImport("bank.csv", bytes(8));
        expect(lastUrl(fetchMock)).toBe("http://127.0.0.1:5000/api/import/stage");
        const init = lastInit(fetchMock);
        expect(init.method).toBe("POST");
        expect((init.headers as Record<string, string>)["Content-Type"]).toBe("application/octet-stream");
        expect((init.headers as Record<string, string>)["X-Ledgeline-Filename"]).toBe("bank.csv");
        expect(init.body).toBeInstanceOf(Uint8Array);
    });

    it("refuses an over-size file locally rather than uploading it to be refused", async () => {
        const fetchMock = vi.fn();
        vi.stubGlobal("fetch", fetchMock);
        await expect(new LedgelineApi("http://127.0.0.1:5000").stageImport("big.xlsx", bytes(MAX_UPLOAD_BYTES + 1))).rejects.toBeInstanceOf(ValidationError);
        expect(fetchMock).not.toHaveBeenCalled();
    });

    it("refuses an empty file without a round trip", async () => {
        const fetchMock = vi.fn();
        vi.stubGlobal("fetch", fetchMock);
        await expect(new LedgelineApi("http://127.0.0.1:5000").stageImport("empty.csv", bytes(0))).rejects.toBeInstanceOf(ValidationError);
        expect(fetchMock).not.toHaveBeenCalled();
    });

    it("maps the engine's 413 to a ValidationError carrying its own sentence", async () => {
        // The local size check is courtesy; `DefaultBodyLimit` is the enforcement.
        vi.stubGlobal("fetch", vi.fn().mockResolvedValue(textResponse("body too large", 413)));
        await expect(new LedgelineApi("http://127.0.0.1:5000").stageImport("bank.csv", bytes(8))).rejects.toThrow("body too large");
    });

    it("maps a 400 from stage onto the same taxonomy every other write uses", async () => {
        vi.stubGlobal("fetch", vi.fn().mockResolvedValue(textResponse("PDF conversion is not supported yet", 400)));
        await expect(new LedgelineApi("http://127.0.0.1:5000").stageImport("s.pdf", bytes(8))).rejects.toBeInstanceOf(ValidationError);
    });

    it("maps an unreachable engine on the upload path to ApiUnreachableError", async () => {
        vi.stubGlobal("fetch", vi.fn().mockRejectedValue(new TypeError("Failed to fetch")));
        await expect(new LedgelineApi("http://127.0.0.1:5000").stageImport("bank.csv", bytes(8))).rejects.toBeInstanceOf(ApiUnreachableError);
    });
});

describe("UNIT LedgelineApi — the rest of the import routes", () => {
    afterEach(() => vi.unstubAllGlobals());

    const RUN_BODY = {stageId: "t", rulesId: "bank.csv.rules", csvPath: "import/bank.csv", journalId: "2026.journal", balance: null, balanceAccount: null};

    it("GETs the capabilities probe", async () => {
        const fetchMock = vi.fn().mockResolvedValue(jsonResponse({journals: []}));
        vi.stubGlobal("fetch", fetchMock);
        await new LedgelineApi("http://127.0.0.1:5000").importCapabilities();
        expect(lastUrl(fetchMock)).toBe("http://127.0.0.1:5000/api/import/capabilities");
    });

    it("treats a 404 on capabilities as 'this server is not the engine'", async () => {
        // An older engine has no /api/import/* at all, and the whole tab has to
        // say so rather than spin.
        vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response("", {status: 404})));
        await expect(new LedgelineApi("http://127.0.0.1:5000").importCapabilities()).rejects.toBeInstanceOf(NativeApiUnavailableError);
    });

    it("POSTs the dry run and the commit as JSON", async () => {
        // A fresh Response per call: a body can only be read once.
        const fetchMock = vi.fn().mockImplementation(() => Promise.resolve(jsonResponse({ok: true})));
        vi.stubGlobal("fetch", fetchMock);
        const api = new LedgelineApi("http://127.0.0.1:5000");
        await api.importDryRun(RUN_BODY);
        expect(lastUrl(fetchMock)).toBe("http://127.0.0.1:5000/api/import/dry-run");
        expect(JSON.parse(lastInit(fetchMock).body as string)).toEqual(RUN_BODY);
        await api.importCommit({...RUN_BODY, writeAssertion: true});
        expect(lastUrl(fetchMock)).toBe("http://127.0.0.1:5000/api/import/commit");
        expect(JSON.parse(lastInit(fetchMock).body as string).writeAssertion).toBe(true);
    });

    it("POSTs the save-csv path with a two-field body and no rules file at all", async () => {
        // The no-rules-file path is its OWN route: a dry-run with no rules file
        // has nothing to propose, so `ImportRunBody`'s handles are not nullable
        // and this request cannot be expressed as a commit.
        const fetchMock = vi.fn().mockResolvedValue(jsonResponse({csvWritten: "import/bank.csv", git: null}));
        vi.stubGlobal("fetch", fetchMock);
        await new LedgelineApi("http://127.0.0.1:5000").importSaveCsv({stageId: "t", csvPath: "import/bank.csv"});
        expect(lastUrl(fetchMock)).toBe("http://127.0.0.1:5000/api/import/save-csv");
        expect(JSON.parse(lastInit(fetchMock).body as string)).toEqual({stageId: "t", csvPath: "import/bank.csv"});
    });

    it("POSTs the confirmed re-sort with just the journal id", async () => {
        const fetchMock = vi.fn().mockResolvedValue(jsonResponse({moved: 3}));
        vi.stubGlobal("fetch", fetchMock);
        await new LedgelineApi("http://127.0.0.1:5000").importSort("2026/2026.journal");
        expect(lastUrl(fetchMock)).toBe("http://127.0.0.1:5000/api/import/sort");
        expect(JSON.parse(lastInit(fetchMock).body as string)).toEqual({journalId: "2026/2026.journal"});
    });

    it("reads and writes the preferences blob, keeping nulls as nulls", async () => {
        const fetchMock = vi.fn().mockImplementation(() => Promise.resolve(jsonResponse({hledgerPath: null, gitAutocommit: null})));
        vi.stubGlobal("fetch", fetchMock);
        const api = new LedgelineApi("http://127.0.0.1:5000");
        await api.getPrefs();
        expect(lastUrl(fetchMock)).toBe("http://127.0.0.1:5000/api/prefs");
        await api.putPrefs({hledgerPath: "/usr/bin/hledger", gitAutocommit: null});
        expect(lastInit(fetchMock).method).toBe("PUT");
        expect(JSON.parse(lastInit(fetchMock).body as string)).toEqual({hledgerPath: "/usr/bin/hledger", gitAutocommit: null});
    });

    it("surfaces the engine's 400 when it rejects a stored hledger path", async () => {
        // Validated at store time, so a bad value is a refusal here rather than
        // a persisted value that fails on the next import.
        vi.stubGlobal("fetch", vi.fn().mockResolvedValue(textResponse("/nope is not an executable file", 400)));
        await expect(new LedgelineApi("http://127.0.0.1:5000").putPrefs({hledgerPath: "/nope", gitAutocommit: null})).rejects.toThrow("not an executable");
    });
});
