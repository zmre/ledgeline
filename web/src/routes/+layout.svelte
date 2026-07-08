<script lang="ts">
    import "../app.css";
    import favicon from "$lib/assets/favicon.svg";
    import {resolve} from "$app/paths";
    import {page} from "$app/state";
    import ServerSetupModal from "$lib/components/ServerSetupModal.svelte";
    import {settings} from "$lib/stores/settings.svelte";

    let {children} = $props();
</script>

<svelte:head><link rel="icon" href={favicon} /></svelte:head>

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
                    <a href={resolve("/reports")} class={page.url.pathname.startsWith("/reports") ? "menu-active" : ""}>Reports</a>
                </li>
            </ul>
        </nav>
        <div class="navbar-end pr-2">
            <span id="connection-status" class="flex items-center gap-2 text-sm" title={settings.serverUrl ?? "No hledger-web server configured"}>
                {#if settings.serverUrl !== null}
                    <span class="status status-success" aria-hidden="true"></span>
                    <span class="text-base-content/70 hidden sm:inline">{settings.serverUrl}</span>
                {:else}
                    <span class="status status-error" aria-hidden="true"></span>
                    <span class="text-base-content/70 hidden sm:inline">not connected</span>
                {/if}
            </span>
        </div>
    </header>

    <main class="mx-auto w-full max-w-7xl grow p-4">
        {@render children()}
    </main>
</div>

{#if settings.serverUrl === null}
    <ServerSetupModal />
{/if}
