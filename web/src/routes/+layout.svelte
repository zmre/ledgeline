<script lang="ts">
    import "../app.css";
    import {afterNavigate, goto} from "$app/navigation";
    import {resolve} from "$app/paths";
    import {page} from "$app/state";
    import ProblemsBadge from "#lib/checks/ProblemsBadge.svelte";
    import ProblemsDrawer from "#lib/checks/ProblemsDrawer.svelte";
    import ServerSetupModal from "#lib/components/ServerSetupModal.svelte";
    import {rulesStore} from "#lib/imports/rulesStore.svelte.js";
    import ChordIndicator from "#lib/keys/ChordIndicator.svelte";
    import KeyHelp from "#lib/keys/KeyHelp.svelte";
    import {keymap, registerKeys} from "#lib/keys/keymap.svelte.js";
    import {globalLayer} from "#lib/keys/navBindings.js";
    import {
        GLOBAL_ACTION_ORDER,
        installNativeBridge,
        noteNavigation,
        publishPage,
        registerNativeActions,
        registerNativeOverlay,
        type NativeAction,
        type SectionId,
    } from "#lib/native/bridge.svelte.js";
    import {isNative} from "#lib/native/detect.js";
    import {localToday} from "#lib/stores/filters.svelte.js";
    import {journal, startPolling} from "#lib/stores/journal.svelte.js";
    import {problems} from "#lib/stores/problems.svelte.js";
    import {refreshEverything} from "#lib/stores/refreshAll.js";
    import {onServerReady} from "#lib/stores/serverWatch.svelte.js";
    import {settings} from "#lib/stores/settings.svelte.js";
    import {useAppearance} from "#lib/theme/appearance.svelte.js";
    import {listenForMenuZoom} from "#lib/theme/zoom.svelte.js";
    import {connectionLabel, connectionTooltip, type ConnState} from "#lib/ui/connectionLabel.js";

    let {children} = $props();

    // The header stays pinned at the top so the ledger name and the section
    // tabs are always in view. Its height is the page's scroll padding, so
    // every `scrollIntoView` and anchor jump lands just below it rather than
    // underneath it — measured, not hard-coded, because it grows with the
    // interface scale.
    // In the iOS app the header is collapsed to nothing and the native bar's
    // height is the top safe-area inset, so that is added (0 everywhere else).
    let headerHeight = $state(0);
    $effect(() => {
        document.documentElement.style.scrollPaddingTop = `calc(${headerHeight}px + env(safe-area-inset-top, 0px))`;
    });

    // Inside the iOS/iPadOS app (plan 42). Set before any script runs and never
    // changes for the life of the document, so a constant.
    const native = isNative();

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

    // Theme and interface scale (plan 39): applied from the saved prefs and the
    // OS colour scheme, with the zoom shortcuts stepping the saved scale. The
    // keyboard half of zoom is in `globalLayer()` above; this is the desktop
    // View menu's half, which arrives as a `ledgeline:zoom` event on `window`.
    useAppearance();
    // Saves still waiting on their debounce go out as the page is hidden or
    // unloaded (a reload, a closed tab), with a request that outlives it.
    $effect(() => settings.flushUiPrefsWhenLeaving());
    $effect(() => listenForMenuZoom());

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

    // The section tabs, drawn as the header's tab row on a wide window and as
    // its menu on a narrow one. Imports hides on an engine with no rules route,
    // and in the iOS app, which cannot run imports at all (plan 42).
    const sections = $derived.by(() => {
        const path = page.url.pathname;
        const all = [
            {id: "journal", href: resolve("/"), label: "Journal", active: path === "/"},
            {id: "holdings", href: resolve("holdings"), label: "Holdings", active: path.startsWith("/holdings")},
            {id: "budget", href: resolve("budget"), label: "Budget", active: path.startsWith("/budget")},
            {id: "projections", href: resolve("projections"), label: "Projections", active: path.startsWith("/projections")},
            {id: "reports", href: resolve("reports"), label: "Reports", active: path.startsWith("/reports")},
            {id: "imports", href: resolve("imports"), label: "Imports", active: path.startsWith("/imports")},
        ] as const;
        return rulesStore.available && !native ? all : all.filter((s) => s.id !== "imports");
    });
    const onSettings = $derived(page.url.pathname.startsWith("/settings"));
    const currentSection = $derived(sections.find((s) => s.active)?.label ?? (onSettings ? "Settings" : "Menu"));

    let reconnectOpen = $state(false);
    let storageNoticeDismissed = $state(false);
    let uiNoticeDismissed = $state(false);

    // The view state (columns, open panels, review numbers, …) lives on the
    // engine: global, with per-ledger overrides (plan 38 WP-5). Loaded once per engine connection —
    // keyed on the nonce, like the journal refresh below, so a reconnect to a
    // restarted engine reloads it. The page waits for the FIRST load (it
    // settles within a few seconds even when the engine cannot answer), so no
    // screen paints, or fetches for, a panel the ledger has closed.
    $effect(() => {
        const serverUrl = settings.serverUrl;
        void settings.serverNonce;
        if (serverUrl === null) return;
        void settings.loadUiPrefs(serverUrl);
    });
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

    // WP-08: live updates on EVERY screen. This used to run on the journal page
    // only, so a report, holdings or budget screen never heard that the file
    // had changed on disk and kept showing the old figures until you left and
    // came back. A round is one small `/api/journal` request the engine answers
    // from its snapshot; a changed generation is what makes each screen refetch
    // (`journal.changes`). startPolling pauses while the window is hidden; the
    // returned stop fn is the effect cleanup (runs if the server URL changes).
    $effect(() => {
        if (settings.serverUrl === null) return;
        return startPolling();
    });

    // The problems badge is always on screen, so its list is fetched HERE, once
    // per journal generation: the engine runs every check (plan 26 B4), as of
    // the browser's local date. `problems.load` dedupes on (server, generation,
    // today), so this re-running on unrelated changes costs nothing.
    $effect(() => {
        const serverUrl = settings.serverUrl;
        const generation = journal.generation;
        if (serverUrl === null || generation === null) return;
        void problems.load(serverUrl, generation, localToday());
    });

    // ---- The iOS app's native chrome (plan 42) — all of it inert elsewhere. ----
    //
    // Imports and the new-ledger wizard need a desktop host (an hledger
    // subprocess, a folder to create a ledger in), so in the app they are not
    // routes at all: nothing renders there and the URL moves to the journal.
    const nativeBlocked = $derived(native && (page.url.pathname.startsWith("/imports") || page.url.pathname.startsWith("/new")));
    $effect(() => {
        if (nativeBlocked) void goto(resolve("/"), {replace: true});
    });

    const sectionHrefs: Record<SectionId, string> = {
        journal: resolve("/"),
        holdings: resolve("holdings"),
        budget: resolve("budget"),
        projections: resolve("projections"),
        reports: resolve("reports"),
        settings: resolve("settings"),
    };
    $effect(() =>
        installNativeBridge({
            navigate: (id) => {
                // eslint-disable-next-line svelte/no-navigation-without-resolve -- every value in sectionHrefs IS a resolve() of its route
                if (page.url.pathname !== sectionHrefs[id]) void goto(sectionHrefs[id]);
            },
            // The app's one Refresh re-reads the files itself, then asks for this.
            refresh: () => void refreshEverything(),
        })
    );
    afterNavigate((navigation) => {
        if (native) noteNavigation(navigation.type, "delta" in navigation ? navigation.delta : undefined);
    });

    // What the header's right-hand icons did, as menu actions after the page's own.
    registerNativeActions(
        () => {
            const actions: NativeAction[] = [];
            // In the app, Refresh is the native menu's own item (it re-reads the files first).
            if (conn !== "none" && !native)
                actions.push({id: "refresh", label: "Refresh everything", icon: "arrow.clockwise", run: () => void refreshEverything()});
            actions.push({
                id: "problems",
                label: problems.count > 0 ? `Problems (${problems.count})` : "Problems",
                icon: "exclamationmark.triangle",
                run: () => (problems.drawerOpen = true),
            });
            return actions;
        },
        {order: GLOBAL_ACTION_ORDER}
    );
    registerNativeOverlay(
        () => problems.drawerOpen,
        () => (problems.drawerOpen = false)
    );

    // The page's state for the native bar: re-posted (deduped) whenever the
    // section, an action, an open overlay or the history depth changes.
    $effect(() => {
        if (!native) return;
        publishPage({
            title: currentSection === "Menu" ? "Ledgeline" : currentSection,
            sections: [
                ...sections.flatMap(({id, label, active}) => (id === "imports" ? [] : [{id, label, active}])),
                {id: "settings" as const, label: "Settings", active: onSettings},
            ],
        });
    });
