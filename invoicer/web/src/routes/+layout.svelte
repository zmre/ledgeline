<script lang="ts">
    import "../app.css";
    import {asset} from "$app/paths";
    import {onMount, untrack} from "svelte";
    import {goto} from "$app/navigation";
    import {resolve} from "$app/paths";
    import {page} from "$app/state";
    import ChordIndicator from "#ui/keys/ChordIndicator.svelte";
    import KeyHelp from "#ui/keys/KeyHelp.svelte";
    import {keymap, registerKeys} from "#ui/keys/keymap.svelte.js";
    import {useAppearance} from "#ui/theme/appearance.svelte.js";
    import {listenForMenuZoom} from "#ui/theme/zoom.svelte.js";
    import {APP_NAME} from "#lib/app.js";
    import {ensureSession, fetchBooks} from "#lib/api.js";
    import {changes, noteChange} from "#lib/changes.svelte.js";
    import {listenForChanges} from "#lib/events.js";
    import type {BooksInfo} from "#lib/invoices.js";
    import {appLayer} from "#lib/keys/appKeys.js";
    import {INVOICER_KEY_GROUPS} from "#lib/keys/groups.js";
    import {uiPrefs} from "#lib/uiPrefs.svelte.js";
    import type {NewChoice} from "#lib/components/NewMenu.svelte";
    import Sidebar from "#lib/components/Sidebar.svelte";
    import {libraryGroup, sectionLinks, type NavHrefs, type NavInput, type SidebarLink} from "#lib/nav.js";
    import {fetchUnbilled} from "#lib/time/api/billing.js";
    import {timeAppLayer} from "#lib/time/keys.js";
    import {elapsedText} from "#lib/time/track/format.js";
    import GuardDialog from "#lib/time/track/GuardDialog.svelte";
    import TimeLamp from "#lib/time/track/TimeLamp.svelte";
    import {TitlePrefix} from "#lib/time/track/titlePrefix.js";
    import {tracker} from "#lib/time/track/tracker.svelte.js";

    let {children} = $props();

    // The shared theme and UI scale, applied the way Ledgeline applies them:
    // defaults until the saved prefs (ui-prefs.toml and the books' pin) load.
    // The zoom keys (in `appLayer`) and the desktop View menu's Zoom items
    // (a `ledgeline:zoom` event) step the saved scale, which is saved; a save
    // still waiting goes out as the page hides.
    useAppearance(uiPrefs);
    $effect(() => listenForMenuZoom());
    $effect(() => uiPrefs.flushWhenLeaving());

    // The keymap (PLAN 5k): packages/ui's engine, Ledgeline's, with the
    // invoicer's own layers. `<svelte:window onkeydown>` below is its one
    // listener; pages and widgets register layers while they are mounted.
    registerKeys(appLayer());
    // Time tracking's app-wide keys (plan 46 K): ⌥⌘digit / Ctrl+Shift+digit
    // toggle a slot from any screen; an empty one opens Track's picker.
    // eslint-disable-next-line svelte/no-navigation-without-resolve -- the path is resolve()d; only the query is added
    registerKeys(timeAppLayer(tracker, {pick: (key) => void goto(`${resolve("/time/track")}?pick=${key}`)}));
    keymap.setGroupOrder(INVOICER_KEY_GROUPS);
    /** The lamp's panel found a slot empty: Track, its picker open for it. */
    // eslint-disable-next-line svelte/no-navigation-without-resolve -- the path is resolve()d; only the query is added
    const pickOnTrack = (key: number): void => void goto(`${resolve("/time/track")}?pick=${key}`);

    let books = $state<BooksInfo | null>(null);
    /** Customers with unbilled time (plan 46 B-unbilled): To bill's count; null before time is set up. */
    let unbilledCustomers = $state<number | null>(null);

    // The sections (lib/nav.ts): Home, Invoices, To bill, Time, Customers,
    // then the Library, collapsible, its open state kept in ui-prefs.toml
    // (`sidebarLibraryOpen`).
    const navHrefs: NavHrefs = {
        home: resolve("/"),
        invoices: resolve("/invoices"),
        toBill: resolve("/to-bill"),
        time: resolve("/time"),
        timeAccounts: resolve("/time/accounts"),
        customers: resolve("/customers"),
        catalog: resolve("/catalog"),
        snippets: resolve("/snippets"),
        templates: resolve("/templates"),
    };
    const nav = $derived<NavInput>({path: page.url.pathname, hrefs: navHrefs, books, unbilledCustomers});
    const links = $derived(sectionLinks(nav));
    const library = $derived(libraryGroup(nav, uiPrefs.libraryOpen, (open) => (uiPrefs.libraryOpen = open)));

    // At the sidebar's foot (SETTINGS-PLAN §4).
    const settingsLink = $derived<SidebarLink>({
        id: "settings",
        href: resolve("/settings"),
        label: "Settings",
        active: page.url.pathname.startsWith(resolve("/settings")),
    });

    // The New button's choices (H20, ARCHITECTURE §6.3): invoices and deposit
    // invoices only among the document types, since the app creates nothing
    // else yet (estimates and credit notes are DocType "later"). `?new=1` asks
    // the page to start one; `/new?deposit=1` starts a deposit invoice. Once
    // time tracking is set up, Bill time… opens To bill on Bill all…
    // (`?bill=all`).
    const newChoices = $derived<NewChoice[]>([
        {id: "invoice", label: "New invoice", href: resolve("/new")},
        {id: "deposit-invoice", label: "New deposit invoice", href: `${resolve("/new")}?deposit=1`},
        {id: "customer", label: "New customer", href: resolve("/customers/new")},
        {id: "catalog-item", label: "New catalog item", href: `${resolve("/catalog")}?new=1`},
        {id: "snippet", label: "New snippet", href: `${resolve("/snippets")}?new=1`},
        {id: "template", label: "New template", href: `${resolve("/templates")}?new=1`},
        ...(tracker.configured === true ? [{id: "bill-time", label: "Bill time…", href: `${resolve("/to-bill")}?bill=all`}] : []),
    ]);

    onMount(() => {
        void uiPrefs.load();
        // The cookie MBR's pages need before any is shown in a frame.
        ensureSession().catch((error: unknown) => console.warn("session:", error));
        // The tracker's state (the lamp), its clock, and a re-read when the
        // window comes back (a timer left running overnight: the guard).
        void tracker.refresh();
        const stopClock = tracker.startClock();
        const onVisible = (): void => {
            if (document.visibilityState === "visible") void tracker.refresh();
        };
        document.addEventListener("visibilitychange", onVisible);
        // Live reload: the pages re-fetch when a file changes on disk.
        const stopListening = listenForChanges({onChange: noteChange});
        return () => {
            stopClock();
            document.removeEventListener("visibilitychange", onVisible);
            stopListening();
        };
    });

    // The time files are watched too: every change re-reads the tracker.
    $effect(() => {
        if (changes.seq === 0) return;
        untrack(() => void tracker.refresh());
    });

    // The window title shows the running timer (T-light).
    const titlePrefix = new TitlePrefix();
    $effect(() => {
        const running = tracker.state?.running ?? null;
        titlePrefix.set(running === null ? null : elapsedText(tracker.elapsed));
    });
    $effect(() => () => titlePrefix.stop());

    // The long-running guard (T-running), on whichever screen is open.
    let guardBusy = $state(false);
    function answerGuard(options: {at?: string; discard?: boolean}): void {
        guardBusy = true;
        void tracker.stop(options).finally(() => (guardBusy = false));
    }

    // To bill's count, once time tracking is set up, again after every change.
    $effect(() => {
        void changes.seq;
        if (tracker.state?.configured !== true) {
            unbilledCustomers = null;
            return;
        }
        fetchUnbilled()
            .then((u) => (unbilledCustomers = u.customers.length))
            .catch((error: unknown) => console.warn("unbilled:", error));
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

<svelte:window onkeydown={keymap.handle} onblur={keymap.disarm} />

<div class="flex h-screen flex-col bg-base-100 text-base-content md:flex-row">
    <Sidebar {links} {books} {newChoices} settings={settingsLink} groups={[library]}>
        {#snippet lamp()}<TimeLamp {tracker} trackHref={resolve("/time/track")} onPick={pickOnTrack} />{/snippet}
    </Sidebar>
    <main class="min-h-0 min-w-0 flex-1 overflow-y-auto p-3 sm:p-4 md:p-6">
        {@render children()}
    </main>
</div>

{#if tracker.guard !== null && tracker.state?.running}
    <GuardDialog
        guard={tracker.guard}
        running={tracker.state.running}
        busy={guardBusy}
        onKeep={() => tracker.keepRunning()}
        onStopAt={(at) => answerGuard({at})}
        onDiscard={() => answerGuard({discard: true})}
    />
{/if}

<KeyHelp
    note="Shortcuts are off while you're typing in a field, except Save (⌘/Ctrl+S), Settings (⌘/Ctrl+,), zoom and the bill table's Alt keys; Esc leaves the field."
/>
<ChordIndicator />
