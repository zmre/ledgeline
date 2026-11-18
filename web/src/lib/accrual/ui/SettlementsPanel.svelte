<script lang="ts">
    // Invoice ↔ payment matching (plan 45 D-settle): the proposals, Link and
    // Link all clear, the fee edit for a short payment, and the invoices and
    // payments with no partner yet. Shown in the Recognition overlay's
    // Invoices tab and on the Invoices page (Q9).
    //
    // Writes are not optimistic: each waits for `accrual.link`, which re-reads
    // the journal and then this list, so a linked card simply goes away.
    import {fmt} from "#lib/format/amounts.js";
    import {shortDate} from "#lib/review/card.js";
    import ReviewIcon from "#lib/review/ui/ReviewIcon.svelte";
    import {journal} from "#lib/stores/journal.svelte.js";
    import {settings} from "#lib/stores/settings.svelte.js";
    import {accrual} from "../store.svelte";
    import type {Money, SettleProposal, SettleSide} from "../types";
    import {bpsToPercent, percentToBps, proposalKey} from "./format";

    const view = $derived(accrual.settlements);
    const styles = $derived(journal.reportStyles);
    const clear = $derived((view?.proposals ?? []).filter((proposal) => proposal.clear));
    const busy = $derived(accrual.busy);

    /** Fee accounts as typed, per proposal; absent means the engine's suggestion. */
    let fees = $state<Record<string, string>>({});
    let failure = $state<string | null>(null);
    let confirmAll = $state(false);
    let linkedAll = $state<number | null>(null);
    let tolerance = $state(bpsToPercent(settings.settlementToleranceBps));

    const money = (amount: Money): string => fmt(amount.commodity, amount.quantity, styles);
    const feeOf = (proposal: SettleProposal): string => fees[proposalKey(proposal)] ?? proposal.feeAccount ?? "";

    function differenceText(proposal: SettleProposal): string {
        const {fit} = proposal;
        if (fit.kind === "exact") return "Exact amount";
        const amount = fit.difference === null ? "" : fmt(proposal.invoice.amount.commodity, fit.difference, styles);
        return fit.kind === "short" ? `Short by ${amount} (a fee)` : `Over by ${amount}: check only, can't link`;
    }

    async function link(proposal: SettleProposal): Promise<void> {
        failure = null;
        const fee = proposal.fit.kind === "short" ? feeOf(proposal).trim() : null;
        if (fee !== null && fee === "") {
            failure = "Name the account the fee goes to.";
            return;
        }
        const outcome = await accrual.link(proposal, fee);
        if (!outcome.ok) failure = outcome.failure.message;
    }

    async function linkAll(): Promise<void> {
        if (!confirmAll) {
            confirmAll = true;
            return;
        }
        confirmAll = false;
        failure = null;
        const outcome = await accrual.linkAll();
        if (outcome.ok) linkedAll = outcome.value.linked;
        else failure = outcome.failure.message;
    }

    function setLookback(event: Event): void {
        settings.settlementLookbackDays = Number((event.currentTarget as HTMLSelectElement).value);
        void accrual.reload();
    }

    function commitTolerance(): void {
        const bps = percentToBps(tolerance);
        if (bps === null) {
            tolerance = bpsToPercent(settings.settlementToleranceBps);
            return;
        }
        settings.settlementToleranceBps = bps;
        tolerance = bpsToPercent(settings.settlementToleranceBps);
        void accrual.reload();
    }

    const LOOKBACKS = [30, 90, 180, 365, 730, 0] as const;
</script>

