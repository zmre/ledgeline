<!-- One top-N list (plan 46 T-dash): the ten largest, then the rest as
     Other, each with its hours, a thin ranked bar of its share (one hue:
     one series, so slot 1 for every row) and, with comparison on, its
     change against the previous period. Ranked by hours, or for notes by
     how often each is written. Each item opens Entries narrowed to it. -->
<script lang="ts">
    import InfoTip from "#ui/components/InfoTip.svelte";
    import Delta from "./Delta.svelte";
    import type {Command} from "./hledger.js";
    import {count, deltaOf, duration, percent, type TopList} from "./model.js";
    import type {Drill} from "./query.js";

    let {
        title,
        list,
        command,
        by = "seconds",
        compare,
        href,
        testid,
    }: {
        title: string;
        list: TopList;
        command: Command;
        /** What ranks the list; with `entries` the count is shown first. */
        by?: "seconds" | "entries";
        compare: boolean;
        href: (drill: Drill) => string;
        testid: string;
    } = $props();

    const widest = $derived(Math.max(1, ...list.items.map((i) => (by === "entries" ? i.entries : i.seconds))));
    const barWidth = (value: number): string => `${Math.max(2, (value / widest) * 100)}%`;
    const id = $props.id();
</script>

<!-- eslint-disable svelte/no-navigation-without-resolve -- every href is built on the page's resolved Entries base -->
<section class="flex min-w-0 flex-col gap-2 rounded-box border border-base-300 bg-base-100 p-4" aria-labelledby="{id}-title" data-testid={testid}>
    <h2 id="{id}-title" class="flex items-center gap-1.5 text-sm font-medium text-base-content/70">
        {title}
        <InfoTip
            label={title.toLowerCase()}
            text={command.caveat === undefined ? "Reproduce it with hledger." : `Reproduce it with hledger. ${command.caveat}`}
            formula={command.text}
        />
    </h2>
    {#if list.items.length === 0}
        <p class="text-sm text-base-content/60">Nothing in this period.</p>
    {:else}
        <ol class="flex flex-col gap-2">
            {#each list.items as item (item.key)}
                <li class="flex min-w-0 flex-col gap-1" data-testid="{testid}-item" data-key={item.key}>
                    <div class="flex min-w-0 items-baseline gap-2 text-sm">
                        {#if item.drill !== null}
                            <a class={["min-w-0 grow link truncate link-hover", item.italic && "italic"]} href={href(item.drill)} title={item.label}
                                >{item.label}</a
                            >
                        {:else}
                            <span class={["min-w-0 grow truncate", item.italic && "italic"]} title={item.label}>{item.label}</span>
                        {/if}
                        {#if by === "entries"}<span class="shrink-0 text-xs text-base-content/60 tabular-nums">{count(item.entries, "time", "times")}</span
                            >{/if}
                        <span class="shrink-0 font-medium tabular-nums">{duration(item.seconds)}</span>
                        {#if by === "seconds"}<span class="w-9 shrink-0 text-right text-xs text-base-content/60 tabular-nums">{percent(item.share)}</span>{/if}
                    </div>
                    <div class="flex items-center gap-2">
                        <span class="h-1 min-w-0 grow" aria-hidden="true">
                            <span
                                class="block h-1 rounded-full"
                                style="width: {barWidth(by === 'entries' ? item.entries : item.seconds)}; background: var(--chart-1)"
                            ></span>
                        </span>
                        {#if compare && item.previous !== null}<Delta delta={deltaOf(item.seconds, item.previous, "duration")} />{/if}
                    </div>
                </li>
            {/each}
            {#if list.other !== null}
                <li class="flex items-baseline gap-2 text-sm text-base-content/70" data-testid="{testid}-other">
                    <span class="grow">Other ({list.other.keys} more)</span>
                    <span class="tabular-nums">{duration(list.other.seconds)}</span>
                    {#if compare && list.other.previous !== null}<Delta delta={deltaOf(list.other.seconds, list.other.previous, "duration")} />{/if}
                </li>
            {/if}
        </ol>
    {/if}
</section>
