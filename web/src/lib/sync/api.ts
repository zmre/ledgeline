// The bank sync routes as one typed interface (plan 43, contract "Routes"):
// what the store calls, decoded. The live implementation wraps
// `LedgelineApi`; the dev-only mock (`devMock.ts`) and component tests hand
// in their own, so nothing above this line knows which it is talking to.

import {LedgelineApi} from "#lib/api/native.js";
import {
    decodeConnectResult,
    decodeLinkPreviews,
    decodeManualAccounts,
    decodeRemoteAccounts,
    decodeSharePreview,
    decodeSyncRun,
    decodeSyncStatus,
} from "./decode";
import type {
    WireAccountPatch,
    WireConnectResult,
    WireLinkPreview,
    WireLinkRequest,
    WireManualAccount,
    WireManualUpdate,
    WireRemoteAccount,
    WireSharePreview,
    WireSyncRun,
    WireSyncStatus,
} from "./types";

export interface SyncApi {
    status(): Promise<WireSyncStatus>;
    connect(setupToken: string, name?: string): Promise<WireConnectResult>;
    replaceToken(connectionId: string, setupToken: string): Promise<WireConnectResult>;
    accounts(connectionId: string): Promise<WireRemoteAccount[]>;
    disconnect(connectionId: string): Promise<WireSyncStatus>;
    diagnostics(connectionId: string): Promise<string>;
    sharePreview(manualRulesId: string, journalAccount: string): Promise<WireSharePreview>;
    linkPreview(links: readonly WireLinkRequest[]): Promise<WireLinkPreview[]>;
    link(links: readonly WireLinkRequest[]): Promise<WireSyncRun>;
    now(target: {connectionId?: string; accountKey?: string}): Promise<WireSyncRun>;
    patchAccount(key: string, patch: WireAccountPatch): Promise<WireSyncStatus>;
    stopAccount(key: string): Promise<WireSyncStatus>;
    resolveHeld(key: string, heldId: string, action: "import" | "duplicate"): Promise<WireSyncStatus>;
    manual(): Promise<WireManualAccount[]>;
    putManual(body: WireManualUpdate): Promise<WireManualAccount[]>;
}

/** Fetching a status or listing waits this long; a sync waits on the bank, through SimpleFIN, so it may take a while. */
const SYNC_TIMEOUT_MS = 180_000;

export function liveSyncApi(serverUrl: string): SyncApi {
    const api = new LedgelineApi(serverUrl);
    // Claims, fetches and syncs reach SimpleFIN (60 s per request on the server, several chunks on a first sync).
    const slow = new LedgelineApi(serverUrl, {timeoutMs: SYNC_TIMEOUT_MS});
    return {
        status: async () => decodeSyncStatus(await api.syncStatus()),
        connect: async (token, name) => decodeConnectResult(await slow.syncConnect(token, name)),
        replaceToken: async (id, token) => decodeConnectResult(await slow.syncReplaceToken(id, token)),
        accounts: async (id) => decodeRemoteAccounts(await slow.syncConnectionAccounts(id)),
        disconnect: async (id) => decodeSyncStatus(await api.syncDisconnect(id)),
        diagnostics: (id) => api.syncDiagnostics(id),
        sharePreview: async (manualId, account) => decodeSharePreview(await slow.syncSharePreview(manualId, account)),
        linkPreview: async (links) => decodeLinkPreviews(await slow.syncLinkPreview(links)),
        link: async (links) => decodeSyncRun(await slow.syncLink(links)),
        now: async (target) => decodeSyncRun(await slow.syncNow(target)),
        patchAccount: async (key, patch) => decodeSyncStatus(await api.syncPatchAccount(key, patch)),
        stopAccount: async (key) => decodeSyncStatus(await api.syncStopAccount(key)),
        resolveHeld: async (key, heldId, action) => decodeSyncStatus(await slow.syncResolveHeld(key, heldId, action)),
        manual: async () => decodeManualAccounts(await api.syncManual()),
        putManual: async (body) => decodeManualAccounts(await api.syncPutManual(body)),
    };
}
