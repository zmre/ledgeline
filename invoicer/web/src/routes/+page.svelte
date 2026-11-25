<!-- The app's home: the receivables dashboard (PLAN 5a, R1), from
     `GET /api/dashboard` on the browser's local date, again whenever a file
     changes on disk (live reload). Books without an invoice (and nothing
     owed in the journal) get the shared empty state. -->
<script lang="ts">
    import {onMount} from "svelte";
    import {resolve} from "$app/paths";
    import AsyncSection from "#ui/components/AsyncSection.svelte";
    import type {DataView} from "#ui/ui/dataView.js";
    import {fetchDashboard, openInLedgeline} from "#lib/api.js";
    import {changes} from "#lib/changes.svelte.js";
    import type {Dashboard} from "#lib/dashboard.js";
    import DashboardView from "#lib/components/DashboardView.svelte";
    import EmptyState from "#lib/components/EmptyState.svelte";

    let dashboard = $state<Dashboard | null>(null);
    let error = $state<Error | null>(null);
    const view = $derived<DataView>(error !== null ? "error" : dashboard !== null ? "data" : "loading");
    let opening = $state(false);
    let openError = $state<string | null>(null);

    function load(): void {
        error = null;
        fetchDashboard()
            .then((d) => (dashboard = d))
            .catch((e: unknown) => (error = e instanceof Error ? e : new Error(String(e))));
    }

    onMount(load);

    // Live reload: again after a change on disk, the old one on screen meanwhile.
    let seenSeq = changes.seq;
    $effect(() => {
        const seq = changes.seq;
        if (seq === seenSeq) return;
        seenSeq = seq;
        fetchDashboard()
            .then((d) => {
                dashboard = d;
                error = null;
            })
            .catch((e: unknown) => console.warn("dashboard:", e));
    });

    function openLedgeline(): void {
        opening = true;
        openError = null;
        openInLedgeline()
            .catch((e: unknown) => (openError = e instanceof Error ? e.message : String(e)))
            .finally(() => (opening = false));
    }

    const listBase = resolve("/invoices");
    const invoiceHref = (number: string): string => resolve("/invoices/[number]", {number});
</script>

<svelte:head>
    <title>Dashboard · Ledgeline Invoicer</title>
</svelte:head>

<div class="@container mx-auto flex max-w-6xl flex-col gap-4">
    <div class="flex flex-wrap items-center justify-between gap-2">
        <h1 class="text-2xl font-semibold tracking-tight">Dashboard</h1>
        {#if dashboard !== null && (dashboard.invoices > 0 || dashboard.commodities.length > 0)}
            <a class="btn btn-primary btn-sm" href={resolve("/new")} data-testid="dashboard-new">New invoice</a>
        {/if}
    </div>
    <AsyncSection {view} value={dashboard} {error} testid="dashboard-error" label="the dashboard" loadingLabel="Loading the dashboard" onRetry={load}>
        {#snippet children(d)}
            {#if d.invoices === 0 && d.commodities.length === 0}
                <EmptyState
                    title="No invoices yet"
                    text="What your customers owe, and how late, shows here once you send an invoice."
                    action="New invoice"
                    href={resolve("/new")}
                    testid="dashboard-empty"
                    actionTestid="dashboard-empty-new"
                />
            {:else}
                <DashboardView dashboard={d} {listBase} {invoiceHref} onOpenLedgeline={openLedgeline} {opening} {openError} />
            {/if}
        {/snippet}
    </AsyncSection>
</div>
