<!-- The Budgeted cell of a table row backed by SEVERAL goals (plan 41 WP-B).

     A row's figure is the sum of every goal behind it — a monthly and a yearly
     goal on one account, or a goal on the account plus one on a subaccount that
     rolled up into it at this depth (`rowGoals.ts` explains the roll-up). One
     edit button cannot honestly stand for that sum, so the amount opens this
     small list instead: each goal with its period, its own amount and its own
     Edit / Remove. A locked goal stays in the list, read-only with the engine's
     reason, so the list always adds up to the figure on the row.

     Remove is the page's inline two-step (`removing` is the page's state, as it
     is for the editor below the table), so arming a removal here and in the
     editor are the same act.

     # Why portalled and `position: fixed`

     For the reason `RowMenu.svelte` gives at length: a dropdown positioned
     inside its row is only as visible as the row's nearest clipping ancestor
     allows. The arithmetic is `anchoredPopup.ts`'s, which is unit-tested; the
     dismissal (Escape, focus restore) is `dismissible`'s. Outside-click and
     scroll are handled here, counting the trigger as inside, as RowMenu does. -->
<script lang="ts">
    import type {Snippet} from "svelte";
    import {goalAmountText, goalKey, lockReason, periodLabel, type GoalRef} from "#lib/budget/rowGoals.js";
    import type {BudgetGoal, BudgetListing, BudgetRule} from "#lib/budget/types.js";
    import type {AmountStyle} from "#lib/domain/types.js";
    import {dismissible} from "#lib/keys/dismissible.js";
    import {remToPx} from "#lib/theme/scale.js";
    import {menuPosition, portal, liveViewport} from "#lib/ui/anchoredPopup.js";

    let {
        account,
        amountText,
        refs,
        listing,
        styles,
        busy,
        removing,
        onEdit,
        onRemove,
        onRemoveCancel,
        onRemoveConfirm,
        pencil,
    }: {
        /** The row's account — the popover's title and the trigger's accessible name. */
        account: string;
        /** The row's Budgeted figure, shown on the trigger. */
        amountText: string;
        refs: readonly GoalRef[];
        listing: BudgetListing;
        styles: ReadonlyMap<string, AmountStyle>;
        busy: boolean;
        removing: {journalId: string; goal: BudgetGoal} | null;
        onEdit: (journalId: string, rule: BudgetRule, goal: BudgetGoal) => void;
        onRemove: (journalId: string, goal: BudgetGoal) => void;
        onRemoveCancel: () => void;
        onRemoveConfirm: (journalId: string, goal: BudgetGoal) => void;
        /** The table's edit cue, drawn inside the trigger before the amount, so the cue and the figure are one button. */
        pencil?: Snippet;
    } = $props();

    /** 20rem: wide enough for a period, an amount and two buttons on one line at any scale. */
    const WIDTH_REM = 20;
    /** Rough row height and padding, used ONLY to decide whether to open upward. */
    const ROW_HEIGHT_REM = 2.75;
    const PADDING_REM = 3;

    let open = $state(false);
    let trigger = $state<HTMLButtonElement | null>(null);
    let panel = $state<HTMLDivElement | null>(null);
    let placement = $state({top: 0, left: 0, width: remToPx(WIDTH_REM), maxHeight: 0, below: true});

    function reposition(): void {
        if (trigger === null) return;
        placement = menuPosition(trigger.getBoundingClientRect(), liveViewport(), {
            width: remToPx(WIDTH_REM),
            height: remToPx(refs.length * ROW_HEIGHT_REM + PADDING_REM),
        });
    }

    function close(): void {
        open = false;
    }

    function toggle(): void {
        if (open) {
            close();
            return;
        }
        // Focused before the panel mounts so `dismissible` hands focus back to
        // it on close; macOS WebKit does not focus a clicked button by itself.
        trigger?.focus();
        reposition();
        open = true;
    }

    function armed(ref: GoalRef): boolean {
        return removing !== null && removing.journalId === ref.file.journalId && removing.goal.index === ref.goal.index;
    }

    function edit(ref: GoalRef): void {
        close();
        onEdit(ref.file.journalId, ref.rule, ref.goal);
    }

    $effect(() => {
        if (!open) return;
        const onPointerDown = (event: PointerEvent): void => {
            const target = event.target as Node | null;
            if (target === null) return;
            if (trigger?.contains(target) === true || panel?.contains(target) === true) return;
            close();
        };
        document.addEventListener("pointerdown", onPointerDown, true);
        document.addEventListener("scroll", close, true);
        return () => {
            document.removeEventListener("pointerdown", onPointerDown, true);
            document.removeEventListener("scroll", close, true);
        };
    });
