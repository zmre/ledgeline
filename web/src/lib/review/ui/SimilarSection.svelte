<script lang="ts">
    // "Similar unreviewed" (plan 32 D10–D12): the transactions with the same
    // payee key that the card's change could also go to. Collapsed by default
    // with its count in the header; every row starts UNchecked; the header box
    // is tri-state; each row shows the chips it will receive. Only what is
    // checked is saved, and never marked reviewed.
    //
    // The set was fetched once, when the card appeared, keyed on the ORIGINAL
    // description; `rows` is that set filtered by the card's current change
    // (client-side, `similarRows`), so the section follows every edit with no
    // request per keystroke.
    //
    // A staged run (plan 32 D24) shows TWO of these, one per source — "in this
    // import" and "already in the journal" — so the heading and the test ids
    // are props, and each keeps its own checked list.
    import {formatAmount} from "#lib/domain/money.js";
    import {txnFlowAmounts} from "#lib/journal/rowModel.js";
    import {checkState, shortAccount, shortDate, type SimilarRow} from "../card";
    import ReviewIcon from "./ReviewIcon.svelte";

    let {
        rows,
        checked = $bindable([]),
        open = $bindable(false),
        applying,
        disabled = false,
        title = "Similar unreviewed",
        testid = "review-similar",
    }: {
        rows: SimilarRow[];
        /** The checked candidates' tindices. */
        checked?: number[];
        open?: boolean;
        /** "description + account", or "" before anything applicable has changed. */
        applying: string;
        disabled?: boolean;
        /** The section's heading. */
        title?: string;
        /** The prefix of every test id in the section (and its list's DOM id), unique per section on a card. */
        testid?: string;
    } = $props();

    const checkedSet = $derived(new Set(checked));
    const tri = $derived(checkState(rows, checkedSet));
    const canApply = $derived(applying !== "" && !disabled);
    let allBox = $state<HTMLInputElement | null>(null);
    // `indeterminate` is a DOM property with no attribute form, so it is set by hand.
    $effect(() => {
        if (allBox !== null) allBox.indeterminate = tri === "some";
    });

    function toggle(index: number): void {
        checked = checkedSet.has(index) ? checked.filter((candidate) => candidate !== index) : [...checked, index];
    }

    /** ⇧A and the header box: all checked → none, otherwise all. */
    export function toggleAll(): void {
        setAll(tri !== "all");
    }

    /** Check every row, or none (⇧A across both of a staged card's sections). */
    export function setAll(on: boolean): void {
        if (!canApply) return;
        checked = on ? rows.map((row) => row.txn.txn.index) : [];
    }

    /** Every row is checked. */
    export function allChecked(): boolean {
        return tri === "all";
    }
</script>

<section class="rounded-box border border-base-content/10 bg-base-200/40" data-testid={testid} data-open={open ? "" : undefined}>
    <!-- Heading and count on one line; the hint beside them when there is
         room, else on its own line below (below `sm` always), never squeezing
         the heading into two lines underneath it. -->
    <div class="flex flex-wrap items-center gap-x-2 gap-y-0.5 px-3 py-2">
        <button
            type="button"
            class="flex max-w-full min-w-0 grow items-center gap-1.5 text-left text-sm font-medium whitespace-nowrap"
            aria-expanded={open}
            aria-controls="{testid}-list"
            onclick={() => (open = !open)}
        >
            <ReviewIcon name={open ? "chevron-down" : "chevron-right"} class="h-3.5 w-3.5 shrink-0 text-base-content/50" />
            <kbd class="kbd kbd-xs font-normal">s</kbd>
            <!-- Its own box: at a big interface scale it wraps inside it, beside the count, never under the hint. -->
            <span class="min-w-0 leading-tight whitespace-normal">{title}</span>
            <span class="text-base-content/40">·</span>
            <span class="tabular-nums" data-testid="{testid}-count">{rows.length}</span>
        </button>
        {#if applying !== ""}
            <span class="min-w-0 basis-full pl-5 text-xs [overflow-wrap:anywhere] text-base-content/50 sm:basis-auto sm:pl-0"
                >apply: <span class="text-base-content/80">{applying}</span></span
            >
        {:else}
            <span class="min-w-0 basis-full pl-5 text-xs text-base-content/40 sm:basis-auto sm:pl-0">change the description or an account to apply it</span>
        {/if}
    </div>
    <!-- 0fr → 1fr: the section's height animates without measuring anything. -->
    <div class={["grid transition-[grid-template-rows] duration-120 ease-out motion-reduce:transition-none", open ? "grid-rows-[1fr]" : "grid-rows-[0fr]"]}>
        <div class="min-h-0 overflow-hidden" inert={!open}>
            <div class="border-t border-base-content/10 px-3 pt-2 pb-3" id="{testid}-list">
                <label class="flex w-fit items-center gap-2 py-1 text-xs text-base-content/60">
                    <input
                        type="checkbox"
                        class="checkbox checkbox-xs"
                        checked={tri === "all"}
                        bind:this={allBox}
                        onchange={toggleAll}
                        disabled={!canApply}
                        aria-label="Check all: {title}"
                        data-testid="{testid}-all"
                    />
                    all <kbd class="kbd kbd-xs">⇧A</kbd>
                </label>
                <ul class="mt-1 flex max-h-56 flex-col overflow-y-auto" data-testid="{testid}-rows" data-similar-rows>
                    {#each rows as row (row.txn.txn.index)}
                        {@const index = row.txn.txn.index}
                        <li>
                            <label
                                class={[
                                    "grid cursor-pointer grid-cols-[auto_3.5rem_minmax(0,1fr)_auto] items-center gap-x-3 rounded-field px-1 py-1 text-sm hover:bg-base-content/5",
                                    checkedSet.has(index) && "bg-success/5",
                                ]}
                                data-testid="{testid}-row"
                                data-txn={index}
                            >
                                <input
                                    type="checkbox"
                                    class="checkbox checkbox-xs checkbox-success"
                                    checked={checkedSet.has(index)}
                                    onchange={() => toggle(index)}
                                    disabled={!canApply}
                                    aria-label="Apply to {row.txn.txn.date} {row.txn.txn.description}"
                                />
                                <span class="text-xs text-base-content/55 tabular-nums">{shortDate(row.txn.txn.date)}</span>
                                <span class="min-w-0">
                                    <span class="block truncate font-mono text-xs" title={row.txn.txn.description}>{row.txn.txn.description}</span>
                                    {#if row.rename !== null || row.moves.length > 0 || row.inQueue}
                                        <span class="mt-0.5 flex flex-wrap gap-1">
                                            {#if row.rename !== null}
                                                <span class="badge border-success/30 bg-success/10 badge-xs text-success" data-testid="review-similar-chip"
                                                    >→ {row.rename}</span
                                                >
                                            {/if}
                                            {#each row.moves as [, to] (to)}
                                                <span
                                                    class="badge border-success/30 bg-success/10 font-mono badge-xs text-success"
                                                    data-testid="review-similar-chip">→ {shortAccount(to)}</span
                                                >
                                            {/each}
                                            {#if row.inQueue}<span class="badge badge-ghost badge-xs">in this run</span>{/if}
                                        </span>
                                    {/if}
                                </span>
                                <span class="text-right font-mono text-xs text-base-content/70 tabular-nums">
                                    {txnFlowAmounts(row.txn.txn)
                                        .map((amount) => formatAmount(amount))
                                        .join(" ")}
                                </span>
                            </label>
                        </li>
                    {/each}
                </ul>
            </div>
        </div>
    </div>
</section>
