<!-- A sideways scroller that SAYS it scrolls.

     macOS hides scrollbars until a scroll starts, so a statement wider than
     the page looked cut off: the last columns (a comparison's Δ and Δ%) were
     simply not there, with nothing to say they were one swipe away. While
     there is more to either side this draws a fade over that edge and a
     chevron button that scrolls a page-width that way; both go when the edge
     is reached. Nothing is drawn when the content fits.

     `enabled: false` renders the children inside `display: contents`
     wrappers, so a caller whose single-column mode needs its children to stay
     direct flex children (Total mode) gets exactly that. -->
<script lang="ts">
    import type {Snippet} from "svelte";

    let {
        enabled = true,
        class: className = "",
        testid,
        children,
    }: {
        enabled?: boolean;
        /** Classes for the scrolling element itself (e.g. `@container`). */
        class?: string;
        /** The scrolling element's `data-testid`. */
        testid?: string;
        children: Snippet;
    } = $props();

    let scroller: HTMLDivElement | undefined = $state();
    let moreLeft = $state(false);
    let moreRight = $state(false);

    /** Within a pixel counts as the edge: zoom leaves fractional scroll offsets. */
    const SLACK = 1;

    function measure(): void {
        if (scroller === undefined) return;
        const {scrollLeft, scrollWidth, clientWidth} = scroller;
        moreLeft = scrollLeft > SLACK;
        moreRight = scrollLeft + clientWidth < scrollWidth - SLACK;
    }

    $effect(() => {
        if (!enabled || scroller === undefined) return;
        const el = scroller;
        measure();
        // jsdom (component tests) has no ResizeObserver; the first measure stands.
        if (typeof ResizeObserver === "undefined") return;
        // The content's width changes with the data (a re-run, a new column
        // count) as well as with the window, so both are watched.
        const observer = new ResizeObserver(measure);
        observer.observe(el);
        if (el.firstElementChild !== null) observer.observe(el.firstElementChild);
        return () => observer.disconnect();
    });

    function nudge(direction: 1 | -1): void {
        scroller?.scrollBy({left: direction * scroller.clientWidth * 0.8, behavior: "smooth"});
    }
</script>

{#if enabled}
    <div class="relative" data-more-left={moreLeft} data-more-right={moreRight}>
        <div bind:this={scroller} class="{className} overflow-x-auto" data-testid={testid} onscroll={measure}>
            {@render children()}
        </div>
        {#if moreLeft}
            <div class="pointer-events-none absolute inset-y-0 left-0 w-10 bg-linear-to-r from-base-100 to-transparent" aria-hidden="true"></div>
            <button
                type="button"
                class="btn absolute top-2 left-1 btn-circle shadow-md btn-xs"
                aria-label="Scroll left"
                title="More columns to the left"
                data-testid={testid === undefined ? undefined : `${testid}-left`}
                onclick={() => nudge(-1)}>‹</button
            >
        {/if}
        {#if moreRight}
            <div class="pointer-events-none absolute inset-y-0 right-0 w-10 bg-linear-to-l from-base-100 to-transparent" aria-hidden="true"></div>
            <button
                type="button"
                class="btn absolute top-2 right-1 btn-circle shadow-md btn-xs"
                aria-label="Scroll right"
                title="More columns to the right"
                data-testid={testid === undefined ? undefined : `${testid}-right`}
                onclick={() => nudge(1)}>›</button
            >
        {/if}
    </div>
{:else}
    <div class="contents" data-testid={testid}>
        {@render children()}
    </div>
{/if}