</script>

{#snippet sectionLinks(inMenu: boolean)}
    {#each sections as section (section.label)}
        <li>
            <!-- A menu link blurs itself: the dropdown is open while focus is inside it. -->
            <a href={section.href} class={section.active ? "menu-active" : ""} onclick={inMenu ? (e) => e.currentTarget.blur() : undefined}>{section.label}</a>
        </li>
    {/each}
{/snippet}

<svelte:head><link rel="icon" type="image/svg+xml" href="/ledgeline-icon-small.svg" /></svelte:head>

<!-- `onblur` disarms a half-typed chord on Cmd-Tab, so returning to the window
     does not find `g` still armed from minutes ago. -->
<svelte:window onkeydown={keymap.handle} onblur={keymap.disarm} />

<div class="drawer drawer-end">
    <input id="problems-drawer" type="checkbox" class="drawer-toggle" bind:checked={problems.drawerOpen} />
    <div class="drawer-content">
        <div class="flex min-h-screen flex-col bg-base-100 text-base-content">
            <header class="app-header navbar sticky top-0 z-30 min-h-12 bg-base-200 shadow-sm" bind:clientHeight={headerHeight} data-testid="app-header">
                <div class="navbar-start">
                    <a href={resolve("/")} class="btn gap-2 btn-ghost px-2 text-lg font-semibold tracking-tight" aria-label="Ledgeline">
                        <img src="/ledgeline-icon-small.svg" alt="" class="h-7 w-7 rounded" />
                        <span class="hidden md:inline">Ledgeline</span>
                    </a>
                    <!-- Below `md` (48rem — in rem, so a big interface scale gets
                         here on a wide window too) the six tabs no longer fit
                         beside the logo and the icons, so they fold into one
                         menu named for the current section. -->
                    <div class="dropdown md:hidden" data-testid="nav-menu">
                        <div tabindex="0" role="button" class="btn gap-1 btn-ghost px-2 btn-sm" aria-label="Sections: {currentSection}">
                            {currentSection}
                            <svg
                                xmlns="http://www.w3.org/2000/svg"
                                viewBox="0 0 24 24"
                                fill="none"
                                stroke="currentColor"
                                stroke-width="2"
                                class="h-4 w-4"
                                aria-hidden="true"
                            >
                                <path stroke-linecap="round" stroke-linejoin="round" d="m6 9 6 6 6-6" />
                            </svg>
                        </div>
                        <ul tabindex="-1" class="menu dropdown-content z-40 mt-2 w-max rounded-box bg-base-200 p-2 shadow-lg">
                            {@render sectionLinks(true)}
                        </ul>
                    </div>
                </div>
                <nav class="navbar-center hidden md:flex">
                    <ul class="menu menu-horizontal gap-1 px-1">
                        {@render sectionLinks(false)}
                    </ul>
                </nav>
                <div class="navbar-end gap-1 pr-2">
                    <a
                        href={resolve("settings")}
                        class="btn btn-circle btn-ghost btn-xs {page.url.pathname.startsWith('/settings') ? 'btn-active' : ''}"
                        title="Settings"
                        aria-label="Settings"
                    >
                        <svg
                            xmlns="http://www.w3.org/2000/svg"
                            viewBox="0 0 24 24"
                            fill="none"
                            stroke="currentColor"
                            stroke-width="2"
                            class="h-4 w-4"
                            aria-hidden="true"
                        >
                            <path
                                stroke-linecap="round"
                                stroke-linejoin="round"
                                d="M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 0 0 2.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 0 0 1.065 2.572c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 0 0-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 0 0-2.572 1.065c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 0 0-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 0 0-1.065-2.572c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 0 0 1.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065Z"
                            />
                            <path stroke-linecap="round" stroke-linejoin="round" d="M15 12a3 3 0 1 1-6 0 3 3 0 0 1 6 0Z" />
                        </svg>
                    </a>
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
                            class="btn btn-circle btn-ghost btn-xs"
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
                            <span class="hidden max-w-64 truncate text-base-content/70 sm:inline">{connLabel}</span>
                        {/if}
                    </span>
                    {#if conn === "error"}
                        <button type="button" class="btn btn-outline btn-error btn-xs" onclick={() => (reconnectOpen = true)}>Reconnect</button>
                    {/if}
                </div>
            </header>

            <!-- 100rem: 25% wider than Tailwind's max-w-7xl (80rem), for the calendar and wide tables on big screens. -->
            <main class="mx-auto w-full max-w-[100rem] grow p-4">
                {#if (settings.serverUrl === null || settings.uiPrefsReady) && !nativeBlocked}
                    {@render children()}
                {/if}
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
        <div class="alert max-w-md alert-warning" data-testid="settings-storage-error">
            <span class="grow break-words">{settings.storageError}</span>
            <button type="button" class="btn shrink-0 btn-sm" onclick={() => (storageNoticeDismissed = true)}>Dismiss</button>
        </div>
    </div>
{/if}

<!-- Settings-file values the engine could not use (a hand edit, a cloned
     ledgeline.toml), changes it could only keep for the session (a symlinked
     ui-prefs.toml), or a view-settings save that failed. -->
{#if settings.uiPrefsNotices.length > 0 && !uiNoticeDismissed}
    <div class="toast toast-start z-40">
        <div class="alert max-w-md alert-warning" data-testid="ui-prefs-notices">
            <div class="grow break-words">
                <p>About your view settings:</p>
                <ul class="list-disc pl-5 text-sm">
                    {#each settings.uiPrefsNotices.slice(0, 5) as notice (notice)}
                        <li>{notice}</li>
                    {/each}
                </ul>
            </div>
            <button type="button" class="btn shrink-0 btn-sm" onclick={() => (uiNoticeDismissed = true)}>Dismiss</button>
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
            class="btn fixed top-4 right-4 z-[1000] btn-circle btn-sm"
            aria-label="Close server setup"
            onclick={() => (reconnectOpen = false)}
        >
            ✕
        </button>
    {/if}
{/if}
