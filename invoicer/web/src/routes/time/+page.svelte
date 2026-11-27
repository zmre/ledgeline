<!-- Time's home (plan 46 T1) until its dashboard lands (Phase 4): today at a
     glance from the tracker's state. What runs now, today's total and where
     it went, and this week against its target, with the way to Track. -->
<script lang="ts">
    import {resolve} from "$app/paths";
    import EmptyState from "#lib/components/EmptyState.svelte";
    import {clockOf, dayHeading, durationText, elapsedText} from "#lib/time/track/format.js";
    import {tracker} from "#lib/time/track/tracker.svelte.js";
    import WeekStrip from "#lib/time/track/WeekStrip.svelte";

    const state = $derived(tracker.state);
    const running = $derived(state?.running ?? null);
    /** Today's accounts with time, most first (the running one live). */
    const spent = $derived(
        (state?.slots ?? [])
            .filter((s) => s.account !== null)
            .map((s) => ({key: s.key, label: s.label ?? s.account ?? "", seconds: s.today.seconds + (s.running ? tracker.elapsed : 0), note: s.last_note}))
            .filter((s) => s.seconds > 0)
            .sort((a, b) => b.seconds - a.seconds)
    );
    const track = resolve("/time/track");
</script>

<svelte:head>
    <title>Time · Ledgeline Invoicer</title>
</svelte:head>

<div class="mx-auto flex w-full max-w-3xl flex-col gap-4" data-testid="time-home">
    <header class="flex flex-wrap items-baseline gap-3">
        <h1 class="text-2xl font-semibold tracking-tight">Time</h1>
        {#if state?.configured}
            <!-- eslint-disable-next-line svelte/no-navigation-without-resolve -- resolve()d above -->
            <a class="btn ml-auto btn-primary btn-sm" href={track}>Track</a>
        {/if}
    </header>
    {#if state === null}
        {#if tracker.loadError !== null}
            <div class="alert alert-warning" role="alert">Time tracking couldn't be read. {tracker.loadError}</div>
        {:else}
            <p class="text-sm text-base-content/60">Loading…</p>
        {/if}
    {:else if !state.configured}
        <EmptyState
            title="Time tracking isn't set up"
            text="Track the time you spend for each customer, then bill it. Setting up creates a few plain text files beside your books."
            action="Set up time tracking"
            href={track}
            testid="time-home-empty"
        />
    {:else}
        <section class="rounded-box border border-base-300 p-4" aria-labelledby="time-home-today">
            <div class="flex flex-wrap items-baseline gap-x-3">
                <h2 id="time-home-today" class="text-base font-semibold">{dayHeading(state.today)}</h2>
                <span class="text-sm text-base-content/70"
                    ><span class="font-medium text-base-content tabular-nums" data-testid="time-home-total">{durationText(tracker.todaySeconds)}</span> today</span
                >
            </div>
            {#if running !== null}
                <p class="mt-2 text-sm" data-testid="time-home-running">
                    <span class="text-success" aria-hidden="true">▶</span>
                    {running.label}, running since {clockOf(running.since)} ({elapsedText(tracker.elapsed)})
                </p>
            {/if}
            {#if spent.length === 0}
                <p class="mt-2 text-sm text-base-content/60">Nothing tracked today yet.</p>
            {:else}
                <table class="table mt-2 table-sm">
                    <tbody>
                        {#each spent as row (row.key)}
                            <tr>
                                <td class="font-medium">{row.label}</td>
                                <td class="text-right tabular-nums">{durationText(row.seconds)}</td>
                                <td class="text-base-content/70">{row.note ?? ""}</td>
                            </tr>
                        {/each}
                    </tbody>
                </table>
            {/if}
        </section>
        <WeekStrip week={state.week} today={state.today} todaySeconds={tracker.todaySeconds} target={state.weekly_target} />
    {/if}
</div>
