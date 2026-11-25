<!-- Apply payment (cash books, invoicer I17): choose a deposit Ledgeline
     imported or synced, see how its income would be split onto the invoice's
     accounts, then apply it. The invoicer never creates a payment: the
     deposit is re-split, its bank side kept as imported.

     The deposits are the server's candidates, best first, the first chosen.
     Each choice is previewed (a dry run of the very apply, writing nothing)
     before Apply payment is on. Keyboard: focus lands on the chosen deposit
     once they load; the arrow keys choose another (radio buttons), Tab moves
     on to Cancel and Apply payment, Escape cancels (ConfirmDialog). -->
<script lang="ts">
    import {onMount, tick} from "svelte";
    import {applyPayment, fetchPaymentOptions} from "#lib/editorApi.js";
    import {errorText} from "#lib/labels.js";
    import {candidateHints, paymentOutcome, type PaymentOptions, type PaymentResult} from "#lib/payments.js";
    import ConfirmDialog from "./ConfirmDialog.svelte";

    let {number, onApplied, onCancel}: {number: string; onApplied: (result: PaymentResult) => void; onCancel: () => void} = $props();

    let options = $state<PaymentOptions | null>(null);
    let loadError = $state<string | null>(null);
    let chosen = $state<string | null>(null);
    let preview = $state<PaymentResult | null>(null);
    let previewError = $state<string | null>(null);
    let applying = $state(false);
    let applyError = $state<string | null>(null);
    let list = $state<HTMLFieldSetElement | null>(null);

    const uid = $props.id();
    /** Only the newest preview answers: a slower one for an earlier choice is stale. */
    let latest = 0;

    async function load(): Promise<void> {
        try {
            options = await fetchPaymentOptions(number);
        } catch (e: unknown) {
            loadError = errorText(e);
            return;
        }
        const first = options.candidates[0]?.reference ?? null;
        if (first !== null) {
            choose(first);
            await tick();
            // Still on the safe choice (Cancel), where the dialog opened: on to the chosen deposit.
            const active = document.activeElement;
            if (active === null || active === document.body || (active instanceof HTMLElement && active.hasAttribute("data-safe-choice"))) {
                list?.querySelector<HTMLInputElement>("input:checked")?.focus();
            }
        }
    }

    function choose(reference: string): void {
        chosen = reference;
        preview = null;
        previewError = null;
        applyError = null;
        const mine = ++latest;
        applyPayment(number, reference, {dryRun: true})
            .then((p) => {
                if (mine === latest) preview = p;
            })
            .catch((e: unknown) => {
                if (mine === latest) previewError = errorText(e);
            });
    }

    async function confirm(): Promise<void> {
        if (chosen === null || preview === null || applying) return;
        applying = true;
        applyError = null;
        try {
            onApplied(await applyPayment(number, chosen));
        } catch (e: unknown) {
            applyError = errorText(e);
        } finally {
            applying = false;
        }
    }

    onMount(() => void load());
</script>

<ConfirmDialog
    title="Apply a payment to invoice {number}"
    confirmLabel="Apply payment"
    busy={applying}
    confirmDisabled={preview === null}
    error={applyError}
    onConfirm={() => void confirm()}
    {onCancel}
    testid="apply-payment"
>
    {#if loadError !== null}
        <div class="alert text-sm alert-error" role="alert" data-testid="apply-payment-load-error">{loadError}</div>
    {:else if options === null}
        <span class="loading loading-sm loading-spinner" aria-label="Loading the deposits"></span>
    {:else}
        <p data-testid="apply-payment-due">
            {options.remaining} still due.
        </p>
        {#if options.candidates.length === 0}
            <p class="text-base-content/70" data-testid="apply-payment-none">
                No imported deposit can pay it yet. Import or sync the bank in Ledgeline; a deposit from this customer, or naming the invoice, of no more than
                what is due shows here.
            </p>
        {:else}
            <fieldset class="flex flex-col gap-1" bind:this={list} aria-describedby="{uid}-how">
                <legend class="mb-1 font-medium">Imported deposits</legend>
                {#each options.candidates as candidate (candidate.reference)}
                    <label
                        class="flex cursor-pointer items-start gap-2 rounded-box border border-base-300 px-2 py-1.5 has-checked:border-primary"
                        data-testid="apply-payment-candidate"
                    >
                        <input
                            type="radio"
                            class="radio mt-0.5 radio-sm"
                            name="{uid}-deposit"
                            value={candidate.reference}
                            checked={chosen === candidate.reference}
                            onchange={() => choose(candidate.reference)}
                        />
                        <span class="flex min-w-0 grow flex-col">
                            <span class="flex flex-wrap items-baseline gap-x-2">
                                <span class="tabular-nums">{candidate.date}</span>
                                <span class="min-w-0 truncate">{candidate.description}</span>
                                <span class="ml-auto font-medium tabular-nums">{candidate.received}</span>
                            </span>
                            <span class="text-xs text-base-content/60">{candidateHints(candidate).join(" · ")}</span>
                        </span>
                    </label>
                {/each}
            </fieldset>
            <p id="{uid}-how" class="text-xs text-base-content/60">
                Its income is split onto the invoice's accounts and tagged with the invoice; the bank side stays as imported.
            </p>
            <div aria-live="polite" data-testid="apply-payment-preview">
                {#if previewError !== null}
                    <div class="alert text-sm alert-error" role="alert" data-testid="apply-payment-preview-error">{previewError}</div>
                {:else if preview === null}
                    <span class="loading loading-xs loading-spinner" aria-label="Working out the split"></span>
                {:else}
                    <table class="table table-sm" aria-label="The split">
                        <thead>
                            <tr>
                                <th>Account</th>
                                <th class="text-right">Amount</th>
                            </tr>
                        </thead>
                        <tbody>
                            {#each preview.applied as share (share.account)}
                                <tr data-testid="apply-payment-share">
                                    <td class="font-mono text-xs break-all">{share.account}</td>
                                    <td class="text-right whitespace-nowrap tabular-nums">{share.amount}</td>
                                </tr>
                            {/each}
                        </tbody>
                    </table>
                    <p class="mt-1" data-testid="apply-payment-outcome">{paymentOutcome(preview)}</p>
                {/if}
            </div>
        {/if}
    {/if}
</ConfirmDialog>
