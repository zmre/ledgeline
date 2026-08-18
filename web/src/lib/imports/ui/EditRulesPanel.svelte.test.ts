// Mounting the Edit Rules tab to pin where its warning banner and its CSV
// preview read from.
//
// # The bug
//
// The banner rendered `openRules.value?.doc.warnings` — the GET resource. A save
// deliberately does NOT refetch that resource (a rules file invalidates no
// transaction, so the store reloads only the index) and re-seeds `baseDoc` from
// the document the engine answered the PUT with. So the two diverged the moment
// a save landed: `baseDoc` held the fresh parse, `openRules` still held the parse
// of the bytes as they were when the file was opened, and the banner was reading
// the stale one. Fix the file, save, and the complaint stayed on screen until
// something unrelated re-opened the document.
//
// # Why this is not about any one warning
//
// The warning below is a `fields` diagnostic that long pre-dates the account
// comment one that surfaced this. EVERY entry in `doc.warnings()` came through
// the same stale reference, so the case is deliberately written with a
// pre-existing warning: it reproduces on a tree where the newer one does not
// exist at all.
//
// # The preview is the same staleness with a DIFFERENT cure
//
// `preview` read the same never-refetched resource, so it went stale by exactly
// the same mechanism — but `baseDoc` cannot rescue it. The PUT answers a
// `wire_doc`, which describes the rules document and says nothing whatsoever
// about the DATA file it names, so the fresh preview does not exist anywhere on
// the client until somebody asks for it. It has to be refetched, and the tests
// below cover the three states that come with a second request: it landed, it is
// still in flight, and it failed.
//
// Note which settings make it stale. `source` is the obvious one and is the
// wrong answer — this GUI shows `source` and never writes it, on purpose. It is
// `skip` and `separator`, both editable on the Preferences tab and both fed
// straight into the engine's preview: `skip` picks which record is the header
// row, `separator` picks where the columns are.
//
// # Why the document is the golden fixture
//
// Same reasoning as `model.test.ts`: a hand-written literal would only prove the
// panel handles the shapes its author remembered, and this one has to survive
// `toForm` → `validateForm` → `toSaveRequest` to reach a save at all. An invented
// document fails client validation and never sends the PUT, which makes the test
// pass for the wrong reason — it did exactly that while being written.
//
// # Why the store is real
//
// Same reasoning as `AliasPanel.svelte.test.ts`: the bytes arrive through a
// stubbed `fetch` rather than a mocked `rulesStore`, because a mocked store would
// prove the panel renders what it is handed — and it renders what it is handed
// perfectly well. What broke was WHICH value it reached for.

import {rulesStore} from "$lib/imports/rulesStore.svelte";
import {settings} from "$lib/stores/settings.svelte";
import {FAKE_ENGINE} from "$lib/testing/fakeEngine";
import {fireEvent, render, screen} from "@testing-library/svelte";
import {readFileSync} from "node:fs";
import {resolve} from "node:path";
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import EditRulesPanel from "./EditRulesPanel.svelte";

// Resolved from the Vite root (`web/`) rather than `import.meta.url`, which the
// `components` project serves over http: under jsdom — `model.test.ts` can use a
// file: URL only because it runs in the `unit` (node) project.
const GOLDEN = JSON.parse(readFileSync(resolve(process.cwd(), "../fixtures/rules/golden/rules-doc.json"), "utf8"));
const ID: string = GOLDEN.id;

/** A pre-existing diagnostic: `classify_fields` has emitted this all along. */
const STALE_WARNING = "hledger rejects a `fields` list followed by whitespace; only text touching the last name is discarded";

/** The document as first opened: one warning, on the `fields` item. */
const BEFORE = {...GOLDEN, warnings: [{itemId: 2, line: 3, message: STALE_WARNING}]};
/** What the engine answers the save with, having re-parsed the bytes it wrote. */
const AFTER = {...GOLDEN, revision: "rev-after", warnings: []};

/**
 * The preview as the file was OPENED — the engine's reading of the data file
 * under the settings on disk at the time.
 *
 * Three columns, matching the golden document's three `fields` names, so the
 * mapping table's row count is the same before and after and the assertions are
 * about the CONTENT rather than about rows appearing and disappearing.
 */
const PREVIEW_OPENED = {
    available: true,
    dataLabel: "bank.csv",
    separator: ",",
    header: ["Txn Date", "Narrative", "Debit"],
    rows: [["2026-01-02", "OLD-PREVIEW-ROW", "-4.50"]],
    columns: 3,
    truncated: false,
};
/** The same file re-read after the save moved `skip`, so a different record is the header. */
const PREVIEW_RESAVED = {
    available: true,
    dataLabel: "bank.csv",
    separator: ",",
    header: ["Posted On", "Memo Line", "Value"],
    rows: [["02/01/2026", "FRESH-PREVIEW-ROW", "-4,50"]],
    columns: 3,
    truncated: false,
};
/** An older in-flight answer, distinguishable from both of the above. */
const PREVIEW_OVERTAKEN = {
    available: true,
    dataLabel: "bank.csv",
    separator: ",",
    header: ["Overtaken On", "Overtaken Memo", "Overtaken Value"],
    rows: [["01/01/2026", "OVERTAKEN-PREVIEW-ROW", "-1.00"]],
    columns: 3,
    truncated: false,
};

