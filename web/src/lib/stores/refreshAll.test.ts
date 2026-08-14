// What the header's Refresh button re-reads.
//
// The bug it exists for was invisible to every other kind of test. Each store's
// `reload…` worked. Each store's `ensure…` correctly returned early on an
// unchanged (nonce, url) key. The button called neither of them for four of the
// five resources on screen — it called `journal.refresh` and stopped — and no
// assertion anywhere was about which of them a press reaches.
//
// So the test that matters here is not "does `reloadIndex` fetch". It is: with
// every `ensure…` key already satisfied, exactly as it is after an ordinary
// startup, does a press still put every route on the wire? That question is only
// answerable by counting requests, so this drives the REAL stores through a
// recording `fetch` stub — the seam `importStore.svelte.ts` documents.

import {describe, expect, it, vi, beforeEach, afterEach} from "vitest";
import {readFileSync} from "node:fs";
import {aliasListing, aliasStore} from "$lib/imports/aliasStore.svelte";
import {importStore} from "$lib/imports/importStore.svelte";
import {openRules, rulesIndex, rulesStore} from "$lib/imports/rulesStore.svelte";
import {CAPABILITIES} from "$lib/testing/importFixtures";
import {FAKE_ENGINE} from "$lib/testing/fakeEngine";
import {settings} from "./settings.svelte";
import {currentRefreshState, holdUnsavedEdits, REFRESH_TARGETS, refreshEverything, refreshPlan} from "./refreshAll";

/** The same goldens `nativeDecode.test.ts` reads, so a renamed Rust field fails here too. */
const golden = (name: string): unknown => JSON.parse(readFileSync(new URL(`../../../../fixtures/rules/golden/${name}.json`, import.meta.url), "utf8"));

const RULES_INDEX = golden("rules-index");
const RULES_DOC = golden("rules-doc");
/** The id the index golden carries, which is the document the editor would have open. */
const OPEN_ID = "import/2026/bank.csv.rules";

const ALIAS_LISTING = {
    editable: true,
    files: [
        {
            journalId: "2026/2026.journal",
            label: "2026.journal",
            revision: "rev-1",
            writable: true,
            aliases: [
                {
                    journalId: "2026/2026.journal",
                    index: 0,
                    line: 3,
                    pattern: "CHASE CHECKING",
                    replacement: "assets:bank:checking",
                    regex: false,
                    forwarded: true,
                    editable: true,
                },
            ],
        },
    ],
};

/** Every URL this test's `fetch` has been asked for, in order. */
let seen: string[] = [];
/** Set by the test that takes one resource away mid-session. */
let aliasesUnreachable = false;

const json = (body: unknown): Response => new Response(JSON.stringify(body), {status: 200, headers: {"Content-Type": "application/json"}});

function respond(url: string): Response {
    if (url.endsWith("/version")) return json("1.52");
    if (url.endsWith("/api/import/capabilities")) return json(CAPABILITIES);
    if (url.endsWith("/api/aliases")) return aliasesUnreachable ? new Response("boom", {status: 500}) : json(ALIAS_LISTING);
    if (url.endsWith("/api/rules")) return json(RULES_INDEX);
    // The preview is decoration and `loadPreview` swallows its failure, so it is
    // deliberately left to 404 — a document that loaded must not be lost with it.
    if (url.includes("/api/rules/")) return json(RULES_DOC);
    if (url.includes("/transactions")) return json([]);
    if (url.includes("/accountnames")) return json([]);
    if (url.includes("/prices")) return json([]);
    if (url.includes("/accounts")) return json([]);
    return new Response(`no route for ${url}`, {status: 404});
}

/** Requests to `path` since the counter was last reset. */
const hits = (path: string): number => seen.filter((url) => url.includes(path)).length;

beforeEach(async () => {
    seen = [];
    aliasesUnreachable = false;
    vi.stubGlobal("fetch", (input: unknown) => {
        const url = String(input);
        seen.push(url);
        return Promise.resolve(respond(url));
    });
    if (settings.serverUrl === null) await settings.setServerUrl(FAKE_ENGINE);
});

afterEach(() => {
    vi.unstubAllGlobals();
    for (const name of REFRESH_TARGETS) holdUnsavedEdits(name, false);
});

describe("UNIT refreshAll — what the button reaches for", () => {
    it("is wired to the header's Refresh button", () => {
        // The defect was never inside a store. Every `reload…` below worked; the
        // BUTTON called `journal.refresh` and nothing else, so a module that
        // refreshes everything correctly and is not wired up fixes nothing.
        //
        // Mounting `+layout.svelte` would be the better test and is not
        // available: it reads `$app/state`'s page, which is populated by a real
        // navigation. So this reads the one line, on the same trade
        // `routes/effectLatch.test.ts` makes.
        const layout = readFileSync(new URL("../../routes/+layout.svelte", import.meta.url), "utf8");
        const button = /aria-label="Refresh[\s\S]*?<\/button>/.exec(layout)?.[0] ?? "";

        expect(button).toContain("refreshEverything()");
        expect(button).not.toContain("journal.refresh");
    });
});

