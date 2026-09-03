// The QuickBooks Journal import store, driven end to end against a stubbed
// fetch — the same seam `importStore.test.ts` uses, and for the same reason:
// what can go wrong here is the WIRING (which stage a preview answers for,
// whether a mapping save reuses the alias wire correctly, whether a commit's
// per-file ordering reaches the screen), not the decisions, which live in
// `qbJournalModel.ts` and are tested there.
//
// EVERY test gets a fresh module graph (`vi.resetModules()` + a dynamic
// re-import). `qbJournalStore`, the `preview`/`commit` resources it wraps,
// and the `aliasStore` singleton it reuses for mapping saves are all
// module-scope state shared across a whole test FILE — and this file's own
// "fetch once per stage" test would otherwise be reading state a PRIOR
// test's stage already primed, exactly the kind of cross-test bleed
// `importStore.test.ts`'s own "fresh store" test guards against.

import {afterEach, describe, expect, it, vi} from "vitest";

const json = (body: unknown): Response => new Response(JSON.stringify(body), {status: 200, headers: {"Content-Type": "application/json"}});

type FetchCall = [string, RequestInit | undefined];

function routes(table: Record<string, unknown>): ReturnType<typeof vi.fn> {
    return vi.fn((url: string) => {
        const key = Object.keys(table).find((route) => url.endsWith(route));
        const body = key === undefined ? undefined : table[key];
        if (body === undefined) return Promise.resolve(new Response("no route", {status: 404}));
        return Promise.resolve(body instanceof Response ? body : json(body));
    });
}

function calls(mock: ReturnType<typeof vi.fn>): FetchCall[] {
    return mock.mock.calls as FetchCall[];
}

function bodyOf(mock: ReturnType<typeof vi.fn>, suffix: string): unknown {
    const call = calls(mock).find(([url]) => url.endsWith(suffix));
    if (call === undefined) throw new Error(`no request to ${suffix}`);
    return JSON.parse(call[1]?.body as string);
}

/** A fresh, isolated `qbJournalStore` connected to a fake engine serving `table`. */
async function freshStore(table: Record<string, unknown>): Promise<{
    store: (typeof import("./qbJournalStore.svelte"))["qbJournalStore"];
    fetchMock: ReturnType<typeof vi.fn>;
}> {
    vi.resetModules();
    const fetchMock = routes({"/version": "1.52", ...table});
    vi.stubGlobal("fetch", fetchMock);
    const {settings} = await import("$lib/stores/settings.svelte");
    await settings.setServerUrl("http://engine.test");
    const {qbJournalStore} = await import("./qbJournalStore.svelte");
    return {store: qbJournalStore, fetchMock};
}

afterEach(() => vi.unstubAllGlobals());

const PREVIEW_UNMAPPED = {
    stageId: "s1",
    transactionCount: 2,
    postingCount: 4,
    dateFormat: {format: "%m/%d/%Y", ambiguous: false},
    unmappedAccounts: ["Riverbank BUSINESS CHECKING (0002)", "3000 Member Equity"],
    sample: [
        {id: "441", date: "2026-01-05", description: "Deposit", postings: ["Riverbank BUSINESS CHECKING (0002)  1000.00", "3000 Member Equity  -1000.00"]},
    ],
    idMatches: null,
};

const PREVIEW_MAPPED = {
    ...PREVIEW_UNMAPPED,
    unmappedAccounts: [],
    idMatches: {new: 2, unchanged: 0, conflicting: [], conflictingTotal: 0},
};

const ALIASES = {
    editable: true,
    files: [{journalId: "main.journal", label: "main.journal", revision: "rev-1", writable: true, aliases: []}],
};

const COMMIT = {
    imported: 2,
    idMatches: {new: 2, unchanged: 0, conflicting: [], conflictingTotal: 0},
    ordering: {
        inOrder: false,
        files: [{journalId: "main.journal", inOrder: false, moves: [{date: "2026-01-05", description: "Deposit", fromLine: 10, toLine: 4}]}],
    },
    git: null,
};