const index = (revision: string) => ({
    rootLabel: "2026",
    editable: true,
    truncated: false,
    warnings: [],
    files: [
        {
            id: ID,
            label: GOLDEN.label,
            revision,
            sizeBytes: 512,
            parsed: true,
            ifBlockCount: 3,
            editableBlockCount: 3,
            opaqueItemCount: 1,
            warnings: [],
        },
    ],
});

/** Every request the panel made, so a test can prove the PUT happened. */
let calls: string[] = [];

/** One answer to one preview request. `park` holds the response open until a test releases it. */
type PreviewReply = {json: unknown} | {fail: true};
type PreviewAnswer = PreviewReply | {park: true};

/**
 * What the preview endpoint answers, request by request. The LAST entry repeats,
 * so a plan only has to describe the requests a test cares about.
 *
 * Request 0 is always the one the document's own load makes — `openRules`
 * fetches `{doc, preview}` as one value — so a plan's second entry is the first
 * post-save refetch.
 */
let previewPlan: PreviewAnswer[] = [];
let previewCalls = 0;
/** Resolvers for parked preview requests, oldest first, so a test can answer them OUT of order. */
let parked: ((reply: PreviewReply) => void)[] = [];

const json = (payload: unknown) => new Response(JSON.stringify(payload), {status: 200, headers: {"Content-Type": "application/json"}});
const replyOf = (reply: PreviewReply): Response => ("json" in reply ? json(reply.json) : new Response("preview unavailable", {status: 500}));

function nextPreviewAnswer(): PreviewAnswer {
    const answer = previewPlan[Math.min(previewCalls, previewPlan.length - 1)];
    previewCalls += 1;
    return answer ?? {json: PREVIEW_OPENED};
}

/** Answer a parked preview request. Deliberately by index: the race test answers the NEWER one first. */
function release(which: number, reply: PreviewReply): void {
    const resolve = parked[which];
    if (resolve === undefined) throw new Error(`no parked preview request at ${which} (have ${parked.length})`);
    resolve(reply);
}

/**
 * A `fetch` answering the PUT differently from the GET.
 *
 * `fakeEngine.routes` keys on URL alone, and the whole point here is that one URL
 * answers a warning-carrying document to a GET and a clean one to the PUT that
 * fixed it — and that the preview URL answers something different each time it
 * is asked.
 */
function stubEngine(): void {
    vi.stubGlobal("fetch", (input: unknown, init?: RequestInit) => {
        const url = String(input);
        const method = init?.method ?? "GET";
        calls.push(`${method} ${url.replace(FAKE_ENGINE, "")}`);

        if (url.endsWith("/version")) return Promise.resolve(json("1.52"));
        // Checked before `/api/rules/`, which is a prefix of neither but shares
        // enough of the path to make ordering worth being explicit about.
        if (url.includes("/api/rules-preview/")) {
            const answer = nextPreviewAnswer();
            if ("park" in answer) return new Promise<Response>((done) => parked.push((reply) => done(replyOf(reply))));
            return Promise.resolve(replyOf(answer));
        }
        if (url.includes(`/api/rules/${ID}`)) return Promise.resolve(json(method === "PUT" ? AFTER : BEFORE));
        if (url.endsWith("/api/rules")) return Promise.resolve(json(index(method === "PUT" ? AFTER.revision : GOLDEN.revision)));
        return Promise.resolve(new Response(`no route for ${url}`, {status: 404}));
    });
}

/** How many times the data file was re-read. */
const previewRequests = () => calls.filter((call) => call.startsWith(`GET /api/rules-preview/`)).length;

const clickTab = (name: string) => fireEvent.click(screen.getByRole("tab", {name}));

/** Dirty `skip` — a preview INPUT — and save. Distinct values keep successive saves dirty. */
async function saveSkip(value: string): Promise<void> {
    await clickTab("Preferences");
    await fireEvent.input(await screen.findByLabelText("Header lines to skip"), {target: {value}});
    await fireEvent.click(screen.getByRole("button", {name: /^Save$/}));
    await vi.waitFor(() => expect(calls).toContain(`PUT /api/rules/${ID}`));
}

beforeEach(async () => {
    calls = [];
    previewPlan = [{json: PREVIEW_OPENED}];
    previewCalls = 0;
    parked = [];
    stubEngine();
    if (settings.serverUrl === null) await settings.setServerUrl(FAKE_ENGINE);
    // `reloadIndex`, not `ensureIndex`: the latter dedupes on (nonce, url), which
    // is stable across tests in this file, so the second test would render
    // against no listing at all.
    await rulesStore.reloadIndex(FAKE_ENGINE);
});

