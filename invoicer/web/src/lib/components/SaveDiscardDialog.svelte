<!-- Leaving with unsaved changes (Phase 4 feedback J1): Save, Discard or
     Cancel (stay). A daisyUI modal drawn open (a div, so it mounts in
     jsdom), focus on Cancel, the safe choice; Escape cancels. -->
<script lang="ts">
    let {
        title = "Save your changes?",
        message,
        saving = false,
        error = null,
        onSave,
        onDiscard,
        onCancel,
    }: {
        title?: string;
        message: string;
        saving?: boolean;
        error?: string | null;
        onSave: () => void;
        onDiscard: () => void;
        onCancel: () => void;
    } = $props();

    const uid = $props.id();

    function focusSafe(node: HTMLButtonElement): void {
        node.focus();
    }

    function onKeydown(event: KeyboardEvent): void {
        if (event.key === "Escape") {
            event.preventDefault();
            event.stopPropagation();
            onCancel();
        }
    }
</script>

<div class="modal modal-open" role="presentation" onkeydown={onKeydown}>
    <div class="modal-box" role="alertdialog" aria-modal="true" aria-labelledby="{uid}-title" data-testid="unsaved-dialog">
        <h2 id="{uid}-title" class="text-lg font-semibold">{title}</h2>
        <p class="mt-2 text-sm">{message}</p>
        {#if error !== null}
            <div class="mt-3 alert text-sm alert-error" role="alert" data-testid="unsaved-dialog-error">{error}</div>
        {/if}
        <div class="modal-action flex-wrap">
            <button type="button" class="btn btn-ghost btn-sm" onclick={onCancel} disabled={saving} use:focusSafe data-testid="unsaved-cancel">Cancel</button>
            <button type="button" class="btn btn-outline btn-sm btn-warning" onclick={onDiscard} disabled={saving} data-testid="unsaved-discard">Discard</button
            >
            <button type="button" class="btn btn-primary btn-sm" onclick={onSave} disabled={saving} data-testid="unsaved-save">
                {#if saving}<span class="loading loading-xs loading-spinner"></span>{/if}
                Save
            </button>
        </div>
    </div>
</div>
