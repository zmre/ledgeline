<!-- "Needs a look" (plan 43 §3.4, §4.2): rows a sync held back because they
     look like a transaction already in the journal. Held rows never write on
     their own; each gets the two answers. -->
<script lang="ts">
    import {localToday} from "#lib/stores/filters.svelte.js";
    import {formatMoney, shortDate} from "../format";
    import {syncStore} from "../syncStore.svelte";
    import type {WireLinkedAccount} from "../types";

    let {account, currency = "USD"}: {account: WireLinkedAccount; currency?: string} = $props();

    let pending = $state<string | null>(null);
    let error = $state<string | null>(null);
    const today = localToday();

    async function resolveHeld(heldId: string, action: "import" | "duplicate"): Promise<void> {
        pending = heldId;
        error = null;
        const outcome = await syncStore.resolveHeld(account.key, heldId, action);
        pending = null;
        if (!outcome.ok) error = outcome.failure.message;
    }
</script>

<section class="flex flex-col gap-2" aria-label="Needs a look" data-testid="sync-held">
    <h3 class="text-sm font-semibold">Needs a look</h3>
    {#if account.held.length === 0}
        <p class="text-sm text-base-content/60">Nothing is waiting.</p>
    {:else}
        <p class="text-xs text-base-content/70">These look like transactions already in your journal, so they weren't added.</p>
        <ul class="flex flex-col gap-2">
            {#each account.held as held (held.heldId)}
                <li class="flex flex-col gap-1 rounded-box border border-base-content/10 p-2 text-sm" data-testid="sync-held-row">
                    <div class="grid grid-cols-[auto_1fr_auto] gap-x-3 gap-y-0.5">
                        <span class="text-xs text-base-content/60">From the bank</span>
                        <span class="truncate">{shortDate(held.date, today)} · {held.payee}</span>
                        <span class="text-right font-mono">{formatMoney(held.amount, currency)}</span>
                        <span class="text-xs text-base-content/60">In your journal</span>
                        <span class="truncate">{shortDate(held.existing.date, today)} · {held.existing.description}</span>
                        <span class="text-right font-mono">{formatMoney(held.existing.amount, currency)}</span>
                    </div>
                    <div class="flex flex-wrap gap-2">
                        <button type="button" class="btn btn-xs" disabled={pending !== null} onclick={() => void resolveHeld(held.heldId, "duplicate")}>
                            {#if pending === held.heldId}<span class="loading loading-xs loading-spinner"></span>{/if}
                            It's a duplicate
                        </button>
                        <button type="button" class="btn btn-ghost btn-xs" disabled={pending !== null} onclick={() => void resolveHeld(held.heldId, "import")}>
                            Import anyway
                        </button>
                    </div>
                </li>
            {/each}
        </ul>
    {/if}
    {#if error !== null}
        <p class="text-xs text-error" role="alert">{error}</p>
    {/if}
</section>
