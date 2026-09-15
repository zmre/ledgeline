<!-- Balances view: the cash half of the journal tab's insights box.
     Cash on hand, short-term liabilities and net cash, then the leaf accounts
     behind them with the date each was last touched.

     Three things about it are deliberate and worth knowing before changing it:

     1. It reads the WHOLE journal, not the filtered view, and always as of
        today. A balance is cumulative, so the journal's 90-day default window
        would understate every figure; and the question this box answers is
        "what do I have right now", which must not move when you scrub the date
        filter. The account filter is ignored for the same reason — filtering
        the journal to `expenses:` should not blank the cash panel.
     2. One commodity at a time, never summed across them. Nothing here needs a
        `P` directive, so a stale or missing price cannot make this read zero.
     3. The account lists SCROLL inside a fixed height. The journal page is
        `height: calc(100dvh - 7rem)` with only the transaction table scrolling,
        so every pixel this box takes comes straight out of the table.

     Balance maths lives in $lib/reports/cashBalances (pure, ported to Rust
     later); this file is display only. -->
<script lang="ts">
    import {PieChart, Tooltip} from "layerchart";
    import type {AccountDecl} from "$lib/domain/accountTypes";
    import {formatAmount, toNumber, type Dec} from "$lib/domain/money";
    import type {Transaction} from "$lib/domain/types";
    import {colorAt, OTHER_COLOR, OTHER_LABEL} from "$lib/format/palette";
    import {signClass} from "$lib/format/sign";
    import {accountBalances, balanceCommodities, guessedLongTerm, summarize, visibleRows, type BalanceRow} from "$lib/reports/cashBalances";
    import {today} from "$lib/reports/periods";
    import {settings} from "$lib/stores/settings.svelte";
    import AccountBalanceRow from "./AccountBalanceRow.svelte";
    import {styleFor} from "./series";

    // allTxns: the UNFILTERED journal. Named for what it must be, because
    // passing the filtered array would silently produce wrong balances rather
    // than an error.
    let {allTxns, decls}: {allTxns: Transaction[]; decls: readonly AccountDecl[]} = $props();

    /** At most this many pie slices, the last of which is "(other)" — same cap the activity chart uses. */
    const MAX_SLICES = 6;

    const asOf = today();
    // Memoized in cashBalances on (txns, decls, asOf) identity, so the
    // `negative-cash` check rule and this view share one whole-journal pass.
    const balances = $derived(accountBalances(allTxns, decls, asOf));
    // Same memoized pass as `balances`, so naming the guesses costs nothing.
    const guessed = $derived(guessedLongTerm(allTxns, decls, asOf));
    const commodities = $derived(balanceCommodities(balances));

    let chosenCommodity = $state<string | null>(null);
    const commodity = $derived(chosenCommodity !== null && commodities.includes(chosenCommodity) ? chosenCommodity : (commodities[0] ?? "$"));
    const style = $derived(styleFor(allTxns, commodity));

    const summary = $derived(summarize(balances, commodity));
    const cash = $derived(visibleRows(summary.cash, settings.hideZeroBalances));
    const liabilities = $derived(visibleRows(summary.liabilities, settings.hideZeroBalances));
    const hiddenCount = $derived(summary.cash.length + summary.liabilities.length - cash.length - liabilities.length);

    const fmt = (qty: Dec): string => formatAmount({commodity, qty, style});

    interface Tile {
        label: string;
        value: string;
        valueClass: string;
        testid: string;
    }

    // "Short-term" is accurate however the journal is tagged: an account with no
    // `bsterm:` defaults to current, exactly as the balance sheet defaults it,
    // so an untagged journal's figures cover everything it has.
    const tiles: Tile[] = $derived([
        {label: "Cash on hand", value: fmt(summary.totalCash), valueClass: signClass(toNumber(summary.totalCash)), testid: "balances-cash"},
        {
            label: "Short-term liabilities",
            value: fmt(summary.totalLiabilities),
            // Owing money is the normal state of a credit card, not an error, so
            // this tile is muted rather than red. Only NET going negative, and an
            // individual overdrawn cash account, earn the alarm colour.
            valueClass: "",
            testid: "balances-liabilities",
        },
        {label: "Net cash", value: fmt(summary.net), valueClass: signClass(toNumber(summary.net)), testid: "balances-net"},
    ]);

    interface Slice {
        account: string;
        value: number;
        formatted: string;
        color: string;
    }

    /**
     * Cash pie: positive balances only, biggest first, tail folded into
     * "(other)". Liabilities are excluded on purpose — the ask is "where is my
     * cash", and a pie divides a whole by AREA, which a debt has none of.
     * Negative (overdrawn) cash is left out for the same reason and is already
     * called out in red in the list.
     */
    const slices: Slice[] = $derived.by(() => {
        const positive = cash.filter((row) => row.qty.m > 0n);
        const keep = positive.length > MAX_SLICES ? MAX_SLICES - 1 : positive.length;
        const head = positive.slice(0, keep).map((row, i) => ({
            account: row.account,
            value: toNumber(row.qty),
            formatted: fmt(row.qty),
            color: colorAt(i),
        }));
        const tail = positive.slice(keep);
        if (tail.length === 0) return head;
        const total = tail.reduce((sum, row) => sum + toNumber(row.qty), 0);
        return [...head, {account: OTHER_LABEL, value: total, formatted: `${tail.length} more`, color: OTHER_COLOR}];
    });
    /** Slice colour by account, so a list row's dot matches its wedge. */
    const colorOf = $derived(new Map(slices.map((slice) => [slice.account, slice.color])));
