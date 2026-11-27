<!-- To bill (plan 46 B-unbilled, B-ap): what to invoice (`/to-bill`) and whom
     to pay (`/to-bill?tab=pay`). The view is lib/time/billing/UnbilledView.svelte;
     this route gives it its tab and its links, and opens a draft Bill… makes
     in the invoice editor. `?bill=all` (the New menu's Bill time…) starts
     Bill all…, then leaves the URL. /time/unbilled, its old address,
     redirects here. -->
<script lang="ts">
    import {goto} from "$app/navigation";
    import {resolve} from "$app/paths";
    import {page} from "$app/state";
    import UnbilledView, {type BillingTab} from "#lib/time/billing/UnbilledView.svelte";

    const entries = resolve("/time/entries");
    const here = resolve("/to-bill");

    const tab = $derived<BillingTab>(page.url.searchParams.get("tab") === "pay" ? "pay" : "bill");
    const startBillAll = $derived(page.url.searchParams.get("bill") === "all");

    /** This page on `next` (and no `bill=all`). */
    const hrefFor = (next: BillingTab): string => (next === "pay" ? `${here}?tab=pay` : here);

    function show(next: BillingTab): void {
        // eslint-disable-next-line svelte/no-navigation-without-resolve -- hrefFor resolves the path; only the query is added
        void goto(hrefFor(next), {replace: true, reset: false});
    }
</script>

<svelte:head><title>{tab === "pay" ? "To pay" : "To bill"} · Ledgeline Invoicer</title></svelte:head>

<UnbilledView
    {tab}
    onTab={show}
    {startBillAll}
    onBillAllStarted={() => show(tab)}
    onOpenDraft={(created) => void goto(resolve("/invoices/[number]/edit", {number: created.number}), {state: {createWarnings: created.warnings}})}
    draftHref={(number) => resolve("/invoices/[number]/edit", {number})}
    entriesHref={(query) => `${entries}?${query}`}
    accountsHref={resolve("/time/accounts")}
    trackHref={resolve("/time/track")}
/>
