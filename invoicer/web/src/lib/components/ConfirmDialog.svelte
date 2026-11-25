<!-- A confirmation before a step that writes the books (ARCHITECTURE §4):
     Send, Void, Re-book, saving a sent invoice. A daisyUI modal drawn open
     (a div, so it mounts as is in jsdom), with focus on the safe choice:
     Enter on a stray keypress cancels, never confirms. Escape cancels. -->
<script lang="ts">
    import {modalKeys} from "#lib/keys/modalKeys.svelte.js";
    import type {Snippet} from "svelte";

    let {
        title,
        confirmLabel,
        cancelLabel = "Cancel",
        tone = "primary",
        busy = false,
        error = null,
        onConfirm,
        onCancel,
        children,
        testid = "confirm-dialog",
    }: {
        title: string;
        confirmLabel: string;
        cancelLabel?: string;
        tone?: "primary" | "warning" | "error";
        busy?: boolean;
        error?: string | null;
        onConfirm: () => void;
        onCancel: () => void;
        children?: Snippet;
        testid?: string;
    } = $props();

    // While open, the page beneath has no keys (PLAN 5k, Y8).
    modalKeys("confirm-dialog");

    const uid = $props.id();
    const TONE = {primary: "btn-primary", warning: "btn-warning", error: "btn-error"} as const;

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
    <div class="modal-box" role="alertdialog" aria-modal="true" aria-labelledby="{uid}-title" data-testid={testid}>
        <h2 id="{uid}-title" class="text-lg font-semibold">{title}</h2>
        <div class="mt-2 flex flex-col gap-2 text-sm">{@render children?.()}</div>
        {#if error !== null}
            <div class="mt-3 alert text-sm alert-error" role="alert" data-testid="{testid}-error">{error}</div>
        {/if}
        <div class="modal-action">
            <button type="button" class="btn btn-ghost btn-sm" onclick={onCancel} disabled={busy} use:focusSafe>{cancelLabel}</button>
            <button type="button" class="btn btn-sm {TONE[tone]}" onclick={onConfirm} disabled={busy} data-testid="{testid}-confirm">
                {#if busy}<span class="loading loading-xs loading-spinner"></span>{/if}
                {confirmLabel}
            </button>
        </div>
    </div>
</div>
