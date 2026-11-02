<script lang="ts" module>
    /** The review overlay's glyphs (plan 32 Phase 3): inline 16×16 SVGs, as everywhere in this app — there is no icon library. */
    export type ReviewIconName =
        | "check"
        | "arrow-right"
        | "arrow-left"
        | "x"
        | "ellipsis"
        | "chevron-right"
        | "chevron-down"
        | "sparkle"
        | "undo"
        | "ban"
        | "pencil"
        | "sliders"
        | "warning"
        | "bolt"
        | "calendar";
</script>

<script lang="ts">
    let {name, class: className = "h-4 w-4"}: {name: ReviewIconName; class?: string} = $props();
</script>

<!-- Stroked glyphs drawn on the 16px grid (1.75 stroke, round joins), so they
     sit at the same weight as the text beside them in a btn-sm. -->
<svg
    xmlns="http://www.w3.org/2000/svg"
    viewBox="0 0 16 16"
    fill="none"
    stroke="currentColor"
    stroke-width="1.75"
    stroke-linecap="round"
    stroke-linejoin="round"
    class={className}
    aria-hidden="true"
>
    {#if name === "check"}
        <path d="M3.5 8.5 6.5 11.5 12.5 4.5" />
    {:else if name === "arrow-right"}
        <path d="M3 8h10M9 4l4 4-4 4" />
    {:else if name === "arrow-left"}
        <path d="M13 8H3M7 4 3 8l4 4" />
    {:else if name === "x"}
        <path d="M4.5 4.5l7 7M11.5 4.5l-7 7" />
    {:else if name === "ellipsis"}
        <circle cx="3.5" cy="8" r="0.6" fill="currentColor" />
        <circle cx="8" cy="8" r="0.6" fill="currentColor" />
        <circle cx="12.5" cy="8" r="0.6" fill="currentColor" />
    {:else if name === "chevron-right"}
        <path d="M6 4l4 4-4 4" />
    {:else if name === "chevron-down"}
        <path d="M4 6l4 4 4-4" />
    {:else if name === "sparkle"}
        <path d="M8 2.5l1.2 3.6a1.2 1.2 0 0 0 .7.7L13.5 8l-3.6 1.2a1.2 1.2 0 0 0-.7.7L8 13.5l-1.2-3.6a1.2 1.2 0 0 0-.7-.7L2.5 8l3.6-1.2a1.2 1.2 0 0 0 .7-.7z" />
    {:else if name === "undo"}
        <path d="M5.5 3.5 2.5 6.5l3 3M2.5 6.5H10a3.5 3.5 0 0 1 0 7H7.5" />
    {:else if name === "ban"}
        <circle cx="8" cy="8" r="5.5" />
        <path d="M4.2 11.8l7.6-7.6" />
    {:else if name === "pencil"}
        <path d="M10.5 3l2.5 2.5-7 7L3 13l.5-3z" />
    {:else if name === "sliders"}
        <path d="M2.5 5h6M11.5 5h2M2.5 11h2M7.5 11h6" />
        <circle cx="10" cy="5" r="1.5" />
        <circle cx="6" cy="11" r="1.5" />
    {:else if name === "warning"}
        <path d="M8 2.5 14 13H2zM8 6.5v3M8 11.5v.01" />
    {:else if name === "bolt"}
        <path d="M9 2 3.5 9H8l-1 5 5.5-7H8z" />
    {:else if name === "calendar"}
        <rect x="2.5" y="3.5" width="11" height="10" rx="1.5" />
        <path d="M2.5 6.5h11M5.5 2v3M10.5 2v3" />
    {/if}
</svg>
