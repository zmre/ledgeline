<!-- Settings tab strip: daisyUI tabs, one per screen. Same markup, same
     $bindable contract as ImportTabs/ReportTabs, so every subnav in the app
     stays the same thing to look at and to operate. -->
<script lang="ts">
    import {registerKeys} from "$lib/keys/keymap.svelte";
    import {PRIORITY} from "$lib/keys/types";
    import {TAB_LABELS, TAB_ORDER, type SettingsTab} from "$lib/settings/params";

    let {tab = $bindable()}: {tab: SettingsTab} = $props();

    registerKeys({
        id: "settings-tabs",
        priority: PRIORITY.page,
        bindings: TAB_ORDER.map((t, at) => ({
            keys: String(at + 1),
            label: TAB_LABELS[t],
            group: "Settings" as const,
            run: () => (tab = t),
        })),
    });
</script>

<div role="tablist" class="tabs tabs-border" aria-label="Settings">
    {#each TAB_ORDER as t (t)}
        <button type="button" role="tab" class="tab whitespace-nowrap {t === tab ? 'tab-active' : ''}" aria-selected={t === tab} onclick={() => (tab = t)}>
            {TAB_LABELS[t]}
        </button>
    {/each}
</div>
