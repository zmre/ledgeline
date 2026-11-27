<!-- Time's dialogs' frame (Accounts' dialogs, Bill and Pay): a modal with
     its title and, top right, its buttons (Save/Discard sit there in this
     app), over a form. Enter in a field submits, unless `enterSubmits` is off
     (the billing dialogs, which write the books: there ⌘↵ / Ctrl+↵ does it,
     from anywhere in the dialog). Escape cancels from anywhere, Tab stays
     inside, and closing hands focus back (`dismissible`); while open the page
     beneath has no keys (`modalKeys`). Focus starts on `initial` (the
     `[data-initial]` field). `header` adds to the title line (a step count). -->
<script lang="ts">
    import {untrack, type Snippet} from "svelte";
    import {dismissible} from "#ui/keys/dismissible.js";
    import {hint} from "#ui/keys/hint.svelte.js";
    import {modalKeys} from "#lib/keys/modalKeys.svelte.js";

    let {
        title,
        submitLabel,
        cancelLabel = "Cancel",
        showCancel = true,
        busy = false,
        submitDisabled = false,
        error = null,
        onSubmit,
        onCancel,
        children,
        header,
        testid,
        initial = "[data-initial]",
        wide = false,
        enterSubmits = true,
    }: {
        title: string;
        submitLabel: string;
        cancelLabel?: string;
        /** Off: only the submit button (a result to acknowledge). */
        showCancel?: boolean;
        busy?: boolean;
        submitDisabled?: boolean;
        error?: string | null;
        onSubmit: () => void;
        onCancel: () => void;
        children: Snippet;
        header?: Snippet;
        testid: string;
        /** What takes focus on open (a selector inside the dialog). */
        initial?: string;
        /** A wide dialog (a table inside). */
        wide?: boolean;
        /** Enter in a field submits; off, only ⌘↵ / Ctrl+↵ and the button do. */
        enterSubmits?: boolean;
    } = $props();

    modalKeys(untrack(() => testid));
    const uid = $props.id();

    function submit(event: SubmitEvent): void {
        event.preventDefault();
        if (!busy && !submitDisabled) onSubmit();
    }

    function keydown(event: KeyboardEvent): void {
        if (event.key !== "Enter" || event.defaultPrevented) return;
        if (event.metaKey || event.ctrlKey) {
            event.preventDefault();
            if (!busy && !submitDisabled) onSubmit();
        } else if (!enterSubmits && event.target instanceof HTMLInputElement && event.target.type !== "checkbox" && event.target.type !== "radio") {
            // A stray Enter in a field never writes the books.
            event.preventDefault();
        }
    }
</script>

<div class="modal modal-open" role="presentation">
    <div
        class={["modal-box", wide ? "@container w-[min(64rem,calc(100vw-1.5rem))] max-w-none" : "max-w-2xl"]}
        role="dialog"
        aria-modal="true"
        aria-labelledby="{uid}-title"
        data-testid={testid}
        use:dismissible={{onDismiss: onCancel, trap: true, initial}}
    >
        <!-- ⌘↵ submits from any field in it; the form is not a control of its own. -->
        <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
        <form onsubmit={submit} onkeydown={keydown} class="flex flex-col gap-3">
            <div class="flex flex-wrap items-center justify-between gap-2">
                <h2 id="{uid}-title" class="text-lg font-semibold">{title}</h2>
                {@render header?.()}
                <div class="ml-auto flex gap-2">
                    {#if showCancel}
                        <button type="button" class="btn btn-ghost btn-sm" onclick={onCancel} disabled={busy} data-testid="{testid}-cancel"
                            >{cancelLabel}</button
                        >
                    {/if}
                    <button
                        type="submit"
                        class="btn btn-primary btn-sm"
                        disabled={busy || submitDisabled}
                        title={enterSubmits ? undefined : hint(submitLabel, "⌘/Ctrl+↵")}
                        data-testid="{testid}-submit"
                    >
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
