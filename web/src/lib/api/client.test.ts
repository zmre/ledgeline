import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import {
    ApiShapeError,
    ApiTimeoutError,
    ApiUnreachableError,
    HledgerApi,
    isNotModified,
    REQUEST_TIMEOUT_MS,
    resetConditionalCache,
    SETTINGS_STORAGE_KEY,
} from "./client";

const jsonResponse = (body: unknown): Response => new Response(JSON.stringify(body), {status: 200, headers: {"Content-Type": "application/json"}});

describe("UNIT HledgerApi", () => {
    afterEach(() => {
        vi.unstubAllGlobals();
    });

    it("strips trailing slashes from the base URL", async () => {
        const fetchMock = vi.fn().mockResolvedValue(jsonResponse("1.52"));
        vi.stubGlobal("fetch", fetchMock);
        const api = new HledgerApi("http://127.0.0.1:5000/");
        await expect(api.version()).resolves.toBe("1.52");
        expect(fetchMock).toHaveBeenCalledWith("http://127.0.0.1:5000/version", expect.anything());
    });

    it("throws ApiUnreachableError on network/CORS failure", async () => {
        vi.stubGlobal("fetch", vi.fn().mockRejectedValue(new TypeError("Failed to fetch")));
        const api = new HledgerApi("http://127.0.0.1:5000");
        await expect(api.version()).rejects.toBeInstanceOf(ApiUnreachableError);
    });

    it("throws ApiUnreachableError on non-2xx responses", async () => {
        vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response("not found", {status: 404, statusText: "Not Found"})));
        const api = new HledgerApi("http://127.0.0.1:5000");
        await expect(api.transactions()).rejects.toBeInstanceOf(ApiUnreachableError);
    });

    it("throws ApiShapeError on non-JSON bodies", async () => {
        vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response("<html></html>", {status: 200})));
        const api = new HledgerApi("http://127.0.0.1:5000");
        await expect(api.version()).rejects.toBeInstanceOf(ApiShapeError);
    });

    it("throws ApiShapeError when /version is not a string", async () => {
        vi.stubGlobal("fetch", vi.fn().mockResolvedValue(jsonResponse({version: "1.52"})));
        const api = new HledgerApi("http://127.0.0.1:5000");
        await expect(api.version()).rejects.toBeInstanceOf(ApiShapeError);
    });

    it("validates /accountnames and /commodities as string arrays", async () => {
        const fetchMock = vi
            .fn()
            .mockResolvedValueOnce(jsonResponse(["assets", "expenses"]))
            .mockResolvedValueOnce(jsonResponse([1, 2]));
        vi.stubGlobal("fetch", fetchMock);
        const api = new HledgerApi("http://127.0.0.1:5000");
        await expect(api.accountNames()).resolves.toEqual(["assets", "expenses"]);
        await expect(api.commodities()).rejects.toBeInstanceOf(ApiShapeError);
    });

    it("returns raw unknown JSON for /transactions and /prices", async () => {
        const fetchMock = vi
            .fn()
            .mockResolvedValueOnce(jsonResponse([{tindex: 1}]))
            .mockResolvedValueOnce(jsonResponse([{mpdate: "2026-01-01"}]));
        vi.stubGlobal("fetch", fetchMock);
        const api = new HledgerApi("http://127.0.0.1:5000");
        await expect(api.transactions()).resolves.toEqual([{tindex: 1}]);
        await expect(api.prices()).resolves.toEqual([{mpdate: "2026-01-01"}]);
    });
});

/**
 * The request deadline (FE-5f) and the conditional GET (PERF-2) share the same
 * few lines, and getting them wrong together is worse than either alone: an
 * `ETag` recorded off the response HEADERS survives a body read that is then
 * aborted, and the next poll replays that tag, is told 304, and keeps the OLDER
 * journal it still has while believing it current. So the tag is only recorded
 * once the body is in hand, and these tests pin both halves.
 */
