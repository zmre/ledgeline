// DEV ONLY: a `SyncApi` over the fixtures, held in memory, so the bank sync
// screens can be built and looked at before the server (WP-S) exists.
//
// Reached only through `syncStore`'s `import.meta.env.DEV` branch, and only
// with `localStorage["ledgeline.dev.syncFixtures"] = "1"`. A production build
// never imports it. Nothing here touches the network or the journal.
//
// Tokens: any token that decodes to a claim URL connects (see wizardModel's
// `checkSetupToken`); one whose URL contains `/claim/USED` answers as an
// already-used token, so the error wording can be seen.

import {SyncProblemError} from "#lib/api/native.js";
import {
    allyRemote,
    chaseRemote,
    CONNECTION_ID,
    emptyStatus,
    firstTechRemote,
    fixtureConnection,
    fixtureManual,
    fixturePreviews,
    fixtureRun,
    fixtureSharePreview,
    fixtureStatus,
    tokenUsedProblem,
} from "./fixtures";
import type {SyncApi} from "./api";
import {decodeBase64} from "./wizardModel";
import type {WireLinkedAccount, WireSyncStatus} from "./types";

const pause = (ms: number): Promise<void> => new Promise((resolve) => setTimeout(resolve, ms));
const clone = <T>(value: T): T => structuredClone(value);

export function devMockApi(): SyncApi {
    const startEmpty = typeof localStorage !== "undefined" && localStorage.getItem("ledgeline.dev.syncFixtures.empty") === "1";
    let status: WireSyncStatus = startEmpty ? emptyStatus() : fixtureStatus();
    let manual = fixtureManual();
    const remotes = [firstTechRemote(), chaseRemote(), allyRemote()];
    const nowSeconds = (): number => Math.floor(Date.now() / 1000);

    function account(key: string): WireLinkedAccount {
        const found = status.accounts.find((a) => a.key === key);
        if (found === undefined) throw new Error(`No linked account ${key}`);
        return found;
    }

    return {
        async status() {
            await pause(150);
            return clone(status);
        },
        async connect(token) {
            await pause(800);
            if ((decodeBase64(token) ?? "").includes("/claim/USED")) throw new SyncProblemError(tokenUsedProblem());
            if (!status.connections.some((c) => c.id === CONNECTION_ID)) {
                status = {...status, connections: [{...fixtureConnection(), state: "ok", lastSuccess: null, lastAttempt: null}]};
            }
            return {connectionId: CONNECTION_ID, accounts: clone(remotes)};
        },
        async replaceToken(id) {
            await pause(800);
            status = {...status, connections: status.connections.map((c) => (c.id === id ? {...c, connected: true, problems: [], state: "ok"} : c))};
            return {connectionId: id, accounts: clone(remotes)};
        },
        async accounts() {
            await pause(500);
            return clone(remotes);
        },
        async disconnect(id) {
            status = {...status, connections: status.connections.filter((c) => c.id !== id)};
            return clone(status);
        },
        async diagnostics(id) {
            return `Ledgeline dev mock\nconnection ${id}\nhost beta-bridge.simplefin.org\naccess URL: https://[redacted]@beta-bridge.simplefin.org/simplefin\n`;
        },
        async sharePreview() {
            await pause(400);
            return fixtureSharePreview();
        },
        async linkPreview(links) {
            await pause(700);
            const previews = fixturePreviews();
            return links.map((l) => previews.find((p) => p.remoteId === l.remoteId) ?? {...previews[0], remoteId: l.remoteId});
        },
        async link(links) {
            await pause(1200);
            const base = fixtureStatus().accounts[0];
            const linked = links.map((l, i): WireLinkedAccount => ({
                ...clone(base),
                key: `dev${i}${l.remoteId.slice(-4)}`,
                remoteId: l.remoteId,
                label: l.label,
                journalAccount: l.journalAccount,
                target: l.target,
                rules: l.rules.mode === "new" ? `imports/rules/${l.rules.name}` : l.rules.id,
                startDate: l.startDate,
                balanceSign: l.balanceSign,
                held: [],
                problems: [],
                lastSuccess: nowSeconds(),
            }));
            status = {
                ...status,
                accounts: [...status.accounts.filter((a) => !links.some((l) => l.remoteId === a.remoteId)), ...linked],
                unlinked: status.unlinked.filter((u) => !links.some((l) => l.remoteId === u.remoteId)),
            };
            const run = fixtureRun();
            return {...run, accounts: linked.map((a, i) => ({...run.accounts[i % run.accounts.length], key: a.key, label: a.label}))};
        },
        async now() {
            status = {...status, running: true};
            await pause(2500);
            status = {
                ...status,
                running: false,
                connections: status.connections.map((c) => ({...c, lastSuccess: nowSeconds(), requestsToday: c.requestsToday + 1})),
            };
            return fixtureRun();
        },
        async patchAccount(key, patch) {
            const updated = {...account(key), ...patch};
            status = {...status, accounts: status.accounts.map((a) => (a.key === key ? updated : a))};
            return clone(status);
        },
        async stopAccount(key) {
            status = {...status, accounts: status.accounts.filter((a) => a.key !== key)};
            return clone(status);
        },
        async resolveHeld(key, heldId) {
            const a = account(key);
            const held = a.held.filter((h) => h.heldId !== heldId);
            const updated = {...a, held, problems: held.length === 0 ? a.problems.filter((p) => p.code !== "held") : a.problems};
            status = {...status, accounts: status.accounts.map((x) => (x.key === key ? updated : x))};
            return clone(status);
        },
        async manual() {
            await pause(150);
            return clone(manual);
        },
        async putManual(body) {
            manual = manual.map((m) => (m.account === body.account ? {...m, downloadUrl: body.downloadUrl ?? null, nudge: body.nudge} : m));
            return clone(manual);
        },
    };
}
