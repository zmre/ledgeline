<script lang="ts">
    // A text field with a list of suggestions: the generic sibling of
    // AccountInput (same keyboard and popup rules, read its comments), for
    // customers, templates, catalog items and field values. Matching is a
    // plain substring, since these lists are names, not colon paths.
    //
    // Keyboard: typing filters the list; ArrowDown (or Alt+ArrowDown), a
    // click in the field, or the chevron button opens it with every option
    // (the current one highlighted) until something is typed; ArrowUp/Down
    // (Ctrl+P/N) move; Enter chooses; Escape closes the list first and only
    // then reaches the page; Tab always leaves (the chevron isn't a tab stop).
    // The optional "Add new" entry is the last option whenever the text is
    // non-empty and names no option exactly.
    import {tick} from "svelte";
    import {matchOptions, type ComboOption} from "#ui/ui/comboMatch.js";
    import {TYPING_ATTRIBUTE} from "#ui/ui/typing.js";
    import {remToPx} from "#ui/theme/scale.js";
    import {popupPosition, portal, liveViewport} from "#ui/ui/anchoredPopup.js";

    let {
        value = $bindable(""),
        options,
        label,
        placeholder = "",
        onChoose,
        addNewLabel,
        onAddNew,
        onCommit,
        disabled = false,
        size = "sm",
        chevron = true,
        testid,
        id,
        passAltArrows = false,
    }: {
        value?: string;
        options: readonly ComboOption[];
        /** The field's accessible name. */
        label: string;
        placeholder?: string;
        onChoose?: (option: ComboOption) => void;
        /** The "Add new" entry's text for what was typed; with `onAddNew`, offers it. */
        addNewLabel?: (text: string) => string;
        onAddNew?: (text: string) => void;
        /** Enter with the list closed, or leaving the field. */
        onCommit?: () => void;
        disabled?: boolean;
        size?: "sm" | "xs";
        /** A button that opens the full list (off for dense grids, where a click in the field does it). */
        chevron?: boolean;
        testid?: string;
        id?: string;
        /**
         * Leave Alt+↑ / Alt+↓ to the page, untouched (the invoicer's grids
         * move the row with them, H16). Otherwise Alt+↓ opens the list, as
         * an ARIA combobox's does.
         */
        passAltArrows?: boolean;
    } = $props();

    const VISIBLE_ROWS = 8;
    const ROW_HEIGHT_REM = 1.75;

    let open = $state(false);
    /** Every option, not just those matching the text: opened by a gesture, nothing typed since. */
    let browsing = $state(false);
    let highlighted = $state(0);
    let field = $state<HTMLInputElement | null>(null);
    let popup = $state<HTMLUListElement | null>(null);
    let placement = $state({top: 0, left: 0, width: 0, maxHeight: 0, below: true});
    let choosing = false;

    type Entry = {kind: "option"; option: ComboOption} | {kind: "add"; text: string};

    const entries = $derived.by((): Entry[] => {
        if (!open) return [];
        const found: Entry[] = matchOptions(browsing ? "" : value, options).map((option) => ({kind: "option", option}));
        const text = value.trim();
        const exact = options.some((o) => o.value.toLowerCase() === text.toLowerCase() || o.label.toLowerCase() === text.toLowerCase());
        if (!browsing && onAddNew !== undefined && addNewLabel !== undefined && text !== "" && !exact) found.push({kind: "add", text});
        return found;
    });

    const uid = $props.id();
    const listId = `combo-list-${uid}`;

    function reposition(): void {
        if (field === null) return;
        placement = popupPosition(field.getBoundingClientRect(), liveViewport(), VISIBLE_ROWS * remToPx(ROW_HEIGHT_REM));
    }

    /** Open the list: filtered by the text (typing), or every option with the current one highlighted (`all`). */
    async function show(all = false): Promise<void> {
        if (disabled) return;
        browsing = all;
        open = true;
        highlighted = 0;
        await tick();
        if (all) {
            const current = entries.findIndex((e) => e.kind === "option" && e.option.value === value);
            if (current > 0) {
                highlighted = current;
                await tick();
                popup?.querySelector(`[data-at="${current}"]`)?.scrollIntoView?.({block: "nearest"});
            }
        }
        reposition();
    }

    function hide(): void {
        open = false;
        browsing = false;
        highlighted = 0;
    }

    function toggle(): void {
        if (open) hide();
        else void show(true);
        field?.focus();
    }

    function choose(entry: Entry): void {
        hide();
        if (entry.kind === "add") {
            onAddNew?.(entry.text);
            return;
        }
        value = entry.option.value;
        onChoose?.(entry.option);
        field?.focus();
    }

    function move(delta: number): void {
        if (entries.length === 0) return;
        const next = highlighted + delta;
        highlighted = next < 0 ? entries.length - 1 : next >= entries.length ? 0 : next;
        popup?.querySelector(`[data-at="${highlighted}"]`)?.scrollIntoView?.({block: "nearest"});
    }

    function onKeydown(event: KeyboardEvent): void {
        if (event.isComposing) return;
        if (passAltArrows && event.altKey && (event.key === "ArrowDown" || event.key === "ArrowUp")) return;
        if (event.key === "Escape") {
            if (open) {
                event.preventDefault();
                hide();
            }
            return;
        }
        if (event.key === "ArrowDown" || (event.ctrlKey && event.key === "n")) {
            event.preventDefault();
            if (open && !event.altKey) move(1);
            else if (!open) void show(true);
            return;
        }
        if (event.key === "ArrowUp" || (event.ctrlKey && event.key === "p")) {
            event.preventDefault();
            if (open) move(-1);
            return;
        }
        if (event.key === "Enter") {
            if (open && entries.length > 0) {
                event.preventDefault();
                choose(entries[highlighted]);
                return;
            }
            hide();
            onCommit?.();
            return;
        }
        if (event.key === "Tab") hide();
    }

    function onBlur(): void {
        if (choosing) return;
        if (open) hide();
        onCommit?.();
    }

    const entryId = (at: number): string => `${listId}-${at}`;
