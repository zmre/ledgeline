// The Entries and Accounts pages, mounted over mocked calls: a URL opened
// directly asks the server for that view, `a` adds, a write is followed by
// a fresh list, and books with no time files say so instead of failing.
import {fireEvent, render, screen, waitFor} from "@testing-library/svelte";
import {beforeEach, describe, expect, it, vi} from "vitest";
import {EditorApiError} from "#lib/editorApi.js";
import {listenForKeys, press} from "#lib/testing/keys.js";
import {billPreview} from "#lib/time/billing/fixtures.js";
import {accountsView, entry, list, lockedEntry} from "#lib/time/testing.js";

const nav = vi.hoisted(() => ({url: new URL("http://localhost/app/time/entries"), goto: vi.fn()}));
vi.mock("$app/state", () => ({
    page: {
        get url() {
            return nav.url;
        },
        params: {},
    },
}));
vi.mock("$app/navigation", () => ({goto: nav.goto, beforeNavigate: vi.fn()}));
vi.mock("$app/paths", () => ({resolve: (path: string, params: Record<string, string> = {}) => `/app${path.replace("[number]", params.number ?? "")}`}));

const api = vi.hoisted(() => ({
    entries: vi.fn(),
    add: vi.fn(),
    accounts: vi.fn(),
    rename: vi.fn(),
    customers: vi.fn(),
    options: vi.fn(),
    billPreview: vi.fn(),
    releaseBill: vi.fn(),
}));
vi.mock("#lib/time/api/entries.js", async (original) => ({...(await original<object>()), fetchEntries: api.entries, addEntry: api.add}));
vi.mock("#lib/time/api/accounts.js", async (original) => ({...(await original<object>()), fetchAccounts: api.accounts, renameAccount: api.rename}));
vi.mock("#lib/time/api/billing.js", async (original) => ({...(await original<object>()), previewBill: api.billPreview, releaseBill: api.releaseBill}));
vi.mock("#lib/customers.js", async (original) => ({...(await original<object>()), fetchCustomers: api.customers}));
vi.mock("#lib/editorApi.js", async (original) => ({...(await original<object>()), fetchEditorOptions: api.options}));
vi.mock("#lib/invoices.js", async (original) => ({...(await original<object>()), localToday: () => "2026-11-26"}));

import EntriesPage from "./entries/+page.svelte";
import AccountsPage from "./accounts/+page.svelte";

const WRITTEN = {done: "time/pat/2026-11.timedot", files: ["time/pat/2026-11.timedot"], retried: false, included: null};

