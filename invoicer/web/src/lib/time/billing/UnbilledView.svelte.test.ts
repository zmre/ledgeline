// The To bill view, mounted over mocked calls: the banner, a card per
// customer with Bill… per period, Needs a customer, an orphan's Release, To
// pay on its own tab, Bill all… running the Bill dialog for each customer in
// turn, and the New menu's Bill time… starting it.
import {fireEvent, render, screen, waitFor, within} from "@testing-library/svelte";
import {beforeEach, describe, expect, it, vi} from "vitest";
import {EditorApiError} from "#lib/editorApi.js";
import {listenForKeys, press} from "#lib/testing/keys.js";
import {billCreated, billPreview, riley, unbilled} from "./fixtures.js";

const api = vi.hoisted(() => ({
    unbilled: vi.fn(),
    unpaid: vi.fn(),
    release: vi.fn(),
    preview: vi.fn(),
    create: vi.fn(),
    payPreview: vi.fn(),
    options: vi.fn(),
}));
vi.mock("../api/billing.js", async (original) => ({
    ...(await original<object>()),
    fetchUnbilled: api.unbilled,
    fetchUnpaid: api.unpaid,
    releaseTime: api.release,
    previewBill: api.preview,
    createBill: api.create,
    previewPay: api.payPreview,
}));
vi.mock("#lib/editorApi.js", async (original) => ({...(await original<object>()), fetchEditorOptions: api.options}));

import UnbilledView from "./UnbilledView.svelte";

function mount(props: {tab?: "bill" | "pay"; startBillAll?: boolean} = {}) {
    const onOpenDraft = vi.fn();
    const onTab = vi.fn();
    const onBillAllStarted = vi.fn();
    const view = render(UnbilledView, {
        ...props,
        onTab,
        onBillAllStarted,
        onOpenDraft,
        draftHref: (n: string) => `/app/invoices/${n}/edit`,
        entriesHref: (q: string) => `/app/time/entries?${q}`,
        accountsHref: "/app/time/accounts",
        trackHref: "/app/time/track",
    });
    return {onOpenDraft, onTab, onBillAllStarted, view};
}

