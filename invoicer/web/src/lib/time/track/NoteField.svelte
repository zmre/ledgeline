<!-- Today's note on one slot (plan 46 T-slots `n`). It warns as you type
     when hledger would misread the text (a `word:` it reads as a tag, a `;`
     or a line break that would end the note) and offers a rewording; the
     server makes the same check, and its refusal shows the same way, with
     its own suggestion. Enter saves, Escape cancels. -->
<script lang="ts">
    import {untrack} from "svelte";
    import {EditorApiError} from "#lib/editorApi.js";
    import {noteProblem, type NoteProblem} from "./format.js";

    let {
        label,
        initial = "",
        onSave,
        onCancel,
    }: {
        /** The slot's name, for the field's label. */
        label: string;
        initial?: string;
        /** Throws the server's refusal. */
        onSave: (note: string) => Promise<void>;
        onCancel: () => void;
    } = $props();

    const uid = $props.id();
    // Starts from the note already there; a re-read under it doesn't wipe what's typed.
    let text = $state(untrack(() => initial));
    let saving = $state(false);
    /** The server's refusal (`note-tag`), shown until the text changes. */
    let refused = $state<NoteProblem | null>(null);
    let failure = $state<string | null>(null);
    let field = $state<HTMLInputElement | null>(null);

    $effect(() => {
        field?.focus();
        field?.select();
    });

    const problem = $derived(refused ?? noteProblem(text));

    function edit(): void {
        refused = null;
        failure = null;
    }

    async function save(): Promise<void> {
        if (saving || problem !== null) return;
        saving = true;
        failure = null;
        try {
            await onSave(text.trim());
        } catch (error) {
            if (error instanceof EditorApiError && error.code === "note-tag") {
                const offending = typeof error.body.offending === "string" ? error.body.offending : "";
                const suggestion = typeof error.body.suggestion === "string" ? error.body.suggestion : null;
                refused = {offending, message: error.message, suggestion};
            } else failure = error instanceof Error ? error.message : String(error);
        } finally {
            saving = false;
        }
    }

    function useSuggestion(): void {
        if (problem?.suggestion == null) return;
        text = problem.suggestion;
        edit();
        field?.focus();
    }

    function onkeydown(event: KeyboardEvent): void {
        if (event.key === "Enter") {
            event.preventDefault();
            void save();
        } else if (event.key === "Escape") {
            event.preventDefault();
            onCancel();
        }
    }
</script>

<form
    class="flex flex-col gap-2 rounded-box border border-base-300 bg-base-200/50 p-3"
    onsubmit={(e) => {
        e.preventDefault();
        void save();
    }}
    data-testid="time-note"
>
    <div class="flex flex-wrap items-center gap-2">
        <label for="{uid}-note" class="text-sm font-medium">Note on {label}</label>
        <div class="ml-auto flex gap-2">
            <button type="button" class="btn btn-ghost btn-sm" onclick={onCancel} disabled={saving}>Discard</button>
            <button type="submit" class="btn btn-primary btn-sm" disabled={saving || problem !== null} data-testid="time-note-save">Save</button>
        </div>
    </div>
    <input
        bind:this={field}
        id="{uid}-note"
        class="input w-full {problem !== null ? 'input-warning' : ''}"
        type="text"
        autocomplete="off"
        placeholder="What you worked on"
        aria-invalid={problem !== null}
        aria-describedby={problem !== null ? `${uid}-problem` : undefined}
        bind:value={text}
        oninput={edit}
        {onkeydown}
        data-testid="time-note-input"
    />
    {#if problem !== null}
        <div id="{uid}-problem" class="flex flex-wrap items-center gap-2 text-sm" role="status" data-testid="time-note-problem">
            <span>{problem.message}</span>
            {#if problem.suggestion !== null}
                <button type="button" class="btn btn-xs" onclick={useSuggestion} data-testid="time-note-suggestion">Use “{problem.suggestion}”</button>
            {/if}
        </div>
    {/if}
    {#if failure !== null}
        <div class="alert text-sm alert-warning" role="alert" data-testid="time-note-error">{failure}</div>
    {/if}
</form>
