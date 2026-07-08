import {afterEach, describe, expect, it, vi} from "vitest";
import {ApiShapeError, ApiUnreachableError, HledgerApi} from "./client";

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
