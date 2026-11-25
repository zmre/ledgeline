<!-- Recently updated invoices (PLAN R15): the server's latest few, latest
     first, as a row of cards, each a link to the invoice: its number, the
     customer, what happened last and when ("Paid 2026-11-05"), the amount,
     and a band at the bottom naming where it stands. The band wears the
     status colour the list's badges use (paid success, open warning,
     overdue error, draft and void neutral) as a strip, and says the status
     in words beside it: never colour alone.

     Narrow widths (container queries): one row that scrolls sideways, cards
     snapping to its start; from `@3xl` a grid of three, from `@5xl` all six
     in a row. Hover and keyboard focus lift a card (no rise under
     prefers-reduced-motion). -->
<script lang="ts">
    import {recentEventText, recentStatus, type RecentView} from "#lib/dashboard.js";

    let {
        recent,
        invoiceHref,
    }: {
        recent: RecentView[];
        /** An invoice's page, resolved, by number. */
        invoiceHref: (number: string) => string;
    } = $props();

    // Literal classes, so the stylesheet keeps them.
    const BAND: Record<string, string> = {
        neutral: "border-base-content/30 bg-base-200",
        warning: "border-warning bg-warning/10",
        error: "border-error bg-error/10",
        success: "border-success bg-success/10",
    };
    const card =
        "flex h-full min-w-0 flex-col overflow-hidden rounded-box border border-base-300 bg-base-100 transition-[translate,box-shadow,opacity] motion-reduce:transition-none motion-reduce:hover:translate-none motion-reduce:focus-visible:translate-none";
    const lift =
        "hover:-translate-y-0.5 hover:border-base-content/30 hover:shadow-md focus-visible:-translate-y-0.5 focus-visible:shadow-md focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-content";
</script>

{#snippet body(item: RecentView)}
    {@const status = recentStatus(item.status)}
    <span class="flex flex-1 flex-col gap-0.5 px-3 pt-3 pb-2">
        <span class="text-xs text-base-content/60">{item.number === null ? "No number" : `No. ${item.number}`}</span>
        <span class="truncate font-medium" title={item.customer ?? undefined}>{item.customer ?? "No customer"}</span>
        <span class="text-xs text-base-content/60">{recentEventText(item)}</span>
        <span class="mt-auto pt-3 text-right text-xl font-semibold tracking-tight {item.status === 'void' ? 'text-base-content/60 line-through' : ''}"
            >{item.total ?? "—"}</span
        >
    </span>
    <span class="border-t-4 px-3 py-1 text-xs font-semibold tracking-wide text-base-content uppercase {BAND[status.tone]}" data-testid="recent-status"
        >{status.label}</span
    >
{/snippet}

<section class="@container flex flex-col gap-3" aria-labelledby="recent-title" data-testid="recent">
    <h2 id="recent-title" class="text-sm font-medium text-base-content/70">Recently updated</h2>
    <ul class="flex snap-x gap-3 overflow-x-auto px-1 pt-1 pb-2 @3xl:grid @3xl:grid-cols-3 @3xl:overflow-visible @5xl:grid-cols-6" data-testid="recent-cards">
        {#each recent as item (item.path)}
            <li class="w-44 shrink-0 snap-start @3xl:w-auto" data-testid="recent-card" data-number={item.number}>
                {#if item.number !== null}
                    <!-- eslint-disable-next-line svelte/no-navigation-without-resolve -- invoiceHref is resolved by the page -->
                    <a class="{card} {lift}" href={invoiceHref(item.number)}>{@render body(item)}</a>
                {:else}
                    <div class={card}>{@render body(item)}</div>
                {/if}
            </li>
        {/each}
    </ul>
</section>
