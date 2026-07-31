<script lang="ts">
    // The master pane: every `*.rules` file beside the open journal.
    //
    // A row shows the file's name and the folder it lives in, because labels are
    // NOT unique — `2025/imports/capitalone.csv.rules` and
    // `2026/imports/capitalone.csv.rules` are two different files with the same
    // name, and the folder is what tells them apart. The counts live in a
    // tooltip instead of under the label, where they were busy and got truncated.
    //
    // The tooltip is daisyUI's, which opens on `:hover` AND on
    // `:has(:focus-visible)` — so tabbing to a row shows it too, with no
    // JavaScript and no custom CSS. `data-tip` renders through a pseudo-element
    // rather than a child node, which matters here: a real child would be styled
    // as another item by the enclosing `menu`.
    //
    // Switching files while there are unsaved edits uses the same inline
    // two-step confirm `TransactionModal` established for deleting a
    // transaction, rather than a `beforeunload` guard or a modal. It is one new
    // element in a list the user is already looking at, it cannot be dismissed
    // by accident, and it does not add a global behaviour to the whole SPA for
    // the sake of one screen.
    import {fileRow} from "../fileList";
    import type {RulesFileSummary} from "../types";

    let {
        files,
        selectedId,
        pendingId,
        onSelect,
        onConfirmSwitch,
        onCancelSwitch,
    }: {
        files: readonly RulesFileSummary[];
        selectedId: string | null;
        /** The file the user asked for while the open one is dirty; null when nothing is pending. */
        pendingId: string | null;
        onSelect: (id: string) => void;
        onConfirmSwitch: () => void;
        onCancelSwitch: () => void;
    } = $props();
</script>

<ul class="menu bg-base-200 rounded-box w-full gap-1 p-2" aria-label="Rules files">
    {#each files as file (file.id)}
        {@const row = fileRow(file)}
        <li class="tooltip tooltip-right w-full" data-tip={row.detail}>
            <!-- `flex` is load-bearing, not redundant with `flex-col`: daisyUI
                 styles a menu item as `display:grid; grid-auto-flow:column`, so
                 `flex-col` alone is a no-op and the two lines lay out side by
                 side instead of stacking. (The confirm block below sets `flex`
                 for the same reason.) -->
            <button
                type="button"
                class="flex flex-col items-start gap-0 {file.id === selectedId ? 'menu-active' : ''}"
                aria-current={file.id === selectedId ? "true" : undefined}
                onclick={() => onSelect(file.id)}
            >
                <span class="flex w-full items-center gap-2">
                    <span class="grow truncate font-medium">{file.label}</span>
                    {#if file.warnings.length > 0 || !file.parsed}
                        <span class="badge badge-warning badge-xs shrink-0" title={file.warnings.join("\n") || "Ledgeline could not read this file"}>!</span>
                    {/if}
                </span>
                {#if row.directory !== ""}
                    <!-- Truncated from the END: the folders that differ between two
                         same-named files are the ones nearest the root. -->
                    <span class="text-base-content/60 w-full truncate text-xs">{row.directory}</span>
                {/if}
                <!-- A tooltip built from `data-tip` is a pseudo-element, so assistive
                     technology never sees it. This keeps the same detail in the
                     button's accessible name, which is where it already was. -->
                <span class="sr-only">{row.detail}</span>
            </button>
            {#if file.id === pendingId}
                <!-- Two-step confirm, inline: the click that would discard the
                     edit is never the click that asked to switch. -->
                <div class="border-warning/40 mt-1 flex flex-col gap-1 rounded border p-2" role="alert">
                    <span class="text-xs">Discard your unsaved changes?</span>
                    <div class="flex gap-1">
                        <button type="button" class="btn btn-warning btn-xs" onclick={onConfirmSwitch}>Discard</button>
                        <button type="button" class="btn btn-ghost btn-xs" onclick={onCancelSwitch}>Keep editing</button>
                    </div>
                </div>
            {/if}
        </li>
    {/each}
</ul>
