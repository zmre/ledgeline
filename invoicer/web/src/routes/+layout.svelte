<script lang="ts">
    import "../app.css";
    import {asset} from "$app/paths";
    import {onMount} from "svelte";
    import {resolve} from "$app/paths";
    import {page} from "$app/state";
    import {useAppearance} from "#ui/theme/appearance.svelte.js";
    import {APP_NAME, defaultAppearance} from "#lib/app.js";
    import {appearanceFrom, ensureSession, fetchBooks, fetchUiPrefs} from "#lib/api.js";
    import {changes, noteChange} from "#lib/changes.svelte.js";
    import {listenForChanges} from "#lib/events.js";
    import type {BooksInfo} from "#lib/invoices.js";
    import Sidebar, {type SidebarLink} from "#lib/components/Sidebar.svelte";

    let {children} = $props();

    // The shared theme and UI scale, applied the way Ledgeline applies them:
    // defaults until the saved prefs (ui-prefs.toml and the books' pin) load.
    const appearance = $state<{theme: string; uiScale: number; uiPrefsReady: boolean}>({...defaultAppearance()});
    useAppearance(appearance);

    let books = $state<BooksInfo | null>(null);

    const links = $derived.by((): SidebarLink[] => {
        const path = page.url.pathname;
        return [
            {id: "new", href: resolve("/new"), label: "New invoice", active: path === resolve("/new")},
            {id: "invoices", href: resolve("/invoices"), label: "Invoices", active: path.startsWith(resolve("/invoices"))},
        ];
    });

    onMount(() => {
        fetchUiPrefs()
            .then((body) => {
                const saved = appearanceFrom(body);
                if (saved !== null) {
                    appearance.theme = saved.theme;
                    appearance.uiScale = saved.uiScale;
                }
                appearance.uiPrefsReady = true;
            })
            .catch((error: unknown) => console.warn("ui prefs:", error));
        // The cookie MBR's pages need before any is shown in a frame.
        ensureSession().catch((error: unknown) => console.warn("session:", error));
        // Live reload: the pages re-fetch when a file changes on disk.
        return listenForChanges({onChange: noteChange});
    });

    // The books at the foot of the sidebar, again after every change.
    $effect(() => {
        void changes.seq;
        fetchBooks()
            .then((info) => (books = info))
            .catch((error: unknown) => console.warn("books:", error));
    });
</script>

<svelte:head>
    <title>{APP_NAME}</title>
    <link rel="icon" type="image/svg+xml" href={asset("ledgeline-invoicer-icon-small.svg")} />
</svelte:head>

<div class="flex h-screen bg-base-100 text-base-content">
    <Sidebar {links} {books} />
    <main class="min-w-0 flex-1 overflow-y-auto p-4 md:p-6">
        {@render children()}
    </main>
</div>
