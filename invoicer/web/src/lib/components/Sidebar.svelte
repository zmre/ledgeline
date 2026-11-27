<!-- The app's sidebar (ARCHITECTURE §6.3): the New split button (H20), the
     sections built so far, each with its counts, and the open books at the
     foot: their name (as Ledgeline shows it), the journal and the basis.
     Links arrive resolved, so this never needs SvelteKit's router (and
     mounts as is in a component test).

     Counts (H17) say what they count, in a tooltip and to screen readers.
     Invoices shows two exclusive ones: overdue in red, then open and not yet
     due; drafts are in neither (the link's tooltip says so).

     Groups follow the main links. A plain group has a heading of its own; a
     collapsible one (the Library: Catalog, Snippets, Templates, Time
     accounts) is a button with `aria-expanded` that shows or hides its
     links, indented under it. Its open state is the layout's to keep
     (`open`, `onToggle`); it opens by itself while one of its pages is the
     current one, unless closed there (until another of its pages is
     opened). The `lamp` is pinned above every link (time tracking's running
     light, T-light).

     Settings sits last, at the foot above the books (one link, pushed down;
     so it is also the last stop on Tab).

     Narrow windows (below `md`, a container query that follows the UI
     scale): the sidebar becomes a bar across the top, its sections in a
     row that wraps (Settings the last of them), the books' name and the
     New button beside the app's. A plain group's heading isn't shown there,
     so a link may carry a `narrowLabel`; a collapsible group's button is,
     its links following it in the row. Printing leaves the sidebar out. -->

<script lang="ts">
    import type {Snippet} from "svelte";
    import type {BooksInfo} from "#lib/invoices.js";
    // The shapes are lib/nav.ts's, which builds them.
    import type {SidebarGroup, SidebarLink} from "#lib/nav.js";
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

    /** A collapsible group closed while one of its pages was current: group id → that link's id. */
    let closedOn = $state<Record<string, string>>({});

    const activeIn = (group: SidebarGroup): string | null => group.links.find((l) => l.active)?.id ?? null;

    /** Whether a group's links show: always for a plain one; else as kept, or while one of them is current. */
    function isOpen(group: SidebarGroup): boolean {
        if (group.onToggle === undefined || group.open === true) return true;
        const active = activeIn(group);
        return active !== null && closedOn[group.id] !== active;
    }

    function toggle(group: SidebarGroup): void {
        const open = !isOpen(group);
        const active = activeIn(group);
        if (open) delete closedOn[group.id];
        else if (active !== null) closedOn[group.id] = active;
        group.onToggle?.(open);
    }

    const linkId = (group: SidebarGroup, link: SidebarLink): string => `sidebar-${group.id}-${link.id}-item`;
</script>

{#snippet item(link: SidebarLink, inGroup: {id: string; hidden: boolean} | null = null)}
    <li id={inGroup?.id} hidden={inGroup?.hidden ?? false}>
        <!-- eslint-disable svelte/no-navigation-without-resolve -- the layout resolves every link -->
        <a
            href={link.href}
            class={[link.active && "menu-active", inGroup !== null && "md:ps-7"]}
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
                            class={count.tone === "error" ? "badge badge-sm tabular-nums badge-error" : "text-xs opacity-60 tabular-nums"}
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
                {#if group.onToggle === undefined}
                    <li class="hidden menu-title pt-3 md:block" data-testid="sidebar-group-{group.id}">{group.title}</li>
                    {#each group.links as link (link.id)}
                        {@render item(link)}
                    {/each}
                {:else}
                    {@const open = isOpen(group)}
                    <li>
                        <button
                            type="button"
                            aria-expanded={open}
                            aria-controls={group.links.map((l) => linkId(group, l)).join(" ")}
                            onclick={() => toggle(group)}
                            data-testid="sidebar-group-{group.id}"
                        >
                            <span>{group.title}</span>
                            <svg
                                viewBox="0 0 16 16"
                                class={["ml-auto h-3.5 w-3.5 transition-transform motion-reduce:transition-none", open && "rotate-90"]}
                                aria-hidden="true"
                                fill="none"
                                stroke="currentColor"
                                stroke-width="2"><path d="M6 4l4 4-4 4" stroke-linecap="round" stroke-linejoin="round" /></svg
                            >
                        </button>
                    </li>
                    {#each group.links as link (link.id)}
                        {@render item(link, {id: linkId(group, link), hidden: !open})}
                    {/each}
                {/if}
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
