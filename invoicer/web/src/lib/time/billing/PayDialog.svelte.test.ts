// The Pay dialog, mounted over mocked calls: what the note lacks starts
// empty and focused, answers are previewed and saved to the note by
// default, a person with no note must pick or create one, and Create bill
// shows what was booked.
import {fireEvent, render, screen, waitFor} from "@testing-library/svelte";
import {beforeEach, describe, expect, it, vi} from "vitest";
import {listenForKeys} from "#lib/testing/keys.js";
import {payCreated, payPreview} from "./fixtures.js";

const api = vi.hoisted(() => ({preview: vi.fn(), create: vi.fn(), options: vi.fn(), dashboard: vi.fn(), open: vi.fn()}));
vi.mock("../api/billing.js", async (original) => ({...(await original<object>()), previewPay: api.preview, createPay: api.create}));
vi.mock("#lib/editorApi.js", async (original) => ({...(await original<object>()), fetchEditorOptions: api.options}));
vi.mock("#lib/api.js", async (original) => ({...(await original<object>()), fetchDashboard: api.dashboard, openInLedgeline: api.open}));

import PayDialog from "./PayDialog.svelte";

const answered = payPreview({rate: "$60", amount: "$180.00", expense_account: "expenses:contractors", needs: [], ready: true});

function mount() {
    const onBooked = vi.fn();
    const onClose = vi.fn();
    render(PayDialog, {person: "riley", name: "Riley Chen", onBooked, onClose});
    return {onBooked, onClose};
}

describe("COMPONENT the Pay dialog", () => {
    listenForKeys();

    beforeEach(() => {
        for (const fn of Object.values(api)) fn.mockReset();
        api.preview.mockResolvedValue(payPreview());
        api.options.mockResolvedValue({accounts: ["expenses:contractors", "liabilities:accounts payable"]});
        api.dashboard.mockResolvedValue({open_ledgeline: true});
        api.open.mockResolvedValue(undefined);
    });

    it("asks for what the note lacks: the rate starts empty and focused", async () => {
        mount();
        const rate = (await screen.findByTestId("pay-rate")) as HTMLInputElement;
        expect(api.preview).toHaveBeenCalledWith({person: "riley"});
        expect(rate.value).toBe("");
        await waitFor(() => expect(document.activeElement).toBe(rate));
        expect(screen.getByTestId("pay-payee").textContent).toBe("Riley Chen");
        expect((screen.getByTestId("pay-save-to-note") as HTMLInputElement).checked).toBe(true);
        expect(screen.getByTestId("pay-save-to-note").parentElement?.textContent).toContain("Save to Riley Chen's note");
        expect((screen.getByTestId("pay-dialog-submit") as HTMLButtonElement).disabled).toBe(true);
    });

    it("previews the answers, books the bill and shows what was written", async () => {
        const {onBooked} = mount();
        const rate = await screen.findByTestId("pay-rate");
        api.preview.mockResolvedValue(answered);
        await fireEvent.input(rate, {target: {value: "$60"}});
        await fireEvent.input(screen.getByRole("combobox", {name: "Expense account"}), {target: {value: "expenses:contractors"}});
        await waitFor(() =>
            expect(api.preview).toHaveBeenLastCalledWith(expect.objectContaining({rate: "$60", expense_account: "expenses:contractors", save_to_note: true}))
        );
        await waitFor(() => expect(screen.getByTestId("pay-total").textContent).toBe("$180.00"));
        api.create.mockResolvedValue(payCreated());
        await fireEvent.keyDown(rate, {key: "Enter", metaKey: true});
        await waitFor(() => expect(onBooked).toHaveBeenCalledWith(payCreated()));
        expect(api.create).toHaveBeenCalledWith(expect.objectContaining({person: "riley", rate: "$60", save_to_note: true}));
        expect(screen.getByTestId("pay-booked-reference").textContent).toBe("riley timesheet 2026-11");
        expect(screen.getByTestId("pay-booked").textContent).toContain("$180.00 owed to Riley Chen");
        expect(screen.getByTestId("pay-booked-saved").textContent).toBe("Saved the hourly rate and the expense account to Riley Chen's note.");
        expect(await screen.findByTestId("pay-open-ledgeline")).toBeDefined();
    });

    it("unchecked, the answers are for this bill only", async () => {
        mount();
        await fireEvent.click(await screen.findByTestId("pay-save-to-note"));
        await waitFor(() => expect(api.preview).toHaveBeenLastCalledWith(expect.objectContaining({save_to_note: false})));
    });

    it("with no note for the person, offers Pick a note and Create note…", async () => {
        api.preview.mockResolvedValue(
            payPreview({
                note: null,
                payee: null,
                needs: [{field: "note", message: "no note links pat's time", candidates: ["Riley Chen", "Sam Lee"]}],
            })
        );
        mount();
        expect(await screen.findByTestId("pay-note")).toBeDefined();
        await fireEvent.click(screen.getByTestId("pay-note-create"));
        const title = screen.getByTestId("pay-note-title") as HTMLInputElement;
        expect(title.value).toBe("riley");
        await fireEvent.input(title, {target: {value: "Riley Co"}});
        await fireEvent.change(screen.getByTestId("pay-note-kind"), {target: {value: "organization"}});
        await waitFor(() => expect(api.preview).toHaveBeenLastCalledWith(expect.objectContaining({create_note: {title: "Riley Co", kind: "organization"}})));
    });

    it("a bill still short of something stays open and says what", async () => {
        api.preview.mockResolvedValue(answered);
        mount();
        await waitFor(() => expect((screen.getByTestId("pay-dialog-submit") as HTMLButtonElement).disabled).toBe(false));
        api.create.mockResolvedValue(payPreview({needs: [{field: "rate", message: "riley's hourly rate", candidates: []}]}));
        await fireEvent.click(screen.getByTestId("pay-dialog-submit"));
        expect((await screen.findByTestId("pay-dialog-error")).textContent).toContain("Still needed: hourly rate.");
    });
});
