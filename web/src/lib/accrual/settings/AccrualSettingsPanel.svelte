<!-- Settings → Books (plan 45 Phase 3, D-settings): post due recognition
     entries in the background (`autoRecognize`), Close books through… (the
     explicit action that writes `[books] closed-through`, Q1), and the invoice
     matcher's lookback and fee tolerance (Q3). Shown only with the accrual
     helpers on (D-gate); the tab itself is hidden otherwise. -->
<script lang="ts">
    import DateInput from "#ui/components/DateInput.svelte";
    import PinToggle from "#ui/components/PinToggle.svelte";
    import {SETTLEMENT_LEAD_RANGE, SETTLEMENT_LOOKBACK_RANGE, SETTLEMENT_TOLERANCE_RANGE, type PinnableUiPrefKey} from "#lib/settings/uiPrefs.js";
    import {localToday} from "#lib/stores/filters.svelte.js";
    import {settings} from "#lib/stores/settings.svelte.js";
    import {accrual} from "../store.svelte";
    import {dayShort} from "../model";

    const status = $derived(accrual.status);
    const closedThrough = $derived(status?.closedThrough ?? null);

    // Recognition and each invoice-matching setting can be kept for this
    // ledger alone (the matching ones are shared with the invoicer, which
    // offers the same pin).
    let pinBusy = $state(false);
    async function pin(key: PinnableUiPrefKey, pinned: boolean): Promise<void> {
        pinBusy = true;
        try {
            await settings.pinToLedger(key, pinned);
        } finally {
            pinBusy = false;
        }
    }

    /** The date typed into Close books through…, before it is confirmed. */
    let closeDate = $state<string>("");
    /** The confirm step: what is about to be written (a date, or null to reopen). */
    let confirming = $state<{date: string | null} | null>(null);
    let failure = $state<string | null>(null);

    const today = localToday();
    const closeDateOk = $derived(/^\d{4}-\d{2}-\d{2}$/.test(closeDate) && closeDate <= today);

    async function confirm(): Promise<void> {
        const asked = confirming;
        if (asked === null) return;
        failure = null;
        const result = await accrual.setClosedThrough(asked.date);
        confirming = null;
        if (result.ok) closeDate = "";
        else if (result.failure.kind !== "conflict") failure = result.failure.message;
    }

    /** Basis points as a percent with one decimal: 300 → "3.0". */
    const percent = (bps: number): string => (bps / 100).toFixed(1);
</script>

