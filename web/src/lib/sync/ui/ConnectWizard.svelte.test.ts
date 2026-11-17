// The Connect-a-bank wizard end to end over a fake engine: choose, the guided
// SimpleFIN step with the live token check, a refused claim in the plan's
// words, the link step's drafts and share prompt, the preview, Finish, and
// Review now.

import {fireEvent, render, screen, within} from "@testing-library/svelte";
import {afterEach, describe, expect, it, vi} from "vitest";
import {SyncProblemError} from "#lib/api/native.js";
import {fakeSyncApi, startFakeSync, type FakeSyncApi} from "#lib/testing/fakeSyncApi.js";
import {tokenUsedProblem} from "../fixtures";

const goto = vi.fn();
vi.mock("$app/navigation", () => ({goto: (...args: unknown[]) => goto(...args)}));
vi.mock("$app/paths", () => ({resolve: (path: string) => path}));

const {default: ConnectWizard} = await import("./ConnectWizard.svelte");
const {reviewLaunch} = await import("#lib/review/ui/launch.svelte.js");
const {settings} = await import("#lib/stores/settings.svelte.js");

const BRIDGE_TOKEN = btoa("https://beta-bridge.simplefin.org/simplefin/claim/DEMO");

let stop: (() => void) | null = null;
afterEach(() => {
    stop?.();
    stop = null;
    goto.mockReset();
    reviewLaunch.close(null);
});

async function open(api: FakeSyncApi = fakeSyncApi()): Promise<{api: FakeSyncApi; onClose: ReturnType<typeof vi.fn>}> {
    stop = await startFakeSync(api);
    const onClose = vi.fn();
    render(ConnectWizard, {onClose});
    return {api, onClose};
}

const step = (): string | undefined => screen.getByTestId("sync-wizard").dataset.step;

async function toSimplefin(): Promise<void> {
    await fireEvent.click(screen.getByRole("button", {name: "Next"}));
    expect(step()).toBe("simplefin");
}

