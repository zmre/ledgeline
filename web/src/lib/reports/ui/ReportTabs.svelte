<!-- Report tab strip (WP-07): daisyUI tabs, one per report. -->
<script lang="ts">
    import {TAB_LABELS, TAB_ORDER, type ReportTab} from "./params";

    let {tab = $bindable()}: {tab: ReportTab} = $props();
</script>

<div role="tablist" class="tabs tabs-border" aria-label="Report">
    {#each TAB_ORDER as t (t)}
        <button type="button" role="tab" class="tab whitespace-nowrap {t === tab ? 'tab-active' : ''}" aria-selected={t === tab} onclick={() => (tab = t)}>
            {TAB_LABELS[t]}
        </button>
    {/each}
</div>
