<!-- The time dashboard's stat tiles (plan 46 T-dash): total hours, billable
     hours and share, the average per working day (Monday to Friday, to the
     period's end or today), days tracked of working days and, when some
     billable account names a customer, unbilled hours. Each shows its change
     against the previous period (with comparison on), opens Entries with
     the same filters, and carries the hledger command that reproduces it.

     Tiles sit two to a row, and as many as fit from `@4xl` of the
     page's own width, which follows the UI scale. -->
<script lang="ts" module>
    import type {Command} from "./hledger.js";

    export interface TileCommands {
        total: Command;
        billable: Command;
        daily: Command;
        unbilled: Command;
    }

    export interface TileLinks {
        all: string;
        billable: string;
        unbilled: string;
    }
</script>

<script lang="ts">
    import InfoTip from "#ui/components/InfoTip.svelte";
    import Delta from "./Delta.svelte";
    import {count, deltaOf, duration, percent, type Kpis} from "./model.js";

    let {
        current,
        previous,
        showUnbilled,
        commands,
        links,
    }: {
        current: Kpis;
        /** The previous period's; null with comparison off. */
        previous: Kpis | null;
        showUnbilled: boolean;
        commands: TileCommands;
        links: TileLinks;
    } = $props();

    const tip = (c: Command): string => (c.caveat === undefined ? "Reproduce it with hledger." : `Reproduce it with hledger. ${c.caveat}`);

    const share = $derived(current.billableShare);
    const tile = "flex min-w-0 flex-col gap-1 rounded-box border border-base-300 bg-base-100 p-4";
    const valueLink =
        "self-start rounded text-2xl font-semibold tracking-tight tabular-nums hover:underline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-content";
</script>

<!-- eslint-disable svelte/no-navigation-without-resolve -- every href is built on the page's resolved Entries base -->
<ul class="grid grid-cols-2 gap-3 @4xl:grid-cols-[repeat(auto-fit,minmax(12rem,1fr))]" aria-label="Time at a glance" data-testid="time-dash-tiles">
    <li class={tile} data-testid="tile-total">
        <span class="flex items-center gap-1.5 text-sm text-base-content/70"
            >Total hours <InfoTip label="total hours" text={tip(commands.total)} formula={commands.total.text} /></span
        >
        <a class={valueLink} href={links.all} data-testid="tile-total-value">{duration(current.total)}</a>
        {#if previous !== null}<Delta delta={deltaOf(current.total, previous.total, "duration")} testid="tile-total-delta" />{/if}
        <span class="text-xs text-base-content/60">{count(current.entries, "entry", "entries")}</span>
    </li>
    <li class={tile} data-testid="tile-billable">
        <span class="flex items-center gap-1.5 text-sm text-base-content/70"
            >Billable hours <InfoTip label="billable hours" text={tip(commands.billable)} formula={commands.billable.text} /></span
        >
        <a class={valueLink} href={links.billable} data-testid="tile-billable-value">{duration(current.billable)}</a>
        {#if previous !== null}<Delta delta={deltaOf(current.billable, previous.billable, "duration")} testid="tile-billable-delta" />{/if}
        <span class="text-xs text-base-content/60" data-testid="tile-billable-share">
            {share === null ? "No time yet" : `${percent(share)} of all time`}
            {#if previous !== null && share !== null && previous.billableShare !== null}
                <span class="mx-0.5" aria-hidden="true">·</span><Delta
                    delta={deltaOf(share, previous.billableShare, "points")}
                    testid="tile-billable-share-delta"
                />
            {/if}
        </span>
    </li>
    <li class={tile} data-testid="tile-average">
        <span class="flex items-center gap-1.5 text-sm text-base-content/70"
            >Average per working day <InfoTip label="the average per working day" text={tip(commands.daily)} formula={commands.daily.text} /></span
        >
        <a class={valueLink} href={links.all} data-testid="tile-average-value">{current.perWorkingDay === null ? "—" : duration(current.perWorkingDay)}</a>
        {#if previous !== null && current.perWorkingDay !== null && previous.perWorkingDay !== null}
            <Delta delta={deltaOf(current.perWorkingDay, previous.perWorkingDay, "duration")} testid="tile-average-delta" />
        {/if}
        <span class="text-xs text-base-content/60">Over {count(current.workingDays, "working day")}, Monday to Friday</span>
    </li>
    <li class={tile} data-testid="tile-days">
        <span class="flex items-center gap-1.5 text-sm text-base-content/70"
            >Days tracked <InfoTip label="days tracked" text={tip(commands.daily)} formula={commands.daily.text} /></span
        >
        <a class={valueLink} href={links.all} data-testid="tile-days-value">{current.trackedWorkingDays} of {current.workingDays}</a>
        {#if previous !== null}<Delta delta={deltaOf(current.trackedWorkingDays, previous.trackedWorkingDays, "days")} testid="tile-days-delta" />{/if}
        <span class="text-xs text-base-content/60"
            >Working days with time{current.weekendDays > 0 ? `, and ${count(current.weekendDays, "weekend day")}` : ""}</span
        >
    </li>
    {#if showUnbilled}
        <li class={tile} data-testid="tile-unbilled">
            <span class="flex items-center gap-1.5 text-sm text-base-content/70"
                >Unbilled hours <InfoTip label="unbilled hours" text={tip(commands.unbilled)} formula={commands.unbilled.text} /></span
            >
            <a class={valueLink} href={links.unbilled} data-testid="tile-unbilled-value">{duration(current.unbilled)}</a>
            {#if previous !== null}<Delta delta={deltaOf(current.unbilled, previous.unbilled, "duration")} testid="tile-unbilled-delta" />{/if}
            <span class="text-xs text-base-content/60">Billable time for a customer, not on an invoice yet</span>
        </li>
    {/if}
</ul>
