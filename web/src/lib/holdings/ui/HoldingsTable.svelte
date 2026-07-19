<!-- Holdings details table (WP-10): one row per held stock. Columns: Name,
     Symbol, Shares, Basis, Price, Price date, Market value, Gain ($), Gain %.
     Default order is the engine's (market value desc, unpriced last);
     clicking a header sorts client-side via sortHoldings — numeric columns
     start desc, text/date columns asc, second click flips, nulls always
     last. The Basis cell carries a daisyUI tooltip with the date the current
     position was opened ("First basis YYYY-MM-DD"), reachable by tap/keyboard
     via a focusable button (same pattern as CommentIndicator); tooltip-left
     because top/bottom tooltips on the first/last rows can still clip
     against the wrapper's vertical overflow edges (overflow-x non-visible
     forces overflow-y to auto per CSS). The wrapper scrolls horizontally
     only, for small screens — vertical scrolling belongs to the page.
     Right-aligned numerics via the exact domain formatters (2dp display
     cap), em-dash for null cells, negatives in text-error (gain cells
     additionally show positives in text-success), "inferred" badge when the
     price came from a cost annotation instead of a P directive. A tfoot
     totals row (always below the body, whatever the sort) shows the ENGINE's
     totals — never recomputed here, so the honest-totals rule holds: basis
     is an em-dash when any holding is tainted or unpriced, matching the stat
     tiles. Only Basis and Market value get totals. -->
<script lang="ts">
    import {toNumber, type Dec} from "$lib/domain/money";
    import type {GainPeriod, Holding, HoldingsReport} from "$lib/holdings/types";
    import {gainWindowSuffix} from "./gainPeriod";
    import {EM_DASH, formatGainPct, formatShares, sortHoldings, type SortKey} from "./view";

    let {holdings, totals, format, gainPeriod = "all"}: {holdings: Holding[]; totals: HoldingsReport["totals"]; format: (v: Dec) => string; gainPeriod?: GainPeriod} =
        $props();

    // Window tag on the Gain header so a YTD/12mo gain number isn't read as all-time.
    const gainHeader = $derived(`Gain${gainWindowSuffix(gainPeriod)}`);

    /** Columns whose first click sorts desc (big numbers first); the rest start asc. */
    const DESC_FIRST: ReadonlySet<SortKey> = new Set(["shares", "basis", "price", "marketValue", "gain", "gainPct"]);

    let sort = $state<{key: SortKey; dir: "asc" | "desc"} | null>(null); // null = engine default order (market value desc)
    const rows = $derived(sort === null ? holdings : sortHoldings(holdings, sort.key, sort.dir));

    function toggleSort(key: SortKey): void {
        if (sort !== null && sort.key === key) sort = {key, dir: sort.dir === "asc" ? "desc" : "asc"};
        else sort = {key, dir: DESC_FIRST.has(key) ? "desc" : "asc"};
    }

    const ariaSort = (key: SortKey): "ascending" | "descending" | undefined =>
        sort !== null && sort.key === key ? (sort.dir === "asc" ? "ascending" : "descending") : undefined;
</script>

