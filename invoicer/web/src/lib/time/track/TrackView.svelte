<!-- Track (plan 46 T-slots): ten slots, each a key. 1–5 are your pins, 6–9
     the recent accounts that aren't pinned, 0 is Other… (any account, or a
     new one). A press on a stopped slot switches the timer to it (one lamp
     lit); on the running one it stops. Each slot shows today's total, the
     running one since when, the others their latest note.

     Under the slots: −step / +step (the selected slot's total; on the
     running one its start), Note, Stop. The header has today's total, the
     week strip below it; `c` shows today's card. `e` changes a pin in
     place. The keys are `trackLayer`'s; the help sheet lists them.

     Wide: one row per slot. Narrow (below `md`, a container query): a
     two-column grid of large buttons, the digit kept on each. -->
<script lang="ts">
    import {untrack} from "svelte";
    import {registerKeys} from "#ui/keys/keymap.svelte.js";
    import {trackLayer} from "#lib/time/keys.js";
    import type {Slot} from "#lib/time/types.js";
    import AccountPicker from "./AccountPicker.svelte";
    import DayCard from "./DayCard.svelte";
    import NoteField from "./NoteField.svelte";
    import WeekStrip from "./WeekStrip.svelte";
    import {daysBetween, weekdayName} from "../dates.js";
    import {durationText} from "../duration.js";
    import {clockOf, dayCardRows, dayHeading, elapsedText, slotAria, slotName, slotSeconds, type CardRow} from "./format.js";
    import {failureText, PIN_COUNT, type Tracker} from "./tracker.svelte.js";

    let {tracker, pick = null}: {tracker: Tracker; /** Open the picker for this slot at once (`?pick=0` from the app-wide keys). */ pick?: number | null} =
        $props();

    const shown = $derived(tracker.state);
    const slots = $derived(shown?.slots ?? []);
    const running = $derived(shown?.running ?? null);
    const step = $derived(shown?.step ?? 5);

    /** The selected slot's key: the running one's at first, else 1. */
    let selected = $state(untrack(() => tracker.state?.running?.slot ?? 1));
    const selectedSlot = $derived(slots.find((s) => s.key === selected));

    type Picking = {mode: "other"} | {mode: "pin"; key: number} | {mode: "empty"; key: number};
    let picking = $state<Picking | null>(null);
    let names = $state<string[] | null>(null);
    let pickError = $state<string | null>(null);
    let pickBusy = $state(false);
    let noting = $state<Slot | null>(null);
    let card = $state(false);
    let cardRows = $state<CardRow[]>([]);
    let cardLoading = $state(false);
    let editingPerson = $state(false);
    let personDraft = $state("");

    const buttons: Record<number, HTMLButtonElement | undefined> = {};

    const recents = $derived(slots.filter((s) => s.kind === "recent" && s.account !== null).map((s) => s.account as string));

    function openPicker(next: Picking): void {
        picking = next;
        pickError = null;
        if (names === null) {
            tracker.api.accountNames().then(
                (list) => (names = list),
                (error: unknown) => {
                    names = [];
                    pickError = `The accounts couldn't be read. ${failureText(error)}`;
                }
            );
        }
    }

    // `?pick=N`: an app-wide key found slot N empty.
    $effect(() => {
        const key = pick;
        if (key === null) return;
        untrack(() => {
            if (shown?.configured) openPicker(key === 0 ? {mode: "other"} : key <= PIN_COUNT ? {mode: "pin", key} : {mode: "empty", key});
        });
    });

    function press(key: number): void {
        selected = key;
        const slot = slots.find((s) => s.key === key);
        if (key === 0) openPicker({mode: "other"});
        else if (slot?.account == null) openPicker(key <= PIN_COUNT ? {mode: "pin", key} : {mode: "empty", key});
        else void tracker.toggle(key);
    }

    /** A click on slot 0 toggles what it holds; the `0` key always opens the picker (with it first). */
    function click(slot: Slot): void {
        if (slot.key === 0 && slot.account !== null) {
            selected = 0;
            void tracker.toggle(0);
        } else press(slot.key);
    }

    async function chosen(account: string, created: boolean): Promise<void> {
        const target = picking;
        if (target === null) return;
        pickBusy = true;
        pickError = null;
        try {
            if (created) await tracker.api.declare(account);
            if (target.mode === "pin") {
                await tracker.savePin(target.key - 1, account);
                selected = target.key;
            } else {
                selected = 0;
                void tracker.toggleAccount(account);
            }
            picking = null;
            if (created) names = null;
        } catch (error) {
            pickError = failureText(error);
        } finally {
            pickBusy = false;
        }
    }

    async function clearPin(key: number): Promise<void> {
        pickBusy = true;
        try {
            await tracker.savePin(key - 1, "");
            picking = null;
        } catch (error) {
            pickError = failureText(error);
        } finally {
            pickBusy = false;
        }
    }

    function adjust(sign: 1 | -1): void {
        const slot = selectedSlot;
        if (slot?.account == null) return;
        void tracker.adjust({account: slot.account}, sign * step);
    }

    function note(): void {
        const slot = selectedSlot?.account != null ? selectedSlot : slots.find((s) => s.running);
        if (slot === undefined || slot.account === null) return;
        noting = slot;
    }

    async function saveNote(text: string): Promise<void> {
        const slot = noting;
        if (slot === null || slot.account === null) return;
        await tracker.saveNote({account: slot.account}, text);
        noting = null;
    }

    async function loadCard(): Promise<void> {
        cardLoading = true;
        try {
            const lines = await tracker.dayLines();
            cardRows = dayCardRows(lines, slots, running === null ? null : {account: running.account, label: running.label, seconds: tracker.elapsed});
        } catch (error) {
            console.warn("time card:", error);
        } finally {
            cardLoading = false;
        }
    }

    function toggleCard(): void {
        card = !card;
        if (card) void loadCard();
    }

    function move(by: 1 | -1): void {
        const keys = slots.map((s) => s.key);
        const at = keys.indexOf(selected);
        const next = keys[Math.min(keys.length - 1, Math.max(0, (at === -1 ? 0 : at) + by))];
        if (next === undefined) return;
        selected = next;
        buttons[next]?.focus();
    }

    registerKeys(
        trackLayer({
            slot: press,
            adjust,
            snap: () => void tracker.snap(),
            note,
            stop: () => void tracker.stop(),
            editPin: () => openPicker({mode: "pin", key: selected}),
            card: toggleCard,
            move,
            canEditPin: () => selected >= 1 && selected <= PIN_COUNT,
            running: () => tracker.state?.running != null,
        })
    );

    function lastUsed(slot: Slot): string | null {
        if (slot.last_used === null || shown === null || slot.last_used === shown.today) return null;
        const days = daysBetween(slot.last_used, shown.today);
        if (days === 1) return "yesterday";
        if (days < 7) return weekdayName(slot.last_used);
        return slot.last_used;
    }

    function todayOf(slot: Slot): number {
        return slotSeconds(slot, tracker.elapsed);
    }

    function savePerson(): void {
        editingPerson = false;
        void tracker.savePerson(personDraft).catch((error: unknown) => (tracker.message = `Your name couldn't be saved. ${failureText(error)}`));
    }
