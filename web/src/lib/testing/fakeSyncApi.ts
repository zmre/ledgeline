// A bank sync `SyncApi` (plan 43) for component tests: every route answers at
// once from the typed fixtures, each one a `vi.fn` so a test can both change
// an answer and see what it was asked. Hand it to the real store with
// `syncStore.useApi(() => api)` and `syncStore.start(…)`.

import {vi} from "vitest";
import type {SyncApi} from "#lib/sync/api.js";
import {
    allyRemote,
    chaseRemote,
    CONNECTION_ID,
    firstTechRemote,
    fixtureManual,
    fixturePreviews,
    fixtureRun,
    fixtureSharePreview,
    fixtureStatus,
} from "#lib/sync/fixtures.js";
import {syncStore} from "#lib/sync/syncStore.svelte.js";
import type {WireSyncStatus} from "#lib/sync/types.js";

export type FakeSyncApi = {[K in keyof SyncApi]: ReturnType<typeof vi.fn<SyncApi[K]>>};

export function fakeSyncApi(status: WireSyncStatus = fixtureStatus()): FakeSyncApi {
    return {
        status: vi.fn<SyncApi["status"]>(async () => status),
        connect: vi.fn<SyncApi["connect"]>(async () => ({connectionId: CONNECTION_ID, accounts: [firstTechRemote(), chaseRemote(), allyRemote()]})),
        replaceToken: vi.fn<SyncApi["replaceToken"]>(async () => ({connectionId: CONNECTION_ID, accounts: [firstTechRemote()]})),
        accounts: vi.fn<SyncApi["accounts"]>(async () => [firstTechRemote(), chaseRemote(), allyRemote()]),
        disconnect: vi.fn<SyncApi["disconnect"]>(async () => ({...status, connections: []})),
        diagnostics: vi.fn<SyncApi["diagnostics"]>(async () => "report"),
        sharePreview: vi.fn<SyncApi["sharePreview"]>(async () => fixtureSharePreview()),
        linkPreview: vi.fn<SyncApi["linkPreview"]>(async () => fixturePreviews()),
        link: vi.fn<SyncApi["link"]>(async () => fixtureRun()),
        now: vi.fn<SyncApi["now"]>(async () => fixtureRun()),
        patchAccount: vi.fn<SyncApi["patchAccount"]>(async () => status),
        stopAccount: vi.fn<SyncApi["stopAccount"]>(async () => status),
        resolveHeld: vi.fn<SyncApi["resolveHeld"]>(async () => status),
        manual: vi.fn<SyncApi["manual"]>(async () => fixtureManual()),
        putManual: vi.fn<SyncApi["putManual"]>(async () => fixtureManual()),
    };
}

/** Start the real store over `api` and wait for its first status. Returns the stop function. */
export async function startFakeSync(api: FakeSyncApi): Promise<() => void> {
    syncStore.reset();
    syncStore.useApi(() => api);
    const stop = syncStore.start("http://engine.test");
    await vi.waitFor(() => {
        if (syncStore.status === null && !syncStore.unsupported) throw new Error("no status yet");
    });
    return () => {
        stop();
        syncStore.useApi(null);
        syncStore.reset();
    };
}
