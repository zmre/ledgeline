// Time tracking's keys (plan 46 K). Two layers:
//
// - `timeAppLayer`, registered by the root layout, so they work on every
//   screen: ⌥⌘1 … ⌥⌘0 on macOS and Ctrl+Shift+1 … 0 elsewhere toggle a slot
//   (`macKeys` picks the platform; the engine reads the digit from the
//   physical key), ⌥⌘. / Ctrl+Shift+. stop, and ⌥⌘↑ / ⌥⌘↓ (Ctrl+Shift+↑/↓)
//   move the running timer by the step. None of them fire while typing in a
//   field, where Ctrl+Shift+arrows extend the selection.
// - `trackLayer`, Track's own bare keys: the digits, `+`/`-`, `_` (Shift+-),
//   `n`, `.`, `e`, `c` and ↑/↓ (which focus the slot, so Enter and Space
//   press it). They act through the page's handlers.
//
// The help sheet lists slot 1 for each set of ten, saying the rest follow,
// rather than ten rows that differ by a digit.
import {PRIORITY, type Binding, type Layer} from "#ui/keys/types.js";
import type {Tracker} from "./track/tracker.svelte.js";

/** The slot keys in keyboard order: 1–9, then 0. */
export const SLOT_KEYS = [1, 2, 3, 4, 5, 6, 7, 8, 9, 0] as const;

export interface TimeAppActions {
    /** Slot `key` holds nothing (an empty Other…): Track, its picker open. */
    pick: (key: number) => void;
}

/** The app-wide time keys over `tracker`. */
export function timeAppLayer(tracker: Tracker, actions: TimeAppActions): Layer {
    const ready = (): boolean => tracker.configured === true;
    const running = (): boolean => ready() && tracker.state?.running != null;
    const step = (): number => tracker.state?.step ?? 5;
    const adjustRunning = (sign: 1 | -1) => (): void => {
        const run = tracker.state?.running;
        if (run == null) return;
        void tracker.adjust({account: run.account}, sign * step());
    };
    const slots: Binding[] = SLOT_KEYS.map((key) => ({
        keys: `ctrl+shift+${key}`,
        macKeys: `meta+alt+${key}`,
        label: key === 1 ? "Start or stop time slot 1 (2–9 and 0 likewise)" : `Start or stop time slot ${key}`,
        group: "Time",
        help: key === 1,
        enabled: ready,
        run: () => {
            if (tracker.slot(key)?.account == null) actions.pick(key);
            else void tracker.toggle(key);
        },
    }));
    return {
        id: "invoicer-time",
        priority: PRIORITY.page,
        bindings: [
            ...slots,
            {keys: "ctrl+shift+.", macKeys: "meta+alt+.", label: "Stop the timer", group: "Time", enabled: running, run: () => void tracker.stop()},
            {
                keys: "ctrl+shift+ArrowUp",
                macKeys: "meta+alt+ArrowUp",
                label: "Add a step to the running timer",
                group: "Time",
                enabled: running,
                run: adjustRunning(1),
            },
            {
                keys: "ctrl+shift+ArrowDown",
                macKeys: "meta+alt+ArrowDown",
                label: "Take a step off the running timer",
                group: "Time",
                enabled: running,
                run: adjustRunning(-1),
            },
        ],
    };
}

/** What Track's keys do; the page supplies them. */
export interface TrackActions {
    /** A digit: toggle that slot (0: the Other… picker). */
    slot: (key: number) => void;
    /** `+`/`-`: move the selected slot by one step either way. */
    adjust: (sign: 1 | -1) => void;
    snap: () => void;
    note: () => void;
    stop: () => void;
    editPin: () => void;
    card: () => void;
    /** ↑/↓: select the slot above or below. */
    move: (by: 1 | -1) => void;
    /** Whether `e` has a pin to edit (slots 1–5 only). */
    canEditPin: () => boolean;
    /** Whether a timer runs (`.` and `_`). */
    running: () => boolean;
}

/** Track's own keys (plan 46 K, "Track"). */
export function trackLayer(actions: TrackActions): Layer {
    const digits: Binding[] = SLOT_KEYS.map((key) => ({
        keys: String(key),
        label: key === 1 ? "Start or stop slot 1 (2–9 likewise; 0 picks another account)" : `Start or stop slot ${key}`,
        group: "Track",
        help: key === 1,
        run: () => actions.slot(key),
    }));
    return {
        id: "invoicer-track",
        priority: PRIORITY.page,
        bindings: [
            ...digits,
            {keys: "ArrowDown", label: "Select the next slot", group: "Track", run: () => actions.move(1)},
            {keys: "ArrowUp", label: "Select the previous slot", group: "Track", run: () => actions.move(-1)},
            {keys: "+", label: "Add a step to the selected slot (on the running one, start earlier)", group: "Track", run: () => actions.adjust(1)},
            {keys: "=", label: "Add a step", group: "Track", help: false, run: () => actions.adjust(1)},
            {keys: "-", label: "Take a step off the selected slot", group: "Track", run: () => actions.adjust(-1)},
            {keys: "_", label: "Start the running timer where the previous entry ended", group: "Track", enabled: actions.running, run: actions.snap},
            {keys: "n", label: "Write a note on the selected slot", group: "Track", run: actions.note},
            {keys: ".", label: "Stop the timer", group: "Track", enabled: actions.running, run: actions.stop},
            {keys: "e", label: "Change the selected pin", group: "Track", enabled: actions.canEditPin, run: actions.editPin},
            {keys: "c", label: "Show or hide today's card", group: "Track", run: actions.card},
        ],
    };
}
