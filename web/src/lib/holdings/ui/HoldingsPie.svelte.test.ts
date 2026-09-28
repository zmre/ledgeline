// The holdings pie, mounted against the REAL `holdingsProfiles` store and
// `settings`, with only `fetch` stubbed (the `FAKE_ENGINE` convention — see
// `UpdatePricesButton.svelte.test.ts`): what is pinned is what the pie does
// with the engine's actual wire JSON.
//
// The store is a module singleton that keeps its last answer, which is the
// behaviour under test in "holds the previous answer". The one test that needs
// NO previous answer therefore runs first.

import {fireEvent, render, screen} from "@testing-library/svelte";
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import {dec, type Dec} from "$lib/domain/money";
import type {Holding} from "$lib/holdings/types";
import {settings} from "$lib/stores/settings.svelte";
import {connectFakeEngine, FAKE_ENGINE} from "$lib/testing/fakeEngine";
import HoldingsPie from "./HoldingsPie.svelte";

const format = (qty: Dec): string => `$${(Number(qty.m) / 10 ** qty.p).toFixed(2)}`;

function holding(symbol: string, dollars: number): Holding {
    return {
        symbol,
        name: `${symbol} Inc.`,
        accounts: ["assets:broker"],
        shares: dec(1n, 0),
        basis: null,
        firstBasisDate: null,
        price: null,
        marketValue: dec(BigInt(dollars * 100), 2),
        gain: null,
        gainPct: null,
    };
}

/** A fresh array each call: the store refetches per report, keyed on the rows' identity. */
const portfolio = (): Holding[] => [holding("AAPL", 600), holding("VTI", 400)];

const EMPTY = {assetClass: [], sector: [], industry: [], securityType: [], category: [], risk: []};

const PROFILES = {
    yahoo: "ok",
    profiles: [
        {
            symbol: "AAPL",
            yahooTicker: "AAPL",
            source: "yahoo",
            fetchedAt: "2026-09-28",
            breakdown: {...EMPTY, assetClass: [{label: "Equity", weight: 1}], sector: [{label: "Technology", weight: 1}]},
        },
        {
            symbol: "VTI",
            yahooTicker: "VTI",
            source: "yahoo",
            fetchedAt: "2026-09-28",
            breakdown: {
                ...EMPTY,
                assetClass: [{label: "Equity", weight: 1}],
                sector: [
                    {label: "Technology", weight: 0.5},
                    {label: "Energy", weight: 0.25},
                ],
            },
        },
    ],
};

/** Yahoo down: AAPL knows nothing, VTI only what its tag says. */
const TAGS_ONLY = {
    yahoo: "unavailable",
    profiles: [
        {symbol: "AAPL", yahooTicker: "AAPL", source: "none", breakdown: EMPTY},
        {symbol: "VTI", yahooTicker: "VTI", source: "tags", breakdown: {...EMPTY, sector: [{label: "Diversified", weight: 1}]}},
    ],
};

let requests: string[] = [];

/** Answer `/api/holdings/profiles?…` with `answer` (a body, or a Response); record every request. */
async function engine(answer: unknown | (() => Promise<Response>)): Promise<void> {
    await connectFakeEngine();
    requests = [];
    vi.stubGlobal("fetch", (input: unknown) => {
        const url = String(input);
        requests.push(url.replace(FAKE_ENGINE, ""));
        if (url.endsWith("/version")) return Promise.resolve(new Response(JSON.stringify("1.52"), {status: 200}));
        if (!url.includes("/api/holdings/profiles")) return Promise.resolve(new Response("no route", {status: 404}));
        if (typeof answer === "function") return (answer as () => Promise<Response>)();
        if (answer instanceof Response) return Promise.resolve(answer);
        return Promise.resolve(new Response(JSON.stringify(answer), {status: 200, headers: {"Content-Type": "application/json"}}));
    });
}

const profileRequests = (): string[] => requests.filter((r) => r.startsWith("/api/holdings/profiles"));
const legend = (): string => screen.getByTestId("holdings-pie-legend").textContent ?? "";
const select = (): HTMLSelectElement => screen.getByTestId("holdings-pie-dimension") as HTMLSelectElement;
const options = (): string[] => [...select().options].map((o) => o.value);

beforeEach(() => {
    settings.holdingsPieDimension = "holding";
});

afterEach(() => vi.unstubAllGlobals());

