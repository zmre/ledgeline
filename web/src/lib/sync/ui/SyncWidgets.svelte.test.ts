// The small bank sync pieces that sit outside the Accounts tab: the header
// dot, Sync now with its quota line and toast, and one problem's notice.

import {fireEvent, render, screen} from "@testing-library/svelte";
import {afterEach, describe, expect, it, vi} from "vitest";
import {fakeSyncApi, startFakeSync} from "#lib/testing/fakeSyncApi.js";
import {conAuthProblem, emptyStatus, fixtureRun, fixtureStatus, revokedProblem} from "../fixtures";
import {syncStore} from "../syncStore.svelte";

const goto = vi.fn();
vi.mock("$app/navigation", () => ({goto: (...args: unknown[]) => goto(...args)}));
vi.mock("$app/paths", () => ({resolve: (path: string) => path}));

const {default: SyncStatusDot} = await import("./SyncStatusDot.svelte");
const {default: SyncNowButton} = await import("./SyncNowButton.svelte");
const {default: ProblemNotice} = await import("./ProblemNotice.svelte");
const {reviewLaunch} = await import("#lib/review/ui/launch.svelte.js");

let stop: (() => void) | null = null;
afterEach(() => {
    stop?.();
    stop = null;
    goto.mockReset();
    reviewLaunch.close(null);
});

describe("COMPONENT SyncStatusDot", () => {
    it("is hidden with nothing linked", async () => {
        stop = await startFakeSync(fakeSyncApi(emptyStatus()));
        render(SyncStatusDot);
        expect(screen.queryByTestId("sync-dot")).toBeNull();
    });

    it("is hidden on an engine without sync", async () => {
        const api = fakeSyncApi();
        const {NativeApiUnavailableError} = await import("#lib/api/native.js");
        api.status.mockRejectedValue(new NativeApiUnavailableError("no"));
        stop = await startFakeSync(api);
        render(SyncStatusDot);
        expect(syncStore.unsupported).toBe(true);
        expect(screen.queryByTestId("sync-dot")).toBeNull();
    });

    it("goes amber for attention and links to Imports → Accounts", async () => {
        stop = await startFakeSync(fakeSyncApi());
        render(SyncStatusDot);
        const dot = screen.getByTestId("sync-dot");
        expect(dot.dataset.state).toBe("attention");
        expect(dot.getAttribute("href")).toBe("/imports?tab=accounts");
        expect(dot.getAttribute("aria-label")).toMatch(/^Bank sync needs attention — Auto-sync on · last synced/);
    });

    it("goes red for a broken connection and spins while syncing", async () => {
        const broken = fixtureStatus();
        broken.connections[0] = {...broken.connections[0], state: "broken", problems: [revokedProblem()]};
        stop = await startFakeSync(fakeSyncApi(broken));
        render(SyncStatusDot);
        expect(screen.getByTestId("sync-dot").dataset.state).toBe("broken");
        syncStore.setStatus({...broken, running: true});
        await vi.waitFor(() => expect(screen.getByTestId("sync-dot").dataset.state).toBe("syncing"));
    });
});

describe("COMPONENT SyncNowButton", () => {
    it("syncs on click and toasts what came in, with Review opening Review Recents", async () => {
        const api = fakeSyncApi();
        stop = await startFakeSync(api);
        render(SyncNowButton);
        await fireEvent.click(screen.getByTestId("sync-now"));
        expect(api.now).toHaveBeenCalledWith({});
        const toast = await screen.findByTestId("sync-toast");
        expect(toast.textContent).toContain("Synced 3 accounts · 9 new transactions");
        await fireEvent.click(screen.getByRole("button", {name: "Review 9"}));
        expect(reviewLaunch.kind).toBe("recents");
        expect(goto).toHaveBeenCalledWith("/");
    });

    it("warns at 12 requests and refuses at 16", async () => {
        const busy = fixtureStatus();
        busy.connections[0].requestsToday = 12;
        stop = await startFakeSync(fakeSyncApi(busy));
        const {unmount} = render(SyncNowButton);
        expect(screen.getByTestId("sync-quota").textContent).toBe("12 syncs today. SimpleFIN allows about 24 before it disables access.");
        expect((screen.getByTestId("sync-now") as HTMLButtonElement).disabled).toBe(false);
        unmount();
        syncStore.setStatus({...busy, connections: [{...busy.connections[0], requestsToday: 16}]});
        render(SyncNowButton);
        expect((screen.getByTestId("sync-now") as HTMLButtonElement).disabled).toBe(true);
    });

    it("says why a sync failed", async () => {
        const api = fakeSyncApi();
        api.now.mockResolvedValue({...fixtureRun(), ok: false, accounts: [], problems: [revokedProblem()]});
        stop = await startFakeSync(api);
        render(SyncNowButton);
        await fireEvent.click(screen.getByTestId("sync-now"));
        expect((await screen.findByTestId("sync-toast")).textContent).toContain("Sync failed: Ledgeline's access to SimpleFIN was turned off or revoked.");
    });

    it("is not offered with nothing linked", async () => {
        stop = await startFakeSync(fakeSyncApi(emptyStatus()));
        render(SyncNowButton);
        expect(screen.queryByTestId("sync-now")).toBeNull();
    });
});

describe("COMPONENT ProblemNotice", () => {
    it("shows who, the sentence, where data stops, a real link to fix it, and I've fixed it", async () => {
        const onEffect = vi.fn();
        render(ProblemNotice, {problem: conAuthProblem(), scope: {connectionId: "c", accountKey: "k"}, onEffect});
        expect(screen.getByText("Your bank")).toBeTruthy();
        expect(screen.getByTestId("sync-problem").textContent).toContain("Chase needs you to sign in again at SimpleFIN.");
        expect(screen.getByTestId("sync-problem").textContent).toContain("Data stops at Nov 11.");
        const fix = screen.getByRole("link", {name: "Fix at SimpleFIN ↗"});
        expect(fix.getAttribute("href")).toBe("https://beta-bridge.simplefin.org/");
        expect(fix.getAttribute("rel")).toContain("noopener");
        await fireEvent.click(screen.getByRole("button", {name: "I've fixed it"}));
        expect(onEffect).toHaveBeenCalledWith({type: "sync", connectionId: "c", accountKey: "k"});
        expect(screen.getByRole("link", {name: /What this means/}).getAttribute("href")).toBe("https://zmre.github.io/ledgeline/imports/bank-sync/#con-auth");
    });

    it("hands a non-link action to the host", async () => {
        const onEffect = vi.fn();
        render(ProblemNotice, {problem: revokedProblem(), scope: {connectionId: null, accountKey: null}, onEffect});
        await fireEvent.click(screen.getByRole("button", {name: "Reconnect…"}));
        expect(onEffect).toHaveBeenCalledWith({type: "reconnect", connectionId: expect.any(String)});
        expect(screen.getByRole("alert")).toBeTruthy();
    });
});
