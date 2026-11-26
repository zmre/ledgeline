<script lang="ts">
    // A payment's invoice candidates (D-editor, D-settle), as Find match's
    // focused view shows a transfer's: `GET /api/settlements/for/{index}`,
    // each proposal with what ties it (the invoice number, the customer), its
    // fit, and Link. A short payment asks for the fee account — prefilled
    // with the one last used on this receivable, and required without one.
    import {fmt} from "#lib/format/amounts.js";
    import {journal} from "#lib/stores/journal.svelte.js";
    import {accrual} from "../store.svelte";
    import type {SettleProposal, SettlementsView} from "../types";

    let {
        index,
        disabled = false,
        onLinked,
        onLoaded,
    }: {
        /** The payment's (or invoice's) transaction. */
        index: number;
        disabled?: boolean;
        /** A link landed: the editor reloads the transaction. */
        onLinked?: () => void;
        /** The candidates arrived: whether there is a clear one (the section opens for it). */
        onLoaded?: (clear: boolean) => void;
    } = $props();

    let view = $state.raw<SettlementsView | null>(null);
    let error = $state<string | null>(null);
    let fees = $state<Record<number, string>>({});
    let busy = $state(false);

    $effect(() => {
        const asked = index;
        let live = true;
        accrual.settlementsFor(asked).then(
            (found) => {
                if (!live) return;
                view = found;
                onLoaded?.(found?.proposals.some((proposal) => proposal.clear) ?? false);
            },
            (cause: unknown) => {
                if (live) error = cause instanceof Error ? cause.message : String(cause);
            }
        );
        return () => {
            live = false;
        };
    });

    const feeOf = (proposal: SettleProposal, at: number): string => fees[at] ?? proposal.feeAccount ?? "";

    async function link(proposal: SettleProposal, at: number): Promise<void> {
        busy = true;
        error = null;
        const fee = proposal.fit.kind === "short" ? feeOf(proposal, at) : null;
        const result = await accrual.link(proposal, fee);
        busy = false;
        if (result.ok) onLinked?.();
        else error = result.failure.message;
    }
</script>

<div class="flex flex-col gap-2 text-sm" data-testid="accrual-settlement-picker">
    {#if error !== null}
        <p class="text-xs text-error" role="alert">{error}</p>
    {/if}
    {#if view === null}
        <span class="loading loading-xs loading-spinner" aria-label="Loading invoice candidates"></span>
    {:else if view.proposals.length === 0}
        <p class="text-xs text-muted">No open invoice matches this payment.</p>
    {:else}
        {#each view.proposals as proposal, at (`${proposal.invoice.index}:${proposal.invoice.posting}:${proposal.payment.index}`)}
            <div class="flex flex-col gap-1 rounded border border-base-content/10 p-2" data-testid="accrual-settle-candidate">
                <div class="flex flex-wrap items-baseline justify-between gap-2">
                    <span>
                        <span class="font-mono text-xs">{proposal.invoice.date}</span>
                        {proposal.invoice.description}
                        {#if proposal.invoice.invoice !== null}<span class="badge badge-ghost badge-sm">INV {proposal.invoice.invoice}</span>{/if}
                    </span>
                    <span class="tabular-nums">{fmt(proposal.invoice.amount.commodity, proposal.invoice.amount.quantity, journal.reportStyles)}</span>
                </div>
                <p class="text-xs text-muted">
                    {proposal.byNumber ? "Same invoice number" : proposal.byParty ? "Same customer" : "Same amount"} · {proposal.gapDays} days later
                    {#if proposal.fit.kind === "short" && proposal.fit.difference !== null}
                        · short {fmt(proposal.invoice.amount.commodity, proposal.fit.difference, journal.reportStyles)} (a fee?)
                    {:else if proposal.fit.kind === "over"}
                        · an overpayment: check it by hand
                    {/if}
                    {#if proposal.ambiguous}· other candidates too{/if}
                </p>
                {#if proposal.fit.kind === "short"}
                    <label class="flex items-center gap-2 text-xs">
                        Fee account
                        <input
                            class="input grow input-xs"
                            value={feeOf(proposal, at)}
                            oninput={(event) => (fees = {...fees, [at]: event.currentTarget.value})}
                            data-testid="accrual-fee-account"
                        />
                    </label>
                {/if}
                <button
                    type="button"
                    class="btn self-end btn-primary btn-xs"
                    disabled={disabled || busy || proposal.fit.kind === "over" || (proposal.fit.kind === "short" && feeOf(proposal, at).trim() === "")}
                    onclick={() => void link(proposal, at)}
                    data-testid="accrual-settle-link">Link</button
                >
            </div>
        {/each}
    {/if}
</div>
