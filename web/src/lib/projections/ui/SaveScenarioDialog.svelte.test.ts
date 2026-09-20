// The Save As dialog and the file bar, mounted.
//
// The two claims only a mount can answer (web/README.md):
//
//   1. THE PATH IS SHOWN BEFORE THE WRITE. `projectionId` is unit-tested on its
//      own, and can be green while the dialog renders nothing — which would
//      make "the dialog shows the resulting path before saving" a sentence in a
//      plan rather than a thing on a screen.
//   2. THE REFUSALS ARE REACHABLE. A name that slugs to nothing, a name already
//      taken, and a file the main journal includes each have to stop a save at
//      the button, not at the server.
//
// Nothing asserts on geometry or computed CSS: jsdom has no layout engine. The
// modal is an ordinary div with `modal-open`, so everything in it is in the
// document and clickable.

import {fireEvent, render, screen} from "@testing-library/svelte";
import {describe, expect, it, vi} from "vitest";
import type {ProjectionFile} from "../types";
import SaveScenarioDialog from "./SaveScenarioDialog.svelte";
import ScenarioFileBar from "./ScenarioFileBar.svelte";

const file = (id: string, over: Partial<ProjectionFile> = {}): ProjectionFile => ({
    id,
    label: id.replace(/^.*\//, "").replace(/\.journal$/, ""),
    name: null,
    created: null,
    updated: null,
    isProjection: id.includes("projection-"),
    sizeBytes: 100,
    writable: true,
    ...over,
});

const FILES: ProjectionFile[] = [file("plans/projection-series-a.journal", {name: "Series A"}), file("budget.journal", {writable: false})];

function mountDialog(over: Partial<Parameters<typeof SaveScenarioDialog>[1]> = {}) {
    const onSave = vi.fn();
    const onCancel = vi.fn();
    render(SaveScenarioDialog, {
        name: "",
        directories: ["", "plans"],
        files: FILES,
        saving: false,
        error: null,
        onSave,
        onCancel,
        ...over,
    });
    return {onSave, onCancel};
}

describe("COMPONENT SaveScenarioDialog", () => {
    it("shows the resulting path as the name is typed", async () => {
        mountDialog();
        const name = screen.getByTestId("projection-save-name");
        await fireEvent.input(name, {target: {value: "Series B with a ramp"}});
        expect(screen.getByTestId("projection-save-path").textContent).toContain("projection-series-b-with-a-ramp.journal");
    });

    it("joins the chosen folder into the path it shows", async () => {
        mountDialog();
        await fireEvent.input(screen.getByTestId("projection-save-name"), {target: {value: "Series B"}});
        await fireEvent.change(screen.getByTestId("projection-save-dir"), {target: {value: "plans"}});
        expect(screen.getByTestId("projection-save-path").textContent).toContain("plans/projection-series-b.journal");
    });

    it("refuses a name with no file name in it, at the button", async () => {
        const {onSave} = mountDialog();
        await fireEvent.input(screen.getByTestId("projection-save-name"), {target: {value: "!!!"}});
        const submit = screen.getByTestId("projection-save-submit") as HTMLButtonElement;
        expect(submit.disabled).toBe(true);
        expect(screen.getByTestId("projection-save-blocker").textContent).toContain("no letters or digits");
        await fireEvent.click(submit);
        expect(onSave).not.toHaveBeenCalled();
    });

    it("flags a name that is already taken before the round trip", async () => {
        // The real refusal is the engine's `O_EXCL`; this is the courtesy that
        // saves a request, and it says the same sentence.
        const {onSave} = mountDialog();
        await fireEvent.input(screen.getByTestId("projection-save-name"), {target: {value: "Series A"}});
        await fireEvent.change(screen.getByTestId("projection-save-dir"), {target: {value: "plans"}});
        expect(screen.getByTestId("projection-save-blocker").textContent).toContain("already exists");
        expect((screen.getByTestId("projection-save-submit") as HTMLButtonElement).disabled).toBe(true);
        expect(onSave).not.toHaveBeenCalled();
    });

    it("hands the id and the TRIMMED name to its caller", async () => {
        const {onSave} = mountDialog();
        await fireEvent.input(screen.getByTestId("projection-save-name"), {target: {value: "  Series B  "}});
        await fireEvent.click(screen.getByTestId("projection-save-submit"));
        expect(onSave).toHaveBeenCalledWith("projection-series-b.journal", "Series B");
    });

    it("shows the engine's own sentence when a save came back refused", () => {
        mountDialog({name: "Series B", error: 'a file already exists at "projection-series-b.journal".'});
        expect(screen.getByTestId("projection-save-error").textContent).toContain("already exists");
    });
});

function mountBar(over: Record<string, unknown> = {}) {
    const onOpen = vi.fn();
    const onSave = vi.fn();
    const onSaveAs = vi.fn();
    render(ScenarioFileBar, {
        files: FILES,
        currentId: null,
        canSave: false,
        dirty: false,
        editable: true,
        busy: false,
        truncated: false,
        onOpen,
        onSave,
        onSaveAs,
        ...over,
    });
    return {onOpen, onSave, onSaveAs};
}

describe("COMPONENT ScenarioFileBar", () => {
    it("groups the projections above the other journals", () => {
        mountBar();
        const groups = Array.from(document.querySelectorAll("optgroup")).map((group) => group.label);
        expect(groups).toEqual(["Projections", "Other journals"]);
        // A `; projection:` name wins over the filename; a file without one
        // shows its filename label.
        expect(screen.getByRole("option", {name: "Series A"})).toBeTruthy();
        expect(screen.getByRole("option", {name: "budget"})).toBeTruthy();
    });

    it("opens a file straight away when there is no unsaved work", async () => {
        const {onOpen} = mountBar();
        await fireEvent.change(screen.getByTestId("projection-file-select"), {target: {value: "budget.journal"}});
        expect(onOpen).toHaveBeenCalledWith("budget.journal");
        expect(screen.queryByTestId("projection-discard")).toBeNull();
    });

    it("asks before discarding unsaved work, and only opens on Discard", async () => {
        const {onOpen} = mountBar({dirty: true});
        await fireEvent.change(screen.getByTestId("projection-file-select"), {target: {value: "budget.journal"}});
        expect(onOpen).not.toHaveBeenCalled();
        await fireEvent.click(screen.getByTestId("projection-discard-confirm"));
        expect(onOpen).toHaveBeenCalledWith("budget.journal");
    });

    it("keeps editing when the confirm is declined", async () => {
        const {onOpen} = mountBar({dirty: true});
        await fireEvent.change(screen.getByTestId("projection-file-select"), {target: {value: "budget.journal"}});
        await fireEvent.click(screen.getByTestId("projection-discard-cancel"));
        expect(onOpen).not.toHaveBeenCalled();
        expect(screen.queryByTestId("projection-discard")).toBeNull();
    });

    it("disables Save with no file of its own, and says to use Save as", () => {
        mountBar();
        expect((screen.getByTestId("projection-save") as HTMLButtonElement).disabled).toBe(true);
        expect((screen.getByTestId("projection-save-as") as HTMLButtonElement).disabled).toBe(false);
    });

    it("explains a read-only file rather than letting the save be refused", () => {
        // DECISION 5: the Projections tab never writes over a plan of record.
        mountBar({currentId: "budget.journal", canSave: false});
        expect(screen.getByTestId("projection-read-only").textContent).toContain("part of your main journal");
        expect((screen.getByTestId("projection-save") as HTMLButtonElement).disabled).toBe(true);
    });

    it("disables every write on a server with no editor bound", () => {
        mountBar({currentId: "plans/projection-series-a.journal", canSave: true, editable: false});
        expect((screen.getByTestId("projection-save") as HTMLButtonElement).disabled).toBe(true);
        expect((screen.getByTestId("projection-save-as") as HTMLButtonElement).disabled).toBe(true);
    });
});
