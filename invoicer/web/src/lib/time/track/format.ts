// Track's and the lamp's pure helpers (plan 46 T-slots, T-light): how a
// running timer reads, the slot key hints per platform, the day card's text,
// and the optimistic toggle (a slot lights before the write comes back,
// T-perf). Durations are ../duration.ts, dates ../dates.ts and the note
// check ../note.ts, shared with Entries and billing.
// No runes, no DOM: every function here is tested in the node project.
import type {Qty, RunningView, Slot, TrackState} from "#lib/time/types.js";
import {monthName, weekdayName} from "../dates.js";
import {durationText, roundMinutes} from "../duration.js";

const pad = (n: number): string => String(n).padStart(2, "0");

/** A running timer, live: `42s`, `12m 05s`, then `1h 22m`. */
export function elapsedText(seconds: number): string {
    const whole = Math.max(0, Math.floor(seconds));
    const h = Math.floor(whole / 3600);
    const m = Math.floor((whole % 3600) / 60);
    const s = whole % 60;
    if (h > 0) return `${h}h ${pad(m)}m`;
    if (m > 0) return `${m}m ${pad(s)}s`;
    return `${s}s`;
}

/** `14:05` from `2026-11-26T14:05:12`. */
export function clockOf(moment: string): string {
    return /T(\d{2}:\d{2})/.exec(moment)?.[1] ?? moment;
}

/** `Thu 26 Nov` from `2026-11-26`: Track's day heading (Entries' is `dayHeading` in ../dates.ts). */
export function dayHeading(date: string): string {
    return `${weekdayName(date)} ${Number(date.slice(8))} ${monthName(date)}`;
}

/** An account for people when the server gave no label: `clients:acme:tax` → `clients · acme · tax`. */
export function accountLabel(account: string): string {
    return account.split(":").join(" · ");
}

/** The app-wide key for slot `key`: `⌥⌘2` on macOS, `Ctrl+Shift+2` elsewhere (K). */
export function slotHint(key: number, mac: boolean): string {
    return mac ? `⌥⌘${key}` : `Ctrl+Shift+${key}`;
}

/** The app-wide stop key. */
export function stopHint(mac: boolean): string {
    return mac ? "⌥⌘." : "Ctrl+Shift+.";
}

/** A slot's name, or what an empty one is for (Track and the lamp's panel). */
export function slotName(slot: Slot): string {
    if (slot.label !== null) return slot.label;
    if (slot.key === 0) return "Other…";
    return slot.kind === "pinned" ? "Empty pin" : "·";
}

/** A slot's seconds today, with `elapsed` (the running timer's, live) on the one that runs. */
export function slotSeconds(slot: Slot, elapsed: number): number {
    return slot.today.seconds + (slot.running ? elapsed : 0);
}

/** A slot button's accessible name: its key, name, state and today's total (`seconds`). */
export function slotAria(slot: Slot, seconds: number): string {
    if (slot.account === null) return `Slot ${slot.key}: ${slot.key === 0 ? "choose an account" : "empty, choose an account to pin"}`;
    const total = durationText(seconds);
    return `Slot ${slot.key}: ${slotName(slot)}, ${slot.running ? "running" : "stopped"}, ${total === "—" ? "nothing" : total} today`;
}

/** One account on the day card: its total and its notes. */
export interface CardRow {
    account: string;
    label: string;
    seconds: number;
    notes: string[];
}

/**
 * Today's lines per account (the day card, T-slots), in first-seen order,
 * with the running timer's time added to its account. Labels come from the
 * slots when one holds the account.
 */
export function dayCardRows(
    lines: {account: string; seconds: number; note: string}[],
    slots: Slot[],
    running: {account: string; label: string; seconds: number} | null
): CardRow[] {
    const labels = new Map(slots.filter((s) => s.account !== null && s.label !== null).map((s) => [s.account as string, s.label as string]));
    const rows = new Map<string, CardRow>();
    const row = (account: string): CardRow => {
        let found = rows.get(account);
        if (found === undefined) {
            found = {account, label: labels.get(account) ?? accountLabel(account), seconds: 0, notes: []};
            rows.set(account, found);
        }
        return found;
    };
    for (const line of lines) {
        const r = row(line.account);
        r.seconds += line.seconds;
        const note = line.note.trim();
        if (note !== "" && !r.notes.includes(note)) r.notes.push(note);
    }
    if (running !== null) {
        const r = row(running.account);
        r.label = running.label;
        r.seconds += running.seconds;
    }
    return [...rows.values()].filter((r) => r.seconds > 0 || r.notes.length > 0);
}

