<!-- A prose field (the markdown above or below the bill table): MBR's
     embeddable editor, `<mbr-markdown-field>` (MBR-CHANGES M6, in MBR 0.7.4),
     loaded from MBR's own assets once the session cookie is set; a plain
     textarea when it can't load (an older server, a test). The host owns
     saving: the field only reports what it holds.

     Tables (Phase 4 feedback H12): MBR's field has a "/" menu (Table is in
     it); Insert table adds an empty one either way (at the caret in the
     textarea, at the end in MBR's field, whose caret isn't ours to read);
     cells pasted from a spreadsheet (an HTML table, or tab-separated text)
     become a markdown table. In MBR's field the paste is handed back to it as
     that markdown, which it reads as a table. -->
<script lang="ts" module>
    /** Where MBR serves the field's module (behind the session cookie, PLAN A2). */
    export const MARKDOWN_FIELD_MODULE = "/.mbr/components/mbr-markdown-field.min.js";

    let loading: Promise<boolean> | null = null;

    /** Load MBR's field once; whether `<mbr-markdown-field>` is defined. */
    export function loadMarkdownField(load: () => Promise<unknown> = () => import(/* @vite-ignore */ MARKDOWN_FIELD_MODULE)): Promise<boolean> {
        if (typeof customElements === "undefined") return Promise.resolve(false);
        if (customElements.get("mbr-markdown-field") !== undefined) return Promise.resolve(true);
        loading ??= load().then(
            () => customElements.get("mbr-markdown-field") !== undefined,
            (error: unknown) => {
                console.warn("mbr-markdown-field: using a plain text field:", error);
                loading = null;
                return false;
            }
        );
        return loading;
    }
</script>

<script lang="ts">
    import {onMount, tick} from "svelte";
    import {ensureSession} from "#lib/api.js";
    import {EMPTY_TABLE, insertBlock, pastedTable} from "#lib/pasteTable.js";

    let {
        value,
        label,
        placeholder = "",
        readonly = false,
        linkBase = "/",
        onChange,
        testid,
    }: {
        value: string;
        label: string;
        placeholder?: string;
        readonly?: boolean;
        /** The note's folder URL, so relative links resolve as on its page. */
        linkBase?: string;
        onChange: (value: string) => void;
        testid?: string;
    } = $props();

    let rich = $state(false);
    let element: (HTMLElement & {value: string; readonly: boolean}) | null = null;
    let textarea = $state<HTMLTextAreaElement | null>(null);

    onMount(() => {
        let gone = false;
        ensureSession()
            .then(() => loadMarkdownField())
            .catch(() => false)
            .then((ok) => {
                if (!gone && ok) rich = true;
            });
        return () => {
            gone = true;
        };
    });

    /** Build MBR's element inside `node` (it is MBR's, not Svelte's: Svelte renders no children here). */
    function mountField(node: HTMLDivElement): {destroy(): void} {
        const field = document.createElement("mbr-markdown-field") as HTMLElement & {value: string; readonly: boolean};
        field.setAttribute("aria-label", label);
        field.setAttribute("placeholder", placeholder);
        field.setAttribute("catalog-url", "/.mbr/site.json");
        field.setAttribute("link-base", linkBase);
        field.value = value;
        field.readonly = readonly;
        field.addEventListener("change", (event) => {
            const detail = (event as CustomEvent<{value?: string}>).detail;
            onChange(detail?.value ?? field.value);
        });
        node.append(field);
        element = field;
        return {
            destroy() {
                element = null;
            },
        };
    }

    /** A spreadsheet paste into MBR's field: handed back to it as markdown, which its clipboard reads as a table. */
    function onRichPaste(event: ClipboardEvent): void {
        if (readonly || event.clipboardData === null) return;
        const table = pastedTable(event.clipboardData.getData("text/html"), event.clipboardData.getData("text/plain"));
        if (table === null) return;
        event.preventDefault();
        event.stopPropagation();
        const target = event.target as HTMLElement | null;
        try {
            const data = new DataTransfer();
            data.setData("text/plain", `${table}\n`);
            target?.dispatchEvent(new ClipboardEvent("paste", {clipboardData: data, bubbles: true, cancelable: true}));
        } catch {
            // No synthetic clipboard here: add it at the end instead.
            append(table);
        }
    }

    /** A spreadsheet paste into the textarea: the markdown table at the caret. */
    async function onTextPaste(event: ClipboardEvent): Promise<void> {
        if (readonly || event.clipboardData === null) return;
        const table = pastedTable(event.clipboardData.getData("text/html"), event.clipboardData.getData("text/plain"));
        if (table === null) return;
        event.preventDefault();
        const field = event.currentTarget as HTMLTextAreaElement;
        const at = field.selectionStart;
        const end = field.selectionEnd;
        const text = field.value.slice(0, at) + field.value.slice(end);
        const next = insertBlock(text, table, at);
        onChange(next.value);
        await tick();
        field.setSelectionRange(next.caret, next.caret);
    }

    function append(block: string): void {
        const next = insertBlock(value, block, null);
        if (element !== null) element.value = next.value;
        onChange(next.value);
    }

    async function insertTable(): Promise<void> {
        if (!rich && textarea !== null) {
            const at = textarea.selectionStart;
            const next = insertBlock(value, EMPTY_TABLE, at);
            onChange(next.value);
            await tick();
            textarea.focus();
            textarea.setSelectionRange(next.caret, next.caret);
            return;
        }
        append(EMPTY_TABLE);
    }

    // A new value from outside (a reload) reaches the field; its own edits don't loop.
    $effect(() => {
        const next = value;
        if (element !== null && element.value !== next) element.value = next;
    });
    $effect(() => {
        if (element !== null) element.readonly = readonly;
    });
</script>

<div class="flex flex-col gap-1">
    {#if rich}
        <div use:mountField onpastecapture={onRichPaste} data-testid={testid} data-kind="mbr"></div>
    {:else}
        <textarea
            bind:this={textarea}
            class="textarea min-h-24 w-full font-mono text-sm"
            aria-label={label}
            {placeholder}
            {readonly}
            {value}
            oninput={(event) => onChange((event.currentTarget as HTMLTextAreaElement).value)}
            onpaste={(event) => void onTextPaste(event)}
            data-testid={testid}
            data-kind="text"></textarea>
    {/if}
    {#if !readonly}
        <div class="flex flex-wrap items-center gap-2 text-xs text-base-content/60">
            <button
                type="button"
                class="btn btn-ghost btn-xs"
                onclick={() => void insertTable()}
                data-testid={testid === undefined ? undefined : `${testid}-table`}>Insert table</button
            >
            <span>{rich ? "Type / for headings, lists and tables. " : ""}Cells pasted from a spreadsheet become a table.</span>
        </div>
    {/if}
</div>
