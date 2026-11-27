<!-- Hours over time (plan 46 T-dash): stacked columns per day, week or month
     (chosen from the period's length, switchable), split by top-level
     account, customer, person or billable, with an optional weekly-target
     line (a day's share of it, target ÷ 5, on working days at day grain).

     Built under the dataviz skill: one y-axis in hours from zero, recessive
     hairline grid, columns at most 24px wide with a 4px rounded top and a
     square base, a 2px surface gap between stacked segments, at most seven
     named series plus Other, each colour following its entity (`chartOf`'s
     stable order), never its rank. The fills are the theme's validated
     `--chart-N` tokens, used as CSS variables: these are HTML boxes, so a
     theme switch recolours them with no resolving step.

     The hit target is the whole column, not the painted bar: hover or focus
     shows one tooltip listing every series in it, value first, with line
     keys; Enter opens that bucket's entries. The columns are one tab stop:
     the arrow keys, Home and End move along them. The legend is always shown
     (identity never colour alone), and the table under the chart holds
     every figure, so the tooltip enhances and never gates. -->
<script lang="ts">
    import ChartLegend, {type LegendEntry} from "#ui/components/ChartLegend.svelte";
    import InfoTip from "#ui/components/InfoTip.svelte";
    import type {Command} from "./hledger.js";
    import {duration, hoursNumber, hourTicks, tickEvery, type Chart} from "./model.js";
    import {GRAINS, SPLIT_LABELS, SPLITS, type Drill, type Grain, type Split} from "./query.js";

    let {
        chart,
        grain,
        split,
        hidePerson = false,
        weeklyTarget,
        showTarget,
        command,
        bucketHref,
        seriesHref,
        onSplit,
        onGrain,
        onTarget,
    }: {
        chart: Chart;
        grain: Grain;
        split: Split;
        /** Only one person: no Person split. */
        hidePerson?: boolean;
        /** Hours a week; 0 is none. */
        weeklyTarget: number;
        showTarget: boolean;
        command: Command;
        /** Entries for one bucket's dates. */
        bucketHref: (from: string, to: string) => string;
        /** Entries for one series, over the period. */
        seriesHref: (drill: Drill) => string;
        onSplit: (split: Split) => void;
        onGrain: (grain: Grain) => void;
        onTarget: (on: boolean) => void;
    } = $props();

    const GRAIN_LABELS: Record<Grain, string> = {day: "Day", week: "Week", month: "Month"};

    /** The target for one bucket, in seconds: a week's, or a working day's fifth of it. */
    const targetSeconds = $derived(
        weeklyTarget <= 0 || !showTarget || grain === "month" ? null : grain === "week" ? weeklyTarget * 3600 : (weeklyTarget * 3600) / 5
    );
    const peak = $derived(Math.max(0, ...chart.totals, targetSeconds ?? 0));
    const ticks = $derived(hourTicks(peak));
    const top = $derived(ticks[ticks.length - 1] ?? 3600);
    const empty = $derived(chart.totals.every((t) => t === 0));

    let width = $state(0);
    // About one label per 44px; jsdom (no layout) shows a dozen.
    const every = $derived(tickEvery(chart.buckets.length, width > 0 ? Math.floor(width / 44) : 12));

    let active = $state<number | null>(null);
    const tooltipId = $props.id();

    const legend = $derived<LegendEntry[]>([
        ...chart.series.map((s) => ({key: s.key, label: s.label, color: s.color, swatch: "square" as const, italic: s.italic, extra: duration(s.total)})),
        ...(targetSeconds === null
            ? []
            : [
                  {
                      key: "target",
                      label: grain === "week" ? `Target ${weeklyTarget}h a week` : `Target ${hoursNumber(targetSeconds)}h a working day`,
                      color: "currentColor",
                      swatch: "line" as const,
                      dash: true,
                  },
              ]),
    ]);

    const bucketLabel = (i: number): string => {
        const b = chart.buckets[i];
        const parts = chart.series.filter((s) => s.values[i] !== 0).map((s) => `${s.label} ${duration(s.values[i])}`);
        return `${b.title}: ${duration(chart.totals[i])}${parts.length > 0 ? `, ${parts.join(", ")}` : ""}`;
    };

    const tipLeft = (i: number): number => Math.min(85, Math.max(15, ((i + 0.5) / chart.buckets.length) * 100));
    const pct = (seconds: number): number => (top > 0 ? (seconds / top) * 100 : 0);
    const targetHere = (i: number): boolean => targetSeconds !== null && (grain === "week" || !chart.buckets[i].weekend);

    // The columns are one tab stop (roving focus): arrows move along them,
    // Home and End go to the ends, Enter opens the focused bucket's entries.
    let focusAt = $state(0);
    const tabStop = $derived(Math.min(focusAt, Math.max(0, chart.buckets.length - 1)));

    function keydown(event: KeyboardEvent, i: number): void {
        if (event.key === "Escape") {
            active = null;
            return;
        }
        const last = chart.buckets.length - 1;
        const to =
            event.key === "ArrowRight"
                ? Math.min(last, i + 1)
                : event.key === "ArrowLeft"
                  ? Math.max(0, i - 1)
                  : event.key === "Home"
                    ? 0
                    : event.key === "End"
                      ? last
                      : null;
        if (to === null) return;
        event.preventDefault();
        focusAt = to;
        const column = (event.currentTarget as HTMLElement).parentElement?.children[to];
        if (column instanceof HTMLElement) column.focus();
    }
</script>

<!-- eslint-disable svelte/no-navigation-without-resolve -- every href is built on the page's resolved Entries base -->
<section
    class="flex min-w-0 flex-col gap-3 rounded-box border border-base-300 bg-base-100 p-4"
    aria-labelledby="{tooltipId}-title"
    data-testid="time-dash-chart"
>
    <div class="flex flex-wrap items-center justify-between gap-2">
        <h2 id="{tooltipId}-title" class="flex items-center gap-1.5 text-sm font-medium text-base-content/70">
            Hours over time
            <InfoTip
                label="hours over time"
                text={command.caveat === undefined ? "Reproduce it with hledger." : `Reproduce it with hledger. ${command.caveat}`}
                formula={command.text}
            />
        </h2>
        <div class="flex flex-wrap items-center gap-2">
            <label class="flex items-center gap-1.5 text-xs whitespace-nowrap text-base-content/70"
                >Split by
                <select class="select select-xs" value={split} onchange={(event) => onSplit(event.currentTarget.value as Split)} data-testid="time-dash-split">
                    {#each SPLITS as s (s)}
                        {#if s !== "person" || !hidePerson || split === "person"}
                            <option value={s}>{SPLIT_LABELS[s]}</option>
                        {/if}
                    {/each}
                </select>
            </label>
            <div class="join" role="group" aria-label="One bar per">
                {#each GRAINS as g (g)}
                    <button
                        type="button"
                        class={["btn join-item btn-xs", g === grain && "btn-active"]}
                        aria-pressed={g === grain}
                        onclick={() => onGrain(g)}
                        data-testid="time-dash-grain-{g}">{GRAIN_LABELS[g]}</button
                    >
                {/each}
            </div>
            {#if weeklyTarget > 0 && grain !== "month"}
                <label class="flex items-center gap-1.5 text-xs text-base-content/70">
                    <input
                        type="checkbox"
                        class="checkbox checkbox-xs"
                        checked={showTarget}
                        onchange={(event) => onTarget(event.currentTarget.checked)}
                        data-testid="time-dash-target"
                    />
                    Target
                </label>
            {/if}
        </div>
    </div>

    {#if empty}
        <p class="py-10 text-center text-sm text-base-content/60" data-testid="time-dash-chart-empty">No time in this period.</p>
    {:else}
        <div class="flex gap-2">
            <!-- The y-axis: hours, round, from zero. For the eye: the table holds every figure. -->
            <div class="relative h-48 w-8 shrink-0 text-right text-xs text-base-content/60 tabular-nums" aria-hidden="true">
                {#each ticks as tick (tick)}
                    <span class="absolute right-0 translate-y-1/2" style="bottom: {pct(tick)}%">{hoursNumber(tick)}h</span>
                {/each}
            </div>
            <div class="flex min-w-0 grow flex-col gap-1">
                <div
                    class="relative h-48"
                    bind:clientWidth={width}
                    role="group"
                    aria-label="Hours per {grain}, split by {SPLIT_LABELS[split].toLowerCase()}"
                    data-testid="time-dash-plot"
                >
                    {#each ticks as tick (tick)}
                        <span class="pointer-events-none absolute inset-x-0 h-px bg-base-content/10" style="bottom: {pct(tick)}%" aria-hidden="true"></span>
                    {/each}
                    <div class="absolute inset-0 flex items-stretch">
                        {#each chart.buckets as bucket, i (bucket.from)}
                            <a
                                class={[
                                    "group relative flex min-w-0 flex-1 items-end justify-center px-px focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-base-content",
                                    bucket.weekend && "bg-base-content/[0.03]",
                                ]}
                                href={bucketHref(bucket.from, bucket.to)}
                                aria-label={bucketLabel(i)}
                                aria-describedby={active === i ? tooltipId : undefined}
                                onpointerenter={() => (active = i)}
                                onpointerleave={() => (active = null)}
                                tabindex={i === tabStop ? 0 : -1}
                                onfocus={() => {
                                    active = i;
                                    focusAt = i;
                                }}
                                onblur={() => (active = null)}
                                onkeydown={(event) => keydown(event, i)}
                                data-testid="time-dash-column"
                                data-total={chart.totals[i]}
                            >
                                {#if chart.totals[i] > 0}
                                    {@const parts = chart.series.filter((s) => s.values[i] > 0)}
                                    <span
                                        class={[
                                            "flex w-full max-w-6 flex-col-reverse gap-0.5 transition-opacity motion-reduce:transition-none",
                                            active !== null && active !== i && "opacity-50",
                                        ]}
                                        style="height: {pct(chart.totals[i])}%"
                                    >
                                        {#each parts as s, at (s.key)}
                                            <span
                                                class={["block min-h-px w-full", at === parts.length - 1 && "rounded-t"]}
                                                style="flex: {s.values[i]} 1 0px; background: {s.color}"
                                                data-testid="time-dash-segment"
                                                data-series={s.key}
                                            ></span>
                                        {/each}
                                    </span>
                                {/if}
                                {#if targetHere(i) && targetSeconds !== null}
                                    <span
                                        class="pointer-events-none absolute inset-x-0 border-t-2 border-dashed border-base-content/60"
                                        style="bottom: {pct(targetSeconds)}%"
                                        aria-hidden="true"
                                    ></span>
                                {/if}
                            </a>
                        {/each}
                    </div>
                    {#if active !== null}
                        {@const i = active}
                        <div
                            id={tooltipId}
                            role="tooltip"
                            class="pointer-events-none absolute bottom-full z-10 mb-2 flex -translate-x-1/2 flex-col gap-0.5 rounded-box border border-base-300 bg-base-100 px-3 py-2 text-xs whitespace-nowrap shadow-md"
                            style="left: {tipLeft(i)}%"
                            data-testid="time-dash-tooltip"
                        >
                            <span class="text-sm font-semibold text-base-content tabular-nums">{duration(chart.totals[i])}</span>
                            <span class="text-base-content/70">{chart.buckets[i].title}</span>
                            {#each chart.series.filter((s) => s.values[i] !== 0) as s (s.key)}
                                <span class="flex items-center gap-1.5 text-base-content/70">
                                    <span class="inline-block h-0.5 w-3 rounded-full" style="background: {s.color}" aria-hidden="true"></span>
                                    <span class="font-semibold text-base-content tabular-nums">{duration(s.values[i])}</span>
                                    <span class={[s.italic && "italic"]}>{s.label}</span>
                                </span>
                            {/each}
                            {#if targetHere(i) && targetSeconds !== null}
                                <span class="text-base-content/60">Target {duration(targetSeconds)}</span>
                            {/if}
                        </div>
                    {/if}
                </div>
                <!-- The x-axis: every k-th bucket labelled, so labels never collide. -->
                <div class="flex text-xs text-base-content/60 tabular-nums" aria-hidden="true">
                    {#each chart.buckets as bucket, i (bucket.from)}
                        <span class="min-w-0 flex-1 overflow-visible text-center whitespace-nowrap">{i % every === 0 ? bucket.tick : ""}</span>
                    {/each}
                </div>
            </div>
        </div>
        <ChartLegend entries={legend} testid="time-dash-legend" />
        <details class="text-sm" data-testid="time-dash-table">
            <summary class="cursor-pointer text-xs text-base-content/70">Table</summary>
            <div class="mt-2 overflow-x-auto">
                <table class="table table-xs">
                    <thead>
                        <tr>
                            <th scope="col">{grain === "day" ? "Day" : grain === "week" ? "Week" : "Month"}</th>
                            {#each chart.series as s (s.key)}
                                <th scope="col" class="text-right">
                                    {#if s.drill !== null}<a class="link link-hover" href={seriesHref(s.drill)}>{s.label}</a>{:else}<span
                                            class={[s.italic && "italic"]}>{s.label}</span
                                        >{/if}
                                </th>
                            {/each}
                            <th scope="col" class="text-right">Total</th>
                        </tr>
                    </thead>
                    <tbody>
                        {#each chart.buckets as bucket, i (bucket.from)}
                            <tr>
                                <th scope="row" class="font-normal"><a class="link link-hover" href={bucketHref(bucket.from, bucket.to)}>{bucket.title}</a></th>
                                {#each chart.series as s (s.key)}
                                    <td class="text-right tabular-nums">{s.values[i] === 0 ? "" : duration(s.values[i])}</td>
                                {/each}
                                <td class="text-right font-medium tabular-nums">{duration(chart.totals[i])}</td>
                            </tr>
                        {/each}
                    </tbody>
                </table>
            </div>
        </details>
    {/if}
</section>
