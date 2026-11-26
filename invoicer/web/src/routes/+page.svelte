<!-- The app's home: the receivables dashboard (PLAN 5a, R1), from
     `GET /api/dashboard` on the browser's local date, again whenever a file
     changes on disk (live reload). Books without an invoice (and nothing
     owed in the journal) get the shared empty state, with no button of its
     own: it points at the sidebar's New (R13), as the page has no New
     invoice button either.

     The paid window is the URL's (`/app/?paid_days=90`, R14): absent, the
     server's `[invoicer] paid-window-days`. Picking another is a history
     step (Back returns to the last one); the setting's own writes no
     query. -->
<script lang="ts">
    import {untrack} from "svelte";
    import {goto} from "$app/navigation";
    import {resolve} from "$app/paths";
    import {page} from "$app/state";
    import AsyncSection from "#ui/components/AsyncSection.svelte";
    import type {DataView} from "#ui/ui/dataView.js";
    import {fetchDashboard, openInLedgeline} from "#lib/api.js";
    import {onFilesChanged} from "#lib/changes.svelte.js";
    import {paidDaysParam, paidDaysQuery, type Dashboard} from "#lib/dashboard.js";
    import {errorText} from "#lib/labels.js";
    import DashboardView from "#lib/components/DashboardView.svelte";
    import EmptyState from "#lib/components/EmptyState.svelte";

    let dashboard = $state<Dashboard | null>(null);
    let error = $state<Error | null>(null);
    const view = $derived<DataView>(error !== null ? "error" : dashboard !== null ? "data" : "loading");
    let opening = $state(false);
    let openError = $state<string | null>(null);
    let loadingPaid = $state(false);

    /** The URL's paid window; null is the setting's. */
    const paidDays = $derived(paidDaysParam(page.url.searchParams));

    // Only the latest request's answer is shown: a slow one for an older
    // window never overwrites a newer one.
    let latest = 0;
    function load(days: number | null): void {
        const mine = ++latest;
        // Keep the figures on screen while another window loads, the paid card
        // dimmed; a live reload of the same window dims nothing.
        loadingPaid = dashboard !== null && (days ?? dashboard.paid_window_days) !== dashboard.paid.days;
        fetchDashboard(undefined, days)
            .then((d) => {
                if (mine !== latest) return;
                dashboard = d;
                error = null;
            })
            .catch((e: unknown) => {
                if (mine !== latest) return;
                if (dashboard === null) error = e instanceof Error ? e : new Error(String(e));
                else console.warn("dashboard:", e);
            })
            .finally(() => {
                if (mine === latest) loadingPaid = false;
            });
    }

    // The dashboard follows the URL's window.
    $effect(() => {
        const days = paidDays;
        untrack(() => load(days));
    });

    function retry(): void {
        error = null;
        load(paidDays);
    }

    /** Show the window of `days`: a history step; the setting's own is the bare page. */
    function setPaidDays(days: number): void {
        const setting = dashboard?.paid_window_days ?? days;
        const qs = paidDaysQuery(days, setting);
        const base = resolve("/");
        // eslint-disable-next-line svelte/no-navigation-without-resolve -- the path is resolve()d; only the query is added
        void goto(qs === "" ? base : `${base}?${qs}`, {reset: false});
    }

    // Live reload: again after a change on disk, the old one on screen meanwhile.
    onFilesChanged(() => load(paidDays));

    function openLedgeline(): void {
        opening = true;
        openError = null;
        openInLedgeline()
            .catch((e: unknown) => (openError = errorText(e)))
            .finally(() => (opening = false));
    }

    const listBase = resolve("/invoices");
    const invoiceHref = (number: string): string => resolve("/invoices/[number]", {number});
</script>

<svelte:head>
    <title>Dashboard · Ledgeline Invoicer</title>
</svelte:head>

<div class="@container mx-auto flex max-w-6xl flex-col gap-4">
    <h1 class="text-2xl font-semibold tracking-tight">Dashboard</h1>
    <AsyncSection {view} value={dashboard} {error} testid="dashboard-error" label="the dashboard" loadingLabel="Loading the dashboard" onRetry={retry}>
        {#snippet children(d)}
            {#if d.invoices === 0 && d.commodities.length === 0}
                <EmptyState
                    title="No invoices yet"
                    text="Start one with New at the top of the sidebar. What your customers owe, and how late, shows here once you send it."
                    testid="dashboard-empty"
                />
            {:else}
                <DashboardView
                    dashboard={d}
                    {listBase}
                    {invoiceHref}
                    onOpenLedgeline={openLedgeline}
                    {opening}
                    {openError}
                    onPaidDays={setPaidDays}
                    {loadingPaid}
                />
            {/if}
        {/snippet}
    </AsyncSection>
</div>
