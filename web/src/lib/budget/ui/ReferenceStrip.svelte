<!-- What this account actually did, period by period — the figures beside the
     amount box.

     Setting a budget number without seeing the history is guesswork, which is
     the whole reason this exists: "you spent $612, $548 and $701 on groceries
     over the last three months, and $389 so far this month" is the sentence a
     monthly grocery goal should be typed against.

     Two things the engine decides and this only renders. The totals are
     subaccount-INCLUSIVE, so they are comparable to the goal being set (the
     budget report aggregates a parent's goal from its children). And the signs
     are already oriented — an income account's actuals arrive positive, exactly
     as its goal is typed — so nothing here negates anything. -->
<script lang="ts">
    import type {AccountReference} from "$lib/budget/types";
    import type {MixedAmount} from "$lib/domain/money";
    import type {AmountStyle} from "$lib/domain/types";
    import {formatTotals} from "$lib/journal/rowModel";
    import type {DataView} from "$lib/stores/loadState";

    let {
        view,
        reference,
        styles,
    }: {
        view: DataView;
        /** The held reference, or null before the first load / on a failure. */
        reference: AccountReference | null;
        styles: ReadonlyMap<string, AmountStyle>;
    } = $props();

    /** One MixedAmount → a compact string; an empty bag reads as a dash, not "0". */
    function fmt(total: MixedAmount): string {
        const parts = formatTotals(total, styles).map((line) => line.text);
        return parts.length > 0 ? parts.join(", ") : "—";
    }

    const periods = $derived(reference?.periods ?? []);

    /**
     * The average, when there is one to show.
     *
     * `averagedPeriods === 0` is "no complete period yet", which is a different
     * fact from an average of zero — printing `$0.00` for it would be a confident
     * answer to a question nobody can answer.
     */
    const average = $derived.by(() => {
        if (reference === null || reference.averagedPeriods === 0) return null;
        return {text: fmt(reference.average), over: reference.averagedPeriods};
    });
</script>

<div class="rounded-box bg-base-200 px-3 py-2" data-testid="reference-strip">
    <div class="mb-1.5 flex items-baseline justify-between gap-2">
        <span class="text-xs font-medium text-base-content/70">Recent activity</span>
        {#if reference !== null}
            <span class="truncate text-xs text-base-content/50">{reference.account}</span>
        {/if}
    </div>

    {#if view === "loading"}
        <div class="flex h-12 items-center justify-center">
            <span class="loading loading-sm loading-spinner" aria-label="Loading recent activity"></span>
        </div>
    {:else if view === "error"}
        <!-- Deliberately quiet. This is a reference figure, not the edit: losing
             it must not stop someone setting a budget they already know they
             want. -->
        <p class="py-2 text-xs text-base-content/50">Couldn't load recent activity for this account.</p>
    {:else if periods.length === 0}
        <p class="py-2 text-xs text-base-content/50">No activity in this account yet.</p>
    {:else}
        <dl class="flex flex-wrap items-start gap-x-6 gap-y-2">
            {#each periods as period (period.key)}
                <div class="flex flex-col">
                    <dt class="text-xs whitespace-nowrap text-base-content/60">
                        {period.label}
                        <!-- A period still running is labelled, never silently
                             shown as a whole one: a third of a month next to
                             four full ones is a misleading comparison unless it
                             says so. -->
                        {#if !period.complete}<span class="text-base-content/40">so far</span>{/if}
                    </dt>
                    <dd class="font-mono text-sm tabular-nums" title={`${period.start} – ${period.end}`}>{fmt(period.total)}</dd>
                </div>
            {/each}

            {#if average !== null}
                <!-- Set off by a rule and given the emphasis, because it is the
                     number most people will actually budget from. It sits after
                     the periods it summarises rather than before them, so the row
                     still reads left-to-right as history → conclusion. -->
                <div class="flex flex-col border-l border-base-content/15 pl-6" data-testid="reference-average">
                    <dt class="text-xs whitespace-nowrap text-base-content/60">
                        Average
                        <span class="text-base-content/40">of {average.over}</span>
                    </dt>
                    <dd
                        class="font-mono text-sm font-semibold tabular-nums"
                        title="The mean of the {average.over} complete {reference?.interval ?? 'period'} period{average.over === 1
                            ? ''
                            : 's'}; the period still running is excluded"
                    >
                        {average.text}
                    </dd>
                </div>
            {/if}
        </dl>
    {/if}
</div>
