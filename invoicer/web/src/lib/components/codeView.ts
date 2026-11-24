// CodeMirror 6 behind CodeEditor.svelte (invoicer PLAN 4d, D2). Only
// CodeEditor imports this, and only through `import()`, so CodeMirror is its
// own chunk, fetched when a template is opened; nothing else in the bundle
// grows.
//
// HTML from @codemirror/lang-html (with its CSS and JavaScript inside
// <style> and <script>); Tera on top as a decoration overlay, not a parser:
// `{{ … }}`, `{% … %}` and `{# … #}` are found with one regular expression
// over the whole text (templates are a few kilobytes) and marked, the tag's
// keyword too. The HTML parser reads them as text, which it tolerates even
// inside a tag. No @codemirror/legacy-modes (its jinja2 mode would be a
// further package and still needs mixing with HTML).
//
// Colours are daisyUI's variables, so the editor follows the app's theme,
// light or dark, with no JavaScript when the theme changes.
import {defaultKeymap, history, historyKeymap, indentWithTab} from "@codemirror/commands";
import {html} from "@codemirror/lang-html";
import {bracketMatching, HighlightStyle, indentOnInput, indentUnit, syntaxHighlighting} from "@codemirror/language";
import {highlightSelectionMatches, search, searchKeymap} from "@codemirror/search";
import {Compartment, EditorState, RangeSetBuilder, StateEffect, StateField, type Extension} from "@codemirror/state";
import {
    Decoration,
    drawSelection,
    EditorView,
    GutterMarker,
    highlightActiveLine,
    highlightActiveLineGutter,
    highlightSpecialChars,
    keymap,
    lineNumberMarkers,
    lineNumbers,
    type DecorationSet,
} from "@codemirror/view";
import {tags as t} from "@lezer/highlight";

export interface CodeViewOptions {
    doc: string;
    /** The content's accessible name. */
    label: string;
    /** The id of the hint under the field (aria-describedby). */
    describedBy: string;
    readonly: boolean;
    problemLine: number | null;
    /** The test id prefix: `${testid}-text` on the content, `${testid}-problem-line` on the marked number. */
    testid: string;
    onChange: (text: string) => void;
}

/** What CodeEditor.svelte drives. */
export interface CodeView {
    /** Make the text `text`, changing only what differs (so the caret and undo survive). */
    setValue(text: string): void;
    setReadOnly(readonly: boolean): void;
    setProblem(line: number | null): void;
    /** Focus, put the caret at `offset` and scroll it into view. */
    focusAt(offset: number): void;
    caret(): number;
    destroy(): void;
}

// ---- Tera ------------------------------------------------------------------

