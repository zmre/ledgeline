// Track's test fixtures: a state shaped like `GET /api/time/state` over
// invoicer/fixtures/time on its clock (2026-11-26, Pat), and a fake API
// that records every call and answers from a script.
import {vi} from "vitest";
import type {TrackApi, TrackWrite} from "#lib/time/api/track.js";
import type {Qty, RunningView, Slot, TrackState} from "#lib/time/types.js";

export const qty = (seconds: number): Qty => ({seconds, hours: String(seconds / 3600), text: `${Math.round(seconds / 60)}m`});

const slot = (key: number, kind: Slot["kind"], account: string | null, label: string | null, today = 0, extra: Partial<Slot> = {}): Slot => ({
    key,
    kind,
    account,
    label,
    today: qty(today),
    running: false,
    last_note: null,
    last_used: account === null ? null : "2026-11-26",
    ...extra,
});

/** Pat's state at 15:00 on 2026-11-26, nothing running. */
export function trackState(overrides: Partial<TrackState> = {}): TrackState {
    return {
        configured: true,
        person: "pat",
        record: "timedot",
        today: "2026-11-26",
        now: "2026-11-26T15:00:00",
        step: 5,
        slots: [
            slot(1, "pinned", "clients:acme:tax-return", "Acme Corp · tax return", 0, {last_used: "2026-11-25"}),
            slot(2, "pinned", "clients:acme:advisory", "Acme Corp · advisory", 3600, {last_note: "call with Fred about the K-1"}),
            slot(3, "pinned", "clients:globex:audit", "Globex · audit", 0, {last_used: "2026-11-03"}),
            slot(4, "pinned", "internal:admin", "Internal · admin", 900),
            slot(5, "pinned", null, null),
            slot(6, "recent", "clients:acme:travel", "Acme Corp · travel", 0, {last_used: "2026-11-25"}),
            slot(7, "recent", null, null),
            slot(8, "recent", null, null),
            slot(9, "recent", null, null),
            slot(0, "other", null, null),
        ],
        running: null,
        others_running: [],
        guard: null,
        banked_today: qty(4500),
        today_total: qty(4500),
        last: {account: "internal:admin", label: "Internal · admin", slot: 4, date: "2026-11-26"},
        week: ["2026-11-23", "2026-11-24", "2026-11-25", "2026-11-26", "2026-11-27", "2026-11-28", "2026-11-29"].map((date) => ({
            date,
            total: qty(date === "2026-11-25" ? 8400 : date === "2026-11-26" ? 4500 : 0),
        })),
        weekly_target: 40,
        findings: 0,
        track_file: "time/pat/2026-11.timedot",
        pinned: ["clients:acme:tax-return", "clients:acme:advisory", "clients:globex:audit", "internal:admin"],
        ...overrides,
    };
}

/** A running timer on slot `key` of `state`, `seconds` old. */
export function running(state: TrackState, key: number, seconds = 0, since = "2026-11-26T14:05:12"): TrackState {
    const held = state.slots.find((s) => s.key === key);
    const view: RunningView = {
        account: held?.account ?? "",
        label: held?.label ?? "",
        slot: key,
        since,
        elapsed: qty(seconds),
        person: state.person,
        file: "time/pat/2026-11.timedot",
        format: "timedot",
    };
    return {
        ...state,
        running: view,
        slots: state.slots.map((s) => ({...s, running: s.key === key})),
        today_total: qty(state.banked_today.seconds + seconds),
    };
}

/** A write's answer carrying `state`. */
export const answer = (state: TrackState): TrackWrite => ({done: null, files: ["time/pat/2026-11.timedot"], state});

/** A fake API: every method a mock; `state` answers `current()`. */
export function fakeApi(current: () => TrackState = () => trackState()): TrackApi & {[K in keyof TrackApi]: ReturnType<typeof vi.fn>} {
    return {
        state: vi.fn(async () => current()),
        toggle: vi.fn(async () => answer(current())),
        stop: vi.fn(async () => answer(current())),
        adjust: vi.fn(async () => answer(current())),
        snap: vi.fn(async () => answer(current())),
        note: vi.fn(async () => ({done: null, files: []})),
        setup: vi.fn(async (person: string) => ({
            files: [
                {path: "time.journal", text: "include time-accounts.journal\n"},
                {path: "time-accounts.journal", text: "account clients:acme\n"},
                {path: person === "" ? "time/2026-11.timedot" : `time/${person}/2026-11.timedot`, text: "# Time\n"},
            ],
            person: person === "" ? null : person,
            written: false,
        })),
        adopt: vi.fn(async (file: string) => ({file, format: "timedot", needs_setting: false, written: false})),
        declare: vi.fn(async () => ({done: null, files: []})),
        accountNames: vi.fn(async () => [
            "clients:acme:advisory",
            "clients:acme:tax-return",
            "clients:globex:audit",
            "clients:initech:discovery",
            "internal:admin",
            "internal:sales",
        ]),
        dayLines: vi.fn(async () => [
            {account: "clients:acme:advisory", seconds: 3600, note: "call with Fred about the K-1"},
            {account: "internal:admin", seconds: 900, note: ""},
        ]),
        savePrefs: vi.fn(async () => ({prefs: {}})),
    } as unknown as TrackApi & {[K in keyof TrackApi]: ReturnType<typeof vi.fn>};
}
