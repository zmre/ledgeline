<script lang="ts">
    // Settings → Appearance (plan 39): the theme and the interface scale. The
    // panel itself is shared with the invoicer (`#ui/theme/AppearancePanel`);
    // this binds it to Ledgeline's settings store.
    //
    // "Use for this ledger only" pins the key in the ledger's ledgeline.toml —
    // the one place the app may create that file, because you asked it to.
    import SharedAppearancePanel from "#ui/theme/AppearancePanel.svelte";
    import {settings, type ThemeId} from "#lib/stores/settings.svelte.js";
</script>

<SharedAppearancePanel
    theme={settings.theme}
    uiScale={settings.uiScale}
    onTheme={(id) => (settings.theme = id as ThemeId)}
    onScale={(percent) => (settings.uiScale = percent)}
    canPin={settings.canPinToLedger}
    isPinned={(key) => settings.isPinned(key)}
    pin={(key, pinned) => settings.pinToLedger(key, pinned)}
>
    {#snippet themePinnedNote()}This ledger has its own theme, kept in its <code>ledgeline.toml</code>.{/snippet}
</SharedAppearancePanel>
