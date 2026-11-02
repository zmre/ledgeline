<script lang="ts">
    // The accounts of a multi-account upload — a "download all accounts" `.qbo`.
    //
    // One account at a time, never merged: a rules file names one `account1`,
    // and each account keeps its own CSV (and so its own `.latest` dedup
    // marker) and its own statement balance. Choosing another re-stages the
    // same file, which resets everything below, exactly as dropping a new one
    // would.
    import {statementChoiceDetail, statementChoiceLabel} from "../importModel";
    import type {StatementChoice} from "../importTypes";

    let {
        statements,
        selected,
        disabled,
        onChoose,
    }: {
        statements: readonly StatementChoice[];
        selected: number;
        disabled: boolean;
        onChoose: (index: number) => void;
    } = $props();
</script>

<section class="flex flex-col gap-2" aria-label="Account" data-testid="imports-statements">
    <h2 class="px-1 text-sm font-semibold tracking-tight">This file holds {statements.length} accounts — import them one at a time</h2>
    <div class="flex flex-wrap gap-2" role="radiogroup" aria-label="Account to import">
        {#each statements as choice, index (index)}
            {@const chosen = index === selected}
            <button
                type="button"
                role="radio"
                aria-checked={chosen}
                class="flex flex-col items-start rounded-box border px-3 py-2 text-left transition-colors {chosen
                    ? 'border-primary bg-primary/5'
                    : 'border-base-content/10 hover:border-base-content/30'}"
                {disabled}
                onclick={() => onChoose(index)}
                data-testid="imports-statement"
            >
                <span class="text-sm font-medium">{statementChoiceLabel(choice, index)}</span>
                <span class="text-xs text-base-content/60">{statementChoiceDetail(choice)}</span>
            </button>
        {/each}
    </div>
</section>
