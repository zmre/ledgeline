<script lang="ts">
    // The Journal toolbar's Recognition button (plan 45 D6, Q8): beside Find
    // Matches, a badge with everything waiting (due + needs a schedule + clear
    // invoice matches, broken down in the tooltip), and a ▾ holding the lookback for
    // "Needs a schedule" and why the helpers are on. Nothing renders while the
    // helpers are off for the ledger (`{enabled: false}`, D-gate).
    import {hint} from "#lib/keys/hint.svelte.js";
    import {dismissible} from "#lib/keys/dismissible.js";
    import {REVIEW_BUTTON} from "#lib/review/ui/buttons.js";
    import ReviewIcon from "#lib/review/ui/ReviewIcon.svelte";
    import {NEEDS_LOOKBACK_CHOICES} from "../model";
    import {lookbackLabel, reasonText} from "./format";
    import {accrual} from "../store.svelte";

    interface Props {
        /** What the button does; by default it opens the overlay on its first non-empty tab. */
        onOpen?: () => void;
        /** Disabled (a read-only engine). */
        disabled?: boolean;
    }

    const {onOpen, disabled = false}: Props = $props();

    let menuOpen = $state(false);
    const status = $derived(accrual.status);
    const count = $derived(accrual.badgeCount);
    const closedCount = $derived(status?.counts.dueClosed ?? 0);
    const tip = $derived(hint(`Recognition: ${accrual.badgeBreakdown}${closedCount > 0 ? ` · ${closedCount} due in closed periods` : ""}`, "a"));

    function open(): void {
        if (disabled) return;
        menuOpen = false;
        if (onOpen !== undefined) onOpen();
        else accrual.openOverlay();
    }

    function setLookback(event: Event): void {
        const days = Number((event.currentTarget as HTMLSelectElement).value);
        void accrual.setNeedsLookback(days);
    }
</script>

{#if status !== null}
    <div class="flex" data-testid="recognition-toolbar">
        <span class="tooltip tooltip-bottom before:max-w-72 before:whitespace-normal" data-tip={tip}>
            <button type="button" class="{REVIEW_BUTTON} gap-1.5 rounded-r-none" {disabled} onclick={open} data-testid="recognition-button">
                <ReviewIcon name="calendar" class="h-3.5 w-3.5" />
                Recognition
                <span class={["badge badge-sm tabular-nums", count > 0 ? "badge-info" : "badge-ghost"]} data-testid="recognition-count">{count}</span>
            </button>
        </span>
        <details class="dropdown dropdown-end" bind:open={menuOpen} use:dismissible={{active: menuOpen, outside: true, onDismiss: () => (menuOpen = false)}}>
            <summary
                class="{REVIEW_BUTTON} rounded-l-none border-l-base-content/20 px-1.5"
                aria-label="Recognition settings"
                title="Recognition settings"
                data-testid="recognition-menu"
            >
                <ReviewIcon name="chevron-down" class="h-3.5 w-3.5" />
            </summary>
            <div class="dropdown-content z-20 mt-1 flex w-72 flex-col gap-2 rounded-box border border-base-300 bg-base-200 p-3 text-sm shadow-lg">
                <label class="flex items-center justify-between gap-2">
                    <span>Needs a schedule looks back</span>
                    <select class="select w-28 select-sm" value={String(accrual.needsLookback)} onchange={setLookback} data-testid="recognition-lookback">
                        {#each NEEDS_LOOKBACK_CHOICES as days (days)}
                            <option value={String(days)}>{lookbackLabel(days)}</option>
                        {/each}
                    </select>
                </label>
                <p class="text-xs text-base-content/60" data-testid="recognition-reason">{reasonText(status)}</p>
            </div>
        </details>
    </div>
{/if}
