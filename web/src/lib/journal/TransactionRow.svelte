<script lang="ts">
    // One transaction (WP-03): a fixed-height table row on desktop, a compact
    // card at narrow widths. Renders only the columns enabled in settings.
    // WP-08: `flags` (background-check problems for this txn) render as a
    // severity dot with a tooltip; `pulse` briefly highlights the row after
    // problems-drawer navigation.
    //
    // Editing (native engine only): clicking the description edits it inline
    // (PATCH {description}); a hover pencil opens the whole-transaction popup
    // (edit-all → PUT). Account/category inline editing lives in AccountsCell.
    //
    // Transfers (plan 35 D11): a collapsed linked pair (`transfer`) shows its
    // anchoring half's own description followed by ⇄, "→ Jan 2" when the
    // other half is dated differently, and the two real accounts in the
    // ordinary account cell (read-only); ▸ (in the gutter) expands it — the
    // table owns that state — and a hover button unlinks it. Its two halves
    // render under it as ordinary, tinted rows (`nested`). A row whose postings carry a `nomatch` rejection (D12)
    // shows a small ≠ that offers Forget rejection. A row with an unlinked
    // clearing posting offers Find match (Amendment A2) among its hover actions.
    import {descriptionPatch} from "#lib/api/editMapping.js";
    import {maxSeverity, type Problem} from "#lib/checks/engine.js";
    import type {Transaction} from "#lib/domain/types.js";
    import type {RunningCell, TransferRow} from "#lib/journalView/types.js";
    import {findableClearing, rejectionsOf} from "#lib/matching/model.js";
    import {editing} from "#lib/stores/editing.svelte.js";
    import type {ColumnConfig} from "#lib/stores/settings.svelte.js";
    import {rowShadow, shortDate, stripeColor, type RowMarker} from "#lib/review/card.js";
    import ReviewIcon from "#lib/review/ui/ReviewIcon.svelte";
    import AccountsCell from "./AccountsCell.svelte";
    import AmountCell from "./AmountCell.svelte";
    import CommentIndicator from "./CommentIndicator.svelte";
    import RunningCells from "./RunningCells.svelte";
    import StatusCell from "./StatusCell.svelte";
    import {txnModal} from "./edit/modalState.svelte";
    import {rowActions} from "./rowAction.svelte";
    import {txnFlowAmounts} from "./rowModel";

    let {
        txn,
        columns,
        mode = "row",
        flags = [],
        pulse = false,
        cursor = false,
        marker = null,
        pinned = null,
        running = null,
        at,
        transfer,
        expanded = false,
        nested = false,
        linked = false,
        onToggle,
        onUnlink,
        onForget,
        onFindMatch,
    }: {
        txn: Transaction;
        columns: ColumnConfig;
        mode?: "row" | "card";
        flags?: Problem[];
        pulse?: boolean;
        cursor?: boolean;
        /** The review stripe (plan 32 D20): unknown, unreviewed-in-window, or none. */
        marker?: RowMarker;
        /** The pinned running balance's account (plan 36): adds the Change and Balance columns. */
        pinned?: string | null;
        /** This row's running-balance cell, while `pinned` is set. */
        running?: RunningCell | null;
        /** The table's display slot, so a click can put the cursor on THIS row (a half shares its index with its transfer row). */
        at?: number;
        /** Plan 35 D11: this row is a collapsed linked pair, `txn` its anchoring half. */
        transfer?: TransferRow;
        /** The transfer row is expanded: its halves show under it. */
        expanded?: boolean;
        /** One half of an expanded transfer row, shown under it. */
        nested?: boolean;
        /** The ⇄ icon: the transaction is in a link but is not collapsed (a half, or a transaction in two links). */
        linked?: boolean;
        onToggle?: () => void;
        onUnlink?: () => void;
        /** Forget rejection (D12): the `nomatch` value to remove. */
        onForget?: (value: string) => void;
        /** Find match (A2): open the matcher on this transaction. Offered when it has an unlinked clearing posting. */
        onFindMatch?: () => void;
    } = $props();

    /** The other half of a transfer row, for "→ Jan 2". */
    const partner = $derived(transfer?.halves.find((half) => half.txn.index !== txn.index)?.txn ?? null);
    /** The transfer's two real accounts, source first, shown like any row's accounts. */
    const ends = $derived.by((): readonly [string, string] | null => {
        const from = transfer?.from.account;
        const to = transfer?.to.account;
        return from === undefined || from === null || to === undefined || to === null ? null : [from, to];
    });
    const rejections = $derived(transfer === undefined ? rejectionsOf(txn) : []);
    const findable = $derived(transfer === undefined && onFindMatch !== undefined && findableClearing(txn).length > 0);

    const amounts = $derived(txnFlowAmounts(txn));
    const flagTip = $derived(flags.map((problem) => problem.message).join("; "));
    const dotClass = $derived(maxSeverity(flags) === "error" ? "bg-error" : maxSeverity(flags) === "warning" ? "bg-warning" : "bg-info");
    const pulseClass = "bg-primary/15 animate-pulse";
    // Distinct from BOTH hover (bg-base-200/60) and pulse (bg-primary/15 +
    // animate-pulse): a stronger tint plus an inset left rule that hover cannot
    // repaint. Inset box-shadow on the first <td> rather than a border on the
    // <tr>, because daisyUI's `.table` is border-collapse:separate (where row
    // borders are not painted) and a border-l would shift the content 4px.
    // The rule itself is part of `rowShadow`, stacked inside the review stripe
    // (plan 32 D20) so the two coexist.
    const cursorClass = "bg-primary/25";
    const shadow = $derived(rowShadow(marker, cursor));
    const cursorCardClass = "ring-primary ring-2 ring-inset";
    // Mutually exclusive, and in this order. `hover:` carries a pseudo-class, so
    // it beats a plain background utility regardless of source order — hovering
    // the cursored row would otherwise make it look un-cursored.
    const rowClass = $derived(pulse ? pulseClass : cursor ? cursorClass : nested ? "bg-base-200/40 hover:bg-base-200/60" : "hover:bg-base-200/60");
    const cardClass = $derived(pulse ? pulseClass : cursor ? cursorCardClass : "");
    const canEdit = $derived(editing.canEdit);

    let editingDesc = $state(false);
    let draft = $state("");

    // Answer `e` from the keyboard. The table cannot reach this state directly:
    // the list is virtualized, so this row may not have been mounted when the key
    // was pressed. See rowAction.svelte.ts.
    $effect(() => {
        const request = rowActions.request;
        if (request === null || request.txnIndex !== txn.index || request.action !== "description") return;
        rowActions.consume(request.nonce);
        startEditDesc();
    });

    function startEditDesc(): void {
        if (!canEdit) return;
        draft = txn.description;
        editingDesc = true;
    }

    async function commitDesc(): Promise<void> {
        if (!editingDesc) return;
        const next = draft.trim();
        editingDesc = false;
        if (next === txn.description) return;
        const result = await editing.patch(txn.index, descriptionPatch(next));
        if (!result.ok && result.failure.kind !== "conflict") editing.reportFailure(result.failure);
    }

    function onDescKeydown(event: KeyboardEvent): void {
        if (event.key === "Enter") {
            event.preventDefault();
            void commitDesc();
        } else if (event.key === "Escape") {
            event.preventDefault();
            editingDesc = false;
        }
    }

    function descAutofocus(node: HTMLInputElement): void {
        node.focus();
        node.select();
    }
