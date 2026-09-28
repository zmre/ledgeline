<!-- A collapsible chart above a report table, in the house style: a daisyUI
     `collapse collapse-arrow` driven by a checkbox whose state the CALLER
     persists (the P&L's Sankey panels do the same). The header — heading and
     "· note" in the chart-heading style — is always rendered and always
     operable; only the content collapses. The table below is the chart's
     accessible twin, so shutting the panel loses nothing. -->
<script lang="ts">
    import type {Snippet} from "svelte";

    let {
        heading,
        note,
        open,
        onToggle,
        testid,
        children,
    }: {
        heading: string;
        note?: string;
        open: boolean;
        onToggle: (open: boolean) => void;
        testid: string;
        children: Snippet;
    } = $props();
</script>

<section class="collapse-arrow collapse bg-base-200" data-testid={testid}>
    <input type="checkbox" checked={open} onchange={(e) => onToggle(e.currentTarget.checked)} aria-label="Toggle {heading}" />
    <div class="collapse-title min-h-0 py-3 pr-10">
        <h3 class="text-xs font-semibold tracking-tight text-base-content/70">
            {heading}
            {#if note !== undefined}<span class="font-normal text-base-content/40">· {note}</span>{/if}
        </h3>
    </div>
    <div class="collapse-content flex flex-col gap-2">
        {#if open}
            {@render children()}
        {/if}
    </div>
</section>
