<script lang="ts">
    import "../app.css";
    import {useAppearance} from "#ui/theme/appearance.svelte.js";
    import {APP_NAME, defaultAppearance} from "#lib/app.js";

    let {children} = $props();

    // The shared theme and UI scale, applied the way Ledgeline applies them.
    // Defaults only until Phase 3 loads the saved prefs.
    const appearance = $state(defaultAppearance());
    useAppearance(appearance);
</script>

<svelte:head>
    <title>{APP_NAME}</title>
</svelte:head>

<div class="flex min-h-screen flex-col bg-base-100 text-base-content">
    <header class="app-header navbar border-b border-base-300 bg-base-200 px-4">
        <span class="text-lg font-semibold">{APP_NAME}</span>
    </header>
    <main class="flex-1 p-4">
        {@render children()}
    </main>
</div>
