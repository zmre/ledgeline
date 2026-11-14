<script lang="ts">
    // The card's header details (plan 32 D33): the date, the status, the code
    // and the secondary date, shown in the hero where its date line was.
    //
    // - `summary`: one muted line ("Oct 12, 2026 · cleared · #1042") that is a
    //   button: a click (or `D` on the card) opens the editor. Queue and staged
    //   cards start here, so they stay as quiet as before.
    // - `editor`: the fields in a row, each with the card-mode key that reaches
    //   it — `D` the date, `!` the status pill (it cycles unmarked → pending →
    //   cleared, as the Journal's status cell does), `#` the code — then the
    //   secondary date when there is one, or a quiet "+ secondary date".
    //
    // The secondary date is optional, so it is a `DateInput`: WebKit paints
    // TODAY into an empty `<input type="date">`.
    import DateInput from "#lib/components/DateInput.svelte";
    import {hint} from "#lib/keys/hint.svelte.js";
    import {nextStatus} from "#lib/journal/rowModel.js";
    import {detailsSummary} from "../card";
    import type {ReviewForm} from "../types";
    import ReviewIcon from "./ReviewIcon.svelte";

    let {
        form = $bindable(),
        mode,
        showDate2,
        canCollapse,
        disabled = false,
        onExpand,
        onCollapse,
        onAddDate2,
        onRemoveDate2,
    }: {
        form: ReviewForm;
        mode: "summary" | "editor";
        /** Whether the secondary date's field is up: it is set, or "+ secondary date" asked for it. */
        showDate2: boolean;
        /** Whether the editor may fold back to the summary (not while one of its fields is changed). */
        canCollapse: boolean;
        disabled?: boolean;
        onExpand: () => void;
        onCollapse: () => void;
        onAddDate2: () => void;
        onRemoveDate2: () => void;
    } = $props();

    const parts = $derived(detailsSummary(form));
    const STATUS = {
        unmarked: {text: "unmarked", tone: "border-dashed border-base-content/25 text-base-content/50"},
        pending: {text: "! pending", tone: "border-warning/50 bg-warning/10 text-warning"},
        cleared: {text: "* cleared", tone: "border-success/50 bg-success/10 text-success"},
    } as const;
</script>

{#snippet key(text: string)}
    <kbd class="kbd w-5 justify-center kbd-xs font-normal text-base-content/50">{text}</kbd>
{/snippet}

{#if mode === "summary"}
    <button
        type="button"
        class="group -mx-1 flex items-center gap-x-2 rounded px-1 tabular-nums hover:bg-base-content/5 hover:text-base-content/80"
        title={hint("Edit the date, status and code", "D")}
        aria-label="Details: {parts.join(', ')}. Edit the date, status and code"
        aria-expanded="false"
        data-testid="review-details-summary"
        onclick={onExpand}
        {disabled}
    >
        {#each parts as part, at (at)}
            {#if at > 0}<span aria-hidden="true" class="text-base-content/25">·</span>{/if}
            <span class={part === "pending" ? "text-warning" : ""}>{part}</span>
        {/each}
        <ReviewIcon name="chevron-down" class="h-3.5 w-3.5 text-base-content/30 group-hover:text-base-content/60" />
    </button>
{:else}
    <div class="flex flex-wrap items-center gap-x-4 gap-y-2" data-testid="review-details" role="group" aria-label="Details">
        <div class="flex items-center gap-1.5" data-review-field="date">
            {@render key("D")}
            <DateInput class="w-36" bind:value={form.date} aria-label="Date" {disabled} />
        </div>
        <div class="flex items-center gap-1.5" data-review-field="status">
            {@render key("!")}
            <button
                type="button"
                class="rounded-full border px-2.5 py-0.5 font-mono text-xs transition-colors {STATUS[form.status].tone}"
                aria-label="Status: {form.status}. Click to cycle"
                title="Click (or !) to cycle unmarked → pending → cleared"
                data-testid="review-status"
                data-status={form.status}
                onclick={() => (form.status = nextStatus(form.status))}
                {disabled}
            >
                {STATUS[form.status].text}
            </button>
        </div>
        <div class="flex items-center gap-1.5" data-review-field="code">
            {@render key("#")}
            <input
                type="text"
                class="input w-24 font-mono input-sm"
                bind:value={form.code}
                placeholder="code"
                aria-label="Code"
                autocomplete="off"
                {disabled}
            />
        </div>
        {#if showDate2}
            <div class="flex items-center gap-1" data-review-field="date2">
                <span class="mr-0.5 text-xs text-base-content/50">secondary</span>
                <DateInput class="w-36" bind:value={form.date2} aria-label="Secondary date" {disabled} />
                <button
                    type="button"
                    class="btn btn-square btn-ghost btn-xs"
                    aria-label="Remove the secondary date"
                    title="Remove the secondary date"
                    onclick={onRemoveDate2}
                    {disabled}
                >
                    <ReviewIcon name="x" class="h-3.5 w-3.5" />
                </button>
            </div>
        {:else}
            <button type="button" class="btn gap-1 btn-ghost font-normal text-base-content/55 btn-xs" onclick={onAddDate2} {disabled}>
                <span class="text-sm leading-none">+</span> secondary date
            </button>
        {/if}
        {#if canCollapse}
            <button
                type="button"
                class="btn ml-auto btn-square btn-ghost text-base-content/40 btn-xs"
                aria-label="Hide the details"
                title="Hide the details"
                aria-expanded="true"
                onclick={onCollapse}
            >
                <ReviewIcon name="chevron-down" class="h-3.5 w-3.5 rotate-180" />
            </button>
        {/if}
    </div>
{/if}
