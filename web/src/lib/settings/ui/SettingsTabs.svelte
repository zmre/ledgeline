<!-- Settings tab strip: daisyUI tabs, one per screen. Same markup, same
     $bindable contract as ImportTabs/ReportTabs, so every subnav in the app
     stays the same thing to look at and to operate. -->
<script lang="ts">
    import HScroll from "#ui/components/HScroll.svelte";
    import {registerKeys} from "#lib/keys/keymap.svelte.js";
    import {PRIORITY} from "#lib/keys/types.js";
    import {accrual} from "#lib/accrual/store.svelte.js";
    import {isNative} from "#ui/native/detect.js";
    import {TAB_LABELS, TAB_ORDER, type SettingsTab} from "#lib/settings/params.js";

    let {tab = $bindable()}: {tab: SettingsTab} = $props();

    // The iOS app (plan 42) runs no background price updater or bank sync
    // (both live in the desktop server's main loop), so the tabs that only
    // configure them go.
    const hosted = isNative() ? TAB_ORDER.filter((t) => t !== "prices" && t !== "sync") : TAB_ORDER;
    // Books (plan 45) exists only with the accrual helpers on (D-gate): no tab, no number key.
    const tabs = $derived(accrual.enabled ? hosted : hosted.filter((t) => t !== "books"));

    registerKeys({
        id: "settings-tabs",
        priority: PRIORITY.page,
        // Over every hosted tab, Books last: its digit is simply disabled while the helpers are off.
        bindings: hosted.map((t, at) => ({
            keys: String(at + 1),
            label: TAB_LABELS[t],
            group: "Settings" as const,
            enabled: () => t !== "books" || accrual.enabled,
            run: () => (tab = t),
        })),
    });
</script>

<!-- One line that scrolls sideways rather than wrapping: a wrapped strip on a
     phone is two or three rows of tabs before any content. -->
<HScroll reveal={tab}>
    <div role="tablist" class="tabs tabs-border w-max min-w-full flex-nowrap" aria-label="Settings">
        {#each tabs as t (t)}
            <button type="button" role="tab" class="tab whitespace-nowrap {t === tab ? 'tab-active' : ''}" aria-selected={t === tab} onclick={() => (tab = t)}>
                {TAB_LABELS[t]}
            </button>
        {/each}
    </div>
</HScroll>
