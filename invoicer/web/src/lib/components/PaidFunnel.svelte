<!-- Getting paid (PLAN R14, R17): the page's headline figures as a funnel,
     Drafts → Outstanding → Paid in the last N days, a card each with its
     amount, a count badge and a line under it, chevrons between. Each card
     is one link to the invoice list, filtered: `status=draft`,
     `status=open`, `paid_from=…&paid_to=…` (money received in the window).
     A second commodity's figure follows the first, smaller.

     The period selector (Last 7 / 30 / 90 / 365 days, plus the setting's own
     `paid-window-days`) sits in the section's header and scopes the Paid
     card; the page keeps the choice in its URL (`?paid_days=`). While the
     new figure loads the card keeps the old one, dimmed.

     Hover and keyboard focus lift a card (a shadow, a 2px rise; no rise
     under prefers-reduced-motion). Narrow widths: the cards stack and the
     chevrons point down (container queries).

     Keyboard (PLAN 5k): `1`, `2`, `3` open Drafts, Outstanding and Paid;
     `w` and `W` step the paid window longer and shorter. -->
<script lang="ts">
    import {registerKeys} from "#ui/keys/keymap.svelte.js";
    import {count, listHref, paidWindowLabel, paidWindows, type AmountView, type Dashboard} from "#lib/dashboard.js";

    let {
        dashboard,
        listBase,
        onPaidDays,
        loadingPaid = false,
    }: {
        dashboard: Dashboard;
        /** The invoice list's URL, resolved (`/app/invoices`). */
        listBase: string;
        /** Pick another paid window, in days. */
        onPaidDays?: (days: number) => void;
        /** A new window's figures are on their way. */
        loadingPaid?: boolean;
    } = $props();

    const drafts = $derived(dashboard.drafts);
    const owed = $derived(dashboard.commodities.filter((c) => c.outstanding.count > 0));
    const paid = $derived(dashboard.paid);
    const choices = $derived(paidWindows(dashboard.paid_window_days));
    const selectId = $props.id();

    const owedCount = $derived(owed.reduce((n, c) => n + c.outstanding.count, 0));
    const overdue = $derived(owed.filter((c) => c.overdue.count > 0));
    const paidCount = $derived(paid.commodities.reduce((n, c) => n + c.count, 0));
    const paidUnfiled = $derived(paid.commodities.reduce((n, c) => n + c.unfiled, 0));

    // The funnel's keys: a card each (its link, so the router and the list's
    // URL are the click's), and the window, a step along the selector's list.
    const cards: Record<string, HTMLAnchorElement | null> = $state({drafts: null, outstanding: null, paid: null});
    const openCard = (name: string) => (): void => cards[name]?.click();
    function stepWindow(delta: number): void {
        const at = choices.indexOf(paid.days);
        const next = choices[at + delta];
        if (next !== undefined) onPaidDays?.(next);
    }
    const canStep = (delta: number) => (): boolean => onPaidDays !== undefined && choices[choices.indexOf(paid.days) + delta] !== undefined;
    registerKeys({
        id: "dashboard-funnel",
        bindings: [
            {keys: "1", label: "Open the drafts", group: "Dashboard", run: openCard("drafts")},
            {keys: "2", label: "Open what's outstanding", group: "Dashboard", run: openCard("outstanding")},
            {keys: "3", label: "Open what was paid", group: "Dashboard", run: openCard("paid")},
            {keys: "w", label: "A longer paid window", group: "Dashboard", enabled: canStep(1), run: () => stepWindow(1)},
            {keys: "W", label: "A shorter paid window", group: "Dashboard", enabled: canStep(-1), run: () => stepWindow(-1)},
        ],
    });

    const card =
        "group flex h-full min-w-0 flex-col overflow-hidden rounded-box border border-base-300 bg-base-100 transition-[translate,box-shadow,opacity] motion-reduce:transition-none motion-reduce:hover:translate-none motion-reduce:focus-visible:translate-none hover:-translate-y-0.5 hover:border-base-content/30 hover:shadow-md focus-visible:-translate-y-0.5 focus-visible:shadow-md focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-content";
</script>

