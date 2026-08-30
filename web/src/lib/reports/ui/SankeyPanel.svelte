<!-- One money-flow diagram: the statement side in one column, the accounts in
     the other, ribbons in between.

     Collapsible in the house style (daisyUI `collapse collapse-arrow` driven by
     a checkbox, state persisted by the caller), with the header row and the
     graph's total staying visible when shut, exactly as InsightsPanel does with
     its net.

     THE SHELL IS STABLE ACROSS LOAD STATES, and that is why `AsyncSection` is
     inside this component rather than around it. Wrapped from outside, there
     was no panel at all until the data landed: no title, no total, and no
     arrow to collapse, so a user who had shut the panel still got a spinner
     block sitting where a shut panel belongs. The header is now always
     rendered and always operable; only `collapse-content` changes.

     Identity is never colour-alone: an always-visible legend names every
     account, each bar carries its own label and figure, and every ribbon has a
     `<title>`. That is the secondary encoding the categorical palette requires
     at its CVD separation (see $lib/format/palette).

     GEOMETRY IS MEASURED, NEVER ASSUMED. The label gutters are derived from the
     container's own width and the labels truncated to a budget derived from the
     same number, because the reports page is used at 375px and a desktop
     padding there would push both columns off screen. The height grows with the
     taller column instead: folding a statement line away would hide exactly the
     spending categories the diagram exists to show. -->
<script lang="ts">
    import {Chart, Link, Svg, Text} from "layerchart";
    import {Sankey} from "layerchart/graph";
    import AsyncSection from "$lib/components/AsyncSection.svelte";
    import type {AmountStyle} from "$lib/domain/types";
    import type {FlowReport} from "$lib/reports/types";
    import {flowPalette, sankeyView, type FlowsPanel} from "./sankeyModel";

    let {
        title,
        caption,
        panel,
        inbound,
        styles,
        open,
        onToggle,
    }: {
        title: string;
        caption: string;
        /** The whole async slice, so the shell can render before the report exists. */
        panel: FlowsPanel;
        /** Which of the report's two graphs this panel draws. */
        inbound: boolean;
        styles: ReadonlyMap<string, AmountStyle>;
        open: boolean;
        onToggle: (open: boolean) => void;
    } = $props();

    /** Gap between a bar and its label. */
    const LABEL_GAP = 6;
    const BAR_WIDTH = 10;
    /** Half the gap between the label line and the figure under it. */
    const LINE_GAP = 6;

    // The palette is built from the WHOLE report, never from the one graph
    // below: that is what keeps an account the same colour in both diagrams.
    const viewOf = (report: FlowReport) => sankeyView(inbound ? report.inflows : report.outflows, flowPalette(report), report.base, styles);

    // The header figure, and only when there IS one. `AsyncSection`'s own
    // condition, mirrored: a zero standing in for an unknown total would be a
    // number the engine never sent.
    const shown = $derived(panel.view === "data" ? panel.report : null);
    const total = $derived(shown === null ? null : viewOf(shown).total);

    // Two instances sit on the P&L tab; the slug keeps their hooks apart without
    // a prop whose only job is to be a test hook.
    const slug = $derived(title.toLowerCase().replace(/[^a-z0-9]+/g, "-"));

    let width = $state(0);

    /** Horizontal gutter for the labels, on each side. */
    const pad = $derived(Math.min(190, Math.max(90, Math.round(width * 0.28))));
    /** Characters a label may occupy before it is cut, at 11px in the app's stack. */
    const budget = $derived(Math.max(6, Math.floor((pad - LABEL_GAP) / 6.2)));

    function truncate(label: string): string {
        return label.length <= budget ? label : `${label.slice(0, budget - 1)}…`;
    }

    /** Tall enough for the taller column, since neither column ever folds a row away. */
    function heightFor(nodes: {side: string}[]): number {
        const sources = nodes.filter((node) => node.side === "source").length;
        return Math.min(900, Math.max(200, Math.max(sources, nodes.length - sources) * 30));
    }
</script>