{#snippet matchingPin(key: PinnableUiPrefKey, testid: string)}
    {#if settings.canPinToLedger}
        <PinToggle pinned={settings.isPinned(key)} busy={pinBusy} onChange={(on) => void pin(key, on)} {testid} />
    {/if}
{/snippet}

<section class="flex max-w-3xl flex-col gap-5" data-testid="settings-books">
    {#if status === null}
        <p class="text-sm text-base-content/60">
            The revenue and expense recognition helpers appear when the ledger has a deferred-revenue, prepaid, receivable or payable account.
        </p>
    {:else}
        <section class="flex flex-col gap-3">
            <div class="flex flex-wrap items-baseline justify-between gap-2">
                <h2 class="text-base font-semibold">Recognition</h2>
                {#if settings.canPinToLedger}
                    <PinToggle pinned={settings.isPinned("autoRecognize")} busy={pinBusy} onChange={(on) => void pin("autoRecognize", on)} testid="books-pin" />
                {/if}
            </div>
            <label class="flex cursor-pointer items-center gap-3">
                <input
                    type="checkbox"
                    class="toggle toggle-primary"
                    checked={settings.autoRecognize}
                    onchange={(e) => (settings.autoRecognize = e.currentTarget.checked)}
                    data-testid="books-auto-recognize"
                />
                <span>Post due recognition entries automatically</span>
            </label>
            <p class="text-sm text-base-content/70">
                With this on, Ledgeline posts the entries that are due when it opens, once a day, after you save a schedule, and after the journal changes on
                disk. Entries in closed periods are never posted automatically.
            </p>
            {#if status.file !== null}
                <p class="text-sm text-base-content/70" data-testid="books-file">
                    Schedules live in <code>{status.file.path}</code>{status.file.exists ? "" : " (not created yet)"}. Your books don't include it.
                </p>
            {/if}
        </section>

        <section class="flex flex-col gap-3">
            <h2 class="text-base font-semibold">Closed periods</h2>
            <p class="text-sm" data-testid="books-closed-through">
                {#if closedThrough === null}
                    The books are open: no period is closed.
                {:else}
                    Closed through <span class="font-medium">{closedThrough}</span>. Entries on or before it are kept as they are; a change becomes one
                    adjustment dated today.
                {/if}
            </p>
            {#if confirming === null}
                <div class="flex flex-wrap items-center gap-2">
                    <span>Close books through</span>
                    <DateInput bind:value={closeDate} max={today} aria-label="Close books through" data-testid="books-close-date" />
                    <button
                        type="button"
                        class="btn btn-sm"
                        disabled={!closeDateOk || accrual.busy}
                        onclick={() => (confirming = {date: closeDate})}
                        data-testid="books-close">Close books…</button
                    >
                    {#if closedThrough !== null}
                        <button
                            type="button"
                            class="btn btn-ghost btn-sm"
                            disabled={accrual.busy}
                            onclick={() => (confirming = {date: null})}
                            data-testid="books-reopen">Reopen…</button
                        >
                    {/if}
                </div>
                {#if closeDate !== "" && !closeDateOk}
                    <p class="text-xs text-error">Pick a date on or before today.</p>
                {/if}
            {:else}
                <div class="flex flex-wrap items-center gap-2 rounded border border-warning/40 px-3 py-2 text-sm" role="alert" data-testid="books-confirm">
                    <span class="grow">
                        {#if confirming.date === null}
                            Reopen every period? Recognition entries before {closedThrough === null ? "" : dayShort(closedThrough)} can be posted into their own months
                            again.
                        {:else}
                            Close the books through {confirming.date}? Ledgeline writes <code>closed-through = "{confirming.date}"</code> to this ledger's
                            <code>ledgeline.toml</code>, creating it if needed.
                        {/if}
                    </span>
                    <button type="button" class="btn btn-warning btn-xs" disabled={accrual.busy} onclick={() => void confirm()} data-testid="books-confirm-go"
                        >{confirming.date === null ? "Reopen" : "Close books"}</button
                    >
                    <button type="button" class="btn btn-ghost btn-xs" onclick={() => (confirming = null)}>Cancel</button>
                </div>
            {/if}
            {#if failure !== null}
                <p class="text-sm text-error" role="alert" data-testid="books-failure">Not saved: {failure}</p>
            {/if}
        </section>

        <section class="flex flex-col gap-3">
            <h2 class="text-base font-semibold">Invoice matching</h2>
            <p class="text-sm text-base-content/70">Ledgeline Invoicer matches payments to invoices with these same settings.</p>
            <div class="flex flex-wrap items-center justify-between gap-x-4 gap-y-2">
                <label class="flex flex-wrap items-center gap-3">
                    <span>Look for payments up to</span>
                    <span class="flex items-center gap-3">
                        <input
                            type="number"
                            class="input w-24 input-sm"
                            min={SETTLEMENT_LOOKBACK_RANGE[0]}
                            max={SETTLEMENT_LOOKBACK_RANGE[1]}
                            step="1"
                            value={settings.settlementLookbackDays}
                            onchange={(e) => {
                                settings.settlementLookbackDays = e.currentTarget.valueAsNumber;
                                e.currentTarget.value = String(settings.settlementLookbackDays);
                            }}
                            data-testid="books-lookback"
                        />
                        <span>days after the invoice</span>
                        <span class="text-xs text-base-content/60">{settings.settlementLookbackDays === 0 ? "0 looks back forever" : ""}</span>
                    </span>
                </label>
                {@render matchingPin("settlementLookbackDays", "books-pin-lookback")}
            </div>
            <div class="flex flex-wrap items-center justify-between gap-x-4 gap-y-2">
                <label class="flex flex-wrap items-center gap-3">
                    <span>Paid up to</span>
                    <span class="flex items-center gap-3">
                        <input
                            type="number"
                            class="input w-20 input-sm"
                            min={SETTLEMENT_LEAD_RANGE[0]}
                            max={SETTLEMENT_LEAD_RANGE[1]}
                            step="1"
                            value={settings.settlementLeadDays}
                            onchange={(e) => {
                                settings.settlementLeadDays = e.currentTarget.valueAsNumber;
                                e.currentTarget.value = String(settings.settlementLeadDays);
                            }}
                            data-testid="books-lead"
                        />
                        <span>days before the invoice still counts (a payroll draw, then its bill)</span>
                    </span>
                </label>
                {@render matchingPin("settlementLeadDays", "books-pin-lead")}
            </div>
            <div class="flex flex-wrap items-center justify-between gap-x-4 gap-y-2">
                <label class="flex flex-wrap items-center gap-3">
                    <span>Treat a payment up to</span>
                    <span class="flex items-center gap-3">
                        <input
                            type="number"
                            class="input w-24 input-sm"
                            min={SETTLEMENT_TOLERANCE_RANGE[0] / 100}
                            max={SETTLEMENT_TOLERANCE_RANGE[1] / 100}
                            step="0.1"
                            value={percent(settings.settlementToleranceBps)}
                            onchange={(e) => {
                                settings.settlementToleranceBps = Math.round(e.currentTarget.valueAsNumber * 100);
                                e.currentTarget.value = percent(settings.settlementToleranceBps);
                            }}
                            data-testid="books-tolerance"
                        />
                        <span>% short of its invoice as paid, with the difference as a fee</span>
                    </span>
                </label>
                {@render matchingPin("settlementToleranceBps", "books-pin-tolerance")}
            </div>
        </section>
    {/if}
</section>
