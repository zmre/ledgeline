<!-- Home's time row, loaded (item 5): `fetchHomeTime` on mount and again
     whenever a file changes on disk, the old figures on screen meanwhile.
     It loads on its own, so the rest of Home never waits for it.

     Nothing shows while it first loads, nor at all when time tracking isn't
     set up or there is no time yet: people who don't track time see no
     empty tiles. A failed first load is the page's usual error alert with
     Retry; a failed reload keeps the figures and only warns. -->
<script lang="ts">
    import {onMount} from "svelte";
    import AsyncSection from "#ui/components/AsyncSection.svelte";
    import type {DataView} from "#ui/ui/dataView.js";
    import {onFilesChanged} from "#lib/changes.svelte.js";
    import {localToday} from "#lib/invoices.js";
    import HomeTimeRow from "./HomeTimeRow.svelte";
    import {fetchHomeTime, type HomeTime, type HomeTimeLinks} from "./homeTime.js";

    let {
        links,
        load = fetchHomeTime,
        today = localToday,
    }: {
        links: HomeTimeLinks;
        /** The figures, or null for no row (tests pass their own). */
        load?: (today: string) => Promise<HomeTime | null>;
        today?: () => string;
    } = $props();

    let time = $state<HomeTime | null>(null);
    /** Loaded, and nothing to show. */
    let hidden = $state(false);
    let error = $state<Error | null>(null);
    const view = $derived<DataView>(error !== null ? "error" : time !== null ? "data" : "loading");

    // Only the latest request's answer is shown.
    let latest = 0;
    function refresh(): void {
        const mine = ++latest;
        load(today())
            .then((answer) => {
                if (mine !== latest) return;
                time = answer;
                hidden = answer === null;
                error = null;
            })
            .catch((e: unknown) => {
                if (mine !== latest) return;
                if (time === null) error = e instanceof Error ? e : new Error(String(e));
                else console.warn("home time:", e);
            });
    }

    onMount(refresh);
    onFilesChanged(refresh);

    function retry(): void {
        error = null;
        refresh();
    }
</script>

{#if !hidden}
    <AsyncSection {view} value={time} {error} testid="home-time-error" label="your time" loadingLabel="Loading your time" onRetry={retry} quietLoading>
        {#snippet children(t)}
            <HomeTimeRow time={t} {links} />
        {/snippet}
    </AsyncSection>
{/if}