describe("UNIT refreshAll — the plan", () => {
    it("re-reads every target by default", () => {
        expect(refreshPlan({openRulesId: "a.rules", unsaved: []})).toEqual([...REFRESH_TARGETS]);
    });

    it("names the journal, the rules index, the open document, the aliases and the capabilities", () => {
        // The list IS the definition of "everything". Pinning it is what makes
        // dropping one a failing test rather than a quiet regression — the exact
        // shape of the bug this module was written for.
        expect([...REFRESH_TARGETS].sort()).toEqual(["aliases", "importCapabilities", "journal", "openRules", "rulesIndex"]);
    });

    it("skips the open document when the editor has none open", () => {
        expect(refreshPlan({openRulesId: null, unsaved: []})).not.toContain("openRules");
        // Everything else still runs: no document open is not a reason to leave
        // the listing it came from stale.
        expect(refreshPlan({openRulesId: null, unsaved: []})).toContain("rulesIndex");
    });

    it("skips a resource an unsaved form is an edit of", () => {
        const plan = refreshPlan({openRulesId: "a.rules", unsaved: ["openRules", "aliases"]});

        expect(plan).not.toContain("openRules");
        expect(plan).not.toContain("aliases");
        // The refusal is per resource, not per press: the index and the journal
        // hold no user input, so an unsaved rules edit must not freeze them too.
        expect(plan).toContain("rulesIndex");
        expect(plan).toContain("journal");
    });
});

describe("UNIT refreshAll — a press, against an app that has already loaded", () => {
    /** Startup: the layout prefetches, the tabs probe, the panels open. Every dedupe key is now set. */
    async function startUp(): Promise<void> {
        await rulesStore.ensureIndex(FAKE_ENGINE, settings.serverNonce);
        await importStore.ensureCapabilities(FAKE_ENGINE, settings.serverNonce);
        await aliasStore.ensureListing(FAKE_ENGINE, settings.serverNonce);
        await rulesStore.open(FAKE_ENGINE, OPEN_ID);
        seen = [];
    }

    it("proves the dedupe keys really are satisfied, so the test below means something", async () => {
        await startUp();

        await rulesStore.ensureIndex(FAKE_ENGINE, settings.serverNonce);
        await importStore.ensureCapabilities(FAKE_ENGINE, settings.serverNonce);
        await aliasStore.ensureListing(FAKE_ENGINE, settings.serverNonce);

        expect(seen).toEqual([]);
    });

    it("re-reads the rules index, the open rules file, the aliases and the capabilities anyway", async () => {
        // The whole bug: "pressing the reload button does reload the aliases,
        // but doesn't seem to reload the rules file". Nothing was reloaded — the
        // aliases had merely been fetched for the first time later in the
        // session, after whatever the user changed on disk.
        await startUp();

        await refreshEverything();

        expect(hits("/api/rules")).toBeGreaterThanOrEqual(2); // the index AND the open document
        expect(hits(`/api/rules/${OPEN_ID.split("/").map(encodeURIComponent).join("/")}`)).toBe(1);
        expect(hits("/api/aliases")).toBe(1);
        expect(hits("/api/import/capabilities")).toBe(1);
        expect(hits("/transactions")).toBe(1);
    });

    it("leaves each store holding what it just read", async () => {
        // "A request went out" is not the claim the user is making; "the screen
        // shows what is on disk" is. These are the values the panels render.
        await startUp();

        await refreshEverything();

        expect(rulesIndex.value?.files[0]?.id).toBe(OPEN_ID);
        expect(openRules.value?.doc.id).toBe(OPEN_ID);
        expect(aliasListing.value?.files[0]?.aliases[0]?.pattern).toBe("CHASE CHECKING");
        expect(importStore.capabilities?.hledger.available).toBe(true);
    });

    it("does not re-read a document the editor has unsaved edits in", async () => {
        // Re-reading it would replace the user's typing with the file's bytes
        // and say nothing. The 409-on-save path already answers this out loud.
        await startUp();
        holdUnsavedEdits("openRules", true);

        await refreshEverything();

        expect(hits("/api/rules/")).toBe(0);
        // …and the rest of the screen is refreshed regardless.
        expect(hits("/api/aliases")).toBe(1);
    });

    it("reads the open document's id from the resource that holds it", async () => {
        await startUp();

        expect(currentRefreshState().openRulesId).toBe(OPEN_ID);
    });

    it("re-reads the rest when one resource has become unreachable", async () => {
        // Concurrent and `allSettled` for this: an engine that has lost one
        // route — an older build, a restart mid-press — must not turn the
        // refresh into a no-op for the other four. Each resource owns its own
        // error state and its own surface shows it.
        await startUp();
        aliasesUnreachable = true;

        await refreshEverything();

        expect(hits("/api/aliases")).toBe(1);
        expect(hits("/api/import/capabilities")).toBe(1);
        expect(hits("/transactions")).toBe(1);
        expect(rulesIndex.value?.files[0]?.id).toBe(OPEN_ID);
    });
});
