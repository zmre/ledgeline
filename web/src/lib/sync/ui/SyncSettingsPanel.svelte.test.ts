// Settings → Sync: the four settings, the folder check, the connections list
// with Disconnect in the plan's words — and no direct-to-bank option.

import {fireEvent, render, screen, within} from "@testing-library/svelte";
import {afterEach, describe, expect, it, vi} from "vitest";
import {fakeSyncApi, startFakeSync} from "#lib/testing/fakeSyncApi.js";

vi.mock("$app/paths", () => ({resolve: (path: string) => path}));

const {default: SyncSettingsPanel} = await import("./SyncSettingsPanel.svelte");
const {settings} = await import("#lib/stores/settings.svelte.js");

let stop: (() => void) | null = null;
afterEach(() => {
    stop?.();
    stop = null;
    settings.autoSync = false;
    settings.autoSyncEveryHours = 6;
    settings.syncSourceDir = "imports/sync";
    settings.syncKeepSourceDays = 90;
});

describe("COMPONENT SyncSettingsPanel", () => {
    it("edits the four settings, clamped, and refuses a folder outside the journal's", async () => {
        stop = await startFakeSync(fakeSyncApi());
        render(SyncSettingsPanel);
        const hours = screen.getByTestId("sync-hours") as HTMLInputElement;
        expect(hours.disabled).toBe(true);
        await fireEvent.click(screen.getByTestId("sync-auto"));
        expect(settings.autoSync).toBe(true);
        expect(hours.disabled).toBe(false);
        await fireEvent.change(hours, {target: {value: "1"}});
        expect(settings.autoSyncEveryHours).toBe(3);
        expect(hours.value).toBe("3");

        const folder = screen.getByTestId("sync-folder");
        expect(screen.getByText("Relative to your journal's folder.")).toBeTruthy();
        await fireEvent.input(folder, {target: {value: "../elsewhere"}});
        await fireEvent.change(folder);
        expect(settings.syncSourceDir).toBe("imports/sync");
        expect(screen.getByText(/Use a folder inside your journal's folder/)).toBeTruthy();
        await fireEvent.input(folder, {target: {value: "bank/feeds"}});
        await fireEvent.change(folder);
        expect(settings.syncSourceDir).toBe("bank/feeds");

        await fireEvent.change(screen.getByTestId("sync-keep"), {target: {value: "0"}});
        expect(settings.syncKeepSourceDays).toBe(0);
        expect(screen.queryByText(/direct/i)).toBeNull();
    });

    it("disconnects a connection after saying what that does", async () => {
        const api = fakeSyncApi();
        stop = await startFakeSync(api);
        render(SyncSettingsPanel);
        const row = screen.getByTestId("sync-settings-connection");
        await fireEvent.click(within(row).getByRole("button", {name: "Disconnect…"}));
        const confirm = within(row).getByTestId("sync-disconnect");
        expect(confirm.textContent).toContain(
            "Removes the access key from this Mac. Your transactions stay. To stop SimpleFIN itself sharing data, also disable the 'Ledgeline' app on SimpleFIN."
        );
        expect(within(confirm).getByRole("link", {name: "Open SimpleFIN ↗"}).getAttribute("href")).toBe("https://beta-bridge.simplefin.org/");
        await fireEvent.click(within(confirm).getByRole("button", {name: "Disconnect"}));
        expect(api.disconnect).toHaveBeenCalledWith(expect.stringMatching(/^4f6c/));
        await vi.waitFor(() => expect(screen.getByText("No connections on this computer.")).toBeTruthy());
    });
});
