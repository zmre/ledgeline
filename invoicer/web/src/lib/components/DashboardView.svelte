<!-- The receivables dashboard's body (PLAN 5a), from `GET /api/dashboard`:
     what is owed per commodity (`ReceivablesPanel`: the hero total, the
     outstanding bar, aging), then a row of stat tiles: the oldest overdue,
     days sales outstanding, average days to pay, drafts waiting and, when
     the books have recognition schedules, the entries due.

     Every figure is a link to the invoice list, filtered (`listHref`; the
     parameters are in `#lib/dashboard.ts`). Links arrive resolved, so this
     mounts as is in a component test. The page is a container: tiles go to
     two columns at `@lg` and to four at `@4xl` of its own width, which
     follows the UI scale. -->
<script lang="ts">
    import {count, days, listHref, type Dashboard} from "#lib/dashboard.js";
    import ReceivablesPanel from "./ReceivablesPanel.svelte";

    let {
        dashboard,
        listBase,
        invoiceHref,
        onOpenLedgeline,
        opening = false,
        openError = null,
    }: {
        dashboard: Dashboard;
        /** The invoice list's URL, resolved (`/app/invoices`). */
        listBase: string;
        /** An invoice's page, resolved, by number. */
        invoiceHref: (number: string) => string;
        /** Open the books in Ledgeline (offered only when the server can). */
        onOpenLedgeline?: () => void;
        opening?: boolean;
        openError?: string | null;
    } = $props();

    const primary = $derived(dashboard.commodities[0] ?? null);
    const oldest = $derived(primary?.oldest_overdue ?? null);
    const dso = $derived(primary?.dso ?? null);
    const paid = $derived(dashboard.average_days_to_pay);
    const drafts = $derived(dashboard.drafts);
    const recognition = $derived(dashboard.recognition);

    // The oldest overdue opens its invoice when it has a file; else (booked
    // in the journal alone) its customer's overdue invoices.
    const oldestHref = $derived(
        oldest === null
            ? listHref(listBase, {status: "overdue"})
            : oldest.path !== null && oldest.number !== null
              ? invoiceHref(oldest.number)
              : listHref(listBase, {status: "overdue", customer: oldest.customer ?? undefined})
    );

    // "No. 2001 · Tailspin · $3,000.00 · due 2026-07-15", one string so formatting can't split it.
    const oldestDetail = $derived(
        oldest === null
            ? ""
            : [oldest.number === null ? "No number" : `No. ${oldest.number}`, oldest.customer, oldest.outstanding, `due ${oldest.due}`]
                  .filter((part) => part !== null)
                  .join(" · ")
    );

    const tile = "flex min-w-0 flex-col gap-1 rounded-box border border-base-300 bg-base-100 p-4";
    const tileLink = `${tile} transition-colors hover:border-base-content/30 hover:bg-base-200/50 focus-visible:outline-2 focus-visible:outline-base-content`;
</script>