/** `{{ … }}`, `{% … %}`, `{# … #}`, across lines; an unclosed one runs to the end. */
const TERA = /\{\{[\s\S]*?(?:\}\}|$)|\{%[\s\S]*?(?:%\}|$)|\{#[\s\S]*?(?:#\}|$)/g;
const TERA_KEYWORD = /^\{%-?\s*([a-z_]+)/;

const teraExpr = Decoration.mark({class: "cm-tera cm-tera-expr"});
const teraTag = Decoration.mark({class: "cm-tera cm-tera-tag"});
const teraComment = Decoration.mark({class: "cm-tera-comment"});
const teraKeyword = Decoration.mark({class: "cm-tera-keyword"});

/** The Tera marks over `text` (exported for the tests). */
export function teraRanges(text: string): {from: number; to: number; kind: "expr" | "tag" | "comment" | "keyword"}[] {
    const out: {from: number; to: number; kind: "expr" | "tag" | "comment" | "keyword"}[] = [];
    for (const m of text.matchAll(TERA)) {
        const from = m.index;
        const to = from + m[0].length;
        if (to === from) continue;
        const open = m[0][1];
        if (open === "#") {
            out.push({from, to, kind: "comment"});
            continue;
        }
        out.push({from, to, kind: open === "{" ? "expr" : "tag"});
        if (open === "%") {
            const kw = TERA_KEYWORD.exec(m[0]);
            if (kw !== null) {
                const at = from + m[0].indexOf(kw[1], 2);
                out.push({from: at, to: at + kw[1].length, kind: "keyword"});
            }
        }
    }
    return out;
}

function teraDecorations(state: EditorState): DecorationSet {
    const builder = new RangeSetBuilder<Decoration>();
    const marks = {expr: teraExpr, tag: teraTag, comment: teraComment, keyword: teraKeyword};
    // Already in document order: a keyword follows (and sits inside) its tag.
    for (const r of teraRanges(state.doc.toString())) builder.add(r.from, r.to, marks[r.kind]);
    return builder.finish();
}

const teraField = StateField.define<DecorationSet>({
    create: teraDecorations,
    update: (deco, tr) => (tr.docChanged ? teraDecorations(tr.state) : deco),
    provide: (f) => EditorView.decorations.from(f),
});

// ---- The problem line --------------------------------------------------------

const setProblemEffect = StateEffect.define<number | null>();
const problemField = StateField.define<number | null>({
    create: () => null,
    update(line, tr) {
        for (const e of tr.effects) if (e.is(setProblemEffect)) return e.value;
        return line;
    },
});

/** The marked line's number, in the gutter in place of the plain one. */
class ProblemNumber extends GutterMarker {
    constructor(
        readonly line: number,
        readonly testid: string
    ) {
        super();
    }
    override eq(other: GutterMarker): boolean {
        return other instanceof ProblemNumber && other.line === this.line;
    }
    override toDOM(): Node {
        const el = document.createElement("span");
        el.className = "cm-problem-number";
        el.dataset.testid = `${this.testid}-problem-line`;
        el.textContent = String(this.line);
        return el;
    }
}

const problemLineDeco = Decoration.line({class: "cm-problem-line"});

function problemExtension(testid: string): Extension {
    /** The marked line's start, or null when there is none (or it is past the end). */
    const start = (state: EditorState): {line: number; from: number} | null => {
        const line = state.field(problemField);
        if (line === null || line < 1 || line > state.doc.lines) return null;
        return {line, from: state.doc.line(line).from};
    };
    return [
        problemField,
        lineNumberMarkers.compute([problemField, "doc"], (state) => {
            const at = start(state);
            const builder = new RangeSetBuilder<GutterMarker>();
            if (at !== null) builder.add(at.from, at.from, new ProblemNumber(at.line, testid));
            return builder.finish();
        }),
        EditorView.decorations.compute([problemField, "doc"], (state) => {
            const at = start(state);
            return at === null ? Decoration.none : Decoration.set([problemLineDeco.range(at.from)]);
        }),
    ];
}

// ---- Theme -----------------------------------------------------------------

/** A daisyUI colour pulled toward the text colour, so it reads on light and dark bases alike. */
const ink = (name: string, pct = 70): string => `color-mix(in oklab, var(--color-${name}) ${pct}%, var(--color-base-content))`;
const faint = (name: string, pct: number): string => `color-mix(in oklab, var(--color-${name}) ${pct}%, transparent)`;

const theme = EditorView.theme({
    "&": {height: "100%", backgroundColor: "var(--color-base-100)", color: "var(--color-base-content)"},
    "&.cm-focused": {outline: "none"},
    // Monospace, size and line height from the wrapper (rem-based, so it scales with the UI).
    ".cm-scroller": {fontFamily: "inherit", lineHeight: "inherit", overflow: "auto"},
    ".cm-content": {padding: "0.5rem 0", caretColor: "var(--color-base-content)"},
    ".cm-line": {padding: "0 0.75rem"},
    ".cm-cursor, .cm-dropCursor": {borderLeftColor: "var(--color-base-content)"},
    "&.cm-focused > .cm-scroller > .cm-selectionLayer .cm-selectionBackground, .cm-selectionBackground, .cm-content ::selection": {
        backgroundColor: `${faint("primary", 30)} !important`,
    },
    ".cm-activeLine": {backgroundColor: faint("base-content", 5)},
    ".cm-gutters": {
        backgroundColor: "var(--color-base-200)",
        color: faint("base-content", 50),
        borderRight: "1px solid var(--color-base-300)",
    },
    ".cm-lineNumbers .cm-gutterElement": {padding: "0 0.5rem 0 0.75rem", fontVariantNumeric: "tabular-nums"},
    ".cm-activeLineGutter": {backgroundColor: faint("base-content", 8), color: "var(--color-base-content)"},
    ".cm-problem-line": {backgroundColor: faint("error", 15)},
    ".cm-problem-number": {
        backgroundColor: "var(--color-error)",
        color: "var(--color-error-content)",
        borderRadius: "0.25rem",
        padding: "0 0.25rem",
    },
    ".cm-matchingBracket, &.cm-focused .cm-matchingBracket": {backgroundColor: faint("success", 25), outline: `1px solid ${faint("success", 60)}`},
    ".cm-nonmatchingBracket, &.cm-focused .cm-nonmatchingBracket": {backgroundColor: faint("error", 25)},
    ".cm-searchMatch": {backgroundColor: faint("warning", 30), outline: `1px solid ${faint("warning", 70)}`},
    ".cm-searchMatch.cm-searchMatch-selected": {backgroundColor: faint("warning", 55)},
    ".cm-selectionMatch": {backgroundColor: faint("info", 20)},
    ".cm-panels": {backgroundColor: "var(--color-base-200)", color: "var(--color-base-content)"},
    ".cm-panels.cm-panels-top": {borderBottom: "1px solid var(--color-base-300)"},
    ".cm-panels.cm-panels-bottom": {borderTop: "1px solid var(--color-base-300)"},
    ".cm-panel.cm-search": {fontFamily: "var(--font-sans, sans-serif)", fontSize: "0.875rem", padding: "0.375rem 0.5rem"},
    ".cm-panel.cm-search input, .cm-panel.cm-search button": {fontSize: "inherit"},
    ".cm-textfield": {
        backgroundColor: "var(--color-base-100)",
        color: "var(--color-base-content)",
        border: "1px solid var(--color-base-300)",
        borderRadius: "0.25rem",
    },
    ".cm-button": {
        backgroundImage: "none",
        backgroundColor: "var(--color-base-300)",
        color: "var(--color-base-content)",
        border: "1px solid var(--color-base-300)",
        borderRadius: "0.25rem",
    },
    ".cm-panel.cm-search [name=close]": {color: "var(--color-base-content)"},
    // Tera: tinted, so the template language stands apart from the HTML. The
    // `*` beats the HTML highlighter's classes wherever the two nest.
    ".cm-tera, .cm-tera *": {color: ink("accent", 75)},
    ".cm-tera": {backgroundColor: faint("accent", 10), borderRadius: "0.2rem"},
    ".cm-tera-keyword, .cm-tera-keyword *": {fontWeight: "600"},
    ".cm-tera-comment, .cm-tera-comment *": {color: faint("base-content", 50), fontStyle: "italic"},
});

const highlight = HighlightStyle.define([
    {tag: [t.tagName, t.standard(t.tagName)], color: ink("primary")},
    {tag: [t.angleBracket, t.processingInstruction, t.documentMeta], color: faint("base-content", 55)},
    {tag: [t.attributeName, t.propertyName], color: ink("secondary")},
    {tag: [t.attributeValue, t.string, t.special(t.string)], color: ink("success", 60)},
    {tag: [t.number, t.unit, t.bool, t.atom, t.color], color: ink("warning", 60)},
    {tag: [t.keyword, t.modifier, t.operatorKeyword, t.controlKeyword], color: ink("info", 65)},
    {tag: [t.className, t.labelName, t.typeName], color: ink("info", 65)},
    {tag: [t.variableName, t.definition(t.variableName)], color: "var(--color-base-content)"},
    {tag: t.comment, color: faint("base-content", 50), fontStyle: "italic"},
    {tag: t.invalid, color: "var(--color-error)"},
]);

// ---- The view --------------------------------------------------------------

export function createCodeView(parent: HTMLElement, opts: CodeViewOptions): CodeView {
    const readOnly = new Compartment();
    const readOnlyExt = (on: boolean): Extension => [EditorState.readOnly.of(on), EditorView.contentAttributes.of({"aria-readonly": String(on)})];
    // Set while we apply `setValue`, so the change isn't echoed back.
    let applying = false;

    const view = new EditorView({
        parent,
        state: EditorState.create({
            doc: opts.doc,
            extensions: [
                lineNumbers(),
                highlightActiveLineGutter(),
                highlightSpecialChars(),
                history(),
                drawSelection(),
                indentOnInput(),
                indentUnit.of("  "),
                EditorState.tabSize.of(2),
                // No auto-closing tags: typing `>` inserts only `>`, as Tera's tags sit between HTML's.
                html({autoCloseTags: false}),
                syntaxHighlighting(highlight),
                bracketMatching(),
                highlightActiveLine(),
                highlightSelectionMatches(),
                search({top: true}),
                // Tab indents. CodeMirror's own escape hatch keeps focus free:
                // Escape, then Tab within two seconds, moves on; Ctrl+M
                // (Shift+Alt+M on a Mac) makes Tab move focus until pressed again.
                keymap.of([...defaultKeymap, ...historyKeymap, ...searchKeymap, indentWithTab]),
                teraField,
                problemExtension(opts.testid),
                readOnly.of(readOnlyExt(opts.readonly)),
                theme,
                EditorView.contentAttributes.of({
                    "aria-label": opts.label,
                    "aria-describedby": opts.describedBy,
                    "data-testid": `${opts.testid}-text`,
                    spellcheck: "false",
                    autocapitalize: "off",
                    autocorrect: "off",
                }),
                EditorView.updateListener.of((update) => {
                    if (update.docChanged && !applying) opts.onChange(update.state.doc.toString());
                }),
            ],
        }),
    });
    view.dispatch({effects: setProblemEffect.of(opts.problemLine)});

    return {
        setValue(text) {
            const current = view.state.doc.toString();
            if (text === current) return;
            // Only the span that differs, so the caret, the scroll and undo stay put.
            let start = 0;
            const max = Math.min(text.length, current.length);
            while (start < max && text.charCodeAt(start) === current.charCodeAt(start)) start++;
            let end = 0;
            while (end < max - start && text.charCodeAt(text.length - 1 - end) === current.charCodeAt(current.length - 1 - end)) end++;
            applying = true;
            try {
                view.dispatch({changes: {from: start, to: current.length - end, insert: text.slice(start, text.length - end)}});
            } finally {
                applying = false;
            }
        },
        setReadOnly(on) {
            view.dispatch({effects: readOnly.reconfigure(readOnlyExt(on))});
        },
        setProblem(line) {
            if (view.state.field(problemField) !== line) view.dispatch({effects: setProblemEffect.of(line)});
        },
        focusAt(offset) {
            const at = Math.max(0, Math.min(offset, view.state.doc.length));
            view.focus();
            view.dispatch({selection: {anchor: at}, effects: EditorView.scrollIntoView(at, {y: "center"})});
        },
        caret: () => view.state.selection.main.head,
        destroy: () => view.destroy(),
    };
}
