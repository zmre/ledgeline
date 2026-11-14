<!-- Settings tab strip: daisyUI tabs, one per screen. Same markup, same
     $bindable contract as ImportTabs/ReportTabs, so every subnav in the app
     stays the same thing to look at and to operate. -->
<script lang="ts">
    import HScroll from "#lib/components/HScroll.svelte";
    import {registerKeys} from "#lib/keys/keymap.svelte.js";
    import {PRIORITY} from "#lib/keys/types.js";
    import {isNative} from "#lib/native/detect.js";
    import {TAB_LABELS, TAB_ORDER, type SettingsTab} from "#lib/settings/params.js";

    let {tab = $bindable()}: {tab: SettingsTab} = $props();

    // The iOS app (plan 42) runs no background price updater (it lives in the
    // desktop server's main loop), so the tab that only configures it goes.
    const tabs = isNative() ? TAB_ORDER.filter((t) => t !== "prices") : TAB_ORDER;

    registerKeys({
        id: "settings-tabs",
        priority: PRIORITY.page,
        bindings: tabs.map((t, at) => ({
            keys: String(at + 1),
            label: TAB_LABELS[t],
            group: "Settings" as const,
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
