import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import {intraday, MIN_INTERVAL_MS} from "./intraday.svelte";

const URL_BASE = "http://engine.test";
const T0 = 1_795_000_000_000;

let posts: {url: string; body: unknown}[] = [];
let reply: () => Response;

const json = (body: unknown, status = 200): Response => new Response(JSON.stringify(body), {status, headers: {"Content-Type": "application/json"}});
const QUOTED = {quotes: [{symbol: "VTI", price: "301.25", time: 1_795_000_000, currency: "USD"}], skipped: [], rateLimited: false};

beforeEach(() => {
    intraday.reset();
    posts = [];
    reply = () => json(QUOTED);
    vi.stubGlobal("fetch", (input: unknown, init?: RequestInit) => {
        posts.push({url: String(input), body: JSON.parse(String(init?.body ?? "null"))});
        return Promise.resolve(reply());
    });
});

afterEach(() => {
    vi.unstubAllGlobals();
});

describe("UNIT intraday store (plan 47 D10/D11)", () => {
    it("POSTs an empty body (every held symbol) and says quotes landed", async () => {
        expect(await intraday.refresh(URL_BASE, T0)).toBe(true);
        expect(posts).toEqual([{url: `${URL_BASE}/api/prices/intraday`, body: {}}]);
        expect(intraday.rateLimited).toBe(false);
    });

    it("asks a server at most once per interval", async () => {
        await intraday.refresh(URL_BASE, T0);
        expect(await intraday.refresh(URL_BASE, T0 + MIN_INTERVAL_MS - 1)).toBe(false);
        expect(posts).toHaveLength(1);
        await intraday.refresh(URL_BASE, T0 + MIN_INTERVAL_MS);
        expect(posts).toHaveLength(2);
    });

    it("never runs two at once", async () => {
        const first = intraday.refresh(URL_BASE, T0);
        expect(await intraday.refresh("http://other.test", T0)).toBe(false);
        await first;
        expect(posts).toHaveLength(1);
    });

    it("a rate-limited round with no quotes is nothing to reload, and says so", async () => {
        reply = () => json({quotes: [], skipped: ["VTI"], rateLimited: true});
        expect(await intraday.refresh(URL_BASE, T0)).toBe(false);
        expect(intraday.rateLimited).toBe(true);
    });

    it("a failure is silent: saved prices stand", async () => {
        reply = () => new Response("boom", {status: 500});
        expect(await intraday.refresh(URL_BASE, T0)).toBe(false);
    });

    it("an engine without the route is never asked again this session", async () => {
        reply = () => new Response("", {status: 404});
        expect(await intraday.refresh(URL_BASE, T0)).toBe(false);
        expect(await intraday.refresh(URL_BASE, T0 + 10 * MIN_INTERVAL_MS)).toBe(false);
        expect(posts).toHaveLength(1);
    });
});
