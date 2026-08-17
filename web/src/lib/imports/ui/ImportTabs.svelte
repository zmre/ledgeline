<!-- Imports tab strip (WP-11): daisyUI tabs, one per screen. The twin of
     ReportTabs — same markup, same $bindable contract, so the two subnavs stay
     the same thing to look at and to operate. -->
<script lang="ts">
    import {TAB_LABELS, TAB_ORDER, type ImportTab} from "$lib/imports/params";

    let {tab = $bindable()}: {tab: ImportTab} = $props();
</script>

<div role="tablist" class="tabs tabs-border" aria-label="Imports">
    {#each TAB_ORDER as t (t)}
        <button type="button" role="tab" class="tab whitespace-nowrap {t === tab ? 'tab-active' : ''}" aria-selected={t === tab} onclick={() => (tab = t)}>
            {TAB_LABELS[t]}
        </button>
    {/each}
</div>
