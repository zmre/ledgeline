<!-- Track (plan 46 Phase 3): the slot tracker over the app's tracker store,
     which the layout reads on start and after every change on disk. Before
     time tracking is set up, the setup flow instead. `?pick=N` opens the
     account picker for slot N (the app-wide keys found it empty). -->
<script lang="ts">
    import {page} from "$app/state";
    import {tracker} from "#lib/time/track/tracker.svelte.js";
    import SetupView from "#lib/time/track/SetupView.svelte";
    import TrackView from "#lib/time/track/TrackView.svelte";

    const pick = $derived.by((): number | null => {
        const raw = page.url.searchParams.get("pick");
        return raw !== null && /^[0-9]$/.test(raw) ? Number(raw) : null;
    });
</script>

<svelte:head>
    <title>Track · Ledgeline Invoicer</title>
</svelte:head>

{#if tracker.state === null}
    {#if tracker.loadError !== null}
        <div class="mx-auto alert max-w-xl alert-warning" role="alert" data-testid="time-track-error">
            <span>Time tracking couldn't be read. {tracker.loadError}</span>
            <button type="button" class="btn btn-sm" onclick={() => void tracker.refresh()}>Try again</button>
        </div>
    {:else}
        <p class="text-sm text-base-content/60">Loading…</p>
    {/if}
{:else if !tracker.state.configured && tracker.state.setup !== undefined}
    <SetupView offer={tracker.state.setup} api={tracker.api} onDone={() => void tracker.refresh()} />
{:else}
    {#key pick}
        <TrackView {tracker} {pick} />
    {/key}
{/if}
