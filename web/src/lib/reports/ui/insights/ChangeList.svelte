<!-- Biggest expense / revenue changes (Boxes 7 & 9): the top leaf accounts ranked
     by how much money the category moved. `goodWhenUp` colours the direction (up
     is green for revenue, red for expenses).

     Categories with no previous-period activity are NOT changes and the engine
     omits them entirely — otherwise they crowd out every real comparison. When
     the list is empty we distinguish "the previous period has no history to
     compare against" from "nothing moved much". -->
<script lang="ts">
    import type {AmountStyle} from "$lib/domain/types";
    import {NEUTRAL_CLASS, sentimentClass} from "$lib/format/sign";
    import type {ChangeRow} from "$lib/reports/insightsTypes";
    import {fmt} from "./format";

    let {
        title,
        rows,
        base,
        styles,
        goodWhenUp,
        hasPrevious,
        testid,
    }: {
        title: string;
        rows: ChangeRow[];
        base: string;
        styles: ReadonlyMap<string, AmountStyle>;
        goodWhenUp: boolean;
        /** Whether the previous period had ANY activity in this category. */
        hasPrevious: boolean;
        testid?: string;
    } = $props();

    function leaf(account: string): string {
        const cut = account.lastIndexOf(":");
        return cut === -1 ? account : account.slice(cut + 1);
    }

    function badge(row: ChangeRow): {text: string; klass: string} {
        if (row.kind === "ended") return {text: "ended", klass: NEUTRAL_CLASS};
        const pct = row.pct ?? 0;
        const arrow = pct > 0 ? "▲" : pct < 0 ? "▼" : "";
        // One decimal, matching `deltaLine`'s "(12.3%)" directly above these
        // rows — this badge alone rounded to whole percents.
        return {text: `${arrow} ${Math.abs(pct).toFixed(1)}%`, klass: sentimentClass(pct, goodWhenUp)};
    }
</script>

<div class="card border border-base-content/5 bg-base-200 shadow-sm" data-testid={testid}>
    <div class="card-body gap-2 p-4">
        <div class="text-xs font-semibold tracking-wide text-base-content/60 uppercase">{title}</div>
        {#if rows.length === 0}
            <div class="text-sm text-base-content/50">
                {hasPrevious ? "No notable changes" : "Not enough history — the previous period has nothing to compare against."}
            </div>
        {:else}
            <ul class="flex flex-col gap-1.5">
                {#each rows as row (row.account)}
                    {@const b = badge(row)}
                    <li class="flex items-center justify-between gap-3 text-sm">
                        <span class="truncate" title={row.account}>{leaf(row.account)}</span>
                        <span class="flex items-center gap-2 whitespace-nowrap">
                            <span class="font-mono tabular-nums">{fmt(base, row.current, styles)}</span>
                            <!-- w-20, not w-16: the badge gained a decimal place. -->
                            <span class="{b.klass} w-20 text-right font-medium">{b.text}</span>
                        </span>
                    </li>
                {/each}
            </ul>
        {/if}
    </div>
</div>
