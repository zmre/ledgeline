// The New Transactions store carrying review edits (plan 32 D23–D25): the
// overrides ride on every dry run and the commit, a stale one is replayed, and
// the edits belong to one staging and one commit. A separate file from
// `importStore.test.ts` so this module instance starts with no edits held.

import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import {settings} from "#lib/stores/settings.svelte.js";
import {CAPABILITIES, STAGE, upload} from "#lib/testing/importFixtures.js";
import {stagedTxnWire, txnFixture} from "#lib/testing/reviewFixture.js";
import type {Transaction} from "#lib/domain/types.js";
import {importStore} from "./importStore.svelte";

const SUPPLIES = {op: "setPostingAccount", posting: 0, account: "expenses:home:supplies"} as const;
const REVIEWED = {op: "setTxnTag", name: "reviewed", value: "2026-10-30"} as const;

function dryRunWire(txns: Record<string, unknown>[], overrides = 0): Record<string, unknown> {
    return {
        ok: true,
        entries: "…",
        count: txns.length,
        status: "would import",
        skipped: null,
        balance: null,
        cliCommand: "ledgeline import",
        idMatches: null,
        txns,
        records: {aligned: false, header: null, fields: null, rows: []},
        overrides,
        cliNote: overrides === 0 ? null : `plus ${overrides} reviewed edits`,
    };
}

const hardware = (edit: (txn: Transaction) => void = () => {}): Transaction => {
    const txn = txnFixture(1, "HARDWARE STORE");
    edit(txn);
    return txn;
};

interface Seen {
    url: string;
    body: Record<string, unknown> | undefined;
}

/** A fetch that answers `/dry-run` with `dryRun(body)`, plus the fixed routes. */
function engine(dryRun: (body: Record<string, unknown>) => Response | Record<string, unknown>, extra: Record<string, unknown> = {}): Seen[] {
    const seen: Seen[] = [];
    const json = (value: unknown): Response => (value instanceof Response ? value : new Response(JSON.stringify(value), {status: 200}));
    vi.stubGlobal("fetch", (url: string, init?: RequestInit) => {
        const body = typeof init?.body === "string" ? (JSON.parse(init.body) as Record<string, unknown>) : undefined;
        seen.push({url, body});
        // As the real fetch does: an aborted request rejects.
        if (init?.signal?.aborted === true) return Promise.reject(new DOMException("aborted", "AbortError"));
        if (url.endsWith("/version")) return Promise.resolve(json("1.52"));
        if (url.endsWith("/api/import/capabilities")) return Promise.resolve(json(CAPABILITIES));
        if (url.endsWith("/api/import/stage")) return Promise.resolve(json(STAGE));
        if (url.endsWith("/api/import/dry-run")) return Promise.resolve(json(dryRun(body ?? {})));
        const key = Object.keys(extra).find((route) => url.endsWith(route));
        if (key !== undefined) return Promise.resolve(json(extra[key]));
        return Promise.resolve(new Response("no route", {status: 404}));
    });
    return seen;
}

const bodies = (seen: Seen[], suffix: string): (Record<string, unknown> | undefined)[] =>
    seen.filter((call) => call.url.endsWith(suffix)).map((call) => call.body);

beforeEach(async () => {
    engine(() => dryRunWire([]));
    if (settings.serverUrl === null) await settings.setServerUrl("http://engine.test");
    await importStore.reloadCapabilities("http://engine.test");
});

afterEach(() => vi.unstubAllGlobals());

/** Stage, dry run, and review the first card with `ops`. */
async function reviewed(ops: readonly (typeof SUPPLIES | typeof REVIEWED)[]): Promise<void> {
    await importStore.offerFile(upload("bank.csv"));
    await importStore.runDryRun();
    const [card] = importStore.stagedTxns;
    if (card === undefined) throw new Error("no staged card");
    expect(importStore.reviewSink.stage({overrides: [{item: card, ops}], journalEdits: [{index: 40, fingerprint: "j40", ops: [SUPPLIES]}]})).toEqual({
        ok: true,
    });
}

describe("UNIT importStore — review edits ride on the run", () => {
    it("sends the overrides on the next dry run and both kinds on the commit, then forgets them", async () => {
        const seen = engine((body) => dryRunWire([stagedTxnWire(hardware(), {fingerprint: "gen-1"})], body.overrides === undefined ? 0 : 1), {
            "/api/import/commit": {
                csvWritten: "import/2026/bank.csv",
                journalWritten: "2026/2026.journal",
                imported: 1,
                ordering: {inOrder: true, moves: []},
                overrides: 1,
                journalEdited: 1,
            },
        });
        await reviewed([SUPPLIES, REVIEWED]);
        expect(importStore.overrideCount).toBe(1);
        expect(importStore.journalEditCount).toBe(1);

        await importStore.runDryRun();
        const runs = bodies(seen, "/dry-run");
        // The first run carried nothing: there was nothing to carry yet.
        expect(runs[0]?.overrides).toBeUndefined();
        expect(runs[1]?.overrides).toEqual([{ordinal: 0, fingerprint: "gen-1", ops: [SUPPLIES, REVIEWED]}]);
        expect(importStore.dryRun?.ok && importStore.dryRun.cliNote).toBe("plus 1 reviewed edits");

        await importStore.writeChanges();
        const [commit] = bodies(seen, "/commit");
        expect(commit?.overrides).toEqual([{ordinal: 0, fingerprint: "gen-1", ops: [SUPPLIES, REVIEWED]}]);
        expect(commit?.journalEdits).toEqual([{index: 40, fingerprint: "j40", ops: [SUPPLIES]}]);
        expect(importStore.committed?.journalEdited).toBe(1);
        // Landed: they belong to no import now.
        expect(importStore.overrideCount).toBe(0);
        expect(importStore.journalEditCount).toBe(0);
    });

    it("forgets the edits when a new file is staged", async () => {
        engine(() => dryRunWire([stagedTxnWire(hardware(), {fingerprint: "gen-1"})]));
        await reviewed([REVIEWED]);
        await importStore.offerFile(upload("other.csv"));
        expect(importStore.overrideCount).toBe(0);
        expect(importStore.journalEditCount).toBe(0);
    });
});

