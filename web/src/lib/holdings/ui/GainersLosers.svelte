<!-- Top gainers / losers (WP-10): two compact lists (≤5 each) of symbol,
     gain %, gain $ — green/red per sign. Each symbol carries a daisyUI
     tooltip with the holding's full name; the focusable button makes it
     reachable by tap/keyboard on mobile (same pattern as CommentIndicator),
     and tooltip-right keeps it from clipping at the viewport's left edge.
     Each list holds only holdings with that gain sign, so an empty list is
     hidden individually; the whole component is hidden when fewer than two
     holdings are priced (a single-entry "top 5" is noise, per plans/10).

     A symbol is a filter (plan 47 D7): clicking it charts only that holding
     (again: every holding), and the ones in the filter are highlighted. The
     full name is still its tooltip. -->
<script lang="ts">
    import {toNumber, type Dec} from "#lib/domain/money.js";
    import {signClass} from "#lib/format/sign.js";
    import type {Holding, HoldingsReport} from "#lib/holdings/types.js";
    import {formatGainPct} from "./view";

    let {
        report,
        format,
        selected = new Set<string>(),
        onSelect,
    }: {
        report: HoldingsReport;
        format: (v: Dec) => string;
        /** The holdings filter: these entries are highlighted. */
        selected?: ReadonlySet<string>;
        /** A symbol was clicked: chart only it. */
        onSelect?: (symbol: string) => void;
    } = $props();

    /**
     * Holdings that could actually be ranked — a gain is what this panel sorts by.
     *
     * Not "priced": a net-short row HAS a market value (negative) but its basis is
     * unknowable, so its gain is null and it can appear in neither list. Counting
     * it toward the threshold showed a one-name "Top gainers" panel whenever a
     * short was in scope, which is the degenerate case the threshold exists to
     * suppress.
     */
    const rankableCount = $derived(report.holdings.filter((h) => h.gain !== null).length);
    const visible = $derived(rankableCount >= 2 && (report.topGainers.length > 0 || report.topLosers.length > 0));
</script>

{#snippet list(title: string, entries: Holding[], testid: string)}
    <div class="min-w-0 flex-1" data-testid={testid}>
        <h3 class="mb-1 text-xs font-semibold tracking-wide text-muted uppercase">{title}</h3>
        <ul class="flex flex-col gap-1">
            {#each entries as h (h.symbol)}
                {@const chosen = selected.has(h.symbol)}
                <li class={["-mx-1 flex items-baseline gap-2 rounded px-1 text-sm", chosen && "bg-primary/10"]} data-selected={chosen ? "true" : undefined}>
                    <span class="tooltip tooltip-right before:max-w-64 before:whitespace-normal" data-tip={h.name}>
                        <button
                            type="button"
                            class={["font-medium", onSelect === undefined ? "cursor-help" : "cursor-pointer hover:underline", chosen && "text-primary"]}
                            aria-pressed={onSelect === undefined ? undefined : chosen}
                            onclick={() => onSelect?.(h.symbol)}
                            data-testid="{testid}-{h.symbol}">{h.symbol}</button
                        >
                    </span>
                    <span class={signClass(h.gainPct)}>{formatGainPct(h.gainPct)}</span>
                    {#if h.gain !== null}
                        <span class="ml-auto font-mono text-xs tabular-nums {signClass(toNumber(h.gain))}">{format(h.gain)}</span>
                    {/if}
                </li>
            {/each}
        </ul>
    </div>
{/snippet}

{#if visible}
    <div class="flex flex-col gap-4 sm:flex-row" data-testid="gainers-losers">
        {#if report.topGainers.length > 0}
            {@render list("Top gainers", report.topGainers, "top-gainers")}
        {/if}
        {#if report.topLosers.length > 0}
            {@render list("Top losers", report.topLosers, "top-losers")}
        {/if}
    </div>
{/if}
