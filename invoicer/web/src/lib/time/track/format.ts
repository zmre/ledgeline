// Track's and the lamp's pure helpers (plan 46 T-slots, T-light): how a
// duration reads, the slot key hints per platform, the note check the server
// also makes (so Track can warn while you type), the day card's text, and the
// optimistic toggle (a slot lights before the write comes back, T-perf).
// No runes, no DOM: every function here is tested in the node project.
import type {Qty, RunningView, Slot, TrackState} from "#lib/time/types.js";

const pad = (n: number): string => String(n).padStart(2, "0");

/** Whole minutes, half up (T4): 30 s is 1m, 1m29s is 1m. */
export function roundedMinutes(seconds: number): number {
    return Math.floor((Math.max(0, seconds) + 30) / 60);
}

/** A banked total: `45m`, `2h`, `2h 55m`; nothing is `—`. */
export function durationText(seconds: number): string {
    const minutes = roundedMinutes(seconds);
    if (minutes === 0) return "—";
    const h = Math.floor(minutes / 60);
    const m = minutes % 60;
    if (h === 0) return `${m}m`;
    return m === 0 ? `${h}h` : `${h}h ${pad(m)}m`;
}

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

/** `Thu` from `2026-11-26` (the local calendar). */
export function weekdayOf(date: string): string {
    const [y, m, d] = date.split("-").map(Number);
    return new Date(y, (m ?? 1) - 1, d ?? 1).toLocaleDateString("en-US", {weekday: "short"});
}

/** `Thu 26 Nov` from `2026-11-26`. */
export function dayHeading(date: string): string {
    const [y, m, d] = date.split("-").map(Number);
    const day = new Date(y, (m ?? 1) - 1, d ?? 1);
    return `${day.toLocaleDateString("en-US", {weekday: "short"})} ${d} ${day.toLocaleDateString("en-US", {month: "short"})}`;
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

/** Why a note can't be written as typed, and a rewording that can. */
export interface NoteProblem {
    /** What hledger would misread: `re:`, `;`, `a line break`. */
    offending: string;
    message: string;
    suggestion: string | null;
}

/** The tag hledger reads in one comma-separated piece: the word before its first colon. */
function tagIn(segment: string): string | null {
    const colon = segment.indexOf(":");
    if (colon === -1) return null;
    const words = segment
        .slice(0, colon)
        .split(/\s+/)
        .filter((w) => w !== "");
    return words.length === 0 ? null : words[words.length - 1];
}

function firstTag(note: string): string | null {
    for (const segment of note.split(",")) {
        const tag = tagIn(segment);
        if (tag !== null) return tag;
    }
    return null;
}

// Control characters are what the server refuses (`char::is_control`).
// eslint-disable-next-line no-control-regex -- matching control characters is the point
const CONTROL = /[\u0000-\u001f\u007f-\u009f]/;
// eslint-disable-next-line no-control-regex -- as above
const CONTROL_ALL = /[\u0000-\u001f\u007f-\u009f]/g;

/** The rewording the server suggests: `;` as `,`, `10:30` as `10.30`, other colons dropped. */
function suggestNote(note: string): string | null {
    let out = "";
    for (let at = 0; at < note.length; at += 1) {
        const c = note[at];
        if (c === ";") out += ",";
        else if (c === ":") {
            if (at > 0 && /\d/.test(note[at - 1]) && /\d/.test(note[at + 1] ?? "")) out += ".";
        } else out += c;
    }
    out = out
        .replace(CONTROL_ALL, " ")
        .split(/\s+/)
        .filter((w) => w !== "")
        .join(" ");
    return out !== "" && firstTag(out) === null ? out : null;
}

/**
 * What the server's note check (`note_problem`) would refuse, so Track warns
 * as you type: a `word:` hledger would read as a tag, a `;` or a line break
 * that would end the note early. Null when the note is fine.
 */
export function noteProblem(note: string): NoteProblem | null {
    const breaks = note.includes(";") || CONTROL.test(note);
    const tag = firstTag(note);
    if (!breaks && tag === null) return null;
    const suggestion = suggestNote(note);
    if (breaks) {
        const offending = note.includes(";") ? ";" : "a line break";
        return {offending, message: `A note can't hold ${offending}: it would end the note early.`, suggestion};
    }
    const offending = `${tag}:`;
    return {offending, message: `“${offending}” would be read as a tag, so the note would change meaning.`, suggestion};
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

/** Days from `from` to `to` (ISO dates), by calendar. */
export function daysBetween(from: string, to: string): number {
    const utc = (date: string): number => {
        const [y, m, d] = date.split("-").map(Number);
        return Date.UTC(y, (m ?? 1) - 1, d ?? 1);
    };
    return Math.round((utc(to) - utc(from)) / 86_400_000);
}

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
    const add = roundedMinutes(elapsed) * 60;
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
