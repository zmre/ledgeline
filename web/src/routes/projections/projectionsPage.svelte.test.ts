// The Projections PAGE: what opens on mount, and what survives a revisit
// (plan 33 D2, D3, D9, D17), mounted for real in jsdom against a stubbed
// `fetch` (`#lib/testing/fakeEngine`).
//
// Page-level because the property is the page's: `start()` decides between the
// default, the last file and a seed, and whether to do anything at all. A
// store test would prove the stores can each do their part, which was never
// the question — the revisit bug was two correct stores driven in the wrong
// order.
//
// Every test gets a FRESH module graph (`vi.resetModules` and dynamic imports),
// because "the first mount after the page loads" is a property of module
// state: `scenarioStore.adopted` is what a page load resets. Within one test,
// the same graph is reused across mounts, exactly as a tab switch reuses it.

import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";

// The URL mirror writes through SvelteKit's router, which a bare mount has not
// started. What the URL says is not under test here.
vi.mock("$app/navigation", () => ({replaceState: vi.fn(), goto: vi.fn()}));

const A = "projection-a.journal";
const B = "projection-b.journal";

const amount = (mantissa: string) => ({commodity: "$", quantity: {mantissa, places: 2}, precision: 2});

/** A scenario body with one rent line at `mantissa`. */
const scenarioBody = (name: string, mantissa: string) => ({
    name,
    created: "2026-10-01",
    updated: "2026-10-01",
    lines: [
        {
            id: "rule:0:0",
            group: "rule:0",
            role: "flow",
            account: "expenses:rent",
            amount: amount(mantissa),
            period: {raw: "monthly", simple: "monthly", base: null, from: null, to: null},
            growth: null,
            opening: null,
            note: "",
            counter: null,
            gains: null,
            source: "journal",
            comment: "",
            ruleComment: "",
        },
    ],
    events: [],
    cash: null,
    autoRules: [],
    accountTypes: {},
    layout: [],
    comments: [],
    fixed: [],
});

const fileBody = (id: string, revision: string, isDefault = false) => ({
    id,
    label: id.replace(/^projection-/, "").replace(/\.journal$/, ""),
    revision,
    writable: true,
    isDefault,
    scenario: scenarioBody(id === A ? "Alpha" : "Beta", id === A ? "100000" : "200000"),
});

const listed = (id: string, isDefault: boolean) => ({
    id,
    label: id.replace(/^projection-/, "").replace(/\.journal$/, ""),
    name: id === A ? "Alpha" : "Beta",
    created: null,
    updated: null,
    isProjection: true,
    sizeBytes: 100,
    writable: true,
    isDefault,
});

/** What the fake engine holds: which file is marked, each file's revision, and a GET answer override. */
interface Disk {
    marked: string | null;
    revisions: Record<string, string>;
    /** A non-200 answer for a file's GET. */
    broken: Record<string, Response>;
}

let disk: Disk;
/** `METHOD path` for every engine request the page made, in order. */
let requests: string[] = [];
/** Every PUT body, by path. */
let puts: {path: string; body: unknown}[] = [];

const json = (body: unknown) => Response.json(body);

function engine(url: string, init?: RequestInit): Response {
    const path = new URL(url).pathname;
    const method = init?.method ?? "GET";
    requests.push(`${method} ${path}`);
    if (method === "PUT") {
        const body: unknown = JSON.parse(String(init?.body));
        puts.push({path, body});
        if (path === "/api/projections/default") {
            // The engine's D13 in miniature: unmark the old, mark the new.
            const id = (body as {id: string | null}).id;
            const changed = [];
            for (const file of [A, B]) {
                const on = file === id;
                if ((disk.marked === file) === on) continue;
                const before = disk.revisions[file];
                const after = `${before}+${on ? "star" : "unstar"}`;
                disk.revisions[file] = after;
                changed.push({id: file, before, after, isDefault: on});
            }
            disk.marked = id;
            return json({changed});
        }
        const id = path.replace("/api/projections/", "");
        const sent = (body as {revision: string}).revision;
        if (sent !== disk.revisions[id]) return new Response(`"${id}" changed on disk since you opened it`, {status: 409});
        disk.revisions[id] = `${sent}+saved`;
        return json(fileBody(id, disk.revisions[id], disk.marked === id));
    }
    if (path === "/api/projections") {
        return json({
            rootLabel: "books",
            editable: true,
            truncated: false,
            files: [listed(A, disk.marked === A), listed(B, disk.marked === B)],
            directories: [""],
            warnings: [],
        });
    }
    if (path === "/api/projections/seed") return json(scenarioBody("", "50000"));
    // No `/api/projections/run`: the projection below the table answers with
    // an error, so jsdom never has to draw a chart (it has no layout engine).
    const id = path.replace("/api/projections/", "");
    if (id === A || id === B) return disk.broken[id] ?? json(fileBody(id, disk.revisions[id], disk.marked === id));
    if (path === "/version") return json("1.52");
    if (path === "/api/journal") return json(FAKE_JOURNAL_INFO);
    if (path === "/api/diagnostics") return json({count: 0, maxSeverity: null, diagnostics: []});
    return new Response(`no route for ${url}`, {status: 404});
}

let FAKE_JOURNAL_INFO: Record<string, unknown>;
/** The current graph's testing-library `cleanup`. */
let unmountAll = (): void => {};

