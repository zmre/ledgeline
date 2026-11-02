<script lang="ts">
    // The review card's description field: a combobox over the journal's payees
    // (plan 32 D15), the same pattern as AccountInput — opens on typing (never
    // on focus), ↑/↓ move, Enter or Tab accepts, and Escape closes the popup
    // FIRST and only then reaches the overlay (which leaves the field).
    //
    // Ranking is `rankPayees` (prefix, then word start, then frequency ×
    // recency), pure and unit-tested. The popup is fixed and portalled, because
    // `.modal-box` makes itself the containing block for fixed children.
    import {tick} from "svelte";
    import {TYPING_ATTRIBUTE} from "#lib/keys/target.js";
    import {popupPosition, portal} from "#lib/ui/anchoredPopup.js";
    import {rankPayees} from "../complete";
    import type {PayeeUse} from "../types";

    let {
        value = $bindable(""),
        payees,
        today,
        disabled = false,
        placeholder = "payee | note",
        onPick,
    }: {
        value?: string;
        payees: readonly PayeeUse[];
        today: string;
        disabled?: boolean;
        placeholder?: string;
        /** A payee was picked from the list (the add card asks for its suggestions, D36). */
        onPick?: (payee: string) => void;
    } = $props();

    const VISIBLE_ROWS = 8;
    const ROW_HEIGHT = 30;

    let open = $state(false);
    let highlighted = $state(0);
    let field = $state<HTMLInputElement | null>(null);
    let placement = $state({top: 0, left: 0, width: 0, maxHeight: 0, below: true});
    let choosing = false;

    const matches = $derived(open ? rankPayees(value, payees, today) : []);
    const uid = $props.id();
    const listId = `payee-list-${uid}`;

    function reposition(): void {
        if (field !== null)
            placement = popupPosition(field.getBoundingClientRect(), {width: window.innerWidth, height: window.innerHeight}, VISIBLE_ROWS * ROW_HEIGHT);
    }

    async function show(): Promise<void> {
        if (disabled) return;
        open = true;
        highlighted = 0;
        await tick();
        reposition();
    }

    function hide(): void {
        open = false;
    }

    function choose(payee: string): void {
        // Keep a `| note` the user already wrote: completion names the payee.
        const bar = value.indexOf("|");
        value = bar < 0 ? payee : `${payee} ${value.slice(bar)}`;
        hide();
        field?.focus();
        onPick?.(payee);
    }

    function onKeydown(event: KeyboardEvent): void {
        if (event.isComposing) return;
        const live = open && matches.length > 0;
        if (event.key === "Escape") {
            if (open) {
                event.preventDefault();
                hide();
            }
            return;
        }
        if (event.key === "ArrowDown") {
            event.preventDefault();
            if (live) highlighted = (highlighted + 1) % matches.length;
            else void show();
            return;
        }
        if (event.key === "ArrowUp" && live) {
            event.preventDefault();
            highlighted = (highlighted - 1 + matches.length) % matches.length;
            return;
        }
        // Enter accepts (and, unmodified, must not also save); ⌘↵ with the popup
        // open accepts too — the overlay sees `defaultPrevented` and waits.
        if ((event.key === "Enter" || (event.key === "Tab" && !event.shiftKey)) && live) {
            event.preventDefault();
            choose(matches[highlighted]?.payee ?? value);
            return;
        }
        if (event.key === "Tab") hide();
    }
</script>

<div class="relative" {...{[TYPING_ATTRIBUTE]: ""}}>
    <input
        bind:this={field}
        type="text"
        class="input w-full input-sm"
        bind:value
        {placeholder}
        {disabled}
        role="combobox"
        aria-label="Description"
        aria-expanded={open}
        aria-controls={listId}
        aria-autocomplete="list"
        aria-activedescendant={open && matches.length > 0 ? `${listId}-${highlighted}` : undefined}
        autocomplete="off"
        spellcheck="false"
        oninput={() => void show()}
        onkeydown={onKeydown}
        onblur={() => !choosing && hide()}
    />
</div>

{#if open && matches.length > 0}
    <ul
        use:portal
        id={listId}
        role="listbox"
        aria-label="Payee suggestions"
        class="menu fixed z-[1001] flex-nowrap overflow-y-auto rounded-box border border-base-300 bg-base-200 p-1 shadow-lg"
        style="top: {placement.top}px; left: {placement.left}px; width: {placement.width}px; max-height: {placement.maxHeight}px"
        {...{[TYPING_ATTRIBUTE]: ""}}
    >
        {#each matches as match, at (match.payee)}
            <li>
                <button
                    type="button"
                    id="{listId}-{at}"
                    role="option"
                    aria-selected={at === highlighted}
                    class="flex w-full items-baseline justify-between gap-3 px-2 py-1 text-left text-sm {at === highlighted ? 'bg-primary/25' : ''}"
                    tabindex="-1"
                    onpointerdown={() => (choosing = true)}
                    onclick={() => {
                        choosing = false;
                        choose(match.payee);
                    }}
                >
                    <span class="truncate">{match.payee}</span>
                    <span class="shrink-0 text-xs text-base-content/50 tabular-nums">{match.count}×</span>
                </button>
            </li>
        {/each}
    </ul>
{/if}

<svelte:window onresize={() => open && reposition()} />