{#snippet figure(amounts: AmountView[], none: string, testid: string)}
    {#if amounts.length === 0}
        <span class="text-3xl font-semibold tracking-tight text-base-content/60" data-testid={testid}>{none}</span>
    {:else}
        <span class="flex flex-wrap items-baseline gap-x-2">
            <span class="text-3xl font-semibold tracking-tight" data-testid={testid}>{amounts[0].text}</span>
            {#each amounts.slice(1) as more (more.text)}
                <span class="text-base font-medium text-base-content/70">+ {more.text}</span>
            {/each}
        </span>
    {/if}
{/snippet}

{#snippet chevron()}
    <li class="flex items-center justify-center text-base-content/40" aria-hidden="true">
        <svg
            class="h-5 w-5 rotate-90 @2xl:rotate-0"
            viewBox="0 0 20 20"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
            stroke-linejoin="round"><path d="M7.5 4.5 13 10l-5.5 5.5" /></svg
        >
    </li>
{/snippet}

<!-- eslint-disable svelte/no-navigation-without-resolve -- every href is built on listBase, which the page resolves -->
<section class="@container flex flex-col gap-3" aria-labelledby="funnel-title" data-testid="funnel">
    <div class="flex flex-wrap items-center justify-between gap-2">
        <h2 id="funnel-title" class="text-sm font-medium text-base-content/70">Getting paid</h2>
        <label class="flex items-center gap-2 text-sm text-base-content/70" for={selectId}
            >Paid in the
            <select
                id={selectId}
                class="select w-auto select-sm"
                value={paid.days}
                onchange={(event) => onPaidDays?.(Number(event.currentTarget.value))}
                data-testid="paid-window"
            >
                {#each choices as days (days)}
                    <option value={days}>{paidWindowLabel(days).replace(/^Last/, "last")}</option>
                {/each}
            </select>
        </label>
    </div>
    <ol class="grid grid-cols-1 gap-2 @2xl:grid-cols-[minmax(0,1fr)_auto_minmax(0,1fr)_auto_minmax(0,1fr)] @2xl:gap-3">
        <li class="min-w-0">
            <a class={card} href={listHref(listBase, {status: "draft"})} bind:this={cards.drafts} data-testid="funnel-drafts">
                <span class="h-1 bg-base-content/25" aria-hidden="true"></span>
                <span class="flex flex-col gap-1 p-4">
                    <span class="text-sm text-base-content/70">Drafts</span>
                    {@render figure(drafts.totals, "None", "funnel-drafts-amount")}
                    <span class="flex flex-wrap items-center gap-2 text-xs text-base-content/60">
                        <span class="badge badge-ghost badge-sm" data-testid="funnel-drafts-count">{count(drafts.count, "draft")}</span>
                        {#if drafts.count === 0}
                            Nothing waiting to be sent.
                        {:else if drafts.oldest !== null}
                            oldest {drafts.oldest}
                        {/if}
                    </span>
                </span>
            </a>
        </li>
        {@render chevron()}
        <li class="min-w-0">
            <a class={card} href={listHref(listBase, {status: "open"})} bind:this={cards.outstanding} data-testid="funnel-outstanding">
                <span class="h-1 bg-warning" aria-hidden="true"></span>
                <span class="flex flex-col gap-1 p-4">
                    <span class="text-sm text-base-content/70">Outstanding</span>
                    {@render figure(
                        owed.map((c) => c.outstanding),
                        "Nothing owed",
                        "funnel-outstanding-amount"
                    )}
                    <span class="flex flex-wrap items-center gap-2 text-xs text-base-content/60">
                        <span class="badge badge-ghost badge-sm" data-testid="funnel-outstanding-count">{owedCount} owed</span>
                        {#each overdue as c (c.commodity)}
                            <span class="flex items-center gap-1" data-testid="funnel-overdue">
                                <span class="inline-block h-2 w-2 rounded-full bg-error" aria-hidden="true"></span>
                                {c.overdue.text} overdue
                            </span>
                        {/each}
                    </span>
                </span>
            </a>
        </li>
        {@render chevron()}
        <li class="min-w-0">
            <a
                class="{card} {loadingPaid ? 'opacity-60' : ''}"
                href={listHref(listBase, {paid_from: paid.from, paid_to: paid.to})}
                aria-busy={loadingPaid}
                bind:this={cards.paid}
                data-testid="funnel-paid"
            >
                <span class="h-1 bg-success" aria-hidden="true"></span>
                <span class="flex flex-col gap-1 p-4">
                    <span class="text-sm text-base-content/70">Paid · {paidWindowLabel(paid.days).toLowerCase()}</span>
                    {@render figure(paid.commodities, "Nothing yet", "funnel-paid-amount")}
                    <span class="flex flex-wrap items-center gap-2 text-xs text-base-content/60">
                        <span class="badge badge-ghost badge-sm" data-testid="funnel-paid-count">{paidCount} paid</span>
                        since {paid.from}
                        {#if paidUnfiled > 0}
                            · {count(paidUnfiled, "item")} without an invoice here
                        {/if}
                    </span>
                </span>
            </a>
        </li>
    </ol>
</section>
