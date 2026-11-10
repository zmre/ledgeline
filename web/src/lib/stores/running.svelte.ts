// The pinned account's running-balance facts (plan 36): `/api/journal/running`
// decoded into `RunningInfo` — the commodities the subtree holds (the pin
// chip's picker) and its balance assertions in evaluation order ("first
// failure", "narrow to gap"). Stale-response handling is `createResource`'s.
//
// Keyed on (account, generation): the answer covers the whole journal and no
// filter, so only an edit or a different pin can change it. The rows' own
// Change/Balance cells come with the rows (`journalView`), not from here.

import {EngineRefusalError, LedgelineApi} from "#lib/api/native.js";
import {decodeRunningInfo} from "#lib/api/journalViewDecode.js";
import type {RunningInfo} from "#lib/journalView/types.js";
import type {LoadStatus} from "./loadState";
import {createResource} from "./resource.svelte";

export interface RunningQuery {
    /** The pinned subtree root. */
    account: string;
    /** The `/api/journal` generation the answer must describe. */
    generation: string;
}

const resource = createResource<RunningQuery, RunningInfo>(async (serverUrl, query) =>
    decodeRunningInfo(await new LedgelineApi(serverUrl).journalRunning(query.account))
);

/** The account the newest load asked about: what `status`/`error` describe. */
let requested = $state<string | null>(null);

export const running = {
    get status(): LoadStatus {
        return resource.status;
    },
    get error(): Error | null {
        return resource.error;
    },
    /** Fetch the facts for `query`; a response superseded by a newer load is discarded. */
    load(serverUrl: string, query: RunningQuery): Promise<void> {
        requested = query.account;
        return resource.load(serverUrl, query);
    },
    /**
     * The held answer, but only when it is about `account` — a previous pin's
     * facts must never label this one. Matched on the QUERY, not on the body's
     * `account`, so an engine that normalises the name it echoes cannot make
     * the answer look foreign.
     */
    for(account: string | null): RunningInfo | null {
        return account !== null && resource.query?.account === account ? resource.value : null;
    },
    /**
     * The engine refused `account` (a 400: it matches nothing — a stale link,
     * or an account an edit renamed away). The rows route refuses it too, so
     * the journal must stop asking for the columns rather than sit on skeleton
     * rows; the chip says why.
     */
    refused(account: string | null): boolean {
        return account !== null && requested === account && resource.status === "error" && resource.error instanceof EngineRefusalError;
    },
};
