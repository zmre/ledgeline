<!-- The header's bank sync dot (plan 43 §4.1): hidden while nothing is linked;
     a spinner while syncing; amber when something needs attention; red for a
     broken connection. A link to Imports → Accounts, with the status line as
     its name. -->
<script lang="ts">
    import {statusLine} from "../statusLine";
    import {syncStore} from "../syncStore.svelte";
    import {importsHref} from "./effects";

    const DOT = {idle: "status-neutral", attention: "status-warning", broken: "status-error"} as const;
    const WORDS = {idle: "Bank sync", syncing: "Bank sync: syncing", attention: "Bank sync needs attention", broken: "Bank sync is broken"} as const;

    // The wall clock a minute at a time, for "last synced 7:12 AM".
    let now = $state(Math.floor(Date.now() / 1000));
    $effect(() => {
        const timer = setInterval(() => (now = Math.floor(Date.now() / 1000)), 60_000);
        return () => clearInterval(timer);
    });

    const dot = $derived(syncStore.dot);
    const line = $derived(syncStore.status === null ? null : statusLine(syncStore.status, now));
    const label = $derived(dot === "hidden" ? "" : line === null ? WORDS[dot] : `${WORDS[dot]} — ${line}`);
</script>

{#if dot !== "hidden"}
    <!-- eslint-disable-next-line svelte/no-navigation-without-resolve -- importsHref resolves the route; the query string is appended -->
    <a href={importsHref("accounts")} class="btn btn-circle btn-ghost btn-xs" title={label} aria-label={label} data-testid="sync-dot" data-state={dot}>
        {#if dot === "syncing"}
            <span class="loading loading-xs loading-spinner" aria-hidden="true"></span>
        {:else}
            <!-- arrows-right-left: bank sync -->
            <span class="indicator">
                <span class="indicator-item status {DOT[dot]}" aria-hidden="true"></span>
                <svg
                    xmlns="http://www.w3.org/2000/svg"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="2"
                    class="h-4 w-4"
                    aria-hidden="true"
                >
                    <path stroke-linecap="round" stroke-linejoin="round" d="M7.5 21 3 16.5m0 0L7.5 12M3 16.5h13.5m0-13.5L21 7.5m0 0L16.5 12M21 7.5H7.5" />
                </svg>
            </span>
        {/if}
    </a>
{/if}
