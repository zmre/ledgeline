<!-- The app's sidebar (ARCHITECTURE §6.3): the New split button (H20), the
     sections built so far, each with its counts, and the open books at the
     foot: their name (as Ledgeline shows it), the journal and the basis.
     Links arrive resolved, so this never needs SvelteKit's router (and
     mounts as is in a component test).

     Counts (H17) say what they count, in a tooltip and to screen readers.
     Invoices shows two exclusive ones: overdue in red, then open and not yet
     due; drafts are in neither (the link's tooltip says so).

     Narrow windows (below `md`, a container query that follows the UI
     scale): the sidebar becomes a bar across the top, its sections in a
     row that wraps, the books' name and the New button beside the app's. -->
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
    }
</script>

<script lang="ts">
    import type {BooksInfo} from "#lib/invoices.js";
    import NewMenu, {type NewChoice} from "./NewMenu.svelte";

    let {links, books, newChoices = []}: {links: SidebarLink[]; books: BooksInfo | null; newChoices?: NewChoice[]} = $props();

    const booksName = $derived(books === null ? null : (books.title ?? "").trim() || books.journal || "Books");
</script>

<aside class="flex shrink-0 flex-col border-b border-base-300 bg-base-200 md:h-full md:w-56 md:border-r md:border-b-0" data-testid="sidebar">
    <div class="flex flex-wrap items-center gap-x-3 gap-y-3 px-4 pt-3 pb-1 md:pt-4 md:pb-2">
        <span class="text-lg font-semibold tracking-tight whitespace-nowrap">Ledgeline Invoicer</span>
        {#if booksName !== null}
            <span class="min-w-0 flex-1 truncate text-sm text-base-content/70 md:hidden" title={books?.journal ?? ""}>{booksName}</span>
        {/if}
        {#if newChoices.length > 0}
            <div class="ml-auto md:ml-0 md:w-full"><NewMenu choices={newChoices} /></div>
        {/if}
    </div>
    <nav aria-label="Sections">
        <ul class="menu menu-horizontal w-full flex-wrap gap-0.5 px-2 py-1 md:menu-vertical md:flex-nowrap md:py-2">
            {#each links as link (link.id)}
                <li>
                    <!-- eslint-disable svelte/no-navigation-without-resolve -- the layout resolves every link -->
                    <a
                        href={link.href}
                        class={link.active ? "menu-active" : ""}
                        aria-current={link.active ? "page" : undefined}
                        title={link.title}
                        data-testid="sidebar-{link.id}"
                    >
                        <span>{link.label}</span>
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
            {/each}
        </ul>
    </nav>
    <div class="mt-auto hidden border-t border-base-300 px-4 py-3 text-xs text-base-content/70 md:block" data-testid="sidebar-books">
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
