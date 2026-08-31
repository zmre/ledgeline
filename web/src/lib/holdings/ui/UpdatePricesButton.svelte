<!-- Holdings tab, Stocks sub-tab: "Update prices" (TODO.md "Stocks" — a Rust
     port of the user's own `update-prices.sh`, generalized to any hledger
     journal). `+page.svelte` loads `pricesStore.status` alongside the holdings
     report; this component only renders it and drives the two writes.

     Click flow: if nothing can hold a price yet, create `prices.journal` first
     (mirrors BudgetEditor's "Create budget.journal" affordance) — then fetch
     and append. Both writes go through `pricesStore`, which reloads its own
     status afterward; the Holdings report itself is reloaded by `+page.svelte`
     so the newly-priced positions' market values update on screen. -->
<script lang="ts">
    import type {Dec} from "$lib/domain/money";
    import type {PriceResult} from "$lib/holdings/pricesTypes";
    import {pricesStore} from "$lib/stores/prices.svelte";

    let {format}: {format: (qty: Dec) => string} = $props();

    const status = $derived(pricesStore.status.value);
    let error = $state<string | null>(null);
    let lastResults = $state<PriceResult[] | null>(null);

    const OUTCOME_LABEL: Record<PriceResult["outcome"], string> = {
        updated: "updated",
        duplicate: "already up to date",
        "not-found": "no quote found",
        "fetch-error": "fetch failed",
    };

    /**
     * Where this click writes, and why `canCreateFile` is asked FIRST.
     *
     * `canCreateFile` is the engine's answer to "does any file this journal
     * includes already hold a `P` directive?" — it is true only when none does
     * (and nothing is already sitting at the name it would create). So a
     * journal that already prices things keeps its own file, whatever that file
     * is called: `prices.journal`, `history.journal`, `kurse.journal`. Where
     * prices live is answered by the directives, never by a filename.
     *
     * Only when the answer is "nowhere" is a file created — and that check has
     * to come before `defaultTarget`, because with no prices anywhere
     * `defaultTarget` is the engine's fallback guess and is usually the MAIN
     * journal. Taking it would append `P` lines into the file holding the
     * user's transactions, when giving them a file for prices was the point.
     */
    async function onClick(): Promise<void> {
        error = null;
        lastResults = null;
        let target: string | null;
        if (status?.canCreateFile === true) {
            const created = await pricesStore.createFile();
            if (!created.ok) {
                error = created.failure.message;
                return;
            }
            target = created.created.journalId;
        } else {
            target = status?.defaultTarget ?? null;
        }
        if (target === null) {
            error = "No journal file can hold a price update.";
            return;
        }
        const outcome = await pricesStore.update(target);
        if (!outcome.ok) {
            error = outcome.failure.message;
            return;
        }
        lastResults = outcome.result.results;
    }
</script>

{#if status !== null && status.editable && status.symbols.length > 0 && (status.defaultTarget !== null || status.canCreateFile)}
    <button type="button" class="btn btn-sm" disabled={pricesStore.busy} onclick={() => void onClick()} data-testid="update-prices">
        {#if pricesStore.busy}
            <span class="loading loading-xs loading-spinner"></span>
        {/if}
        Update prices
    </button>
{/if}

{#if lastResults !== null}
    {@const updated = lastResults.filter((r) => r.outcome === "updated").length}
    {@const duplicate = lastResults.filter((r) => r.outcome === "duplicate").length}
    {@const missing = lastResults.filter((r) => r.outcome === "not-found" || r.outcome === "fetch-error").length}
    <div class="toast toast-end z-30" data-testid="update-prices-results">
        <div class="alert flex flex-col items-start alert-success">
            <div class="flex w-full items-center justify-between gap-4">
                <span>
                    {updated} price{updated === 1 ? "" : "s"} updated{duplicate > 0 ? `, ${duplicate} already current` : ""}{missing > 0
                        ? `, ${missing} unavailable`
                        : ""}.
                </span>
                <button type="button" class="btn btn-sm" onclick={() => (lastResults = null)}>Dismiss</button>
            </div>
            {#if missing > 0}
                <ul class="list-inside list-disc text-xs opacity-80">
                    {#each lastResults.filter((r) => r.outcome !== "updated" && r.outcome !== "duplicate") as result (result.symbol)}
                        <li>{result.symbol}: {OUTCOME_LABEL[result.outcome]}</li>
                    {/each}
                </ul>
            {/if}
            {#if updated > 0}
                <ul class="list-inside list-disc text-xs opacity-80">
                    {#each lastResults.filter((r) => r.outcome === "updated") as result (result.symbol)}
                        <li>{result.symbol}: {result.price !== null ? format(result.price) : ""} as of {result.date}</li>
                    {/each}
                </ul>
            {/if}
        </div>
    </div>
{/if}

{#if error !== null}
    <div class="toast toast-end z-30">
        <div class="alert alert-error">
            <span class="max-w-xs truncate" title={error}>Updating prices failed: {error}</span>
            <button type="button" class="btn btn-sm" onclick={() => (error = null)}>Dismiss</button>
        </div>
    </div>
{/if}
