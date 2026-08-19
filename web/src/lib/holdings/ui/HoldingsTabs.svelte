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

<div role="tablist" class="tabs tabs-border" aria-label="Holdings">
    {#each TAB_ORDER as t (t)}
        <button type="button" role="tab" class="tab whitespace-nowrap {t === tab ? 'tab-active' : ''}" aria-selected={t === tab} onclick={() => (tab = t)}>
            {TAB_LABELS[t]}
        </button>
    {/each}
</div>
