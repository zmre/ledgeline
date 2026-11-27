// Durations as people type and read them, for every time page: Track's
// banked totals, Entries' week grid and sums, billing. They mirror the
// server (`invoicer_core::time::parse_duration` and `TimeQty`'s Display), so
// the week grid can work out a cell's change before it writes, and a sum the
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

/** Whole minutes, half up (T4), what a timedot line holds: 30 s is 1m, 1m29s is 1m; a negative rounds the same way, below zero. */
export function roundMinutes(seconds: number): number {
    const minutes = Math.floor((Math.abs(seconds) + 30) / 60);
    return seconds < 0 && minutes > 0 ? -minutes : minutes;
}

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

/** A banked total: `45m`, `2h`, `2h 05m`; nothing (or less than none) is `—`. */
export function durationText(seconds: number): string {
    return roundMinutes(Math.max(0, seconds)) === 0 ? "—" : formatDuration(Math.max(0, seconds));
}