describe("UNIT qbJournalStore — preview", () => {
    it("loads the preview for a stage and reports which accounts are unmapped", async () => {
        const {store} = await freshStore({"/api/import/qb-journal/s1": PREVIEW_UNMAPPED});
        await store.ensurePreview("http://engine.test", "s1");

        expect(store.previewView).toBe("data");
        expect(store.preview?.unmappedAccounts).toEqual(["Riverbank BUSINESS CHECKING (0002)", "3000 Member Equity"]);
        expect(store.preview?.transactionCount).toBe(2);
    });

    it("fetches once per stage — a repeated call for the same stage is a no-op", async () => {
        const {store, fetchMock} = await freshStore({"/api/import/qb-journal/s1": PREVIEW_UNMAPPED});
        await store.ensurePreview("http://engine.test", "s1");
        await store.ensurePreview("http://engine.test", "s1");

        expect(calls(fetchMock).filter(([url]) => url.endsWith("/qb-journal/s1")).length).toBe(1);
    });

    it("refreshPreview always refetches, unlike ensurePreview", async () => {
        const {store, fetchMock} = await freshStore({"/api/import/qb-journal/s1": PREVIEW_UNMAPPED});
        await store.ensurePreview("http://engine.test", "s1");
        await store.refreshPreview("http://engine.test", "s1");

        expect(calls(fetchMock).filter(([url]) => url.endsWith("/qb-journal/s1")).length).toBe(2);
    });

    it("reports a stage the server does not recognise as an error surface", async () => {
        const {store} = await freshStore({"/api/import/qb-journal/gone": new Response("not staged", {status: 404})});
        await store.ensurePreview("http://engine.test", "gone");

        expect(store.previewView).toBe("error");
        expect(store.previewError).not.toBeNull();
    });
});

describe("UNIT qbJournalStore — resolving unmapped accounts", () => {
    it("refuses to submit when nothing was typed, without making a request", async () => {
        const {store, fetchMock} = await freshStore({"/api/import/qb-journal/s1": PREVIEW_UNMAPPED, "/api/aliases": ALIASES});
        await store.ensurePreview("http://engine.test", "s1");

        const ok = await store.saveMappings("http://engine.test", "s1", PREVIEW_UNMAPPED.unmappedAccounts);

        expect(ok).toBe(false);
        expect(store.mappingError).toMatch(/type an account/i);
        expect(calls(fetchMock).some(([url]) => url.includes("/api/aliases/"))).toBe(false);
    });

    it("submits every typed row through the EXISTING alias-editing wire, then re-fetches the preview", async () => {
        const {store, fetchMock} = await freshStore({
            "/api/import/qb-journal/s1": PREVIEW_UNMAPPED,
            "/api/aliases": ALIASES,
            "/api/aliases/main.journal": {...ALIASES.files[0], revision: "rev-2"},
        });
        await store.ensurePreview("http://engine.test", "s1");
        store.setDraft("Riverbank BUSINESS CHECKING (0002)", "assets:bank:checking");
        store.setDraft("3000 Member Equity", "equity:opening");

        // A repeated call for /api/import/qb-journal/s1 must answer the MAPPED
        // shape the second time (the refresh after the save), so swap the fetch
        // stub's route to that once the alias write has happened.
        fetchMock.mockImplementation((url: string) => {
            if (url.endsWith("/api/aliases/main.journal")) return Promise.resolve(json({...ALIASES.files[0], revision: "rev-2"}));
            if (url.endsWith("/api/aliases")) return Promise.resolve(json(ALIASES));
            if (url.endsWith("/version")) return Promise.resolve(json("1.52"));
            if (url.endsWith("/qb-journal/s1")) return Promise.resolve(json(PREVIEW_MAPPED));
            return Promise.resolve(new Response("no route", {status: 404}));
        });

        const ok = await store.saveMappings("http://engine.test", "s1", PREVIEW_UNMAPPED.unmappedAccounts);

        expect(ok).toBe(true);
        expect(store.mappingError).toBeNull();
        const sent = bodyOf(fetchMock, "/api/aliases/main.journal") as {revision: string; edits: unknown[]};
        expect(sent).toEqual({
            revision: "rev-1",
            edits: [
                {kind: "append", pattern: "Riverbank BUSINESS CHECKING (0002)", replacement: "assets:bank:checking", regex: false},
                {kind: "append", pattern: "3000 Member Equity", replacement: "equity:opening", regex: false},
            ],
        });
        // The preview was re-fetched, and the accounts that resolved are gone.
        expect(store.preview?.unmappedAccounts).toEqual([]);
    });

    it("surfaces a conflict (the journal changed underneath the edit) as a mapping error, and leaves the preview alone", async () => {
        const {store, fetchMock} = await freshStore({
            "/api/import/qb-journal/s1": PREVIEW_UNMAPPED,
            "/api/aliases": ALIASES,
            "/api/aliases/main.journal": new Response("the file changed on disk", {status: 409}),
        });
        await store.ensurePreview("http://engine.test", "s1");
        store.setDraft("Riverbank BUSINESS CHECKING (0002)", "assets:bank:checking");

        const ok = await store.saveMappings("http://engine.test", "s1", PREVIEW_UNMAPPED.unmappedAccounts);

        expect(ok).toBe(false);
        expect(store.mappingError).toMatch(/changed/i);
        // Only ONE call to the preview route — the initial load. A conflict does
        // not trigger the post-save refresh, since nothing was actually written.
        expect(calls(fetchMock).filter(([url]) => url.endsWith("/qb-journal/s1")).length).toBe(1);
    });
});

