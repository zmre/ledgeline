// Find Matches on an import's result screen (plan 35 D9 "Import"): offered
// only when the import's new transactions have a proposed partner, opening
// the same overlay scoped by `since`, which closing drops again. A QuickBooks
// import (no scope) offers the whole journal's pairs.

import {fireEvent, render, screen} from "@testing-library/svelte";
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";

import {keymap} from "#lib/keys/keymap.svelte.js";
import {resetReviewLaunch} from "#lib/review/ui/launch.svelte.js";
import {journal} from "#lib/stores/journal.svelte.js";
import {matches, resetMatches} from "#lib/stores/matches.svelte.js";
import {connectFakeEngine, FAKE_ENGINE, journalInfoBody} from "#lib/testing/fakeEngine.js";
import {matchesPageWire, proposalWire, transferHalf} from "#lib/testing/matchFixture.js";
import ImportMatches from "./ImportMatches.svelte";
import MatchOverlay from "./MatchOverlay.svelte";

const OLD_OUT = transferHalf(1, "2026-11-05", "TRANSFER TO SAVINGS", "assets:checking", -50000);
const OLD_IN = transferHalf(2, "2026-11-06", "ONLINE TRANSFER FROM CHK", "assets:savings", 50000);
const OUT = transferHalf(3, "2026-12-31", "ACH AMEX EPAYMENT", "assets:checking", -50000);
const IN = transferHalf(4, "2027-01-02", "PAYMENT THANK YOU", "liabilities:amex", 50000);

const OLD = proposalWire(OLD_OUT, OLD_IN);
const NEW = proposalWire(OUT, IN);

/** Every `GET /api/matches` query string's `since` (null when absent). */
let sinces: (string | null)[] = [];

/** The whole journal proposes `all`; a scoped read proposes `scoped`. */
async function serve(all: Record<string, unknown>[], scoped: Record<string, unknown>[]): Promise<void> {
    await connectFakeEngine();
    sinces = [];
    vi.stubGlobal("fetch", (input: RequestInfo | URL) => {
        const url = new URL(String(input));
        const json = (value: unknown) => Promise.resolve(new Response(JSON.stringify(value), {status: 200}));
        const path = url.pathname;
        if (path.endsWith("/api/journal")) return json(journalInfoBody({generation: "g1"}));
        if (path.endsWith("/api/diagnostics")) return json({count: 0, maxSeverity: null, diagnostics: []});
        if (path.endsWith("/api/matches")) {
            const since = url.searchParams.get("since");
            sinces.push(since);
            return json(matchesPageWire(since === null ? all : scoped, {generation: "g1"}));
        }
        return Promise.resolve(new Response(`no route for ${url}`, {status: 404}));
    });
    await journal.refresh({force: true});
}

beforeEach(() => {
    resetMatches();
    resetReviewLaunch();
});
afterEach(() => {
    vi.unstubAllGlobals();
    keymap.reset();
    resetMatches();
});

describe("COMPONENT ImportMatches", () => {
    it("offers the pairs with a half in the import, and opens the overlay scoped to them", async () => {
        await serve([OLD, NEW], [NEW]);
        render(ImportMatches, {since: "amex.journal:2", imported: 2});
        await vi.waitFor(() => expect(screen.getByTestId("imports-find-matches")).toBeDefined());
        expect(screen.getByTestId("imports-find-matches-text").textContent).toContain("1 transfer pair has a half in this import.");
        expect(sinces).toEqual(["amex.journal:2"]);
        // Reading the offer leaves the journal badge's list alone.
        expect(matches.page).toBeNull();

        await fireEvent.click(screen.getByTestId("imports-find-matches-button"));
        expect(matches.open).toBe(true);
        expect(matches.scope).toBe("amex.journal:2");
        await vi.waitFor(() => expect(matches.page?.proposals).toHaveLength(1));
        expect(sinces.at(-1)).toBe("amex.journal:2");
        expect(matches.count).toBe(1);

        // Closing drops the scope and reads the whole journal's list again.
        matches.close();
        expect(matches.scope).toBeNull();
        await vi.waitFor(() => expect(matches.count).toBe(2));
        expect(sinces.at(-1)).toBeNull();
    });

    it("the overlay says it is showing this import", async () => {
        await serve([OLD, NEW], [NEW]);
        matches.openRun("amex.journal:2");
        render(MatchOverlay);
        await vi.waitFor(() => expect(screen.getByTestId("match-counts").textContent?.replace(/\s+/g, " ")).toContain("1 clear, 0 to check"));
        expect(screen.getByTestId("match-scope").textContent).toBe("this import");
    });

    it("offers nothing when no new half has a partner", async () => {
        await serve([OLD], []);
        render(ImportMatches, {since: "amex.journal:2", imported: 2});
        await vi.waitFor(() => expect(sinces).toEqual(["amex.journal:2"]));
        await Promise.resolve();
        expect(screen.queryByTestId("imports-find-matches")).toBeNull();
    });

    it("offers nothing, and asks nothing, when the import wrote no transactions", async () => {
        await serve([OLD, NEW], [NEW]);
        render(ImportMatches, {since: null, imported: 0});
        await Promise.resolve();
        expect(screen.queryByTestId("imports-find-matches")).toBeNull();
        expect(sinces).toEqual([]);
    });

    it("with no scope (a QuickBooks import), offers the whole journal's pairs", async () => {
        await serve([OLD, NEW], [NEW]);
        render(ImportMatches, {since: null, imported: 5});
        await vi.waitFor(() => expect(screen.getByTestId("imports-find-matches-text").textContent).toContain("2 transfer pairs are waiting"));
        expect(sinces).toEqual([null]);
        await fireEvent.click(screen.getByTestId("imports-find-matches-button"));
        expect(matches.open).toBe(true);
        expect(matches.scope).toBeNull();
        expect(matches.count).toBe(2);
        expect(sinces).toEqual([null]);
    });
});

describe("UNIT the matches store's scope", () => {
    it("is part of the read: a scoped run reads with since, an unscoped one without", async () => {
        await serve([OLD, NEW], [NEW]);
        await matches.sync(FAKE_ENGINE, "g1");
        expect(matches.count).toBe(2);
        matches.openRun("amex.journal:2");
        expect(matches.page).toBeNull();
        await vi.waitFor(() => expect(matches.count).toBe(1));
        // The journal toolbar's sync during the run reads the scoped list, not the whole one.
        await matches.sync(FAKE_ENGINE, "g1");
        expect(matches.count).toBe(1);
        expect(sinces).toEqual([null, "amex.journal:2"]);
    });
});