{#snippet sortButton(key: SortKey, label: string)}
    <button type="button" class="cursor-pointer whitespace-nowrap" onclick={() => toggleSort(key)}>
        {label}{#if sort !== null && sort.key === key}<span aria-hidden="true">{sort.dir === "asc" ? " ▲" : " ▼"}</span>{/if}
    </button>
{/snippet}

{#snippet money(v: Dec | null)}
    {#if v === null}
        <span class="text-base-content/40">{EM_DASH}</span>
    {:else}
        <span class={toNumber(v) < 0 ? "text-error" : ""}>{format(v)}</span>
    {/if}
{/snippet}

{#snippet gainMoney(v: Dec | null)}
    {#if v === null}
        <span class="text-base-content/40">{EM_DASH}</span>
    {:else}
        <span class={toNumber(v) < 0 ? "text-error" : toNumber(v) > 0 ? "text-success" : ""}>{format(v)}</span>
    {/if}
{/snippet}

<div class="border-base-content/10 rounded-box overflow-x-auto border">
    <table class="table-zebra table-sm table" data-testid="holdings-table">
        <thead>
            <tr>
                <th aria-sort={ariaSort("name")}>{@render sortButton("name", "Name")}</th>
                <th aria-sort={ariaSort("symbol")}>{@render sortButton("symbol", "Symbol")}</th>
                <td class="text-right" aria-sort={ariaSort("shares")}>{@render sortButton("shares", "Shares")}</td>
                <td class="text-right" aria-sort={ariaSort("basis")}>{@render sortButton("basis", "Basis")}</td>
                <td class="text-right" aria-sort={ariaSort("price")}>{@render sortButton("price", "Price")}</td>
                <td aria-sort={ariaSort("priceDate")}>{@render sortButton("priceDate", "Price date")}</td>
                <td class="text-right" aria-sort={ariaSort("marketValue")}>{@render sortButton("marketValue", "Market value")}</td>
                <td class="text-right" aria-sort={ariaSort("gain")}>{@render sortButton("gain", gainHeader)}</td>
                <td class="text-right" aria-sort={ariaSort("gainPct")}>{@render sortButton("gainPct", "Gain %")}</td>
            </tr>
        </thead>
        <tbody>
            {#each rows as h (h.symbol)}
                <tr data-testid="holding-{h.symbol}">
                    <th class="font-normal whitespace-nowrap" title={h.accounts.join(", ")}>{h.name}</th>
                    <th class="font-medium">{h.symbol}</th>
                    <td class="text-right font-mono whitespace-nowrap tabular-nums" data-testid="shares-{h.symbol}">{formatShares(h.shares)}</td>
                    <td class="text-right font-mono whitespace-nowrap tabular-nums">
                        {#if h.firstBasisDate === null}
                            {@render money(h.basis)}
                        {:else}
                            <span class="tooltip tooltip-left" data-tip="First basis {h.firstBasisDate}">
                                <button type="button" class="cursor-help">{@render money(h.basis)}</button>
                            </span>
                        {/if}
                    </td>
                    <td class="text-right font-mono whitespace-nowrap tabular-nums">
                        {#if h.price === null}
                            <span class="text-base-content/40">{EM_DASH}</span>
                        {:else}
                            {format(h.price.qty)}
                            {#if h.price.source === "cost"}
                                <span class="badge badge-ghost badge-xs align-middle" title="No P price directive — inferred from the latest cost annotation"
                                    >inferred</span
                                >
                            {/if}
                        {/if}
                    </td>
                    <td class="whitespace-nowrap">
                        {#if h.price === null}
                            <span class="text-base-content/40">{EM_DASH}</span>
                        {:else}
                            {h.price.date}
                        {/if}
                    </td>
                    <td class="text-right font-mono whitespace-nowrap tabular-nums">{@render money(h.marketValue)}</td>
                    <td class="text-right font-mono whitespace-nowrap tabular-nums">{@render gainMoney(h.gain)}</td>
                    <td class="text-right font-mono whitespace-nowrap tabular-nums">
                        <span class={h.gainPct === null ? "text-base-content/40" : h.gainPct < 0 ? "text-error" : h.gainPct > 0 ? "text-success" : ""}
                            >{formatGainPct(h.gainPct)}</span
                        >
                    </td>
                </tr>
            {/each}
        </tbody>
        <tfoot>
            <tr class="border-base-content/20 bg-base-200 text-base-content border-t text-sm font-bold" data-testid="holdings-totals">
                <th class="font-bold whitespace-nowrap">Total ({holdings.length} holdings):</th>
                <th></th>
                <td></td>
                <td class="text-right font-mono whitespace-nowrap tabular-nums">{@render money(totals.basis)}</td>
                <td></td>
                <td></td>
                <td class="text-right font-mono whitespace-nowrap tabular-nums">{@render money(totals.marketValue)}</td>
                <td></td>
                <td></td>
            </tr>
        </tfoot>
    </table>
</div>
