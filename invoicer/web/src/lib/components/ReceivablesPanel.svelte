<!-- What is owed in one commodity (PLAN 5a, R16): the outstanding bar on its
     axis (`OutstandingBar`, with its legend), the total as the page's hero
     figure with its caption to the bar's right, and the aging buckets as a
     compact aligned list beside it: name, amount and count per row, each a
     link to its range (`aged=`). The list is the bar's table view: every
     number is printed. A swatch per row (warning for Current, error for the
     overdue buckets, as in the bar) is identity beside the words, never
     instead of them.

     Narrow widths (container queries, which follow the UI scale): the bar
     takes the whole row and the total and the list go under it, side by
     side from `@md`, stacked below that. -->
<script lang="ts">
    import {bucketName, count, listHref, type CommodityView} from "#lib/dashboard.js";
    import OutstandingBar from "./OutstandingBar.svelte";

    let {
        view,
        listBase,
        hero = true,
    }: {
        view: CommodityView;
        /** The invoice list's URL, resolved (`/app/invoices`). */
        listBase: string;
        /** The first commodity's total is the page's one hero figure; the others are smaller. */
        hero?: boolean;
    } = $props();

    const owed = $derived(view.outstanding.count > 0);
</script>

<!-- eslint-disable svelte/no-navigation-without-resolve -- every href is built on listBase, which the page resolves -->
<section
    class="@container flex flex-col gap-4 rounded-box border border-base-300 bg-base-100 p-4"
    aria-labelledby="owed-{view.commodity}"
    data-testid="receivables-panel"
    data-commodity={view.commodity}
>
    <h2 id="owed-{view.commodity}" class="text-sm font-medium text-base-content/70">
        Outstanding{hero ? "" : ` in ${view.commodity}`}
    </h2>
    <div class="flex flex-col gap-6 @5xl:flex-row @5xl:items-start">
        <div class="min-w-0 @5xl:flex-1">
            <OutstandingBar {view} {listBase} />
        </div>
        <div class="flex flex-col gap-5 @md:flex-row @md:items-start @md:gap-6">
            <!-- The hero figure: proportional digits at display size (dataviz), a link to everything owed, its caption under it. -->
            <div class="flex flex-col @md:min-w-40 @5xl:items-end @5xl:text-right">
                <a
                    class="link font-semibold tracking-tight link-hover {hero ? 'text-5xl' : 'text-3xl'}"
                    href={listHref(listBase, {status: "open"})}
                    aria-label="Outstanding: {view.outstanding.text}, {count(view.outstanding.count, 'item')}"
                    data-testid="dashboard-total">{view.outstanding.text}</a
                >
                <span class="text-sm text-base-content/70">Total outstanding</span>
                <span class="text-xs text-base-content/60" data-testid="dashboard-total-count"
                    >{owed ? count(view.outstanding.count, "item") + " owed" : "Nothing owed"}</span
                >
            </div>
            <section class="min-w-0 @md:w-64 @md:border-l @md:border-base-300 @md:pl-6" aria-labelledby="aging-{view.commodity}">
                <h3 id="aging-{view.commodity}" class="mb-1 text-xs font-medium text-base-content/60">Aging</h3>
                <ul class="flex flex-col" data-testid="aging">
                    {#each view.buckets as bucket (bucket.key)}
                        {@const late = bucket.key !== "current"}
                        <li data-testid="aging-row" data-key={bucket.key}>
                            {#snippet row()}
                                <span class="inline-block h-2 w-2 shrink-0 rounded-full {late ? 'bg-error' : 'bg-warning'}" aria-hidden="true"></span>
                                <span class="min-w-0 flex-1 truncate">{late ? `${bucket.label} days` : "Current"}</span>
                                <span class="font-medium tabular-nums">{bucket.text}</span>
                                <span class="w-6 text-right text-xs text-base-content/60 tabular-nums">{bucket.count}</span>
                            {/snippet}
                            {#if bucket.count > 0}
                                <a
                                    class="-mx-2 flex items-center gap-2 rounded px-2 py-1 text-sm hover:bg-base-200 focus-visible:outline-2 focus-visible:outline-base-content motion-reduce:transition-none"
                                    href={listHref(listBase, {aged: bucket.key})}
                                    aria-label="{bucketName(bucket)}: {bucket.text}, {count(bucket.count, 'item')}"
                                    title="{bucketName(bucket)}: {bucket.text}, {count(bucket.count, 'item')}"
                                >
                                    {@render row()}
                                </a>
                            {:else}
                                <div class="-mx-2 flex items-center gap-2 px-2 py-1 text-sm text-base-content/60">{@render row()}</div>
                            {/if}
                        </li>
                    {/each}
                </ul>
            </section>
        </div>
    </div>
    {#if view.unfiled.count > 0}
        <p class="text-xs text-base-content/60" data-testid="dashboard-unfiled">
            {view.unfiled.text} of it ({count(view.unfiled.count, "item")}) was booked without an invoice here, such as an opening balance, so the invoice list
            doesn't show it.
        </p>
    {/if}
</section>
