<!-- Report tab strip (WP-07): daisyUI tabs, one per report. -->
<script lang="ts">
    import {registerKeys} from "$lib/keys/keymap.svelte";
    import {PRIORITY} from "$lib/keys/types";
    import {TAB_LABELS, TAB_ORDER, type ReportTab} from "./params";

    let {tab = $bindable()}: {tab: ReportTab} = $props();

    // Digits, not more `g` chords. Seven report tabs under one prefix would mean
    // arming `g` on every page for chords that exist on one, and seven rows in
    // the GLOBAL half of the help sheet. A digit is one keystroke, matches how
    // tab strips work everywhere, and — because this layer is page-local — the
    // digits only appear in `?` while this strip is mounted.
    //
    // These write `tab` and let the reports page's existing `searchMirror`
    // effect put it in the URL: no `goto`, so no
    // `svelte/no-navigation-without-resolve` disable anywhere in this feature.
    registerKeys({
        id: "report-tabs",
        priority: PRIORITY.page,
        bindings: TAB_ORDER.map((t, at) => ({
            keys: String(at + 1),
            label: TAB_LABELS[t],
            group: "Reports" as const,
            run: () => (tab = t),
        })),
    });
</script>

<div role="tablist" class="tabs tabs-border" aria-label="Report">
    {#each TAB_ORDER as t (t)}
        <button type="button" role="tab" class="tab whitespace-nowrap {t === tab ? 'tab-active' : ''}" aria-selected={t === tab} onclick={() => (tab = t)}>
            {TAB_LABELS[t]}
        </button>
    {/each}
</div>
