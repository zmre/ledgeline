<script lang="ts" module>
    import {datePattern} from "#lib/format/datePattern.js";

    /** The runtime's locale, once: it does not change under a running page. */
    const HINT = datePattern();
</script>

<script lang="ts">
    // A native `<input type="date">` for a date that may be UNSET.
    //
    // # The bug
    //
    // WebKit — the desktop app's webview — paints TODAY's date, in blue, into an
    // empty date input. An open-ended projection bound read as "ends today", and
    // an unfiltered range as "today to today". Chromium shows `mm/dd/yyyy`
    // instead, which is why the e2e suite never saw it.
    //
    // # The fix
    //
    // While the box is empty and unfocused, the native text is hidden and a grey
    // hint in the locale's own order (`m/d/y`, `d.m.y`, `y-m-d`…) sits in its
    // place. Focusing hands the field back to the
    // browser untouched. The text is hidden by OPACITY: WebKit colours the
    // month/day/year fields itself, so a `color` on the wrapper does not reach
    // them.
    //
    // Emptiness is read from the BOX: `bind:value` writes what the user types
    // back into the local `value` even when the caller passes it one-way. A
    // caller may hold its typed text back before committing it (the filter bar
    // debounces), and the hint must not reappear over a date the user can see.
    import type {HTMLInputAttributes} from "svelte/elements";

    type Props = Omit<HTMLInputAttributes, "type" | "value" | "size" | "class"> & {
        value?: string | null;
        size?: "xs" | "sm";
        /** Width and layout classes for the input; sizing comes from `size`. */
        class?: string;
    };

    let {value = $bindable(""), size = "sm", class: extra = "", ...rest}: Props = $props();

    // Spelled out, not interpolated, so Tailwind sees both class names.
    const SIZE = {xs: {input: "input-xs", hint: "left-2"}, sm: {input: "input-sm", hint: "left-3"}} as const;

    const empty = $derived(value === null || value === "");
</script>

<div class="relative">
    <input {...rest} type="date" class={["peer input", SIZE[size].input, extra, empty && "not-focus:[&::-webkit-datetime-edit]:opacity-0"]} bind:value />
    {#if empty}
        <span
            class={["pointer-events-none absolute inset-y-0 flex items-center text-xs text-base-content/30 peer-focus:hidden", SIZE[size].hint]}
            aria-hidden="true"
            data-testid="date-unset">{HINT}</span
        >
    {/if}
</div>
