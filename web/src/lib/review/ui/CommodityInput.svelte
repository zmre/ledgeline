<script lang="ts">
    // A posting's commodity (plan 32 D33): a small borderless field at the front
    // of the amount box, as wide as what it holds, completing from the journal's
    // commodities. The same combobox pattern as `DescriptionInput`: it opens on
    // typing (never on focus), ↑/↓ move, Enter or Tab accepts, and Escape
    // closes the popup FIRST and only then reaches the overlay.
    import {tick} from "svelte";
    import {TYPING_ATTRIBUTE} from "#lib/keys/target.js";
    import {popupPosition, portal} from "#lib/ui/anchoredPopup.js";
    import {rankCommodities} from "../complete";

    let {
        value = $bindable(""),
        commodities,
        label,
        placeholder = "",
        disabled = false,
        title,
    }: {value?: string; commodities: readonly string[]; label: string; placeholder?: string; disabled?: boolean; title?: string} = $props();

    const VISIBLE_ROWS = 6;
    const ROW_HEIGHT = 30;
    /** Room for the popup beside a field only a few characters wide. */
    const POPUP_WIDTH = 160;

    let open = $state(false);
    let highlighted = $state(0);
    let field = $state<HTMLInputElement | null>(null);
    let placement = $state({top: 0, left: 0, width: 0, maxHeight: 0, below: true});
    let choosing = false;

    const matches = $derived(open ? rankCommodities(value, commodities) : []);
    /** Its width in characters: what it holds (or the hint), at least one, at most ten. */
    const width = $derived(Math.min(10, Math.max(1, (value || placeholder).length)) + 0.5);
    const uid = $props.id();
    const listId = `commodity-list-${uid}`;

    function reposition(): void {
        if (field === null) return;
        const box = field.getBoundingClientRect();
        const anchor = {top: box.top, bottom: box.bottom, left: box.left, width: Math.max(box.width, POPUP_WIDTH)};
        placement = popupPosition(anchor, {width: window.innerWidth, height: window.innerHeight}, VISIBLE_ROWS * ROW_HEIGHT);
    }

    async function show(): Promise<void> {
        if (disabled) return;
        open = true;
        highlighted = 0;
        await tick();
        reposition();
    }

    function choose(commodity: string): void {
        value = commodity;
        open = false;
        field?.focus();
    }

    function onKeydown(event: KeyboardEvent): void {
        if (event.isComposing) return;
        const live = open && matches.length > 0;
        if (event.key === "Escape") {
            if (open) {
                event.preventDefault();
                open = false;
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
        if ((event.key === "Enter" || (event.key === "Tab" && !event.shiftKey)) && live) {
            event.preventDefault();
            choose(matches[highlighted] ?? value);
            return;
        }
        if (event.key === "Tab") open = false;
    }
</script>

<span class="contents" {...{[TYPING_ATTRIBUTE]: ""}}>
    <input
        bind:this={field}
        type="text"
        class="shrink-0 bg-transparent text-base-content/50 outline-none placeholder:text-base-content/25 focus:text-base-content"
        style="width: {width}ch"
        bind:value
        {placeholder}
        {disabled}
        {title}
        role="combobox"
        aria-label={label}
        aria-expanded={open}
        aria-controls={listId}
        aria-autocomplete="list"
        aria-activedescendant={open && matches.length > 0 ? `${listId}-${highlighted}` : undefined}
        autocomplete="off"
        spellcheck="false"
        oninput={() => void show()}
        onkeydown={onKeydown}
        onblur={() => !choosing && (open = false)}
    />
</span>

{#if open && matches.length > 0}
    <ul
        use:portal
        id={listId}
        role="listbox"
        aria-label="Commodities"
        class="menu fixed z-[1001] flex-nowrap overflow-y-auto rounded-box border border-base-300 bg-base-200 p-1 shadow-lg"
        style="top: {placement.top}px; left: {placement.left}px; width: {placement.width}px; max-height: {placement.maxHeight}px"
        {...{[TYPING_ATTRIBUTE]: ""}}
    >
        {#each matches as commodity, at (commodity)}
            <li>
                <button
                    type="button"
                    id="{listId}-{at}"
                    role="option"
                    aria-selected={at === highlighted}
                    class="px-2 py-1 text-left font-mono text-sm {at === highlighted ? 'bg-primary/25' : ''}"
                    tabindex="-1"
                    onpointerdown={() => (choosing = true)}
                    onclick={() => {
                        choosing = false;
                        choose(commodity);
                    }}
                >
                    {commodity}
                </button>
            </li>
        {/each}
    </ul>
{/if}

<svelte:window onresize={() => open && reposition()} />
