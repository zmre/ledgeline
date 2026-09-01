<script lang="ts">
    // The ranked rules files, as radio cards.
    //
    // Each card shows the score, the counts that produced it, and two of the
    // transactions that rules file would actually make. The sample is not
    // decoration: fact 4 of the plan is that a MISMATCHED rules file frequently
    // parses, exits 0, and produces garbage — `income:unknown` postings, a
    // posting with no amount, amounts with no commodity that form a separate
    // commodity so the `$` balance never moves. `hledger check` is happy with
    // all of it. A percentage alone cannot convey that; two real transactions
    // and a line saying "12 amounts with no currency" can.
    //
    // The last card is always "just save the CSV". Deselecting has to be
    // reachable — it is what turns the action button into `Save CSV` — and a
    // radio group with no way back to "none" is a trap.
    import {candidateCards, formatScore, scoreTone, signalLines} from "../importModel";
    import type {RulesCandidate} from "../importTypes";

    let {
        candidates,
        selectedId,
        disabled,
        creating,
        createdId,
        onSelect,
        onCreate,
    }: {
        /** Ranked best-first by the engine. This component never re-orders them. */
        candidates: readonly RulesCandidate[];
        selectedId: string | null;
        disabled: boolean;
        /** The Create panel is open, so its own button should not offer to open it again. */
        creating: boolean;
        /** A rules file this session wrote, so the empty state can say so instead of repeating itself. */
        createdId: string | null;
        onSelect: (id: string | null) => void;
        onCreate: () => void;
    } = $props();

    const cards = $derived(candidateCards(candidates));
</script>

<section class="flex flex-col gap-2" aria-label="Rules file" data-testid="imports-candidates">
    <h2 class="px-1 text-sm font-semibold tracking-tight">Read it with</h2>

    {#if candidates.length === 0}
        <div class="flex flex-col items-start gap-2 rounded-box border border-base-content/10 p-3" data-testid="imports-no-candidates">
            <p class="text-sm">
                None of the <code>*.rules</code> files beside your journal fit this data, so Ledgeline has nothing to read it through.
            </p>
            <p class="text-sm text-base-content/60">
                Ledgeline can write one for you: it reads your file, guesses which column is the date, the payee and the amount, and shows you the mapping
                before anything is saved. You can still just keep the converted CSV instead — the destination below is where it goes.
            </p>
            {#if createdId !== null}
                <!-- A file WAS written and this list is still empty, which is a
                     real outcome and a confusing one: the new file scored zero
                     against the very data it was written for. Saying so beats
                     an unchanged empty state that reads as "nothing happened". -->
                <p class="text-sm text-warning" data-testid="imports-create-no-match">
                    <code>{createdId}</code> was created, but it still does not match this file — open it in
                    <strong>Edit Rules</strong> and check the column mapping.
                </p>
            {/if}
            <button type="button" class="btn btn-primary btn-sm" disabled={disabled || creating} onclick={onCreate} data-testid="imports-create-rules">
                Create rules file…
            </button>
        </div>
    {:else}
        {#each cards as card (card.candidate.id)}
            {@const chosen = card.candidate.id === selectedId}
            <label
                class="flex cursor-pointer gap-3 rounded-box border p-3 transition-colors {chosen ? 'border-primary bg-primary/5' : 'border-base-content/10'}"
            >
                <input
                    type="radio"
                    class="radio mt-1 shrink-0 radio-sm"
                    name="import-rules-candidate"
                    checked={chosen}
                    {disabled}
                    onchange={() => onSelect(card.candidate.id)}
                />
                <span class="flex min-w-0 grow flex-col gap-2">
                    <span class="flex flex-wrap items-center gap-2">
                        <span class="grow truncate font-medium">{card.candidate.label}</span>
                        <span class="badge badge-{scoreTone(card.candidate.score)} badge-sm">{formatScore(card.candidate.score)}</span>
                    </span>
                    <span class="truncate text-xs text-base-content/50">{card.candidate.id}</span>
                    <ul class="text-xs">
                        {#each signalLines(card.candidate.signals) as line (line.text)}
                            <li class={line.bad ? "text-warning" : "text-base-content/60"}>{line.text}</li>
                        {/each}
                    </ul>
                    {#if card.sample.length > 0}
                        <span class="rounded bg-base-200 p-2 font-mono text-xs whitespace-pre-wrap"
                            >{card.sample
                                .map((txn) => [`${txn.date} ${txn.description}`, ...txn.postings.map((posting) => `    ${posting}`)].join("\n"))
                                .join("\n\n")}</span
                        >
                    {/if}
                </span>
            </label>
        {/each}

        <label
            class="flex cursor-pointer items-center gap-3 rounded-box border p-3 {selectedId === null
                ? 'border-primary bg-primary/5'
                : 'border-base-content/10'}"
        >
            <input
                type="radio"
                class="radio shrink-0 radio-sm"
                name="import-rules-candidate"
                checked={selectedId === null}
                {disabled}
                onchange={() => onSelect(null)}
            />
            <span class="flex flex-col">
                <span class="font-medium">Don't import — just save the CSV</span>
                <span class="text-xs text-base-content/60">Writes the converted file to the destination below and leaves your journal alone.</span>
            </span>
        </label>
    {/if}
</section>
