<script lang="ts">
    // Journal route (WP-03): filter bar (WP-04) and insights panel (WP-05) mount
    // above the virtualized transaction table; the totals footer stays pinned.
    // On mount (and whenever a server URL is first configured) → journal.refresh().
    import {onMount, untrack} from "svelte";
    import ErrorToast from "#lib/components/ErrorToast.svelte";
    import {styleOf} from "#lib/format/amounts.js";
    import ExportButton from "#lib/reports/ui/ExportButton.svelte";
    import FilterBar from "#lib/filters/FilterBar.svelte";
    import {startUrlSync} from "#lib/filters/urlSync.js";
    import InsightsPanel from "#lib/insights/InsightsPanel.svelte";
    import TotalsFooter from "#lib/journal/TotalsFooter.svelte";
    import TransactionTable from "#lib/journal/TransactionTable.svelte";
    import {txnModal} from "#lib/journal/edit/modalState.svelte.js";
    import MatchOverlay from "#lib/matching/ui/MatchOverlay.svelte";
    import ReviewOverlay from "#lib/review/ui/ReviewOverlay.svelte";
    import {reviewLaunch} from "#lib/review/ui/launch.svelte.js";
    import {periodLabel} from "#lib/journal/rowModel.js";
    import {editing} from "#lib/stores/editing.svelte.js";
    import {filters} from "#lib/stores/filters.svelte.js";
    import {journal} from "#lib/stores/journal.svelte.js";
    import {journalView} from "#lib/stores/journalView.svelte.js";
    import {running} from "#lib/stores/running.svelte.js";
    import {matches} from "#lib/stores/matches.svelte.js";
    import {onServerReady} from "#lib/stores/serverWatch.svelte.js";
    import {settings} from "#lib/stores/settings.svelte.js";

    const period = $derived(periodLabel(filters.value.from, filters.value.to));

    // The journal VIEW follows (server, generation, filter): rows, totals and
    // charts are fetched from the engine for exactly what is on screen (plan 26
    // B5). A generation change — an edit, a file change the poll noticed —
    // re-reads the same view; a filter change asks for a different one.
    //
    // A pinned running balance (plan 36) rides along on the rows. An account
    // the engine refuses (a stale `bal=` link) is dropped from the request
    // rather than left to fail every block; the pin chip says why.
    $effect(() => {
        const serverUrl = settings.serverUrl;
        const generation = journal.generation;
        const filter = filters.value;
        if (serverUrl === null || generation === null) return;
        const pin = filter.pin ?? null;
        journalView.sync(serverUrl, generation, filter, pin === null || running.refused(pin.account) ? null : pin);
    });

    // The pin's facts — its commodities and assertions, for the chip and the
    // marks — follow (server, generation, pinned account). A derived string, so
    // filter changes that keep the pin do not refetch them.
    const pinnedAccount = $derived(filters.value.pin?.account ?? null);
    $effect(() => {
        const serverUrl = settings.serverUrl;
        const generation = journal.generation;
        const account = pinnedAccount;
        if (serverUrl === null || generation === null || account === null) return;
        void running.load(serverUrl, {account, generation});
    });
    const count = $derived(journalView.total ?? 0);
    /** A collapsed transfer pair is one row (plan 35 D11): with any, the footer counts rows, not transactions. */
    const countsRows = $derived((journalView.summary?.transfers ?? 0) > 0);
    const total = $derived(journalView.summary?.visibleNet ?? []);

    /**
     * The journal could not be loaded and there is nothing held over from a
     * previous load.
     *
     * Everything below summarizes the journal, and an empty journal is a
     * perfectly valid input to all of it — so a failed load used to fall
     * through to the happy path and describe itself as a clean, empty journal:
     * "No transactions match the current filters" (a causal claim about the
     * filters that is simply untrue), Income / Expenses / Net all `$0.00` with
     * Net in success green, "0 transactions" in the footer. Only a corner toast
     * said otherwise (FE-5b). A load that failed AFTER data arrived is
     * different and keeps the old rows plus that toast, which is the intended
     * stale-but-labelled behaviour.
     */
    /**
     * Download exactly what the table shows — the date, account and search
     * filters already applied — as a QuickBooks-style journal an accountant can
     * work from. The builder (and exceljs) load on first click only.
     */
    async function exportJournal(): Promise<void> {
        const {exportJournalXlsx} = await import("#lib/export/journalXlsx.js");
        const styles = journal.reportStyles;
        const {from, to} = filters.value;
        const filename = from === null && to === null ? "journal-all-dates.xlsx" : `journal-${from ?? "start"}-to-${to ?? "end"}.xlsx`;
        // Oldest first, as a journal is read (the table shows newest first): the
        // engine pages the filtered view in exactly that order.
        const ordered = await journalView.allViewRows();
        // With a pin, the export carries the Change and Balance columns too: it
        // is "exactly what the table shows", and the table shows them.
        const pin = journalView.query?.pin ?? null;
        const runningColumns =
            pin === null
                ? undefined
                : {account: pin.account, byIndex: new Map(ordered.flatMap((row) => (row.running == null ? [] : [[row.txn.index, row.running] as const])))};
        await exportJournalXlsx(
            ordered.map((row) => row.txn),
            (commodity) => styleOf(styles, commodity),
            {title: journal.title, period, running: runningColumns},
            filename
        );
    }

    const loadFailed = $derived(journal.status === "error" && journal.generation === null);

    // Restore filters from ?from=&to=&acct=&q= once, then mirror changes to the
    // URL (debounced replaceState). onMount's return value is its cleanup.
    onMount(() => startUrlSync());

    // Keyed on the server nonce, not just the URL: reconnecting usually leaves
    // the URL identical (the engine restarted on the same port), and latching on
    // the URL alone is why a reconnect never re-probed (FE-5d). `onServerReady`
    // owns that latch now, for all three pages.
    onServerReady(() => {
        void journal.refresh();
        // Detect the native write endpoints so edit affordances only show
        // against the Ledgeline engine (not a plain, read-only hledger-web).
        void editing.probe();
    });

    // A probe that could not reach the server left `canEdit` a guess rather than
    // a fact (FE-5g). Re-ask on the next round that DID reach it, so a transient
    // blip costs the edit affordances one refresh, not the session. Keyed on
    // `fetchedAt` so it fires once per successful round; `probeError` is read
    // untracked so re-probing cannot re-trigger this effect.
    let lastProbedAt: number | null = null;
    $effect(() => {
        const at = journal.fetchedAt;
        if (at === null || at === lastProbedAt) return;
        lastProbedAt = at;
        if (untrack(() => editing.probeError) !== null) void editing.probe();
    });
