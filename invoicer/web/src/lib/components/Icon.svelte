<!-- The customer card's icons, inline (no icon dependency): 16px line
     glyphs in the current text colour, hidden from screen readers (the
     control beside or around one carries the words). -->
<script lang="ts" module>
    export type IconName = "mail" | "phone" | "external" | "plus" | "copy" | "check" | "paperclip" | "file" | "note" | "chevron-down";
</script>

<script lang="ts">
    let {name, class: className = "size-4"}: {name: IconName; class?: string} = $props();
</script>

<svg
    class="shrink-0 {className}"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    stroke-width="2"
    stroke-linecap="round"
    stroke-linejoin="round"
    aria-hidden="true"
    data-icon={name}
>
    {#if name === "mail"}
        <rect x="3" y="5" width="18" height="14" rx="2" /><path d="m3 7 9 6 9-6" />
    {:else if name === "phone"}
        <path
            d="M22 16.9v3a2 2 0 0 1-2.2 2 19.8 19.8 0 0 1-8.6-3.1 19.5 19.5 0 0 1-6-6A19.8 19.8 0 0 1 2.1 4.2 2 2 0 0 1 4.1 2h3a2 2 0 0 1 2 1.7c.1 1 .4 1.9.7 2.8a2 2 0 0 1-.5 2.1L8 9.9a16 16 0 0 0 6 6l1.3-1.3a2 2 0 0 1 2.1-.4c.9.3 1.8.6 2.8.7a2 2 0 0 1 1.7 2z"
        />
    {:else if name === "external"}
        <path d="M14 4h6v6" /><path d="M20 4 10 14" /><path d="M18 14v5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1h5" />
    {:else if name === "plus"}
        <path d="M12 5v14M5 12h14" />
    {:else if name === "copy"}
        <rect x="9" y="9" width="12" height="12" rx="2" /><path d="M5 15H4a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1h10a1 1 0 0 1 1 1v1" />
    {:else if name === "check"}
        <path d="m5 12 5 5L20 7" />
    {:else if name === "paperclip"}
        <path d="m21 11-8.6 8.6a5 5 0 0 1-7-7L14 4a3.5 3.5 0 0 1 5 5l-8.6 8.6a2 2 0 0 1-2.8-2.8L15.5 7" />
    {:else if name === "file"}
        <path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z" /><path d="M14 3v5h5" />
    {:else if name === "note"}
        <path d="M4 4h16v16H4z" /><path d="M8 9h8M8 13h8M8 17h5" />
    {:else if name === "chevron-down"}
        <path d="m6 9 6 6 6-6" />
    {/if}
</svg>
