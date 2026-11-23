<script lang="ts">
    import "../app.css";
    import {onMount} from "svelte";
    import {useAppearance} from "#ui/theme/appearance.svelte.js";
    import {APP_NAME, defaultAppearance} from "#lib/app.js";
    import {appearanceFrom, fetchUiPrefs, startSession} from "#lib/api.js";

    let {children} = $props();

    // The shared theme and UI scale, applied the way Ledgeline applies them:
    // defaults until the saved prefs (ui-prefs.toml and the books' pin) load.
    const appearance = $state<{theme: string; uiScale: number; uiPrefsReady: boolean}>({...defaultAppearance()});
    useAppearance(appearance);

    onMount(() => {
        fetchUiPrefs()
            .then((body) => {
                const saved = appearanceFrom(body);
                if (saved !== null) {
                    appearance.theme = saved.theme;
                    appearance.uiScale = saved.uiScale;
                }
                appearance.uiPrefsReady = true;
            })
            .catch((error: unknown) => console.warn("ui prefs:", error));
        // The cookie MBR's pages need before any is shown in a frame.
        startSession().catch((error: unknown) => console.warn("session:", error));
    });
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
