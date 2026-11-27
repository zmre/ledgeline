<!-- The app's sidebar (ARCHITECTURE §6.3): the New split button (H20), the
     sections built so far, each with its counts, and the open books at the
     foot: their name (as Ledgeline shows it), the journal and the basis.
     Links arrive resolved, so this never needs SvelteKit's router (and
     mounts as is in a component test).

     Counts (H17) say what they count, in a tooltip and to screen readers.
     Invoices shows two exclusive ones: overdue in red, then open and not yet
     due; drafts are in neither (the link's tooltip says so).

     Groups follow the main links under a heading of their own (plan 46:
     Time · Overview, Entries, Track, Accounts, Unbilled). The `lamp` is pinned above
     every link (time tracking's running light, T-light).

     Settings sits last, at the foot above the books (one link, pushed down;
     so it is also the last stop on Tab).

     Narrow windows (below `md`, a container query that follows the UI
     scale): the sidebar becomes a bar across the top, its sections in a
     row that wraps (Settings the last of them), the books' name and the
     New button beside the app's. A group's heading isn't shown there, so a
     link may carry a `narrowLabel`. Printing leaves the sidebar out. -->
<script lang="ts" module>
    /** One count beside a section ("2 overdue"). */
    export interface SidebarCount {
        /** Its testid's suffix (`sidebar-<link>-<id>`). */
        id: string;
        value: number;
        /** What it counts, after the number ("overdue", "customers"). */
        label: string;
        /** `error`: a red badge; else quiet text. */
        tone?: "error";
    }

    /** A section in the sidebar, its link already resolved. */
    export interface SidebarLink {
        id: string;
        href: string;
        label: string;
        active: boolean;
        /** Its counts, in order; none shown when empty or absent. */
        counts?: SidebarCount[];
        /** The link's tooltip (every count in words). */
        title?: string;
        /** Its name in the narrow top bar, where the group's heading isn't shown ("Time" for time's Overview). */
        narrowLabel?: string;
        /** Its accessible name when the visible label leans on the group's heading ("Time overview" for Overview under Time). */
        ariaLabel?: string;
    }

    /** A titled run of links after the main ones (plan 46: Time). */
    export interface SidebarGroup {
        id: string;
        title: string;
        links: SidebarLink[];
    }
</script>

<script lang="ts">
    import type {Snippet} from "svelte";
    import type {BooksInfo} from "#lib/invoices.js";
    import NewMenu, {type NewChoice} from "./NewMenu.svelte";

    let {
        links,
        books,
        newChoices = [],
        settings = null,
        groups = [],
        lamp,
    }: {
        links: SidebarLink[];
        books: BooksInfo | null;
        newChoices?: NewChoice[];
        settings?: SidebarLink | null;
        groups?: SidebarGroup[];
        /** Pinned above the links on every screen: time tracking's lamp (plan 46 T-light). */
        lamp?: Snippet;
    } = $props();

    const booksName = $derived(books === null ? null : (books.title ?? "").trim() || books.journal || "Books");
</script>

{#snippet item(link: SidebarLink)}
    <li>
        <!-- eslint-disable svelte/no-navigation-without-resolve -- the layout resolves every link -->
        <a
            href={link.href}
            class={link.active ? "menu-active" : ""}
            aria-current={link.active ? "page" : undefined}
            title={link.title}
            aria-label={link.ariaLabel}
            data-testid="sidebar-{link.id}"
        >
            {#if link.narrowLabel !== undefined}
                <span class="md:hidden">{link.narrowLabel}</span><span class="hidden md:inline">{link.label}</span>
            {:else}
                <span>{link.label}</span>
            {/if}
            {#if (link.counts ?? []).length > 0}
                <span class="ml-auto flex items-center gap-1.5">
                    {#each link.counts ?? [] as count (count.id)}
                        <!-- The words are for screen readers and the tooltip; a leading space Svelte would trim from markup. -->
                        <span
                            class={count.tone === "error" ? "badge badge-sm tabular-nums badge-error" : "text-xs text-base-content/60 tabular-nums"}
                            title="{count.value} {count.label}"
                            data-testid="sidebar-{link.id}-{count.id}">{count.value}<span class="sr-only">{` ${count.label}`}</span></span
                        >
                    {/each}
                </span>
            {/if}
        </a>
        <!-- eslint-enable svelte/no-navigation-without-resolve -->
    </li>
{/snippet}

<aside class="flex shrink-0 flex-col border-b border-base-300 bg-base-200 md:h-full md:w-56 md:border-r md:border-b-0 print:hidden" data-testid="sidebar">
    <div class="flex flex-wrap items-center gap-x-3 gap-y-3 px-4 pt-3 pb-1 md:pt-4 md:pb-2">
        <span class="text-lg font-semibold tracking-tight whitespace-nowrap">Ledgeline Invoicer</span>
        {#if booksName !== null}
            <span class="min-w-0 flex-1 truncate text-sm text-base-content/70 md:hidden" title={books?.journal ?? ""}>{booksName}</span>
        {/if}
        {#if newChoices.length > 0}
            <div class="ml-auto md:ml-0 md:w-full"><NewMenu choices={newChoices} /></div>
        {/if}
    </div>
    {@render lamp?.()}
    <nav aria-label="Sections" class="md:flex md:min-h-0 md:flex-1 md:flex-col">
        <ul class="menu menu-horizontal w-full flex-wrap gap-0.5 px-2 py-1 md:menu-vertical md:flex-1 md:flex-nowrap md:py-2">
            {#each links as link (link.id)}
                {@render item(link)}
            {/each}
            {#each groups as group (group.id)}
                <li class="hidden menu-title pt-3 md:block" data-testid="sidebar-group-{group.id}">{group.title}</li>
                {#each group.links as link (link.id)}
                    {@render item(link)}
                {/each}
            {/each}
            {#if settings !== null}
                <li class="md:mt-auto">
                    <!-- eslint-disable svelte/no-navigation-without-resolve -- the layout resolves it -->
                    <a
                        href={settings.href}
                        class={settings.active ? "menu-active" : ""}
                        aria-current={settings.active ? "page" : undefined}
                        data-testid="sidebar-{settings.id}">{settings.label}</a
                    >
                    <!-- eslint-enable svelte/no-navigation-without-resolve -->
                </li>
            {/if}
        </ul>
    </nav>
    <div class="hidden border-t border-base-300 px-4 py-3 text-xs text-base-content/70 md:block" data-testid="sidebar-books">
        {#if books === null}
            <span>No books open</span>
        {:else}
            <div class="truncate text-sm font-medium text-base-content" title={booksName ?? ""} data-testid="sidebar-books-name">{booksName}</div>
            <div class="mt-0.5 truncate">
                {#if books.journal !== null && books.journal !== booksName}<span title="The main journal">{books.journal}</span> ·
                {/if}<span data-testid="sidebar-basis">{books.basis === "cash" ? "Cash basis" : "Accrual basis"}</span>
            </div>
        {/if}
    </div>
</aside>
