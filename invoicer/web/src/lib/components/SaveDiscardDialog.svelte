<!-- Leaving with unsaved changes (Phase 4 feedback J1): Save, Discard or
     Cancel (stay), for the customer page and the invoice editor (H13). A
     daisyUI modal drawn open (a div, so it mounts in jsdom), focus on
     Cancel, the safe choice; Tab and Shift+Tab stay inside; Escape cancels. -->
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
    let box = $state<HTMLDivElement | null>(null);

    function focusSafe(node: HTMLButtonElement): void {
        node.focus();
    }

    function onKeydown(event: KeyboardEvent): void {
        if (event.key === "Escape") {
            event.preventDefault();
            event.stopPropagation();
            onCancel();
            return;
        }
        if (event.key !== "Tab" || box === null) return;
        const buttons = Array.from(box.querySelectorAll<HTMLButtonElement>("button:not([disabled])"));
        if (buttons.length === 0) return;
        const first = buttons[0];
        const last = buttons[buttons.length - 1];
        if (event.shiftKey && document.activeElement === first) {
            event.preventDefault();
            last.focus();
        } else if (!event.shiftKey && document.activeElement === last) {
            event.preventDefault();
            first.focus();
        }
    }
</script>

<div class="modal modal-open" role="presentation" onkeydown={onKeydown}>
    <div bind:this={box} class="modal-box" role="alertdialog" aria-modal="true" aria-labelledby="{uid}-title" data-testid="unsaved-dialog">
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