describe("UNIT HledgerApi deadlines and conditional GET", () => {
    /** 200 + ETag; `body` may reject to model a read that was cut off mid-stream. */
    const tagged = (etag: string, body: () => Promise<unknown>): Response =>
        ({status: 200, ok: true, headers: new Headers({ETag: etag}), json: body}) as unknown as Response;

    const lastInit = (mock: ReturnType<typeof vi.fn>): RequestInit => mock.mock.calls[mock.mock.calls.length - 1][1] as RequestInit;
    const headersOf = (mock: ReturnType<typeof vi.fn>): Record<string, string> => (lastInit(mock).headers ?? {}) as Record<string, string>;

    beforeEach(() => {
        resetConditionalCache();
    });
    afterEach(() => {
        vi.unstubAllGlobals();
        vi.useRealTimers();
        resetConditionalCache();
    });

    it("replays the recorded ETag and reads a 304 as NOT_MODIFIED", async () => {
        const fetchMock = vi
            .fn()
            .mockResolvedValueOnce(tagged('"v1"', () => Promise.resolve([{tindex: 1}])))
            .mockResolvedValueOnce({status: 304, ok: false, headers: new Headers()} as unknown as Response);
        vi.stubGlobal("fetch", fetchMock);

        const api = new HledgerApi("http://127.0.0.1:5000", undefined, {conditional: true});
        await expect(api.transactions()).resolves.toEqual([{tindex: 1}]);
        expect(headersOf(fetchMock)["If-None-Match"]).toBeUndefined();

        expect(isNotModified(await api.transactions())).toBe(true);
        expect(headersOf(fetchMock)["If-None-Match"]).toBe('"v1"');
    });

    it("stays unconditional for a client that did not opt in", async () => {
        const fetchMock = vi.fn().mockResolvedValue(tagged('"v1"', () => Promise.resolve([])));
        vi.stubGlobal("fetch", fetchMock);
        const api = new HledgerApi("http://127.0.0.1:5000");
        await api.transactions();
        await api.transactions();
        expect(headersOf(fetchMock)["If-None-Match"]).toBeUndefined();
    });

    it("does not record an ETag for a body it never finished reading", async () => {
        // What an aborted or timed-out read looks like: headers (with the tag)
        // arrived, the body did not.
        const fetchMock = vi
            .fn()
            .mockResolvedValueOnce(tagged('"v2"', () => Promise.reject(new DOMException("aborted", "AbortError"))))
            .mockResolvedValueOnce(tagged('"v2"', () => Promise.resolve([{tindex: 1}])));
        vi.stubGlobal("fetch", fetchMock);

        const api = new HledgerApi("http://127.0.0.1:5000", undefined, {conditional: true});
        await expect(api.transactions()).rejects.toThrow(ApiShapeError);
        // The retry must ask unconditionally — we hold nothing that '"v2"' describes.
        await expect(api.transactions()).resolves.toEqual([{tindex: 1}]);
        expect(fetchMock.mock.calls.every((call) => (call[1] as RequestInit & {headers: Record<string, string>}).headers["If-None-Match"] === undefined)).toBe(
            true
        );
    });

    it("passes an abort signal to fetch and fails a response that never arrives", async () => {
        vi.useFakeTimers();
        const fetchMock = vi.fn().mockImplementation(
            (_url: string, init: RequestInit) =>
                new Promise((_resolve, reject) => {
                    init.signal?.addEventListener("abort", () => reject(new DOMException("aborted", "AbortError")), {once: true});
                })
        );
        vi.stubGlobal("fetch", fetchMock);

        const api = new HledgerApi("http://127.0.0.1:5000");
        const pending = api.transactions();
        const assertion = expect(pending).rejects.toThrow(ApiTimeoutError);
        await vi.advanceTimersByTimeAsync(REQUEST_TIMEOUT_MS + 1);
        await assertion;
        expect(lastInit(fetchMock).signal).toBeInstanceOf(AbortSignal);
    });

    it("cancels an in-flight request when the caller's signal aborts", async () => {
        const controller = new AbortController();
        const fetchMock = vi.fn().mockImplementation(
            (_url: string, init: RequestInit) =>
                new Promise((_resolve, reject) => {
                    init.signal?.addEventListener("abort", () => reject(new DOMException("aborted", "AbortError")), {once: true});
                })
        );
        vi.stubGlobal("fetch", fetchMock);

        const api = new HledgerApi("http://127.0.0.1:5000", undefined, {signal: controller.signal});
        const pending = api.transactions();
        const assertion = expect(pending).rejects.toThrow(ApiUnreachableError);
        controller.abort();
        await assertion;
    });
});

