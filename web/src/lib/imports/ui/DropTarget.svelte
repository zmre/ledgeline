<script lang="ts">
    // The drop zone: a full-width dashed panel plus a "Choose file…" button over
    // a hidden `<input type="file">`.
    //
    // Drag and drop is new to this codebase, and there are exactly three things
    // to get right:
    //
    //  1. `preventDefault()` on ALL FOUR of dragenter/dragover/dragleave/drop.
    //     The browser's default action for a dropped file is to NAVIGATE TO IT —
    //     miss one and the SPA is replaced by a rendering of the user's bank
    //     statement, losing whatever was on screen. `dragover` is the one people
    //     forget; without it `drop` never fires at all.
    //  2. A dragenter/dragleave DEPTH COUNT, not a boolean. Both events fire as
    //     the pointer crosses each child element, so a boolean makes the
    //     highlight strobe as the cursor moves over the text inside the panel.
    //  3. Clearing `input.value` after a pick, so choosing the same file twice
    //     in a row still fires `change`. Without it, "retry with the same file"
    //     silently does nothing.
    //
    // `role="presentation"` on the panel is deliberate and is not a suppression:
    // drag-and-drop is inherently pointer-only, so the panel carries no
    // semantics of its own and the accessible, keyboard-operable path is the
    // real `<button>` inside it. Marking it presentational is what says that.
    import {acceptAttribute, formatList} from "../importModel";

    let {
        formats,
        busy,
        rejection,
        onFile,
    }: {
        /** Extensions the engine says it reads, e.g. `["csv","ofx"]`. Drives `accept` and the copy. */
        formats: readonly string[];
        /** A file is being converted right now. */
        busy: boolean;
        /** A file refused before upload (a `.pdf`, an extension this engine does not read). */
        rejection: string | null;
        onFile: (file: File) => void;
    } = $props();

    let input = $state<HTMLInputElement | null>(null);
    /** Nesting depth of the drag, not a boolean — see (2) above. */
    let depth = $state(0);
    const over = $derived(depth > 0);

    /** The default action for a dropped file is to navigate to it. Refuse it on every event. */
    function halt(event: DragEvent): void {
        event.preventDefault();
        event.stopPropagation();
    }

    function onDragEnter(event: DragEvent): void {
        halt(event);
        depth += 1;
    }

    function onDragOver(event: DragEvent): void {
        halt(event);
        // Says "this is a copy, not a move" to the OS, which is what changes the
        // cursor. Without it some browsers show the no-entry cursor over a zone
        // that will in fact accept the file.
        if (event.dataTransfer !== null) event.dataTransfer.dropEffect = "copy";
    }

    function onDragLeave(event: DragEvent): void {
        halt(event);
        depth = Math.max(0, depth - 1);
    }

    function onDrop(event: DragEvent): void {
        halt(event);
        depth = 0;
        const file = event.dataTransfer?.files?.[0];
        // One file: an import is one statement into one CSV into one journal,
        // and silently taking the first of five would be a guess.
        if (file !== undefined) onFile(file);
    }

    function onPick(event: Event): void {
        const target = event.currentTarget as HTMLInputElement;
        const file = target.files?.[0];
        // Cleared BEFORE the handler runs, so re-picking the same file fires
        // `change` again — see (3) above.
        target.value = "";
        if (file !== undefined) onFile(file);
    }
</script>

<div
    class="flex flex-col items-center gap-3 rounded-box border-2 border-dashed px-4 py-10 text-center transition-colors {over
        ? 'border-primary bg-primary/10'
        : 'border-base-content/25 bg-base-200'}"
    role="presentation"
    data-testid="imports-drop-target"
    ondragenter={onDragEnter}
    ondragover={onDragOver}
    ondragleave={onDragLeave}
    ondrop={onDrop}
>
    {#if busy}
        <span class="loading loading-lg loading-spinner" aria-label="Reading the file"></span>
        <p class="text-sm text-base-content/70">Reading the file…</p>
    {:else}
        <h2 class="text-base font-semibold tracking-tight">Drop a statement here</h2>
        <p class="max-w-lg text-sm text-base-content/60">
            Ledgeline reads {formatList(formats)}. It converts whatever you drop to one CSV, offers the rules file that fits it, and shows you every transaction
            it proposes before anything is written.
        </p>
        <button type="button" class="btn btn-primary btn-sm" onclick={() => input?.click()}>Choose file…</button>
        <input bind:this={input} type="file" class="hidden" accept={acceptAttribute(formats)} onchange={onPick} />
    {/if}
</div>

{#if rejection !== null}
    <div class="alert items-start rounded-box py-2 text-sm alert-warning" role="alert" data-testid="imports-file-rejected">
        <span>{rejection}</span>
    </div>
{/if}
