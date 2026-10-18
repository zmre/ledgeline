// The journal VIEW store: the filtered journal fetched a block at a time. What
// matters is which requests it makes and what it keeps — the table renders
// whatever `rowAt` answers, so a block fetched twice, kept too long, or dropped
// on an edit is visible to the user as wasted time, memory, or a flash of
// skeleton rows.

import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import type {Transaction} from "#lib/domain/types.js";
import {FAKE_ENGINE} from "#lib/testing/fakeEngine.js";
import {summaryBody, wireRow} from "#lib/testing/journalViewFixture.js";
import {BLOCK_SIZE, journalView, resetJournalView} from "./journalView.svelte";

const txn = (index: number, description = `txn ${index}`): Transaction => ({
    index,
    date: "2026-01-01",
    status: "unmarked",
    description,
    code: "",
    comment: "",
    tags: [],
    postings: [],
});

const ALL = {from: null, to: null, accounts: new Set<string>(), query: ""};

/** A fake engine over `count` rows, tagging each row's description with the generation it was served at. */
function serve(count: number): {calls: URL[]; generation: {value: string}} {
    const calls: URL[] = [];
    const generation = {value: "g1"};
    vi.stubGlobal("fetch", (input: RequestInfo | URL) => {
        const url = new URL(String(input));
        calls.push(url);
        if (url.pathname.endsWith("/api/journal/summary"))
            return Promise.resolve(new Response(JSON.stringify(summaryBody(count, {generation: generation.value}))));
        if (url.pathname.endsWith("/api/journal/rows")) {
            const offset = Number(url.searchParams.get("offset"));
            const limit = Number(url.searchParams.get("limit"));
            const asc = url.searchParams.get("order") === "asc";
            const rows = Array.from({length: Math.max(0, Math.min(limit, count - offset))}, (_, i) => {
                const position = offset + i;
                const index = asc ? position + 1 : count - position;
                return wireRow(txn(index, `${generation.value}:${index}`));
            });
            return Promise.resolve(new Response(JSON.stringify({generation: generation.value, total: count, offset, rows})));
        }
        return Promise.resolve(new Response("no route", {status: 404}));
    });
    return {calls, generation};
}

const rowsCalls = (calls: URL[]): number[] =>
    calls.filter((url) => url.pathname.endsWith("/rows")).map((url) => Number(url.searchParams.get("offset")) / BLOCK_SIZE);

beforeEach(() => resetJournalView());
afterEach(() => vi.unstubAllGlobals());

describe("UNIT journalView", () => {
    it("loads the summary and the first block on sync, and nothing more until asked", async () => {
        const {calls} = serve(1000);
        journalView.sync(FAKE_ENGINE, "g1", ALL);
        await vi.waitFor(() => expect(journalView.rowAt(0)).not.toBeNull());
        expect(journalView.total).toBe(1000);
        expect(journalView.summary?.total).toBe(1000);
        expect(journalView.rowAt(0)?.txn.index).toBe(1000);
        expect(journalView.rowAt(BLOCK_SIZE)).toBeNull();
        expect(rowsCalls(calls)).toEqual([0]);
        // Re-syncing to the same question is free.
        journalView.sync(FAKE_ENGINE, "g1", {...ALL, accounts: new Set<string>()});
        expect(calls).toHaveLength(2);
    });

    it("fetches the blocks a window touches plus one either side, once each", async () => {
        const {calls} = serve(2000);
        journalView.sync(FAKE_ENGINE, "g1", ALL);
        await vi.waitFor(() => expect(journalView.rowAt(0)).not.toBeNull());
        journalView.ensure(450, 480);
        journalView.ensure(450, 480);
        await vi.waitFor(() => expect(journalView.rowAt(3 * BLOCK_SIZE)).not.toBeNull());
        expect(rowsCalls(calls).sort()).toEqual([0, 1, 2, 3]);
        expect(journalView.rowAt(460)?.txn.index).toBe(2000 - 460);
    });

    it("never asks past the end of the view", async () => {
        const {calls} = serve(250);
        journalView.sync(FAKE_ENGINE, "g1", ALL);
        await vi.waitFor(() => expect(journalView.total).toBe(250));
        journalView.ensure(200, 250);
        await vi.waitFor(() => expect(journalView.rowAt(249)).not.toBeNull());
        expect(Math.max(...rowsCalls(calls))).toBe(1);
    });

    it("keeps the old rows on screen across a generation change until fresh ones arrive", async () => {
        const {generation} = serve(500);
        journalView.sync(FAKE_ENGINE, "g1", ALL);
        await vi.waitFor(() => expect(journalView.rowAt(0)).not.toBeNull());
        generation.value = "g2";
        journalView.sync(FAKE_ENGINE, "g2", ALL);
        // Still the g1 copy — no skeleton flash after an edit.
        expect(journalView.rowAt(0)?.txn.description).toBe("g1:500");
        journalView.ensure(0, 20);
        await vi.waitFor(() => expect(journalView.rowAt(0)?.txn.description).toBe("g2:500"));
    });

    it("drops everything on a filter change: a different list", async () => {
        serve(500);
        journalView.sync(FAKE_ENGINE, "g1", ALL);
        await vi.waitFor(() => expect(journalView.rowAt(0)).not.toBeNull());
        journalView.sync(FAKE_ENGINE, "g1", {...ALL, query: "rent"});
        expect(journalView.rowAt(0)).toBeNull();
        expect(journalView.total).toBeNull();
        expect(journalView.summary).toBeNull();
        await vi.waitFor(() => expect(journalView.rowAt(0)).not.toBeNull());
    });

    it("holds a bounded number of blocks, evicting the furthest from where the table looks", async () => {
        serve(200 * BLOCK_SIZE);
        journalView.sync(FAKE_ENGINE, "g1", ALL);
        await vi.waitFor(() => expect(journalView.rowAt(0)).not.toBeNull());
        for (let block = 1; block < 120; block += 3) {
            journalView.ensure(block * BLOCK_SIZE, block * BLOCK_SIZE + 10);
            await vi.waitFor(() => expect(journalView.rowAt(block * BLOCK_SIZE)).not.toBeNull());
        }
        // The start of the view is long gone; the end is held.
        expect(journalView.rowAt(0)).toBeNull();
        expect(journalView.rowAt(118 * BLOCK_SIZE)).not.toBeNull();
    });

    it("pages the whole view oldest-first for the export", async () => {
        const {calls} = serve(2500);
        journalView.sync(FAKE_ENGINE, "g1", ALL);
        await vi.waitFor(() => expect(journalView.total).toBe(2500));
        const rows = await journalView.allRows();
        expect(rows).toHaveLength(2500);
        expect(rows[0].index).toBe(1);
        expect(rows.at(-1)?.index).toBe(2500);
        const pages = calls.filter((url) => url.searchParams.get("order") === "asc");
        expect(pages.map((url) => url.searchParams.get("limit"))).toEqual(["1000", "1000", "1000"]);
    });

    it("locates a transaction through the engine", async () => {
        serve(10);
        journalView.sync(FAKE_ENGINE, "g1", ALL);
        vi.stubGlobal("fetch", (input: RequestInfo | URL) =>
            Promise.resolve(
                new Response(JSON.stringify(String(input).includes("/position") ? {generation: "g1", position: 7, date: "2026-01-01", total: 10} : {}))
            )
        );
        expect(await journalView.locate(3)).toEqual({generation: "g1", position: 7, date: "2026-01-01", total: 10});
    });
});
