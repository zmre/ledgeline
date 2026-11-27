// A customer page's Time accounts mounted: the linked accounts as a tree
// with this month's and the unbilled hours, linked into Entries; nothing
// before setup or with no linked account; a reload when a file changes.
import {render, screen, waitFor} from "@testing-library/svelte";
import {describe, expect, it, vi} from "vitest";
import {noteChange} from "#lib/changes.svelte.js";
import {EditorApiError} from "#lib/editorApi.js";
import type {AccountsView} from "../api/accounts.js";
import {accountsView, qty} from "../testing.js";
import CustomerTimeAccounts from "./CustomerTimeAccounts.svelte";

const props = {
    names: ["Acme Corp", "Acme Corp"],
    entriesHref: (query: string) => `/app/time/entries?${query}`,
    accountsHref: "/app/time/accounts",
    today: () => "2026-11-26",
};

describe("COMPONENT CustomerTimeAccounts", () => {
    it("lists the linked accounts with this month's and unbilled hours, linked into Entries", async () => {
        const load = vi.fn<(month: string) => Promise<AccountsView>>(() => Promise.resolve(accountsView()));
        render(CustomerTimeAccounts, {...props, load});
        await screen.findByTestId("customer-time-accounts");
        expect(load).toHaveBeenCalledWith("2026-11");
        const rows = screen.getAllByTestId("customer-time-account");
        expect(rows.map((r) => r.dataset.account)).toEqual(["clients:acme", "clients:acme:advisory", "clients:acme:tax-return"]);
        const months = screen.getAllByTestId("customer-time-month");
        expect(months.map((m) => m.textContent)).toEqual(["4h 04m", "1h 04m", "3h 40m"]);
        expect(months[0]?.getAttribute("href")).toBe("/app/time/entries?period=this-month&account=clients%3Aacme");
        const unbilled = screen.getAllByTestId("customer-time-unbilled");
        expect(unbilled[0]?.textContent).toBe("4h 04m");
        expect(unbilled[0]?.getAttribute("href")).toBe("/app/time/entries?period=all&billed=unbilled&account=clients%3Aacme");
        expect(unbilled[1]?.textContent).toBe("—");
        expect(rows[1]?.textContent).toContain("linked through clients:acme");
        expect(screen.getByTestId("customer-time-entries").getAttribute("href")).toBe("/app/time/entries?period=all&customer=Acme+Corp");
        expect(screen.getByTestId("customer-time-accounts-link").getAttribute("href")).toBe("/app/time/accounts");
    });

    it("shows nothing for a customer no account names", async () => {
        const load = vi.fn(() => Promise.resolve(accountsView()));
        const {container} = render(CustomerTimeAccounts, {...props, names: ["Globex"], load});
        await waitFor(() => expect(load).toHaveBeenCalled());
        await Promise.resolve();
        expect(screen.queryByTestId("customer-time-accounts")).toBeNull();
        expect(container.textContent?.trim()).toBe("");
    });

    it("shows nothing before time tracking is set up", async () => {
        const load = vi.fn(() => Promise.reject(new EditorApiError(409, "not-set-up", "Time tracking isn't set up")));
        const {container} = render(CustomerTimeAccounts, {...props, load});
        await waitFor(() => expect(load).toHaveBeenCalled());
        await Promise.resolve();
        expect(screen.queryByTestId("customer-time-error")).toBeNull();
        expect(container.textContent?.trim()).toBe("");
    });

    it("shows another failure with Retry", async () => {
        const load = vi.fn(() => Promise.reject(new Error("boom")));
        render(CustomerTimeAccounts, {...props, load});
        expect((await screen.findByTestId("customer-time-error")).textContent).toContain("Couldn't load the time accounts: boom");
    });

    it("loads again when a file changes", async () => {
        const later = accountsView();
        const first = later.accounts[1];
        if (first !== undefined) later.accounts[1] = {...first, month: qty(300)};
        const load = vi.fn<(month: string) => Promise<AccountsView>>().mockResolvedValueOnce(accountsView()).mockResolvedValueOnce(later);
        render(CustomerTimeAccounts, {...props, load});
        await screen.findByTestId("customer-time-accounts");
        noteChange(["time/pat/2026-11.timedot"]);
        await waitFor(() => expect(screen.getAllByTestId("customer-time-month")[0]?.textContent).toBe("5h"));
    });
});
