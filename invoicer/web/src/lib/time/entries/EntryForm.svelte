<!-- One time line, to add or change (plan 46 T-entries): date · account ·
     duration · note, or for a timeclock session its start and end (the
     session is the record there). Enter saves (in the account picker, the
     first Enter takes the suggestion, the second saves); Escape cancels.

     The note is checked as it is typed: hledger reads `word:` in a note as a
     tag and `;` as its end, so the form warns, and when the server refuses
     one (`note-tag`) it offers the server's rewrite. Narrow widths: the
     fields wrap, the note taking a row of its own. -->
<script lang="ts">
    import {untrack} from "svelte";
    import AccountInput from "#ui/components/AccountInput.svelte";
    import DateInput from "#ui/components/DateInput.svelte";
    import type {EntryEdit, EntryView, NewEntry} from "../api/entries.js";
    import type {WriteOutcome} from "./entriesStore.svelte.js";
    import {parseDuration} from "../duration.js";
    import {noteProblem} from "../note.js";

    let {
        entry = null,
        today,
        accounts,
        person,
        onAdd,
        onEdit,
        onCancel,
        testid = "entry-form",
    }: {
        /** The line to change; null adds one. */
        entry?: EntryView | null;
        /** The date a new line starts on. */
        today: string;
        /** The account picker's names (archived ones left out). */
        accounts: string[];
        /** Whose file a new line goes in, when the list is filtered to one person. */
        person?: string;
        onAdd?: (entry: NewEntry) => Promise<WriteOutcome>;
        onEdit?: (edit: EntryEdit) => Promise<WriteOutcome>;
        onCancel: () => void;
        testid?: string;
    } = $props();

    const original = untrack(() => entry);
    const timeclock = original?.format === "timeclock";

    let date = $state(original?.date ?? untrack(() => today));
    let account = $state(original?.account ?? "");
    let duration = $state(original === null ? "" : original.quantity.text.replace(" ", ""));
    let note = $state(original?.note ?? "");
    let start = $state("");
    let end = $state("");
    let busy = $state(false);
    let error = $state<string | null>(null);
    let suggestion = $state<string | null>(null);

    const warning = $derived(noteProblem(note));
    const durationBad = $derived(duration.trim() !== "" && parseDuration(duration) === null);

    /** What the server is asked to change: only what differs from the line. */
    function edits(line: EntryView): EntryEdit {
        const out: EntryEdit = {id: line.id};
        if (timeclock) {
            if (start !== "") out.start = start;
            if (end !== "") out.end = end;
            return out;
        }
        if (date !== line.date) out.date = date;
        if (account.trim() !== line.account) out.account = account.trim();
        const seconds = parseDuration(duration);
        if (seconds !== null && Math.round(seconds / 60) !== Math.round(line.quantity.seconds / 60)) out.duration = duration.trim();
        if (note.trim() !== line.note) out.note = note.trim();
        return out;
    }

    function check(): string | null {
        if (timeclock) return start === "" && end === "" ? "Change the start or the end." : null;
        if (date === "") return "Pick a date.";
        if (account.trim() === "") return "Pick an account.";
        if (parseDuration(duration) === null) return "Write a duration such as 45m, 1h30, 1.5 or 0:45.";
        return null;
    }

    async function submit(event: SubmitEvent): Promise<void> {
        event.preventDefault();
        if (busy) return;
        error = check();
        suggestion = null;
        if (error !== null) return;
        busy = true;
        let outcome: WriteOutcome;
        if (original === null) {
            const fields: NewEntry = {date, account: account.trim(), duration: duration.trim(), note: note.trim()};
            if (person !== undefined) fields.person = person;
            outcome = (await onAdd?.(fields)) ?? {ok: true};
        } else {
            const change = edits(original);
            if (Object.keys(change).length === 1) {
                busy = false;
                onCancel();
                return;
            }
            outcome = (await onEdit?.(change)) ?? {ok: true};
        }
        busy = false;
        if (outcome.ok) return;
        error = outcome.message;
        if (outcome.code === "note-tag" && typeof outcome.body.suggestion === "string") suggestion = outcome.body.suggestion;
    }

    function onKeydown(event: KeyboardEvent): void {
        if (event.key !== "Escape" || event.defaultPrevented) return;
        event.preventDefault();
        onCancel();
    }

    function useSuggestion(): void {
        if (suggestion === null) return;
        note = suggestion;
        suggestion = null;
        error = null;
    }