describe("UNIT importStore — a stale override is replayed (D25)", () => {
    it("never shows the expected 409 while it replays: the panel goes from loading straight to the replayed run", async () => {
        let ruleSaved = false;
        // What the panel could see at the moment each request went out. The
        // run after the 409 is where a published conflict would show.
        const visible: (string | null)[] = [];
        engine((body) => {
            visible.push(importStore.dryRunError === null ? null : String(importStore.dryRunError));
            if (!ruleSaved) return dryRunWire([stagedTxnWire(hardware(), {fingerprint: "gen-1"})]);
            const sent = body.overrides as {fingerprint: string}[] | undefined;
            if (sent?.some((override) => override.fingerprint === "gen-1")) return new Response("staged transaction 0 changed", {status: 409});
            return dryRunWire([stagedTxnWire(hardware(), {fingerprint: "gen-1-ruled"})], sent?.length ?? 0);
        });
        await reviewed([SUPPLIES, REVIEWED]);
        ruleSaved = true;
        visible.length = 0;
        await importStore.refreshDryRun();
        expect(visible).toEqual([null, null, null]);
        expect(importStore.dryRunError).toBeNull();
    });

    it("on a 409 regenerates without overrides, drops the one a rule now covers, and keeps the rest", async () => {
        let ruleSaved = false;
        const seen = engine((body) => {
            if (!ruleSaved) return dryRunWire([stagedTxnWire(hardware(), {fingerprint: "gen-1"})]);
            // After the rule: the account is the rule's, so the generated text changed.
            const ruled = stagedTxnWire(
                hardware((txn) => {
                    txn.postings[0]!.account = "expenses:home:supplies";
                }),
                {fingerprint: "gen-1-ruled"}
            );
            const sent = body.overrides as {fingerprint: string}[] | undefined;
            if (sent?.some((override) => override.fingerprint === "gen-1")) return new Response("staged transaction 0 changed", {status: 409});
            return dryRunWire([ruled], sent?.length ?? 0);
        });
        await reviewed([SUPPLIES, REVIEWED]);
        ruleSaved = true;
        await importStore.refreshDryRun();

        const runs = bodies(seen, "/dry-run").slice(1);
        expect(runs.map((body) => (body?.overrides as unknown[] | undefined)?.length ?? 0)).toEqual([1, 0, 1]);
        // The account op is the rule's now; the reviewed stamp carries on, unflagged.
        expect(importStore.overrides.map((override) => [override.fingerprint, override.ops])).toEqual([["gen-1-ruled", [REVIEWED]]]);
        expect(importStore.flaggedOverrides).toEqual([]);
        expect(importStore.lastReplay?.kept).toHaveLength(1);
        expect(importStore.dryRun?.ok).toBe(true);
        expect(importStore.dryRunError).toBeNull();
    });

    it("drops an override entirely when the rule covers all of it", async () => {
        let ruleSaved = false;
        engine((body) => {
            if (!ruleSaved) return dryRunWire([stagedTxnWire(hardware(), {fingerprint: "gen-1"})]);
            if (body.overrides !== undefined) return new Response("stale", {status: 409});
            return dryRunWire([
                stagedTxnWire(
                    hardware((txn) => {
                        txn.postings[0]!.account = "expenses:home:supplies";
                    }),
                    {fingerprint: "gen-1-ruled"}
                ),
            ]);
        });
        await reviewed([SUPPLIES]);
        ruleSaved = true;
        await importStore.runDryRun();
        expect(importStore.overrideCount).toBe(0);
        expect(importStore.lastReplay?.dropped.map((drop) => drop.reason)).toEqual(["covered"]);
    });
});

describe("UNIT importStore — the rule builder's live check", () => {
    it("asks with the dry run's four handles and the rule, nothing else", async () => {
        const seen = engine(() => dryRunWire([]), {
            "/api/import/rule-preview": {ok: true, matched: 1, total: 5, alreadyCategorised: 0, changed: [], changedTotal: 1},
        });
        await importStore.offerFile(upload("bank.csv"));
        const preview = await importStore.rulePreview({
            matchers: [{field: "description", pattern: "hardware.*store"}],
            assignments: [{field: "account2", value: "expenses:home:supplies"}],
        });
        expect(preview).toMatchObject({ok: true, matched: 1, total: 5});
        expect(bodies(seen, "/rule-preview")[0]).toEqual({
            stageId: "stage-1",
            rulesId: "import/2026/bank.csv.rules",
            csvPath: "import/2026/bank.csv",
            journalId: "2026/2026.journal",
            rule: {matchers: [{field: "description", pattern: "hardware.*store"}], assignments: [{field: "account2", value: "expenses:home:supplies"}]},
        });
    });

    it("rejects with the abort when a newer check cancels it", async () => {
        engine(() => dryRunWire([]), {"/api/import/rule-preview": {ok: true, matched: 0, total: 0, alreadyCategorised: 0, changed: [], changedTotal: 0}});
        await importStore.offerFile(upload("bank.csv"));
        const controller = new AbortController();
        controller.abort();
        await expect(importStore.rulePreview({matchers: [{field: null, pattern: "x"}], assignments: []}, controller.signal)).rejects.toThrow();
    });
});
