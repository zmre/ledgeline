<!-- The app's sidebar (ARCHITECTURE §6.3): the sections built so far, and
     the open books at the foot. Links arrive resolved, so this never needs
     SvelteKit's router (and mounts as is in a component test). -->
<script lang="ts" module>
    /** A section in the sidebar, its link already resolved. */
    export interface SidebarLink {
        id: string;
        href: string;
        label: string;
        active: boolean;
    }
</script>

<script lang="ts">
    import type {BooksInfo} from "#lib/invoices.js";

    let {links, books}: {links: SidebarLink[]; books: BooksInfo | null} = $props();
</script>

<aside class="flex h-full w-56 shrink-0 flex-col border-r border-base-300 bg-base-200" data-testid="sidebar">
    <div class="px-4 pt-4 pb-3 text-lg font-semibold tracking-tight">Ledgeline Invoicer</div>
    <nav aria-label="Sections">
        <ul class="menu w-full gap-0.5 px-2">
            {#each links as link (link.id)}
                <li>
                    <!-- eslint-disable-next-line svelte/no-navigation-without-resolve -- the layout resolves every link -->
                    <a href={link.href} class={link.active ? "menu-active" : ""} aria-current={link.active ? "page" : undefined}>{link.label}</a>
                </li>
            {/each}
        </ul>
    </nav>
    <div class="mt-auto border-t border-base-300 px-4 py-3 text-xs text-base-content/70" data-testid="sidebar-books">
        {#if books === null}
            <span>No books open</span>
        {:else}
            <div class="truncate font-medium text-base-content" title={books.journal ?? ""}>{books.journal ?? "Books"}</div>
            <div class="mt-1 flex items-center gap-2">
                <span class="badge badge-outline badge-xs">{books.basis === "cash" ? "Cash basis" : "Accrual basis"}</span>
                <span>{books.documents} {books.documents === 1 ? "document" : "documents"}</span>
            </div>
        {/if}
    </div>
</aside>
