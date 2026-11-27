<!-- What to do with the picked lines (T-entries "Bulk"): mark them billable
     again, mark them not billable (with an optional reason), move them to
     another account, or set who did them. One action at a time: choosing one
     opens its field in the bar, Enter applies it, Escape goes back.

     `onBill` is Phase 5's hook (Bill time → invoice draft); without it there
     is no Bill button. -->
<script lang="ts">
    import AccountInput from "#ui/components/AccountInput.svelte";
    import type {BulkOp} from "../api/entries.js";
    import {formatDuration} from "../duration.js";

    let {
        count,
        seconds,
        accounts,
        busy = false,
        error = null,
        onApply,
        onClear,
        onBill,
    }: {
        count: number;
        seconds: number;
        accounts: string[];
        busy?: boolean;
        error?: string | null;
        onApply: (op: BulkOp) => void;
        onClear: () => void;
        onBill?: () => void;
    } = $props();

    type Step = "nobill" | "move" | "person";
    let step = $state<Step | null>(null);
    let reason = $state("");
    let account = $state("");
    let person = $state("");

    function apply(event: SubmitEvent): void {
        event.preventDefault();
        if (step === "nobill") onApply({op: "nobill", reason: reason.trim()});
        else if (step === "move" && account.trim() !== "") onApply({op: "move", account: account.trim()});
        else if (step === "person") onApply({op: "person", person: person.trim()});
    }

    function back(event: KeyboardEvent): void {
        if (event.key !== "Escape" || event.defaultPrevented) return;
        event.preventDefault();
        step = null;
    }

    const LABELS: Record<Step, string> = {nobill: "Mark not billable", move: "Move", person: "Set person"};
</script>

<section class="flex flex-col gap-2 rounded-box border border-primary/40 bg-primary/5 p-3" aria-label="Picked entries" data-testid="bulk-bar">
    <div class="flex flex-wrap items-center gap-2">
        <span class="text-sm font-medium" role="status" data-testid="bulk-count">{count} picked · {formatDuration(seconds)}</span>
        <div class="ml-auto flex flex-wrap gap-2">
            <button type="button" class="btn btn-outline btn-xs" disabled={busy} onclick={() => onApply({op: "billable"})} data-testid="bulk-billable"
                >Mark billable</button
            >
            <button type="button" class="btn btn-outline btn-xs" disabled={busy} onclick={() => (step = "nobill")} data-testid="bulk-nobill"
                >Mark not billable…</button
            >
            <button type="button" class="btn btn-outline btn-xs" disabled={busy} onclick={() => (step = "move")} data-testid="bulk-move"
                >Move to account…</button
            >
            <button type="button" class="btn btn-outline btn-xs" disabled={busy} onclick={() => (step = "person")} data-testid="bulk-person">Set person…</button
            >
            {#if onBill !== undefined}
                <button type="button" class="btn btn-primary btn-xs" disabled={busy} onclick={onBill} data-testid="bulk-bill">Bill…</button>
            {/if}
            <button type="button" class="btn btn-ghost btn-xs" disabled={busy} onclick={onClear} data-testid="bulk-clear">Clear</button>
        </div>
    </div>
    {#if step !== null}
        <!-- Escape works from any field in it; the form is not a control of its own. -->
        <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
        <form class="flex flex-wrap items-end gap-2" onsubmit={apply} onkeydown={back} aria-label={LABELS[step]} data-testid="bulk-form">
            {#if step === "nobill"}
                <label class="flex min-w-0 grow basis-56 flex-col gap-1 text-sm"
                    >Reason (optional)
                    <!-- svelte-ignore a11y_autofocus -->
                    <input class="input w-full input-sm" bind:value={reason} autofocus autocomplete="off" data-testid="bulk-reason" />
                </label>
            {:else if step === "move"}
                <div class="flex min-w-0 grow basis-56 flex-col gap-1 text-sm" data-testid="bulk-account">
                    <span aria-hidden="true">To account</span>
                    <AccountInput bind:value={account} accountNames={accounts} label="To account" autofocus />
                </div>
            {:else}
                <label class="flex min-w-0 grow basis-40 flex-col gap-1 text-sm"
                    >Person (empty: no one)
                    <!-- svelte-ignore a11y_autofocus -->
                    <input class="input w-full input-sm" bind:value={person} autofocus autocomplete="off" data-testid="bulk-person-name" />
                </label>
            {/if}
            <button type="button" class="btn btn-ghost btn-sm" onclick={() => (step = null)}>Back</button>
            <button type="submit" class="btn btn-primary btn-sm" disabled={busy || (step === "move" && account.trim() === "")} data-testid="bulk-apply">
                {#if busy}<span class="loading loading-xs loading-spinner"></span>{/if}
                {LABELS[step]}
            </button>
        </form>
    {/if}
    {#if error !== null}
        <div class="alert py-2 text-sm alert-error" role="alert" data-testid="bulk-error">{error}</div>
    {/if}
</section>
