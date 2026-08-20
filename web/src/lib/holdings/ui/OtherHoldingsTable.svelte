<!-- Other holdings table (plans/14): one row per tracked asset that is neither a
     security nor cash — a house, a van, a partnership interest. Columns: Name,
     Account, Holding, Value, Cost, Change, Change %.

     Deliberately HoldingsTable's twin, down to the sortButton/money snippets,
     the right-aligned `font-mono tabular-nums` numeric cells and the engine
     `tfoot`: the two tabs sit one click apart under one scope bar, so a reader
     moving between them should not have to relearn what a cell means. Default
     order is the engine's (value desc, unpriced last, then by account); clicking
     a header sorts client-side via sortOtherHoldings — numeric columns start
     desc, text columns asc, second click flips, nulls always last.

     What differs, and why:
       - The row key is the ACCOUNT, not a symbol. Rows are flat, one per
         posting-bearing account, and two accounts may share a `name:` tag.
       - "Holding" shows the balance as written ("1 HOUSE") and is BLANK when the
         only commodity is the base — see formatHeldCommodities.
       - There is no Price column. A house has no per-unit quote worth a column;
         its value IS the row.
       - Only Value and Cost get totals, and they are the ENGINE's, never
         recomputed here, so the honest-totals rule holds: an unpriced row
         contributes to nothing and raises the warning shown above this table
         instead. -->
<script lang="ts">
    import {toNumber, type Dec} from "$lib/domain/money";
    import {signClass} from "$lib/format/sign";
    import type {GainPeriod, OtherHolding, OtherHoldingsReport} from "$lib/holdings/types";
    import {openJournal} from "$lib/journal/openJournal";
    import {registerKeys} from "$lib/keys/keymap.svelte";
    import {PRIORITY} from "$lib/keys/types";
    import {listCursor} from "$lib/ui/listCursor.svelte";
    import {gainWindowSuffix} from "./gainPeriod";
    import {EM_DASH, formatGainPct, formatHeldCommodities, sortOtherHoldings, type OtherSortKey} from "./view";

    let {
        holdings,
        totals,
        base,
        format,
        formatUnits,
        gainPeriod = "all",
    }: {
        holdings: OtherHolding[];
        totals: OtherHoldingsReport["totals"];
        /** The report's base commodity — decides which Holding cells stay blank. */
        base: string;
        /** Base-commodity money, exact, 2dp-capped (the page's shared formatter). */
        format: (v: Dec) => string;
        /** One held commodity as the journal writes it, e.g. "1 HOUSE". */
        formatUnits: (commodity: string, qty: Dec) => string;
        gainPeriod?: GainPeriod;
    } = $props();

    // Window tag on the Change header so a YTD/12mo figure isn't read as all-time.
    const changeHeader = $derived(`Change${gainWindowSuffix(gainPeriod)}`);

    /** Columns whose first click sorts desc (big numbers first); the rest start asc. */
    const DESC_FIRST: ReadonlySet<OtherSortKey> = new Set(["value", "cost", "change", "changePct"]);

    let sort = $state<{key: OtherSortKey; dir: "asc" | "desc"} | null>(null); // null = engine default order (value desc)
    const rows = $derived(sort === null ? holdings : sortOtherHoldings(holdings, sort.key, sort.dir));

    function toggleSort(key: OtherSortKey): void {
        if (sort !== null && sort.key === key) sort = {key, dir: sort.dir === "asc" ? "desc" : "asc"};
        else sort = {key, dir: DESC_FIRST.has(key) ? "desc" : "asc"};
    }

    const ariaSort = (key: OtherSortKey): "ascending" | "descending" | undefined =>
        sort !== null && sort.key === key ? (sort.dir === "asc" ? "ascending" : "descending") : undefined;

    // Keyed on account over the SORTED rows, so clicking a column header keeps
    // you on the same asset rather than on whatever slid into that position.
    const cursor = listCursor(
        () => rows,
        (h) => h.account
    );

    function move(delta: number): void {
        cursor.move(delta);
        // The page scrolls here, not a container, and every row is mounted (not
        // virtualized) — so `scrollIntoView` is the honest mechanism.
        document.querySelector(`[data-testid="other-holding-${CSS.escape(String(cursor.key ?? ""))}"]`)?.scrollIntoView({block: "nearest"});
    }

    // Its own layer id: the two tables never mount together (the page branches on
    // the tab), but a shared id would make the survivor of a re-registration
    // ambiguous, and `id` is the debug handle that says which one is live.
    registerKeys({
        id: "other-holdings-table",
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
                    // One account per row here, unlike a stock that can span two brokerages.
                    if (holding !== null) void openJournal({accounts: [holding.account], preset: "all"});
                },
            },
        ],
    });
