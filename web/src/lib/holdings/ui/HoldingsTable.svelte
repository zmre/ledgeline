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
    import {signClass} from "$lib/format/sign";
    import type {GainPeriod, Holding, HoldingsReport} from "$lib/holdings/types";
    import {openJournal} from "$lib/journal/openJournal";
    import {registerKeys} from "$lib/keys/keymap.svelte";
    import {PRIORITY} from "$lib/keys/types";
    import {listCursor} from "$lib/ui/listCursor.svelte";
    import {gainWindowSuffix} from "./gainPeriod";
    import {EM_DASH, formatGainPct, formatShares, sortHoldings, type SortKey} from "./view";

    let {
        holdings,
        totals,
        format,
        gainPeriod = "all",
    }: {holdings: Holding[]; totals: HoldingsReport["totals"]; format: (v: Dec) => string; gainPeriod?: GainPeriod} = $props();

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

    // Keyed on symbol over the SORTED rows, so clicking a column header keeps
    // you on the same holding rather than on whatever slid into that position.
    const cursor = listCursor(
        () => rows,
        (h) => h.symbol
    );

    function move(delta: number): void {
        cursor.move(delta);
        // The page scrolls here, not a container, and every row is mounted (not
        // virtualized) — so `scrollIntoView` is the honest mechanism, unlike in
        // the journal's virtual list.
        document.querySelector(`[data-testid="holding-${CSS.escape(String(cursor.key ?? ""))}"]`)?.scrollIntoView({block: "nearest"});
    }

    // "Holdings", not "Journal". These borrowed the Journal heading while this
    // table was the page's only keyboard surface; plans/14 added a second (the
    // tab strip), and one feature filed under two headings in the help drawer is
    // how a reader stops trusting the drawer.
    registerKeys({
        id: "holdings-table",
        priority: PRIORITY.widget,
        bindings: [
            {keys: "j", label: "Next holding", group: "Holdings", run: () => move(1)},
            {keys: "ArrowDown", label: "Next holding", group: "Holdings", run: () => move(1)},
            {keys: "k", label: "Previous holding", group: "Holdings", run: () => move(-1)},
            {keys: "ArrowUp", label: "Previous holding", group: "Holdings", run: () => move(-1)},
            {keys: "g g", label: "First holding", group: "Holdings", run: () => (cursor.first(), move(0))},
            {keys: "G", label: "Last holding", group: "Holdings", run: () => (cursor.last(), move(0))},
            {keys: "Escape", label: "Clear the cursor", group: "Holdings", run: () => cursor.clear()},
            {
                keys: "Enter",
                label: "Show this holding in the journal",
                group: "Holdings",
                run: () => {
                    const holding = cursor.item;
                    // A holding can span several accounts (the same stock in two
                    // brokerages), so all of them go into the filter.
                    if (holding !== null) void openJournal({accounts: holding.accounts, preset: "all"});
                },
            },
        ],
    });
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
        <span class={signClass(toNumber(v))}>{format(v)}</span>
    {/if}
{/snippet}

<div class="overflow-x-auto rounded-box border border-base-content/10">
    <table class="table table-zebra table-sm" data-testid="holdings-table">
        <thead>
            <!-- Every header is a <th scope="col">, never a <td>: aria-sort is only
                 valid on a columnheader, so on a <td> the sort state was announced to
                 nobody, and the numeric cells below had no column association at all.
                 text-left pins the alignment the old <td> had by default (a bare <th>
                 centers). OtherHoldingsTable is this header's deliberate twin. -->
            <tr>
                <th scope="col" aria-sort={ariaSort("name")}>{@render sortButton("name", "Name")}</th>
                <th scope="col" aria-sort={ariaSort("symbol")}>{@render sortButton("symbol", "Symbol")}</th>
                <th scope="col" class="text-right" aria-sort={ariaSort("shares")}>{@render sortButton("shares", "Shares")}</th>
                <th scope="col" class="text-right" aria-sort={ariaSort("basis")}>{@render sortButton("basis", "Basis")}</th>
                <th scope="col" class="text-right" aria-sort={ariaSort("price")}>{@render sortButton("price", "Price")}</th>
                <th scope="col" class="text-left" aria-sort={ariaSort("priceDate")}>{@render sortButton("priceDate", "Price date")}</th>
                <th scope="col" class="text-right" aria-sort={ariaSort("marketValue")}>{@render sortButton("marketValue", "Market value")}</th>
                <th scope="col" class="text-right" aria-sort={ariaSort("gain")}>{@render sortButton("gain", gainHeader)}</th>
                <th scope="col" class="text-right" aria-sort={ariaSort("gainPct")}>{@render sortButton("gainPct", "Gain %")}</th>
            </tr>
        </thead>
        <tbody>
            {#each rows as h (h.symbol)}
                <tr
                    data-testid="holding-{h.symbol}"
                    class={cursor.key === h.symbol ? "bg-primary/25" : ""}
                    aria-current={cursor.key === h.symbol ? "true" : undefined}
                >
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
                                <span class="badge badge-ghost align-middle badge-xs" title="No P price directive — inferred from the latest cost annotation"
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
                        <span class={h.gainPct === null ? "text-base-content/40" : signClass(h.gainPct)}>{formatGainPct(h.gainPct)}</span>
                    </td>
                </tr>
            {/each}
        </tbody>
        <tfoot>
            <tr class="border-t border-base-content/20 bg-base-200 text-sm font-bold text-base-content" data-testid="holdings-totals">
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