<section class="collapse-arrow collapse bg-base-200" data-testid="sankey-panel-{slug}">
    <input type="checkbox" checked={open} onchange={(e) => onToggle(e.currentTarget.checked)} aria-label="Toggle {title}" />
    <div class="collapse-title flex min-h-0 items-center justify-between gap-2 py-3 pr-10">
        <h3 class="text-sm font-semibold tracking-tight">{title}</h3>
        {#if total !== null}
            <span class="font-mono text-sm font-semibold tabular-nums">{total}</span>
        {/if}
    </div>
    <div class="collapse-content flex flex-col gap-2">
        <p class="text-xs text-base-content/50">{caption}</p>

        <!-- Error branch BEFORE the data branch, which is `AsyncSection`'s whole
             job (FE-5). A flows fetch that fails says so here and leaves the
             statement below completely untouched. -->
        <AsyncSection
            view={panel.view}
            value={panel.report}
            error={panel.error}
            testid="flows-error-{slug}"
            label="the money flows"
            loadingLabel="Loading {title}"
            onRetry={panel.retry}
        >
            {#snippet children(report)}
                {@const view = viewOf(report)}
                {#if view.links.length === 0}
                    <!-- The two reasons read differently, and only one of them is
                         about this date range. -->
                    <p class="py-8 text-center text-sm text-base-content/60" data-testid="sankey-empty-{slug}">
                        {#if report.base === null}
                            Several commodities here, with no prices between them, so there is no width to draw.
                        {:else}
                            Nothing in this range.
                        {/if}
                    </p>
                {:else}
                    <div class="w-full" style="height: {heightFor(view.nodes)}px" bind:clientWidth={width} data-testid="sankey-chart-{slug}">
                        {#if width > 0}
                            <Chart data={{nodes: view.nodes, links: view.links}} padding={{top: 8, right: pad, bottom: 8, left: pad}}>
                                <Svg>
                                    <!-- `nodeId` is REQUIRED here: Sankey defaults
                                         it to `d.index`, and our links reference keys. -->
                                    <Sankey nodeId={(d) => d.key} nodeWidth={BAR_WIDTH} nodePadding={12}>
                                        {#snippet children({nodes, links})}
                                            {#each links as link (`${link.source.key}>${link.target.key}`)}
                                                <g>
                                                    <title>{link.title}</title>
                                                    <Link
                                                        sankey
                                                        data={link}
                                                        fill="none"
                                                        stroke={link.color}
                                                        strokeWidth={Math.max(1, link.width)}
                                                        strokeOpacity={0.28}
                                                    />
                                                </g>
                                            {/each}
                                            {#each nodes as node (node.key)}
                                                {@const middle = (node.y0 + node.y1) / 2}
                                                {@const source = node.side === "source"}
                                                {@const x = source ? node.x0 - LABEL_GAP : node.x1 + LABEL_GAP}
                                                <g>
                                                    <title>{node.label}: {node.amount}</title>
                                                    <rect
                                                        x={node.x0}
                                                        y={node.y0}
                                                        width={node.x1 - node.x0}
                                                        height={Math.max(1, node.y1 - node.y0)}
                                                        rx="2"
                                                        fill={node.color ?? undefined}
                                                        class={node.color === null ? "fill-base-content/30" : ""}
                                                    />
                                                    <Text
                                                        value={truncate(node.label)}
                                                        {x}
                                                        y={middle - LINE_GAP}
                                                        textAnchor={source ? "end" : "start"}
                                                        verticalAnchor="middle"
                                                        fontSize={11}
                                                        class="fill-base-content"
                                                    />
                                                    <Text
                                                        value={node.amount}
                                                        {x}
                                                        y={middle + LINE_GAP}
                                                        textAnchor={source ? "end" : "start"}
                                                        verticalAnchor="middle"
                                                        fontSize={10}
                                                        class="fill-base-content/55"
                                                    />
                                                </g>
                                            {/each}
                                        {/snippet}
                                    </Sankey>
                                </Svg>
                            </Chart>
                        {/if}
                    </div>

                    <!-- Always visible: identity is never colour-alone. -->
                    <ul class="flex flex-wrap gap-x-3 gap-y-1 text-xs text-base-content/70" data-testid="sankey-legend-{slug}">
                        {#each view.legend as entry (entry.key)}
                            <li class="flex items-center gap-1">
                                <span class="inline-block h-2 w-2 shrink-0 rounded-full" style="background:{entry.color}"></span>
                                {entry.label}
                                <span class="text-base-content/50">{entry.amount}</span>
                            </li>
                        {/each}
                    </ul>

                    {#if !view.complete}
                        <!-- The gap is a fact about the journal (a posting with no
                             counterparty, or a line that netted negative over the
                             window), and this line is the only place it can be seen. -->
                        <p class="text-xs text-base-content/50" data-testid="sankey-incomplete-{slug}">
                            Showing {view.total} of {view.sectionTotal}
                        </p>
                    {/if}
                {/if}
            {/snippet}
        </AsyncSection>
    </div>
</section>
