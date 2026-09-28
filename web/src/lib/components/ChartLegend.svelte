<!-- The always-visible legend under a chart: a colour key, the name, and an
     optional muted figure after it. One markup for every chart so a legend
     reads the same wherever it sits.

     It is the charts' own markup rather than layerchart's built-in legend, for
     the reason `reports/ui/SankeyPanel.svelte` gives: it is always visible, at
     every width, and it survives a container that has not been measured yet.
     The key is what carries identity beside the colour (dataviz: identity is
     never colour-alone), so text stays in text ink, never the series colour.

     Keys: a `dot` for a slice or node, a `square` for a stacked bar or area
     segment, a `line` (solid or dashed) for a line series or the net. -->
<script lang="ts" module>
    export interface LegendEntry {
        key: string;
        label: string;
        color: string;
        /** The colour key's shape. Defaults to `dot`. */
        swatch?: "dot" | "square" | "line";
        /** Draw a `line` key dashed, as its series is. */
        dash?: boolean;
        /** A muted figure after the label (a share, an amount). */
        extra?: string;
        /** Hover text for the whole entry. */
        title?: string;
        /** Italicise the label: for a bucket that is not a real category ("(unclassified)"). */
        italic?: boolean;
    }
</script>

<script lang="ts">
    let {entries, class: className = "", testid}: {entries: readonly LegendEntry[]; class?: string; testid?: string} = $props();
</script>

<ul class={["flex flex-wrap gap-x-3 gap-y-1 text-xs text-base-content/70", className]} data-testid={testid}>
    {#each entries as entry (entry.key)}
        <li class="flex items-center gap-1" title={entry.title}>
            {#if entry.swatch === "line"}
                <span class={["inline-block w-4 shrink-0 border-t-2", entry.dash === true && "border-dashed"]} style="border-color:{entry.color}"></span>
            {:else}
                <span class={["inline-block h-2 w-2 shrink-0", entry.swatch === "square" ? "rounded-xs" : "rounded-full"]} style="background:{entry.color}"
                ></span>
            {/if}
            <span class={[entry.italic === true && "italic"]}>{entry.label}</span>
            {#if entry.extra !== undefined}<span class="text-base-content/50">{entry.extra}</span>{/if}
        </li>
    {/each}
</ul>
