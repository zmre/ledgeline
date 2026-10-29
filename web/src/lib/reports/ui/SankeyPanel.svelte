<!-- One money-flow diagram: the statement side in one column, the accounts in
     the other, ribbons in between.

     Collapsible through `ChartPanel`, the house shell (state persisted by the
     caller). The header row always shows the title and the arrow; the graph's
     total joins it, as the panel's `aside`, ONLY while the panel is
     shut, the same job InsightsPanel's net does. Expanded, the node labels
     below carry the detail and the statement box below that carries the total,
     so a figure in the header would be the duplicate total this report was
     redesigned to remove. On an ordinary journal it is the very same string as
     the box footer, one screen apart.

     THE SHELL IS STABLE ACROSS LOAD STATES, and that is why `AsyncSection` is
     inside the panel rather than around it. Wrapped from outside, there
     was no panel at all until the data landed: no title, no total, and no
     arrow to collapse, so a user who had shut the panel still got a spinner
     block sitting where a shut panel belongs. The header is now always
     rendered and always operable; only the panel's content changes.

     Identity is never colour-alone: an always-visible legend names every
     account with its figure, each bar carries its own label, and every bar and
     ribbon answers a hover with its name, figure and share. That is the
     secondary encoding the categorical palette requires at its CVD separation
     (see #lib/format/palette).

     LABELS ARE LAID OUT, NOT DROPPED ON THE BAR. A ledger with forty small
     accounts stacks forty bars a few pixels tall, and a label at each bar's own
     middle landed on its neighbours until none could be read. Each column's
     labels now go through `layoutLabels` (./sankeyLabels): kept in bar order,
     pushed a label's height apart, held inside the chart. A label that had to
     move gets a thin leader line back to its bar, and the whole column steps
     out by the leader's width so every label in it still starts on one edge.
     A column that cannot fit name-and-figure labels loses the figure line from
     its smallest bars first; the figure is still in the legend and the hover.
     Only when even names alone cannot fit does the chart grow (`heightFor`).

     THE HOVER IS OURS, NOT LAYERCHART'S. `Tooltip.Root` tracks the pointer
     against the chart's data grid, and a Sankey's marks are bars and ribbon
     paths with no grid under them. So one small box, styled as the other
     charts' tooltips are, follows the pointer over a bar or a ribbon, and sits
     beside a bar that has keyboard focus. Each bar is a tab stop that reads out
     the same text; the ribbons are pointer-only, because the legend and the
     bars already name both ends of every one. The native `<title>`s are gone:
     the browser's own box would show up a second later, on top of this one.

     GEOMETRY IS MEASURED, NEVER ASSUMED. The label gutters are derived from the
     container's own width and the labels truncated to a budget derived from the
     same number, because the reports page is used at 375px and a desktop
     padding there would push both columns off screen. The height grows with the
     taller column instead: folding a statement line away would hide exactly the
     spending categories the diagram exists to show. -->
<script lang="ts">
    import {Chart, Link, Svg, Text} from "layerchart";
    import {Sankey} from "layerchart/graph";
    import AsyncSection from "#lib/components/AsyncSection.svelte";
    import ChartLegend from "#lib/components/ChartLegend.svelte";
    import ChartPanel from "./ChartPanel.svelte";
    import type {AmountStyle} from "#lib/domain/types.js";
    import {chartColors} from "#lib/format/chartColors.svelte.js";
    import type {FlowReport} from "#lib/reports/types.js";
    import {placeLabels, type Bar} from "./sankeyLabels";
    import {flowPalette, linkTooltip, nodeTooltip, sankeyView, tooltipText, type FlowsPanel, type SankeyTooltip} from "./sankeyModel";

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
    /** The chart's padding above and below the bars; labels may use it. */
    const PAD_Y = 8;
    /** A name-and-figure label's height: 11px name, 10px figure, 5px air. */
    const TWO_LINE = 26;
    /** A name-only label's height: 11px name, 3px air. */
    const ONE_LINE = 14;
    /** How far a column with leader lines steps its labels out, to give the lines room. */
    const LEADER = 10;
    /** Room per bar in the taller column, the height a normal P&L has always had. */
    const ROW = 30;
    /** The ordinary ceiling, and the one that only one-line labels may push past. */
    const USUAL_MAX = 900;
    const GROWN_MAX = 1600;

    // The palette is built from the WHOLE report, never from the one graph
    // below: that is what keeps an account the same colour in both diagrams.
    const viewOf = (report: FlowReport) =>
        sankeyView(inbound ? report.inflows : report.outflows, flowPalette(report, chartColors.current), report.base, styles);

    // The header figure, and only when there IS one. `AsyncSection`'s own
    // condition, mirrored: a zero standing in for an unknown total would be a
    // number the engine never sent.
    //
    // Note both panels shut means nothing is fetched at all (stores/flows), so a
    // shut panel shows a figure only while its sibling is open.
    const shown = $derived(panel.view === "data" ? panel.report : null);
    const total = $derived(shown === null ? null : viewOf(shown).total);

    // Two instances sit on the P&L tab; the slug keeps their hooks apart without
    // a prop whose only job is to be a test hook.
    const slug = $derived(title.toLowerCase().replace(/[^a-z0-9]+/g, "-"));

    let width = $state(0);

    /** Horizontal gutter for the labels, on each side. */
    const pad = $derived(Math.min(190, Math.max(90, Math.round(width * 0.28))));
    /**
     * Characters a label may occupy before it is cut, at 11px in the app's stack.
     * A column with leader lines has `LEADER` less of the gutter to spend.
     */
    function truncate(label: string, leaders: boolean): string {
        const budget = Math.max(6, Math.floor((pad - LABEL_GAP - (leaders ? LEADER : 0)) / 6.2));
        return label.length <= budget ? label : `${label.slice(0, budget - 1)}…`;
    }

    /**
     * Tall enough for the taller column, since neither column ever folds a row
     * away: `ROW` a bar up to the usual 900px, which is every P&L this panel was
     * first drawn for. Past that it grows only as far as one-line labels need,
     * so a column that `layoutLabels` has already cut to names alone still fits.
     */
    function heightFor(nodes: {side: string}[]): number {
        const sources = nodes.filter((node) => node.side === "source").length;
        const rows = Math.max(sources, nodes.length - sources);
        return Math.min(GROWN_MAX, Math.max(200, Math.min(USUAL_MAX, rows * ROW), rows * ONE_LINE));
    }

    /** What the hover box shows, where (container px), and which mark it belongs to. */
    interface Hover {
        tip: SankeyTooltip;
        color: string | null;
        x: number;
        y: number;
        /** The bar under the pointer or focus, which keeps its own ribbons lit. */
        node: string | null;
    }

    let hover = $state<Hover | null>(null);
    let frame = $state<HTMLDivElement>();

    /** The pointer, in the chart box's own pixels: the tooltip's offset parent. */
    function pointer(event: PointerEvent): {x: number; y: number} {
        const rect = frame?.getBoundingClientRect();
        return {x: event.clientX - (rect?.left ?? 0), y: event.clientY - (rect?.top ?? 0)};
    }

    /** Whether a ribbon stays lit: always, unless a bar is hovered and it is not one of that bar's. */
    function lit(link: {source: Bar; target: Bar}): boolean {
        return hover === null || hover.node === null || link.source.key === hover.node || link.target.key === hover.node;
    }
</script>

<!-- Escape dismisses the hover whether it came from the pointer or from focus. -->
<svelte:window
    onkeydown={(event) => {
        if (event.key === "Escape") hover = null;
    }}
/>

<ChartPanel heading={title} {open} {onToggle} testid="sankey-panel-{slug}">
    {#snippet aside()}
        {#if total !== null && !open}
            <span class="font-mono text-xs font-semibold tabular-nums">{total}</span>
        {/if}
    {/snippet}
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
                {@const height = heightFor(view.nodes)}
                <div class="relative w-full" style="height: {height}px" bind:clientWidth={width} bind:this={frame} data-testid="sankey-chart-{slug}">
                    {#if width > 0}
                        <Chart data={{nodes: view.nodes, links: view.links}} padding={{top: PAD_Y, right: pad, bottom: PAD_Y, left: pad}}>
                            <Svg>
                                <!-- `nodeId` is REQUIRED here: Sankey defaults
                                     it to `d.index`, and our links reference keys. -->
                                <Sankey nodeId={(d) => d.key} nodeWidth={BAR_WIDTH} nodePadding={12}>
                                    {#snippet children({nodes, links})}
                                        {@const spots = placeLabels(nodes as Bar[], {
                                            top: -PAD_Y,
                                            bottom: height - PAD_Y,
                                            twoLine: TWO_LINE,
                                            oneLine: ONE_LINE,
                                        })}
                                        <!-- Pointer-only, and hidden from the
                                             accessibility tree: the legend and
                                             the bars already name both ends. -->
                                        {#each links as link (`${link.source.key}>${link.target.key}`)}
                                            <g
                                                aria-hidden="true"
                                                onpointerenter={(event) => {
                                                    hover = {tip: linkTooltip(link), color: link.color, ...pointer(event), node: null};
                                                }}
                                                onpointermove={(event) => {
                                                    if (hover !== null) hover = {...hover, ...pointer(event)};
                                                }}
                                                onpointerleave={() => (hover = null)}
                                            >
                                                <Link
                                                    sankey
                                                    data={link}
                                                    fill="none"
                                                    stroke={link.color}
                                                    strokeWidth={Math.max(1, link.width)}
                                                    strokeOpacity={lit(link) ? 0.28 : 0.1}
                                                />
                                            </g>
                                        {/each}
                                        {#each nodes as node (node.key)}
                                            {@const middle = (node.y0 + node.y1) / 2}
                                            {@const source = node.side === "source"}
                                            {@const spot = spots[node.key]}
                                            {@const edge = source ? node.x0 : node.x1}
                                            {@const out = source ? -1 : 1}
                                            {@const x = edge + out * (LABEL_GAP + (spot?.leaders === true ? LEADER : 0))}
                                            {@const y = spot?.y ?? middle}
                                            {@const tip = nodeTooltip(node)}
                                            <!-- A tab stop that DOES nothing but
                                                 describe itself, which is what a
                                                 focusable chart mark is: no role
                                                 Svelte calls interactive fits
                                                 without promising an action
                                                 (`button`) that is not there. -->
                                            <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
                                            <g
                                                tabindex="0"
                                                role="img"
                                                aria-label={tooltipText(tip)}
                                                class="outline-none"
                                                onpointerenter={(event) => {
                                                    hover = {tip, color: node.color, ...pointer(event), node: node.key};
                                                }}
                                                onpointermove={(event) => {
                                                    if (hover !== null) hover = {...hover, ...pointer(event)};
                                                }}
                                                onpointerleave={() => (hover = null)}
                                                onfocus={() => {
                                                    hover = {tip, color: node.color, x: edge + pad, y: middle + PAD_Y, node: node.key};
                                                }}
                                                onblur={() => (hover = null)}
                                            >
                                                <rect
                                                    x={node.x0}
                                                    y={node.y0}
                                                    width={node.x1 - node.x0}
                                                    height={Math.max(1, node.y1 - node.y0)}
                                                    rx="2"
                                                    fill={node.color ?? undefined}
                                                    class={[
                                                        node.color === null && "fill-base-content/30",
                                                        hover?.node === node.key && "stroke-base-content stroke-1",
                                                    ]}
                                                />
                                                {#if spot?.displaced === true}
                                                    <!-- Out of the bar's side, a short
                                                         elbow, then straight to the
                                                         label. Labels keep bar order, so
                                                         two of these never cross. -->
                                                    <path
                                                        d="M {edge} {middle} h {out * 3} L {x - out * 3} {y}"
                                                        fill="none"
                                                        class="stroke-base-content/25"
                                                        stroke-width="1"
                                                        data-leader={node.key}
                                                    />
                                                {/if}
                                                <g data-label={node.key} data-lines={spot?.twoLine === false ? 1 : 2}>
                                                    <Text
                                                        value={truncate(node.label, spot?.leaders === true)}
                                                        {x}
                                                        y={spot?.twoLine === false ? y : y - LINE_GAP}
                                                        textAnchor={source ? "end" : "start"}
                                                        verticalAnchor="middle"
                                                        fontSize={11}
                                                        class="fill-base-content"
                                                    />
                                                    {#if spot?.twoLine !== false}
                                                        <Text
                                                            value={node.amount}
                                                            {x}
                                                            y={y + LINE_GAP}
                                                            textAnchor={source ? "end" : "start"}
                                                            verticalAnchor="middle"
                                                            fontSize={10}
                                                            class="fill-base-content/55"
                                                        />
                                                    {/if}
                                                </g>
                                            </g>
                                        {/each}
                                    {/snippet}
                                </Sankey>
                            </Svg>
                        </Chart>
                    {/if}
                    {#if hover !== null}
                        <!-- Placed off the pointer toward the roomier side, so it
                             never sits under the cursor or off the panel's edge. -->
                        <div
                            role="tooltip"
                            class="pointer-events-none absolute z-50 rounded-sm bg-base-100/90 px-2 py-1 shadow-sm backdrop-blur-[2px]"
                            style="left: {hover.x}px; top: {hover.y}px; transform: translate({hover.x > width / 2 ? 'calc(-100% - 12px)' : '12px'}, 12px)"
                            data-testid="sankey-tooltip-{slug}"
                        >
                            <div class="flex max-w-[min(15rem,60vw)] min-w-36 flex-col gap-1 text-xs">
                                <div class="flex items-center gap-2 font-semibold">
                                    {#if hover.color !== null}
                                        <span class="inline-block w-3 shrink-0 border-t-2" style="border-color:{hover.color}"></span>
                                    {/if}
                                    <span>{hover.tip.title}</span>
                                </div>
                                {#each hover.tip.lines as line, i (i)}
                                    <div class={["tabular-nums", i === 0 ? "font-semibold" : "text-base-content/60"]}>{line}</div>
                                {/each}
                            </div>
                        </div>
                    {/if}
                </div>

                <!-- Always visible: identity is never colour-alone. -->
                <ChartLegend entries={view.legend.map((entry) => ({...entry, extra: entry.amount}))} testid="sankey-legend-{slug}" />

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
</ChartPanel>
