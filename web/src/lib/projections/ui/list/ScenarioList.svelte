<!-- The scenario as one list in FILE ORDER (plan 31 C1–C13).

     Every piece of the scenario file is an entry here, top to bottom as the
     file has it: recurring rules, one-offs, automatic postings, section
     headings and notes, and (read-only) the file text the editor does not
     model. Each reads like the text it is; editing happens in a dialog over a
     DRAFT (`EntryDialog`), so nothing here changes until Done, and Done is one
     edit — one revision, one run.

     # Order

     The layout IS the file's order, so any entry moves anywhere the file can
     store it: by its grip (native drag, a drop line between cards), by J/K, or
     by the menu's Move up / Move down (the accessible path; the grip is
     `aria-hidden`). File text never moves. While a filter hides entries,
     reordering is off — a move relative to entries you cannot see would be a
     guess — and the list says so.

     # Keyboard (C13)

     j/k (↓/↑) move the cursor, g g / G jump to the ends, Enter or e edit, J/K
     move the entry, c duplicate, C duplicate as alternative, t turn off/on,
     x then x delete, n opens the Add menu, Escape disarms a delete or clears
     the cursor. Every binding is also a menu item. Nothing fires while the
     dialog is open or focus is in a field.

     # Announcements

     One polite live region says what an action did by name ("Turned off
     payroll", "Original turned off; copy added"), because a card that dims or
     moves in silence is invisible to a screen reader. -->
<script lang="ts">
    import {tick} from "svelte";
    import {flip} from "svelte/animate";
    import type {AccountClasses} from "#lib/domain/accountClass.js";
    import type {AmountStyle, ISODate} from "#lib/domain/types.js";
    import AccountInput from "#lib/journal/edit/AccountInput.svelte";
    import DateInput from "#lib/components/DateInput.svelte";
    import type {RenderProjectionBody} from "#lib/api/native.js";
    import {registerKeys} from "#lib/keys/keymap.svelte.js";
    import {PRIORITY} from "#lib/keys/types.js";
    import {listCursor} from "#lib/ui/listCursor.svelte.js";
    import {
        canDisable,
        ENTRY_FILTERS,
        entriesOf,
        entryMatches,
        entryVisible,
        filterCounts,
        isSection,
        mergeSteps,
        moveEntry,
        moveEntryBy,
        removeEntry,
        stepProblem,
        toggleDisabled,
        type AddKind,
        type Entry,
        type EntryFilter,
    } from "../../entries";
    import {addDraft, draftOf, duplicateDraft, stepDraft, type Draft} from "../../entryDraft.svelte";
    import {accountOrNull, periodWords, sectionPrefix} from "../../scenarioModel";
    import type {AssetRow, ForecastDefaults, Scenario} from "../../types";
    import RowMenu from "../RowMenu.svelte";
    import AutoEntry from "./AutoEntry.svelte";
    import CommentEntry from "./CommentEntry.svelte";
    import EntryDialog from "./EntryDialog.svelte";
    import EventEntry from "./EventEntry.svelte";
    import FixedEntry from "./FixedEntry.svelte";
    import type {FrameMenuItem, FrameProps} from "./frame";
    import RuleEntry from "./RuleEntry.svelte";

    let {
        scenario,
        accountNames,
        declared,
        commodity,
        stepDate,
        assetBalances,
        defaults = null,
        styles,
        seeded = false,
        readOnly = false,
        filter = $bindable("all"),
        render,
        onCommit,
    }: {
        /** The committed scenario. Never mutated here: every edit is a new scenario handed to `onCommit`. */
        scenario: Scenario;
        accountNames: string[];
        declared: AccountClasses;
        /** The commodity a new amount opens in. */
        commodity: string;
        /** The date a new one-off and a new step open on: the middle of the projected window. */
        stepDate: ISODate;
        /** What the last projection made of each asset rule, by group: the journal's own balances. */
        assetBalances: ReadonlyMap<string, AssetRow>;
        /** The engine's greyed counter, gains and cash accounts; null before the first projection. */
        defaults?: ForecastDefaults | null;
        styles: ReadonlyMap<string, AmountStyle>;
        /** A seed: nothing is saved until Save As. */
        seeded?: boolean;
        /** The open file is part of the main journal: it can be edited here and saved only as a copy. */
        readOnly?: boolean;
        /** The filter chip; mirrored to the URL by the page (`?show=`). */
        filter?: EntryFilter;
        /** The dialog's render call (tests); the engine's by default. */
        render?: (body: RenderProjectionBody) => Promise<string>;
        /** One edit, as the whole next scenario. */
        onCommit: (next: Scenario) => void;
    } = $props();

    const entries = $derived(entriesOf(scenario, declared));
    const counts = $derived(filterCounts(entries));
    const filtered = $derived(filter !== "all");
    const visible = $derived(entries.filter((entry) => entryVisible(entry, filter)));
    const matching = $derived(visible.filter((entry) => entryMatches(entry, filter) && !(filter !== "notes" && isSection(entry))));

    const FILTER_LABEL: Record<EntryFilter, string> = {
        all: "All",
        income: "Income",
        expense: "Expense",
        asset: "Asset",
        oneoff: "One-off",
        auto: "Auto",
        off: "Off",
        notes: "Notes",
    };
    /** "No {kind} entries", in words. */
    const EMPTY_WORD: Record<EntryFilter, string> = {
        all: "",
        income: "income",
        expense: "expense",
        asset: "asset",
        oneoff: "one-off",
        auto: "automatic posting",
        off: "turned-off",
        notes: "note or section",
    };

    // --- The cursor --------------------------------------------------------

    const cursor = listCursor<Entry>(
        () => visible,
        (entry) => entry.key
    );

    function cardOf(key: string): HTMLElement | null {
        const escaped = typeof CSS !== "undefined" && typeof CSS.escape === "function" ? CSS.escape(key) : key;
        return document.querySelector<HTMLElement>(`[data-testid="projection-entry"][data-key="${escaped}"]`);
    }

    function reveal(): void {
        const key = cursor.key;
        if (typeof key === "string") cardOf(key)?.scrollIntoView?.({block: "nearest"});
    }

    function cursorTo(key: string): void {
        const at = visible.findIndex((entry) => entry.key === key);
        if (at >= 0) cursor.to(at);
        void tick().then(reveal);
    }

    // --- Announcements -----------------------------------------------------

    let announcement = $state("");
    function announce(message: string): void {
        // Cleared first, so the same sentence twice is still read twice.
        announcement = "";
        void tick().then(() => (announcement = message));
    }

    /** Hand an edit up, then put the cursor on `key` and say `message`. */
    function commit(next: Scenario, key: string | null, message: string): void {
        armed = null;
        onCommit(next);
        if (key !== null) void tick().then(() => cursorTo(key));
        if (message !== "") announce(message);
    }

    // --- The dialog --------------------------------------------------------

    let dialog = $state<Draft | null>(null);

    /** Open the dialog on `draft`, from the card `from` (whose Edit button takes focus back on close). */
    function openDialog(draft: Draft | null, from: string | null): void {
        if (draft === null) return;
        armed = null;
        stepPrompt = null;
        // Focus the card's Edit button BEFORE the dialog mounts, so `dismissible`
        // records it as the opener and returns focus there on close.
        if (from !== null) cardOf(from)?.querySelector<HTMLElement>('[data-testid="entry-edit"], [data-testid="row-menu-trigger"]')?.focus();
        dialog = draft;
    }

    function edit(entry: Entry): void {
        if (entry.kind === "fixed") {
            toggleFixed(entry.key);
            return;
        }
        openDialog(draftOf(scenario, entry.key), entry.key);
    }

    function duplicate(entry: Entry, alternative: boolean): void {
        if (entry.kind === "fixed") return;
        openDialog(duplicateDraft(scenario, entry.key, alternative && canDisable(entry.kind)), entry.key);
    }

    function blankContext() {
        return {commodity, date: stepDate, assetPrefix: sectionPrefix("asset", accountNames, declared), cash: scenario.cash ?? defaults?.cash ?? null};
    }

    function add(kind: AddKind): void {
        const after = typeof cursor.key === "string" ? cursor.key : null;
        openDialog(addDraft(scenario, kind, after, blankContext()), null);
    }

    function closeDialog(): void {
        dialog = null;
    }

    function dialogDone(next: Scenario, key: string, draft: Draft): void {
        dialog = null;
        const name = entriesOf(next, declared).find((entry) => entry.key === key)?.name ?? "the entry";
        const message =
            draft.origin === "alternative"
                ? "Original turned off; copy added"
                : draft.origin === "duplicate"
                  ? `Copy added: ${name}`
                  : draft.origin === "add" || draft.origin === "step"
                    ? `Added ${name}`
                    : `Saved changes to ${name}`;
        commit(next, key, message);
    }

    function jumpFromDialog(group: string): void {
        dialog = null;
        cursorTo(group);
    }

    // --- Actions -----------------------------------------------------------

    function toggle(entry: Entry): void {
        if (!canDisable(entry.kind)) return;
        commit(toggleDisabled(scenario, entry.key), entry.key, `${entry.disabled ? "Turned on" : "Turned off"} ${entry.name}`);
    }

    function remove(entry: Entry): void {
        if (entry.kind === "fixed") return;
        commit(removeEntry(scenario, entry.key), null, `Deleted ${entry.name}`);
        void tick().then(() => cursor.reanchor());
    }

    function move(entry: Entry, delta: -1 | 1): void {
        if (filtered || entry.kind === "fixed") return;
        const next = moveEntryBy(scenario, entry.key, delta);
        if (next === scenario) return;
        commit(next, entry.key, `Moved ${entry.name} ${delta < 0 ? "up" : "down"}`);
    }

    function merge(entry: Entry): void {
        const merged = mergeSteps(scenario, entry.key);
        if (merged !== null) commit(merged.scenario, merged.key, `Merged the step change of ${entry.name}`);
    }

    /** "Add a step change…": a date prompt under the card, then the new segment's draft. */
    let stepPrompt = $state<{key: string; date: ISODate} | null>(null);
    const stepPromptProblem = $derived.by(() => {
        if (stepPrompt === null) return null;
        const lines = scenario.lines.filter((line) => line.group === stepPrompt?.key);
        return stepProblem(lines, stepPrompt.date);
    });

    function splitAt(): void {
        if (stepPrompt === null || stepPromptProblem !== null) return;
        const {key, date} = stepPrompt;
        openDialog(stepDraft(scenario, key, date), key);
    }

    // --- File text ---------------------------------------------------------

    let expandedFixed = $state<Record<string, boolean>>({});
    function toggleFixed(key: string): void {
        expandedFixed[key] = !expandedFixed[key];
    }

    // --- Delete: x, then x -------------------------------------------------

    let armed = $state<string | null>(null);
    function armOrDelete(): void {
        const entry = cursor.item;
        if (entry === null || entry.kind === "fixed") return;
        if (armed === entry.key) {
            remove(entry);
            return;
        }
        armed = entry.key;
        announce(`Press x again to delete ${entry.name}`);
    }
    const armedEntry = $derived(armed === null ? null : (entries.find((entry) => entry.key === armed) ?? null));

    // --- Drag --------------------------------------------------------------

    let dragging = $state<string | null>(null);
    let dropAt = $state<{key: string; side: "before" | "after"} | null>(null);

    function dragStart(event: DragEvent, entry: Entry): void {
        if (filtered || entry.kind === "fixed") {
            event.preventDefault();
            return;
        }
        dragging = entry.key;
        // Firefox will not start a drag that carries no data.
        event.dataTransfer?.setData("text/plain", entry.key);
        if (event.dataTransfer) event.dataTransfer.effectAllowed = "move";
    }

    function dragOver(event: DragEvent, entry: Entry): void {
        if (dragging === null) return;
        event.preventDefault();
        const box = (event.currentTarget as HTMLElement).getBoundingClientRect();
        // No layout in jsdom: a zero-height box reads as "before".
        const side = box.height > 0 && event.clientY > box.top + box.height / 2 ? "after" : "before";
        if (dropAt?.key !== entry.key || dropAt.side !== side) dropAt = {key: entry.key, side};
    }

    function drop(event: DragEvent, entry: Entry): void {
        const from = dragging;
        const side = dropAt?.key === entry.key ? dropAt.side : "before";
        dragEnd();
        if (from === null) return;
        event.preventDefault();
        if (from === entry.key) return;
        const layout = scenario.layout;
        const at = layout.findIndex((item) => item.key === from);
        const target = layout.findIndex((item) => item.key === entry.key);
        if (at < 0 || target < 0) return;
        const point = side === "before" ? target : target + 1;
        const next = moveEntry(scenario, from, at < point ? point - 1 : point);
        if (next === scenario) return;
        const name = entries.find((candidate) => candidate.key === from)?.name ?? "the entry";
        commit(next, from, `Moved ${name} ${side} ${entry.name}`);
    }

    function dragEnd(): void {
        dragging = null;
        dropAt = null;
    }

    // --- Step siblings -----------------------------------------------------

    let hoveredSteps = $state<string[]>([]);

    // --- Motion ------------------------------------------------------------

    const reduceMotion =
        typeof window !== "undefined" && typeof window.matchMedia === "function" && window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    const FLIP_MS = reduceMotion ? 0 : 120;

    // --- Menus -------------------------------------------------------------

    function menuOf(entry: Entry): FrameMenuItem[] {
        const items: FrameMenuItem[] = [];
        if (entry.kind === "fixed") return items;
        items.push({label: "Edit", onSelect: () => edit(entry)});
        items.push({label: "Duplicate", onSelect: () => duplicate(entry, false)});
        if (canDisable(entry.kind)) {
            items.push({label: "Duplicate as alternative", onSelect: () => duplicate(entry, true)});
            items.push({label: entry.disabled ? "Turn on" : "Turn off", onSelect: () => toggle(entry)});
        }
        if (entry.kind === "rule") {
            // Never on an asset rule: two segments would compound the same balance twice.
            if (entry.lines.every((line) => line.role !== "asset") && entry.lines[0] !== undefined && periodWords(entry.lines[0].period) !== null) {
                items.push({label: "Add a step change…", onSelect: () => (stepPrompt = {key: entry.key, date: stepDate})});
            }
            if (entry.step !== null) items.push({label: "Merge with the other step", onSelect: () => merge(entry)});
        }
        if (!filtered) {
            const index = entry.index;
            if (index > 0) items.push({label: "Move up", onSelect: () => move(entry, -1)});
            if (index < scenario.layout.length - 1) items.push({label: "Move down", onSelect: () => move(entry, 1)});
        }
        items.push({label: "Delete", onSelect: () => remove(entry), danger: true});
        return items;
    }

    function frameOf(entry: Entry, at: number): FrameProps {
        return {
            cursor: cursor.index === at,
            canDrag: !filtered && entry.kind !== "fixed",
            outlined: hoveredSteps.includes(entry.key),
            hint: dialog?.disableOriginal === entry.key ? "Will be turned off when you press Done." : armed === entry.key ? "Press x again to delete." : null,
            menu: menuOf(entry),
            onEdit: entry.kind === "fixed" ? null : () => edit(entry),
            onActivate: () => cursor.to(at),
            onDragStart: (event) => dragStart(event, entry),
            onDragEnd: dragEnd,
            onHover: (inside) => {
                if (entry.kind !== "rule" || entry.step === null) return;
                hoveredSteps = inside ? entry.step.groups.filter((group) => group !== entry.key) : [];
            },
        };
    }

    // --- The Add menu and keys ---------------------------------------------

    let addMenu = $state<{openMenu: () => void} | null>(null);
    const ADD_ITEMS: {kind: AddKind; label: string}[] = [
        {kind: "flow", label: "Recurring flow"},
        {kind: "asset", label: "Asset"},
        {kind: "oneoff", label: "One-off"},
        {kind: "auto", label: "Automatic posting"},
        {kind: "note", label: "Note"},
        {kind: "section", label: "Section heading"},
    ];

    const idle = (): boolean => dialog === null;
    const onCursor = (): boolean => dialog === null && cursor.item !== null;
    const withCursor = (run: (entry: Entry) => void) => () => {
        const entry = cursor.item;
        if (entry !== null) run(entry);
    };

    function moveCursor(delta: number): void {
        armed = null;
        cursor.move(delta);
        reveal();
    }

    registerKeys({
        id: "projection-list",
        priority: PRIORITY.widget,
        bindings: [
            {keys: "j", label: "Next entry", group: "Projections", enabled: idle, run: () => moveCursor(1)},
            {keys: "ArrowDown", label: "Next entry", group: "Projections", enabled: idle, run: () => moveCursor(1)},
            {keys: "k", label: "Previous entry", group: "Projections", enabled: idle, run: () => moveCursor(-1)},
            {keys: "ArrowUp", label: "Previous entry", group: "Projections", enabled: idle, run: () => moveCursor(-1)},
            {keys: "g g", label: "First entry", group: "Projections", enabled: idle, run: () => (cursor.first(), reveal())},
            {keys: "G", label: "Last entry", group: "Projections", enabled: idle, run: () => (cursor.last(), reveal())},
            {keys: "Enter", label: "Edit this entry", group: "Projections", enabled: onCursor, run: withCursor(edit)},
            {keys: "e", label: "Edit this entry", group: "Projections", enabled: onCursor, run: withCursor(edit)},
            {keys: "J", label: "Move entry down", group: "Projections", enabled: () => onCursor() && !filtered, run: withCursor((entry) => move(entry, 1))},
            {keys: "K", label: "Move entry up", group: "Projections", enabled: () => onCursor() && !filtered, run: withCursor((entry) => move(entry, -1))},
            {keys: "c", label: "Duplicate this entry", group: "Projections", enabled: onCursor, run: withCursor((entry) => duplicate(entry, false))},
            {keys: "C", label: "Duplicate as an alternative", group: "Projections", enabled: onCursor, run: withCursor((entry) => duplicate(entry, true))},
            {keys: "t", label: "Turn this entry off or on", group: "Projections", enabled: onCursor, run: withCursor(toggle)},
            {keys: "x", label: "Delete this entry (press twice)", group: "Projections", enabled: onCursor, run: armOrDelete},
            {keys: "n", label: "Add an entry", group: "Projections", enabled: idle, run: () => addMenu?.openMenu()},
            {
                keys: "Escape",
                label: "Cancel a delete, or clear the cursor",
                group: "Projections",
                enabled: idle,
                run: () => {
                    if (armed !== null) {
                        armed = null;
                        announce("Delete cancelled");
                    } else cursor.clear();
                },
            },
        ],
    });

    // --- The default cash account --------------------------------------------

    let cashText = $derived(scenario.cash ?? "");
    function commitCash(): void {
        const next = accountOrNull(cashText);
        if (next === scenario.cash) return;
        commit({...scenario, cash: next}, null, next === null ? "Default cash account cleared" : `Default cash account set to ${next}`);
    }

    /** The placeholder card a new entry will land at while its dialog is open. */
    const pendingAfter = $derived(dialog !== null && dialog.origin !== "edit" ? dialog.after : undefined);
    const pendingLabel = $derived.by(() => {
        switch (dialog?.origin) {
            case "duplicate":
                return "New copy — editing";
            case "alternative":
                return "New alternative — editing";
            case "step":
                return "New step — editing";
            default:
                return "New entry — editing";
        }
    });
    const pendingAtEnd = $derived(pendingAfter === null || (pendingAfter !== undefined && !visible.some((entry) => entry.key === pendingAfter)));
</script>

{#snippet placeholder(inline: boolean)}
    <div
        class={["rounded-box border border-dashed border-primary/50 bg-primary/5 px-3 py-2 text-xs text-primary/80", inline && "mt-1"]}
        role={inline ? "note" : "listitem"}
        data-testid="entry-placeholder"
        aria-label={pendingLabel}
    >
        {pendingLabel}
    </div>
{/snippet}

<div class="flex flex-col gap-3" data-testid="projection-list-panel">
    <!-- Header: the scenario's own default cash account, the filters, Add. -->
    <div class="flex flex-wrap items-end justify-between gap-x-6 gap-y-3">
        <div class="flex flex-wrap items-center gap-2 text-sm" data-testid="projection-cash-default">
            <span class="text-base-content/70">Default cash account</span>
            <div class="w-64">
                <AccountInput
                    bind:value={() => cashText, (next) => (cashText = next)}
                    {accountNames}
                    size="xs"
                    label="Default cash account"
                    placeholder={defaults?.cash ?? "default"}
                    onCommit={commitCash}
                />
            </div>
            <span class="text-xs text-base-content/45">Where a rule's money goes when it names no counter-account.</span>
        </div>
        <RowMenu
            bind:this={addMenu}
            label="Add an entry"
            text="+ Add ▾"
            triggerClass="btn btn-sm btn-soft btn-primary"
            testid="projection-add-menu"
            items={ADD_ITEMS.map((item) => ({label: item.label, onSelect: () => add(item.kind)}))}
        />
    </div>

    {#if readOnly}
        <p class="rounded-box border border-info/25 bg-info/6 px-3 py-2 text-xs text-base-content/70" data-testid="projection-read-only">
            <strong class="font-semibold text-info">Read-only here:</strong> this file is part of your main journal, so it is never saved over. Edit freely,
            then
            <strong>Save as…</strong> a copy.
        </p>
    {/if}
    {#if seeded}
        <p class="rounded-box border border-base-content/10 bg-base-200/50 px-3 py-2 text-xs text-base-content/65" data-testid="projection-seeded">
            Seeded from your budget and the last 12 months — nothing is saved until you <strong>Save as…</strong>
        </p>
    {/if}

    {#if entries.length > 0}
        <div class="flex flex-wrap items-center gap-1.5" role="group" aria-label="Show entries">
            {#each ENTRY_FILTERS as chip (chip)}
                {#if chip === "all" || counts[chip] > 0 || filter === chip}
                    <button
                        type="button"
                        class={[
                            "btn h-7 min-h-7 gap-1.5 rounded-full px-3 text-xs font-normal",
                            filter === chip
                                ? "border-primary/60 bg-primary/15 text-base-content"
                                : "border-base-content/10 bg-transparent text-base-content/70 hover:bg-base-200",
                        ]}
                        aria-pressed={filter === chip}
                        data-testid="projection-filter-{chip}"
                        onclick={() => (filter = chip)}
                    >
                        {FILTER_LABEL[chip]}
                        <span class="font-mono text-[0.6875rem] text-base-content/45 tabular-nums">{counts[chip]}</span>
                    </button>
                {/if}
            {/each}
            {#if filtered}
                <span class="ml-1 text-xs text-base-content/45" data-testid="projection-filter-hint">Clear the filter to reorder.</span>
            {/if}
        </div>
    {/if}

    {#if armedEntry !== null}
        <div class="flex flex-wrap items-center gap-2 rounded-box border border-error/40 px-3 py-1.5 text-sm" role="alert" data-testid="entry-delete-confirm">
            <span>Delete <strong class="font-medium">{armedEntry.name}</strong>? Press <kbd class="kbd kbd-xs">x</kbd> again.</span>
            <button type="button" class="btn btn-error btn-xs" onclick={() => remove(armedEntry)}>Delete</button>
            <button type="button" class="btn btn-ghost btn-xs" onclick={() => (armed = null)}>Keep</button>
        </div>
    {/if}

    {#if entries.length === 0}
        <div
            class="flex flex-col items-center gap-3 rounded-box border border-dashed border-base-content/12 px-6 py-8 text-center"
            data-testid="projection-empty"
        >
            <p class="text-sm text-base-content/70">No entries yet.</p>
            <p class="max-w-md text-xs text-base-content/50">
                A scenario is a list of what you expect: money in and out each month, balances that grow, one-off events, and postings that follow other
                postings.
            </p>
            <div class="flex flex-wrap justify-center gap-2">
                {#each ADD_ITEMS.slice(0, 4) as item (item.kind)}
                    <button type="button" class="btn btn-outline btn-sm" onclick={() => add(item.kind)}>+ {item.label}</button>
                {/each}
            </div>
        </div>
    {:else}
        <div class="flex flex-col gap-1" role="list" aria-label="Scenario entries, in file order" data-testid="projection-list">
            {#each visible as entry, at (entry.key)}
                {@const frame = frameOf(entry, at)}
                <div
                    role="listitem"
                    class={[
                        "relative",
                        isSection(entry) && "sticky top-12 z-[6] -mx-1 mt-4 bg-base-100 px-1 first:mt-0",
                        dragging === entry.key && "opacity-40",
                    ]}
                    animate:flip={{duration: FLIP_MS}}
                    ondragover={(event) => dragOver(event, entry)}
                    ondrop={(event) => drop(event, entry)}
                >
                    {#if dropAt?.key === entry.key && dragging !== entry.key}
                        <div
                            class={[
                                "pointer-events-none absolute inset-x-1 z-10 h-0.5 rounded-full bg-primary",
                                dropAt.side === "before" ? "-top-[3px]" : "-bottom-[3px]",
                            ]}
                            aria-hidden="true"
                            data-testid="drop-line"
                        ></div>
                    {/if}
                    {#if entry.kind === "rule"}
                        <RuleEntry {entry} {frame} {declared} {styles} {assetBalances} {defaults} onJump={cursorTo} />
                    {:else if entry.kind === "event"}
                        <EventEntry {entry} {frame} {styles} {defaults} />
                    {:else if entry.kind === "auto"}
                        <AutoEntry {entry} {frame} {styles} />
                    {:else if entry.kind === "comment"}
                        <CommentEntry {entry} {frame} />
                    {:else}
                        <FixedEntry {entry} {frame} expanded={expandedFixed[entry.key] === true} />
                    {/if}
                    {#if stepPrompt?.key === entry.key}
                        <div class="mt-1 ml-6 flex flex-wrap items-center gap-2 rounded-box bg-base-200 px-3 py-2 text-xs" data-testid="step-prompt">
                            <span class="text-base-content/70">Step on</span>
                            <DateInput
                                size="xs"
                                value={stepPrompt.date}
                                aria-label="Date the step starts"
                                onchange={(e) => stepPrompt && (stepPrompt.date = e.currentTarget.value)}
                            />
                            <button type="button" class="btn btn-primary btn-xs" disabled={stepPromptProblem !== null} onclick={splitAt}>Split here</button>
                            <button type="button" class="btn btn-ghost btn-xs" onclick={() => (stepPrompt = null)}>Cancel</button>
                            {#if stepPromptProblem !== null}<span class="text-warning">{stepPromptProblem}</span>{/if}
                        </div>
                    {/if}
                    {#if pendingAfter === entry.key}
                        {@render placeholder(true)}
                    {/if}
                </div>
            {/each}
            {#if pendingAtEnd}
                {@render placeholder(false)}
            {/if}
        </div>
        {#if matching.length === 0}
            <p
                class="rounded-box border border-dashed border-base-content/12 px-4 py-4 text-center text-sm text-base-content/60"
                data-testid="projection-filter-empty"
            >
                No {EMPTY_WORD[filter]} entries.
                <button type="button" class="link link-primary" onclick={() => (filter = "all")}>Clear filter</button>
            </p>
        {/if}
    {/if}

    <div class="sr-only" role="status" aria-live="polite" aria-atomic="true" data-testid="projection-list-status">{announcement}</div>
</div>

{#if dialog !== null}
    <EntryDialog
        initial={dialog}
        {scenario}
        {declared}
        {accountNames}
        {commodity}
        {styles}
        {defaults}
        {assetBalances}
        {...render === undefined ? {} : {render}}
        onDone={dialogDone}
        onCancel={closeDialog}
        onJump={jumpFromDialog}
    />
{/if}