/**
 * SEC-16. `authHeaders` attached the ambient token to whatever URL it was given,
 * and `HledgerApi` is constructed with an arbitrary base URL by
 * `settings.setServerUrl` — the setup modal's "verify this server" button. So
 * typing any address there with an empty token box sent the running engine's
 * credential to that address.
 *
 * These tests stand up a minimal browser-shaped global (`window` + a
 * `localStorage` the client reads the persisted blob out of), because in the
 * node project neither exists and the origin check correctly stands aside.
 */
describe("UNIT access-token targeting", () => {
    const PAGE_ORIGIN = "http://localhost:4173";
    const ENGINE = "http://127.0.0.1:5099";
    const TOKEN = "ledgeline-e2e-token";

    /** A `localStorage` shim holding one settings blob. */
    const storageWith = (settings: Record<string, unknown>): Storage =>
        ({
            getItem: (key: string) => (key === SETTINGS_STORAGE_KEY ? JSON.stringify(settings) : null),
        }) as unknown as Storage;

    /** Put the page on `PAGE_ORIGIN` with `settings` persisted. */
    function browserAt(settings: Record<string, unknown>): void {
        vi.stubGlobal("window", {location: {origin: PAGE_ORIGIN, href: `${PAGE_ORIGIN}/`}});
        vi.stubGlobal("localStorage", storageWith(settings));
    }

    afterEach(() => {
        vi.unstubAllGlobals();
    });

    it("sends the token to the configured engine, cross-origin (the vite-dev / e2e flow)", async () => {
        browserAt({serverUrl: ENGINE, serverToken: TOKEN});
        const fetchMock = vi.fn().mockResolvedValue(jsonResponse("1.52"));
        vi.stubGlobal("fetch", fetchMock);

        await new HledgerApi(ENGINE).version();
        const headers = (fetchMock.mock.calls[0][1] as RequestInit).headers as Record<string, string>;
        expect(headers.Authorization).toBe(`Bearer ${TOKEN}`);
    });

    it("sends the token to its own origin (the packaged, same-origin app)", async () => {
        browserAt({serverUrl: PAGE_ORIGIN, serverToken: TOKEN});
        const fetchMock = vi.fn().mockResolvedValue(jsonResponse("1.52"));
        vi.stubGlobal("fetch", fetchMock);

        await new HledgerApi(PAGE_ORIGIN).version();
        const headers = (fetchMock.mock.calls[0][1] as RequestInit).headers as Record<string, string>;
        expect(headers.Authorization).toBe(`Bearer ${TOKEN}`);
    });

    it("never sends the token to any other origin", async () => {
        browserAt({serverUrl: ENGINE, serverToken: TOKEN});
        // A FRESH Response per call: a `Response` body may only be read once.
        const fetchMock = vi.fn().mockImplementation(() => Promise.resolve(jsonResponse("1.52")));
        vi.stubGlobal("fetch", fetchMock);

        // Every one of these is reachable by typing it into the setup modal.
        for (const hostile of ["http://evil.example", "https://evil.example", "http://127.0.0.1:5100", "http://localhost:4173.evil.example"]) {
            await new HledgerApi(hostile).version();
            const headers = (fetchMock.mock.calls.at(-1)?.[1] as RequestInit).headers as Record<string, string>;
            expect(headers.Authorization, `${hostile} must not receive the token`).toBeUndefined();
        }
    });

    it("still sends a token the caller passed explicitly", async () => {
        browserAt({serverUrl: ENGINE, serverToken: TOKEN});
        const fetchMock = vi.fn().mockResolvedValue(jsonResponse("1.52"));
        vi.stubGlobal("fetch", fetchMock);

        // The setup modal verifying a candidate server WITH a candidate token:
        // the user typed both, so both are theirs to send.
        await new HledgerApi("http://127.0.0.1:5100", "a-token-the-user-typed").version();
        const headers = (fetchMock.mock.calls[0][1] as RequestInit).headers as Record<string, string>;
        expect(headers.Authorization).toBe("Bearer a-token-the-user-typed");
    });
});
