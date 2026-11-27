<!-- The Time area's heading and its two tabs, Overview (/time) and Entries
     (/time/entries). Each tab is a page of its own, so the tabs are links
     (`aria-current` on the one shown), styled as daisyUI's tabs; their
     hrefs carry the shared filters, so switching keeps them. `actions` sits
     at the end of the row (Entries' Add time).

     Mount it at the top of both pages with resolved hrefs:
         <TimeTabs current="entries" overviewHref={…} entriesHref={…} /> -->
<script lang="ts">
    import type {Snippet} from "svelte";

    let {
        current,
        overviewHref,
        entriesHref,
        actions,
    }: {
        current: "overview" | "entries";
        /** Overview's URL with the shared filters, resolved. */
        overviewHref: string;
        /** Entries' URL with the shared filters, resolved. */
        entriesHref: string;
        actions?: Snippet;
    } = $props();

    const tabs = $derived([
        {id: "overview", label: "Overview", href: overviewHref},
        {id: "entries", label: "Entries", href: entriesHref},
    ] as const);
</script>

<header class="flex flex-wrap items-center gap-x-4 gap-y-2" data-testid="time-header">
    <h1 class="text-2xl font-semibold tracking-tight">Time</h1>
    <nav class="tabs tabs-border" aria-label="Time views">
        {#each tabs as tab (tab.id)}
            <!-- eslint-disable svelte/no-navigation-without-resolve -- the hrefs are resolve()d by the page -->
            <a
                href={tab.href}
                class={["tab", tab.id === current && "tab-active"]}
                aria-current={tab.id === current ? "page" : undefined}
                data-testid="time-tab-{tab.id}">{tab.label}</a
            >
            <!-- eslint-enable svelte/no-navigation-without-resolve -->
        {/each}
    </nav>
    {#if actions !== undefined}
        <div class="ml-auto flex flex-wrap items-center gap-2">{@render actions()}</div>
    {/if}
</header>
