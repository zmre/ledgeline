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
        onSelect,
    }: {
        /** Ranked best-first by the engine. This component never re-orders them. */
        candidates: readonly RulesCandidate[];
        selectedId: string | null;
        disabled: boolean;
        onSelect: (id: string | null) => void;
    } = $props();

    const cards = $derived(candidateCards(candidates));
</script>

<section class="flex flex-col gap-2" aria-label="Rules file" data-testid="imports-candidates">
    <h2 class="px-1 text-sm font-semibold tracking-tight">Read it with</h2>

    {#if candidates.length === 0}
        <div class="border-base-content/10 rounded-box flex flex-col items-start gap-2 border p-3" data-testid="imports-no-candidates">
            <p class="text-sm">
                None of the <code>*.rules</code> files beside your journal fit this data, so Ledgeline has nothing to read it through.
            </p>
            <p class="text-base-content/60 text-sm">
                You can still save the converted CSV — the destination below is where it goes — and write the rules file by hand next to it. The
                <strong>Edit Rules</strong> tab will pick it up as soon as it exists.
            </p>
            <!-- Disabled, with the reason in the tooltip rather than hidden:
                 generating a rules file from a CSV is the next work package, and
                 an absent button reads as "this is impossible" rather than "not
                 yet". The tooltip is on the WRAPPER because a disabled button
                 fires no pointer events, so a tooltip on it never opens. -->
            <span class="tooltip tooltip-right" data-tip="Generating a rules file from your data is the next piece of work — it isn't built yet.">
                <button type="button" class="btn btn-sm" disabled data-testid="imports-create-rules">Create rules file…</button>
            </span>
        </div>
    {:else}
        {#each cards as card (card.candidate.id)}
            {@const chosen = card.candidate.id === selectedId}
            <label
                class="rounded-box flex cursor-pointer gap-3 border p-3 transition-colors {chosen ? 'border-primary bg-primary/5' : 'border-base-content/10'}"
            >
                <input
                    type="radio"
                    class="radio radio-sm mt-1 shrink-0"
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
                    <span class="text-base-content/50 truncate text-xs">{card.candidate.id}</span>
                    <ul class="text-xs">
                        {#each signalLines(card.candidate.signals) as line (line.text)}
                            <li class={line.bad ? "text-warning" : "text-base-content/60"}>{line.text}</li>
                        {/each}
                    </ul>
                    {#if card.sample.length > 0}
                        <span class="bg-base-200 rounded p-2 font-mono text-xs whitespace-pre-wrap"
                            >{card.sample
                                .map((txn) => [`${txn.date} ${txn.description}`, ...txn.postings.map((posting) => `    ${posting}`)].join("\n"))
                                .join("\n\n")}</span
                        >
                    {/if}
                </span>
            </label>
        {/each}

        <label
            class="rounded-box flex cursor-pointer items-center gap-3 border p-3 {selectedId === null
                ? 'border-primary bg-primary/5'
                : 'border-base-content/10'}"
        >
            <input
                type="radio"
                class="radio radio-sm shrink-0"
                name="import-rules-candidate"
                checked={selectedId === null}
                {disabled}
                onchange={() => onSelect(null)}
            />
            <span class="flex flex-col">
                <span class="font-medium">Don't import — just save the CSV</span>
                <span class="text-base-content/60 text-xs">Writes the converted file to the destination below and leaves your journal alone.</span>
            </span>
        </label>
    {/if}
</section>
