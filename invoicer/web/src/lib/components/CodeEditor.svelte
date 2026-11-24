<!-- A code field for a template's HTML and Tera (invoicer PLAN 4d): a plain
     textarea with line numbers, no wrapping, and an editor's keys. Not
     CodeMirror: that would be a new dependency (bun.lock), so this is the
     fallback until one is approved; the page sees only `value`.

     Keys: Tab indents (two spaces, or every selected line), Shift+Tab
     outdents, Enter keeps the line's indentation. Tab never traps focus:
     Escape, then Tab, moves on (the hint under the field says so), as in
     CodeMirror. Typing uses the browser's own insert where it has one, so
     undo (Cmd/Ctrl+Z) keeps working. Cmd/Ctrl+S is left to the page.

     `problemLine` marks a line number (Tera's error); `goToLine` and
     `focusAt` put the caret somewhere from outside. -->
<script lang="ts">
    let {
        value = $bindable(""),
        label,
        readonly = false,
        problemLine = null,
        testid = "code-editor",
    }: {
        value?: string;
        /** The field's accessible name. */
        label: string;
        readonly?: boolean;
        /** A line to mark (1-based), or null. */
        problemLine?: number | null;
        testid?: string;
    } = $props();

    const INDENT = "  ";
    const uid = $props.id();

    let area = $state<HTMLTextAreaElement | null>(null);
    let gutter = $state<HTMLDivElement | null>(null);
    /** Escape was pressed: the next Tab leaves the field. */
    let tabLeaves = $state(false);

    const lineCount = $derived(value.split("\n").length);
    const numbers = $derived(Array.from({length: lineCount}, (_, i) => i + 1));

    function syncScroll(): void {
        if (area !== null && gutter !== null) gutter.scrollTop = area.scrollTop;
    }

    /** Replace the selection `[start, end)` with `text`, through the browser's undo stack where it can. */
    function replace(start: number, end: number, text: string, caret: number = start + text.length, caretEnd: number = caret): void {
        if (area === null) return;
        area.setSelectionRange(start, end);
        // execCommand keeps undo working; jsdom and some browsers lack it.
        const done = typeof document.execCommand === "function" && document.execCommand("insertText", false, text);
        if (!done) {
            area.setRangeText(text, start, end, "end");
            value = area.value;
            area.dispatchEvent(new Event("input", {bubbles: true}));
        }
        area.setSelectionRange(caret, caretEnd);
    }

    /** The start of the line holding `at`. */
    const lineStart = (text: string, at: number): number => text.lastIndexOf("\n", at - 1) + 1;

    function indent(outdent: boolean): void {
        if (area === null) return;
        const text = area.value;
        const {selectionStart: start, selectionEnd: end} = area;
        const multiLine = text.slice(start, end).includes("\n");
        if (!outdent && !multiLine) {
            replace(start, end, INDENT);
            return;
        }
        const from = lineStart(text, start);
        const lines = text.slice(from, end).split("\n");
        let removedFirst = 0;
        const changed = lines.map((line, i) => {
            if (!outdent) return INDENT + line;
            const cut = line.startsWith(INDENT) ? INDENT.length : line.startsWith(" ") || line.startsWith("\t") ? 1 : 0;
            if (i === 0) removedFirst = cut;
            return line.slice(cut);
        });
        const out = changed.join("\n");
        const shift = outdent ? -removedFirst : INDENT.length;
        const newStart = Math.max(from, start + shift);
        replace(from, end, out, newStart, from + out.length);
    }

    function newline(): void {
        if (area === null) return;
        const text = area.value;
        const {selectionStart: start, selectionEnd: end} = area;
        const from = lineStart(text, start);
        const lead = /^[ \t]*/.exec(text.slice(from, start))?.[0] ?? "";
        replace(start, end, `\n${lead}`);
    }

    function onKeydown(event: KeyboardEvent): void {
        if (readonly || event.metaKey || event.ctrlKey || event.altKey) return;
        if (event.key === "Escape") {
            tabLeaves = true;
            return;
        }
        if (event.key === "Tab") {
            if (tabLeaves) {
                tabLeaves = false;
                return; // the browser moves focus
            }
            event.preventDefault();
            indent(event.shiftKey);
            return;
        }
        tabLeaves = false;
        if (event.key === "Enter" && !event.shiftKey) {
            event.preventDefault();
            newline();
        }
    }

    /** Put the caret at `offset` and show it. */
    export function focusAt(offset: number): void {
        if (area === null) return;
        area.focus();
        area.setSelectionRange(offset, offset);
        const line = value.slice(0, offset).split("\n").length;
        scrollToLine(line);
    }

    /** Put the caret at the start of `line` (1-based) and show it. */
    export function goToLine(line: number): void {
        const lines = value.split("\n");
        const at = lines.slice(0, Math.max(0, Math.min(line, lines.length) - 1)).reduce((n, l) => n + l.length + 1, 0);
        focusAt(at);
    }

    /** The caret's offset, or null when the field has never had focus. */
    export function caret(): number | null {
        return area === null ? null : area.selectionStart;
    }

    function scrollToLine(line: number): void {
        if (area === null) return;
        const height = parseFloat(getComputedStyle(area).lineHeight) || 24;
        const top = (line - 1) * height;
        if (top < area.scrollTop || top > area.scrollTop + area.clientHeight - height * 2) {
            area.scrollTop = Math.max(0, top - area.clientHeight / 3);
            syncScroll();
        }
    }
</script>

<div class="flex flex-col gap-1">
    <div
        class="flex h-[32rem] max-h-[70vh] min-h-64 overflow-hidden rounded-box border border-base-300 bg-base-100 font-mono text-sm leading-6 focus-within:outline-2 focus-within:outline-primary"
        data-testid={testid}
    >
        <div
            bind:this={gutter}
            class="shrink-0 overflow-hidden border-r border-base-300 bg-base-200 px-2 py-2 text-right text-base-content/50 tabular-nums select-none"
            aria-hidden="true"
        >
            {#each numbers as n (n)}
                <div
                    class={n === problemLine ? "rounded bg-error px-1 text-error-content" : "px-1"}
                    data-testid={n === problemLine ? `${testid}-problem-line` : undefined}
                >
                    {n}
                </div>
            {/each}
        </div>
        <textarea
            bind:this={area}
            bind:value
            id="{uid}-code"
            aria-label={label}
            aria-describedby="{uid}-hint"
            {readonly}
            wrap="off"
            spellcheck="false"
            autocapitalize="off"
            autocomplete="off"
            class="min-w-0 flex-1 resize-none overflow-auto bg-transparent px-3 py-2 whitespace-pre outline-none {readonly ? 'text-base-content/70' : ''}"
            onkeydown={onKeydown}
            onscroll={syncScroll}
            onfocus={() => (tabLeaves = false)}
            data-testid="{testid}-text"></textarea>
    </div>
    <p id="{uid}-hint" class="text-xs text-base-content/60">
        {#if readonly}
            Read only.
        {:else}
            Tab indents. Press Esc, then Tab, to move on.
        {/if}
    </p>
</div>
