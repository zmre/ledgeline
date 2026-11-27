// The tracker's API calls (plan 46 Phase 3): `GET /api/time/state` (this is
// its one reader: Track, the lamp, the dashboard and billing all come here),
// the tracker's writes and setup. The reads Track needs from elsewhere (the
// account names for Other…, today's lines for the day card) go through those
// endpoints' own modules, ./accounts.ts and ./entries.ts.
// Same-origin and token-gated like lib/api.ts. A refusal is an EditorApiError
// carrying the server's `code` (`not-set-up`, `note-tag`, `conflict`,
// `external-change`) and its whole body (`offending`, `suggestion`).
//
// `TrackApi` is the seam the tracker store takes, so its tests pass their own.
import {send} from "#lib/editorApi.js";
import type {TrackState} from "#lib/time/types.js";
import {declareAccount, fetchAccounts, type AccountsView} from "./accounts.js";
import {fetchEntries, type EntryList} from "./entries.js";

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
    /** `commodity` lines `hledger -s` needs that the file lacks, for the person to add (fact 10). */
    missing?: string[];
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

/** The declared account names the tree lists, in its order, archived ones left out. */
export function accountNamesOf(view: Pick<AccountsView, "accounts">): string[] {
    return view.accounts.filter((row) => row.archived === null || !/^(yes|y|true|1)$/i.test(row.archived.value)).map((row) => row.account);
}

/** A list's lines as account, seconds and note (the day card's). */
export function dayLinesOf(list: Pick<EntryList, "entries">): DayLine[] {
    return list.entries.map((row) => ({account: row.account, seconds: row.quantity.seconds, note: row.note}));
}

/** `GET /api/time/state`: the tracker, or `{configured: false, setup}` before setup. */
export function fetchTrackState(): Promise<TrackState> {
    return send<TrackState>("GET", "/api/time/state", isTrackState);
}

/** The server's API. */
export const httpTrackApi: TrackApi = {
    state: fetchTrackState,
    toggle: (target) => write("/api/time/toggle", target),
    stop: (options = {}) => write("/api/time/stop", options),
    adjust: (target, minutes) => write("/api/time/adjust", {...target, minutes}),
    snap: () => write("/api/time/snap", {}),
    note: (target, note) => send("POST", "/api/time/note", isObject, {...target, note}),
    setup: (person, dryRun) => send<SetupAnswer>("POST", "/api/time/setup", (b) => isObject(b) && Array.isArray(b.files), {person, dry_run: dryRun}),
    adopt: (file, dryRun) => send<AdoptAnswer>("POST", "/api/time/adopt", (b) => isObject(b) && typeof b.file === "string", {file, dry_run: dryRun}),
    declare: (account) => declareAccount(account),
    accountNames: async () => accountNamesOf(await fetchAccounts()),
    dayLines: async (date, person) => {
        const who = person === null ? "" : `&person=${encodeURIComponent(person)}`;
        return dayLinesOf(await fetchEntries(`from=${date}&to=${date}${who}`));
    },
    savePrefs: (values) => send("PATCH", "/api/ui-prefs", isObject, {time: values}),
};
