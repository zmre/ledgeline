// The `?` sheet renders from the live registry, so these assertions are really
// about the anti-drift guarantee: what the sheet says is what the dispatcher
// would do. `dispatch.test.ts` proves that for the pure functions; this proves
// the component actually reads them rather than a hand-written list.

import {render, screen} from "@testing-library/svelte";
import {tick} from "svelte";
import {afterEach, describe, expect, it} from "vitest";
import KeysProbe from "$lib/testing/fixtures/KeysProbe.svelte";
import KeyHelp from "./KeyHelp.svelte";
import {keymap} from "./keymap.svelte";
import {PRIORITY, type Layer} from "./types";

afterEach(() => keymap.reset());

function layer(id: string, bindings: Layer["bindings"], extra: Partial<Layer> = {}): Layer {
    return {id, bindings, ...extra};
}

describe("COMPONENT KeyHelp", () => {
    it("renders nothing until it is opened", () => {
        render(KeyHelp);

        expect(screen.queryByTestId("key-help")).toBeNull();
    });

    it("lists a registered binding's label and its keys", async () => {
        render(KeysProbe, {
            props: {layerOf: () => layer("nav", [{keys: "g j", label: "Go to Journal", group: "Navigation", run: () => {}}])},
        });
        render(KeyHelp);

        keymap.openHelp();
        await tick();

        expect(screen.getByText("Go to Journal")).toBeDefined();
        // One <kbd> per chord STEP, so `g j` reads as two keys pressed in turn.
        expect(screen.getAllByText("g")).toHaveLength(1);
        expect(screen.getAllByText("j")).toHaveLength(1);
    });

    it("shows a shadowed key once, as the binding that would actually run", async () => {
        render(KeysProbe, {
            props: {layerOf: () => layer("page", [{keys: "j", label: "Page down", group: "Journal", run: () => {}}], {priority: PRIORITY.page})},
        });
        render(KeysProbe, {
            props: {
                layerOf: () => layer("widget", [{keys: "j", label: "Widget down", group: "Journal", run: () => {}}], {priority: PRIORITY.widget}),
            },
        });
        render(KeyHelp);

        keymap.openHelp();
        await tick();

        expect(screen.getByText("Widget down")).toBeDefined();
        expect(screen.queryByText("Page down")).toBeNull();
    });

    it("omits a binding that is currently disabled", async () => {
        // A sheet that advertises a key which does nothing is worse than no sheet.
        render(KeysProbe, {
            props: {
                layerOf: () => layer("nav", [{keys: "g i", label: "Go to Imports", group: "Navigation", run: () => {}, enabled: () => false}]),
            },
        });
        render(KeyHelp);

        keymap.openHelp();
        await tick();

        expect(screen.queryByText("Go to Imports")).toBeNull();
    });

    it("closes on Escape", async () => {
        render(KeyHelp);
        keymap.openHelp();
        await tick();

        document.dispatchEvent(new KeyboardEvent("keydown", {key: "Escape"}));
        await tick();

        expect(keymap.helpOpen).toBe(false);
    });

    it("closes on a backdrop click", async () => {
        render(KeyHelp);
        keymap.openHelp();
        await tick();

        screen.getByLabelText("Close").click();
        await tick();

        expect(keymap.helpOpen).toBe(false);
    });
});