</script>

{#if shown !== null}
    <div class="mx-auto flex w-full max-w-3xl flex-col gap-4" data-testid="time-track">
        <header class="flex flex-wrap items-baseline gap-x-3 gap-y-1 print:hidden">
            <h1 class="text-xl font-semibold tracking-tight">Track</h1>
            <p class="ml-auto text-sm text-base-content/70" data-testid="time-track-today">
                {dayHeading(shown.today)} · <span class="font-medium text-base-content tabular-nums">{durationText(tracker.todaySeconds)}</span> today
            </p>
        </header>

        <div class="print:hidden">
            <WeekStrip week={shown.week} today={shown.today} todaySeconds={tracker.todaySeconds} target={shown.weekly_target} />
        </div>

        {#if tracker.message !== null}
            <div class="alert text-sm alert-warning print:hidden" role="status" data-testid="time-track-message">{tracker.message}</div>
        {/if}

        {#if card}
            <DayCard date={shown.today} rows={cardRows} person={shown.person} loading={cardLoading} onClose={() => (card = false)} />
        {/if}

        <ol class="grid grid-cols-2 gap-2 md:grid-cols-1 md:gap-1 print:hidden" aria-label="Slots" data-testid="time-slots">
            {#each slots as slot (slot.key)}
                {#if slot.key === 6}
                    <li class="col-span-2 mt-1 flex items-center gap-2 text-xs text-base-content/60 md:col-span-1" aria-hidden="true">
                        <span>recent</span><span class="h-px flex-1 bg-base-300"></span>
                    </li>
                {/if}
                <li class={slot.key === 0 ? "col-span-2 md:col-span-1" : ""}>
                    <button
                        bind:this={buttons[slot.key]}
                        type="button"
                        class="flex h-full w-full flex-col items-start gap-1 rounded-box border px-3 py-3 text-left md:flex-row md:items-center md:gap-3 md:py-2
                            {slot.running ? 'border-success/50 bg-success/10' : 'border-base-300 hover:bg-base-200'}
                            {selected === slot.key ? 'outline-2 outline-offset-1 outline-primary' : ''}"
                        aria-pressed={slot.running}
                        aria-label={slotAria(slot, todayOf(slot))}
                        onclick={() => click(slot)}
                        onfocus={() => (selected = slot.key)}
                        data-testid="time-slot-{slot.key}"
                        data-selected={selected === slot.key ? "" : undefined}
                    >
                        <span class="flex w-full items-center gap-2 md:w-auto md:flex-1">
                            <kbd class="kbd kbd-sm">{slot.key}</kbd>
                            <span class={slot.running ? "text-success" : "text-base-content/40"} aria-hidden="true"
                                >{slot.running ? "▶" : slot.key === 0 ? "⋯" : "○"}</span
                            >
                            <span class="min-w-0 flex-1 truncate {slot.account === null ? 'text-base-content/50' : 'font-medium'}">{slotName(slot)}</span>
                        </span>
                        <span class="flex w-full items-center gap-2 text-sm md:w-auto">
                            <span class="tabular-nums {slot.running ? 'font-semibold' : ''}" data-testid="time-slot-{slot.key}-today">
                                {slot.running ? elapsedText(todayOf(slot)) : durationText(todayOf(slot))}
                            </span>
                            <span class="min-w-0 flex-1 truncate text-xs text-base-content/60 md:w-48 md:flex-none">
                                {#if slot.running && running !== null}
                                    since {clockOf(running.since)}
                                {:else if slot.last_note !== null}
                                    ▸ {slot.last_note}
                                {:else if lastUsed(slot) !== null}
                                    {lastUsed(slot)}
                                {/if}
                            </span>
                        </span>
                    </button>
                </li>
            {/each}
        </ol>

        <div class="flex flex-wrap items-center gap-2 print:hidden" data-testid="time-controls">
            <button type="button" class="btn btn-sm" onclick={() => adjust(-1)} disabled={selectedSlot?.account == null} data-testid="time-minus">
                − {step}m <kbd class="kbd kbd-xs">-</kbd>
            </button>
            <button type="button" class="btn btn-sm" onclick={() => adjust(1)} disabled={selectedSlot?.account == null} data-testid="time-plus">
                + {step}m <kbd class="kbd kbd-xs">+</kbd>
            </button>
            <button type="button" class="btn btn-sm" onclick={note} disabled={selectedSlot?.account == null && running === null} data-testid="time-note-open">
                Note <kbd class="kbd kbd-xs">n</kbd>
            </button>
            <button type="button" class="btn btn-sm" onclick={() => void tracker.stop()} disabled={running === null} data-testid="time-stop">
                Stop <kbd class="kbd kbd-xs">.</kbd>
            </button>
            <span class="ml-auto flex flex-wrap gap-2">
                <button
                    type="button"
                    class="btn btn-ghost btn-sm"
                    onclick={() => openPicker({mode: "pin", key: selected})}
                    disabled={selected < 1 || selected > PIN_COUNT}
                    data-testid="time-edit-pin"
                >
                    Change pin <kbd class="kbd kbd-xs">e</kbd>
                </button>
                <button type="button" class="btn btn-ghost btn-sm" onclick={toggleCard} aria-pressed={card} data-testid="time-card-toggle">
                    Today's card <kbd class="kbd kbd-xs">c</kbd>
                </button>
            </span>
        </div>

        {#if noting !== null}
            <div class="print:hidden">
                <NoteField label={slotName(noting)} initial={noting.last_note ?? ""} onSave={saveNote} onCancel={() => (noting = null)} />
            </div>
        {/if}

        <footer class="flex flex-wrap items-center gap-2 text-xs text-base-content/60 print:hidden" data-testid="time-person">
            {#if editingPerson}
                <label for="time-person-input">Your name in the time files</label>
                <input
                    id="time-person-input"
                    class="input w-32 input-xs"
                    type="text"
                    bind:value={personDraft}
                    onkeydown={(e) => {
                        if (e.key === "Enter") savePerson();
                        else if (e.key === "Escape") {
                            e.preventDefault();
                            editingPerson = false;
                        }
                    }}
                />
                <button type="button" class="btn btn-ghost btn-xs" onclick={() => (editingPerson = false)}>Discard</button>
                <button type="button" class="btn btn-xs" onclick={savePerson}>Save</button>
            {:else}
                <span>{shown.person === null ? "Tracking without a name" : `Tracking as ${shown.person}`}</span>
                <button
                    type="button"
                    class="btn btn-ghost btn-xs"
                    onclick={() => {
                        personDraft = shown?.person ?? "";
                        editingPerson = true;
                    }}>Change</button
                >
            {/if}
        </footer>
    </div>

    {#if picking !== null}
        {@const mode = picking}
        <AccountPicker
            title={mode.mode === "pin" ? `Pin to slot ${mode.key}` : "Track another account"}
            {names}
            {recents}
            current={mode.mode === "pin"
                ? (slots.find((s) => s.key === mode.key && s.kind === "pinned")?.account ?? null)
                : (slots.find((s) => s.key === 0)?.account ?? null)}
            onClear={mode.mode === "pin" && (shown.pinned[mode.key - 1] ?? "") !== "" ? () => void clearPin(mode.key) : undefined}
            error={pickError}
            busy={pickBusy}
            onChoose={(account, created) => void chosen(account, created)}
            onCancel={() => (picking = null)}
        />
    {/if}
{/if}
