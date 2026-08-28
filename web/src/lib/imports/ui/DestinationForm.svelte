<script lang="ts">
    // Where the CSV lands and which journal the transactions go into.
    //
    // The journal select shows each file's transaction count and newest date
    // because the ranking is derived from exactly those two numbers and from no
    // filename at all — the engine ranks by "whose newest transaction is closest
    // to today", which is what gets year-files, month-files, a single file and
    // per-account files all right, and what demotes `accounts.journal` and
    // `prices.journal` without ever recognising their names. Showing the numbers
    // is what makes that legible instead of magic; a file offered with "no
    // transactions" explains its own position at the bottom.
    import {journalOptionLabel} from "../importModel";
    import type {JournalTarget} from "../importTypes";

    let {
        csvPath,
        journals,
        journalId,
        needsJournal,
        problems,
        disabled,
        onCsvPath,
        onJournal,
    }: {
        csvPath: string;
        journals: readonly JournalTarget[];
        journalId: string | null;
        /** False on the Save-CSV-only path: no rules file, so nothing is imported anywhere. */
        needsJournal: boolean;
        /** Complaints about the CSV path, from `validateCsvPath`. */
        problems: readonly string[];
        disabled: boolean;
        onCsvPath: (value: string) => void;
        onJournal: (value: string | null) => void;
    } = $props();

    // Unwritable targets are OFFERED but not selectable — a symlink or a file
    // outside the include root is still worth naming, so the user can see why
    // the one they expected is not an option.
    const selectable = $derived(journals.filter((journal) => journal.writable));
    const unwritable = $derived(journals.filter((journal) => !journal.writable));
</script>

<section class="flex flex-col gap-3 rounded-box border border-base-content/10 p-3" aria-label="Destinations" data-testid="imports-destinations">
    <h2 class="text-sm font-semibold tracking-tight">Where it goes</h2>

    <label class="form-control w-full">
        <span class="label-text text-xs">CSV file</span>
        <input
            type="text"
            class="input w-full font-mono input-sm {problems.length > 0 ? 'input-error' : ''}"
            value={csvPath}
            {disabled}
            spellcheck="false"
            autocomplete="off"
            data-testid="imports-csv-path"
            oninput={(event) => onCsvPath(event.currentTarget.value)}
        />
        <span class="label-text-alt text-xs text-base-content/50"
            >Relative to the folder your journal is in. hledger keeps its import state beside this file.</span
        >
    </label>
    {#each problems as problem (problem)}
        <p class="text-xs text-error" role="alert">{problem}</p>
    {/each}

    {#if needsJournal}
        <label class="form-control w-full">
            <span class="label-text text-xs">Import into</span>
            <select
                class="select w-full select-sm"
                value={journalId ?? ""}
                {disabled}
                data-testid="imports-journal"
                onchange={(event) => onJournal(event.currentTarget.value === "" ? null : event.currentTarget.value)}
            >
                <option value="">Choose a journal…</option>
                {#each selectable as journal (journal.id)}
                    <option value={journal.id}>{journalOptionLabel(journal)}</option>
                {/each}
            </select>
        </label>
        {#if selectable.length === 0}
            <p class="text-xs text-warning" role="alert">
                None of the files Ledgeline can see are writable, so there is nowhere to import to. Saving the CSV still works.
            </p>
        {/if}
        {#each unwritable as journal (journal.id)}
            <p class="text-xs text-base-content/50">
                {journal.label} can't be written to — it isn't a plain file inside the folder your journal is in.
            </p>
        {/each}
    {/if}
</section>
