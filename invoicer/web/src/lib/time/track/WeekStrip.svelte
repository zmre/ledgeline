<!-- This week, Monday to Sunday (plan 46 T-slots' "week"): each day's total,
     today marked, and the week's total against the weekly target when one
     is set. Today's figure is live (`todaySeconds`, the running timer in). -->
<script lang="ts">
    import type {Day} from "#lib/time/types.js";
    import {durationText, weekdayOf} from "./format.js";

    let {week, today, todaySeconds, target}: {week: Day[]; today: string; todaySeconds: number; target: number} = $props();

    const days = $derived(week.map((d) => ({date: d.date, seconds: d.date === today ? Math.max(d.total.seconds, todaySeconds) : d.total.seconds})));
    const total = $derived(days.reduce((sum, d) => sum + d.seconds, 0));
    const percent = $derived(target > 0 ? Math.min(100, Math.round((total / (target * 3600)) * 100)) : null);
</script>

<section aria-label="This week" class="flex flex-col gap-2" data-testid="time-week">
    <ol class="grid grid-cols-7 gap-1">
        {#each days as day (day.date)}
            <li
                class="flex flex-col items-center rounded-field px-1 py-1 text-xs {day.date === today ? 'bg-base-300 font-semibold' : 'text-base-content/70'}"
                aria-current={day.date === today ? "date" : undefined}
                data-testid="time-week-{day.date}"
            >
                <span>{weekdayOf(day.date)}</span>
                <span class="tabular-nums">{durationText(day.seconds)}</span>
            </li>
        {/each}
    </ol>
    <p class="text-xs text-base-content/70" data-testid="time-week-total">
        This week: <span class="font-medium text-base-content tabular-nums">{durationText(total)}</span>
        {#if percent !== null}
            of {target}h
            <progress
                class="progress ml-2 inline-block w-24 align-middle progress-success"
                value={percent}
                max="100"
                aria-label="{percent}% of the week's target"
            ></progress>
        {/if}
    </p>
</section>
