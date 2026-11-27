// Time tracking's wire types (plan 46), shared by Track, the lamp, Entries
// and Accounts. They mirror invoicer-core's `time::track` structs as
// `GET /api/time/state` serialises them. Pure types: no runes, no DOM.

/** A quantity of time: exact seconds, exact hours, and the server's text (`1h 05m`). */
export interface Qty {
    seconds: number;
    hours: string;
    text: string;
}

/** Why a slot holds its account (T8). */
export type SlotKind = "pinned" | "recent" | "other";

/** One of the ten slots: 1–9, then 0. */
export interface Slot {
    key: number;
    kind: SlotKind;
    /** None for an empty slot (or Other… with nothing chosen). */
    account: string | null;
    /** For people: `Acme Corp · tax return`. */
    label: string | null;
    /** Mine today, banked (the running timer is `TrackState.running`). */
    today: Qty;
    running: boolean;
    last_note: string | null;
    /** ISO date. */
    last_used: string | null;
}

/** A running timer, as the lamp shows it. */
export interface RunningView {
    account: string;
    label: string;
    slot: number | null;
    /** `YYYY-MM-DDTHH:MM:SS`, local. */
    since: string;
    elapsed: Qty;
    person: string | null;
    file: string;
    format: "timedot" | "timeclock";
}

/** The long-running guard (T-running). */
export interface Guard {
    age: Qty;
    previous_day: boolean;
    /** 0: only the previous-day rule. */
    threshold_minutes: number;
    /** `YYYY-MM-DDTHH:MM:SS`, for *Stop at last activity*. */
    last_activity: string | null;
}

/** The resume hint: the most recent account and its key. */
export interface Last {
    account: string;
    label: string;
    slot: number | null;
    date: string;
}

/** One day of the week strip. */
export interface Day {
    date: string;
    total: Qty;
}

/** What setup offers before the time files exist. */
export interface SetupOffer {
    suggested_person: string | null;
    file: string;
    accounts_file: string;
    track_file: string;
    /** Time files already under the books folder, to adopt instead. */
    found: string[];
}

/** `GET /api/time/state`. */
export interface TrackState {
    configured: boolean;
    person: string | null;
    record: "timedot" | "timeclock";
    today: string;
    now: string;
    step: number;
    slots: Slot[];
    running: RunningView | null;
    others_running: RunningView[];
    guard: Guard | null;
    banked_today: Qty;
    today_total: Qty;
    last: Last | null;
    week: Day[];
    /** Hours; 0 is none. */
    weekly_target: number;
    findings: number;
    track_file: string | null;
    /** Slots 1–5 as saved (`""` for an empty pin). */
    pinned: string[];
    /** Present only when `configured` is false. */
    setup?: SetupOffer;
}
