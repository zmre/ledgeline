// The Accounts editor's "Clearing account" checkbox (plan 35 D2): it writes
// or removes `clearing:` on the account's own directive, and a subaccount of
// a clearing account shows the inherited state read-only. A file of its own:
// the listing is loaded once per (server, nonce), so it cannot share
// AccountsPanel.svelte.test.ts's fixture.

import {accountsStore} from "#lib/accounts/accountsStore.svelte.js";
import {settings} from "#lib/stores/settings.svelte.js";
import {connectFakeEngine, FAKE_ENGINE} from "#lib/testing/fakeEngine.js";
import {fireEvent, render, screen} from "@testing-library/svelte";
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import AccountsPanel from "./AccountsPanel.svelte";

const entry = (index: number, name: string, tags: {name: string; value: string}[]) => ({
    journalId: "accounts.journal",
    index,
    line: index + 1,
    name,
    tags,
    note: "",
});

const FILE = {
    journalId: "accounts.journal",
    label: "accounts.journal",
    revision: "rev-1",
    writable: true,
    accounts: [entry(0, "assets:checking", [{name: "type", value: "A"}]), entry(1, "transfers", [{name: "clearing", value: ""}]), entry(2, "transfers:cc", [])],
};
const LISTING = {editable: true, canCreateFile: false, createFileName: "accounts.journal", files: [FILE]};
const NAMES = ["assets:checking", "transfers", "transfers:cc", "transfers:bank"];

let saves: {method: string; url: string; body: unknown}[] = [];

beforeEach(async () => {
    await connectFakeEngine({"/api/accounts": LISTING});
    const routed = globalThis.fetch;
    saves = [];
    vi.stubGlobal("fetch", (input: RequestInfo | URL, init?: RequestInit) => {
        const url = String(input);
        if (init?.method === "PUT") {
            saves.push({method: init.method, url, body: JSON.parse(String(init.body)) as unknown});
            return Promise.resolve(new Response(JSON.stringify(FILE), {status: 200}));
        }
        return routed(input, init);
    });
    await accountsStore.ensureListing(FAKE_ENGINE, settings.serverNonce);
});

afterEach(() => vi.unstubAllGlobals());

const box = (): HTMLInputElement => screen.getByTestId("settings-account-clearing-box") as HTMLInputElement;

describe("COMPONENT AccountsPanel clearing account", () => {
    it("writes clearing: on the directive when ticked", async () => {
        render(AccountsPanel, {accountNames: NAMES});
        await fireEvent.click(screen.getByText("assets:checking"));
        expect(box().checked).toBe(false);
        expect(box().disabled).toBe(false);
        await fireEvent.click(box());
        expect(screen.getByTestId("settings-account-dirty")).toBeDefined();
        await fireEvent.click(screen.getByTestId("settings-account-save"));
        await vi.waitFor(() => expect(saves).toHaveLength(1));
        expect(saves[0]?.body).toMatchObject({
            revision: "rev-1",
            edits: [
                {
                    kind: "replace",
                    index: 0,
                    tags: [
                        {name: "type", value: "A"},
                        {name: "clearing", value: ""},
                    ],
                },
            ],
        });
    });

    it("removes it when unticked", async () => {
        render(AccountsPanel, {accountNames: NAMES});
        await fireEvent.click(screen.getByText("transfers"));
        expect(box().checked).toBe(true);
        expect(screen.queryByTestId("settings-account-clearing-inherited")).toBeNull();
        await fireEvent.click(box());
        await fireEvent.click(screen.getByTestId("settings-account-save"));
        await vi.waitFor(() => expect(saves).toHaveLength(1));
        expect(saves[0]?.body).toMatchObject({edits: [{kind: "replace", index: 1, tags: []}]});
    });

    it("shows a subaccount's inherited state read-only, declared or not", async () => {
        render(AccountsPanel, {accountNames: NAMES});
        for (const name of ["transfers:cc", "transfers:bank"]) {
            await fireEvent.click(screen.getByText(name));
            expect(box().checked).toBe(true);
            expect(box().disabled).toBe(true);
            expect(screen.getByTestId("settings-account-clearing-inherited").textContent).toContain("inherited from transfers");
        }
    });
});
