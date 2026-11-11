<!-- One dashboard metric box: a title, a big headline figure, optional secondary
     commodity lines, and a small line (a delta / performance figure with an
     optional arrow and colour). Presentational only. `aside` sits at the title
     row's right edge (the startup boxes' segment chevron); `help` puts an (i)
     after the title. -->
<script lang="ts">
    import type {Snippet} from "svelte";
    import InfoTip from "#lib/components/InfoTip.svelte";
    import type {MetricHelp} from "./metricHelp";

    let {
        title,
        big,
        bigClass = "",
        extras = [],
        small,
        testid,
        aside,
        help,
    }: {
        title: string;
        big: string;
        bigClass?: string;
        extras?: string[];
        small?: {arrow?: string; text: string; klass?: string};
        testid?: string;
        aside?: Snippet;
        help?: MetricHelp;
    } = $props();
</script>

<div class="card border border-base-content/5 bg-base-200 shadow-sm" data-testid={testid}>
    <div class="card-body gap-1 p-4">
        <div class="flex items-start justify-between gap-2">
            <div class="flex items-center gap-1">
                <div class="text-xs font-semibold tracking-wide text-base-content/60 uppercase">{title}</div>
                {#if help !== undefined}<InfoTip {...help} placement="bottom-start" />{/if}
            </div>
            {#if aside !== undefined}{@render aside()}{/if}
        </div>
        <div class="font-mono text-2xl font-semibold tabular-nums md:text-3xl {bigClass}" data-testid={testid ? `${testid}-big` : undefined}>{big}</div>
        {#if extras.length > 0}
            <div class="text-xs text-base-content/50">{extras.join(" · ")}</div>
        {/if}
        {#if small}
            <div class="mt-1 text-sm {small.klass ?? ''}" data-testid={testid ? `${testid}-small` : undefined}>
                {#if small.arrow}<span class="mr-0.5">{small.arrow}</span>{/if}{small.text}
            </div>
        {/if}
    </div>
</div>
