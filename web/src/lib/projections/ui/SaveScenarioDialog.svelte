<script lang="ts">
    // Save As: name the scenario, pick a directory, and see the path BEFORE the
    // write (plan 22, Phase 3, §"Save As, and the name").
    //
    // # The preview is the feature
    //
    // The name governs both the display name and the filename, and the mapping
    // between them is lossy — `Series A!!` and `Series A` are the same file. So
    // the resulting path is shown as it is typed, computed by `projectionId`,
    // which mirrors the engine's own `slug_filename` character for character. A
    // dialog that wrote to a path it had not shown would be the one thing a
    // "save as" must not do.
    //
    // # The directory is a CHOICE from a list, never free text
    //
    // The engine refuses to create a directory (`resolve_new` requires a real,
    // non-symlink parent), so a free-text path would let a user type somewhere
    // the save is about to refuse. The list is the directories the scan found a
    // journal in, which is exactly the set `resolve_new` will accept.
    //
    // # The refusal is the kernel's
    //
    // Saving over an existing file is refused by `O_EXCL` on the engine, not by
    // a check here — a check would expire the moment it returned. The local
    // "that name is taken" note is a courtesy that saves a round trip; the
    // `conflict` failure is the real answer and is shown when it comes back.
    import {untrack} from "svelte";
    import {projectionId} from "../scenarioModel";
    import type {ProjectionFile} from "../types";

    interface Props {
        /** The name to start from — the loaded scenario's, or empty for a seed. */
        name: string;
        /** The directories the scan found a journal in, `""` for the root. */
        directories: readonly string[];
        /** The files already there, so a name that is taken can be flagged before the round trip. */
        files: readonly ProjectionFile[];
        /** A save is in flight. */
        saving: boolean;
        /** The engine's own sentence from a failed save, or null. */
        error: string | null;
        onSave: (id: string, name: string) => void;
        onCancel: () => void;
    }

    const {name, directories, files, saving, error, onSave, onCancel}: Props = $props();

    // Seeded ONCE from the props, with `untrack` — the same pattern
    // `GoalModal.svelte` uses, and for the same reason: this is a draft the user
    // is editing, and a reactive reseed would undo their typing the moment the
    // parent re-rendered.
    let draftName = $state(untrack(() => name));
    let directory = $state(untrack(() => directories[0] ?? ""));

    const id = $derived(projectionId(directory, draftName));
    const taken = $derived(id !== null && files.some((file) => file.id === id));

    /** The one thing stopping a save, in the order a user would hit them. */
    const blocker = $derived.by(() => {
        if (draftName.trim() === "") return "Give the scenario a name.";
        if (id === null) return "That name has no letters or digits in it, so there is no file name to make from it.";
        if (taken) return "A file already exists there — choose another name.";
        return null;
    });

    function submit(event: SubmitEvent): void {
        event.preventDefault();
        if (blocker !== null || saving || id === null) return;
        onSave(id, draftName.trim());
    }
</script>

<div class="modal modal-open" role="dialog" aria-modal="true" aria-label="Save projection as" data-testid="projection-save-dialog">
    <div class="modal-box max-w-lg">
        <h3 class="mb-3 text-base font-semibold">Save projection as</h3>
        <form onsubmit={submit}>
            <div class="form-control mb-3">
                <label class="label-text text-xs" for="projection-save-name">Name</label>
                <input
                    id="projection-save-name"
                    type="text"
                    class="input w-full input-sm"
                    bind:value={draftName}
                    disabled={saving}
                    autocomplete="off"
                    spellcheck="false"
                    data-testid="projection-save-name"
                />
                <span class="label-text-alt mt-1 text-xs text-base-content/60"> Shown in the picker, and used to build the file name. </span>
            </div>

            {#if directories.length > 1}
                <div class="form-control mb-3">
                    <label class="label-text text-xs" for="projection-save-dir">Folder</label>
                    <select id="projection-save-dir" class="select w-full select-sm" bind:value={directory} disabled={saving} data-testid="projection-save-dir">
                        {#each directories as dir (dir)}
                            <option value={dir}>{dir === "" ? "(journal folder)" : dir}</option>
                        {/each}
                    </select>
                    <span class="label-text-alt mt-1 text-xs text-base-content/60">
                        Only folders that already hold a journal — Ledgeline never creates one.
                    </span>
                </div>
            {/if}

            <!-- The path, before the write. -->
            <p class="mb-3 text-xs text-base-content/70" data-testid="projection-save-path">
                {#if id === null}
                    <span class="text-warning">No file name yet.</span>
                {:else}
                    Saves to <code class="font-mono">{id}</code>
                {/if}
            </p>

            {#if error !== null}
                <div class="mb-3 alert items-start rounded-box px-3 py-2 text-sm alert-error" role="alert" data-testid="projection-save-error">
                    {error}
                </div>
            {/if}

            <div class="modal-action">
                <button type="button" class="btn btn-ghost btn-sm" onclick={onCancel} disabled={saving} data-testid="projection-save-cancel">Cancel</button>
                <button type="submit" class="btn btn-primary btn-sm" disabled={blocker !== null || saving} data-testid="projection-save-submit">
                    {#if saving}<span class="loading loading-xs loading-spinner"></span>{/if}
                    Save
                </button>
            </div>
            {#if blocker !== null}
                <p class="mt-2 text-right text-xs text-warning" role="status" data-testid="projection-save-blocker">{blocker}</p>
            {/if}
        </form>
    </div>
    <button type="button" class="modal-backdrop" onclick={onCancel} aria-label="Close" disabled={saving}></button>
</div>
