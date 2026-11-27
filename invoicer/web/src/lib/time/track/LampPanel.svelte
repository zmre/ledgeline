<!-- The lamp's tracker (plan 46 T-light): a compact panel the lamp opens
     (click, or `T` from any screen), so a slot can be started, switched,
     moved or noted without leaving the page. TimeLamp owns it: anchored to
     the lamp, Escape or a click outside closes it, and focus goes back to
     the lamp.

     At the top, the running line: its account, since when, the live time,
     −step / +step (its start, as Track's `+`/`-` on the running slot), Note
     and Stop. Below, the ten slots as Track has them, each with its digit,
     name and today's total; a press does what it does on Track (stop the
     running one, switch to another). Slot 0 picks another account here;
     an empty slot 1–9 opens Track with its picker (`onPick`), as the
     app-wide keys do. Before time tracking is set up, it points to Track's
     setup instead.

     Its keys, while it is open and over any page's: the digits, `+`/`-`,
     `n` and `.`; they are a transient layer, so they beat Track's own. The
     state and every write are the shared tracker's. -->
<script lang="ts">
    import {onMount} from "svelte";
    import {registerKeys} from "#ui/keys/keymap.svelte.js";
    import {PRIORITY, type Binding} from "#ui/keys/types.js";
    import {SLOT_KEYS} from "#lib/time/keys.js";
    import type {Slot} from "#lib/time/types.js";
    import AccountPicker from "./AccountPicker.svelte";
    import NoteField from "./NoteField.svelte";
    import {durationText} from "../duration.js";
    import {clockOf, elapsedText, slotAria, slotName, slotSeconds} from "./format.js";
    import {failureText, type Tracker} from "./tracker.svelte.js";

    let {
        tracker,
        trackHref,
        id,
        onPick,
        onClose,
    }: {
        tracker: Tracker;
        trackHref: string;
        /** The panel's id, for the lamp's `aria-controls`. */
        id: string;
        /** An empty slot 1–9: Track, its picker open for that slot. */
        onPick: (key: number) => void;
        /** Close the panel (focus goes back to the lamp). */
        onClose: () => void;
    } = $props();

    const uid = $props.id();
    const shown = $derived(tracker.state);
    const configured = $derived(shown?.configured === true);
    const slots = $derived(shown?.slots ?? []);
    const running = $derived(shown?.running ?? null);
    const step = $derived(shown?.step ?? 5);
    const runningSlot = $derived(running?.slot != null ? slots.find((s) => s.key === running.slot) : undefined);
    const recents = $derived(slots.filter((s) => s.kind === "recent" && s.account !== null).map((s) => s.account as string));

    let noting = $state(false);
    let picking = $state(false);
    let names = $state<string[] | null>(null);
    let pickError = $state<string | null>(null);
    let pickBusy = $state(false);
    let noteButton = $state<HTMLButtonElement | null>(null);
    let setupLink = $state<HTMLAnchorElement | null>(null);
    const buttons: Record<number, HTMLButtonElement | undefined> = {};

    // Into the panel on open: the running slot, else slot 1 (setup's link before setup).
    onMount(() => {
        if (!configured) setupLink?.focus();
        else (buttons[running?.slot ?? 1] ?? buttons[1])?.focus();
    });

    // The running line's note closes with the line it was for.
    $effect(() => {
        if (running === null) noting = false;
    });

    function openPicker(): void {
        picking = true;
        pickError = null;
        if (names !== null) return;
        tracker.api.accountNames().then(
            (list) => (names = list),
            (error: unknown) => {
                names = [];
                pickError = `The accounts couldn't be read. ${failureText(error)}`;
            }
        );
    }

    /** A digit: what it does on Track (0 always picks; an empty slot opens Track's picker). */
    function press(key: number): void {
        if (!configured) return;
        if (key === 0) openPicker();
        else if (tracker.slot(key)?.account == null) {
            onPick(key);
            onClose();
        } else void tracker.toggle(key);
    }

    /** A click on slot 0 toggles what it holds, as on Track; the `0` key always picks. */
    function click(slot: Slot): void {
        if (slot.key === 0 && slot.account !== null) void tracker.toggle(0);
        else press(slot.key);
    }

    async function chosen(account: string, created: boolean): Promise<void> {
        pickBusy = true;
        pickError = null;
        try {
            if (created) {
                await tracker.api.declare(account);
                names = null;
            }
            void tracker.toggleAccount(account);
            picking = false;
        } catch (error) {
            pickError = failureText(error);
        } finally {
            pickBusy = false;
        }
    }

    function adjust(sign: 1 | -1): void {
        if (running === null) return;
        void tracker.adjust({account: running.account}, sign * step);
    }

    function note(): void {
        if (running !== null) noting = true;
    }

    function closeNote(): void {
        noting = false;
        queueMicrotask(() => noteButton?.focus());
    }

    async function saveNote(text: string): Promise<void> {
        if (running === null) return;
        await tracker.saveNote({account: running.account}, text);
        closeNote();
    }

    const isRunning = (): boolean => configured && running !== null;
    const digits: Binding[] = SLOT_KEYS.map((key) => ({
        keys: String(key),
        label: key === 1 ? "Start or stop slot 1 in the tracker (2–9 likewise; 0 picks another account)" : `Start or stop slot ${key} in the tracker`,
        group: "Time",
        help: key === 1,
        enabled: () => configured,
        run: () => press(key),
    }));
    registerKeys({
        id: "invoicer-time-lamp-panel",
        priority: PRIORITY.transient,
        bindings: [
            ...digits,
            {keys: "+", label: "Start the running timer a step earlier", group: "Time", enabled: isRunning, run: () => adjust(1)},
            {keys: "=", label: "Start the running timer a step earlier", group: "Time", help: false, enabled: isRunning, run: () => adjust(1)},
            {keys: "-", label: "Start the running timer a step later", group: "Time", enabled: isRunning, run: () => adjust(-1)},
            {keys: "n", label: "Write a note on the running timer", group: "Time", enabled: isRunning, run: note},
            {keys: ".", label: "Stop the timer", group: "Time", enabled: isRunning, run: () => void tracker.stop()},
        ],
    });