describe("COMPONENT HoldingsPie", () => {
    it("says so, with a Retry, when the classifications cannot be loaded at all", async () => {
        await engine(new Response("boom", {status: 500}));
        settings.holdingsPieDimension = "sector";
        render(HoldingsPie, {holdings: portfolio(), format});
        await vi.waitFor(() => expect(screen.getByTestId("holdings-pie-error")).toBeTruthy());

        await engine(PROFILES);
        await fireEvent.click(screen.getByRole("button", {name: "Retry"}));
        await vi.waitFor(() => expect(legend()).toContain("Technology"));
        expect(profileRequests()).toHaveLength(1);
    });

    it("renders by holding from local data alone, asking the engine nothing", async () => {
        await engine(PROFILES);
        render(HoldingsPie, {holdings: portfolio(), format});
        expect(select().value).toBe("holding");
        expect(legend()).toContain("AAPL");
        expect(legend()).toContain("60.0%");
        expect(profileRequests()).toEqual([]);
    });

    it("fetches classifications once when a category view is chosen, and remembers the choice", async () => {
        let release: () => void = () => {};
        const gate = new Promise<void>((resolve) => (release = resolve));
        await engine(async () => {
            await gate;
            return new Response(JSON.stringify(PROFILES), {status: 200, headers: {"Content-Type": "application/json"}});
        });
        render(HoldingsPie, {holdings: portfolio(), format});

        await fireEvent.change(select(), {target: {value: "sector"}});
        expect(settings.holdingsPieDimension).toBe("sector");
        await vi.waitFor(() => expect(screen.getByTestId("holdings-pie-loading")).toBeTruthy());

        release();
        await vi.waitFor(() => expect(legend()).toContain("Technology"));
        // AAPL $600 + half of VTI's $400 = $800 of $1,000.
        expect(legend()).toContain("80.0%");
        expect(legend()).toContain("$800.00");
        expect(legend()).toContain("Energy");
        // A quarter of VTI has no sector: shown, and named for what it is.
        expect(legend()).toContain("(unclassified)");
        expect(legend()).toContain("$100.00");
        expect(profileRequests()).toHaveLength(1);
        expect(decodeURIComponent(profileRequests()[0])).toContain("symbols=AAPL,VTI");
        expect(screen.queryByTestId("holdings-pie-yahoo-hint")).toBeNull();
    });

    it("offers only the views some holding has data for, once it knows", async () => {
        await engine(PROFILES);
        settings.holdingsPieDimension = "assetClass";
        render(HoldingsPie, {holdings: portfolio(), format});
        await vi.waitFor(() => expect(legend()).toContain("Equity"));
        expect(options()).toEqual(["holding", "assetClass", "sector"]);
    });

    it("falls back to tag data with a quiet hint when Yahoo is unreachable", async () => {
        await engine(TAGS_ONLY);
        settings.holdingsPieDimension = "sector";
        render(HoldingsPie, {holdings: portfolio(), format});
        await vi.waitFor(() => expect(legend()).toContain("Diversified"));
        expect(screen.getByTestId("holdings-pie-yahoo-hint").textContent).toContain("Couldn't reach Yahoo — showing tag data only");
        expect(legend()).toContain("(unclassified)");
    });

    it("explains how to tag when no holding has data for the chosen view", async () => {
        await engine(TAGS_ONLY);
        settings.holdingsPieDimension = "industry";
        render(HoldingsPie, {holdings: portfolio(), format});
        await vi.waitFor(() => expect(screen.getByTestId("holdings-pie-unclassified")).toBeTruthy());
        expect(screen.getByTestId("holdings-pie-unclassified").textContent).toContain("industry: …");
        // The selected view stays listed even though it has no data.
        expect(select().value).toBe("industry");
    });

    it("goes back to by-holding without another request", async () => {
        await engine(PROFILES);
        settings.holdingsPieDimension = "sector";
        render(HoldingsPie, {holdings: portfolio(), format});
        await vi.waitFor(() => expect(legend()).toContain("Technology"));
        await fireEvent.change(select(), {target: {value: "holding"}});
        await vi.waitFor(() => expect(legend()).toContain("AAPL"));
        expect(profileRequests()).toHaveLength(1);
    });

    it("holds the previous answer and flags it when a refresh fails", async () => {
        await engine(new Response("boom", {status: 500}));
        settings.holdingsPieDimension = "sector";
        render(HoldingsPie, {holdings: portfolio(), format});
        await vi.waitFor(() => expect(screen.getByTestId("holdings-pie-stale")).toBeTruthy());
        expect(legend()).toContain("Technology");
    });
});
