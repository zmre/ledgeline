<script lang="ts">
    import "../app.css";
    import favicon from "$lib/assets/favicon.svg";
    import {resolve} from "$app/paths";
    import {page} from "$app/state";
    import ProblemsBadge from "$lib/checks/ProblemsBadge.svelte";
    import ProblemsDrawer from "$lib/checks/ProblemsDrawer.svelte";
    import ServerSetupModal from "$lib/components/ServerSetupModal.svelte";
    import {rulesStore} from "$lib/imports/rulesStore.svelte";
    import ChordIndicator from "$lib/keys/ChordIndicator.svelte";
    import KeyHelp from "$lib/keys/KeyHelp.svelte";
    import {keymap, registerKeys} from "$lib/keys/keymap.svelte";
    import {globalLayer} from "$lib/keys/navBindings";
    import {journal} from "$lib/stores/journal.svelte";
    import {problems} from "$lib/stores/problems.svelte";
    import {refreshEverything} from "$lib/stores/refreshAll";
    import {onServerReady} from "$lib/stores/serverWatch.svelte";
    import {settings} from "$lib/stores/settings.svelte";
    import {connectionLabel, connectionTooltip, type ConnState} from "$lib/ui/connectionLabel";

    let {children} = $props();

    // The Imports nav item is hidden on an engine that has no `/api/rules` route
    // at all (it 404s, which the client reports as `NativeApiUnavailableError`).
    // `available` starts TRUE, so the item does not blink out of existence
    // during every ordinary load — the same reasoning as `editing.probe`'s
    // refusal to read an unanswered probe as a "no" (FE-5g). The listing this
    // fetches is also the one the Imports page renders, and `ensureIndex`
    // dedupes on (server, reconnect), so this is a prefetch and not a second
    // directory walk.
    onServerReady((url) => void rulesStore.ensureIndex(url, settings.serverNonce));

    // The app's global keymap. `<svelte:window onkeydown>` below is the only
    // key listener in the app that is always attached; everything else registers
    // a layer for as long as its component is mounted. See keymap.svelte.ts for
    // why bubble phase (and not a capture-phase document listener) is what keeps
    // the four pre-existing element handlers working untouched.
    registerKeys(globalLayer());

    // WP-08: connection status dot fed by journal.status (green ready / yellow
    // loading / red error), with a reconnect affordance back to the setup modal.
    const conn = $derived<ConnState>(settings.serverUrl === null ? "none" : journal.status);
    const dotClass = $derived(
        conn === "ready" ? "status-success" : conn === "loading" ? "status-warning" : conn === "idle" ? "status-neutral" : "status-error"
    );
    // The label names the JOURNAL, not the server — see connectionLabel.ts for
    // the fallback chain and why the URL is now only its last rung.
    const connLabel = $derived(connectionLabel(conn, journal.title));
    const connTitle = $derived(connectionTooltip(conn, journal.file, settings.serverUrl, journal.error));

    let reconnectOpen = $state(false);
    let storageNoticeDismissed = $state(false);
    let lastVerified = settings.serverNonce;
    // Every SUCCESSFUL verification refetches — keyed on settings.serverNonce,
    // not on the URL. Reconnecting to the same address (the overwhelmingly
    // common case: the engine restarted on the same port) left the URL
    // unchanged, so this effect never fired and neither did the pages' own
    // `url !== attemptedUrl` guards — the Reconnect button did nothing at all.
    // `force` because the round it needs to replace is the hung one (FE-5d).
    $effect(() => {
        const nonce = settings.serverNonce;
        if (nonce === lastVerified) return;
        lastVerified = nonce;
        reconnectOpen = false;
        if (settings.serverUrl !== null) void journal.refresh({force: true});
    });
</script>

<svelte:head><link rel="icon" href={favicon} /></svelte:head>

<!-- `onblur` disarms a half-typed chord on Cmd-Tab, so returning to the window
     does not find `g` still armed from minutes ago. -->
<svelte:window onkeydown={keymap.handle} onblur={keymap.disarm} />

<div class="drawer drawer-end">
    <input id="problems-drawer" type="checkbox" class="drawer-toggle" bind:checked={problems.drawerOpen} />
    <div class="drawer-content">
        <div class="bg-base-100 text-base-content flex min-h-screen flex-col">
            <header class="navbar bg-base-200 min-h-12 shadow-sm">
                <div class="navbar-start">
                    <a href={resolve("/")} class="btn btn-ghost gap-2 px-2 text-lg font-semibold tracking-tight">
                        <img src="/ledgeline-icon.png" alt="Ledgeline" class="h-7 w-7 rounded" />
                        Ledgeline
                    </a>
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
                        {#if rulesStore.available}
                            <li>
                                <a href={resolve("/imports")} class={page.url.pathname.startsWith("/imports") ? "menu-active" : ""}>Imports</a>
                            </li>
                        {/if}
                    </ul>
                </nav>
                <div class="navbar-end gap-1 pr-2">
                    <ProblemsBadge />
                    {#if conn !== "none"}
                        <!-- Every resource on screen, not just the journal —
                             `refreshAll.ts` says what "every" means and why the
                             journal alone was the wrong promise for this icon.
                             The spinner still hangs off `journal.status`: it is
                             the long one, and the other four are a round trip
                             each. -->
                        <button
                            type="button"
                            class="btn btn-ghost btn-xs btn-circle"
                            title="Refresh everything on screen now"
                            aria-label="Refresh everything on screen now"
                            disabled={conn === "loading"}
                            onclick={() => void refreshEverything()}
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
                        <!-- Rendered only when there is a ledger to name: an
                             engine that cannot say leaves the dot alone rather
                             than an empty span holding a `gap-2` open. Capped
                             and ellipsised because a title runs to 60 characters
                             and an uncapped one squeezes the centre nav. -->
                        {#if connLabel !== ""}
                            <span class="text-base-content/70 hidden max-w-64 truncate sm:inline">{connLabel}</span>
                        {/if}
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

<KeyHelp />
<ChordIndicator />

<!-- Corrupt localStorage used to drop the saved server URL and every column
     preference in silence, so the app just reappeared at first-run setup. -->
{#if settings.storageError !== null && !storageNoticeDismissed}
    <div class="toast toast-start z-40">
        <div class="alert alert-warning max-w-md" data-testid="settings-storage-error">
            <span class="grow break-words">{settings.storageError}</span>
            <button type="button" class="btn btn-sm shrink-0" onclick={() => (storageNoticeDismissed = true)}>Dismiss</button>
        </div>
    </div>
{/if}

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
