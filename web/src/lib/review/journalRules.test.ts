// "Remember as a rule" on a journal card (plan 32 D27): the choice of rules
// file, the builder's source for each candidate, and the engine I/O behind it
// — the preview route, and the same rules-PUT append the import uses.

import {readFileSync} from "node:fs";
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import {decodeQueueItem} from "#lib/api/reviewDecode.js";
import type {RulesCandidate, RulesFor, RuleSpec} from "#lib/imports/reviewTypes.js";
import {settings} from "#lib/stores/settings.svelte.js";
import {queueItemWire, txnFixture} from "#lib/testing/reviewFixture.js";
import {engineRuleIo, fileReason, loadRuleChoice, ruleChoice, type JournalRuleIo} from "./journalRules";

const RULES_DOC = JSON.parse(readFileSync(new URL("../../../../fixtures/rules/golden/rules-doc.json", import.meta.url), "utf8")) as {revision: string};

function candidate(rulesId: string, recommended: boolean, extra: Partial<RulesCandidate> = {}): RulesCandidate {
    return {
        rulesId,
        label: rulesId.replace(/^.*\//, "").replace(/\.csv\.rules$/, ""),
        family: "citi",
        familySize: 2,
        year: 2026,
        account1: "liabilities:credit-card:citi",
        reasons: [],
        recommended,
        matchField: "description",
        fields: ["date", "description", "amount"],
        ...extra,
    };
}

const DECIDED: RulesFor = {
    index: 3,
    needsChoice: false,
    candidates: [candidate("import/citi-2026.csv.rules", true), candidate("import/citi-2025.csv.rules", false)],
};

const TIE: RulesFor = {
    index: 5,
    needsChoice: true,
    candidates: [candidate("import/amex.csv.rules", false, {family: "amex", familySize: 1}), candidate("import/amex-business.csv.rules", false)],
};

const RULE: RuleSpec = {matchers: [{field: "description", pattern: "blue.*bottle"}], assignments: [{field: "account2", value: "expenses:food:coffee"}]};

function fakeIo(): JournalRuleIo & {preview: ReturnType<typeof vi.fn>; save: ReturnType<typeof vi.fn>} {
    return {
        preview: vi.fn(async () => ({ok: true as const, basis: "journal" as const, matched: 1, total: 2})),
        save: vi.fn(async () => ({ok: true as const})),
    };
}

describe("UNIT journalRules — the choice", () => {
    it("offers nothing without candidates", () => {
        expect(ruleChoice({index: 1, needsChoice: false, candidates: []}, fakeIo())).toBeNull();
    });

    it("preselects the one recommendation", () => {
        const choice = ruleChoice(DECIDED, fakeIo());
        expect(choice?.recommended).toBe("import/citi-2026.csv.rules");
        expect(choice?.needsChoice).toBe(false);
        expect(choice?.candidates).toHaveLength(2);
    });

    it("preselects nothing when the user must choose", () => {
        const choice = ruleChoice(TIE, fakeIo());
        expect(choice?.recommended).toBeNull();
        expect(choice?.needsChoice).toBe(true);
    });

    it("refuses a source for a file that is not a candidate", () => {
        expect(() => ruleChoice(DECIDED, fakeIo())?.source("import/other.csv.rules")).toThrow();
    });
});

describe("UNIT journalRules — a candidate's source", () => {
    it("prefills from the description, names the file, and routes the check and the save to the picked file", async () => {
        const io = fakeIo();
        const source = ruleChoice(DECIDED, io)!.source("import/citi-2025.csv.rules");
        expect(source.rulesLabel).toBe("citi-2025.csv.rules");
        expect(source.fileLabel).toBe("citi-2025");
        expect(source.account).toBe("liabilities:credit-card:citi");
        expect(source.prefill.field).toBe("description");
        const item = decodeQueueItem(queueItemWire(txnFixture(3, "BLUE BOTTLE COFFEE 4412"), {key: "blue bottle coffee"}), "card");
        expect(source.prefill.pattern(item, "description")).toBe("blue.*bottle.*coffee");

        const signal = new AbortController().signal;
        await expect(source.preview(RULE, signal)).resolves.toEqual({ok: true, basis: "journal", matched: 1, total: 2});
        expect(io.preview).toHaveBeenCalledWith(3, "import/citi-2025.csv.rules", RULE, signal);
        await expect(source.save(RULE)).resolves.toEqual({ok: true});
        expect(io.save).toHaveBeenCalledWith("import/citi-2025.csv.rules", RULE);
        // Saving a rule changes nothing already in the journal.
        await expect(source.refresh()).resolves.toBeNull();
    });
});

describe("UNIT journalRules — same-named rules files", () => {
    it("labels each by its relative path, so the builder says which one it appends to", () => {
        const twins: RulesFor = {index: 7, needsChoice: true, candidates: [candidate("2025/citi.csv.rules", false), candidate("2026/citi.csv.rules", false)]};
        const choice = ruleChoice(twins, fakeIo())!;
        expect(choice.source("2025/citi.csv.rules").rulesLabel).toBe("2025/citi.csv.rules");
        expect(choice.source("2026/citi.csv.rules").rulesLabel).toBe("2026/citi.csv.rules");
    });
});

describe("UNIT journalRules — the reason beside the file", () => {
    const named = (year: number | null, reasons: string[] = ["Named for its year"]) =>
        candidate(`import/citi-${year ?? "x"}.csv.rules`, false, {year, reasons});

    it("calls the newest file of a family of several the newest", () => {
        const files = [named(2026), named(2025)];
        expect(fileReason(files, files[0]!)).toBe("newest of 2 for citi");
    });

    it("gives an older year's file its place in the family instead (the last reason; the first is the shared account)", () => {
        const files = [named(2026), named(2025, ["Imports into liabilities:credit-card:citi", "Named for 2025, this transaction's year"])];
        expect(fileReason(files, files[1]!)).toBe("Named for 2025, this transaction's year");
    });

    it("calls a family without years' pick the newest (it is chosen by .latest, then mtime)", () => {
        const files = [named(null), {...named(null), rulesId: "import/citi-old.csv.rules"}];
        expect(fileReason(files, files[0]!)).toBe("newest of 2 for citi");
    });

    it("gives a file alone in its family its first reason, or nothing", () => {
        const alone = candidate("import/chase.csv.rules", true, {family: "chase", familySize: 1, year: null, reasons: ["Books to assets:bank:chase"]});
        expect(fileReason([alone], alone)).toBe("Books to assets:bank:chase");
        expect(fileReason([alone], {...alone, reasons: []})).toBe("");
    });
});

interface Seen {
    method: string;
    url: string;
    body: Record<string, unknown> | undefined;
}

function engine(): Seen[] {
    const seen: Seen[] = [];
    const json = (value: unknown): Response => new Response(JSON.stringify(value), {status: 200});
    vi.stubGlobal("fetch", (url: string, init?: RequestInit) => {
        const method = init?.method ?? "GET";
        const body = typeof init?.body === "string" ? (JSON.parse(init.body) as Record<string, unknown>) : undefined;
        seen.push({method, url, body});
        if (url.endsWith("/version")) return Promise.resolve(json("1.52"));
        if (url.endsWith("/api/review/rules-for/3"))
            return Promise.resolve(json({index: 3, needsChoice: false, candidates: [{...candidate("import/citi-2026.csv.rules", true)}]}));
        if (url.endsWith("/api/review/rules-for/9")) return Promise.resolve(json({index: 9, needsChoice: false, candidates: []}));
        if (url.endsWith("/api/review/rule-preview"))
            return Promise.resolve(
                json({basis: "csv", file: "citi-2026.csv", ok: true, matched: 0, total: 3, alreadyCategorised: 0, changed: [], changedTotal: 0})
            );
        if (url.includes("/api/rules/") && method === "GET") return Promise.resolve(json(RULES_DOC));
        if (url.includes("/api/rules/") && method === "PUT") return Promise.resolve(json(RULES_DOC));
        return Promise.resolve(new Response("no route", {status: 404}));
    });
    return seen;
}

describe("UNIT journalRules — against the engine", () => {
    beforeEach(async () => {
        engine();
        if (settings.serverUrl === null) await settings.setServerUrl("http://engine.test");
    });
    afterEach(() => vi.unstubAllGlobals());

    it("loads the choice for a transaction, or null when nothing fits", async () => {
        const choice = await loadRuleChoice(3);
        expect(choice?.recommended).toBe("import/citi-2026.csv.rules");
        await expect(loadRuleChoice(9)).resolves.toBeNull();
    });

    it("previews through the review route, and flags a pattern that met no CSV row", async () => {
        const seen = engine();
        const preview = await engineRuleIo("http://engine.test").preview(3, "import/citi-2026.csv.rules", RULE);
        expect(preview).toMatchObject({ok: true, basis: "csv", file: "citi-2026.csv", noCsvMatch: true});
        const call = seen.find((entry) => entry.url.endsWith("/api/review/rule-preview"));
        expect(call?.method).toBe("POST");
        expect(call?.body).toEqual({index: 3, rulesId: "import/citi-2026.csv.rules", rule: RULE});
    });

    it("saves with the import's own append: read, then PUT pinned to that revision", async () => {
        const seen = engine();
        await expect(engineRuleIo("http://engine.test").save("import/citi-2026.csv.rules", RULE)).resolves.toEqual({ok: true});
        const calls = seen.filter((entry) => entry.url.includes("/api/rules/"));
        expect(calls.map((entry) => entry.method)).toEqual(["GET", "PUT"]);
        expect(calls[1]?.url).toContain(encodeURIComponent("import/citi-2026.csv.rules").replaceAll("%2F", "/"));
        expect(calls[1]?.body?.revision).toBe(RULES_DOC.revision);
    });
});
