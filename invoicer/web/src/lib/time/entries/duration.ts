// Durations as people type and read them, mirroring the server
// (`invoicer_core::time::parse_duration` and `TimeQty`'s Display) so the
// week grid can work out a cell's change before it writes, and a sum the
// page makes reads like the server's own figures. The server stays the
// judge: a duration it refuses still comes back as its sentence.

/**
 * Seconds for `45m`, `2h`, `1h30`, `1h 30m`, `1.5` (hours), `0:45`, `90s`;
 * null for anything else, or a negative.
 */
export function parseDuration(text: string): number | null {
    const compact = text.replace(/\s+/g, "");
    if (compact === "") return null;
    const number = (s: string): number | null => (/^\d+(\.\d+)?$|^\.\d+$/.test(s) ? Number(s) : null);
    const colon = /^(\d+):(\d+)$/.exec(compact);
    if (colon !== null) {
        const minutes = Number(colon[2]);
        return minutes < 60 ? (Number(colon[1]) * 60 + minutes) * 60 : null;
    }
    if (compact.includes(":")) return null;
    const scaled = (s: string, unit: number): number | null => {
        const n = number(s);
        return n === null ? null : Math.round(n * unit);
    };
    if (compact.endsWith("s")) return scaled(compact.slice(0, -1), 1);
    if (compact.endsWith("m")) {
        const rest = compact.slice(0, -1);
        const at = rest.indexOf("h");
        if (at < 0) return scaled(rest, 60);
        const h = scaled(rest.slice(0, at), 3600);
        const m = scaled(rest.slice(at + 1), 60);
        return h === null || m === null ? null : h + m;
    }
    const at = compact.indexOf("h");
    if (at >= 0) {
        const h = scaled(compact.slice(0, at), 3600);
        if (h === null) return null;
        const rest = compact.slice(at + 1);
        if (rest === "") return h;
        const m = scaled(rest, 60);
        return m === null ? null : h + m;
    }
    return scaled(compact, 3600);
}

/** Whole minutes, half up: what a timedot line holds. */
export const roundMinutes = (seconds: number): number => Math.sign(seconds) * Math.floor(Math.abs(seconds) / 60 + 0.5);

/** `1h 05m`, `45m`, `2h`, `0m`: as the server writes `quantity.text`. */
export function formatDuration(seconds: number): string {
    const total = roundMinutes(seconds);
    const sign = total < 0 ? "-" : "";
    const minutes = Math.abs(total);
    const h = Math.floor(minutes / 60);
    const m = minutes % 60;
    if (h === 0) return `${sign}${m}m`;
    if (m === 0) return `${sign}${h}h`;
    return `${sign}${h}h ${String(m).padStart(2, "0")}m`;
}

/** What a write sends for `seconds`: whole minutes, `90m`. */
export const minutesText = (seconds: number): string => `${roundMinutes(seconds)}m`;

/** A note hledger would read differently, and why: the same rule as the server's, to warn while typing. */
export interface NoteWarning {
    /** What to take out: `re:`, `;`, "a line break". */
    offending: string;
    message: string;
}

/**
 * A note's problem, or null. hledger reads `word:` in a comment as a tag and
 * `;` as the end of the note; the server refuses both (`note-tag`) and
 * suggests a rewrite, which the form then offers.
 */
export function noteWarning(note: string): NoteWarning | null {
    if (note.includes(";")) return {offending: ";", message: "A note can't hold “;”: it would end the note early."};
    if (/[\n\r]/.test(note)) return {offending: "a line break", message: "A note is one line."};
    const tag = /(?:^|[\s,])([^\s,:]+):/.exec(note);
    if (tag !== null) {
        const offending = `${tag[1]}:`;
        return {offending, message: `hledger reads “${offending}” as a tag named “${tag[1]}”, so the note would change meaning. Reword it.`};
    }
    return null;
}
