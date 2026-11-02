// "Remember as a rule" (plan 32 D25), the store's half: the rule builder's view
// of the import, the rule appended through the rules editor's own PUT (pinned
// to the revision just read, at the end), and letting a flagged override go.
// Its own file so this module instance starts with nothing staged.

import {readFileSync} from "node:fs";
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import {settings} from "#lib/stores/settings.svelte.js";
import {CAPABILITIES, STAGE, upload} from "#lib/testing/importFixtures.js";
import {stagedTxnWire, txnFixture} from "#lib/testing/reviewFixture.js";
import {importStore} from "./importStore.svelte";

const RULES_DOC = JSON.parse(readFileSync(new URL("../../../../fixtures/rules/golden/rules-doc.json", import.meta.url), "utf8")) as {
    revision: string;
    items: {id: number}[];
};
const RECORDS = {
    aligned: true,
    header: ["Date", "Description", "Amount"],
    fields: ["date", "description", "amount"],
    rows: [["01/19/2024", "HARDWARE STORE", "-62.13"]],
};
const SUPPLIES = {op: "setPostingAccount", posting: 0, account: "expenses:home:supplies"} as const;

function dryRunWire(overrides: number): Record<string, unknown> {
    return {
        ok: true,
        entries: "…",
        count: 1,
        status: "would import",
        skipped: null,
        balance: null,
        cliCommand: "ledgeline import",
        idMatches: null,
        txns: [stagedTxnWire(txnFixture(1, "HARDWARE STORE"), {fingerprint: "gen-1"})],
        records: RECORDS,
        overrides,
        cliNote: null,
    };
}

interface Seen {
    method: string;
    url: string;
    body: Record<string, unknown> | undefined;
}

function engine(rulesPut: () => Response = () => new Response(JSON.stringify(RULES_DOC), {status: 200})): Seen[] {
    const seen: Seen[] = [];
    const json = (value: unknown): Response => new Response(JSON.stringify(value), {status: 200});
    vi.stubGlobal("fetch", (url: string, init?: RequestInit) => {
        const method = init?.method ?? "GET";
        const body = typeof init?.body === "string" ? (JSON.parse(init.body) as Record<string, unknown>) : undefined;
        seen.push({method, url, body});
        if (url.endsWith("/version")) return Promise.resolve(json("1.52"));
        if (url.endsWith("/api/import/capabilities")) return Promise.resolve(json(CAPABILITIES));
        if (url.endsWith("/api/import/stage")) return Promise.resolve(json(STAGE));
        if (url.endsWith("/api/import/dry-run")) return Promise.resolve(json(dryRunWire(body?.overrides === undefined ? 0 : 1)));
        if (url.includes("/api/rules/") && method === "GET") return Promise.resolve(json(RULES_DOC));
        if (url.includes("/api/rules/") && method === "PUT") return Promise.resolve(rulesPut());
        return Promise.resolve(new Response("no route", {status: 404}));
    });
    return seen;
}

beforeEach(async () => {
    engine();
    if (settings.serverUrl === null) await settings.setServerUrl("http://engine.test");
    await importStore.reloadCapabilities("http://engine.test");
});
afterEach(() => vi.unstubAllGlobals());

async function dryRunReady(): Promise<void> {
    await importStore.offerFile(upload("bank.csv"));
    await importStore.runDryRun();
}

describe("UNIT importStore — the rule builder's source (D25)", () => {
    it("prefills from the dry run's records, and names its file and its rules file", async () => {
        engine();
        await importStore.offerFile(upload("bank.csv"));
        expect(importStore.ruleSource()).toBeNull();
        await importStore.runDryRun();
        const source = importStore.ruleSource();
        expect(source?.prefill.fields).toEqual(["date", "description", "amount"]);
        expect(source?.prefill.field).toBe("description");
        expect(source?.fileLabel).toBe("bank.csv");
        expect(source?.rulesLabel).toBe("bank.csv.rules");
    });

    it("appends the rule at the end of the rules file, pinned to the revision it read", async () => {
        const seen = engine();
        await dryRunReady();
        const result = await importStore.saveReviewRule({
            matchers: [{field: "description", pattern: "hardware.*store"}],
            assignments: [{field: "account2", value: "expenses:home:supplies"}],
        });
        expect(result).toEqual({ok: true});
        const put = seen.find((call) => call.method === "PUT");
        expect(put?.url).toContain("/api/rules/");
        expect(put?.body?.revision).toBe(RULES_DOC.revision);
        expect(put?.body?.delete).toEqual([]);
        const items = put?.body?.items as Record<string, unknown>[];
        // Every existing item kept, in order, and the new rule last.
        expect(items.slice(0, -1)).toEqual(RULES_DOC.items.map((item) => ({kind: "keep", id: item.id})));
        expect(items.at(-1)).toEqual({
            kind: "ifBlock",
            groups: [{matchers: [{field: "description", pattern: "hardware.*store"}]}],
            assignments: [{field: "account2", value: "expenses:home:supplies"}],
        });
    });

    it("writes a whole-record matcher with no field", async () => {
        const seen = engine();
        await dryRunReady();
        await importStore.saveReviewRule({matchers: [{field: null, pattern: "hardware"}], assignments: [{field: "account2", value: "expenses:home"}]});
        const items = seen.find((call) => call.method === "PUT")?.body?.items as Record<string, unknown>[];
        expect(items.at(-1)?.groups).toEqual([{matchers: [{pattern: "hardware"}]}]);
    });

    it("hands back the refusal when the rules file changed underneath", async () => {
        engine(() => new Response("the rules file changed on disk", {status: 409}));
        await dryRunReady();
        const result = await importStore.saveReviewRule({matchers: [{field: null, pattern: "x"}], assignments: [{field: "account2", value: "a"}]});
        expect(result.ok).toBe(false);
        if (!result.ok) expect(result.failure.kind).toBe("conflict");
    });
});

describe("UNIT importStore — discarding a held override", () => {
    it("lets it go and runs the dry run without it", async () => {
        const seen = engine();
        await dryRunReady();
        const [card] = importStore.stagedTxns;
        if (card === undefined) throw new Error("no staged card");
        importStore.reviewSink.stage({overrides: [{item: card, ops: [SUPPLIES]}], journalEdits: []});
        expect(importStore.overrideCount).toBe(1);
        await importStore.discardOverrides([card.ref]);
        expect(importStore.overrideCount).toBe(0);
        const runs = seen.filter((call) => call.url.endsWith("/api/import/dry-run"));
        expect(runs.at(-1)?.body?.overrides).toBeUndefined();
    });
});