</script>

{#snippet descriptionContent()}
    <!-- Every row's description text starts at the same x (plan 35 D11): the
         transfer toggle hangs in the gutter to its left (row mode), the ⇄ icon
         follows the text, and expanded halves are told apart by their tint,
         not an indent. -->
    <div class="group/desc relative flex min-w-0 items-center gap-1.5">
        {#if transfer !== undefined}
            <button
                type="button"
                class={["btn btn-square shrink-0 btn-ghost btn-xs", mode === "row" && "absolute top-1/2 right-full mr-1 -translate-y-1/2"]}
                aria-expanded={expanded}
                aria-label={expanded ? "Collapse the transfer" : "Expand the transfer"}
                title={expanded ? "Collapse (l)" : "Show both halves (l)"}
                onclick={() => onToggle?.()}
                data-testid="transfer-toggle"
            >
                <ReviewIcon name={expanded ? "chevron-down" : "chevron-right"} class="h-3.5 w-3.5" />
            </button>
        {/if}
        {#if flags.length > 0}
            <span class="tooltip tooltip-right shrink-0" data-tip={flagTip}>
                <span class="block h-2 w-2 rounded-full {dotClass}" role="img" aria-label={flagTip}></span>
            </span>
        {/if}
        {#if editingDesc}
            <input
                type="text"
                class="input w-full min-w-0 input-xs"
                bind:value={draft}
                aria-label="Edit description"
                autocomplete="off"
                onkeydown={onDescKeydown}
                onblur={commitDesc}
                use:descAutofocus
            />
        {:else if canEdit}
            <button type="button" class="min-w-0 truncate text-left" title="Click to edit · {txn.description}" onclick={startEditDesc}>
                {txn.description || "(no description)"}
            </button>
            {@render linkIcon()}
            {@render partnerDate()}
            <CommentIndicator {txn} />
            {@render rejectionChips()}
            <button
                type="button"
                class="btn btn-square shrink-0 btn-ghost opacity-0 btn-xs group-hover/desc:opacity-100 focus:opacity-100"
                title="Edit whole transaction"
                aria-label="Edit whole transaction"
                onclick={() => txnModal.openEdit(txn.index)}
            >
                ✎
            </button>
            {#if findable}
                <button
                    type="button"
                    class="btn btn-square shrink-0 btn-ghost opacity-0 btn-xs group-hover/desc:opacity-100 focus:opacity-100"
                    title="Find a match for this transfer half (⇧M)"
                    aria-label="Find a match"
                    onclick={() => onFindMatch?.()}
                    data-testid="row-find-match"
                >
                    <ReviewIcon name="link" class="h-3.5 w-3.5" />
                </button>
            {/if}
            {#if transfer !== undefined}
                <button
                    type="button"
                    class="btn btn-square shrink-0 btn-ghost opacity-0 btn-xs group-hover/desc:opacity-100 focus:opacity-100"
                    title="Unlink the transfer"
                    aria-label="Unlink the transfer"
                    onclick={() => onUnlink?.()}
                    data-testid="transfer-unlink"
                >
                    <ReviewIcon name="unlink" class="h-3.5 w-3.5" />
                </button>
            {/if}
        {:else}
            <span class="truncate" title={txn.description}>{txn.description}</span>
            {@render linkIcon()}
            {@render partnerDate()}
            <CommentIndicator {txn} />
        {/if}
    </div>
{/snippet}

{#snippet linkIcon()}
    {#if transfer !== undefined || linked}
        <span class="shrink-0 text-info" title={transfer !== undefined ? "Linked transfer" : "Half of a linked transfer"} data-testid="transfer-icon">
            <ReviewIcon name="link" class="h-3.5 w-3.5" />
        </span>
    {/if}
{/snippet}

{#snippet partnerDate()}
    {#if partner !== null && partner.date !== txn.date}
        <span class="shrink-0 text-xs whitespace-nowrap text-base-content/55" title="The other half is dated {partner.date}" data-testid="transfer-partner-date"
            >→ {shortDate(partner.date)}</span
        >
    {/if}
{/snippet}

{#snippet rejectionChips()}
    <!-- D12: a rejected pair, remembered in the file. The least intrusive undo: a small ≠ that forgets it (the toolbar confirms). -->
    {#each rejections as value (value)}
        <button
            type="button"
            class="badge shrink-0 cursor-pointer badge-ghost badge-xs text-base-content/60 hover:text-base-content"
            title="Rejected as a transfer match (nomatch:{value}) — click to forget the rejection"
            aria-label="Forget rejection {value}"
            onclick={() => onForget?.(value)}
            data-testid="forget-rejection"
        >
            ≠
        </button>
    {/each}
{/snippet}

{#snippet accountsContent()}
    {#if transfer !== undefined && ends !== null}
        <div class="min-w-0" data-testid="transfer-accounts">
            <AccountsCell {txn} transferEnds={ends} />
        </div>
    {:else}
        <AccountsCell {txn} />
    {/if}
{/snippet}

{#if mode === "row"}
    <!-- `aria-current` rather than `aria-selected`: it is a GLOBAL aria
         attribute, so no role constraints apply, and it already has house
         precedent in RulesFileList. `aria-selected` would only be meaningful
         inside a role="grid", which would drag required row/gridcell roles
         across a <tbody> that also contains aria-hidden spacer rows. -->
    <tr
        class="h-10 {rowClass} [&>td:first-child]:[box-shadow:var(--row-shadow)]"
        style:--row-shadow={shadow}
        aria-current={cursor ? "true" : undefined}
        data-txn={txn.index}
        data-slot={at}
        data-transfer={transfer?.matchId}
        data-review-marker={marker ?? undefined}
    >
        {#if columns.date}
            <td class="py-1 font-mono text-xs whitespace-nowrap text-base-content/80">{txn.date}</td>
        {/if}
        {#if columns.status}
            <td class="py-1"><StatusCell {txn} /></td>
        {/if}
        {#if columns.description}
            <td class="max-w-0 py-1">{@render descriptionContent()}</td>
        {/if}
        {#if columns.accounts}
            <td class="max-w-0 py-1">{@render accountsContent()}</td>
        {/if}
        {#if columns.amount}
            <td class="py-1"><AmountCell {amounts} /></td>
        {/if}
        {#if pinned !== null}
            <RunningCells cell={running} {pinned} txnIndex={txn.index} txnDate={txn.date} />
        {/if}
    </tr>
{:else}
    <!-- A ring, not a background: the card already has `bg-base-200`. -->
    <article
        class={["card relative mb-2 h-24 overflow-hidden bg-base-200", nested && "ml-4", cardClass]}
        aria-current={cursor ? "true" : undefined}
        data-txn={txn.index}
        data-slot={at}
        data-transfer={transfer?.matchId}
        data-review-marker={marker ?? undefined}
    >
        {#if marker !== null}
            <span class="absolute inset-y-0 left-0 w-[3px]" style:background={stripeColor(marker)} aria-hidden="true"></span>
        {/if}
        <div class="card-body gap-1 p-3">
            <div class="flex items-center justify-between gap-2">
                <div class="flex min-w-0 items-center gap-1.5">
                    {#if columns.status}<StatusCell {txn} />{/if}
                    {@render descriptionContent()}
                </div>
                {#if columns.amount}<AmountCell {amounts} />{/if}
            </div>
            <div class="flex items-center justify-between gap-2">
                {#if columns.accounts}
                    <div class="min-w-0 overflow-hidden">{@render accountsContent()}</div>
                {/if}
                {#if pinned !== null}
                    <RunningCells cell={running} {pinned} txnIndex={txn.index} txnDate={txn.date} mode="card" />
                {/if}
                {#if columns.date}
                    <span class="shrink-0 font-mono text-xs whitespace-nowrap text-base-content/60">{txn.date}</span>
                {/if}
            </div>
        </div>
    </article>
{/if}