describe("COMPONENT ConnectWizard", () => {
    it("offers SimpleFIN and download-myself, and no direct option", async () => {
        await open();
        expect(step()).toBe("choose");
        expect(screen.getByText(/Through SimpleFIN Bridge/)).toBeTruthy();
        expect(screen.getByText(/I'll download statements myself/)).toBeTruthy();
        expect(screen.getAllByRole("radio")).toHaveLength(2);
        expect(screen.queryByText(/direct/i)).toBeNull();
        expect(screen.queryByRole("button", {name: "Back"})).toBeNull();
    });

    it("closes and highlights Manual downloads for 'download myself'", async () => {
        const {onClose} = await open();
        await fireEvent.click(screen.getAllByRole("radio")[1]);
        await fireEvent.click(screen.getByRole("button", {name: "Show my accounts"}));
        expect(onClose).toHaveBeenCalledWith({highlightManual: true});
    });

    it("guides through SimpleFIN and checks the token as it is pasted", async () => {
        await open();
        await toSimplefin();
        expect(screen.getByRole("link", {name: "Open SimpleFIN ↗"}).getAttribute("href")).toBe("https://beta-bridge.simplefin.org/");
        const connect = screen.getByRole("button", {name: "Connect"}) as HTMLButtonElement;
        expect(connect.disabled).toBe(true);
        const field = screen.getByTestId("sync-token");

        await fireEvent.input(field, {target: {value: btoa("http://beta-bridge.simplefin.org/simplefin/claim/X")}});
        expect(screen.getByTestId("sync-token-check").textContent).toMatch(/plain http/);
        expect(connect.disabled).toBe(true);

        await fireEvent.input(field, {target: {value: btoa("https://beta-bridge-example.com/claim/X")}});
        expect(screen.getByTestId("sync-token-check").textContent).toContain("Not SimpleFIN Bridge. Only continue if you trust beta-bridge-example.com.");
        expect(connect.disabled).toBe(false);

        await fireEvent.input(field, {target: {value: BRIDGE_TOKEN}});
        expect(screen.getByTestId("sync-token-check").textContent).toContain("Token from beta-bridge.simplefin.org");
        expect(screen.getByText(/A setup token works once/)).toBeTruthy();
    });

    it("shows a refused claim in the plan's words, with Open SimpleFIN", async () => {
        const api = fakeSyncApi();
        api.connect.mockRejectedValue(new SyncProblemError(tokenUsedProblem()));
        await open(api);
        await toSimplefin();
        await fireEvent.input(screen.getByTestId("sync-token"), {target: {value: BRIDGE_TOKEN}});
        await fireEvent.click(screen.getByRole("button", {name: "Connect"}));
        const problem = await screen.findByTestId("sync-problem");
        expect(problem.textContent).toContain("This token was already used. If you didn't use it in another app, someone else may have");
        expect(within(problem).getByRole("link", {name: "Open SimpleFIN ↗"})).toBeTruthy();
        expect(step()).toBe("simplefin");
    });

    it("links, previews, finishes, turns auto-sync on, and offers Review now", async () => {
        const {api} = await open();
        await toSimplefin();
        await fireEvent.input(screen.getByTestId("sync-token"), {target: {value: BRIDGE_TOKEN}});
        await fireEvent.click(screen.getByRole("button", {name: "Connect"}));
        await vi.waitFor(() => expect(step()).toBe("link"));
        expect(api.connect).toHaveBeenCalledWith(BRIDGE_TOKEN);
        expect(screen.getByText("SimpleFIN found 3 accounts. Choose where each one goes.")).toBeTruthy();

        const forms = screen.getAllByTestId("sync-link-form");
        expect(forms).toHaveLength(3);
        // Chase: the reversed sign is asked, and its manual rules want an answer.
        expect(within(forms[1]).getByTestId("sync-sign-question").textContent).toContain("Chase reports this as +1,032.18. Your ledger has −1,032.18.");
        expect(within(forms[1]).getByTestId("sync-share-prompt").textContent).toContain("chase.csv.rules already categorizes this account (41 rules)");
        // Ally starts as Don't sync.
        expect((within(forms[2]).getByRole("radio", {name: "Don't sync"}) as HTMLInputElement).checked).toBe(true);

        await fireEvent.click(screen.getByTestId("sync-preview"));
        expect(screen.getByRole("alert").textContent).toContain("Chase · Sapphire ••9921: Say whether to share the existing categorization rules.");
        expect(api.linkPreview).not.toHaveBeenCalled();

        // Share them: the dialog shows what moves and the proof, then marks the draft.
        await fireEvent.click(within(forms[1]).getByRole("button", {name: "Share them"}));
        const dialog = await screen.findByTestId("sync-share-dialog");
        await vi.waitFor(() => expect(within(dialog).getByTestId("sync-share-identical")).toBeTruthy());
        expect(within(dialog).getByTestId("sync-share-kept").textContent).toContain("if %trntype CHECK");
        await fireEvent.click(within(dialog).getByTestId("sync-share-confirm"));
        expect(within(forms[1]).getByText(/They'll be shared when you press Finish/)).toBeTruthy();

        await fireEvent.click(screen.getByTestId("sync-preview"));
        await vi.waitFor(() => expect(step()).toBe("preview"));
        const links = api.linkPreview.mock.calls[0][0];
        expect(links.map((l) => l.remoteId)).toEqual(["ACT-firsttech-1234", "ACT-chase-9921"]);
        expect(links[1]).toMatchObject({balanceSign: -1, share: {manualRulesId: "imports/rules/chase.csv.rules"}, rules: {mode: "existing"}});
        expect(screen.getAllByTestId("sync-preview-row")[0].textContent).toContain(
            "38 new · 0 already in your journal · 2 possible duplicates held · 6 uncategorized"
        );

        const keep = screen.getByTestId("sync-keep-up-to-date") as HTMLInputElement;
        expect(keep.checked).toBe(false);
        await fireEvent.click(keep);
        await fireEvent.click(screen.getByTestId("sync-finish"));
        await vi.waitFor(() => expect(step()).toBe("done"));
        expect(api.link).toHaveBeenCalledTimes(1);
        expect(settings.autoSync).toBe(true);
        expect(screen.getByTestId("sync-done-summary").textContent).toBe("Synced 3 accounts · 9 new transactions");

        await fireEvent.click(screen.getByTestId("sync-review-now"));
        expect(reviewLaunch.kind).toBe("recents");
        expect(goto).toHaveBeenCalledWith("/");
        settings.autoSync = false;
    });

    it("goes Back from the link step and keeps the drafts", async () => {
        await open();
        await toSimplefin();
        await fireEvent.input(screen.getByTestId("sync-token"), {target: {value: BRIDGE_TOKEN}});
        await fireEvent.click(screen.getByRole("button", {name: "Connect"}));
        await vi.waitFor(() => expect(step()).toBe("link"));
        await fireEvent.click(screen.getByRole("button", {name: "Back"}));
        expect(step()).toBe("simplefin");
    });

    it("says what to do when SimpleFIN has no banks yet, and re-checks", async () => {
        const api = fakeSyncApi();
        api.connect.mockResolvedValue({connectionId: "c1", accounts: []});
        await open(api);
        await toSimplefin();
        await fireEvent.input(screen.getByTestId("sync-token"), {target: {value: BRIDGE_TOKEN}});
        await fireEvent.click(screen.getByRole("button", {name: "Connect"}));
        await vi.waitFor(() => expect(step()).toBe("link"));
        expect(screen.getByText("SimpleFIN has no banks connected yet. Connect one on SimpleFIN, then press Re-check.")).toBeTruthy();
        await fireEvent.click(screen.getByRole("button", {name: "Re-check"}));
        expect(api.accounts).toHaveBeenCalledWith("c1");
        await vi.waitFor(() => expect(screen.getAllByTestId("sync-link-form")).toHaveLength(3));
    });
});
