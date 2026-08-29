<!-- One dashboard metric box: a title, a big headline figure, optional secondary
     commodity lines, and a small line (a delta / performance figure with an
     optional arrow and colour). Presentational only. -->
<script lang="ts">
    let {
        title,
        big,
        bigClass = "",
        extras = [],
        small,
        testid,
    }: {
        title: string;
        big: string;
        bigClass?: string;
        extras?: string[];
        small?: {arrow?: string; text: string; klass?: string};
        testid?: string;
    } = $props();
</script>

<div class="card border border-base-content/5 bg-base-200 shadow-sm" data-testid={testid}>
    <div class="card-body gap-1 p-4">
        <div class="text-xs font-semibold tracking-wide text-base-content/60 uppercase">{title}</div>
        <div class="font-mono text-2xl font-semibold tabular-nums md:text-3xl {bigClass}" data-testid={testid ? `${testid}-big` : undefined}>{big}</div>
        {#if extras.length > 0}
            <div class="text-xs text-base-content/50">{extras.join(" · ")}</div>
        {/if}
        {#if small}
            <div class="mt-1 text-sm {small.klass ?? ''}">
                {#if small.arrow}<span class="mr-0.5">{small.arrow}</span>{/if}{small.text}
            </div>
        {/if}
    </div>
</div>
