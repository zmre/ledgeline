<!-- Imports tab strip (WP-11): daisyUI tabs, one per screen. The twin of
     ReportTabs — same markup, same $bindable contract, so the two subnavs stay
     the same thing to look at and to operate. -->
<script lang="ts">
    import {TAB_LABELS, TAB_ORDER, type ImportTab} from "$lib/imports/params";
    import {registerKeys} from "$lib/keys/keymap.svelte";
    import {PRIORITY} from "$lib/keys/types";

    let {tab = $bindable()}: {tab: ImportTab} = $props();

    // Digit-per-tab, same reasoning as ReportTabs (see the comment there). The
    // two strips stay the same thing to look at and to operate.
    registerKeys({
        id: "import-tabs",
        priority: PRIORITY.page,
        bindings: TAB_ORDER.map((t, at) => ({
            keys: String(at + 1),
            label: TAB_LABELS[t],
            group: "Imports" as const,
            run: () => (tab = t),
        })),
    });
</script>

<div role="tablist" class="tabs tabs-border" aria-label="Imports">
    {#each TAB_ORDER as t (t)}
        <button type="button" role="tab" class="tab whitespace-nowrap {t === tab ? 'tab-active' : ''}" aria-selected={t === tab} onclick={() => (tab = t)}>
            {TAB_LABELS[t]}
        </button>
    {/each}
</div>
