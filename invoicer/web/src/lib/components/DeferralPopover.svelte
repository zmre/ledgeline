<!-- The books cell's deferral (ARCHITECTURE §3.3): book the row to a deferred
     account and recognize it into another over N months, from a date,
     prorated by day or by calendar month. It writes the cell grammar
     `deferred → recognize-into, N months [from YYYY-MM-DD] [calendar]` and
     reads it back; text it can't show (a `through` term) is left to the
     cell's text field. The deferred account starts as the row's account:
     which deferred account a business uses is its own choice. -->
<script lang="ts">
    import AccountInput from "#ui/components/AccountInput.svelte";
    import DateInput from "#ui/components/DateInput.svelte";
    import {formatBooksCell, parseBooksCell} from "#lib/editor.js";

    let {
        cell,
        accounts,
        label,
        onApply,
    }: {
        /** The books cell as written. */
        cell: string;
        accounts: string[];
        /** What the trigger button says to a screen reader ("Row 2 deferral"). */
        label: string;
        onApply: (text: string) => void;
    } = $props();

    let open = $state(false);
    let account = $state("");
    let target = $state("");
    let months = $state(12);
    let from = $state("");
    let calendar = $state(false);
    let readable = $state(true);

    const current = $derived(parseBooksCell(cell));
    const deferred = $derived(current?.deferral != null);

    function show(): void {
        const parsed = parseBooksCell(cell);
        readable = parsed !== null;
        account = parsed?.account ?? "";
        target = parsed?.deferral?.target ?? "";
        months = parsed?.deferral?.months ?? 12;
        from = parsed?.deferral?.from ?? "";
        calendar = parsed?.deferral?.calendar ?? false;
        open = true;
    }

    const valid = $derived(account.trim() !== "" && target.trim() !== "" && Number.isInteger(months) && months > 0);

    function apply(): void {
        if (!valid) return;
        onApply(formatBooksCell({account, deferral: {target, months, from: from === "" ? null : from, calendar}}));
        open = false;
    }

    function remove(): void {
        onApply(formatBooksCell({account: target.trim() !== "" ? target : account, deferral: null}));
        open = false;
    }

    function onKeydown(event: KeyboardEvent): void {
        if (event.key === "Escape" && !event.defaultPrevented) {
            event.preventDefault();
            open = false;
        }
    }
</script>

<button
    type="button"
    class="btn btn-square btn-ghost btn-xs {deferred ? 'text-primary' : ''}"
    aria-label={label}
    title={deferred ? "Deferred: edit the recognition" : "Defer this row's revenue"}
    onclick={show}
    data-testid="deferral-open">⏲</button
>

{#if open}
    <div class="modal modal-open" role="presentation" onkeydown={onKeydown}>
        <div class="modal-box" role="dialog" aria-modal="true" aria-label="Deferral" data-testid="deferral-dialog">
            <h2 class="text-lg font-semibold">Recognize over time</h2>
            <p class="mt-1 text-sm text-base-content/70">Book the row to a deferred account when it is sent, and recognize it month by month into another.</p>
            {#if !readable}
                <p class="mt-2 alert text-sm alert-warning">This cell's deferral isn't one this form can show ({cell}); saving here replaces it.</p>
            {/if}
            <div class="mt-3 grid gap-3">
                <label class="flex flex-col gap-1 text-sm"
                    >Deferred account
                    <AccountInput bind:value={account} accountNames={accounts} label="Deferred account" placeholder="liabilities:deferred-revenue" />
                </label>
                <label class="flex flex-col gap-1 text-sm"
                    >Recognize into
                    <AccountInput bind:value={target} accountNames={accounts} label="Recognize into" placeholder="revenue:…" />
                </label>
                <div class="flex flex-wrap items-end gap-3">
                    <label class="flex flex-col gap-1 text-sm"
                        >Months
                        <input
                            class="input w-24 input-sm"
                            type="number"
                            min="1"
                            step="1"
                            bind:value={months}
                            aria-label="Months"
                            data-testid="deferral-months"
                        />
                    </label>
                    <label class="flex flex-col gap-1 text-sm"
                        >From (optional)
                        <DateInput bind:value={from} aria-label="Service start" />
                    </label>
                    <label class="flex items-center gap-2 text-sm">
                        <input type="checkbox" class="checkbox checkbox-sm" bind:checked={calendar} />
                        Calendar months
                    </label>
                </div>
            </div>
            <div class="modal-action">
                {#if deferred}
                    <button type="button" class="btn mr-auto btn-ghost btn-sm" onclick={remove}>Don't defer</button>
                {/if}
                <button type="button" class="btn btn-ghost btn-sm" onclick={() => (open = false)}>Cancel</button>
                <button type="button" class="btn btn-primary btn-sm" onclick={apply} disabled={!valid} data-testid="deferral-apply">Apply</button>
            </div>
        </div>
    </div>
{/if}