/** A fresh page load: new stores, `lastProjectionId` as given, connected to the fake engine. */
async function pageLoad(lastProjectionId: string | null) {
    vi.resetModules();
    const fake = await import("#lib/testing/fakeEngine.js");
    FAKE_JOURNAL_INFO = fake.FAKE_JOURNAL_INFO;
    const {settings} = await import("#lib/stores/settings.svelte.js");
    vi.stubGlobal("fetch", async (url: string, init?: RequestInit) => engine(String(url), init));
    await settings.setServerUrl(fake.FAKE_ENGINE);
    settings.lastProjectionId = lastProjectionId;
    const {fileStore} = await import("#lib/projections/fileStore.svelte.js");
    const {scenarioStore} = await import("#lib/projections/scenarioStore.svelte.js");
    const {default: Page} = await import("./+page.svelte");
    // From the SAME fresh graph: a `render` bound to the previous Svelte
    // runtime would mount the page outside any component context.
    const {render, cleanup} = await import("@testing-library/svelte");
    // Its own cleanup, too: the setup file's auto-cleanup belongs to the
    // original instance and never sees these mounts.
    unmountAll = cleanup;
    return {settings, fileStore, scenarioStore, mount: () => render(Page)};
}

const gets = (path: string) => requests.filter((request) => request === `GET ${path}`).length;

beforeEach(() => {
    localStorage.clear();
    disk = {marked: null, revisions: {[A]: "ra-1", [B]: "rb-1"}, broken: {}};
    requests = [];
    puts = [];
});

afterEach(() => {
    unmountAll();
    vi.unstubAllGlobals();
});

describe("COMPONENT Projections page — what opens, and what a revisit keeps", () => {
    it("a revisit keeps the open file's unsaved edits and does not re-read it", async () => {
        const page = await pageLoad(A);
        const first = page.mount();
        await vi.waitFor(() => expect(page.fileStore.currentId).toBe(A));
        await vi.waitFor(() => expect(page.scenarioStore.scenario.name).toBe("Alpha"));

        page.scenarioStore.commit({...page.scenarioStore.scenario, name: "Alpha, edited"});
        expect(page.scenarioStore.dirty).toBe(true);
        first.unmount();

        // Another tab, then back: a new page component over the same stores.
        page.mount();
        await new Promise((resolve) => setTimeout(resolve, 50));
        expect(page.scenarioStore.scenario.name).toBe("Alpha, edited");
        expect(page.scenarioStore.dirty).toBe(true);
        expect(gets(`/api/projections/${A}`)).toBe(1);
    });

    it("the default wins on the first mount, and the last file is never asked for", async () => {
        disk.marked = B;
        const page = await pageLoad(A);
        page.mount();
        await vi.waitFor(() => expect(page.fileStore.currentId).toBe(B));
        expect(gets(`/api/projections/${A}`)).toBe(0);
        expect(page.settings.lastProjectionId).toBe(B);
    });

    it("a default that cannot be opened says so, then opens the last file, which is not forgotten", async () => {
        disk.marked = B;
        disk.broken[B] = new Response(`"${B}" cannot be opened as a projection: line 3`, {status: 400});
        const page = await pageLoad(A);
        const view = page.mount();
        await vi.waitFor(() => expect(page.fileStore.currentId).toBe(A));
        const error = await view.findByTestId("projection-file-error");
        expect(error.textContent).toContain("Your default scenario couldn't be opened");
        expect(error.textContent).toContain(B);
        expect(page.settings.lastProjectionId).toBe(A);
        expect(gets("/api/projections/seed")).toBe(0);
    });

    it("with no default: the last file, as today", async () => {
        const page = await pageLoad(A);
        page.mount();
        await vi.waitFor(() => expect(page.fileStore.currentId).toBe(A));
        expect(gets(`/api/projections/${B}`)).toBe(0);
        expect(gets("/api/projections/seed")).toBe(0);
    });

    it("with no default and no last file: a seed, as today", async () => {
        const page = await pageLoad(null);
        page.mount();
        await vi.waitFor(() => expect(gets("/api/projections/seed")).toBe(1));
        await vi.waitFor(() => expect(page.scenarioStore.adopted).toBe(true));
        expect(page.fileStore.currentId).toBeNull();
        expect(requests.filter((request) => request.startsWith("GET /api/projections/projection-"))).toEqual([]);
    });

    it("with no default and a last file that no longer opens: forgotten, then a seed, as today", async () => {
        disk.broken[A] = new Response(`"${A}" cannot be opened as a projection: line 1`, {status: 400});
        const page = await pageLoad(A);
        page.mount();
        await vi.waitFor(() => expect(gets("/api/projections/seed")).toBe(1));
        expect(page.settings.lastProjectionId).toBeNull();
    });

    it("starring a file with unsaved edits keeps them, and the next save carries the star's revision", async () => {
        const page = await pageLoad(B);
        const view = page.mount();
        await vi.waitFor(() => expect(page.fileStore.currentId).toBe(B));
        page.scenarioStore.commit({...page.scenarioStore.scenario, name: "Beta, edited"});

        const star = await view.findByTestId("projection-default-toggle");
        expect(star.getAttribute("aria-pressed")).toBe("false");
        star.click();
        await vi.waitFor(() => expect(star.getAttribute("aria-pressed")).toBe("true"));
        expect(page.scenarioStore.dirty).toBe(true);
        expect(page.scenarioStore.scenario.name).toBe("Beta, edited");
        expect(puts.map((put) => put.path)).toEqual(["/api/projections/default"]);

        (await view.findByTestId("projection-save")).click();
        await vi.waitFor(() => expect(puts).toHaveLength(2));
        expect(puts[1]).toEqual({path: `/api/projections/${B}`, body: expect.objectContaining({revision: "rb-1+star"})});
        await vi.waitFor(() => expect(page.scenarioStore.dirty).toBe(false));
        expect(view.queryByTestId("projection-file-error")).toBeNull();
    });
});
