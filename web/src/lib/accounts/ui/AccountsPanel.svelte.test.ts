// Mounting the Settings → Accounts tab — the same reason `AliasPanel.svelte.test.ts`
// mounts its own screen: a seeding effect latched on an object ($state<AccountEntry
// | null>) is exactly the shape that once produced a self-feeding effect and froze
// the whole app (see `routes/effectLatch.test.ts`). A pure-function test cannot make
// that claim; mounting the component can.

import {accountsStore} from "$lib/accounts/accountsStore.svelte";
import {settings} from "$lib/stores/settings.svelte";
import {connectFakeEngine, FAKE_ENGINE} from "$lib/testing/fakeEngine";
import {fireEvent, render, screen} from "@testing-library/svelte";
import {flushSync} from "svelte";
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import AccountsPanel from "./AccountsPanel.svelte";

const ACCOUNTS = {
    editable: true,
    canCreateFile: false,
    createFileName: "accounts.journal",
    files: [
        {
            journalId: "main.journal",
            label: "main.journal",
            revision: "rev-1",
            writable: true,
            accounts: [
                {journalId: "main.journal", index: 0, line: 2, name: "assets:bank:checking", tags: [{name: "type", value: "A"}], note: ""},
                {journalId: "main.journal", index: 1, line: 3, name: "expenses:food", tags: [], note: "groceries"},
            ],
        },
    ],
};

const ACCOUNT_NAMES = ["assets:bank:checking", "expenses:food", "expenses:not-yet-declared"];

beforeEach(async () => {
    await connectFakeEngine({"/api/accounts": ACCOUNTS});
    // Loaded through `ensureListing`, not `reload`, so the panel's own
    // `onServerReady` finds the same (nonce, url) key already served and does
    // not fire a second, asynchronous load underneath the assertions.
    await accountsStore.ensureListing(FAKE_ENGINE, settings.serverNonce);
});

afterEach(() => vi.unstubAllGlobals());

describe("COMPONENT AccountsPanel", () => {
    it("mounts without a self-feeding effect", () => {
        expect(() => {
            render(AccountsPanel, {accountNames: ACCOUNT_NAMES});
            flushSync();
        }).not.toThrow();
    });

    it("lists every known account, declared or not", () => {
        render(AccountsPanel, {accountNames: ACCOUNT_NAMES});

        const text = screen.getAllByTestId("settings-account-row").map((row) => row.textContent ?? "");
        for (const name of ["assets:bank:checking", "expenses:food", "expenses:not-yet-declared"]) {
            expect(text.some((row) => row.includes(name))).toBe(true);
        }
    });

    it("seeds the editor from a declared account, type pulled out of its tags", async () => {
        render(AccountsPanel, {accountNames: ACCOUNT_NAMES});

        await fireEvent.click(screen.getByText("assets:bank:checking"));

        const editor = screen.getByTestId("settings-account-editor");
        expect(editor.textContent).toContain("assets:bank:checking");
        expect(screen.getByDisplayValue("Asset")).toBeDefined();
    });

    it("offers a declare affordance for an undeclared account, and keeps typing across a re-render", async () => {
        render(AccountsPanel, {accountNames: ACCOUNT_NAMES});

        await fireEvent.click(screen.getByText("expenses:not-yet-declared"));
        expect(screen.getByText("not yet declared")).toBeDefined();

        await fireEvent.click(screen.getByTestId("settings-account-add-tag"));
        const nameField = screen.getByPlaceholderText("name") as HTMLInputElement;
        await fireEvent.input(nameField, {target: {value: "priority"}});

        // A re-render (the same reactive tick a poll or an unrelated prop
        // change would cause) must not clobber the row the user is mid-edit
        // on — the exact failure mode a self-feeding or over-eager seeding
        // effect produces.
        flushSync();
        expect((screen.getByPlaceholderText("name") as HTMLInputElement).value).toBe("priority");
        expect(screen.getByTestId("settings-account-dirty")).toBeDefined();
    });

    it("renders every special tag as its own field, with inline help", async () => {
        render(AccountsPanel, {accountNames: ACCOUNT_NAMES});

        await fireEvent.click(screen.getByText("assets:bank:checking"));

        expect(screen.getByLabelText("Type")).toBeDefined();
        expect(screen.getByLabelText("Income statement section")).toBeDefined();
        expect(screen.getByLabelText("Holdings tab")).toBeDefined();
        expect(screen.getByLabelText("Balance sheet line")).toBeDefined();
        expect(screen.getAllByLabelText(/^About /).length).toBe(7);
    });

    it("lets a brand-new name (known to nothing else) be added and edited", async () => {
        render(AccountsPanel, {accountNames: ACCOUNT_NAMES});

        const nameField = screen.getByTestId("settings-accounts-new-name") as HTMLInputElement;
        await fireEvent.input(nameField, {target: {value: "assets:brand-new"}});
        await fireEvent.click(screen.getByTestId("settings-accounts-add"));

        const editor = screen.getByTestId("settings-account-editor");
        expect(editor.textContent).toContain("assets:brand-new");
        expect(screen.getByText("not yet declared")).toBeDefined();
        // The field cleared, ready for the next one.
        expect(nameField.value).toBe("");
    });

    it("stages a removal behind Save rather than deleting immediately", async () => {
        render(AccountsPanel, {accountNames: ACCOUNT_NAMES});

        await fireEvent.click(screen.getByText("assets:bank:checking"));
        expect(screen.queryByTestId("settings-account-removal-warning")).toBeNull();

        await fireEvent.click(screen.getByTestId("settings-account-remove-toggle"));

        expect(screen.getByTestId("settings-account-removal-warning")).toBeDefined();
        expect(screen.getByTestId("settings-account-dirty")).toBeDefined();
        expect(screen.getByText("Keep account")).toBeDefined();

        // Toggling back off unstages it without anything having been saved.
        await fireEvent.click(screen.getByTestId("settings-account-remove-toggle"));
        expect(screen.queryByTestId("settings-account-removal-warning")).toBeNull();
    });

    it("refuses an ordinary tag row named after a special tag", async () => {
        render(AccountsPanel, {accountNames: ACCOUNT_NAMES});

        await fireEvent.click(screen.getByText("expenses:not-yet-declared"));
        await fireEvent.click(screen.getByTestId("settings-account-add-tag"));
        await fireEvent.input(screen.getByPlaceholderText("name"), {target: {value: "bsgroup"}});
        await fireEvent.input(screen.getByPlaceholderText("value"), {target: {value: "x"}});

        await fireEvent.click(screen.getByTestId("settings-account-save"));

        expect(screen.getByTestId("settings-account-client-errors").textContent).toContain("special tag");
    });
});