</script>

<button
    bind:this={trigger}
    type="button"
    class="inline-flex cursor-pointer items-center gap-1.5 font-mono text-primary tabular-nums hover:underline"
    aria-haspopup="dialog"
    aria-expanded={open}
    aria-label="Edit budget for {account} ({refs.length} goals)"
    title="{refs.length} goals add up to this — click to see them"
    data-testid="budgeted-goals"
    onclick={toggle}
>
    {@render pencil?.()}<span>{amountText}<span class="ml-0.5 text-xs" aria-hidden="true">▾</span></span>
</button>

{#if open}
    <!-- `bg-base-200` and a border on a `bg-base-100` page, as RowMenu does;
         `z-[1001]` clears daisyUI's `.modal` (999). -->
    <div
        use:portal
        use:dismissible={{onDismiss: close, trap: true}}
        bind:this={panel}
        role="dialog"
        aria-label="Budget goals for {account}"
        data-testid="budgeted-goals-popover"
        class="fixed z-[1001] flex flex-col gap-1 overflow-y-auto rounded-box border border-base-300 bg-base-200 p-3 text-sm shadow-lg"
        style="top: {placement.top}px; left: {placement.left}px; width: {placement.width}px; max-height: {placement.maxHeight}px"
    >
        <div class="truncate pb-1 text-xs font-semibold text-base-content/70" title={account}>{account}</div>
        <ul class="flex flex-col">
            {#each refs as ref (goalKey(ref.file.journalId, ref.goal))}
                <li class="flex flex-wrap items-center gap-x-2 gap-y-1 border-b border-base-content/5 py-1.5 last:border-b-0">
                    <span class="min-w-0 grow">
                        <span class="block">{periodLabel(ref.rule)}</span>
                        <!-- The goal's own account when it rolled up from deeper:
                             otherwise two lines reading "Monthly" would not say
                             which subaccount each one budgets. -->
                        {#if ref.goal.account !== account}
                            <span class="block truncate text-xs text-base-content/50" title={ref.goal.account}>{ref.goal.account}</span>
                        {/if}
                    </span>
                    <span class="font-mono tabular-nums">{goalAmountText(ref.goal, styles)}</span>
                    {#if !ref.editable}
                        <span class="text-xs text-base-content/50" title={lockReason(listing, ref)} data-testid="goal-locked">🔒 read-only</span>
                    {:else if armed(ref)}
                        <span class="flex items-center gap-1">
                            <span class="text-xs">Remove it?</span>
                            <button
                                type="button"
                                class="btn btn-error btn-xs"
                                disabled={busy}
                                onclick={() => onRemoveConfirm(ref.file.journalId, ref.goal)}
                                data-testid="confirm-remove"
                            >
                                Confirm
                            </button>
                            <button type="button" class="btn btn-ghost btn-xs" disabled={busy} onclick={onRemoveCancel}>Keep</button>
                        </span>
                    {:else}
                        <span class="flex gap-1">
                            <button type="button" class="btn btn-ghost btn-xs" disabled={busy} onclick={() => edit(ref)}>Edit</button>
                            <button
                                type="button"
                                class="btn btn-ghost text-error btn-xs"
                                disabled={busy}
                                onclick={() => onRemove(ref.file.journalId, ref.goal)}
                            >
                                Remove
                            </button>
                        </span>
                    {/if}
                </li>
            {/each}
        </ul>
    </div>
{/if}

<svelte:window onresize={() => open && reposition()} />