<!-- eslint-disable svelte/no-navigation-without-resolve -- every href is built on listBase or invoiceHref, which the page resolves -->
<div class="flex flex-col gap-6" data-testid="dashboard">
    {#if dashboard.commodities.length === 0}
        <section class="flex flex-col gap-1" data-testid="dashboard-nothing-owed">
            <h2 class="text-sm font-medium text-base-content/70">Outstanding</h2>
            <p class="text-5xl font-semibold tracking-tight">Nothing owed</p>
            <p class="text-sm text-base-content/60">Every invoice sent has been paid.</p>
        </section>
    {:else}
        {#each dashboard.commodities as view, at (view.commodity)}
            <ReceivablesPanel {view} {listBase} hero={at === 0} />
        {/each}
    {/if}

    <ul class="grid gap-3 @lg:grid-cols-2 @4xl:grid-cols-4" aria-label="Receivables at a glance" data-testid="dashboard-tiles">
        <li class="flex">
            <a class="{tileLink} flex-1" href={oldestHref} data-testid="tile-oldest-overdue">
                <span class="text-sm text-base-content/70">Oldest overdue</span>
                {#if oldest === null}
                    <span class="text-2xl font-semibold">None</span>
                    <span class="text-xs text-base-content/60">Nothing is past due.</span>
                {:else}
                    <span class="text-2xl font-semibold">{days(oldest.days_past_due)}</span>
                    <span class="text-xs text-base-content/60">{oldestDetail}</span>
                {/if}
            </a>
        </li>
        <li class="flex">
            <a class="{tileLink} flex-1" href={listHref(listBase, {status: "open"})} data-testid="tile-dso">
                <span class="text-sm text-base-content/70"><abbr title="Days sales outstanding" class="no-underline">DSO</abbr></span>
                {#if dso === null || dso.value === null}
                    <span class="text-2xl font-semibold">—</span>
                    <span class="text-xs text-base-content/60">Nothing billed in the last {days(dashboard.dso_days)}.</span>
                {:else}
                    <span class="text-2xl font-semibold">{days(dso.value)}</span>
                    <span class="text-xs text-base-content/60">{dso.outstanding} owed for {dso.billed} billed in the last {days(dso.days)}</span>
                {/if}
            </a>
        </li>
        <li class="flex">
            <a class="{tileLink} flex-1" href={listHref(listBase, {status: "paid"})} data-testid="tile-days-to-pay">
                <span class="text-sm text-base-content/70">Average days to pay</span>
                {#if paid.average === null}
                    <span class="text-2xl font-semibold">—</span>
                    <span class="text-xs text-base-content/60">No invoice is paid in full yet.</span>
                {:else}
                    <span class="text-2xl font-semibold">{days(paid.average)}</span>
                    <span class="text-xs text-base-content/60">Over {count(paid.invoices, "invoice")} paid in full</span>
                {/if}
            </a>
        </li>
        <li class="flex">
            <a class="{tileLink} flex-1" href={listHref(listBase, {status: "draft"})} data-testid="tile-drafts">
                <span class="text-sm text-base-content/70">Drafts waiting</span>
                <span class="text-2xl font-semibold">{drafts.count}</span>
                <span class="text-xs text-base-content/60">
                    {#if drafts.count === 0}
                        Nothing waiting to be sent.
                    {:else}
                        {drafts.totals.map((t) => t.text).join(" + ")}{drafts.oldest === null ? "" : ` · oldest ${drafts.oldest}`}
                    {/if}
                </span>
            </a>
        </li>
        {#if recognition !== null}
            <li class="flex @lg:col-span-2 @4xl:col-span-4">
                <section class="{tile} flex-1" aria-labelledby="tile-recognition-title" data-testid="tile-recognition">
                    <div class="flex flex-wrap items-center justify-between gap-3">
                        <div class="flex min-w-0 flex-col gap-1">
                            <h2 id="tile-recognition-title" class="text-sm font-normal text-base-content/70">Recognition due</h2>
                            <span class="text-2xl font-semibold" data-testid="recognition-due">{count(recognition.due, "entry", "entries")}</span>
                            <span class="text-xs text-base-content/60">
                                {#if recognition.due === 0}
                                    Every scheduled entry through today is posted.
                                {:else}
                                    Scheduled revenue and expenses to post{recognition.earliest === null ? "" : `, from ${recognition.earliest}`}.
                                {/if}
                                {#if recognition.closed > 0}
                                    {count(recognition.closed, "more is", "more are")} due in a closed period.
                                {/if}
                                {#if !dashboard.open_ledgeline}
                                    Post them in Ledgeline: Journal → Recognition.
                                {/if}
                            </span>
                        </div>
                        {#if dashboard.open_ledgeline}
                            <button
                                type="button"
                                class="btn btn-outline btn-sm"
                                disabled={opening}
                                onclick={() => onOpenLedgeline?.()}
                                data-testid="open-ledgeline">Open in Ledgeline</button
                            >
                        {/if}
                    </div>
                    {#if openError !== null}
                        <p class="text-xs text-error" role="alert" data-testid="open-ledgeline-error">{openError}</p>
                    {/if}
                </section>
            </li>
        {/if}
    </ul>
</div>