</script>

<div
    {id}
    class="absolute top-full right-2 left-2 z-40 mt-1 flex max-h-[calc(100vh-6rem)] flex-col gap-3 overflow-y-auto rounded-box border border-base-300 bg-base-100 p-3 text-base-content shadow-lg md:top-0 md:right-auto md:left-full md:mt-0 md:ml-1 md:w-80"
    role="dialog"
    aria-labelledby="{uid}-title"
    data-testid="time-lamp-panel"
>
    <header class="flex items-baseline gap-2">
        <h2 id="{uid}-title" class="text-sm font-semibold">Time tracker</h2>
        {#if configured}
            <span class="ml-auto text-xs text-base-content/70" data-testid="time-lamp-panel-today"
                ><span class="font-medium text-base-content tabular-nums">{durationText(tracker.todaySeconds)}</span> today</span
            >
        {/if}
        <button
            type="button"
            class="btn btn-square btn-ghost btn-xs {configured ? '' : 'ml-auto'}"
            aria-label="Close the tracker"
            onclick={onClose}
            data-testid="time-lamp-close">✕</button
        >
    </header>

    {#if !configured}
        <p class="text-sm text-base-content/70">
            Time tracking isn't set up for these books yet. Track sets it up: who is tracking and where the time files go.
        </p>
        <!-- eslint-disable svelte/no-navigation-without-resolve -- the layout resolves it -->
        <a bind:this={setupLink} href={trackHref} class="btn btn-primary btn-sm" onclick={onClose} data-testid="time-lamp-panel-setup">Set up time tracking</a>
        <!-- eslint-enable svelte/no-navigation-without-resolve -->
    {:else}
        {#if tracker.message !== null}
            <p class="text-xs text-base-content/70" role="status" data-testid="time-lamp-panel-message">{tracker.message}</p>
        {/if}

        <section
            class="flex flex-col gap-2 rounded-field border px-2 py-2 {running !== null ? 'border-success/40 bg-success/10' : 'border-base-300'}"
            aria-label="Running timer"
            data-testid="time-lamp-panel-running"
        >
            {#if running !== null}
                <div class="flex items-center gap-2 text-sm">
                    <span class="text-success" aria-hidden="true">▶</span>
                    <span class="min-w-0 flex-1 truncate font-medium">{running.label}</span>
                    <span class="text-xs text-base-content/60">since {clockOf(running.since)}</span>
                    <span class="font-semibold tabular-nums">{elapsedText(tracker.elapsed)}</span>
                </div>
                <div class="flex flex-wrap gap-1">
                    <button type="button" class="btn btn-xs" aria-label="Start {step} minutes later" onclick={() => adjust(-1)} data-testid="time-lamp-minus"
                        >− {step}m <kbd class="kbd kbd-xs">-</kbd></button
                    >
                    <button type="button" class="btn btn-xs" aria-label="Start {step} minutes earlier" onclick={() => adjust(1)} data-testid="time-lamp-plus"
                        >+ {step}m <kbd class="kbd kbd-xs">+</kbd></button
                    >
                    <button type="button" class="btn btn-xs" bind:this={noteButton} onclick={note} aria-pressed={noting} data-testid="time-lamp-note-open"
                        >Note <kbd class="kbd kbd-xs">n</kbd></button
                    >
                    <button type="button" class="btn ml-auto btn-xs" onclick={() => void tracker.stop()} data-testid="time-lamp-stop"
                        >Stop <kbd class="kbd kbd-xs">.</kbd></button
                    >
                </div>
                {#if noting}
                    <NoteField label={running.label} initial={runningSlot?.last_note ?? ""} onSave={saveNote} onCancel={closeNote} />
                {/if}
            {:else}
                <p class="flex items-center gap-2 text-sm text-base-content/70">
                    <span class="text-base-content/50" aria-hidden="true">■</span> Not tracking. Press a slot's digit to start it.
                </p>
            {/if}
        </section>

        <ol class="flex flex-col gap-0.5" aria-label="Slots" data-testid="time-lamp-slots">
            {#each slots as slot (slot.key)}
                <li>
                    <button
                        bind:this={buttons[slot.key]}
                        type="button"
                        class="flex w-full items-center gap-2 rounded-field px-2 py-1 text-left text-sm {slot.running ? 'bg-success/10' : 'hover:bg-base-200'}"
                        aria-pressed={slot.running}
                        aria-label={slotAria(slot, slotSeconds(slot, tracker.elapsed))}
                        onclick={() => click(slot)}
                        data-testid="time-lamp-slot-{slot.key}"
                    >
                        <kbd class="kbd kbd-xs">{slot.key}</kbd>
                        <span class={slot.running ? "text-success" : "text-base-content/40"} aria-hidden="true"
                            >{slot.running ? "▶" : slot.key === 0 ? "⋯" : "○"}</span
                        >
                        <span class="min-w-0 flex-1 truncate {slot.account === null ? 'text-base-content/50' : ''}">{slotName(slot)}</span>
                        <span
                            class="text-xs tabular-nums {slot.running ? 'font-semibold' : 'text-base-content/70'}"
                            data-testid="time-lamp-slot-{slot.key}-today"
                            >{slot.running ? elapsedText(slotSeconds(slot, tracker.elapsed)) : durationText(slotSeconds(slot, tracker.elapsed))}</span
                        >
                    </button>
                </li>
            {/each}
        </ol>

        <footer class="flex items-center gap-2 text-xs text-base-content/60">
            <!-- eslint-disable svelte/no-navigation-without-resolve -- the layout resolves it -->
            <a href={trackHref} class="link link-hover" onclick={onClose} data-testid="time-lamp-open-track">Open Track</a>
            <!-- eslint-enable svelte/no-navigation-without-resolve -->
            <span class="ml-auto"><kbd class="kbd kbd-xs">T</kbd> or <kbd class="kbd kbd-xs">Esc</kbd> closes</span>
        </footer>
    {/if}
</div>

{#if picking}
    <AccountPicker
        title="Track another account"
        {names}
        {recents}
        current={slots.find((s) => s.key === 0)?.account ?? null}
        error={pickError}
        busy={pickBusy}
        onChoose={(account, created) => void chosen(account, created)}
        onCancel={() => (picking = false)}
    />
{/if}