describe("COMPONENT the To bill view", () => {
    listenForKeys();

    beforeEach(() => {
        for (const fn of Object.values(api)) fn.mockReset();
        api.unbilled.mockResolvedValue(unbilled());
        api.unpaid.mockResolvedValue([riley()]);
        api.preview.mockResolvedValue(billPreview());
        api.create.mockResolvedValue(billCreated());
        api.options.mockResolvedValue({catalog: [], accounts: []});
        api.release.mockResolvedValue({done: 1, files: []});
    });

    it("sums the suggestions in the banner, and lists each customer and Needs a customer", async () => {
        mount();
        expect((await screen.findByTestId("unbilled-banner")).textContent).toContain(
            "2 customers have unbilled time through October: Acme Corp 50m · Globex 3h"
        );
        const cards = screen.getAllByTestId("unbilled-customer");
        expect(cards.map((c) => c.dataset.customer)).toEqual(["Acme Corp", "Globex"]);
        const acme = within(cards[0]);
        expect(acme.getByTestId("unbilled-customer-hours").textContent).toContain("about $1,583.33");
        expect(cards[0].textContent).toContain("Oldest Oct 6, 2026 · Billed monthly");
        expect(acme.getByTestId("unbilled-period").textContent).toContain("October 2026 · 50m · 1 entry");
        expect(acme.getByTestId("unbilled-everything")).toBeDefined();
        expect(acme.getByRole("link", {name: "See the entries"}).getAttribute("href")).toBe("/app/time/entries?period=all&customer=Acme+Corp&billed=unbilled");
        expect(screen.getByTestId("unbilled-needs-row").textContent).toContain("clients:orphan");
        expect(screen.getByTestId("unbilled-link-customer").getAttribute("href")).toBe("/app/time/accounts");
        // To pay is the other tab's.
        expect(screen.queryByTestId("unbilled-to-pay")).toBeNull();
    });

    it("has To bill and To pay tabs, each with its count; 2 and the arrows ask for the other", async () => {
        const {onTab, view} = mount();
        await screen.findByTestId("unbilled-banner");
        const bill = screen.getByRole("tab", {name: /^To bill/});
        const pay = screen.getByRole("tab", {name: /^To pay/});
        expect(screen.getByTestId("to-bill-tab-bill-count").textContent).toBe("2 customers");
        expect(screen.getByTestId("to-bill-tab-pay-count").textContent).toBe("1 person to pay");
        expect(bill.getAttribute("aria-selected")).toBe("true");
        expect(pay.getAttribute("aria-selected")).toBe("false");
        expect(screen.getByRole("tabpanel").getAttribute("aria-labelledby")).toBe("to-bill-tab-bill");
        await fireEvent.click(pay);
        expect(onTab).toHaveBeenLastCalledWith("pay");
        await press("2");
        expect(onTab).toHaveBeenCalledTimes(2);
        await fireEvent.keyDown(bill, {key: "ArrowRight"});
        expect(onTab).toHaveBeenCalledTimes(3);

        await view.rerender({tab: "pay"});
        expect(screen.getByRole("tab", {name: /^To pay/}).getAttribute("aria-selected")).toBe("true");
        expect(within(screen.getByTestId("unpaid-person")).getByTestId("unpaid-create-bill")).toBeDefined();
        expect(screen.queryByTestId("unbilled-banner")).toBeNull();
        await press("1");
        expect(onTab).toHaveBeenLastCalledWith("bill");
    });

    it("Bill… opens the dialog for that period, and the draft it makes is opened", async () => {
        const {onOpenDraft} = mount();
        const acme = (await screen.findAllByTestId("unbilled-customer"))[0];
        await fireEvent.click(within(acme).getByRole("button", {name: "Bill Acme Corp, October 2026…"}));
        await screen.findByTestId("bill-row");
        expect(api.preview).toHaveBeenCalledWith({customer: "Acme Corp", from: "2026-10-01", to: "2026-10-31", group: "account"});
        await waitFor(() => expect((screen.getByTestId("bill-dialog-submit") as HTMLButtonElement).disabled).toBe(false));
        await fireEvent.click(screen.getByTestId("bill-dialog-submit"));
        await waitFor(() => expect(onOpenDraft).toHaveBeenCalledWith(billCreated()));
        expect(screen.queryByTestId("bill-dialog")).toBeNull();
    });

    it("Bill all… (Shift+B) bills each customer in turn, Skip moves on, and lists the drafts", async () => {
        mount();
        await screen.findByTestId("unbilled-banner");
        await press("B", document.body, {shiftKey: true});
        expect((await screen.findByTestId("bill-dialog-step")).textContent).toBe("1 of 2");
        await waitFor(() => expect((screen.getByTestId("bill-dialog-submit") as HTMLButtonElement).disabled).toBe(false));
        await fireEvent.click(screen.getByTestId("bill-dialog-submit"));
        await waitFor(() => expect(screen.getByTestId("bill-dialog-step").textContent).toBe("2 of 2"));
        expect(api.preview).toHaveBeenLastCalledWith(expect.objectContaining({customer: "Globex", from: "2026-09-29", to: "2026-10-31"}));
        await fireEvent.click(screen.getByTestId("bill-dialog-skip"));
        const done = await screen.findByTestId("unbilled-run-done");
        expect(done.textContent).toContain("Made a draft:");
        expect(within(done).getByRole("link", {name: "invoice 102"}).getAttribute("href")).toBe("/app/invoices/102/edit");
        expect(api.create).toHaveBeenCalledTimes(1);
    });

    it("Bill time… (startBillAll) starts Bill all… once the lists are in, and says so", async () => {
        const {onBillAllStarted} = mount({startBillAll: true});
        expect((await screen.findByTestId("bill-dialog-step")).textContent).toBe("1 of 2");
        expect(onBillAllStarted).toHaveBeenCalledTimes(1);
    });

    it("Shift+B on To pay goes back to To bill for Bill all…", async () => {
        const {onTab} = mount({tab: "pay"});
        await screen.findByTestId("unpaid-person");
        await press("B", document.body, {shiftKey: true});
        expect(onTab).toHaveBeenLastCalledWith("bill");
        expect((await screen.findByTestId("bill-dialog-step")).textContent).toBe("1 of 2");
    });

    it("releases an orphaned invoice tag", async () => {
        api.unbilled.mockResolvedValue(unbilled({orphans: [{invoice: "104", entries: ["a@1", "a@2"], quantity: {seconds: 3600, hours: "1", text: "1h"}}]}));
        mount();
        const orphan = await screen.findByTestId("unbilled-orphan");
        expect(orphan.textContent).toContain("2 lines (1h) still say they're on invoice 104");
        await fireEvent.click(within(orphan).getByRole("button", {name: "Release invoice 104's time"}));
        await waitFor(() => expect(api.release).toHaveBeenCalledWith("104"));
        await waitFor(() => expect(api.unbilled).toHaveBeenCalledTimes(2));
    });

    it("Create bill… opens the Pay dialog for the person", async () => {
        api.payPreview.mockReturnValue(new Promise(() => undefined));
        mount({tab: "pay"});
        await fireEvent.click(await screen.findByRole("button", {name: "Create a bill for Riley Chen…"}));
        expect(screen.getByTestId("pay-dialog")).toBeDefined();
        expect(api.payPreview).toHaveBeenCalledWith({person: "riley"});
    });

    it("says so when there is nothing to bill, and when time isn't set up", async () => {
        api.unbilled.mockResolvedValue(unbilled({customers: [], needs_customer: []}));
        api.unpaid.mockResolvedValue([]);
        const {view} = mount();
        expect(await screen.findByTestId("unbilled-empty")).toBeDefined();
        expect(screen.queryByTestId("unbilled-banner")).toBeNull();
        expect(screen.getByRole("tab", {name: "To pay"})).toBeDefined();
        await view.rerender({tab: "pay"});
        expect(screen.getByTestId("unbilled-to-pay").textContent).toContain("No contractor time is waiting");
    });

    it("points to Track before time tracking is set up", async () => {
        api.unbilled.mockRejectedValue(new EditorApiError(409, "not-set-up", "time isn't set up"));
        mount();
        expect(await screen.findByTestId("unbilled-not-set-up")).toBeDefined();
        expect(screen.queryByRole("tablist")).toBeNull();
    });
});
