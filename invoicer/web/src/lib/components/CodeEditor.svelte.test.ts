// The template code field (invoicer PLAN 4d), mounted: line numbers, the
// editor's keys, Esc-then-Tab to leave, and the problem line.
import {fireEvent, render, screen} from "@testing-library/svelte";
import {describe, expect, it} from "vitest";
import CodeEditor from "./CodeEditor.svelte";

const text = (): HTMLTextAreaElement => screen.getByTestId("code-editor-text") as HTMLTextAreaElement;

function at(area: HTMLTextAreaElement, start: number, end: number = start): void {
    area.focus();
    area.setSelectionRange(start, end);
}

describe("COMPONENT CodeEditor", () => {
    it("numbers its lines and marks the problem line", () => {
        render(CodeEditor, {value: "a\nb\nc", label: "Template code", problemLine: 2});
        expect(screen.getByRole("textbox", {name: "Template code"})).toBeDefined();
        const gutter = screen.getByTestId("code-editor").firstElementChild as HTMLElement;
        expect([...gutter.children].map((n) => n.textContent?.trim())).toEqual(["1", "2", "3"]);
        expect(screen.getByTestId("code-editor-problem-line").textContent?.trim()).toBe("2");
        expect(text().getAttribute("wrap")).toBe("off");
    });

    it("indents with Tab, outdents with Shift+Tab, keeps indentation on Enter", async () => {
        render(CodeEditor, {value: "<p>\nx\n</p>", label: "Code"});
        const area = text();
        at(area, 4);
        expect(await fireEvent.keyDown(area, {key: "Tab"})).toBe(false);
        expect(area.value).toBe("<p>\n  x\n</p>");
        // Several lines selected: each indented, then outdented.
        at(area, 0, area.value.length);
        await fireEvent.keyDown(area, {key: "Tab"});
        expect(area.value).toBe("  <p>\n    x\n  </p>");
        at(area, 0, area.value.length);
        await fireEvent.keyDown(area, {key: "Tab", shiftKey: true});
        expect(area.value).toBe("<p>\n  x\n</p>");
        // Enter after "  x" starts the next line at the same indent.
        at(area, "<p>\n  x".length);
        await fireEvent.keyDown(area, {key: "Enter"});
        expect(area.value).toBe("<p>\n  x\n  \n</p>");
        expect(area.selectionStart).toBe("<p>\n  x\n  ".length);
    });

    it("lets Tab move on after Escape, so focus is never trapped", async () => {
        render(CodeEditor, {value: "x", label: "Code"});
        const area = text();
        at(area, 1);
        await fireEvent.keyDown(area, {key: "Escape"});
        // Not prevented: the browser moves focus.
        expect(await fireEvent.keyDown(area, {key: "Tab"})).toBe(true);
        expect(area.value).toBe("x");
        // The next Tab indents again.
        expect(await fireEvent.keyDown(area, {key: "Tab"})).toBe(false);
        expect(screen.getByText(/Press Esc, then Tab/)).toBeDefined();
    });

    it("is read only when asked, and its keys do nothing", async () => {
        render(CodeEditor, {value: "x", label: "Code", readonly: true});
        const area = text();
        expect(area.readOnly).toBe(true);
        at(area, 0);
        expect(await fireEvent.keyDown(area, {key: "Tab"})).toBe(true);
        expect(area.value).toBe("x");
        expect(screen.getByText("Read only.")).toBeDefined();
    });
});
