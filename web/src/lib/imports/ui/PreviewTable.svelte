<script lang="ts">
    // What the converter made of the file: the detected format, the row count,
    // the first rows as a plain table, and every `ConvertNote` as a sentence.
    //
    // The notes are the point of this panel, not the table. Each one is a
    // JUDGEMENT the conversion made — a sheet it picked out of three, an
    // encoding it guessed, preamble rows it threw away, a running balance that
    // did not add up — and every one of them is a plausible reason an import
    // looks wrong a month later. Showing the rows without them would be showing
    // the answer without the assumptions.
    import {noteIsWarning, noteText, previewSummary, statementFacts} from "../importModel";
    import type {StagedFile} from "../importTypes";

    let {staged}: {staged: StagedFile} = $props();

    const facts = $derived(statementFacts(staged.statement));
    const columns = $derived(staged.preview.header ?? []);
</script>

<section class="flex flex-col gap-3 rounded-box border border-base-content/10 p-3" aria-label="Converted file" data-testid="imports-preview">
    <header class="flex flex-wrap items-center gap-2">
        <h2 class="grow text-sm font-semibold tracking-tight">{previewSummary(staged)}</h2>
        {#each facts as fact (fact.label)}
            <span class="badge badge-ghost badge-sm whitespace-nowrap">{fact.label}: {fact.value}</span>
        {/each}
    </header>

    {#each staged.notes as note, i (i)}
        <p class="text-sm {noteIsWarning(note) ? 'text-warning' : 'text-base-content/60'}" role={noteIsWarning(note) ? "alert" : undefined}>
            {noteText(note)}
        </p>
    {/each}
    {#if staged.unknownNoteCount > 0}
        <p class="text-xs text-base-content/50">
            {staged.unknownNoteCount}
            note{staged.unknownNoteCount === 1 ? "" : "s"} from the engine that this build of Ledgeline doesn't understand — it is newer than this page.
        </p>
    {/if}

    <div class="overflow-x-auto">
        <table class="table table-zebra table-xs">
            {#if columns.length > 0}
                <thead>
                    <tr>
                        {#each columns as column, i (i)}
                            <th class="whitespace-nowrap">{column}</th>
                        {/each}
                    </tr>
                </thead>
            {/if}
            <tbody>
                {#each staged.preview.rows as row, r (r)}
                    <tr>
                        {#each row as cell, c (c)}
                            <td class="whitespace-nowrap">{cell}</td>
                        {/each}
                    </tr>
                {/each}
            </tbody>
        </table>
    </div>

    {#if staged.preview.rows.length === 0}
        <p class="text-sm text-base-content/60">The conversion produced no rows at all — there is nothing here to import.</p>
    {/if}
</section>