</script>

<!-- Escape works from any field in it; the form is not a control of its own. -->
<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<form
    class="flex flex-col gap-2 rounded-box border border-primary/40 bg-base-200/60 p-3"
    onsubmit={(event) => void submit(event)}
    onkeydown={onKeydown}
    aria-label={original === null ? "Add time" : "Change this time"}
    data-testid={testid}
>
    <div class="flex flex-wrap items-end gap-x-3 gap-y-2">
        {#if timeclock}
            <p class="basis-full text-xs text-base-content/70">A clocked session changes by its start and end. Leave one empty to keep it.</p>
            <label class="flex grow basis-48 flex-col gap-1 text-sm sm:grow-0"
                >Start
                <input class="input w-full input-sm" type="datetime-local" step="1" bind:value={start} data-testid="{testid}-start" />
            </label>
            <label class="flex grow basis-48 flex-col gap-1 text-sm sm:grow-0"
                >End
                <input class="input w-full input-sm" type="datetime-local" step="1" bind:value={end} data-testid="{testid}-end" />
            </label>
        {:else}
            <label class="flex grow basis-36 flex-col gap-1 text-sm sm:grow-0"
                >Date
                <DateInput bind:value={date} class="w-full" required data-testid="{testid}-date" />
            </label>
            <div class="flex min-w-0 grow basis-56 flex-col gap-1 text-sm" data-testid="{testid}-account">
                <span aria-hidden="true">Account</span>
                <AccountInput bind:value={account} accountNames={accounts} placeholder="clients:acme" autofocus={original === null} />
            </div>
            <label class="flex grow basis-24 flex-col gap-1 text-sm sm:grow-0"
                >Duration
                <input
                    class={["input w-full input-sm sm:w-24", durationBad && "input-warning"]}
                    bind:value={duration}
                    placeholder="45m"
                    autocomplete="off"
                    aria-invalid={durationBad}
                    data-testid="{testid}-duration"
                />
            </label>
            <label class="flex min-w-0 grow basis-full flex-col gap-1 text-sm md:basis-64"
                >Note
                <input
                    class={["input w-full input-sm", warning !== null && "input-warning"]}
                    bind:value={note}
                    autocomplete="off"
                    aria-describedby={warning === null ? undefined : `${testid}-note-warning`}
                    data-testid="{testid}-note"
                />
            </label>
        {/if}
        <div class="ml-auto flex gap-2">
            <button type="button" class="btn btn-ghost btn-sm" onclick={onCancel} disabled={busy}>Cancel</button>
            <button type="submit" class="btn btn-primary btn-sm" disabled={busy} data-testid="{testid}-save">
                {#if busy}<span class="loading loading-xs loading-spinner"></span>{/if}
                {original === null ? "Add" : "Save"}
            </button>
        </div>
    </div>
    {#if warning !== null && !timeclock}
        <p id="{testid}-note-warning" class="text-xs text-warning" data-testid="{testid}-note-warning">{warning.message}</p>
    {/if}
    {#if error !== null}
        <div class="alert py-2 text-sm alert-error" role="alert" data-testid="{testid}-error">
            <span>{error}</span>
            {#if suggestion !== null}
                <button type="button" class="btn btn-outline btn-xs" onclick={useSuggestion} data-testid="{testid}-use-suggestion">Use “{suggestion}”</button>
            {/if}
        </div>
    {/if}
</form>
