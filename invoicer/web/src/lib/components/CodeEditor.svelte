<!-- A code field for a template's HTML and Tera (invoicer PLAN 4d, D2):
     CodeMirror 6, loaded on first use (./codeView.ts, its own chunk) so the
     rest of the app doesn't carry it. Line numbers, no wrapping, HTML and
     Tera highlighting, bracket matching, undo (Cmd/Ctrl+Z), search
     (Cmd/Ctrl+F), colours from the app's theme. The page sees only `value`.

     Keys: Tab indents (two spaces, or every selected line), Shift+Tab
     outdents, Enter keeps the indentation. Tab never traps focus: Escape,
     then Tab within two seconds, moves on (the hint under the field says
     so), and Ctrl+M (Shift+Alt+M on a Mac) makes Tab move focus until it is
     pressed again: CodeMirror's own rules. Cmd/Ctrl+S is left to the page.

     `problemLine` marks a line number (Tera's error); `goToLine` and
     `focusAt` put the caret somewhere from outside, and wait for the editor
     if it is still loading. -->
<script lang="ts">
    import {onMount} from "svelte";
    import type {CodeView} from "./codeView";

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

    const uid = $props.id();

    let host = $state<HTMLDivElement | null>(null);
    let view = $state<CodeView | null>(null);
    let failed = $state(false);
    /** A caret move asked for before the editor loaded. */
    let pending: ((v: CodeView) => void) | null = null;

    onMount(() => {
        let gone = false;
        let made: CodeView | null = null;
        import("./codeView")
            .then(({createCodeView}) => {
                if (gone || host === null) return;
                made = createCodeView(host, {
                    doc: value,
                    label,
                    describedBy: `${uid}-hint`,
                    readonly,
                    problemLine,
                    testid,
                    onChange: (text) => (value = text),
                });
                view = made;
                pending?.(made);
                pending = null;
            })
            .catch((error: unknown) => {
                console.error("The code editor didn't load", error);
                failed = true;
            });
        return () => {
            gone = true;
            made?.destroy();
        };
    });

    // From the page to the editor: a load, Discard, an inserted block.
    $effect(() => {
        const text = value;
        view?.setValue(text);
    });
    $effect(() => {
        const on = readonly;
        view?.setReadOnly(on);
    });
    $effect(() => {
        const line = problemLine;
        view?.setProblem(line);
    });

    function whenReady(run: (v: CodeView) => void): void {
        if (view !== null) run(view);
        else pending = run;
    }

    /** Put the caret at `offset` and show it. */
    export function focusAt(offset: number): void {
        whenReady((v) => v.focusAt(offset));
    }

    /** Put the caret at the start of `line` (1-based) and show it. */
    export function goToLine(line: number): void {
        const lines = value.split("\n");
        const at = lines.slice(0, Math.max(0, Math.min(line, lines.length) - 1)).reduce((n, l) => n + l.length + 1, 0);
        focusAt(at);
    }

    /** The caret's offset, or null while the editor is loading. */
    export function caret(): number | null {
        return view === null ? null : view.caret();
    }
</script>

<div class="flex flex-col gap-1">
    <div
        bind:this={host}
        class="flex h-[32rem] max-h-[70vh] min-h-64 overflow-hidden rounded-box border border-base-300 bg-base-100 font-mono text-sm leading-6 *:min-w-0 *:flex-1 focus-within:outline-2 focus-within:outline-primary"
        data-testid={testid}
        data-ready={view !== null}
    >
        {#if view === null}
            <p class="self-center p-4 text-center font-sans text-base-content/60">
                {failed ? "The code editor didn't load. Reload the page to try again." : "Loading the editor…"}
            </p>
        {/if}
    </div>
    <p id="{uid}-hint" class="text-xs text-base-content/60">
        {#if readonly}
            Read only. Cmd/Ctrl+F searches.
        {:else}
            Tab indents. Press Esc, then Tab, to move on. Cmd/Ctrl+F searches.
        {/if}
    </p>
</div>