describe("COMPONENT time pages", () => {
    listenForKeys();

    beforeEach(() => {
        for (const fn of Object.values(api)) fn.mockReset();
        api.entries.mockResolvedValue(list([entry(), lockedEntry()]));
        api.accounts.mockResolvedValue(accountsView());
        api.customers.mockResolvedValue([]);
        api.options.mockResolvedValue({customers: [], catalog: [{code: "ARCH", description: "Architecture", unit: null, rate: null, books: null}]});
        api.add.mockResolvedValue(WRITTEN);
        nav.goto.mockReset();
    });

    it("Entries asks for the URL's view and lists it by day", async () => {
        nav.url = new URL("http://localhost/app/time/entries?period=last-month&customer=Acme+Corp&billed=unbilled");
        render(EntriesPage);
        await waitFor(() => expect(api.entries).toHaveBeenCalledWith("from=2026-10-01&to=2026-10-31&customer=Acme+Corp&billed=unbilled"));
        expect((await screen.findAllByTestId("entry-row")).length).toBe(2);
        expect(screen.getByTestId("entries-count").textContent).toContain("2 entries");
        expect((screen.getByTestId("entry-filter-period") as HTMLSelectElement).value).toBe("last-month");
    });

    it("the week view asks for its week and shows the grid", async () => {
        nav.url = new URL("http://localhost/app/time/entries?group=week&week=2026-11-23");
        render(EntriesPage);
        await waitFor(() => expect(api.entries).toHaveBeenCalledWith("from=2026-11-23&to=2026-11-29"));
        expect(await screen.findByTestId("week-grid")).toBeDefined();
        expect(screen.getByTestId("week-label").textContent).toBe("Week of Nov 23, 2026");
        await fireEvent.click(screen.getByRole("button", {name: "Previous week"}));
        expect(nav.goto).toHaveBeenCalledWith("/app/time/entries?group=week&week=2026-11-16", expect.anything());
    });

    it("`a` adds a line, then the list is fetched again", async () => {
        nav.url = new URL("http://localhost/app/time/entries");
        render(EntriesPage);
        await screen.findAllByTestId("entry-row");
        await press("a");
        const form = await screen.findByTestId("entry-add");
        expect(form).toBeDefined();
        await fireEvent.input(screen.getAllByRole("combobox", {name: "Account"}).at(-1) as HTMLElement, {target: {value: "internal:admin"}});
        await fireEvent.input(screen.getByTestId("entry-add-duration"), {target: {value: "45m"}});
        await fireEvent.click(screen.getByTestId("entry-add-save"));
        await waitFor(() => expect(api.add).toHaveBeenCalledWith({date: "2026-11-26", account: "internal:admin", duration: "45m", note: ""}));
        await waitFor(() => expect(api.entries).toHaveBeenCalledTimes(2));
        await waitFor(() => expect(screen.queryByTestId("entry-add")).toBeNull());
    });

    it("`b` bills the picked lines when they are one customer's unbilled time", async () => {
        api.billPreview.mockResolvedValue(billPreview());
        nav.url = new URL("http://localhost/app/time/entries");
        render(EntriesPage);
        const rows = await screen.findAllByTestId("entry-row");
        await fireEvent.click(rows[0].querySelector("input[type=checkbox]") as HTMLElement);
        await press("b");
        expect(await screen.findByTestId("bill-dialog")).toBeDefined();
        expect(api.billPreview).toHaveBeenCalledWith({customer: "Acme Corp", entries: ["time/pat/2026-11.timedot@20"], group: "account"});
    });

    it("Bill… refuses picked time for two customers, in words", async () => {
        api.entries.mockResolvedValue(list([entry(), entry({id: "time/pat/2026-11.timedot@21", customer: "Globex", account: "clients:globex:audit"})]));
        nav.url = new URL("http://localhost/app/time/entries");
        render(EntriesPage);
        const rows = await screen.findAllByTestId("entry-row");
        for (const row of rows) await fireEvent.click(row.querySelector("input[type=checkbox]") as HTMLElement);
        await fireEvent.click(screen.getByTestId("bulk-bill"));
        expect(screen.getByTestId("bulk-error").textContent).toContain("for 2 customers (Acme Corp, Globex)");
        expect(screen.queryByTestId("bill-dialog")).toBeNull();
    });

    it("a line on a booked bill offers Void bill and release time, after a confirmation", async () => {
        api.entries.mockResolvedValue(list([lockedEntry({invoice: null, bill: "sam timesheet 2026-11", locked: "it is on the bill sam timesheet 2026-11"})]));
        api.releaseBill.mockResolvedValue({done: 1, files: []});
        nav.url = new URL("http://localhost/app/time/entries");
        render(EntriesPage);
        const [row] = await screen.findAllByTestId("entry-row");
        await fireEvent.click(row.querySelector("[data-testid=entry-edit]") as HTMLElement);
        await fireEvent.click(await screen.findByTestId("entry-release-bill"));
        await fireEvent.click(screen.getByTestId("entry-release-bill-dialog-confirm"));
        await waitFor(() => expect(api.releaseBill).toHaveBeenCalledWith("sam timesheet 2026-11"));
        await waitFor(() => expect(api.entries).toHaveBeenCalledTimes(2));
    });

    it("books with no time files say so", async () => {
        api.entries.mockRejectedValue(new EditorApiError(409, "not-set-up", "time isn't set up"));
        api.accounts.mockRejectedValue(new EditorApiError(409, "not-set-up", "time isn't set up"));
        nav.url = new URL("http://localhost/app/time/entries");
        render(EntriesPage);
        expect(await screen.findByTestId("entries-not-set-up")).toBeDefined();
    });

    it("Accounts shows the tree for the URL's month and the undeclared accounts", async () => {
        nav.url = new URL("http://localhost/app/time/accounts?month=2026-10");
        render(AccountsPage);
        await waitFor(() => expect(api.accounts).toHaveBeenCalledWith("2026-10"));
        expect((await screen.findAllByTestId("account-row")).length).toBe(6);
        expect(screen.getByTestId("accounts-month-label").textContent).toBe("October 2026");
        expect(screen.getByTestId("accounts-undeclared-row").dataset.account).toBe("internal:training");
    });

    it("Rename previews, then applies and fetches the tree again", async () => {
        nav.url = new URL("http://localhost/app/time/accounts");
        api.rename.mockImplementation(async (from: string, to: string, apply: boolean) => ({
            ...WRITTEN,
            done: {
                from,
                to,
                lines: [{file: "time-accounts.journal", line: 16, before: `account ${from}`, after: `account ${to}`}],
                files: ["time-accounts.journal"],
                applied: apply,
            },
        }));
        render(AccountsPage);
        const rows = await screen.findAllByTestId("account-row");
        await fireEvent.click(rows.find((r) => r.dataset.account === "internal")?.querySelector("[data-testid=account-rename]") as HTMLElement);
        await fireEvent.input(screen.getByTestId("account-rename-to"), {target: {value: "overhead"}});
        await fireEvent.click(screen.getByTestId("account-rename-dialog-submit"));
        await screen.findByTestId("account-rename-summary");
        expect(api.rename).toHaveBeenLastCalledWith("internal", "overhead", false);
        await fireEvent.click(screen.getByTestId("account-rename-dialog-submit"));
        await waitFor(() => expect(api.rename).toHaveBeenLastCalledWith("internal", "overhead", true));
        await waitFor(() => expect(screen.queryByTestId("account-rename-dialog")).toBeNull());
        expect(api.accounts).toHaveBeenCalledTimes(2);
    });
});
