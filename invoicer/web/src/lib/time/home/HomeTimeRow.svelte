<!-- Home's time row (item 5): three stat tiles under the receivables.
     - Hours this week, with the change against last week to the same
       weekday (the Overview's comparison), opening the Overview on This
       week;
     - the billable share this week, with its change in points, opening the
       Overview on This week split by billable;
     - what is unbilled across customers: the estimate at each account's
       item rate (per commodity) and the hours, opening Entries' unbilled
       time, with Bill… to To bill.

     Stat tiles, not charts (dataviz: a single headline is a number, no
     plot). Deltas stay in text ink: more hours is neither good nor bad.
     Links arrive resolved, so this mounts as is in a component test. The
     tiles go three to a row from `@lg` of the page's own width, which
     follows the UI scale. -->
<script lang="ts">
    import Delta from "../dashboard/Delta.svelte";
    import {count, deltaOf, duration, percent} from "../dashboard/model.js";
    import {moneyText} from "../billing/words.js";
    import {weekdayName} from "../dates.js";
    import type {HomeTime, HomeTimeLinks} from "./homeTime.js";

    let {time, links}: {time: HomeTime; links: HomeTimeLinks} = $props();

    const now = $derived(time.thisWeek);
    const last = $derived(time.lastWeek);
    const unbilled = $derived(time.unbilled);
    /** Last week counts to today's weekday; a Sunday's is the whole week. */
    const lastLabel = $derived(weekdayName(last.end) === "Sun" ? "Last week" : `Last week to ${weekdayName(last.end)}`);
    const money = $derived(unbilled.money.map((m) => moneyText(m.amount, m.commodity === "" ? null : m.commodity) ?? m.amount).join(" + "));
    const unbilledDetail = $derived(
        [
            unbilled.seconds === 0 ? null : `${duration(unbilled.seconds)} for ${count(unbilled.customers, "customer")}`,
            unbilled.unpriced && unbilled.money.length > 0 ? "some without a rate" : null,
            unbilled.needsCustomer > 0 ? `${duration(unbilled.needsCustomer)} needs a customer` : null,
        ]
            .filter((part) => part !== null)
            .join(" · ")
    );

    const tile = "flex min-w-0 flex-col gap-1 rounded-box border border-base-300 bg-base-100 p-4";
    const tileLink = `${tile} transition-[translate,box-shadow] motion-reduce:transition-none motion-reduce:hover:translate-none motion-reduce:focus-visible:translate-none hover:-translate-y-0.5 hover:border-base-content/30 hover:shadow-md focus-visible:-translate-y-0.5 focus-visible:shadow-md focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-content`;
    const valueLink =
        "self-start rounded text-2xl font-semibold tracking-tight tabular-nums hover:underline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-content";
</script>

<!-- eslint-disable svelte/no-navigation-without-resolve -- every href arrives resolved from the page -->
<section class="flex flex-col gap-2" aria-labelledby="home-time-title" data-testid="home-time-row">
    <h2 id="home-time-title" class="text-sm font-medium text-base-content/70">
        <a class="link link-hover" href={links.overview}>Time</a>
    </h2>
    <ul class="grid gap-3 @lg:grid-cols-3" aria-label="Time at a glance">
        <li class="flex">
            <a class="{tileLink} flex-1" href={links.hours} data-testid="home-time-hours">
                <span class="text-sm text-base-content/70">Hours this week</span>
                <span class="text-2xl font-semibold tracking-tight tabular-nums" data-testid="home-time-hours-value">{duration(now.total)}</span>
                <span class="flex flex-wrap items-baseline gap-x-2 text-xs text-base-content/60">
                    <Delta delta={deltaOf(now.total, last.total, "duration")} testid="home-time-hours-delta" />
                    <span>{lastLabel}: {duration(last.total)}</span>
                </span>
            </a>
        </li>
        <li class="flex">
            <a class="{tileLink} flex-1" href={links.billable} data-testid="home-time-billable">
                <span class="text-sm text-base-content/70">Billable this week</span>
                <span class="text-2xl font-semibold tracking-tight tabular-nums" data-testid="home-time-billable-value"
                    >{now.share === null ? "—" : percent(now.share)}</span
                >
                <span class="flex flex-wrap items-baseline gap-x-2 text-xs text-base-content/60">
                    {#if now.share !== null && last.share !== null}
                        <Delta delta={deltaOf(now.share, last.share, "points")} testid="home-time-billable-delta" />
                    {/if}
                    <span>{now.share === null ? "No time this week yet" : `${duration(now.billable)} of ${duration(now.total)}`}</span>
                </span>
            </a>
        </li>
        <li class="flex">
            <div class="{tile} flex-1" data-testid="home-time-unbilled">
                <span class="text-sm text-base-content/70">Unbilled</span>
                {#if unbilled.seconds === 0}
                    <a class={valueLink} href={links.bill} data-testid="home-time-unbilled-value">Nothing to bill</a>
                    {#if unbilledDetail !== ""}<span class="text-xs text-base-content/60">{unbilledDetail}</span>{/if}
                {:else}
                    <div class="flex flex-wrap items-center justify-between gap-2">
                        <a class={valueLink} href={links.unbilled} data-testid="home-time-unbilled-value">{money === "" ? duration(unbilled.seconds) : money}</a
                        >
                        <a class="btn btn-primary btn-sm" href={links.bill} data-testid="home-time-bill">Bill…</a>
                    </div>
                    <span class="text-xs text-base-content/60" data-testid="home-time-unbilled-detail">{unbilledDetail}</span>
                {/if}
            </div>
        </li>
    </ul>
</section>
