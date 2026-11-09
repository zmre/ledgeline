<script lang="ts">
    // Settings → Projections (plan 33 D22): which scenario file the Projections
    // tab opens by default. One control, the same marker the file bar's star
    // writes, so the two are always showing the same thing.
    //
    // A choice applies at once (`fileStore.setDefault`), as the star does: there
    // is nothing else on this tab to save alongside it. The list offers only
    // files that can carry the marker (D5) — Ledgeline's own scenario files the
    // tab can write — plus the current default if it somehow is not one, so
    // the select never renders blank.
    //
    // Hidden entirely on an engine that predates the marker (D19), which would
    // read `PUT /api/projections/default` as a request for a file named
    // `default`.

    import {fileStore} from "../fileStore.svelte";
    import {canBeDefault, defaultConflict, effectiveDefault} from "../opening";
    import {onServerReady} from "#lib/stores/serverWatch.svelte.js";
    import {settings} from "#lib/stores/settings.svelte.js";
    import ScenarioOptions from "./ScenarioOptions.svelte";

    // Re-read on every visit, not once per session: the default lives in the
    // files, and another program (or another machine, through a sync) may
    // have moved it since the Projections tab last looked.
    onServerReady((url) => void fileStore.reloadIndex(url));

    const index = $derived(fileStore.index.value);
    const defaultId = $derived(effectiveDefault(index?.files ?? [])?.id ?? null);
    const eligible = $derived((index?.files ?? []).filter((file) => canBeDefault(file) || file.id === defaultId));
    const conflict = $derived(index === null ? null : defaultConflict(index));

    /** The engine's sentence from the last failed change, or null. */
    let error = $state<string | null>(null);

    async function choose(select: HTMLSelectElement): Promise<void> {
        const url = settings.serverUrl;
        if (url === null) return;
        const previous = defaultId ?? "";
        const id = select.value === "" ? null : select.value;
        const outcome = await fileStore.setDefault(url, id);
        if (outcome.ok) {
            error = null;
            return;
        }
        error = outcome.failure.message;
        // The select is one-way bound, so a refused choice would otherwise stay
        // on screen as though it had applied.
        select.value = previous;
    }
</script>

<section class="flex max-w-2xl flex-col gap-3" data-testid="settings-projections">
    <h2 class="text-base font-semibold">Default scenario</h2>
    {#if !fileStore.available}
        <p class="text-sm text-base-content/70">This Ledgeline engine has no scenario files to choose from.</p>
    {:else if index === null}
        {#if fileStore.index.error !== null}
            <div class="alert items-start rounded-box py-2 text-sm alert-error" role="alert">{fileStore.index.error.message}</div>
        {:else}
            <span class="loading loading-sm loading-spinner" aria-label="Loading the scenario files"></span>
        {/if}
    {:else if index.files.length === 0}
        <p class="text-sm text-base-content/70">
            Save a scenario from the Projections tab first; then you can choose one here to open whenever you go to that tab.
        </p>
    {:else if !index.defaultSupported}
        <p class="text-sm text-base-content/70">This Ledgeline engine is too old to set a default scenario.</p>
    {:else}
        <p class="text-sm text-base-content/70">
            The Projections tab opens this scenario when Ledgeline starts. Coming back to the tab keeps whatever you had open. You can also set it with the star
            next to the scenario picker.
        </p>
        {#if !index.editable}
            <div class="alert items-start rounded-box py-2 text-sm alert-info" role="status" data-testid="settings-projections-read-only">
                <span>This server has no journal open for editing, so this is shown but cannot be changed.</span>
            </div>
        {/if}
        <label class="form-control">
            <span class="label-text mb-1 block text-xs text-base-content/70">Default scenario</span>
            <select
                class="select w-96 max-w-full select-sm"
                value={defaultId ?? ""}
                disabled={!index.editable || fileStore.settingDefault}
                onchange={(event) => void choose(event.currentTarget)}
                aria-label="Default scenario"
                data-testid="settings-projections-default"
            >
                <option value="">None — reopen the last scenario, or start from your journal</option>
                <ScenarioOptions files={eligible} defaultId={null} />
            </select>
        </label>
        {#if conflict !== null}
            <p class="text-xs text-warning" role="status" data-testid="settings-projections-conflict">{conflict}</p>
        {/if}
        {#if error !== null}
            <div class="alert items-start rounded-box py-2 text-sm alert-error" role="alert" data-testid="settings-projections-error">{error}</div>
        {/if}
    {/if}
</section>
