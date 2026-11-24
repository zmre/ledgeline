<!-- The app's sidebar (ARCHITECTURE §6.3): the sections built so far, each
     with its count (invoices: open, and overdue in red), and the open books
     at the foot: their name (as Ledgeline shows it), the journal and the
     basis. Links arrive resolved, so this never needs SvelteKit's router
     (and mounts as is in a component test).

     Narrow windows (below `md`, a container query that follows the UI
     scale): the sidebar becomes a bar across the top, its sections in a
     row that wraps, the books' name beside the app's. -->
<script lang="ts" module>
    /** A section in the sidebar, its link already resolved. */
    export interface SidebarLink {
        id: string;
        href: string;
        label: string;
        active: boolean;
        /** How many (open invoices, customers, …); none shown when absent. */
        count?: number | null;
        /** What `count` counts, for its tooltip and screen readers ("open"). */
        countLabel?: string;
        /** Overdue invoices: shown in red when more than none. */
        overdue?: number | null;
    }
</script>

<script lang="ts">
    import type {BooksInfo} from "#lib/invoices.js";

    let {links, books}: {links: SidebarLink[]; books: BooksInfo | null} = $props();

    /** For screen readers, after the overdue count (a leading space Svelte would trim from markup). */
    const OVERDUE = " overdue";

    const booksName = $derived(books === null ? null : (books.title ?? "").trim() || books.journal || "Books");
</script>

<aside class="flex shrink-0 flex-col border-b border-base-300 bg-base-200 md:h-full md:w-56 md:border-r md:border-b-0" data-testid="sidebar">
    <div class="flex items-baseline gap-2 px-4 pt-3 pb-1 md:pt-4 md:pb-3">
        <span class="text-lg font-semibold tracking-tight whitespace-nowrap">Ledgeline Invoicer</span>
        {#if booksName !== null}
            <span class="ml-auto min-w-0 truncate text-sm text-base-content/70 md:hidden" title={books?.journal ?? ""}>{booksName}</span>
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
                        data-testid="sidebar-{link.id}"
                    >
                        <span>{link.label}</span>
                        {#if (link.count ?? null) !== null || (link.overdue ?? 0) > 0}
                            <span class="ml-auto flex items-center gap-1">
                                {#if (link.overdue ?? 0) > 0}
                                    <span class="badge badge-sm badge-error" title="{link.overdue} overdue" data-testid="sidebar-{link.id}-overdue"
                                        >{link.overdue}<span class="sr-only">{OVERDUE}</span></span
                                    >
                                {/if}
                                {#if (link.count ?? null) !== null}
                                    <span
                                        class="text-xs text-base-content/60 tabular-nums"
                                        title={link.countLabel === undefined ? undefined : `${link.count} ${link.countLabel}`}
                                        data-testid="sidebar-{link.id}-count"
                                        >{link.count}{#if link.countLabel !== undefined}<span class="sr-only">{` ${link.countLabel}`}</span>{/if}</span
                                    >
                                {/if}
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