</script>

<svelte:head><title>Ledgeline — Journal</title></svelte:head>

<!-- From `md` the page is exactly one window tall and only the transaction
     list scrolls: the window less the chrome above <main> (`--ll-chrome-top`:
     the HTML header, or the iOS app's native bar) and 4rem of padding and gaps.
     Below `md` the filter bar and insights alone are taller than a phone, and
     a fixed height squeezed the list to nothing; there the page scrolls and the
     list (TransactionTable) takes a screenful of its own. -->
<div class="flex min-h-0 flex-col gap-3 md:h-[calc(100dvh-4rem-var(--ll-chrome-top))]">
    <FilterBar accountNames={journal.accountNames} />

    <!-- The summaries are suppressed rather than fed an empty journal: zeroes
         computed from nothing are indistinguishable from zeroes computed from a
         journal that really nets to zero. -->
    {#if !loadFailed}
        <InsightsPanel />
    {/if}

    {#if loadFailed}
        <div class="flex grow items-center justify-center" data-testid="journal-error">
            <!-- `flex` before `flex-col`: `.alert` is a grid with
                 `grid-auto-flow:column`, so `flex-col` alone leaves the
                 headline, the message and the button as three thin columns.
                 See `routes/alertStacking.test.ts`. -->
            <div class="alert flex max-w-xl flex-col items-start gap-2 rounded-box px-4 py-3 text-sm alert-error" role="alert">
                <span class="font-semibold">Couldn't load the journal — no transactions were read.</span>
                <span class="break-words">{journal.error ?? "unknown error"}</span>
                <button type="button" class="btn btn-sm" onclick={() => void journal.refresh({force: true})}>Retry</button>
            </div>
        </div>
    {:else if journal.generation === null}
        <div class="flex grow items-center justify-center" aria-label="Loading transactions">
            <span class="loading loading-lg loading-spinner"></span>
        </div>
    {:else}
        <TransactionTable />
    {/if}

    {#if !loadFailed}
        <TotalsFooter {count} {period} {total} noun={countsRows ? "row" : "transaction"}>
            {#snippet actions()}
                {#if count > 0}
                    <ExportButton run={exportJournal} label="Export journal as xlsx" />
                {/if}
            {/snippet}
        </TotalsFooter>
    {/if}
</div>

<!-- The review overlay (plan 32): mounted while a run is open, so its modal key layer exists exactly then.
     It is the transaction editor too (D31, D39): the txnModal store opens it on one transaction, or on a blank one. -->
{#if matches.open}
    <!-- Find Matches (plan 35 D9): the same kind of layer, opened from the table's toolbar. -->
    <MatchOverlay />
{:else if reviewLaunch.kind !== null}
    <ReviewOverlay kind={reviewLaunch.kind} />
{:else if txnModal.open}
    {#key txnModal.nonce}
        <ReviewOverlay kind={txnModal.mode} index={txnModal.targetIndex} />
    {/key}
{/if}

{#if editing.conflict}
    <div class="toast toast-center toast-top z-40">
        <div class="alert alert-warning">
            <span>The journal changed on disk — the view was refreshed. Re-apply your edit if needed.</span>
            <button type="button" class="btn btn-sm" onclick={() => editing.clearConflict()}>Dismiss</button>
        </div>
    </div>
{/if}

{#if editing.notice !== null}
    <div class="toast toast-end z-40">
        <div class="alert max-w-md alert-error">
            <span class="grow break-words whitespace-pre-wrap">{editing.notice.message}</span>
            <button type="button" class="btn shrink-0 btn-sm" onclick={() => editing.clearNotice()}>Dismiss</button>
        </div>
    </div>
{/if}

<!-- Stale-data case only: when the load failed with nothing to show, the panel
     above says so in full and this would just repeat it. -->
<ErrorToast message={journal.status === "error" && !loadFailed ? journal.error : null} onRetry={() => void journal.refresh({force: true})} />