{#snippet side(label: string, item: SettleSide, testid: string)}
    <div class="flex min-w-0 flex-col gap-0.5 rounded-box border border-base-content/10 p-3" data-testid={testid}>
        <span class="text-xs tracking-wide text-base-content/50 uppercase">{label}</span>
        <span class="truncate font-medium" title={item.description}>{item.description || "(no description)"}</span>
        <span class="text-sm text-base-content/70"><span class="font-mono">{shortDate(item.date)}</span> · <span class="truncate">{item.account}</span></span>
        <span class="font-mono text-sm tabular-nums">{money(item.amount)}</span>
        {#if item.invoice !== null || item.party !== null}
            <span class="flex flex-wrap gap-1 text-xs">
                {#if item.invoice !== null}<span class="badge badge-ghost badge-sm">INV {item.invoice}</span>{/if}
                {#if item.party !== null}<span class="badge badge-ghost badge-sm">{item.party.name}</span>{/if}
            </span>
        {/if}
    </div>
{/snippet}

{#snippet openList(title: string, items: readonly SettleSide[], testid: string)}
    <section class="flex flex-col gap-1" data-testid={testid}>
        <h3 class="text-sm font-semibold">{title} <span class="badge badge-ghost badge-sm tabular-nums">{items.length}</span></h3>
        {#if items.length === 0}
            <p class="text-sm text-base-content/50">None.</p>
        {:else}
            <ul class="divide-y divide-base-content/10 text-sm">
                {#each items as item (`${item.index}:${item.posting}`)}
                    <li class="flex items-center gap-3 py-1">
                        <span class="w-16 shrink-0 font-mono text-base-content/60">{shortDate(item.date)}</span>
                        <span class="min-w-0 grow truncate">{item.description || "(no description)"}</span>
                        {#if item.invoice !== null}<span class="badge badge-ghost badge-sm">INV {item.invoice}</span>{/if}
                        <span class="shrink-0 font-mono tabular-nums">{money(item.amount)}</span>
                    </li>
                {/each}
            </ul>
        {/if}
    </section>
{/snippet}

<div class="flex flex-col gap-4" data-testid="settlements-panel">
    <div class="flex flex-wrap items-center gap-3">
        {#if view !== null}
            <p class="text-sm tabular-nums" data-testid="settlements-counts">
                <span class="font-semibold text-success">{view.counts.clear} clear</span>, <span class="font-semibold">{view.counts.check} to check</span>
            </p>
        {/if}
        <label class="flex items-center gap-1.5 text-sm">
            Look back
            <select class="select w-28 select-sm" value={String(settings.settlementLookbackDays)} onchange={setLookback} data-testid="settlements-lookback">
                {#each LOOKBACKS as days (days)}
                    <option value={String(days)}>{days === 0 ? "All time" : `${days} days`}</option>
                {/each}
            </select>
        </label>
        <label
            class="flex items-center gap-1.5 text-sm"
            title="How far a payment may fall short of its invoice and still be proposed, with the difference as a fee"
        >
            Fee tolerance
            <input
                class="input w-20 text-right tabular-nums input-sm"
                inputmode="decimal"
                bind:value={tolerance}
                onchange={commitTolerance}
                data-testid="settlements-tolerance"
            />%
        </label>
        <div class="ml-auto flex items-center gap-2">
            {#if confirmAll}
                <span class="text-sm" role="alert">Link {clear.length} clear {clear.length === 1 ? "pair" : "pairs"}?</span>
                <button type="button" class="btn btn-ghost btn-sm" onclick={() => (confirmAll = false)}>Keep</button>
            {/if}
            <button
                type="button"
                class="btn btn-primary btn-sm"
                disabled={busy || clear.length === 0}
                onclick={() => void linkAll()}
                data-testid="settlements-link-all"
            >
                <ReviewIcon name="link" class="h-3.5 w-3.5" />
                {confirmAll ? "Link them" : `Link all clear (${clear.length})`}
            </button>
        </div>
    </div>

    {#if linkedAll !== null}
        <p class="text-sm text-success" data-testid="settlements-linked-all">Linked {linkedAll} {linkedAll === 1 ? "pair" : "pairs"}.</p>
    {/if}
    {#if failure !== null}
        <div class="alert py-2 text-sm alert-error" role="alert" data-testid="settlements-failure">
            <span class="grow break-words">Not saved: {failure}</span>
            <button type="button" class="btn btn-ghost btn-xs" onclick={() => (failure = null)}>Dismiss</button>
        </div>
    {/if}

    {#if view === null}
        <div class="flex items-center justify-center gap-2 py-10 text-sm text-base-content/60" role="status">
            {#if accrual.error !== null}
                <span class="text-error">Couldn't read the invoices: {accrual.error}</span>
            {:else}
                <span class="loading loading-sm loading-spinner"></span> Reading invoices and payments…
            {/if}
        </div>
    {:else}
        {#if view.proposals.length === 0}
            <p class="py-6 text-center text-sm text-base-content/60" data-testid="settlements-empty">No invoice has a payment to propose in the lookback.</p>
        {/if}
        <datalist id="settlement-fee-accounts">
            {#each journal.accountNames as name (name)}
                <option value={name}></option>
            {/each}
        </datalist>
        {#each view.proposals as proposal (proposalKey(proposal))}
            {@const key = proposalKey(proposal)}
            <article class="flex flex-col gap-3 rounded-box border border-base-content/10 bg-base-100 p-4 shadow-sm" data-testid="settlement-card">
                <header class="flex flex-wrap items-center gap-2 text-sm">
                    <span class="font-semibold">{proposal.invoice.party?.name ?? proposal.payment.party?.name ?? proposal.invoice.account}</span>
                    {#if proposal.invoice.invoice !== null}<span class="badge badge-ghost badge-sm">INV {proposal.invoice.invoice}</span>{/if}
                    {#if proposal.clear}
                        <span class="badge badge-sm badge-success" data-testid="settlement-clear">clear</span>
                    {:else}
                        <span class="badge badge-sm badge-warning">check</span>
                    {/if}
                    {#if proposal.ambiguous}<span class="badge badge-sm badge-warning">ambiguous</span>{/if}
                    {#if proposal.byNumber}<span class="badge badge-ghost badge-sm">invoice number</span>{/if}
                    {#if proposal.byParty}<span class="badge badge-ghost badge-sm">same {proposal.invoice.party?.kind ?? "party"}</span>{/if}
                    <span class="text-base-content/60">{proposal.gapDays === 0 ? "same day" : `${proposal.gapDays} days later`}</span>
                </header>
                <div class="grid gap-3 sm:grid-cols-2">
                    {@render side("Invoice", proposal.invoice, "settlement-invoice")}
                    {@render side("Payment", proposal.payment, "settlement-payment")}
                </div>
                <div class="flex flex-wrap items-center gap-3">
                    <span class={["text-sm", proposal.fit.kind === "over" && "text-warning"]} data-testid="settlement-fit">{differenceText(proposal)}</span>
                    {#if proposal.fit.kind === "short"}
                        <label class="flex grow items-center gap-1.5 text-sm">
                            Fee account
                            <input
                                class="input min-w-48 grow input-sm"
                                list="settlement-fee-accounts"
                                placeholder="expenses:fees:payment processing"
                                value={feeOf(proposal)}
                                oninput={(event) => (fees = {...fees, [key]: event.currentTarget.value})}
                                data-testid="settlement-fee"
                            />
                        </label>
                    {/if}
                    <button
                        type="button"
                        class="btn ml-auto btn-primary btn-sm"
                        disabled={busy || proposal.fit.kind === "over" || proposal.invoice.fingerprint === null || proposal.payment.fingerprint === null}
                        onclick={() => void link(proposal)}
                        data-testid="settlement-link"
                    >
                        <ReviewIcon name="link" class="h-3.5 w-3.5" /> Link
                    </button>
                </div>
            </article>
        {/each}
        <div class="grid gap-4 sm:grid-cols-2">
            {@render openList("Open invoices and bills", view.open.invoices, "settlements-open-invoices")}
            {@render openList("Payments with no invoice", view.open.payments, "settlements-open-payments")}
        </div>
    {/if}
</div>
