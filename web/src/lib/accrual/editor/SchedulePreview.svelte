<script lang="ts">
    // A new schedule's live preview (D-editor): the period table — dates and
    // amounts, the opening stub and closing true-up labelled — its first and
    // last dates, and the rule text exactly as it will be written. The engine
    // built all of it (`POST /api/recognition/preview`); this only lays it out.
    import {fmt} from "#lib/format/amounts.js";
    import {add, neg, type Dec} from "#lib/domain/money.js";
    import {journal} from "#lib/stores/journal.svelte.js";
    import {dayShort} from "../model";
    import type {Schedule} from "../types";

    let {schedule}: {schedule: Schedule} = $props();

    const commodity = $derived(schedule.spec.lines[0]?.total.commodity ?? "");
    const first = $derived(schedule.periods[0]?.date ?? null);
    const last = $derived(schedule.periods[schedule.periods.length - 1]?.date ?? null);
    /** What a period recognizes, as a positive figure: the source lines' amounts, negated for deferred revenue. */
    function shown(amounts: readonly Dec[]): string {
        const total = amounts.reduce((sum, amount) => add(sum, amount), {m: 0n, p: 0} as Dec);
        return fmt(commodity, total.m < 0n ? neg(total) : total, journal.reportStyles);
    }
    const PART_LABEL = {opening: "stub", closing: "true-up", regular: ""} as const;
</script>

<div class="flex flex-col gap-2 text-sm" data-testid="accrual-schedule-preview">
    <p class="text-xs text-base-content/60">
        {schedule.periods.length}
        {schedule.periods.length === 1 ? "entry" : "entries"}
        {#if first !== null && last !== null}· {dayShort(first)} {first.slice(0, 4)} → {dayShort(last)} {last.slice(0, 4)}{/if}
        · renews {schedule.renewalDate}
    </p>
    <div class="max-h-48 overflow-y-auto rounded border border-base-content/10">
        <table class="table table-xs">
            <tbody>
                {#each schedule.periods as period (period.date)}
                    <tr data-testid="accrual-period">
                        <td class="font-mono">{period.date}</td>
                        <td class="text-xs text-base-content/60">{PART_LABEL[period.part]}</td>
                        {#if period.status !== null}<td class="text-xs text-base-content/60">{period.status}</td>{/if}
                        <td class="text-right tabular-nums">{shown(period.amounts)}</td>
                    </tr>
                {/each}
            </tbody>
        </table>
    </div>
    <details class="text-xs">
        <summary class="cursor-pointer text-base-content/60">Rule text</summary>
        {#each schedule.rules as rule (rule.period + rule.role)}
            <pre class="mt-1 overflow-x-auto rounded bg-base-200 p-2 font-mono text-xs" data-testid="accrual-rule-text">{rule.text}</pre>
        {/each}
    </details>
</div>
