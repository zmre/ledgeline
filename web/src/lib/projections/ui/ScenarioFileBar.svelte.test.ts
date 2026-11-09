// The scenario file bar's STAR (plan 33 D20, D21): when it shows, when it is
// pressed, and why it is disabled. The tooltips are `opening.ts`'s
// `defaultBlocker`, as the page computes them, so the sentence asserted is the
// one a user sees.

import {cleanup, fireEvent, render, screen} from "@testing-library/svelte";
import {describe, expect, it, vi} from "vitest";
import {defaultBlocker} from "../opening";
import type {ProjectionFile} from "../types";
import ScenarioFileBar from "./ScenarioFileBar.svelte";

const file = (id: string, over: Partial<ProjectionFile> = {}): ProjectionFile => ({
    id,
    label: id.replace(/\.journal$/, ""),
    name: null,
    created: null,
    updated: null,
    isProjection: id.startsWith("projection-"),
    sizeBytes: 100,
    writable: true,
    isDefault: false,
    ...over,
});

const ALPHA = file("projection-alpha.journal", {name: "Alpha"});
const BUDGET = file("budget.journal", {writable: false});
const INCLUDED = file("projection-inc.journal", {name: "Included", writable: false});
const FILES = [ALPHA, INCLUDED, BUDGET];

function mount(
    over: {
        currentId?: string | null;
        dirty?: boolean;
        editable?: boolean;
        busy?: boolean;
        isDefault?: boolean;
        defaultSupported?: boolean;
        defaultId?: string | null;
    } = {}
) {
    const currentId = over.currentId === undefined ? ALPHA.id : over.currentId;
    const editable = over.editable ?? true;
    const onToggleDefault = vi.fn();
    render(ScenarioFileBar, {
        files: FILES,
        currentId,
        canSave: true,
        dirty: over.dirty ?? false,
        editable,
        busy: over.busy ?? false,
        truncated: false,
        onOpen: vi.fn(),
        onSave: vi.fn(),
        onSaveAs: vi.fn(),
        defaultSupported: over.defaultSupported ?? true,
        defaultId: over.defaultId ?? null,
        isDefault: over.isDefault ?? false,
        defaultBlocker: defaultBlocker(currentId, FILES.find((f) => f.id === currentId) ?? null, editable),
        onToggleDefault,
    });
    return {onToggleDefault};
}

const star = () => screen.getByTestId("projection-default-toggle");

describe("COMPONENT ScenarioFileBar — the default star", () => {
    it("is hidden on an engine that predates the marker", () => {
        mount({defaultSupported: false});
        expect(screen.queryByTestId("projection-default-toggle")).toBeNull();
    });

    it("is enabled, and clickable, with unsaved edits", async () => {
        const {onToggleDefault} = mount({dirty: true});
        expect((star() as HTMLButtonElement).disabled).toBe(false);
        await fireEvent.click(star());
        expect(onToggleDefault).toHaveBeenCalledOnce();
    });

    it("is disabled for a seed, another journal, a file the main journal includes, and a read-only server, each saying why", () => {
        const cases: [Parameters<typeof mount>[0], RegExp][] = [
            [{currentId: null}, /Save this scenario to a file first/],
            [{currentId: BUDGET.id}, /Only a scenario file Ledgeline saved/],
            [{currentId: INCLUDED.id}, /part of your main journal/],
            [{editable: false}, /can't write to this journal's folder/],
        ];
        for (const [over, reason] of cases) {
            mount(over);
            expect((star() as HTMLButtonElement).disabled, String(reason)).toBe(true);
            expect(star().getAttribute("title")).toMatch(reason);
            cleanup();
        }
    });

    it("is disabled while busy", () => {
        mount({busy: true});
        expect((star() as HTMLButtonElement).disabled).toBe(true);
    });

    it("is pressed, with the clearing label, when the open file is the default", () => {
        mount({isDefault: true, defaultId: ALPHA.id});
        expect(star().getAttribute("aria-pressed")).toBe("true");
        expect(star().getAttribute("aria-label")).toBe("Default scenario — click to stop opening it by default");
    });

    it("is not pressed, and offers to set it, otherwise", () => {
        mount();
        expect(star().getAttribute("aria-pressed")).toBe("false");
        expect(star().getAttribute("aria-label")).toBe("Open this scenario by default");
    });

    it("the picker names the default with a suffix", () => {
        mount({defaultId: ALPHA.id});
        const options = Array.from(screen.getByTestId("projection-file-select").querySelectorAll("option")).map((option) => option.textContent);
        expect(options).toEqual(["Seeded from your journal", "Alpha (default)", "Included", "budget"]);
    });

    it("shows the listing's conflict warning", () => {
        render(ScenarioFileBar, {
            files: FILES,
            currentId: ALPHA.id,
            canSave: true,
            dirty: false,
            editable: true,
            busy: false,
            truncated: false,
            onOpen: vi.fn(),
            onSave: vi.fn(),
            onSaveAs: vi.fn(),
            defaultSupported: true,
            defaultWarning: "2 scenario files are marked as the default (a, b); the first one opens. Star one to fix it.",
        });
        expect(screen.getByTestId("projection-default-conflict").textContent).toMatch(/Star one to fix it/);
    });
});
