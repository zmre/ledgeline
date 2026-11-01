<script lang="ts">
    // Which rules file a journal card's rule goes to (plan 32 D27), above
    // Match in the rule builder:
    //
    //   Saving to [citi-2026.csv.rules ▾] (newest of 2 for citi)        Why?
    //   Choose a rules file [Choose… ▾]  the ledger doesn't say which    Why?
    //
    // The engine infers the candidates from the transaction's funding account,
    // the FITID convention and the `.latest` markers, then the year in the
    // file's name. When that leaves one file it is preselected; when it
    // doesn't (`needsChoice`) nothing is, and the overlay saves no rule until
    // the user picks. "Why?" lists every candidate with the engine's reasons.
    // With one candidate there is nothing to pick, so no select.
    import {rulesFileLabel} from "#lib/imports/ruleBuilder.js";
    import type {RuleChoice} from "#lib/imports/reviewTypes.js";
    import {fileReason} from "../journalRules";

    let {
        choice,
        picked,
        onPick,
        disabled = false,
    }: {
        choice: RuleChoice;
        /** The file the rule goes to, or null while the user has yet to choose. */
        picked: string | null;
        onPick: (rulesId: string) => void;
        disabled?: boolean;
    } = $props();

    let why = $state(false);

    const ids = $derived(choice.candidates.map((candidate) => candidate.rulesId));
    /** File names, or relative paths where two candidates share a file name. */
    const labelOf = (rulesId: string): string => rulesFileLabel(rulesId, ids);
    const current = $derived(picked === null ? null : (choice.candidates.find((candidate) => candidate.rulesId === picked) ?? null));
    const reason = $derived(current === null ? "" : fileReason(choice.candidates, current));

    function choose(value: string): void {
        if (value !== "") onPick(value);
    }
</script>

<div class="flex flex-col gap-1.5 text-sm" data-testid="review-rule-file" data-needs-choice={current === null ? "" : undefined}>
    <div class="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1">
        <!-- One select for both states, so it keeps focus when the first pick is made. -->
        {#if current === null}
            <span class="font-medium text-warning">Choose a rules file</span>
        {:else}
            <span class="text-base-content/70">Saving to</span>
        {/if}
        {#if current !== null && choice.candidates.length === 1}
            <strong class="font-mono font-semibold" data-testid="review-rule-file-name">{labelOf(current.rulesId)}</strong>
        {:else}
            <select
                class={["select w-auto max-w-full font-mono select-sm", current === null ? "min-w-40" : "select-ghost font-semibold"]}
                value={current?.rulesId ?? ""}
                onchange={(event) => choose(event.currentTarget.value)}
                {disabled}
                aria-label={current === null ? "Choose a rules file" : "Rules file"}
                data-testid="review-rule-file-select"
            >
                {#if current === null}
                    <option value="" disabled>Choose…</option>
                {/if}
                {#each choice.candidates as candidate (candidate.rulesId)}
                    <option value={candidate.rulesId} title={candidate.reasons.join(" ")}
                        >{labelOf(candidate.rulesId)}{candidate.recommended ? " (suggested)" : ""}</option
                    >
                {/each}
            </select>
        {/if}
        {#if current === null}
            <span class="text-xs text-base-content/55">the ledger doesn't say which</span>
        {:else if reason !== ""}
            <span class="text-xs text-base-content/55" data-testid="review-rule-file-reason">({reason})</span>
        {/if}
        <span class="grow"></span>
        <button
            type="button"
            class="text-xs text-base-content/55 underline-offset-2 hover:text-base-content hover:underline"
            aria-expanded={why}
            onclick={() => (why = !why)}
            data-testid="review-rule-file-why"
        >
            Why?
        </button>
    </div>
    {#if why}
        <ul class="flex flex-col gap-1 rounded-field bg-base-100/60 px-2 py-1.5 text-xs" data-testid="review-rule-file-reasons">
            {#each choice.candidates as candidate (candidate.rulesId)}
                <li class="flex min-w-0 flex-col">
                    <span class={["font-mono", candidate.rulesId === picked ? "font-semibold text-base-content" : "text-base-content/75"]}
                        >{candidate.rulesId}</span
                    >
                    <span class="text-base-content/55">{candidate.reasons.join(" · ") || "No reason given."}</span>
                </li>
            {/each}
        </ul>
    {/if}
</div>
