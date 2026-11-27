<!-- Time → Unbilled (plan 46 B-unbilled, B-ap): what to invoice and whom to
     pay. The view is lib/time/billing/UnbilledView.svelte; this route gives
     it its links and opens a draft Bill… makes in the invoice editor. -->
<script lang="ts">
    import {goto} from "$app/navigation";
    import {resolve} from "$app/paths";
    import UnbilledView from "#lib/time/billing/UnbilledView.svelte";

    const entries = resolve("/time/entries");
</script>

<svelte:head><title>Unbilled time · Ledgeline Invoicer</title></svelte:head>

<UnbilledView
    onOpenDraft={(created) => void goto(resolve("/invoices/[number]/edit", {number: created.number}), {state: {createWarnings: created.warnings}})}
    draftHref={(number) => resolve("/invoices/[number]/edit", {number})}
    entriesHref={(query) => `${entries}?${query}`}
    accountsHref={resolve("/time/accounts")}
    trackHref={resolve("/time/track")}
/>