/** The day card as plain text, to copy or paste into a chat. */
export function dayCardText(date: string, rows: CardRow[], person: string | null): string {
    const total = rows.reduce((sum, r) => sum + r.seconds, 0);
    const head = `${dayHeading(date)}${person === null ? "" : ` · ${person}`} · ${durationText(total)}`;
    const width = Math.max(0, ...rows.map((r) => r.label.length));
    const lines = rows.map((r) => {
        const notes = r.notes.length === 0 ? "" : `  ${r.notes.join("; ")}`;
        return `${r.label.padEnd(width)}  ${durationText(r.seconds).padStart(7)}${notes}`;
    });
    return [head, "", ...lines].join("\n");
}

const zero = (): Qty => ({seconds: 0, hours: "0", text: "0m"});
const plus = (q: Qty, seconds: number): Qty => {
    const total = Math.max(0, q.seconds + seconds);
    return {seconds: total, hours: String(total / 3600), text: durationText(total)};
};

/** `moment` (`YYYY-MM-DDTHH:MM:SS`, local) plus `minutes`, as `YYYY-MM-DDTHH:MM` (a datetime field's value). */
export function addMinutes(moment: string, minutes: number): string {
    const parts = /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2})/.exec(moment);
    if (parts === null) return moment.slice(0, 16);
    const [y, mo, d, h, mi] = parts.slice(1).map(Number);
    const at = new Date(Date.UTC(y, mo - 1, d, h, mi) + minutes * 60_000);
    return `${at.getUTCFullYear()}-${pad(at.getUTCMonth() + 1)}-${pad(at.getUTCDate())}T${pad(at.getUTCHours())}:${pad(at.getUTCMinutes())}`;
}

/** `YYYY-MM-DDTHH:MM:SS` for a local `Date`. */
export function stamp(at: Date): string {
    return `${at.getFullYear()}-${pad(at.getMonth() + 1)}-${pad(at.getDate())}T${pad(at.getHours())}:${pad(at.getMinutes())}:${pad(at.getSeconds())}`;
}

/** `state` with the running timer's `elapsed` seconds banked into its slot (and off). */
function banked(state: TrackState, elapsed: number): TrackState {
    const running = state.running;
    if (running === null) return state;
    const add = roundMinutes(Math.max(0, elapsed)) * 60;
    const slots = state.slots.map((s) =>
        s.running || (running.slot !== null && s.key === running.slot) ? {...s, running: false, today: plus(s.today, add)} : s
    );
    const total = plus(state.banked_today, add);
    return {
        ...state,
        slots,
        running: null,
        guard: null,
        banked_today: total,
        today_total: total,
        last: {account: running.account, label: running.label, slot: running.slot, date: state.today},
    };
}

/**
 * What a toggle will do, before the server says so (the < 50 ms budget): on
 * the running account it stops; on another, the running one stops and banks
 * its rounded minutes, and this one starts now. Null when there is nothing to
 * start (an empty slot): the request goes out and the server explains.
 */
export function optimisticToggle(
    state: TrackState,
    target: {slot: number} | {account: string; label?: string},
    elapsed: number,
    nowMs: number
): TrackState | null {
    const slot = "slot" in target ? state.slots.find((s) => s.key === target.slot) : state.slots.find((s) => s.account === target.account);
    const account = "slot" in target ? (slot?.account ?? null) : target.account;
    if (account === null) return null;
    const label = slot?.label ?? ("label" in target && target.label !== undefined ? target.label : accountLabel(account));
    const wasRunning = state.running?.account === account;
    const after = banked(state, elapsed);
    if (wasRunning) return after;
    const running: RunningView = {
        account,
        label,
        slot: slot?.key ?? null,
        since: stamp(new Date(nowMs)),
        elapsed: zero(),
        person: state.person,
        file: state.running?.file ?? state.track_file ?? "",
        format: state.record,
    };
    return {...after, running, slots: after.slots.map((s) => (slot !== undefined && s.key === slot.key ? {...s, running: true} : s))};
}

/** What a stop will do, before the server says so. */
export function optimisticStop(state: TrackState, elapsed: number, discard = false): TrackState {
    return banked(state, discard ? 0 : elapsed);
}
