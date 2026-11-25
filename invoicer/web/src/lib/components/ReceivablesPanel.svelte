<!-- What is owed in one commodity (PLAN 5a): the total as the page's hero
     figure, the outstanding bar, its two figures, and the aging buckets.

     The bar (dataviz "tall segmented bar", 28px): overdue in the theme's
     error colour, not yet due in its warning colour, and a grey rest up to
     the scale (the total, or $10,000 when that is more), so a small balance
     reads small. Status colours, never a series palette: they mean the same
     here as in the sidebar's red badge and Ledgeline's budget bars, and every
     theme validates them against its own surfaces. The bar prints no text;
     the figures under it carry the amounts with a swatch each (the legend),
     and every block is a link with its amount in its name.

     The aging buckets are a meter list: a name, a thin bar (the bucket's
     share of the total) and the amount and count per row, Current in the
     warning colour and the overdue buckets in the error colour, as in the
     bar. The rows are the table view: every number is printed.

     Every figure is a link to the invoice list, filtered (`listHref`). -->
<script lang="ts">
    import {barBlocks, bucketName, count, listHref, share, type CommodityView} from "#lib/dashboard.js";

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

    const blocks = $derived(barBlocks(view));
    const owed = $derived(view.outstanding.count > 0);
    const blockLabel = (id: string): string =>
        id === "overdue"
            ? `Overdue: ${view.overdue.text}, ${count(view.overdue.count, "item")}`
            : id === "not-due"
              ? `Not yet due: ${view.not_due.text}, ${count(view.not_due.count, "item")}`
              : `The bar's scale: ${view.scale.text}`;
    const blockHref = (id: string): string => listHref(listBase, id === "overdue" ? {status: "overdue"} : {aged: "current"});
    const blockClass = (id: string): string => (id === "overdue" ? "bg-error" : id === "not-due" ? "bg-warning" : "bg-base-content/10");
</script>

