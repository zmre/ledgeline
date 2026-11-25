// The template code field (invoicer PLAN 4d, D2), mounted: CodeMirror loads
// lazily, then the API the page uses — `value` both ways, the problem line,
// Go to line and the caret, read only — and the keys CodeMirror's own
// commands answer (Tab, Shift+Tab, Enter, Escape then Tab). jsdom has no
// layout, so nothing here asserts geometry or scrolling; the keys are
// dispatched as CodeMirror reads them (keydown on its content).
import {fireEvent, render, screen, waitFor} from "@testing-library/svelte";
import {describe, expect, it} from "vitest";
import {flushSync} from "svelte";
import CodeEditor from "./CodeEditor.svelte";
import {html} from "@codemirror/lang-html";
import {highlightTree, tags as t} from "@lezer/highlight";
import {cssRanges, highlight, teraRanges} from "./codeView";

/** The editor's content element, once CodeMirror has loaded. */
async function content(name = "Template code"): Promise<HTMLElement> {
    await waitFor(() => expect(screen.getByTestId("code-editor").dataset.ready).toBe("true"));
    return screen.getByRole("textbox", {name});
}

/** The text as CodeMirror shows it, one string per line. */
const shown = (el: HTMLElement): string[] => [...el.querySelectorAll(".cm-line")].map((l) => l.textContent ?? "");

async function key(el: HTMLElement, k: string, opts: {shiftKey?: boolean} = {}): Promise<boolean> {
    const codes: Record<string, number> = {Tab: 9, Enter: 13, Escape: 27, ArrowLeft: 37};
    return fireEvent.keyDown(el, {key: k, keyCode: codes[k], ...opts});
}

describe("COMPONENT CodeEditor", () => {
    it("loads CodeMirror with the text, labelled and described, and marks the problem line", async () => {
        render(CodeEditor, {value: "a\nb\nc", label: "Template code", problemLine: 2});
        const el = await content();
        expect(el.classList.contains("cm-content")).toBe(true);
        expect(el.getAttribute("data-testid")).toBe("code-editor-text");
        expect(shown(el)).toEqual(["a", "b", "c"]);
        expect(document.getElementById(el.getAttribute("aria-describedby") ?? "")?.textContent).toMatch(/Press Esc, then Tab/);
        await waitFor(() => expect(screen.getByTestId("code-editor-problem-line").textContent).toBe("2"));
        expect(screen.getByTestId("code-editor").querySelectorAll(".cm-problem-line")).toHaveLength(1);
    });

    it("moves the problem marker and clears it with the prop", async () => {
        const {rerender} = render(CodeEditor, {value: "a\nb\nc", label: "Code", problemLine: 1});
        await content("Code");
        await waitFor(() => expect(screen.getByTestId("code-editor-problem-line").textContent).toBe("1"));
        await rerender({problemLine: 3});
        await waitFor(() => expect(screen.getByTestId("code-editor-problem-line").textContent).toBe("3"));
        await rerender({problemLine: null});
        await waitFor(() => expect(screen.queryByTestId("code-editor-problem-line")).toBeNull());
        // A line past the end marks nothing rather than failing.
        await rerender({problemLine: 99});
        expect(screen.queryByTestId("code-editor-problem-line")).toBeNull();
    });

    it("follows `value` from the page, keeps the caret where the text is unchanged, and goes to a line", async () => {
        const {component, rerender} = render(CodeEditor, {value: "<p>\nx\n</p>", label: "Code"});
        const el = await content("Code");
        component.focusAt(4);
        expect(component.caret()).toBe(4);
        await rerender({value: "<p>\nx\n</p>\n<hr>"});
        await waitFor(() => expect(shown(el)).toEqual(["<p>", "x", "</p>", "<hr>"]));
        expect(component.caret()).toBe(4);
        component.goToLine(3);
        expect(component.caret()).toBe("<p>\nx\n".length);
        // Past the end: the last line.
        component.goToLine(40);
        expect(component.caret()).toBe("<p>\nx\n</p>\n".length);
    });

    it("indents with Tab, outdents with Shift+Tab, keeps indentation on Enter, and binds the text back", async () => {
        let text = "<p>\nx\n</p>";
        const {component} = render(CodeEditor, {
            label: "Code",
            get value() {
                return text;
            },
            set value(v: string) {
                text = v;
            },
        });
        const el = await content("Code");
        component.focusAt(4);
        // Prevented: CodeMirror handled it, the browser doesn't move focus.
        expect(await key(el, "Tab")).toBe(false);
        flushSync();
        expect(text).toBe("<p>\n  x\n</p>");
        expect(await key(el, "Tab", {shiftKey: true})).toBe(false);
        expect(text).toBe("<p>\nx\n</p>");
        await key(el, "Tab");
        component.focusAt("<p>\n  x".length);
        await key(el, "Enter");
        expect(text).toBe("<p>\n  x\n  \n</p>");
        expect(component.caret()).toBe("<p>\n  x\n  ".length);
    });

    it("lets Tab move on after Escape, so focus is never trapped", async () => {
        const {component} = render(CodeEditor, {value: "x", label: "Code"});
        const el = await content("Code");
        component.focusAt(1);
        await key(el, "Escape");
        // Not prevented: the browser moves focus.
        expect(await key(el, "Tab")).toBe(true);
        expect(shown(el)).toEqual(["x"]);
        // Any other key ends it (CodeMirror's own rule, which also lapses after
        // two seconds): Tab indents again.
        await key(el, "ArrowLeft");
        expect(await key(el, "Tab")).toBe(false);
        expect(screen.getByText(/Press Esc, then Tab/)).toBeDefined();
    });

    it("is read only when asked, and its keys change nothing", async () => {
        const {component, rerender} = render(CodeEditor, {value: "x", label: "Code", readonly: true});
        const el = await content("Code");
        expect(el.getAttribute("aria-readonly")).toBe("true");
        component.focusAt(0);
        await key(el, "Tab");
        expect(shown(el)).toEqual(["x"]);
        expect(screen.getByText(/^Read only\./)).toBeDefined();
        await rerender({readonly: false});
        await waitFor(() => expect(el.getAttribute("aria-readonly")).toBe("false"));
    });

    it("waits for the editor when asked to move the caret while it loads", async () => {
        const {component} = render(CodeEditor, {value: "a\nb", label: "Code"});
        expect(component.caret()).toBeNull();
        component.goToLine(2);
        await content("Code");
        expect(component.caret()).toBe(2);
    });
});

