<!-- The Accounts page's dialogs' frame: a modal with its title and, top
     right, its buttons (Save/Discard sit there in this app), over a form so
     Enter in a field submits. Escape cancels from anywhere, Tab stays inside,
     and closing hands focus back (`dismissible`); while open the page beneath
     has no keys (`modalKeys`). Focus starts on `initial` (the `[data-initial]` field). -->
<script lang="ts">
    import {untrack, type Snippet} from "svelte";
    import {dismissible} from "#ui/keys/dismissible.js";
    import {modalKeys} from "#lib/keys/modalKeys.svelte.js";

    let {
        title,
        submitLabel,
        cancelLabel = "Cancel",
        busy = false,
        submitDisabled = false,
        error = null,
        onSubmit,
        onCancel,
        children,
        testid,
        initial = "[data-initial]",
    }: {
        title: string;
        submitLabel: string;
        cancelLabel?: string;
        busy?: boolean;
        submitDisabled?: boolean;
        error?: string | null;
        onSubmit: () => void;
        onCancel: () => void;
        children: Snippet;
        testid: string;
        /** What takes focus on open (a selector inside the dialog). */
        initial?: string;
    } = $props();

    modalKeys(untrack(() => testid));
    const uid = $props.id();

    function submit(event: SubmitEvent): void {
        event.preventDefault();
        if (!busy && !submitDisabled) onSubmit();
    }
</script>

<div class="modal modal-open" role="presentation">
    <div
        class="modal-box max-w-2xl"
        role="dialog"
        aria-modal="true"
        aria-labelledby="{uid}-title"
        data-testid={testid}
        use:dismissible={{onDismiss: onCancel, trap: true, initial}}
    >
        <form onsubmit={submit} class="flex flex-col gap-3">
            <div class="flex flex-wrap items-center justify-between gap-2">
                <h2 id="{uid}-title" class="text-lg font-semibold">{title}</h2>
                <div class="ml-auto flex gap-2">
                    <button type="button" class="btn btn-ghost btn-sm" onclick={onCancel} disabled={busy} data-testid="{testid}-cancel">{cancelLabel}</button>
                    <button type="submit" class="btn btn-primary btn-sm" disabled={busy || submitDisabled} data-testid="{testid}-submit">
                        {#if busy}<span class="loading loading-xs loading-spinner"></span>{/if}
                        {submitLabel}
                    </button>
                </div>
            </div>
            {@render children()}
            {#if error !== null}
                <div class="alert py-2 text-sm alert-error" role="alert" data-testid="{testid}-error">{error}</div>
            {/if}
        </form>
    </div>
</div>
