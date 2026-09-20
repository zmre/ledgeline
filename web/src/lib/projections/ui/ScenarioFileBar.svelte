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

    import type {ProjectionFile} from "../types";

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
    }

    const {files, currentId, canSave, dirty, editable, busy, truncated, onOpen, onSave, onSaveAs}: Props = $props();

    const projections = $derived(files.filter((file) => file.isProjection));
    const others = $derived(files.filter((file) => !file.isProjection));

    /** The picker's label for one file: its `; projection:` name, else its filename label. */
    const labelOf = (file: ProjectionFile): string => file.name ?? file.label;

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
                {#if projections.length > 0}
                    <optgroup label="Projections">
                        {#each projections as file (file.id)}
                            <option value={file.id}>{labelOf(file)}</option>
                        {/each}
                    </optgroup>
                {/if}
                {#if others.length > 0}
                    <optgroup label="Other journals">
                        {#each others as file (file.id)}
                            <option value={file.id}>{labelOf(file)}</option>
                        {/each}
                    </optgroup>
                {/if}
            </select>
        </label>

        <div class="flex items-center gap-2">
            <button
                type="button"
                class="btn btn-sm"
                disabled={!editable || !canSave || busy}
                onclick={onSave}
                data-testid="projection-save"
                title={canSave ? "" : "This scenario has no file of its own yet — use Save as."}
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

    {#if currentId !== null && !canSave}
        <!-- Decision 5, said out loud rather than discovered at the moment of a
             refused save: the Projections tab never writes over a plan of
             record. -->
        <p class="text-xs text-base-content/60" data-testid="projection-read-only">
            This file is part of your main journal, so it can be read but never saved over. Use <strong>Save as…</strong> to keep your changes.
        </p>
    {/if}

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

    {#if truncated}
        <p class="text-xs text-warning" role="status">
            There are more journal files here than this list shows. Move the ones you use into a folder beside your journal.
        </p>
    {/if}
</div>