</script>

{#snippet sortButton(key: OtherSortKey, label: string)}
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

{#snippet changeMoney(v: Dec | null)}
    {#if v === null}
        <span class="text-base-content/40">{EM_DASH}</span>
    {:else}
        <span class={signClass(toNumber(v))}>{format(v)}</span>
    {/if}
{/snippet}

<div class="border-base-content/10 rounded-box overflow-x-auto border">
    <table class="table-zebra table-sm table" data-testid="other-holdings-table">
        <thead>
            <!-- Every header is a <th scope="col">, never a <td>: aria-sort is only
                 valid on a columnheader, so on a <td> the sort state was announced to
                 nobody, and the numeric cells below had no column association at all.
                 HoldingsTable is this header's deliberate twin — keep them in lockstep. -->
            <tr>
                <th scope="col" aria-sort={ariaSort("name")}>{@render sortButton("name", "Name")}</th>
                <th scope="col" aria-sort={ariaSort("account")}>{@render sortButton("account", "Account")}</th>
                <th scope="col" class="text-right">Holding</th>
                <th scope="col" class="text-right" aria-sort={ariaSort("value")}>{@render sortButton("value", "Value")}</th>
                <th scope="col" class="text-right" aria-sort={ariaSort("cost")}>{@render sortButton("cost", "Cost")}</th>
                <th scope="col" class="text-right" aria-sort={ariaSort("change")}>{@render sortButton("change", changeHeader)}</th>
                <th scope="col" class="text-right" aria-sort={ariaSort("changePct")}>{@render sortButton("changePct", "Change %")}</th>
            </tr>
        </thead>
        <tbody>
            {#each rows as h (h.account)}
                <tr
                    data-testid="other-holding-{h.account}"
                    class={cursor.key === h.account ? "bg-primary/25" : ""}
                    aria-current={cursor.key === h.account ? "true" : undefined}
                >
                    <th class="font-normal whitespace-nowrap" title={h.account}>{h.name}</th>
                    <th class="text-base-content/70 font-normal whitespace-nowrap">{h.account}</th>
                    <td class="text-right font-mono whitespace-nowrap tabular-nums" data-testid="held-{h.account}">
                        {formatHeldCommodities(h.commodities, base, formatUnits)}
                    </td>
                    <td class="text-right font-mono whitespace-nowrap tabular-nums">{@render money(h.value)}</td>
                    <td class="text-right font-mono whitespace-nowrap tabular-nums">{@render money(h.cost)}</td>
                    <td class="text-right font-mono whitespace-nowrap tabular-nums">{@render changeMoney(h.change)}</td>
                    <td class="text-right font-mono whitespace-nowrap tabular-nums">
                        <span class={h.changePct === null ? "text-base-content/40" : signClass(h.changePct)}>{formatGainPct(h.changePct)}</span>
                    </td>
                </tr>
            {/each}
        </tbody>
        <tfoot>
            <tr class="border-base-content/20 bg-base-200 text-base-content border-t text-sm font-bold" data-testid="other-holdings-totals">
                <th class="font-bold whitespace-nowrap">Total ({holdings.length} holdings):</th>
                <th></th>
                <td></td>
                <td class="text-right font-mono whitespace-nowrap tabular-nums">{@render money(totals.value)}</td>
                <td class="text-right font-mono whitespace-nowrap tabular-nums">{@render money(totals.cost)}</td>
                <td></td>
                <td></td>
            </tr>
        </tfoot>
    </table>
</div>
