// Bank sync's problems in the header badge and the Problems drawer, as their
// own "Bank sync" group (plan 43 §4.1).

import {fireEvent, render, screen, within} from "@testing-library/svelte";
import {afterEach, describe, expect, it, vi} from "vitest";
import {fakeSyncApi, startFakeSync} from "#lib/testing/fakeSyncApi.js";

const goto = vi.fn();
vi.mock("$app/navigation", () => ({goto: (...args: unknown[]) => goto(...args)}));
vi.mock("$app/paths", () => ({resolve: (path: string) => path}));

const {default: SyncProblemsGroup} = await import("./SyncProblemsGroup.svelte");
const {default: ProblemsBadge} = await import("#lib/checks/ProblemsBadge.svelte");
const {problems} = await import("#lib/stores/problems.svelte.js");
const {syncStore} = await import("../syncStore.svelte");

let stop: (() => void) | null = null;
afterEach(() => {
    stop?.();
    stop = null;
    problems.drawerOpen = false;
});

describe("COMPONENT bank sync in Problems", () => {
    it("counts sync problems on the badge", async () => {
        stop = await startFakeSync(fakeSyncApi());
        render(ProblemsBadge);
        expect(screen.getByRole("button", {name: "2 problems"})).toBeTruthy();
    });

    it("lists them under Bank sync, each with where it is and its fix", async () => {
        stop = await startFakeSync(fakeSyncApi());
        render(SyncProblemsGroup);
        const group = screen.getByTestId("sync-problems-group");
        expect(within(group).getByRole("heading").textContent).toContain("Bank sync");
        expect(group.textContent).toContain("First Tech · Checking ••1234");
        expect(group.textContent).toContain("Chase needs you to sign in again at SimpleFIN.");

        // "Review" opens the account's drawer on the Accounts tab: a request, and a navigation there.
        problems.drawerOpen = true;
        await fireEvent.click(within(group).getByRole("button", {name: "Review"}));
        expect(problems.drawerOpen).toBe(false);
        expect(syncStore.intent).toEqual({type: "review-held", accountKey: "a1b2c3d4e5"});
        expect(goto).toHaveBeenCalledWith("/imports?tab=accounts");
    });
});
