// The bridge's registries as components use them (plan 42): an overlay built
// on `use:dismissible` is closed by the app's back button with no code of its
// own, and an Export button is the page's "Export to Excel" menu action. Both
// only in the app: outside it, mounting registers nothing.

import {render, screen} from "@testing-library/svelte";
import {flushSync} from "svelte";
import {afterEach, beforeEach, describe, expect, it} from "vitest";
import ExportButton from "#lib/reports/ui/ExportButton.svelte";
import DismissibleProbe from "#lib/testing/fixtures/DismissibleProbe.svelte";
import {back, currentActions, openOverlays, resetNativeBridge} from "./bridge.svelte";
import {NATIVE_CLASS} from "./detect";

function setNative(on: boolean): void {
    document.documentElement.classList.toggle(NATIVE_CLASS, on);
}

beforeEach(() => resetNativeBridge());

afterEach(() => {
    setNative(false);
    resetNativeBridge();
});

describe("COMPONENT native overlays and actions", () => {
    it("closes a dismissible overlay on back(), and forgets it once unmounted", () => {
        setNative(true);
        let dismissed = 0;
        const {unmount} = render(DismissibleProbe, {props: {optionsOf: () => ({onDismiss: () => (dismissed += 1)})}});
        expect(screen.getByTestId("overlay")).toBeTruthy();
        expect(openOverlays()).toBe(1);

        expect(back()).toBe(true);
        expect(dismissed).toBe(1);

        unmount();
        expect(openOverlays()).toBe(0);
        expect(back()).toBe(false);
    });

    it("registers no overlay outside the app", () => {
        render(DismissibleProbe, {props: {optionsOf: () => ({onDismiss: () => {}})}});
        expect(openOverlays()).toBe(0);
    });

    it("offers an Export button as the export action, and runs its export", async () => {
        setNative(true);
        let exported = 0;
        const {unmount} = render(ExportButton, {
            props: {
                run: () => {
                    exported += 1;
                    return Promise.resolve();
                },
            },
        });
        flushSync();
        expect(currentActions().map(({id, label, icon}) => ({id, label, icon}))).toEqual([
            {id: "export", label: "Export to Excel", icon: "square.and.arrow.up"},
        ]);

        currentActions()[0].run();
        await Promise.resolve();
        expect(exported).toBe(1);

        unmount();
        expect(currentActions()).toEqual([]);
    });

    it("offers nothing outside the app", () => {
        render(ExportButton, {props: {run: () => Promise.resolve()}});
        flushSync();
        expect(currentActions()).toEqual([]);
    });
});