afterEach(() => vi.unstubAllGlobals());

describe("COMPONENT EditRulesPanel", () => {
    it("shows the parse warnings the engine reported for the open document", async () => {
        render(EditRulesPanel);

        const banner = await screen.findByTestId("imports-warnings");
        expect(banner.textContent).toContain(STALE_WARNING);
        expect(banner.textContent).toContain("Line 3:");
    });

    it("clears a warning the save fixed, without refetching the document", async () => {
        render(EditRulesPanel);
        await screen.findByTestId("imports-warnings");

        await saveSkip("7");

        // The PUT's own response — not a re-read — is what the banner must
        // believe. `openRules` is deliberately never refetched.
        await vi.waitFor(() => expect(screen.queryByTestId("imports-warnings")).toBeNull());
        expect(calls.filter((call) => call === `GET /api/rules/${ID}`)).toHaveLength(1);
    });

    it("re-reads the data file after a save, so the mapping shows the saved settings", async () => {
        previewPlan = [{json: PREVIEW_OPENED}, {json: PREVIEW_RESAVED}];
        render(EditRulesPanel);
        await screen.findByTestId("imports-open-file");

        await clickTab("Row mapping");
        expect(await screen.findByText("Txn Date")).toBeDefined();

        await saveSkip("7");

        // `skip` decides which record is the header, so the header the file was
        // OPENED with is now the wrong one. This is the whole bug: it used to
        // stay on screen until something unrelated re-opened the document.
        await clickTab("Row mapping");
        await vi.waitFor(() => expect(screen.getByText("Posted On")).toBeDefined());
        expect(screen.queryByText("Txn Date")).toBeNull();
        expect(screen.queryByText("OLD-PREVIEW-ROW")).toBeNull();

        // The DOCUMENT is still not refetched — only the data file is. The three
        // rules GETs are `no-store` with no ETag on purpose, and a conditional
        // GET here would be the thing that design exists to avoid.
        expect(previewRequests()).toBe(2);
        expect(calls.filter((call) => call === `GET /api/rules/${ID}`)).toHaveLength(1);
    });

    it("withholds the pre-save preview while the re-read is still in flight", async () => {
        previewPlan = [{json: PREVIEW_OPENED}, {park: true}];
        render(EditRulesPanel);
        await screen.findByTestId("imports-open-file");

        await saveSkip("7");
        await clickTab("Row mapping");

        // Mid-flight the old sample values describe settings that are no longer
        // the ones on disk, so they are not shown AND not described as a
        // failure — the request has not failed, it has not answered.
        await vi.waitFor(() => expect(screen.getByTestId("imports-preview-pending")).toBeDefined());
        expect(screen.queryByText("OLD-PREVIEW-ROW")).toBeNull();
        expect(screen.queryByTestId("imports-no-preview")).toBeNull();

        release(0, {json: PREVIEW_RESAVED});
        await vi.waitFor(() => expect(screen.getByText("FRESH-PREVIEW-ROW")).toBeDefined());
        expect(screen.queryByTestId("imports-preview-pending")).toBeNull();
    });

    it("says the re-read failed rather than leaving the pre-save preview up", async () => {
        previewPlan = [{json: PREVIEW_OPENED}, {fail: true}];
        render(EditRulesPanel);
        await screen.findByTestId("imports-open-file");

        await saveSkip("7");
        await clickTab("Row mapping");

        // The save LANDED, so this may not be reported as a save failure — but
        // the columns are now unverified, and saying so is the only honest
        // answer. Falling back to the pre-save preview would show wrong data
        // with nothing on screen to suggest it was wrong.
        await vi.waitFor(() => expect(screen.getByTestId("imports-no-preview").textContent).toContain("the preview request failed"));
        expect(screen.queryByText("OLD-PREVIEW-ROW")).toBeNull();
        expect(screen.queryByTestId("imports-server-error")).toBeNull();
        expect(screen.getByTestId("imports-saved")).toBeDefined();
    });

    it("drops a preview response that a newer save has already superseded", async () => {
        previewPlan = [{json: PREVIEW_OPENED}, {park: true}];
        render(EditRulesPanel);
        await screen.findByTestId("imports-open-file");

        await saveSkip("7");
        await saveSkip("9");
        await vi.waitFor(() => expect(parked).toHaveLength(2));

        // Out of order on purpose: the SECOND save's re-read answers first, and
        // the first save's answer arrives after it. Without the token the older
        // response lands last and wins, which is the same stale preview this
        // refetch exists to remove — only now it is stale by a race rather than
        // by design, and correspondingly harder to see.
        release(1, {json: PREVIEW_RESAVED});
        release(0, {json: PREVIEW_OVERTAKEN});

        await clickTab("Row mapping");
        await vi.waitFor(() => expect(screen.getByText("FRESH-PREVIEW-ROW")).toBeDefined());
        expect(screen.queryByText("OVERTAKEN-PREVIEW-ROW")).toBeNull();
        expect(screen.queryByText("Overtaken On")).toBeNull();
    });
});
