// Settings → Projections (plan 33 D22), mounted over the REAL `fileStore`
// against a stubbed `fetch` (`#lib/testing/fakeEngine`): which files it
// offers, what a choice sends, and what a refusal leaves on screen.

import {fireEvent, render, screen} from "@testing-library/svelte";
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import {connectFakeEngine} from "#lib/testing/fakeEngine.js";
import ProjectionSettingsPanel from "./ProjectionSettingsPanel.svelte";

const listed = (id: string, over: Record<string, unknown> = {}) => ({
    id,
    label: id.replace(/\.journal$/, ""),
    name: null,
    created: null,
    updated: null,
    isProjection: id.startsWith("projection-"),
    sizeBytes: 1,
    writable: true,
    isDefault: false,
    ...over,
});

interface Engine {
    files: ReturnType<typeof listed>[];
    editable: boolean;
    warnings: string[];
    /** What the next PUT answers; null applies it. */
    refuse: Response | null;
}

let engine: Engine;
let puts: unknown[] = [];

beforeEach(async () => {
    engine = {
        files: [
            listed("projection-alpha.journal", {name: "Alpha"}),
            listed("projection-beta.journal", {name: "Beta"}),
            listed("projection-inc.journal", {name: "Included", writable: false}),
            listed("budget.journal", {writable: false}),
        ],
        editable: true,
        warnings: [],
        refuse: null,
    };
    puts = [];
    await connectFakeEngine();
    vi.stubGlobal("fetch", async (url: string, init?: RequestInit) => {
        const path = new URL(String(url)).pathname;
        if (path === "/api/projections/default" && init?.method === "PUT") {
            const body = JSON.parse(String(init.body)) as {id: string | null};
            puts.push(body);
            if (engine.refuse !== null) return engine.refuse;
            const changed = engine.files
                .filter((file) => file.isDefault !== (file.id === body.id))
                .map((file) => ({id: file.id, before: "r1", after: "r2", isDefault: file.id === body.id}));
            engine.files = engine.files.map((file) => ({...file, isDefault: file.id === body.id}));
            return Response.json({changed});
        }
        if (path === "/api/projections") {
            return Response.json({
                rootLabel: "books",
                editable: engine.editable,
                truncated: false,
                files: engine.files,
                directories: [""],
                warnings: engine.warnings,
            });
        }
        if (path === "/version") return Response.json("1.52");
        return new Response(`no route for ${url}`, {status: 404});
    });
});

afterEach(() => {
    vi.unstubAllGlobals();
});

const select = async () => (await screen.findByTestId("settings-projections-default")) as HTMLSelectElement;
const options = async () => Array.from((await select()).querySelectorAll("option")).map((option) => option.value);

describe("COMPONENT ProjectionSettingsPanel", () => {
    it("selects None when nothing is the default", async () => {
        render(ProjectionSettingsPanel);
        expect((await select()).value).toBe("");
    });

    it("selects the default when there is one", async () => {
        engine.files[1] = {...engine.files[1], isDefault: true};
        render(ProjectionSettingsPanel);
        await vi.waitFor(async () => expect((await select()).value).toBe("projection-beta.journal"));
    });

    it("offers only scenario files the tab can write", async () => {
        render(ProjectionSettingsPanel);
        await vi.waitFor(async () => expect(await options()).toEqual(["", "projection-alpha.journal", "projection-beta.journal"]));
    });

    it("sends one PUT for a choice, and `{id: null}` for None", async () => {
        render(ProjectionSettingsPanel);
        const control = await select();
        await vi.waitFor(async () => expect(await options()).toContain("projection-alpha.journal"));
        await fireEvent.change(control, {target: {value: "projection-alpha.journal"}});
        await vi.waitFor(() => expect(puts).toEqual([{id: "projection-alpha.journal"}]));
        await vi.waitFor(() => expect(control.value).toBe("projection-alpha.journal"));

        await fireEvent.change(control, {target: {value: ""}});
        await vi.waitFor(() => expect(puts).toEqual([{id: "projection-alpha.journal"}, {id: null}]));
    });

    it("shows the listing's warning when several files are marked", async () => {
        engine.files[0] = {...engine.files[0], isDefault: true};
        engine.files[1] = {...engine.files[1], isDefault: true};
        engine.warnings = [
            "2 scenario files are marked as the default (projection-alpha.journal, projection-beta.journal); the first one opens. Star one to fix it.",
        ];
        render(ProjectionSettingsPanel);
        expect((await screen.findByTestId("settings-projections-conflict")).textContent).toMatch(/Star one to fix it/);
        await vi.waitFor(async () => expect((await select()).value).toBe("projection-alpha.journal"));
    });

    it("is read-only, and says so, when the server cannot write", async () => {
        engine.editable = false;
        render(ProjectionSettingsPanel);
        await screen.findByTestId("settings-projections-read-only");
        expect((await select()).disabled).toBe(true);
    });

    it("shows the engine's sentence for a refusal and keeps the previous selection", async () => {
        engine.refuse = new Response('"projection-alpha.journal" changed on disk while the default was being set. Nothing was written; try again.', {
            status: 409,
        });
        render(ProjectionSettingsPanel);
        const control = await select();
        await vi.waitFor(async () => expect(await options()).toContain("projection-alpha.journal"));
        await fireEvent.change(control, {target: {value: "projection-alpha.journal"}});
        expect((await screen.findByTestId("settings-projections-error")).textContent).toMatch(/Nothing was written/);
        expect(control.value).toBe("");
    });
});
