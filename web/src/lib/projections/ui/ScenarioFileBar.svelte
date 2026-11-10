<script lang="ts">
    // The scenario picker and the save controls (plan 22, Phase 3).
    //
    // One row above the what-if table: which file this scenario came from, a
    // Save, and a Save as. Inline rather than a modal, following
    // `RulesFileList.svelte` — it is one control in a page the user is already
    // looking at, and pulling them out of the table to change file would be
    // worse than the select box it replaces.
    //
    // # Two groups, because the ask asks for two
    //
    // `projection-*.journal` files are the ones Ledgeline wrote and are listed
    // first, under their own heading. Everything else is "Other journals" —
    // "really any journal file with budget-like entries could be used", and the
    // active budget file is the one the ask wants defaulted to. Only the first
    // group has a `; projection:` name; the rest show their filename.
    //
    // # Discarding unsaved work is a two-step, never a dialog
    //
    // Switching file with `dirty` work shows an inline confirm under the select,
    // the way `RulesFileList` does: it cannot be dismissed by accident and it
    // adds no global behaviour to the SPA for the sake of one screen.
    //
    // # The star (plan 33 D20)
    //
    // Right after the select: filled when the open file is the one the tab
    // opens by default, and a click clears it; outlined otherwise, and a click
    // makes this file the default. Enabled with unsaved edits, because it
    // writes only the file's marker line (D17). Disabled, with the reason as
    // its tooltip, for a seed, a file that is not a scenario file, a file the
    // main journal includes, and a server that cannot write. Hidden on an
    // engine that predates the marker.

    import type {ProjectionFile} from "../types";
    import ScenarioOptions from "./ScenarioOptions.svelte";

    interface Props {
        files: readonly ProjectionFile[];
        /** The file the scenario came from, or null when it was seeded. */
        currentId: string | null;
        /** Whether the current file may be saved over — false for one the main journal includes. */
        canSave: boolean;
        /** Whether the scenario has unsaved edits. */
        dirty: boolean;
        /** Whether this server has an editor bound; false disables every write. */
        editable: boolean;
        busy: boolean;
        /** The scan hit a cap, so the list below is a subset. */
        truncated: boolean;
        onOpen: (id: string) => void;
        onSave: () => void;
        onSaveAs: () => void;
        /** Whether this engine knows the default marker; false hides the star (plan 33 D19). */
        defaultSupported?: boolean;
        /** The effective default's id, for the picker's " (default)" suffix. */
        defaultId?: string | null;
        /** Whether the open file is the effective default: the star is filled. */
        isDefault?: boolean;
        /** Why the open scenario cannot be starred, or null when it can. The disabled star's tooltip. */
        defaultBlocker?: string | null;
        /** The listing's "several files are marked" warning (D6), or null. */
        defaultWarning?: string | null;
        onToggleDefault?: () => void;
    }

    const {
        files,
        currentId,
        canSave,
        dirty,
        editable,
        busy,
        truncated,
        onOpen,
        onSave,
        onSaveAs,
        defaultSupported = false,
        defaultId = null,
        isDefault = false,
        defaultBlocker = null,
        defaultWarning = null,
        onToggleDefault = () => {},
    }: Props = $props();

    const starLabel = $derived(isDefault ? "Default scenario — click to stop opening it by default" : "Open this scenario by default");

    /** The id the select box is showing — `""` is the seeded, file-less scenario. */
    let pending = $state<string | null>(null);

    function choose(id: string): void {
        if (id === "" || id === currentId) return;
        // The unsaved-work gate. Confirmed below, or abandoned.
        if (dirty) {
            pending = id;
            return;
        }
        onOpen(id);
    }

    function confirmSwitch(): void {
        const id = pending;
        pending = null;
        if (id !== null) onOpen(id);
    }
</script>

<div class="flex flex-col gap-2" data-testid="projection-file-bar">
    <div class="flex flex-wrap items-end gap-x-3 gap-y-2">
        <label class="form-control">
            <span class="label-text mb-1 block text-xs text-base-content/70">Scenario</span>
            <select
                class="select w-64 max-w-full select-sm"
                value={currentId ?? ""}
                disabled={busy}
                onchange={(event) => choose(event.currentTarget.value)}
                aria-label="Scenario file"
                data-testid="projection-file-select"
            >
                <!-- The seeded scenario has no file, and must stay selectable as
                     a label even though choosing it does nothing: a select whose
                     value is not among its options renders blank. -->
                <option value="">Seeded from your journal</option>
                <ScenarioOptions {files} {defaultId} />
            </select>
        </label>

        {#if defaultSupported}
            <button
                type="button"
                class="btn btn-square btn-ghost btn-sm"
                class:text-warning={isDefault}
                aria-pressed={isDefault}
                aria-label={starLabel}
                title={defaultBlocker ?? starLabel}
                disabled={defaultBlocker !== null || busy}
                onclick={onToggleDefault}
                data-testid="projection-default-toggle"
            >
                <svg viewBox="0 0 24 24" class="h-4 w-4" aria-hidden="true" fill={isDefault ? "currentColor" : "none"} stroke="currentColor" stroke-width="2">
                    <path stroke-linejoin="round" d="M12 3l2.8 5.7 6.2.9-4.5 4.4 1.1 6.2L12 17.3 6.4 20.2l1.1-6.2L3 9.6l6.2-.9z" />
                </svg>
            </button>
        {/if}

        <div class="flex items-center gap-2">
            <button
                type="button"
                class="btn btn-sm"
                disabled={!editable || !canSave || busy}
                onclick={onSave}
                data-testid="projection-save"
                title={canSave
                    ? ""
                    : currentId === null
                      ? "This scenario has no file of its own yet — use Save as."
                      : "This file is part of your main journal, so it is never saved over — use Save as."}
            >
                {#if busy}<span class="loading loading-xs loading-spinner"></span>{/if}
                Save
            </button>
            <button type="button" class="btn btn-sm" disabled={!editable || busy} onclick={onSaveAs} data-testid="projection-save-as">Save as…</button>
            {#if dirty}
                <span class="badge badge-ghost badge-sm" data-testid="projection-dirty">edited</span>
            {/if}
        </div>
    </div>

    {#if pending !== null}
        <div class="flex flex-col gap-1 rounded border border-warning/40 p-2" role="alert" data-testid="projection-discard">
            <span class="text-xs">Open another scenario and discard your unsaved changes?</span>
            <div class="flex gap-1">
                <button type="button" class="btn btn-warning btn-xs" onclick={confirmSwitch} data-testid="projection-discard-confirm">Discard</button>
                <button type="button" class="btn btn-ghost btn-xs" onclick={() => (pending = null)} data-testid="projection-discard-cancel">Keep editing</button
                >
            </div>
        </div>
    {/if}

    {#if defaultWarning !== null}
        <p class="text-xs text-warning" role="status" data-testid="projection-default-conflict">{defaultWarning}</p>
    {/if}

    {#if truncated}
        <p class="text-xs text-warning" role="status">
            There are more journal files here than this list shows. Move the ones you use into a folder beside your journal.
        </p>
    {/if}
</div>
