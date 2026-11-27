// The tracker's API calls (plan 46 Phase 3): the state Track and the lamp
// show, the tracker's writes, setup, and the two small reads Track needs from
// elsewhere (the account names for Other…, today's lines for the day card).
// Same-origin and token-gated like lib/api.ts. A refusal is an EditorApiError
// carrying the server's `code` (`not-set-up`, `note-tag`, `conflict`,
// `external-change`) and its whole body (`offending`, `suggestion`).
//
// `TrackApi` is the seam the tracker store takes, so its tests pass their own.
import {send} from "#lib/editorApi.js";
import type {TrackState} from "#lib/time/types.js";

const isObject = (body: unknown): body is Record<string, unknown> => typeof body === "object" && body !== null && !Array.isArray(body);

/** Does `body` look like `GET /api/time/state`'s answer? */
export function isTrackState(body: unknown): body is TrackState {
    return (
        isObject(body) &&
        typeof body.configured === "boolean" &&
        Array.isArray(body.slots) &&
        Array.isArray(body.week) &&
        typeof body.step === "number" &&
        isObject(body.today_total)
    );
}

/** A slot by its key, or an account by name. */
export type SlotTarget = {slot: number} | {account: string};

/** What a tracker write answers: the outcome, the files written, and the new state. */
export interface TrackWrite {
    done: {started?: string | null; stopped?: {account: string; at: string} | null; since?: string | null} | null;
    files: string[];
    state?: TrackState;
}

/** One file setup would write, and its text. */
export interface SetupFile {
    path: string;
    text: string;
}

/** `POST /api/time/setup`'s answer. */
export interface SetupAnswer {
    files: SetupFile[];
    person: string | null;
    written: boolean;
}

/** `POST /api/time/adopt`'s answer. */
export interface AdoptAnswer {
    file: string;
    format: string | null;
    /** The books' settings file must name it (an unusual name). */
    needs_setting: boolean;
    written: boolean;
}

/** One of today's lines, for the day card. */
export interface DayLine {
    account: string;
    seconds: number;
    note: string;
}

/** The requests the tracker makes. */
export interface TrackApi {
    state: () => Promise<TrackState>;
    toggle: (target: SlotTarget) => Promise<TrackWrite>;
    stop: (options?: {at?: string; discard?: boolean}) => Promise<TrackWrite>;
    adjust: (target: SlotTarget, minutes: number) => Promise<TrackWrite>;
    snap: () => Promise<TrackWrite>;
    note: (target: SlotTarget, note: string) => Promise<unknown>;
    setup: (person: string, dryRun: boolean) => Promise<SetupAnswer>;
    adopt: (file: string, dryRun: boolean) => Promise<AdoptAnswer>;
    declare: (account: string) => Promise<unknown>;
    accountNames: () => Promise<string[]>;
    dayLines: (date: string, person: string | null) => Promise<DayLine[]>;
    /** `PATCH /api/ui-prefs` with `{time: values}` (`pinned`, `person`). */
    savePrefs: (values: Record<string, unknown>) => Promise<unknown>;
}

const isWrite = (body: unknown): boolean => isObject(body) && Array.isArray(body.files);

/** Write answers carry `state` for tracker writes; check it when present. */
function write(path: string, body: unknown): Promise<TrackWrite> {
    return send<TrackWrite>("POST", path, (b) => isWrite(b) && (!isObject(b) || b.state === undefined || isTrackState(b.state)), body);
}

/** The account names `GET /api/time/accounts` lists, archived ones left out. */
export function accountNamesOf(body: unknown): string[] {
    if (!isObject(body) || !Array.isArray(body.accounts)) return [];
    const names: string[] = [];
    for (const row of body.accounts) {
        if (!isObject(row) || typeof row.account !== "string") continue;
        const archived = isObject(row.archived) && typeof row.archived.value === "string" && /^(yes|y|true|1)$/i.test(row.archived.value);
        if (!archived) names.push(row.account);
    }
    return names;
}

/** Today's lines from `GET /api/time/entries`: account, seconds, note. */
export function dayLinesOf(body: unknown): DayLine[] {
    if (!isObject(body) || !Array.isArray(body.entries)) return [];
    const lines: DayLine[] = [];
    for (const row of body.entries) {
        if (!isObject(row) || typeof row.account !== "string" || !isObject(row.quantity)) continue;
        const seconds = typeof row.quantity.seconds === "number" ? row.quantity.seconds : 0;
        lines.push({account: row.account, seconds, note: typeof row.note === "string" ? row.note : ""});
    }
    return lines;
}

/** The server's API. */
export const httpTrackApi: TrackApi = {
    state: () => send<TrackState>("GET", "/api/time/state", isTrackState),
    toggle: (target) => write("/api/time/toggle", target),
    stop: (options = {}) => write("/api/time/stop", options),
    adjust: (target, minutes) => write("/api/time/adjust", {...target, minutes}),
    snap: () => write("/api/time/snap", {}),
    note: (target, note) => send("POST", "/api/time/note", isObject, {...target, note}),
    setup: (person, dryRun) => send<SetupAnswer>("POST", "/api/time/setup", (b) => isObject(b) && Array.isArray(b.files), {person, dry_run: dryRun}),
    adopt: (file, dryRun) => send<AdoptAnswer>("POST", "/api/time/adopt", (b) => isObject(b) && typeof b.file === "string", {file, dry_run: dryRun}),
    declare: (account) => send("POST", "/api/time/accounts/declare", isObject, {account}),
    accountNames: async () => accountNamesOf(await send<unknown>("GET", "/api/time/accounts", isObject)),
    dayLines: async (date, person) => {
        const who = person === null ? "" : `&person=${encodeURIComponent(person)}`;
        return dayLinesOf(await send<unknown>("GET", `/api/time/entries?from=${date}&to=${date}${who}`, isObject));
    },
    savePrefs: (values) => send("PATCH", "/api/ui-prefs", isObject, {time: values}),
};
