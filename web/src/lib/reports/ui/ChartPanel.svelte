<!-- A collapsible chart above a report table, in the house style: a daisyUI
     `collapse collapse-arrow` driven by a checkbox whose state the CALLER
     persists. The header — `ChartHeading`, plus an optional `aside` at its
     right (the Sankey panels' total while shut) — is always rendered and
     always operable; only the content collapses, and is not rendered at all
     while shut. The table below is the chart's accessible twin, so shutting
     the panel loses nothing. -->
<script lang="ts">
    import type {Snippet} from "svelte";
    import ChartHeading from "$lib/components/ChartHeading.svelte";

    let {
        heading,
        note,
        open,
        onToggle,
        testid,
        aside,
        children,
    }: {
        heading: string;
        note?: string;
        open: boolean;
        onToggle: (open: boolean) => void;
        testid: string;
        /** Rendered at the header's right edge, beside the arrow. */
        aside?: Snippet;
        children: Snippet;
    } = $props();
</script>

<section class="collapse-arrow collapse bg-base-200" data-testid={testid}>
    <input type="checkbox" checked={open} onchange={(e) => onToggle(e.currentTarget.checked)} aria-label="Toggle {heading}" />
    <div class="collapse-title flex min-h-0 items-center justify-between gap-2 py-3 pr-10">
        <ChartHeading {heading} {note} />
        {#if aside !== undefined}{@render aside()}{/if}
    </div>
    <div class="collapse-content flex flex-col gap-2">
        {#if open}
            {@render children()}
        {/if}
    </div>
</section>
