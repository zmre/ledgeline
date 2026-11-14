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
     3. The account lists SCROLL inside a fixed height. From `md` the journal
        page is one window tall with only the transaction table scrolling,
        so every pixel this box takes comes straight out of the table.

     4. WHILE AN ACCOUNT IS PINNED, THE PIE IS ITS BALANCE LINE (plan 36
        amendment A1). The pin decides it — clicking a row pins, ✕ on the pin
        chip unpins — and there is no separate toggle. Unlike the list, the
        line follows the journal's DATE range: it describes the rows on screen.
        It takes the pie's place and its `lg` breakpoint, a little wider. While
        a new answer loads, the last line for the same account stays up rather
        than flashing the pie.

     Balance maths lives in the engine (`crates/ledgeline-core/src/view/cash.rs`,
     served at `/api/balances/cash`); this file is display only. -->
<script lang="ts">
    import {PieChart, Tooltip} from "layerchart";
    import {formatAmount, toNumber, type Dec} from "#lib/domain/money.js";
    import {DEFAULT_AMOUNT_STYLE, ZERO} from "#lib/format/amounts.js";
    import {chartColors} from "#lib/format/chartColors.svelte.js";
    import {OTHER_LABEL} from "#lib/format/palette.js";
    import {signClass} from "#lib/format/sign.js";
    import type {BalanceRow, BalanceSeries, BalanceSummary, CashBalancesView} from "#lib/journalView/types.js";
    import {filters} from "#lib/stores/filters.svelte.js";
    import {settings} from "#lib/stores/settings.svelte.js";
    import AccountBalanceRow from "./AccountBalanceRow.svelte";
    import BalanceTrend from "./BalanceTrend.svelte";

    // view: the engine's answer for the WHOLE journal as of today, every
    // commodity at once (`/api/balances/cash`), so switching currency is local.
    // trend: the pinned account's balance line (`/api/journal/running/series`),
    // the newest answer held — possibly for an older query while a new one
    // loads; null before the first. trendError: why the newest load failed.
    let {
        view,
        trend = null,
        trendError = null,
    }: {
        view: CashBalancesView;
        trend?: BalanceSeries | null;
        trendError?: Error | null;
    } = $props();

    const pin = $derived(filters.value.pin ?? null);
    /** The held line, but only when it is about the pinned account: another account's line must never stand in for it. */
    const pinnedTrend = $derived(pin !== null && trend !== null && trend.account === pin.account ? trend : null);

    const asOf = $derived(view.asOf);
    const guessed = $derived(view.guessedLongTerm);
    const commodities = $derived(view.commodities);

    /** Rows worth drawing: every row when `hideZero` is false, else only those with a non-zero balance. */
    const visibleRows = (rows: readonly BalanceRow[], hideZero: boolean): BalanceRow[] => (hideZero ? rows.filter((row) => row.qty.m !== 0n) : [...rows]);
    /** A journal holding no cash or liability account at all: no rows, zero totals. */
    const emptySummary = (c: string): BalanceSummary => ({
        commodity: c,
        style: DEFAULT_AMOUNT_STYLE,
        cash: [],
        liabilities: [],
        totalCash: ZERO,
        totalLiabilities: ZERO,
        net: ZERO,
    });

    let chosenCommodity = $state<string | null>(null);
    const commodity = $derived(chosenCommodity !== null && commodities.includes(chosenCommodity) ? chosenCommodity : (commodities[0] ?? "$"));
    const summary = $derived(view.summaries[Math.max(0, commodities.indexOf(commodity))] ?? emptySummary(commodity));
    const style = $derived(summary.style);
    const cash = $derived(visibleRows(summary.cash, settings.hideZeroBalances));
    const liabilities = $derived(visibleRows(summary.liabilities, settings.hideZeroBalances));
    /** No cash or short-term liability account holds this commodity at all. */
    const nothingHeld = $derived(summary.cash.length === 0 && summary.liabilities.length === 0);
    const hiddenCount = $derived(summary.cash.length + summary.liabilities.length - cash.length - liabilities.length);

    const fmt = (qty: Dec): string => formatAmount({commodity, qty, style});

    /**
     * Filter the journal to `account` and pin its running balance; the date
     * range stays. The commodity on screen comes with it when there is a
     * choice, so picking EUR here shows the EUR column there.
     */
    function openAccount(account: string): void {
        filters.focusAccount(account, commodities.length > 1 ? commodity : null);
    }

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
     * Cash pie: positive balances only, biggest first, one slice per account
     * until the palette's slots run out, and only the tail past the last slot
     * folded into "(other)". Liabilities are excluded on purpose — the ask is "where is my
     * cash", and a pie divides a whole by AREA, which a debt has none of.
     * Negative (overdrawn) cash is left out for the same reason and is already
     * called out in red in the list.
     */
    const slices: Slice[] = $derived.by(() => {
        const positive = cash.filter((row) => row.qty.m > 0n);
        const head = positive.slice(0, chartColors.categorical.length).map((row, i) => ({
            account: row.account,
            value: toNumber(row.qty),
            formatted: fmt(row.qty),
            color: chartColors.colorAt(i),
        }));
        const tail = positive.slice(head.length);
        if (tail.length === 0) return head;
        const total = tail.reduce((sum, row) => sum + toNumber(row.qty), 0);
        return [...head, {account: OTHER_LABEL, value: total, formatted: `${tail.length} more`, color: chartColors.other}];
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
                    <AccountBalanceRow
                        {entry}
                        formatted={fmt(entry.qty)}
                        {negativeIsWrong}
                        dotColor={pin === null ? colorOf.get(entry.account) : undefined}
                        pinned={pin?.account === entry.account}
                        onOpen={() => openAccount(entry.account)}
                    />
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
         views read as siblings. Side by side from `sm`; below it the three
         figures did not fit across a phone and the row scrolled sideways, so
         they stack, each as one compact label … figure line (a stacked tile
         would cost ~140px of the table's height on exactly the screens with
         least to spare).
         Hidden while an account is pinned: the view is then about that one
         account, and whole-journal totals beside its line only confuse. -->
    {#if pin === null}
        <div class="stats w-full stats-vertical bg-base-200 shadow-none sm:stats-horizontal" data-testid="balances-tiles">
            {#each tiles as tile (tile.label)}
                <div class="stat flex flex-wrap items-baseline justify-between gap-x-3 px-3 py-1.5 sm:grid sm:py-2" data-testid={tile.testid}>
                    <div class="stat-title text-xs">{tile.label}</div>
                    <div class="stat-value min-w-0 font-mono text-lg [overflow-wrap:anywhere] tabular-nums md:text-2xl {tile.valueClass}">{tile.value}</div>
                </div>
            {/each}
        </div>
    {/if}

    <div class="flex gap-4">
        {#if pin !== null}
            <!-- The pinned account's line, in the pie's place: hidden below `lg`
                 like the pie, so the transaction table keeps its height on a
                 small window; `w-80` against the pie's `w-56`, because a line
                 needs width more than a circle does. -->
            <div class="hidden h-48 w-80 shrink-0 lg:block" data-testid="balances-line">
                {#if pinnedTrend !== null}
                    <BalanceTrend series={pinnedTrend} />
                {:else if trendError !== null}
                    <p class="py-6 text-center text-xs text-error">Couldn't chart {pin.account}: {trendError.message}</p>
                {:else}
                    <div class="flex h-full items-center justify-center" aria-label="Loading balance line"><span class="loading loading-spinner"></span></div>
                {/if}
            </div>
        {:else if !nothingHeld && slices.length > 0}
            <div class="hidden h-48 w-56 shrink-0 lg:block" data-testid="balances-pie">
                <PieChart data={slices} key="account" label="account" value={(d) => d.value} cRange={slices.map((d) => d.color)} padAngle={0.02} legend={false}>
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
        {#if nothingHeld}
            <p class="flex-1 py-6 text-center text-sm text-base-content/60">
                No cash or short-term liability accounts hold {commodity}. Declare your bank and card accounts with
                <code class="text-xs">type: C</code>
                and <code class="text-xs">type: L</code> to see them here.
            </p>
        {:else}
            <!-- The scroll container. Bounded in vh so it shrinks with the
                 window rather than pushing the transaction table off screen. -->
            <div class="max-h-48 min-w-0 flex-1 overflow-y-auto pr-1 sm:max-h-56">
                {@render list("Cash", cash, true)}
                {@render list("Liabilities", liabilities, false)}
            </div>
        {/if}
    </div>

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
