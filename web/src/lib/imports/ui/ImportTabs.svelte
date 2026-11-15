<!-- Imports tab strip (WP-11): daisyUI tabs, one per screen. The twin of
     ReportTabs — same markup, same $bindable contract, so the two subnavs stay
     the same thing to look at and to operate. -->
<script lang="ts">
    import {TAB_LABELS, TAB_ORDER, type ImportTab} from "#lib/imports/params.js";
    import {registerKeys} from "#lib/keys/keymap.svelte.js";
    import {PRIORITY} from "#lib/keys/types.js";
    import {isNative} from "#lib/native/detect.js";

    let {tab = $bindable()}: {tab: ImportTab} = $props();

    // Bank sync runs in the desktop engine only (plan 43), so the iOS app
    // (plan 42) has no Accounts tab — as Settings has no Prices tab there.
    const tabs = isNative() ? TAB_ORDER.filter((t) => t !== "accounts") : TAB_ORDER;

    // Digit-per-tab, same reasoning as ReportTabs (see the comment there). The
    // two strips stay the same thing to look at and to operate.
    registerKeys({
        id: "import-tabs",
        priority: PRIORITY.page,
        bindings: tabs.map((t, at) => ({
            keys: String(at + 1),
            label: TAB_LABELS[t],
            group: "Imports" as const,
            run: () => (tab = t),
        })),
    });
</script>

<div role="tablist" class="tabs tabs-border" aria-label="Imports">
    {#each tabs as t (t)}
        <button type="button" role="tab" class="tab whitespace-nowrap {t === tab ? 'tab-active' : ''}" aria-selected={t === tab} onclick={() => (tab = t)}>
            {TAB_LABELS[t]}
        </button>
    {/each}
</div>
