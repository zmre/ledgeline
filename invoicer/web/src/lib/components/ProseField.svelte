<!-- A prose field (the markdown above or below the bill table): MBR's
     embeddable editor, `<mbr-markdown-field>` (MBR-CHANGES M6, in MBR 0.7.4),
     loaded from MBR's own assets once the session cookie is set; a plain
     textarea when it can't load (an older server, a test). The host owns
     saving: the field only reports what it holds. -->
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
    import {onMount} from "svelte";
    import {ensureSession} from "#lib/api.js";

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

    // A new value from outside (a reload) reaches the field; its own edits don't loop.
    $effect(() => {
        const next = value;
        if (element !== null && element.value !== next) element.value = next;
    });
    $effect(() => {
        if (element !== null) element.readonly = readonly;
    });
</script>

{#if rich}
    <div use:mountField data-testid={testid} data-kind="mbr"></div>
{:else}
    <textarea
        class="textarea min-h-24 w-full font-mono text-sm"
        aria-label={label}
        {placeholder}
        {readonly}
        {value}
        oninput={(event) => onChange((event.currentTarget as HTMLTextAreaElement).value)}
        data-testid={testid}
        data-kind="text"></textarea>
{/if}
