<!-- A confirmation before a step that writes the books (ARCHITECTURE §4):
     Send, Void, Re-book, saving a sent invoice. A daisyUI modal drawn open
     (a div, so it mounts as is in jsdom), with focus on the safe choice:
     Enter on a stray keypress cancels, never confirms. Escape cancels, from
     anywhere; Tab and Shift+Tab stay inside (fields included); closing hands
     focus back to what had it (`dismissible`). -->
<script lang="ts">
    import {dismissible} from "#ui/keys/dismissible.js";
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

    let box = $state<HTMLDivElement | null>(null);

    // While busy both buttons are disabled, which drops their focus: keep it
    // in the dialog, so Tab and Escape still land here.
    $effect(() => {
        if (!busy || box === null) return;
        const active = document.activeElement;
        if (active instanceof HTMLElement && box.contains(active) && !(active instanceof HTMLButtonElement && active.disabled)) return;
        box.tabIndex = -1;
        box.focus();
    });
</script>

<div class="modal modal-open" role="presentation">
    <div
        bind:this={box}
        class="modal-box"
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="{uid}-title"
        data-testid={testid}
        use:dismissible={{onDismiss: onCancel, trap: true, initial: "[data-safe-choice]"}}
    >
        <h2 id="{uid}-title" class="text-lg font-semibold">{title}</h2>
        <div class="mt-2 flex flex-col gap-2 text-sm">{@render children?.()}</div>
        {#if error !== null}
            <div class="mt-3 alert text-sm alert-error" role="alert" data-testid="{testid}-error">{error}</div>
        {/if}
        <div class="modal-action">
            <button type="button" class="btn btn-ghost btn-sm" onclick={onCancel} disabled={busy} data-safe-choice>{cancelLabel}</button>
            <button type="button" class="btn btn-sm {TONE[tone]}" onclick={onConfirm} disabled={busy} data-testid="{testid}-confirm">
                {#if busy}<span class="loading loading-xs loading-spinner"></span>{/if}
                {confirmLabel}
            </button>
        </div>
    </div>
</div>
