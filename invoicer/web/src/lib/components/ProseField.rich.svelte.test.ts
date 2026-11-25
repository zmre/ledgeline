// A prose field over MBR's field (a stand-in with its semantics: `value` is
// the live text, an edit is reported by `change` only after typing pauses,
// and setting `value` restarts the editor). The page re-rendering with the
// text it had (a live reload re-selecting the same snippet) must not push
// it back over typing the field hasn't reported yet (the library e2e's lost
// snippet text).
import {fireEvent, render, waitFor} from "@testing-library/svelte";
import {flushSync} from "svelte";
import {describe, expect, it, vi} from "vitest";
import ProseFieldHarness from "./ProseFieldHarness.svelte";

vi.mock("#lib/api.js", () => ({ensureSession: () => Promise.resolve()}));

class FakeMarkdownField extends HTMLElement {
    live = "";
    restarts = 0;
    get value(): string {
        return this.live;
    }
    set value(text: string) {
        if (text === this.live) return;
        this.live = text;
        this.restarts++;
    }
    readonly = false;
    /** Keys typed, not yet reported. */
    type(text: string): void {
        this.live += text;
    }
    report(): void {
        this.dispatchEvent(new CustomEvent("change", {detail: {value: this.live}}));
    }
}
customElements.define("mbr-markdown-field", FakeMarkdownField);

async function mounted() {
    const onChange = vi.fn();
    const {component, container} = render(ProseFieldHarness, {onChange});
    let field: FakeMarkdownField | null = null;
    await waitFor(() => {
        field = container.querySelector("mbr-markdown-field");
        expect(field).not.toBeNull();
    });
    return {onChange, component, field: field as unknown as FakeMarkdownField};
}

describe("COMPONENT ProseField over MBR's field", () => {
    it("keeps unreported typing when the page re-renders with the text it had", async () => {
        const {onChange, component, field} = await mounted();
        const restarts = field.restarts;
        field.type("A late fee");
        component.reassert();
        flushSync();
        expect(field.value).toBe("A late fee");
        expect(field.restarts).toBe(restarts);

        // Reported, then typed on before the page's echo lands: the echo isn't pushed back.
        field.report();
        expect(onChange).toHaveBeenLastCalledWith("A late fee");
        field.type(" applies");
        flushSync();
        component.reassert();
        flushSync();
        expect(field.value).toBe("A late fee applies");
        expect(field.restarts).toBe(restarts);
    });

    it("still takes a new text from outside (a change on disk, Discard)", async () => {
        const {component, field} = await mounted();
        field.type("typed");
        field.report();
        flushSync();
        component.set("theirs");
        flushSync();
        expect(field.value).toBe("theirs");
        component.set("");
        flushSync();
        expect(field.value).toBe("");
    });

    it("reports unreported typing on Cmd/Ctrl+S, before the page's save runs", async () => {
        const {onChange, field} = await mounted();
        let seenBySave: unknown = null;
        const save = (): void => {
            seenBySave = onChange.mock.lastCall?.[0];
        };
        window.addEventListener("keydown", save);
        try {
            field.type("Thanks!");
            await fireEvent.keyDown(field, {key: "s", metaKey: true});
            expect(seenBySave).toBe("Thanks!");
            onChange.mockClear();
            // Nothing new: no second report.
            await fireEvent.keyDown(field, {key: "s", ctrlKey: true});
            expect(onChange).not.toHaveBeenCalled();
            // Another key doesn't report.
            field.type(" more");
            await fireEvent.keyDown(field, {key: "a", metaKey: true});
            expect(onChange).not.toHaveBeenCalled();
        } finally {
            window.removeEventListener("keydown", save);
        }
    });
});
