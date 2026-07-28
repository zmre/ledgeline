<!-- The Subscriptions report tab: recurring annual and monthly charges inferred
     from the journal's expense history.

     Unlike the other report tabs this takes no period controls — detection
     always scans a trailing window (24 months) ending at today, so the window is
     stated in the header rather than being adjustable. The window is anchored to
     the browser's local date, not the server's UTC one. -->
<script lang="ts">
    import {NativeApiUnavailableError} from "$lib/api/native";
    import type {AmountStyle} from "$lib/domain/types";
    import {today} from "$lib/reports/periods";
    import {dataView} from "$lib/stores/loadState";
    import {subscriptions} from "$lib/stores/subscriptions.svelte";
    import SubscriptionsBox from "./SubscriptionsBox.svelte";

    let {serverUrl, styles, base = "$"}: {serverUrl: string | null; styles: ReadonlyMap<string, AmountStyle>; base?: string} = $props();

    $effect(() => {
        const url = serverUrl;
        if (url === null) return;
        void subscriptions.load(url, today());
    });

    const report = $derived(subscriptions.report);
    // Error before data (FE-5): with the data branch first, a failed reload left
    // yesterday's detected subscriptions on screen with nothing to say so.
    const view = $derived(dataView(subscriptions.status, report !== null));
    const nativeUnavailable = $derived(subscriptions.error instanceof NativeApiUnavailableError);
</script>

<div class="flex flex-col gap-4" data-testid="subscriptions-panel">
    {#if view === "error"}
        <div class="alert alert-error rounded-box flex-col items-start gap-2 px-3 py-3 text-sm" role="alert" data-testid="subscriptions-error">
            <span>{nativeUnavailable ? subscriptions.error?.message : `Couldn't load subscriptions: ${subscriptions.error?.message ?? "unknown error"}`}</span>
            {#if !nativeUnavailable}
                <button type="button" class="btn btn-sm" onclick={() => void subscriptions.load(serverUrl ?? "", today())}>Retry</button>
            {/if}
        </div>
    {:else if view === "data" && report !== null}
        <div class="text-base-content/60 text-xs">
            Recurring charges detected in <span class="text-base-content/80 font-medium">{report.lookbackStart} → {report.asOf}</span>
        </div>

        <div class="grid grid-cols-1 gap-4 lg:grid-cols-2">
            <SubscriptionsBox
                title="Annual Subscriptions"
                cadence="annual"
                rows={report.annual}
                {base}
                {styles}
                lookbackStart={report.lookbackStart}
                testid="subs-box-annual"
            />
            <SubscriptionsBox
                title="Monthly Subscriptions"
                cadence="monthly"
                rows={report.monthly}
                {base}
                {styles}
                lookbackStart={report.lookbackStart}
                testid="subs-box-monthly"
            />
        </div>

        <p class="text-base-content/50 text-xs">
            A charge is counted when it repeats at a steady price on a consistent day — so variable bills (utilities, groceries), one-off purchases, and
            anything matching <code class="text-base-content/70">mortgage</code> are left out. A charge that stops is treated as cancelled and dropped, but only once
            the account paying it has newer activity — so an account you haven't imported lately keeps its subscriptions. Click any row to see its transactions in
            the journal.
        </p>
        <p class="text-base-content/50 text-xs">
            To overrule any of that, tag a transaction's comment: <code class="text-base-content/70">subscription:true</code> puts that payee on the list
            whatever the amounts do, and <code class="text-base-content/70">subscription:false</code> takes it off. If the comment already has a tag, separate
            them with a comma (<code class="text-base-content/70">category:infra, subscription:true</code>) — a tag's value runs to the next comma, so without
            one the new tag is swallowed by the previous one.
        </p>
    {:else}
        <div class="flex items-center justify-center py-24" aria-label="Loading subscriptions">
            <span class="loading loading-spinner loading-lg"></span>
        </div>
    {/if}
</div>
