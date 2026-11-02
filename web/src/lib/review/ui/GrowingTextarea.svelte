<script lang="ts">
    // A comment box that grows with its text (plan 32 D33, p7 amendment 17).
    //
    // A comment can span lines — the transaction's free text over `;`
    // continuation lines, a posting's comment over the lines under it — and a
    // text input strips line breaks from its value, so an edit there would
    // merge the lines or drop all but the first. This is a textarea: it starts
    // one row tall, grows to `maxRows`, and scrolls after that.
    //
    // Keys are the field's own: Enter is a newline. ⌘↵ still saves and Escape
    // still leaves the field, because the overlay catches both on its root
    // before the browser acts (a textarea is a typing target, so the keymap
    // never sees them).
    import type {HTMLTextareaAttributes} from "svelte/elements";

    type Props = Omit<HTMLTextareaAttributes, "value" | "rows" | "class"> & {
        value?: string;
        /** How tall it grows, in lines, before it scrolls. */
        maxRows?: number;
        size?: "xs" | "sm";
        class?: string;
    };

    let {value = $bindable(""), maxRows = 6, size = "sm", class: extra = "", ...rest}: Props = $props();

    // Spelled out, so Tailwind sees each class. One row is the matching input's
    // height: line + padding + a 1px border each side (20 + 10 + 2 = 2rem for sm).
    const SIZE = {xs: "textarea-xs min-h-6 py-[3px] leading-4", sm: "textarea-sm min-h-8 py-[5px] leading-5"} as const;

    let field = $state<HTMLTextAreaElement | null>(null);

    /** Fit the box to its text, up to `maxRows` lines. Skipped before layout (and under jsdom), where there is nothing to measure. */
    function fit(): void {
        if (field === null || field.scrollHeight === 0) return;
        const style = getComputedStyle(field);
        const line = Number.parseFloat(style.lineHeight) || 20;
        const frame = ["paddingTop", "paddingBottom", "borderTopWidth", "borderBottomWidth"] as const;
        const chrome = frame.reduce((sum, edge) => sum + (Number.parseFloat(style[edge]) || 0), 0);
        const borders = (Number.parseFloat(style.borderTopWidth) || 0) + (Number.parseFloat(style.borderBottomWidth) || 0);
        const max = line * maxRows + chrome;
        field.style.height = "auto";
        const wanted = field.scrollHeight + borders;
        field.style.height = `${Math.min(wanted, max)}px`;
        field.style.overflowY = wanted > max ? "auto" : "hidden";
    }

    $effect(() => {
        void value;
        fit();
    });
</script>

<textarea bind:this={field} bind:value rows={1} class={["textarea w-full resize-none", SIZE[size], extra]} {...rest}></textarea>