<!-- eslint-disable svelte/no-navigation-without-resolve -- every href is built on listBase, which the page resolves -->
<section class="flex flex-col gap-4" aria-labelledby="owed-{view.commodity}" data-testid="receivables-panel" data-commodity={view.commodity}>
    <div class="flex flex-wrap items-end justify-between gap-x-6 gap-y-1">
        <div class="flex flex-col">
            <h2 id="owed-{view.commodity}" class="text-sm font-medium text-base-content/70">Outstanding</h2>
            <!-- The hero figure: proportional digits at display size (dataviz), a link to everything owed. -->
            <a
                class="link font-semibold tracking-tight link-hover {hero ? 'text-5xl' : 'text-3xl'}"
                href={listHref(listBase, {status: "open"})}
                aria-label="Outstanding: {view.outstanding.text}, {count(view.outstanding.count, 'item')}"
                data-testid="dashboard-total">{view.outstanding.text}</a
            >
        </div>
        <span class="text-sm text-base-content/60" data-testid="dashboard-total-count"
            >{owed ? count(view.outstanding.count, "item") + " owed" : "Nothing owed"}</span
        >
    </div>

    <!-- The bar: one row of blocks, a 2px gap between them, 4px corners each. -->
    <div
        class="flex h-7 w-full gap-0.5"
        role="group"
        aria-label="Outstanding, overdue and not yet due, on a scale of {view.scale.text}"
        data-testid="dashboard-bar"
    >
        {#each blocks as block (block.id)}
            {#if block.id === "rest"}
                <div class="h-full min-w-0 flex-1 rounded {blockClass(block.id)}" title={blockLabel(block.id)} data-testid="bar-rest"></div>
            {:else}
                <a
                    class="h-full min-w-1 rounded transition-opacity hover:opacity-80 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-content {blockClass(
                        block.id
                    )}"
                    style="flex: 0 1 {block.width}%"
                    href={blockHref(block.id)}
                    title={blockLabel(block.id)}
                    aria-label={blockLabel(block.id)}
                    data-testid="bar-{block.id}"
                ></a>
            {/if}
        {/each}
        {#if blocks.length === 0}
            <div class="h-full flex-1 rounded bg-base-content/10" data-testid="bar-rest"></div>
        {/if}
    </div>

    <!-- The figures under the bar, a swatch each: the bar's legend. -->
    <div class="flex flex-wrap items-baseline gap-x-6 gap-y-2 text-sm">
        <a class="group flex items-baseline gap-2" href={listHref(listBase, {status: "overdue"})} data-testid="dashboard-overdue">
            <span class="inline-block h-2.5 w-2.5 shrink-0 rounded-xs bg-error" aria-hidden="true"></span>
            <span class="text-base-content/70">Overdue</span>
            <span class="font-semibold group-hover:underline">{view.overdue.text}</span>
            <span class="text-base-content/60">{count(view.overdue.count, "item")}</span>
        </a>
        <a class="group flex items-baseline gap-2" href={listHref(listBase, {aged: "current"})} data-testid="dashboard-not-due">
            <span class="inline-block h-2.5 w-2.5 shrink-0 rounded-xs bg-warning" aria-hidden="true"></span>
            <span class="text-base-content/70">Not yet due</span>
            <span class="font-semibold group-hover:underline">{view.not_due.text}</span>
            <span class="text-base-content/60">{count(view.not_due.count, "item")}</span>
        </a>
        {#if blocks.some((b) => b.id === "rest")}
            <span class="flex items-baseline gap-2 text-base-content/60" data-testid="dashboard-scale">
                <span class="inline-block h-2.5 w-2.5 shrink-0 rounded-xs bg-base-content/10" aria-hidden="true"></span>
                Bar drawn to {view.scale.text}
            </span>
        {/if}
    </div>
    {#if view.unfiled.count > 0}
        <p class="text-xs text-base-content/60" data-testid="dashboard-unfiled">
            {view.unfiled.text} of it ({count(view.unfiled.count, "item")}) was booked without an invoice here, such as an opening balance, so the invoice list
            doesn't show it.
        </p>
    {/if}

    <section aria-labelledby="aging-{view.commodity}">
        <h3 id="aging-{view.commodity}" class="mb-2 text-sm font-medium text-base-content/70">Aging</h3>
        <ul class="flex flex-col gap-1" data-testid="aging">
            {#each view.buckets as bucket (bucket.key)}
                {@const overdue = bucket.key !== "current"}
                <li data-testid="aging-row" data-key={bucket.key}>
                    {#snippet row()}
                        <!-- The bar drops under the name where the list is narrow (a container query). -->
                        <span class="flex min-w-0 flex-col gap-1 @md:flex-row @md:items-center @md:gap-3">
                            <span class="flex items-baseline justify-between gap-3 @md:w-56 @md:shrink-0">
                                <span class="truncate">{bucket.key === "current" ? "Current" : `${bucket.label} days`}</span>
                                <span class="whitespace-nowrap">
                                    <span class="font-medium tabular-nums">{bucket.text}</span>
                                    <span class="text-xs text-base-content/60">· {bucket.count}</span>
                                </span>
                            </span>
                            <span class="h-2 w-full min-w-0 rounded-full bg-base-content/10" aria-hidden="true">
                                <span
                                    class="block h-full rounded-full {overdue ? 'bg-error' : 'bg-warning'}"
                                    style="width: {share(bucket.amount, view.outstanding.amount)}%"
                                ></span>
                            </span>
                        </span>
                    {/snippet}
                    {#if bucket.count > 0}
                        <a
                            class="block rounded px-2 py-1.5 text-sm hover:bg-base-200 focus-visible:outline-2 focus-visible:outline-base-content"
                            href={listHref(listBase, {aged: bucket.key})}
                            aria-label="{bucketName(bucket)}: {bucket.text}, {count(bucket.count, 'item')}"
                            title="{bucketName(bucket)}: {bucket.text}, {count(bucket.count, 'item')}"
                        >
                            {@render row()}
                        </a>
                    {:else}
                        <div class="px-2 py-1.5 text-sm text-base-content/60">{@render row()}</div>
                    {/if}
                </li>
            {/each}
        </ul>
    </section>
</section>
