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
    import {invoiceCounts, invoiceCountsTitle, type BooksInfo} from "#lib/invoices.js";
    import type {NewChoice} from "#lib/components/NewMenu.svelte";
    import Sidebar, {type SidebarCount, type SidebarLink} from "#lib/components/Sidebar.svelte";

    let {children} = $props();

    // The shared theme and UI scale, applied the way Ledgeline applies them:
    // defaults until the saved prefs (ui-prefs.toml and the books' pin) load.
    const appearance = $state<{theme: string; uiScale: number; uiPrefsReady: boolean}>({...defaultAppearance()});
    useAppearance(appearance);

    let books = $state<BooksInfo | null>(null);

    const plural = (n: number, one: string, many: string): string => (n === 1 ? one : many);

    const links = $derived.by((): SidebarLink[] => {
        const path = page.url.pathname;
        // Counts beside each section (Phase 4 feedback J11, H17), each saying
        // what it counts. Invoices: overdue (red) and open but not yet due,
        // exclusive, shown when more than none; drafts in the tooltip only.
        const invoices = invoiceCounts(books?.invoices);
        const invoiceBadges: SidebarCount[] =
            invoices === null
                ? []
                : [
                      {id: "overdue", value: invoices.overdue, label: "overdue", tone: "error" as const},
                      {id: "count", value: invoices.notDue, label: "open, not yet due"},
                  ].filter((c) => c.value > 0);
        const total = (id: string, value: number | undefined, one: string, many: string): SidebarCount[] =>
            value === undefined ? [] : [{id, value, label: plural(value, one, many)}];
        // The dashboard is home (PLAN 5a, R1): active only on the root itself.
        const home = resolve("/");
        return [
            {
                id: "dashboard",
                href: home,
                label: "Dashboard",
                active: path.replace(/\/$/, "") === home.replace(/\/$/, ""),
            },
            {
                id: "invoices",
                href: resolve("/invoices"),
                label: "Invoices",
                active: path.startsWith(resolve("/invoices")),
                counts: invoiceBadges,
                title: invoices === null ? undefined : invoiceCountsTitle(invoices),
            },
            {
                id: "customers",
                href: resolve("/customers"),
                label: "Customers",
                active: path.startsWith(resolve("/customers")),
                counts: total("count", books?.customers, "customer", "customers"),
            },
            {
                id: "catalog",
                href: resolve("/catalog"),
                label: "Catalog",
                active: path.startsWith(resolve("/catalog")),
                counts: total("count", books?.catalog, "item", "items"),
            },
            {
                id: "snippets",
                href: resolve("/snippets"),
                label: "Snippets",
                active: path.startsWith(resolve("/snippets")),
                counts: total("count", books?.snippets, "snippet", "snippets"),
            },
        ];
    });

    // The New button's choices (H20): invoices only among the document
    // types, since the app creates nothing else yet (estimates and credit
    // notes are DocType "later"). `?new=1` asks the page to start one.
    const newChoices: NewChoice[] = [
        {id: "invoice", label: "New invoice", href: resolve("/new")},
        {id: "customer", label: "New customer", href: resolve("/customers/new")},
        {id: "catalog-item", label: "New catalog item", href: `${resolve("/catalog")}?new=1`},
        {id: "snippet", label: "New snippet", href: `${resolve("/snippets")}?new=1`},
    ];

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

<div class="flex h-screen flex-col bg-base-100 text-base-content md:flex-row">
    <Sidebar {links} {books} {newChoices} />
    <main class="min-h-0 min-w-0 flex-1 overflow-y-auto p-3 sm:p-4 md:p-6">
        {@render children()}
    </main>
</div>
