// The Bill dialog, mounted over mocked calls: it previews on open and again
// after each change (the edited fields only), shows the rounding and the
// timesheet, creates with ⌘↵ but never with a stray Enter, and cancels with
// Escape.
import {fireEvent, render, screen, waitFor} from "@testing-library/svelte";
import {beforeEach, describe, expect, it, vi} from "vitest";
import {listenForKeys} from "#lib/testing/keys.js";
import {billCreated, billGroup, billPreview} from "./fixtures.js";

const api = vi.hoisted(() => ({preview: vi.fn(), create: vi.fn(), options: vi.fn()}));
vi.mock("../api/billing.js", async (original) => ({...(await original<object>()), previewBill: api.preview, createBill: api.create}));
vi.mock("#lib/editorApi.js", async (original) => ({...(await original<object>()), fetchEditorOptions: api.options}));

import BillDialog from "./BillDialog.svelte";

const lastPreview = () => api.preview.mock.calls.at(-1)?.[0];

function mount(props: Record<string, unknown> = {}) {
    const onCreated = vi.fn();
    const onCancel = vi.fn();
    render(BillDialog, {customer: "Acme Corp", name: "Acme Corp", from: "2026-10-01", to: "2026-10-31", onCreated, onCancel, ...props});
    return {onCreated, onCancel};
}

describe("COMPONENT the Bill dialog", () => {
    listenForKeys();

    beforeEach(() => {
        for (const fn of Object.values(api)) fn.mockReset();
        api.preview.mockResolvedValue(billPreview());
        api.create.mockResolvedValue(billCreated());
        api.options.mockResolvedValue({
            catalog: [{code: "ARCH", description: "Senior advisory hours", unit: "h", rate: "$250", books: "revenue:consulting"}],
            accounts: ["revenue:consulting"],
        });
    });

    it("previews the period: rows, where the item came from, the rounding and the timesheet", async () => {
        mount();
        const row = await screen.findByTestId("bill-row");
        expect(api.preview).toHaveBeenCalledWith({customer: "Acme Corp", from: "2026-10-01", to: "2026-10-31", group: "account"});
        expect(screen.getByTestId("bill-dialog-period").textContent).toContain("October 2026");
        expect(row.querySelector("[data-testid=bill-row-source]")?.textContent).toBe("Set on the time account");
        expect(row.querySelector("[data-testid=bill-row-hours]")?.textContent).toMatch(/50m\s*→\s*0.90 h\s*\(\+4m\)/);
        expect((row.querySelector("[data-testid=bill-row-rate]") as HTMLInputElement).value).toBe("$250");
        expect((row.querySelector("[data-testid=bill-row-description]") as HTMLInputElement).value).toBe("Advisory, Oct 2026");
        expect(screen.getByTestId("bill-rounding").textContent).toContain("Rounded up to 6 minutes");
        expect(screen.getByTestId("bill-total-amount").textContent).toBe("$225.00");
        expect(screen.getAllByTestId("bill-timesheet-row")[0].textContent).toContain("year-end planning");
        // Focus starts on the grouping.
        expect(document.activeElement).toBe(screen.getByTestId("bill-grouping"));
    });

    it("sends what was edited and previews again; a new grouping drops the rows' choices", async () => {
        mount();
        const rate = (await screen.findByTestId("bill-row-rate")) as HTMLInputElement;
        await fireEvent.input(rate, {target: {value: "$300"}});
        await waitFor(() => expect(lastPreview()).toMatchObject({groups: [{key: "clients:acme:advisory", rate: "$300"}]}));
        await fireEvent.click(screen.getByTestId("bill-row-remember"));
        await waitFor(() => expect(lastPreview()?.groups?.[0]).toMatchObject({remember: true}));
        await fireEvent.change(screen.getByTestId("bill-detail"), {target: {value: "entries"}});
        await waitFor(() => expect(lastPreview()).toMatchObject({detail: "entries"}));
        await fireEvent.change(screen.getByTestId("bill-grouping"), {target: {value: "account-person"}});
        await waitFor(() => expect(lastPreview()).toMatchObject({group: "account-person"}));
        expect(lastPreview()?.groups).toBeUndefined();
    });

    it("leaves a row off, and says what a row needs", async () => {
        api.preview.mockResolvedValue(billPreview({groups: [billGroup({item: null, rate: null, amount: null})], ready: false, amount: null}));
        mount();
        expect((await screen.findByTestId("bill-row-need")).textContent).toBe("Choose an item or enter a rate.");
        expect((screen.getByTestId("bill-dialog-submit") as HTMLButtonElement).disabled).toBe(true);
        await fireEvent.click(screen.getByTestId("bill-row-exclude"));
        await waitFor(() => expect(lastPreview()?.groups).toEqual([{key: "clients:acme:advisory", exclude: true}]));
    });

    it("creates the draft with ⌘↵, never with Enter in a field", async () => {
        const {onCreated} = mount();
        const description = (await screen.findByTestId("bill-row-description")) as HTMLInputElement;
        await waitFor(() => expect((screen.getByTestId("bill-dialog-submit") as HTMLButtonElement).disabled).toBe(false));
        await fireEvent.keyDown(description, {key: "Enter"});
        expect(api.create).not.toHaveBeenCalled();
        await fireEvent.keyDown(description, {key: "Enter", metaKey: true});
        await waitFor(() => expect(onCreated).toHaveBeenCalledWith(billCreated()));
        expect(api.create).toHaveBeenCalledWith({customer: "Acme Corp", from: "2026-10-01", to: "2026-10-31", group: "account"});
    });

    it("bills exactly the picked lines, and Escape cancels", async () => {
        const {onCancel} = mount({from: undefined, to: undefined, entries: ["time/pat/2026-10.timedot@6"]});
        await screen.findByTestId("bill-row");
        expect(api.preview).toHaveBeenCalledWith({customer: "Acme Corp", entries: ["time/pat/2026-10.timedot@6"], group: "account"});
        expect(screen.getByTestId("bill-dialog-period").textContent).toContain("1 picked line");
        await fireEvent.keyDown(screen.getByTestId("bill-grouping"), {key: "Escape"});
        expect(onCancel).toHaveBeenCalled();
    });

    it("shows the server's refusal when the draft can't be made", async () => {
        api.create.mockRejectedValue(new Error("the catalog has no item ARCH"));
        const {onCreated} = mount();
        await waitFor(() => expect((screen.getByTestId("bill-dialog-submit") as HTMLButtonElement).disabled).toBe(false));
        await fireEvent.click(screen.getByTestId("bill-dialog-submit"));
        expect((await screen.findByTestId("bill-dialog-error")).textContent).toContain("The catalog has no item ARCH");
        expect(onCreated).not.toHaveBeenCalled();
    });
});