</script>

{#snippet list(title: string, rows: BalanceRow[], negativeIsWrong: boolean)}
    <section>
        <h3 class="sticky top-0 z-10 bg-base-200 py-1 text-xs font-semibold tracking-wide text-base-content/60 uppercase">{title}</h3>
        {#if rows.length === 0}
            <p class="px-1 py-1 text-xs text-base-content/50">None.</p>
        {:else}
            <ul class="divide-y divide-base-300/60">
                {#each rows as entry (entry.account)}
                    <AccountBalanceRow {entry} formatted={fmt(entry.qty)} {negativeIsWrong} dotColor={colorOf.get(entry.account)} />
                {/each}
            </ul>
        {/if}
    </section>
{/snippet}

<div class="flex flex-col gap-3" data-testid="balances-view">
    <div class="flex flex-wrap items-center gap-3">
        {#if commodities.length > 1}
            <select class="select w-24 select-xs" value={commodity} onchange={(e) => (chosenCommodity = e.currentTarget.value)} aria-label="Balance currency">
                {#each commodities as c (c)}
                    <option value={c}>{c}</option>
                {/each}
            </select>
        {/if}
        <label class="flex cursor-pointer items-center gap-2 text-xs">
            <input
                type="checkbox"
                class="checkbox checkbox-xs"
                checked={settings.hideZeroBalances}
                onchange={(e) => (settings.hideZeroBalances = e.currentTarget.checked)}
            />
            Hide zero balances
            {#if hiddenCount > 0}<span class="text-base-content/50">({hiddenCount})</span>{/if}
        </label>
        <span class="ml-auto text-xs text-base-content/50">as of {asOf}</span>
    </div>

    <!-- Same `stats` treatment as the activity tab's big numbers so the two
         views read as siblings, but horizontal at every width: three money
         figures fit across a phone, and stacking them cost ~140px of the
         table's height on exactly the screens with least to spare. -->
    <div class="stats w-full stats-horizontal bg-base-200 shadow-none">
        {#each tiles as tile (tile.label)}
            <div class="stat px-3 py-2" data-testid={tile.testid}>
                <div class="stat-title text-xs">{tile.label}</div>
                <div class="stat-value font-mono text-lg tabular-nums md:text-2xl {tile.valueClass}">{tile.value}</div>
            </div>
        {/each}
    </div>

    {#if summary.cash.length === 0 && summary.liabilities.length === 0}
        <p class="py-6 text-center text-sm text-base-content/60">
            No cash or short-term liability accounts hold {commodity}. Declare your bank and card accounts with
            <code class="text-xs">type: C</code>
            and <code class="text-xs">type: L</code> to see them here.
        </p>
    {:else}
        <div class="flex gap-4">
            {#if slices.length > 0}
                <div class="hidden h-48 w-56 shrink-0 lg:block" data-testid="balances-pie">
                    <PieChart
                        data={slices}
                        key="account"
                        label="account"
                        value={(d) => d.value}
                        cRange={slices.map((d) => d.color)}
                        padAngle={0.02}
                        legend={false}
                    >
                        {#snippet tooltip()}
                            <Tooltip.Root>
                                {#snippet children({data})}
                                    {@const d = data as Slice}
                                    <div class="flex items-center gap-2 text-xs">
                                        <span class="inline-block h-2 w-2 rounded-full" style="background:{d.color}"></span>
                                        <span class="text-base-content/70">{d.account}</span>
                                        <span class="font-semibold">{d.formatted}</span>
                                    </div>
                                {/snippet}
                            </Tooltip.Root>
                        {/snippet}
                    </PieChart>
                </div>
            {/if}
            <!-- The scroll container. Bounded in vh so it shrinks with the
                 window rather than pushing the transaction table off screen. -->
            <div class="max-h-48 min-w-0 flex-1 overflow-y-auto pr-1 sm:max-h-56">
                {@render list("Cash", cash, true)}
                {@render list("Liabilities", liabilities, false)}
            </div>
        </div>
    {/if}

    <!-- The guess, said out loud. Accounts kept out by their own `bsterm:` tag
         are NOT listed here — the owner wrote that tag and knows. These were
         removed because a word in the name looked long-term, which is a guess,
         and a guess that quietly takes a debt off the reader's screen is the one
         failure this heuristic can cause. Naming them makes it correctable. -->
    {#if guessed.length > 0}
        <p class="px-1 text-xs text-base-content/50" data-testid="balances-guessed">
            Treated as long-term from the name, so not counted above: {guessed.join(", ")}. Tag an account
            <code>bsterm: current</code> to include it.
        </p>
    {/if}
</div>
