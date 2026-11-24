<!-- Leaving with unsaved changes (invoicer PLAN, Phase 4 feedback H13): Save,
     Discard or Cancel (stay). A daisyUI modal drawn open, as ConfirmDialog.
     Keyboard: focus starts on Save (it loses nothing); Tab and Shift+Tab stay
     inside; Escape cancels. -->
<script lang="ts">
    let {
        title,
        busy = false,
        error = null,
        onSave,
        onDiscard,
        onCancel,
    }: {
        title: string;
        busy?: boolean;
        error?: string | null;
        onSave: () => void;
        onDiscard: () => void;
        onCancel: () => void;
    } = $props();

    const uid = $props.id();
    let box = $state<HTMLDivElement | null>(null);

    function focusFirst(node: HTMLButtonElement): void {
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
    <div
        bind:this={box}
        class="modal-box"
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="{uid}-title"
        aria-describedby="{uid}-body"
        data-testid="unsaved-dialog"
    >
        <h2 id="{uid}-title" class="text-lg font-semibold">{title}</h2>
        <p id="{uid}-body" class="mt-2 text-sm">Your changes aren't saved. Save them before leaving, or discard them?</p>
        {#if error !== null}
            <div class="mt-3 alert text-sm alert-error" role="alert" data-testid="unsaved-dialog-error">{error}</div>
        {/if}
        <div class="modal-action">
            <button type="button" class="btn btn-primary btn-sm" onclick={onSave} disabled={busy} use:focusFirst data-testid="unsaved-save">
                {#if busy}<span class="loading loading-xs loading-spinner"></span>{/if}
                Save
            </button>
            <button type="button" class="btn btn-outline btn-error btn-sm" onclick={onDiscard} disabled={busy} data-testid="unsaved-discard">Discard</button>
            <button type="button" class="btn btn-ghost btn-sm" onclick={onCancel} disabled={busy} data-testid="unsaved-cancel">Cancel</button>
        </div>
    </div>
</div>
