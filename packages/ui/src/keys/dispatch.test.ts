// The keymap's decision layer. Everything here is pure, so it runs in the fast
// `unit` project with no DOM — which is why `resolveBindings`/`handleKey` were
// written to take plain data rather than read the store directly.

import {describe, expect, it} from "vitest";
import {fieldBindings, handleKey, helpSections, inScope, keysFor, resolveBindings, type Dispatch} from "./dispatch";
import {PRIORITY, type Binding, type Layer, type RegisteredLayer} from "./types";

function binding(keys: string, label = keys, extra: Partial<Binding> = {}): Binding {
    return {keys, label, group: "Journal", run: () => {}, ...extra};
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

    it("lets any page layer shadow the app layer, whichever registered last", () => {
        const resolved = resolveBindings(
            stack(
                {id: "page", priority: PRIORITY.page, bindings: [binding("n", "page new")]},
                {id: "app", priority: PRIORITY.app, bindings: [binding("n", "app new"), binding("?", "help")]}
            )
        );
        expect(resolved.filter((b) => b.keys === "n").map((b) => b.label)).toEqual(["page new"]);
        expect(resolved.map((b) => b.keys)).toContain("?");
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

        expect(helpSections(active, ["Global", "Journal"]).map((s) => s.group)).toEqual(["Global", "Journal"]);
    });

    it("puts a section the app's order leaves out after the named ones, never hiding it", () => {
        const active = resolveBindings(
            stack({
                id: "test",
                bindings: [
                    {...binding("x", "Stray"), group: "Unlisted"},
                    {...binding("j", "Move down"), group: "Journal"},
                    {...binding("?", "Show shortcuts"), group: "Global"},
                ],
            })
        );

        expect(helpSections(active, ["Global", "Journal"]).map((s) => s.group)).toEqual(["Global", "Journal", "Unlisted"]);
        // No order at all: first-resolved first.
        expect(helpSections(active).map((s) => s.group)).toEqual(["Unlisted", "Journal", "Global"]);
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

describe("UNIT always bindings (zoom, plan 39)", () => {
    it("still resolves an `always` binding from below a modal layer, and nothing else from there", () => {
        const layers = stack(
            {id: "global", priority: PRIORITY.page, bindings: [binding("?"), binding("mod+0", "zoom", {always: true})]},
            {id: "overlay", priority: PRIORITY.overlay, modal: true, bindings: [binding("j")]}
        );

        const active = resolveBindings(layers);

        expect(active.map((b) => b.label)).toEqual(["j", "zoom"]);
    });

    it("lets a modal layer still shadow an `always` key it binds itself", () => {
        const layers = stack(
            {id: "global", priority: PRIORITY.page, bindings: [binding("mod+0", "zoom", {always: true})]},
            {id: "overlay", priority: PRIORITY.overlay, modal: true, bindings: [binding("mod+0", "overlay's own")]}
        );

        expect(resolveBindings(layers).map((b) => b.label)).toEqual(["overlay's own"]);
    });

    it("keeps a `help: false` alias off the sheet while it still dispatches", () => {
        const active = resolveBindings(stack({id: "global", bindings: [binding("ctrl++", "Zoom in"), binding("ctrl+=", "Zoom in", {help: false})]}));

        expect(helpSections(active).flatMap((s) => s.rows.map((r) => r.keys))).toEqual(["ctrl++"]);
        expect(handleKey(active, "", press("=", {ctrlKey: true})).kind).toBe("run");
    });
});

describe("UNIT scoped and field bindings (invoicer Phase 5k)", () => {
    const inside = {id: "cell"};
    const outside = {id: "elsewhere"};
    const grid = {contains: (node: unknown) => node === inside};

    it("lets a `within` binding answer only keys aimed inside its element", () => {
        const active = resolveBindings(stack({id: "grid", bindings: [binding("alt+ArrowUp", "Move up", {within: () => grid}), binding("j")]}));

        expect(inScope(active, inside).map((b) => b.label)).toEqual(["Move up", "j"]);
        expect(inScope(active, outside).map((b) => b.label)).toEqual(["j"]);
        expect(inScope(active, null).map((b) => b.label)).toEqual(["j"]);
    });

    it("treats an unmounted scope as nowhere", () => {
        const active = resolveBindings(stack({id: "grid", bindings: [binding("alt+ArrowUp", "Move up", {within: () => null})]}));

        expect(inScope(active, inside)).toEqual([]);
    });

    it("still lists a scoped binding on the sheet, which has focus when open", () => {
        const active = resolveBindings(stack({id: "grid", bindings: [binding("alt+ArrowUp", "Move up", {within: () => grid})]}));

        expect(helpSections(active).flatMap((s) => s.rows.map((r) => r.label))).toEqual(["Move up"]);
    });

    it("keeps `fields` and `always` bindings for a field, and nothing else", () => {
        const active = resolveBindings(
            stack({id: "page", bindings: [binding("j"), binding("mod+s", "Save", {always: true}), binding("alt+I", "Add an item", {fields: true})]})
        );

        expect(fieldBindings(active).map((b) => b.label)).toEqual(["Save", "Add an item"]);
    });

    it("blinds a `fields` binding under a modal layer, where an `always` one still reaches", () => {
        const layers = stack(
            {id: "editor", bindings: [binding("mod+s", "Save", {always: true}), binding("alt+I", "Add an item", {fields: true})]},
            {id: "dialog", priority: PRIORITY.overlay, modal: true, bindings: []}
        );

        expect(resolveBindings(layers).map((b) => b.label)).toEqual(["Save"]);
    });
});

// Plan 46 (K, T9): a binding may spell itself differently on macOS. The
// keymap resolves that once, so the dispatcher, shadowing and the `?` sheet
// only ever see the current platform's keys.
describe("UNIT per-platform bindings (macKeys)", () => {
    const slot = (extra: Partial<Binding> = {}) => binding("ctrl+shift+1", "Toggle slot 1", {macKeys: "meta+alt+1", ...extra});

    it("keysFor picks macKeys on macOS and keys elsewhere", () => {
        expect(keysFor(slot(), true)).toBe("meta+alt+1");
        expect(keysFor(slot(), false)).toBe("ctrl+shift+1");
        // Without macKeys, `keys` (with `mod+`) serves both.
        expect(keysFor(binding("mod+Enter"), true)).toBe("mod+Enter");
    });

    it("resolves to the platform's keys, so each platform's press runs it and the other's does not", () => {
        const layers = stack({id: "global", bindings: [slot()]});
        const mac = resolveBindings(layers, true);
        const other = resolveBindings(layers, false);

        expect(mac.map((b) => b.keys)).toEqual(["meta+alt+1"]);
        expect(other.map((b) => b.keys)).toEqual(["ctrl+shift+1"]);

        const optionCmd1 = {...press("¡", {altKey: true, metaKey: true}), code: "Digit1"};
        const ctrlShift1 = {...press("!", {ctrlKey: true, shiftKey: true}), code: "Digit1"};
        expect(handleKey(mac, "", optionCmd1).kind).toBe("run");
        expect(handleKey(mac, "", ctrlShift1).kind).toBe("ignore");
        expect(handleKey(other, "", ctrlShift1).kind).toBe("run");
        expect(handleKey(other, "", optionCmd1).kind).toBe("ignore");
    });

    it("shadows by the resolved keys: on macOS it does not collide with a ctrl+shift+1 below", () => {
        const layers = stack(
            {id: "page", priority: PRIORITY.page, bindings: [binding("ctrl+shift+1", "page")]},
            {id: "global", priority: PRIORITY.widget, bindings: [slot()]}
        );

        expect(resolveBindings(layers, true).map((b) => b.label)).toEqual(["Toggle slot 1", "page"]);
        expect(resolveBindings(layers, false).map((b) => b.label)).toEqual(["Toggle slot 1"]);
    });

    it("is not a field binding unless it says so, so Ctrl+Shift+arrows keep extending a selection", () => {
        const adjust = binding("ctrl+shift+ArrowDown", "Adjust down", {macKeys: "meta+alt+ArrowDown"});
        const active = resolveBindings(stack({id: "global", bindings: [adjust, slot()]}), false);
        const pressed = press("ArrowDown", {ctrlKey: true, shiftKey: true});

        expect(handleKey(active, "", pressed).kind).toBe("run");
        expect(fieldBindings(active)).toEqual([]);
        expect(handleKey(fieldBindings(active), "", pressed).kind).toBe("ignore");
        // Opting in with `fields` behaves as it does for a plain binding.
        const opted = resolveBindings(stack({id: "global", bindings: [{...adjust, fields: true}]}), false);
        expect(handleKey(fieldBindings(opted), "", pressed).kind).toBe("run");
    });

    it("renders the current platform's keys on the help sheet", () => {
        const layers = stack({id: "global", bindings: [slot(), binding("mod+Enter", "Save")]});
        const rows = (mac: boolean) => helpSections(resolveBindings(layers, mac), [], mac)[0].rows.map((row) => row.tokens.map((t) => t.text).join(" "));

        expect(rows(true)).toEqual(["⌥-⌘-1", "⌘-Enter"]);
        expect(rows(false)).toEqual(["Ctrl+Shift+1", "Ctrl+Enter"]);
    });
});
