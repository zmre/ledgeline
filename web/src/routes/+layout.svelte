<script lang="ts">
    import "../app.css";
    import favicon from "$lib/assets/favicon.svg";
    import {resolve} from "$app/paths";
    import {page} from "$app/state";
    import ProblemsBadge from "$lib/checks/ProblemsBadge.svelte";
    import ProblemsDrawer from "$lib/checks/ProblemsDrawer.svelte";
    import ServerSetupModal from "$lib/components/ServerSetupModal.svelte";
    import {journal} from "$lib/stores/journal.svelte";
    import {problems} from "$lib/stores/problems.svelte";
    import {settings} from "$lib/stores/settings.svelte";

    let {children} = $props();

    // WP-08: connection status dot fed by journal.status (green ready / yellow
    // loading / red error), with a reconnect affordance back to the setup modal.
    type ConnState = "none" | "idle" | "loading" | "ready" | "error";
    const conn = $derived<ConnState>(settings.serverUrl === null ? "none" : journal.status);
    const dotClass = $derived(
        conn === "ready" ? "status-success" : conn === "loading" ? "status-warning" : conn === "idle" ? "status-neutral" : "status-error"
    );
    const connLabel = $derived(conn === "none" ? "not connected" : (settings.serverUrl ?? ""));
    const connTitle = $derived(conn === "error" ? (journal.error ?? "connection error") : (settings.serverUrl ?? "No hledger-web server configured"));

    let reconnectOpen = $state(false);
    let lastServerUrl = settings.serverUrl;
    // When the setup modal verifies a NEW url, close the reconnect modal and refetch.
    $effect(() => {
        const url = settings.serverUrl;
        if (url !== lastServerUrl) {
            lastServerUrl = url;
            reconnectOpen = false;
            if (url !== null) void journal.refresh();
        }
    });
</script>

<svelte:head><link rel="icon" href={favicon} /></svelte:head>

<div class="drawer drawer-end">
    <input id="problems-drawer" type="checkbox" class="drawer-toggle" bind:checked={problems.drawerOpen} />
    <div class="drawer-content">
        <div class="bg-base-100 text-base-content flex min-h-screen flex-col">
            <header class="navbar bg-base-200 min-h-12 shadow-sm">
                <div class="navbar-start">
                    <a href={resolve("/")} class="btn btn-ghost px-2 text-lg font-semibold tracking-tight">Ledgeline</a>
                </div>
                <nav class="navbar-center">
                    <ul class="menu menu-horizontal gap-1 px-1">
                        <li>
                            <a href={resolve("/")} class={page.url.pathname === "/" ? "menu-active" : ""}>Journal</a>
                        </li>
                        <li>
                            <a href={resolve("/holdings")} class={page.url.pathname.startsWith("/holdings") ? "menu-active" : ""}>Holdings</a>
                        </li>
                        <li>
                            <a href={resolve("/reports")} class={page.url.pathname.startsWith("/reports") ? "menu-active" : ""}>Reports</a>
                        </li>
                    </ul>
                </nav>
                <div class="navbar-end gap-1 pr-2">
                    <ProblemsBadge />
                    {#if conn !== "none"}
                        <button
                            type="button"
                            class="btn btn-ghost btn-xs btn-circle"
                            title="Refresh journal data now"
                            aria-label="Refresh journal data now"
                            disabled={conn === "loading"}
                            onclick={() => void journal.refresh()}
                        >
                            <svg
                                class="h-4 w-4 {conn === 'loading' ? 'animate-spin' : ''}"
                                xmlns="http://www.w3.org/2000/svg"
                                viewBox="0 0 24 24"
                                fill="none"
                                stroke="currentColor"
                                stroke-width="2"
                                aria-hidden="true"
                            >
                                <path d="M21 12a9 9 0 1 1-2.64-6.36M21 3v6h-6" stroke-linecap="round" stroke-linejoin="round" />
                            </svg>
                        </button>
                    {/if}
                    <span id="connection-status" class="flex items-center gap-2 text-sm" title={connTitle}>
                        <span class="status {dotClass}" aria-hidden="true"></span>
                        <span class="text-base-content/70 hidden sm:inline">{connLabel}</span>
                    </span>
                    {#if conn === "error"}
                        <button type="button" class="btn btn-outline btn-error btn-xs" onclick={() => (reconnectOpen = true)}>Reconnect</button>
                    {/if}
                </div>
            </header>

            <main class="mx-auto w-full max-w-7xl grow p-4">
                {@render children()}
            </main>
        </div>
    </div>
    <ProblemsDrawer />
</div>

{#if settings.serverUrl === null || reconnectOpen}
    <ServerSetupModal />
    {#if reconnectOpen}
        <!-- The first-run modal has no dismiss (a URL is required); when reopened
             as a reconnect affordance the user must be able to bail out. -->
        <button
            type="button"
            class="btn btn-sm btn-circle fixed top-4 right-4 z-[1000]"
            aria-label="Close server setup"
            onclick={() => (reconnectOpen = false)}
        >
            ✕
        </button>
    {/if}
{/if}
