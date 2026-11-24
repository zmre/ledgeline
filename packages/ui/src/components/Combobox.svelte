<script lang="ts">
    // A text field with a list of suggestions: the generic sibling of
    // AccountInput (same keyboard and popup rules, read its comments), for
    // customers, templates, catalog items and field values. Matching is a
    // plain substring, since these lists are names, not colon paths.
    //
    // Keyboard: typing or ArrowDown opens the list; ArrowUp/Down (Ctrl+P/N)
    // move; Enter chooses; Escape closes the list first and only then reaches
    // the page; Tab always leaves. The optional "Add new" entry is the last
    // option whenever the text is non-empty and names no option exactly.
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
        testid,
        id,
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
        testid?: string;
        id?: string;
    } = $props();

    const VISIBLE_ROWS = 8;
    const ROW_HEIGHT_REM = 1.75;

    let open = $state(false);
    let highlighted = $state(0);
    let field = $state<HTMLInputElement | null>(null);
    let popup = $state<HTMLUListElement | null>(null);
    let placement = $state({top: 0, left: 0, width: 0, maxHeight: 0, below: true});
    let choosing = false;

    type Entry = {kind: "option"; option: ComboOption} | {kind: "add"; text: string};

    const entries = $derived.by((): Entry[] => {
        if (!open) return [];
        const found: Entry[] = matchOptions(value, options).map((option) => ({kind: "option", option}));
        const text = value.trim();
        const exact = options.some((o) => o.value.toLowerCase() === text.toLowerCase() || o.label.toLowerCase() === text.toLowerCase());
        if (onAddNew !== undefined && addNewLabel !== undefined && text !== "" && !exact) found.push({kind: "add", text});
        return found;
    });

    const uid = $props.id();
    const listId = `combo-list-${uid}`;

    function reposition(): void {
        if (field === null) return;
        placement = popupPosition(field.getBoundingClientRect(), liveViewport(), VISIBLE_ROWS * remToPx(ROW_HEIGHT_REM));
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
        highlighted = 0;
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
        if (event.key === "Escape") {
            if (open) {
                event.preventDefault();
                hide();
            }
            return;
        }
        if (event.key === "ArrowDown" || (event.ctrlKey && event.key === "n")) {
            event.preventDefault();
            if (open) move(1);
            else void show();
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
        class="input {size === 'xs' ? 'input-xs' : 'input-sm'} w-full"
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
        onkeydown={onKeydown}
        onblur={onBlur}
    />
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
