// The keymap's decision layer. Everything here is pure, so it runs in the fast
// `unit` project with no DOM — which is why `resolveBindings`/`handleKey` were
// written to take plain data rather than read the store directly.

import {describe, expect, it} from "vitest";
import {handleKey, helpSections, resolveBindings, type Dispatch} from "./dispatch";
import {PRIORITY, type Binding, type KeyGroup, type Layer, type RegisteredLayer} from "./types";

function binding(keys: string, label = keys, extra: Partial<Binding> = {}): Binding {
    return {keys, label, group: "Journal" as KeyGroup, run: () => {}, ...extra};
}

/** Register in the order given, stamping the `seq` the store would. */
function stack(...layers: Layer[]): RegisteredLayer[] {
    return layers.map((layer, at) => ({...layer, seq: at + 1}));
}

function press(key: string, mods: Partial<{ctrlKey: boolean; altKey: boolean; metaKey: boolean; shiftKey: boolean}> = {}) {
    return {key, ctrlKey: false, altKey: false, metaKey: false, shiftKey: false, ...mods};
}

describe("UNIT resolveBindings", () => {
    it("lets a higher-priority layer shadow a duplicate key", () => {
        const layers = stack(
            {id: "table", priority: PRIORITY.widget, bindings: [binding("j", "table down")]},
            {id: "tree", priority: PRIORITY.transient, bindings: [binding("j", "tree down")]}
        );

        const active = resolveBindings(layers);

        expect(active.filter((b) => b.keys === "j")).toHaveLength(1);
        expect(active.find((b) => b.keys === "j")?.label).toBe("tree down");
    });

    it("breaks a priority tie on registration order, later wins", () => {
        const layers = stack(
            {id: "first", priority: PRIORITY.widget, bindings: [binding("j", "first")]},
            {id: "second", priority: PRIORITY.widget, bindings: [binding("j", "second")]}
        );

        expect(resolveBindings(layers).find((b) => b.keys === "j")?.label).toBe("second");
    });

    it("blinds every layer below a modal one, INCLUDING for keys the modal does not bind", () => {
        // The point of `modal`: an overlay owning the screen must not let `j`
        // scroll the table behind it just because the overlay has no `j`.
        const layers = stack(
            {id: "table", priority: PRIORITY.widget, bindings: [binding("j"), binding("x")]},
            {id: "help", priority: PRIORITY.overlay, modal: true, bindings: [binding("?")]}
        );

        const active = resolveBindings(layers);

        expect(active.map((b) => b.keys)).toEqual(["?"]);
    });

    it("skips a disabled binding so a lower layer's same key wins instead", () => {
        const layers = stack(
            {id: "page", priority: PRIORITY.page, bindings: [binding("g i", "page fallback")]},
            {id: "widget", priority: PRIORITY.widget, bindings: [binding("g i", "disabled", {enabled: () => false})]}
        );

        expect(resolveBindings(layers).find((b) => b.keys === "g i")?.label).toBe("page fallback");
    });
});

describe("UNIT handleKey", () => {
    const active = resolveBindings(
        stack({
            id: "test",
            bindings: [binding("j", "down"), binding("G", "bottom"), binding("g g", "top"), binding("g j", "journal"), binding("ctrl+d", "half down")],
        })
    );

    it("runs an exact single-key match", () => {
        const decision = handleKey(active, "", press("j"));

        expect(decision).toMatchObject({kind: "run", binding: {label: "down"}});
    });

    it("arms a prefix, then completes it", () => {
        const armed = handleKey(active, "", press("g"));
        expect(armed).toEqual<Dispatch>({kind: "pending", sequence: "g"});

        expect(handleKey(active, "g", press("j"))).toMatchObject({kind: "run", binding: {label: "journal"}});
    });

    it("clears without running when an armed prefix goes nowhere", () => {
        expect(handleKey(active, "g", press("x"))).toEqual<Dispatch>({kind: "clear"});
    });

    it("ignores an unbound key when nothing is armed", () => {
        expect(handleKey(active, "", press("q"))).toEqual<Dispatch>({kind: "ignore"});
    });

    it("prefers the completed sequence over arming a longer one", () => {
        // `g g` is a complete binding AND `g` is a live prefix. Exact must win,
        // or "top" would be unreachable.
        expect(handleKey(active, "g", press("g"))).toMatchObject({kind: "run", binding: {label: "top"}});
    });

    it("does NOT arm a prefix whose only owner is disabled", () => {
        // Otherwise `g` swallows the next keystroke on behalf of a binding that
        // cannot run — the worst kind of dead key, because nothing is on screen.
        const off = resolveBindings(stack({id: "off", bindings: [binding("g i", "imports", {enabled: () => false})]}));

        expect(handleKey(off, "", press("g"))).toEqual<Dispatch>({kind: "ignore"});
    });

    it("distinguishes G from g rather than reading shiftKey", () => {
        // `event.key` already carries the shifted character; a binding spelled
        // "shift+g" would never match. This is the classic keymap bug.
        expect(handleKey(active, "", press("G", {shiftKey: true}))).toMatchObject({kind: "run", binding: {label: "bottom"}});
    });

    it("matches a ctrl chord without matching the bare key", () => {
        expect(handleKey(active, "", press("d", {ctrlKey: true}))).toMatchObject({kind: "run", binding: {label: "half down"}});
        expect(handleKey(active, "", press("d"))).toEqual<Dispatch>({kind: "ignore"});
    });

    it("does not match a ctrl chord when Cmd or Alt is also held", () => {
        // So Cmd-D still bookmarks and Option-D still does whatever the OS wants.
        expect(handleKey(active, "", press("d", {ctrlKey: true, metaKey: true}))).toEqual<Dispatch>({kind: "ignore"});
        expect(handleKey(active, "", press("d", {ctrlKey: true, altKey: true}))).toEqual<Dispatch>({kind: "ignore"});
    });
});

describe("UNIT helpSections", () => {
    it("renders rows from the resolved list, in group order, with no empty groups", () => {
        const active = resolveBindings(
            stack({
                id: "test",
                bindings: [
                    {...binding("j", "Move down"), group: "Journal"},
                    {...binding("?", "Show shortcuts"), group: "Global"},
                ],
            })
        );

        expect(helpSections(active).map((s) => s.group)).toEqual(["Global", "Journal"]);
    });

    it("REGRESSION: the sheet and the dispatcher agree on every shadowed key", () => {
        // THE structural guarantee of this feature. `?` help is generated from
        // `resolveBindings`, and `handleKey` searches the same list — so the
        // sheet cannot advertise a binding that does not fire, or omit one that
        // does. If anyone gives `Binding` a `hidden` flag or gives the sheet its
        // own list, this test is what fails.
        const layers = stack(
            {id: "page", priority: PRIORITY.page, bindings: [binding("j", "page down"), binding("x", "page delete")]},
            {id: "widget", priority: PRIORITY.widget, bindings: [binding("j", "widget down")]}
        );
        const active = resolveBindings(layers);

        const advertised = helpSections(active).flatMap((section) => section.rows);
        for (const row of advertised) {
            const decision = handleKey(active, "", press(row.keys));
            expect(decision).toMatchObject({kind: "run", binding: {label: row.label}});
        }
        // And the shadowed binding is advertised exactly once, as the winner.
        expect(advertised.filter((row) => row.keys === "j").map((row) => row.label)).toEqual(["widget down"]);
    });
});
