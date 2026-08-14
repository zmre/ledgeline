// What the drop zone looks like in each of its two states.
//
// This is the cheap half of the FE bug that shipped: the screen opened with a
// spinner and "Reading the file…" in the drop zone before the user had dropped
// anything, and then — once a file finally landed — went back to "Drop a
// statement here" exactly when there WAS something to report. Backwards, both
// halves.
//
// The defect was not here. `DropTarget` renders `busy` correctly and always did;
// the caller passed it `stagedView === "loading"`, which is true at rest.
// `NewTransactionsPanel.svelte.test.ts` is where that is caught. This file's job
// is to keep that one honest: it pins both states, so "the panel shows no
// spinner" cannot quietly become true because the spinner stopped existing.

import {render, screen} from "@testing-library/svelte";
import {describe, expect, it} from "vitest";
import DropTarget from "./DropTarget.svelte";

const props = (busy: boolean) => ({formats: ["csv", "ofx"], busy, rejection: null, onFile: () => {}});

describe("COMPONENT DropTarget", () => {
    it("invites a drop when nothing is being read", () => {
        const {container} = render(DropTarget, {props: props(false)});

        expect(screen.getByRole("heading", {name: "Drop a statement here"})).toBeDefined();
        expect(screen.getByRole("button", {name: "Choose file…"})).toBeDefined();
        expect(screen.queryByLabelText("Reading the file")).toBeNull();
        expect(container.querySelector(".loading-spinner")).toBeNull();
    });

    it("says it is reading while a file is in flight", () => {
        // The non-vacuity half. Without it, "no spinner at rest" is satisfied by
        // a component that can never show one.
        const {container} = render(DropTarget, {props: props(true)});

        expect(screen.getByLabelText("Reading the file")).toBeDefined();
        expect(container.querySelector(".loading-spinner")).not.toBeNull();
        expect(screen.queryByRole("heading", {name: "Drop a statement here"})).toBeNull();
    });
});
