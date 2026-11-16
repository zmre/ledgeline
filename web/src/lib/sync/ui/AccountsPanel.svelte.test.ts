// Imports → Accounts over the fixtures: the status line, the connection card
// with its rows, the unlinked account, Manual downloads, and the account
// drawer's held list and Stop syncing.

import {fireEvent, render, screen, within} from "@testing-library/svelte";
import {afterEach, describe, expect, it, vi} from "vitest";
import {fakeSyncApi, startFakeSync, type FakeSyncApi} from "#lib/testing/fakeSyncApi.js";
import {NativeApiUnavailableError} from "#lib/api/native.js";
import {emptyStatus, fixtureStatus, FIXTURE_NOW} from "../fixtures";
import {syncStore} from "../syncStore.svelte";

vi.mock("$app/navigation", () => ({goto: vi.fn()}));
vi.mock("$app/paths", () => ({resolve: (path: string) => path}));

const {default: AccountsPanel} = await import("./AccountsPanel.svelte");

let stop: (() => void) | null = null;
afterEach(() => {
    stop?.();
    stop = null;
    vi.useRealTimers();
});

async function show(api: FakeSyncApi = fakeSyncApi()): Promise<FakeSyncApi> {
    vi.useFakeTimers({toFake: ["Date"], now: FIXTURE_NOW * 1000});
    stop = await startFakeSync(api);
    render(AccountsPanel);
    await vi.waitFor(() => expect(syncStore.manual).not.toBeNull());
    return api;
}

describe("COMPONENT AccountsPanel", () => {
    it("lays out the connection, its accounts, the unlinked one and manual downloads", async () => {
        await show();
        expect(screen.getByTestId("sync-status-line").textContent).toMatch(/^Auto-sync on · last synced 7:12\sAM · next around 1:15\sPM$/);
        const card = screen.getByTestId("sync-connection");
        expect(within(card).getByRole("heading", {name: "SimpleFIN Bridge"})).toBeTruthy();
        expect(within(card).getByText("Needs attention")).toBeTruthy();
        const rows = within(card).getAllByTestId("sync-linked-row");
        expect(rows).toHaveLength(2);
        expect(rows[0].textContent).toContain("→ assets:bank:firsttech:checking");
        expect(within(rows[0]).getByTestId("sync-linked-line").textContent).toMatch(/4 new · 2 to review · balance matches ✓/);
        expect(within(rows[1]).getByTestId("sync-problem").dataset.code).toBe("con-auth");
        expect(within(card).getByTestId("sync-unlinked-row").textContent).toContain("Ally · Savings ••0077");
        const manual = screen.getAllByTestId("sync-manual-row");
        expect(manual[0].dataset.stale).toBe("true");
        expect(within(manual[0]).getByRole("link", {name: "Open download page ↗"})).toBeTruthy();
        expect(screen.getByTestId("sync-connect")).toBeTruthy();
    });

    it("invites a first connection when nothing is linked", async () => {
        await show(fakeSyncApi(emptyStatus()));
        expect(screen.getByText(/Link a bank account and Ledgeline brings in its new transactions/)).toBeTruthy();
        expect(screen.queryByTestId("sync-now")).toBeNull();
    });

    it("says an engine without sync can't do it, and offers nothing", async () => {
        const api = fakeSyncApi();
        api.status.mockRejectedValue(new NativeApiUnavailableError("no"));
        vi.useFakeTimers({toFake: ["Date"], now: FIXTURE_NOW * 1000});
        stop = await startFakeSync(api);
        render(AccountsPanel);
        expect(screen.getByText(/This Ledgeline engine can't sync with banks/)).toBeTruthy();
        expect(screen.queryByTestId("sync-connect")).toBeNull();
    });

    it("opens the drawer, answers a held row, and stops syncing after asking", async () => {
        const api = await show();
        await fireEvent.click(screen.getByRole("button", {name: "First Tech · Checking ••1234"}));
        const drawer = screen.getByTestId("sync-account-drawer");
        expect(within(drawer).getAllByTestId("sync-held-row")).toHaveLength(2);
        await fireEvent.click(within(drawer).getAllByRole("button", {name: "It's a duplicate"})[0]);
        expect(api.resolveHeld).toHaveBeenCalledWith("a1b2c3d4e5", "a1b2c3d4e5-TRN-0007", "duplicate");
        await fireEvent.click(within(drawer).getByRole("button", {name: "Stop syncing…"}));
        await fireEvent.click(within(drawer).getByRole("button", {name: "Stop syncing"}));
        expect(api.stopAccount).toHaveBeenCalledWith("a1b2c3d4e5");
        await vi.waitFor(() => expect(screen.queryByTestId("sync-account-drawer")).toBeNull());
    });

    it("changes the balance sign with one PATCH", async () => {
        const api = await show();
        await fireEvent.click(screen.getByRole("button", {name: "Review"}));
        const drawer = screen.getByTestId("sync-account-drawer");
        await fireEvent.change(within(drawer).getByRole("combobox", {name: "Balance sign"}), {target: {value: "-1"}});
        expect(api.patchAccount).toHaveBeenCalledWith("a1b2c3d4e5", {balanceSign: -1});
    });

    it("reads quietly on a computer that another one syncs for, and offers to sync here too", async () => {
        const elsewhere = fixtureStatus();
        elsewhere.connections = elsewhere.connections.map((c) => ({...c, state: "elsewhere" as const, connected: false, problems: []}));
        elsewhere.accounts = elsewhere.accounts.map((a) => ({...a, problems: []}));
        await show(fakeSyncApi(elsewhere));
        expect(screen.getByTestId("sync-status-line").textContent).toBe("Bank sync runs on another computer");
        expect(screen.queryByTestId("sync-now")).toBeNull();
        const card = screen.getByTestId("sync-connection");
        expect(within(card).getByText("Syncs on another computer")).toBeTruthy();
        expect(within(card).getAllByTestId("sync-linked-line")[0].textContent).toBe("Synced on another computer");
        expect(within(card).queryByTestId("sync-problem")).toBeNull();
        await fireEvent.click(within(screen.getByTestId("sync-elsewhere")).getByRole("button", {name: "Sync from this computer too…"}));
        const wizard = screen.getByTestId("sync-wizard");
        expect(within(wizard).getByRole("heading", {name: "Sync from this computer too"})).toBeTruthy();
        expect(within(wizard).getByTestId("sync-also-here").textContent).toMatch(/one computer only/);
    });

    it("opens the wizard on the link step for an unlinked account", async () => {
        await show();
        await fireEvent.click(screen.getByRole("button", {name: "Link…"}));
        const wizard = screen.getByTestId("sync-wizard");
        expect(wizard.dataset.step).toBe("link");
        expect(within(wizard).getAllByTestId("sync-link-form")).toHaveLength(1);
    });

    it("takes a request from the Problems drawer when it mounts", async () => {
        syncStore.reset();
        const api = fakeSyncApi();
        vi.useFakeTimers({toFake: ["Date"], now: FIXTURE_NOW * 1000});
        stop = await startFakeSync(api);
        syncStore.request({type: "review-held", accountKey: "a1b2c3d4e5"});
        render(AccountsPanel);
        await vi.waitFor(() => expect(screen.getByTestId("sync-account-drawer")).toBeTruthy());
        expect(syncStore.intent).toBeNull();
    });
});
