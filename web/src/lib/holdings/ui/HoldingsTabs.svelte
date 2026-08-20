<!-- Holdings tab strip (plans/14): daisyUI tabs, one per sub-screen. The third of
     the family — ReportTabs and ImportTabs are the others — and deliberately the
     same markup, so the e2e convention (click by role, assert `aria-selected`)
     works here unchanged and the three subnavs stay the same thing to look at
     and to operate. -->
<script lang="ts">
    import {TAB_LABELS, TAB_ORDER, type HoldingsTab} from "$lib/holdings/params";
    import {registerKeys} from "$lib/keys/keymap.svelte";
    import {PRIORITY} from "$lib/keys/types";

    let {tab = $bindable()}: {tab: HoldingsTab} = $props();

    // The rest of the WAI-ARIA tabs pattern (the role alone is a promise the
    // keyboard must keep): a roving tabindex so Tab enters the strip once, on
    // the selected tab, and Left/Right arrows move BOTH selection and focus,
    // wrapping at the ends. Selection follows focus (the APG's "automatic
    // activation") because switching a tab here is free — no fetch, see the
    // page's `otherOpened` latch.
    const els: HTMLButtonElement[] = $state([]);

    function onKeydown(event: KeyboardEvent, at: number): void {
        const delta = event.key === "ArrowRight" ? 1 : event.key === "ArrowLeft" ? -1 : 0;
        if (delta === 0) return;
        event.preventDefault();
        event.stopPropagation(); // the strip's own keys; the global keymap has no claim on them
        const to = (at + delta + TAB_ORDER.length) % TAB_ORDER.length;
        tab = TAB_ORDER[to];
        els[to]?.focus();
    }

    // Digit-per-tab, same reasoning as ReportTabs (see the comment there). These
    // write `tab` and let the holdings page's single URL sync put it in the
    // query string: no `goto`, so no `svelte/no-navigation-without-resolve`
    // disable anywhere in this feature.
    registerKeys({
        id: "holdings-tabs",
        priority: PRIORITY.page,
        bindings: TAB_ORDER.map((t, at) => ({
            keys: String(at + 1),
            label: TAB_LABELS[t],
            group: "Holdings" as const,
            run: () => (tab = t),
        })),
    });
</script>

<!-- `aria-controls` names the ONE switched region: the holdings page renders it as
     `id="holdings-panel"` with role="tabpanel", labelled back by the active tab's
     id — see routes/holdings/+page.svelte (and the wiring test pinning the pair). -->
<div role="tablist" class="tabs tabs-border" aria-label="Holdings">
    {#each TAB_ORDER as t, at (t)}
        <button
            bind:this={els[at]}
            type="button"
            role="tab"
            id="holdings-tab-{t}"
            class="tab whitespace-nowrap {t === tab ? 'tab-active' : ''}"
            aria-selected={t === tab}
            aria-controls="holdings-panel"
            tabindex={t === tab ? 0 : -1}
            onclick={() => (tab = t)}
            onkeydown={(event) => onKeydown(event, at)}
        >
            {TAB_LABELS[t]}
        </button>
    {/each}
</div>
