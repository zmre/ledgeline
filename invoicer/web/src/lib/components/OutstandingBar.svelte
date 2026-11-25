<!-- The outstanding bar (PLAN R3, R16): one horizontal stacked bar on a
     tick-labelled axis, overdue in the theme's error colour then not yet due
     in its warning colour, against round ticks from 0 to the first step past
     the scale (the total, or $10,000 when that is more), labelled compactly
     ("0, 2k, 4k…"). Status colours, not a series palette (R3).

     Each segment is a link to the invoices behind it and its own hit target:
     hover or keyboard focus lifts it (an outline; the other segment dims) and
     shows one tooltip, value first: the amount, then the name with a line key
     in its colour, the count and its share of what is owed. The tooltip
     enhances, never gates: the legend under the bar prints every amount with
     a swatch, so nothing relies on colour alone (yellow on white is 1.72:1),
     and the aging list beside it is the table view. Escape hides the
     tooltip. Transitions are opacity only, and off under
     prefers-reduced-motion. -->
<script lang="ts">
    import {axisTicks, barSegments, compactNumber, count, listHref, percent, type BarSegment, type CommodityView} from "#lib/dashboard.js";

    let {
        view,
        listBase,
    }: {
        view: CommodityView;
        /** The invoice list's URL, resolved (`/app/invoices`). */
        listBase: string;
    } = $props();

    const ticks = $derived(axisTicks(view.scale.amount));
    const top = $derived(ticks[ticks.length - 1] ?? 0);
    const segments = $derived(barSegments(view, top));
    const tooltipId = $props.id();

    let active = $state<BarSegment["id"] | null>(null);
    const shown = $derived(segments.find((s) => s.id === active) ?? null);

    const NAMES = {overdue: "Overdue", "not-due": "Not yet due"} as const;
    const FILL = {overdue: "bg-error", "not-due": "bg-warning"} as const;
    const part = (id: BarSegment["id"]) => (id === "overdue" ? view.overdue : view.not_due);
    const label = (s: BarSegment): string => `${NAMES[s.id]}: ${part(s.id).text}, ${count(part(s.id).count, "item")}, ${percent(s.share)} of what is owed`;
    const href = (id: BarSegment["id"]): string => listHref(listBase, id === "overdue" ? {status: "overdue"} : {aged: "current"});
    // The second segment starts 2px in: the gap between fills (dataviz).
    const place = (s: BarSegment): string =>
        s.start === 0 ? `left: 0; width: ${s.width}%` : `left: calc(${s.start}% + 2px); width: max(4px, calc(${s.width}% - 2px))`;
    // The tooltip over the segment's middle, kept inside the bar's box.
    const tipLeft = (s: BarSegment): number => Math.min(80, Math.max(20, s.start + s.width / 2));
    // The first and last labels sit inside the axis; the rest centre on their tick.
    const tickShift = (at: number): string => (at === 0 ? "translateX(0)" : at === ticks.length - 1 ? "translateX(-100%)" : "translateX(-50%)");

    function keydown(event: KeyboardEvent): void {
        if (event.key === "Escape") active = null;
    }
</script>

<!-- eslint-disable svelte/no-navigation-without-resolve -- every href is built on listBase, which the page resolves -->
<div class="flex min-w-0 flex-col gap-1" data-testid="dashboard-bar-chart">
    <!-- The axis: tick labels over the bar, a short tick under each. For the eye: the legend and the aging list print every figure. -->
    <div class="relative h-4 text-xs text-base-content/60 tabular-nums" aria-hidden="true" data-testid="bar-axis">
        {#each ticks as tick, at (tick)}
            <span class="absolute top-0 whitespace-nowrap" style="left: {top > 0 ? (tick / top) * 100 : 0}%; transform: {tickShift(at)}" data-testid="bar-tick"
                >{compactNumber(tick)}</span
            >
        {/each}
    </div>
    <div class="relative h-1" aria-hidden="true">
        {#each ticks as tick (tick)}
            <span class="absolute top-0 h-1 w-px bg-base-content/25" style="left: min({top > 0 ? (tick / top) * 100 : 0}%, calc(100% - 1px))"></span>
        {/each}
    </div>

    <!-- The bar: a quiet track the width of the axis, the segments on it, 4px corners each. -->
    <div
        class="relative h-7 w-full rounded bg-base-content/5"
        role="group"
        aria-label="Outstanding, overdue and not yet due, on an axis to {compactNumber(top)}"
        data-testid="dashboard-bar"
    >
        {#each segments as segment (segment.id)}
            <a
                class="absolute inset-y-0 rounded {FILL[
                    segment.id
                ]} transition-opacity hover:outline-2 hover:outline-offset-2 hover:outline-base-content/60 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-content motion-reduce:transition-none {shown !==
                    null && shown.id !== segment.id
                    ? 'opacity-50'
                    : ''}"
                style={place(segment)}
                href={href(segment.id)}
                aria-label={label(segment)}
                aria-describedby={active === segment.id ? tooltipId : undefined}
                onpointerenter={() => (active = segment.id)}
                onpointerleave={() => (active = null)}
                onfocus={() => (active = segment.id)}
                onblur={() => (active = null)}
                onkeydown={keydown}
                data-testid="bar-{segment.id}"
                data-width={segment.width.toFixed(2)}
            ></a>
        {/each}
        {#if shown !== null}
            {@const p = part(shown.id)}
            <div
                id={tooltipId}
                role="tooltip"
                class="pointer-events-none absolute bottom-full z-10 mb-2 flex -translate-x-1/2 flex-col gap-0.5 rounded-box border border-base-300 bg-base-100 px-3 py-2 text-xs whitespace-nowrap shadow-md"
                style="left: {tipLeft(shown)}%"
                data-testid="bar-tooltip"
            >
                <span class="text-sm font-semibold text-base-content">{p.text}</span>
                <span class="flex items-center gap-1.5 text-base-content/70">
                    <span class="inline-block h-0.5 w-3 rounded-full {FILL[shown.id]}" aria-hidden="true"></span>
                    {NAMES[shown.id]}
                </span>
                <span class="text-base-content/60">{count(p.count, "item")} · {percent(shown.share)} of what is owed</span>
            </div>
        {/if}
    </div>

    <!-- The legend: a swatch, the name and the figures, each a link to the invoices behind it. -->
    <div class="mt-2 flex flex-wrap items-baseline gap-x-5 gap-y-2 text-sm">
        <a
            class="group flex items-baseline gap-2 rounded px-1 hover:bg-base-200 focus-visible:outline-2 focus-visible:outline-base-content"
            href={listHref(listBase, {status: "overdue"})}
            data-testid="dashboard-overdue"
        >
            <span class="inline-block h-2.5 w-2.5 shrink-0 rounded-xs bg-error" aria-hidden="true"></span>
            <span class="text-base-content/70">Overdue</span>
            <span class="font-semibold group-hover:underline">{view.overdue.text}</span>
            <span class="text-base-content/60">{count(view.overdue.count, "item")}</span>
        </a>
        <a
            class="group flex items-baseline gap-2 rounded px-1 hover:bg-base-200 focus-visible:outline-2 focus-visible:outline-base-content"
            href={listHref(listBase, {aged: "current"})}
            data-testid="dashboard-not-due"
        >
            <span class="inline-block h-2.5 w-2.5 shrink-0 rounded-xs bg-warning" aria-hidden="true"></span>
            <span class="text-base-content/70">Not yet due</span>
            <span class="font-semibold group-hover:underline">{view.not_due.text}</span>
            <span class="text-base-content/60">{count(view.not_due.count, "item")}</span>
        </a>
    </div>
</div>
