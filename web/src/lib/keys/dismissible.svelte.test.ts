// `dismissible` exists because six overlays in this app each roll their own
// dismissal and NONE of them restores focus. The focus-restore and
// topmost-only tests are the two that justify the file: the first is the
// behaviour every overlay is missing, the second is what lets nested overlays
// work without `stopPropagation` (which this codebase never uses).
//
// jsdom has no layout engine, so nothing here asserts geometry — only which
// element holds focus and whether the dismiss callback ran.

import {render, screen} from "@testing-library/svelte";
import {describe, expect, it} from "vitest";
import DismissibleProbe from "$lib/testing/fixtures/DismissibleProbe.svelte";

function escape(): void {
    document.dispatchEvent(new KeyboardEvent("keydown", {key: "Escape"}));
}

describe("COMPONENT dismissible", () => {
    it("dismisses on Escape", () => {
        let dismissed = 0;
        render(DismissibleProbe, {props: {optionsOf: () => ({onDismiss: () => (dismissed += 1)})}});

        escape();

        expect(dismissed).toBe(1);
    });

    it("focuses the first focusable child when trapping", () => {
        render(DismissibleProbe, {props: {optionsOf: () => ({onDismiss: () => {}, trap: true})}});

        expect(document.activeElement).toBe(screen.getByText("first"));
    });

    it("restores focus to whatever opened it", () => {
        // The single highest-value line in the action. Today, closing the column
        // menu leaves focus on <body>, so the next keystroke goes nowhere useful.
        const opener = document.createElement("button");
        document.body.append(opener);
        opener.focus();

        const view = render(DismissibleProbe, {props: {optionsOf: () => ({onDismiss: () => {}, trap: true})}});
        expect(document.activeElement).not.toBe(opener);

        view.unmount();

        expect(document.activeElement).toBe(opener);
        opener.remove();
    });

    it("only the topmost instance responds to Escape", () => {
        // How nested overlays behave correctly with no `stopPropagation`:
        // Escape closes the dropdown, not the modal behind it.
        let outer = 0;
        let inner = 0;
        render(DismissibleProbe, {props: {optionsOf: () => ({onDismiss: () => (outer += 1)})}});
        render(DismissibleProbe, {props: {optionsOf: () => ({onDismiss: () => (inner += 1)})}});

        escape();

        expect({outer, inner}).toEqual({outer: 0, inner: 1});
    });

    it("hands Escape back to the layer below once the top one goes away", () => {
        let outer = 0;
        render(DismissibleProbe, {props: {optionsOf: () => ({onDismiss: () => (outer += 1)})}});
        const top = render(DismissibleProbe, {props: {optionsOf: () => ({onDismiss: () => {}})}});

        top.unmount();
        escape();

        expect(outer).toBe(1);
    });

    it("does not dismiss on a pointerdown outside unless asked to", () => {
        // Modals use a backdrop button; only dropdowns want outside-click.
        let dismissed = 0;
        render(DismissibleProbe, {props: {optionsOf: () => ({onDismiss: () => (dismissed += 1)})}});

        document.body.dispatchEvent(new PointerEvent("pointerdown", {bubbles: true}));

        expect(dismissed).toBe(0);
    });

    it("dismisses on an outside pointerdown when opted in", () => {
        let dismissed = 0;
        render(DismissibleProbe, {props: {optionsOf: () => ({onDismiss: () => (dismissed += 1), outside: true})}});

        document.body.dispatchEvent(new PointerEvent("pointerdown", {bubbles: true}));

        expect(dismissed).toBe(1);
    });

    it("does not dismiss on a pointerdown inside itself", () => {
        let dismissed = 0;
        render(DismissibleProbe, {props: {optionsOf: () => ({onDismiss: () => (dismissed += 1), outside: true})}});

        screen.getByText("first").dispatchEvent(new PointerEvent("pointerdown", {bubbles: true}));

        expect(dismissed).toBe(0);
    });
});
