// `fileStore.checkDisk` — how the tab learns that the open scenario file was
// edited in another program — driven through the REAL `LedgelineApi` against a
// stubbed `fetch`, as `scenarioStore.svelte.test.ts` is.
//
// The engine's journal watcher never sees a scenario file (the main journal
// does not include it), so this re-read is the only way such an edit reaches
// the tab. What is pinned here:
//
//   1. a different revision is a change, the same one is not;
//   2. the check adopts NOTHING — in particular the held revision is not moved,
//      which is what makes a save of edits made before the change a 409 rather
//      than a silent overwrite;
//   3. an answer that a load or save has overtaken is dropped, never reported.

import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import {connectFakeEngine, FAKE_ENGINE} from "#lib/testing/fakeEngine.js";
import {fileStore} from "./fileStore.svelte";
import {emptyScenario} from "./scenarioModel";

const ID = "projection-plan.journal";
const ROUTE = `/api/projections/${ID}`;

const amount = (mantissa: string) => ({commodity: "$", quantity: {mantissa, places: 2}, precision: 2});

/** The wire body for the file, at `revision`, with rent at `mantissa`. Mutated between reads. */
const fileBody = (revision: string, mantissa: string) => ({
    id: ID,
    label: "plan",
    revision,
    writable: true,
    scenario: {
        name: "Plan",
        created: "2026-10-01",
        updated: "2026-10-01",
        lines: [
            {
                id: "rule:0:0",
                group: "rule:0",
                role: "flow",
                account: "expenses:rent",
                amount: amount(mantissa),
                period: {raw: "monthly", simple: "monthly", from: null, to: null},
                growth: null,
                opening: null,
                note: "",
                counter: null,
                gains: null,
                source: "journal",
            },
        ],
        events: [],
        cash: null,
    },
});

let onDisk = fileBody("fa-1", "420000");
/** Every PUT body the store sent. */
let puts: unknown[] = [];
/** Set to make the next GET wait until released. */
let gate: Promise<void> | null = null;

beforeEach(async () => {
    onDisk = fileBody("fa-1", "420000");
    puts = [];
    gate = null;
    await connectFakeEngine();
    vi.stubGlobal("fetch", async (url: string, init?: RequestInit) => {
        if (String(url).endsWith(ROUTE)) {
            if (init?.method === "PUT") {
                const body = JSON.parse(String(init.body)) as {revision: string};
                puts.push(body);
                // The engine's optimistic-concurrency check, in miniature.
                if (body.revision !== onDisk.revision) return new Response("changed on disk", {status: 409});
                onDisk = fileBody("fa-saved", "420000");
                return Response.json(onDisk);
            }
            if (gate !== null) await gate;
            return Response.json(onDisk);
        }
        return new Response(`no route for ${url}`, {status: 404});
    });
    fileStore.forget();
    const opened = await fileStore.open(FAKE_ENGINE, ID);
    if (!opened.ok) throw new Error(JSON.stringify(opened.failure));
});

afterEach(() => {
    fileStore.forget();
    vi.unstubAllGlobals();
});

describe("COMPONENT fileStore.checkDisk — edits made outside Ledgeline", () => {
    it("reports nothing when the file still has the revision the tab holds", async () => {
        expect(await fileStore.checkDisk(FAKE_ENGINE)).toEqual({kind: "unchanged"});
    });

    it("reports a change, with the file as it now reads, when the revision moved", async () => {
        onDisk = fileBody("fa-2", "430000");
        const check = await fileStore.checkDisk(FAKE_ENGINE);
        expect(check.kind).toBe("changed");
        if (check.kind !== "changed") return;
        expect(check.file.revision).toBe("fa-2");
        expect(check.file.scenario.lines[0].amount.quantity.m).toBe(430000n);
    });

    it("adopts nothing, so a save of the OLD edits is refused rather than overwriting the new file", async () => {
        onDisk = fileBody("fa-2", "430000");
        await fileStore.checkDisk(FAKE_ENGINE);

        const saved = await fileStore.save(FAKE_ENGINE, emptyScenario());
        expect(saved.ok).toBe(false);
        expect(puts).toEqual([expect.objectContaining({revision: "fa-1"})]);
        expect(onDisk.revision).toBe("fa-2");
    });

    it("once the caller adopts the change, the next save carries the NEW revision", async () => {
        onDisk = fileBody("fa-2", "430000");
        const check = await fileStore.checkDisk(FAKE_ENGINE);
        if (check.kind !== "changed") throw new Error("expected a change");
        fileStore.adopted(check.file);

        expect((await fileStore.save(FAKE_ENGINE, emptyScenario())).ok).toBe(true);
        expect(puts).toEqual([expect.objectContaining({revision: "fa-2"})]);
    });

    it("drops an answer a save overtook, which would otherwise report the tab's own write as somebody else's", async () => {
        let release = (): void => {};
        gate = new Promise((resolve) => (release = resolve));
        const checking = fileStore.checkDisk(FAKE_ENGINE);
        // The file moves under the check, then OUR save lands and moves the revision.
        onDisk = fileBody("fa-1", "420000");
        gate = null;
        expect((await fileStore.save(FAKE_ENGINE, emptyScenario())).ok).toBe(true);
        onDisk = fileBody("fa-old-read", "999900");
        release();
        expect(await checking).toEqual({kind: "skipped"});
    });

    it("runs one check at a time, so a poll tick and a focus event do not both read the file", async () => {
        let release = (): void => {};
        gate = new Promise((resolve) => (release = resolve));
        const first = fileStore.checkDisk(FAKE_ENGINE);
        expect(await fileStore.checkDisk(FAKE_ENGINE)).toEqual({kind: "skipped"});
        release();
        expect(await first).toEqual({kind: "unchanged"});
    });

    it("says why when the file can no longer be read", async () => {
        vi.stubGlobal("fetch", async () => new Response('"projection-plan.journal" cannot be opened as a projection: line 2', {status: 400}));
        const check = await fileStore.checkDisk(FAKE_ENGINE);
        expect(check.kind).toBe("failed");
        if (check.kind === "failed") expect(check.failure.message).toContain("cannot be opened");
    });

    it("has nothing to check when the scenario on screen came from no file", async () => {
        fileStore.forget();
        expect(await fileStore.checkDisk(FAKE_ENGINE)).toEqual({kind: "skipped"});
    });
});