describe("UNIT teraRanges", () => {
    it("marks Tera's three delimiters, a tag's keyword, and runs an unclosed one to the end", () => {
        const text = "<h1>{{ invoice_num }}</h1>{% block theme %}{# note\nmore #}{% if x";
        const got = teraRanges(text).map((r) => [r.kind, text.slice(r.from, r.to)]);
        expect(got).toEqual([
            ["expr", "{{ invoice_num }}"],
            ["tag", "{% block theme %}"],
            ["keyword", "block"],
            ["comment", "{# note\nmore #}"],
            ["tag", "{% if x"],
            ["keyword", "if"],
        ]);
        expect(teraRanges("<p>plain</p>")).toEqual([]);
    });
});

describe("UNIT cssRanges", () => {
    const marks = (text: string): [string, string][] => cssRanges(text).map((r) => [r.kind, text.slice(r.from, r.to)]);

    it("colours a child's theme block as CSS, Tera inside it left to the Tera overlay", () => {
        const text = [
            '{% extends "_invoice_base.v1.html" %}',
            "{% block theme %}",
            "{{ super() }}",
            "@page { size: letter; margin: 0.7in; }",
            "/* the sheet */",
            '.sheet h1:first-child { color: #1d1d1b; font-family: "Iowan", serif; }',
            "@media (max-width: 640px) { .sheet { padding: 1.5rem 0; } }",
            "{% endblock theme %}",
            "<p>10px is text here</p>",
        ].join("\n");
        expect(marks(text)).toEqual([
            ["atrule", "@page"],
            ["property", "size"],
            ["property", "margin"],
            ["number", "0.7in"],
            ["comment", "/* the sheet */"],
            ["selector", ".sheet h1:first-child"],
            ["property", "color"],
            ["number", "#1d1d1b"],
            ["property", "font-family"],
            ["string", '"Iowan"'],
            ["atrule", "@media"],
            ["number", "640px"],
            ["selector", ".sheet"],
            ["property", "padding"],
            ["number", "1.5rem"],
            ["number", "0"],
        ]);
    });

    it("leaves a theme block inside <style> to lang-css, and text without one alone", () => {
        expect(cssRanges("<style>\n{% block theme %}a { color: red; }{% endblock theme %}\n</style>")).toEqual([]);
        expect(cssRanges("<p>a { color: red; }</p>")).toEqual([]);
        // Whitespace-trimming tags, and an unclosed block runs to the end.
        expect(marks("{%- block theme -%}\n.x { top: 1px")).toEqual([
            ["selector", ".x"],
            ["property", "top"],
            ["number", "1px"],
        ]);
    });
});

describe("UNIT lang-html's nested CSS", () => {
    /** The highlight classes over each token's text. */
    function classes(text: string): Map<string, string> {
        const tree = html({autoCloseTags: false}).language.parser.parse(text);
        const out = new Map<string, string>();
        highlightTree(tree, highlight, (from, to, cls) => out.set(text.slice(from, to), cls));
        return out;
    }
    const property = highlight.style([t.propertyName]);
    const tagName = highlight.style([t.tagName]);

    it("colours CSS in <style> and in style attributes, and HTML around them", () => {
        const got = classes('<style>.sheet { color: red; }</style>\n<p style="margin: 0">x</p>');
        expect(property).not.toBeNull();
        expect(got.get("color")).toBe(property);
        expect(got.get("margin")).toBe(property);
        expect(got.get("p")).toBe(tagName);
    });
});