</script>

<div class="relative" {...{[TYPING_ATTRIBUTE]: ""}}>
    <input
        bind:this={field}
        {id}
        type="text"
        class="input {size === 'xs' ? 'input-xs' : 'input-sm'} w-full {chevron ? 'pr-7' : ''}"
        bind:value
        {placeholder}
        {disabled}
        role="combobox"
        aria-label={label}
        aria-expanded={open}
        aria-controls={listId}
        aria-autocomplete="list"
        aria-activedescendant={open && entries.length > 0 ? entryId(highlighted) : undefined}
        autocomplete="off"
        spellcheck="false"
        data-testid={testid}
        oninput={() => void show()}
        onclick={() => {
            if (!open) void show(true);
        }}
        onkeydown={onKeydown}
        onblur={onBlur}
    />
    {#if chevron && !disabled && options.length > 0}
        <button
            type="button"
            class="absolute inset-y-0 right-0 flex w-7 items-center justify-center text-base-content/50 hover:text-base-content"
            tabindex="-1"
            aria-label="Show all"
            aria-controls={listId}
            aria-expanded={open}
            onpointerdown={(event) => {
                // Keep the field's focus: its blur would close the list this reopens.
                event.preventDefault();
                choosing = true;
            }}
            onclick={() => {
                choosing = false;
                toggle();
            }}
            data-testid={testid === undefined ? undefined : `${testid}-chevron`}
        >
            <svg viewBox="0 0 20 20" class="size-3.5" aria-hidden="true" fill="currentColor"
                ><path d="M5.3 7.3a1 1 0 0 1 1.4 0L10 10.6l3.3-3.3a1 1 0 1 1 1.4 1.4l-4 4a1 1 0 0 1-1.4 0l-4-4a1 1 0 0 1 0-1.4Z" /></svg
            >
        </button>
    {/if}
</div>

{#if open && entries.length > 0}
    <ul
        use:portal
        bind:this={popup}
        id={listId}
        role="listbox"
        aria-label="{label} suggestions"
        class="menu fixed z-[1001] flex-nowrap overflow-y-auto rounded-box border border-base-300 bg-base-200 p-1 shadow-lg"
        style="top: {placement.top}px; left: {placement.left}px; width: {placement.width}px; max-height: {placement.maxHeight}px"
        {...{[TYPING_ATTRIBUTE]: ""}}
    >
        {#each entries as entry, at (entry.kind === "add" ? "\u0000add" : entry.option.value)}
            <li>
                <button
                    type="button"
                    id={entryId(at)}
                    data-at={at}
                    role="option"
                    aria-selected={at === highlighted}
                    class="block w-full truncate px-2 py-1 text-left text-sm {at === highlighted ? 'bg-primary/25' : ''} {entry.kind === 'add'
                        ? 'font-medium text-primary'
                        : ''}"
                    tabindex="-1"
                    onpointerdown={() => (choosing = true)}
                    onclick={() => {
                        choosing = false;
                        choose(entry);
                    }}
                >
                    {#if entry.kind === "add"}
                        {addNewLabel?.(entry.text)}
                    {:else}
                        {entry.option.label}
                        {#if entry.option.detail}<span class="ml-1 text-xs text-base-content/60">{entry.option.detail}</span>{/if}
                    {/if}
                </button>
            </li>
        {/each}
    </ul>
{/if}

<svelte:window onresize={() => open && reposition()} onscroll={() => open && reposition()} />