describe("UNIT qbJournalStore — commit", () => {
    it("is not requested before the button is pressed", async () => {
        const {store} = await freshStore({});
        expect(store.commitRequested).toBe(false);
        expect(store.commitResult).toBeNull();
    });

    it("loads what was written, including per-file ordering", async () => {
        const {store} = await freshStore({"/api/import/qb-journal/commit": COMMIT});
        await store.commitStage("http://engine.test", "s1");

        expect(store.commitRequested).toBe(true);
        expect(store.commitView).toBe("data");
        expect(store.commitResult?.imported).toBe(2);
        expect(store.commitResult?.ordering.files[0]?.inOrder).toBe(false);
    });

    it("sends only the stage id — no journalId, per the pipeline's own contract", async () => {
        const {store, fetchMock} = await freshStore({"/api/import/qb-journal/commit": COMMIT});
        await store.commitStage("http://engine.test", "s1");

        expect(bodyOf(fetchMock, "/api/import/qb-journal/commit")).toEqual({stageId: "s1"});
    });

    it("surfaces the 400 refusal naming unmapped accounts as an error, not a thrown crash", async () => {
        const {store} = await freshStore({
            "/api/import/qb-journal/commit": new Response("these QuickBooks accounts have no matching alias: 3000 Member Equity", {status: 400}),
        });
        await store.commitStage("http://engine.test", "s1");

        expect(store.commitView).toBe("error");
        expect(store.commitError?.message).toMatch(/3000 Member Equity/);
    });

    it("outlives the shared 30s request timeout — a bulk write of thousands of rows is not a hung connection", async () => {
        // The exact shape `client.test.ts`'s own "passes an abort signal to
        // fetch and fails a response that never arrives" test uses for
        // REQUEST_TIMEOUT_MS, aimed at this route's own, longer deadline.
        vi.useFakeTimers();
        vi.resetModules();
        const fetchMock = vi.fn().mockImplementation((url: string, init?: RequestInit) => {
            if ((url as string).endsWith("/version")) return Promise.resolve(json("1.52"));
            return new Promise((_resolve, reject) => {
                init?.signal?.addEventListener("abort", () => reject(new DOMException("aborted", "AbortError")), {once: true});
            });
        });
        vi.stubGlobal("fetch", fetchMock);
        const {settings} = await import("$lib/stores/settings.svelte");
        await settings.setServerUrl("http://engine.test");
        const {qbJournalStore, QB_JOURNAL_COMMIT_TIMEOUT_MS} = await import("./qbJournalStore.svelte");
        const {REQUEST_TIMEOUT_MS} = await import("$lib/api/client");

        const pending = qbJournalStore.commitStage("http://engine.test", "s1");
        // Past the shared 30s default: a hung connection would already have
        // been aborted by now under the old, shared timeout. This route must
        // still be waiting.
        await vi.advanceTimersByTimeAsync(REQUEST_TIMEOUT_MS + 1);
        expect(qbJournalStore.commitView).not.toBe("error");

        // Past ITS OWN, longer deadline: still fails closed eventually,
        // rather than hanging forever.
        await vi.advanceTimersByTimeAsync(QB_JOURNAL_COMMIT_TIMEOUT_MS - REQUEST_TIMEOUT_MS + 1);
        await pending;
        expect(qbJournalStore.commitView).toBe("error");

        vi.useRealTimers();
    });
});

describe("UNIT qbJournalStore — the post-commit re-sort", () => {
    it("reports how many transactions moved for the file that was sorted", async () => {
        const {store} = await freshStore({"/api/import/sort": {moved: 3, git: null}});
        await store.resortFile("http://engine.test", "main.journal");

        expect(store.sortMovedFor("main.journal")).toBe(3);
        expect(store.sortErrorFor("main.journal")).toBeNull();
        expect(store.sortingJournalId).toBeNull();
    });

    it("warns when the re-sort happened but git did not commit it", async () => {
        const {store} = await freshStore({
            "/api/import/sort": {moved: 1, git: {committed: false, paths: [], skipped: [], message: "pre-commit hook rejected"}},
        });
        await store.resortFile("http://engine.test", "main.journal");

        expect(store.sortMovedFor("main.journal")).toBe(1);
        expect(store.sortErrorFor("main.journal")).toMatch(/git did not commit/);
    });

    it("reports a failed re-sort as an error, without throwing", async () => {
        const {store} = await freshStore({"/api/import/sort": new Response("boom", {status: 500})});

        await store.resortFile("http://engine.test", "main.journal");

        expect(store.sortErrorFor("main.journal")).not.toBeNull();
        expect(store.sortMovedFor("main.journal")).toBeNull();
    });
});
