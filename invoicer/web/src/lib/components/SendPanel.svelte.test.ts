// The Send panel, mounted: the prompts an export asks, the portal step, and
// the buttons this process can back.
import {fireEvent, render, screen} from "@testing-library/svelte";
import {describe, expect, it, vi} from "vitest";
import SendPanel from "./SendPanel.svelte";
import {DELIVERY_105, DELIVERY_108} from "./fixtures.js";
import type {Delivery, Exported} from "#lib/delivery.js";

function mount(delivery: Delivery, extra: {exported?: Exported | null; busy?: boolean; error?: string | null} = {}) {
    const calls = {
        onExport: vi.fn(),
        onSendAndExport: vi.fn(),
        onShare: vi.fn(),
        onCopyFile: vi.fn(),
        onOpenPortal: vi.fn(),
        onCopyText: vi.fn(),
    };
    render(SendPanel, {delivery, ...extra, ...calls});
    return calls;
}

describe("COMPONENT SendPanel", () => {
    it("exports a sent, unchanged invoice straight away", async () => {
        const calls = mount(DELIVERY_105);
        expect(screen.getByTestId("send-sent").textContent).toContain("Sent 2026-11-10 by portal");
        expect(screen.queryByTestId("send-changed")).toBeNull();
        await fireEvent.click(screen.getByTestId("send-export"));
        expect(calls.onExport).toHaveBeenCalledWith({temp: false});
        expect(screen.queryByTestId("send-prompt")).toBeNull();
    });

    it("asks a draft whether to mark it sent, by portal for a portal customer", async () => {
        const calls = mount(DELIVERY_108);
        await fireEvent.click(screen.getByTestId("send-export"));
        const prompt = screen.getByRole("alertdialog");
        expect(prompt.textContent).toContain("Mark invoice 108 as sent?");
        expect((screen.getByTestId("send-via") as HTMLSelectElement).value).toBe("portal");
        await fireEvent.click(screen.getByTestId("send-mark-and-export"));
        expect(calls.onSendAndExport).toHaveBeenCalledWith("portal");
        expect(calls.onExport).not.toHaveBeenCalled();
        expect(screen.queryByRole("alertdialog")).toBeNull();
    });

    it("lets a draft be exported as a draft, or not at all", async () => {
        const calls = mount(DELIVERY_108);
        await fireEvent.click(screen.getByTestId("send-export"));
        await fireEvent.change(screen.getByTestId("send-via"), {target: {value: "email"}});
        await fireEvent.click(screen.getByTestId("send-draft-only"));
        expect(calls.onExport).toHaveBeenCalledWith({temp: true});
        expect(calls.onSendAndExport).not.toHaveBeenCalled();

        await fireEvent.click(screen.getByTestId("send-export"));
        await fireEvent.click(screen.getByRole("button", {name: "Cancel"}));
        expect(screen.queryByRole("alertdialog")).toBeNull();
        expect(calls.onExport).toHaveBeenCalledTimes(1);
    });

    it("won't mark sent when the portal needs a reference the invoice lacks", async () => {
        const calls = mount({...DELIVERY_108, copy: {...DELIVERY_108.copy, reference: null}});
        await fireEvent.click(screen.getByTestId("send-export"));
        expect(screen.getByTestId("send-blocker").textContent).toContain("reference");
        expect((screen.getByTestId("send-mark-and-export") as HTMLButtonElement).disabled).toBe(true);
        expect(calls.onSendAndExport).not.toHaveBeenCalled();
    });

    it("warns before exporting a sent invoice that changed since its PDF was kept", async () => {
        const calls = mount({...DELIVERY_105, changed_since_sent: true});
        expect(screen.getByTestId("send-changed").textContent).toContain("will differ");
        await fireEvent.click(screen.getByTestId("send-export"));
        expect(screen.getByRole("alertdialog").textContent).toContain("changed since it was sent");
        expect(calls.onExport).not.toHaveBeenCalled();
        await fireEvent.click(screen.getByTestId("send-export-anyway"));
        expect(calls.onExport).toHaveBeenCalledWith({temp: false});
    });

    it("shows the portal's instructions, opens it, and copies its fields", async () => {
        const calls = mount(DELIVERY_105);
        expect(screen.getByTestId("send-instructions").textContent).toContain("Create from PO");
        await fireEvent.click(screen.getByTestId("send-open-portal"));
        expect(calls.onOpenPortal).toHaveBeenCalledOnce();
        await fireEvent.click(screen.getByTestId("send-copy-reference"));
        expect(calls.onCopyText).toHaveBeenCalledWith("Reference", "PO #PY75334, rev. 2");
        await fireEvent.click(screen.getByTestId("send-copy-total"));
        expect(calls.onCopyText).toHaveBeenLastCalledWith("Total", "$8,350.00");
        await fireEvent.click(screen.getByTestId("send-copy-supplier-id"));
        expect(calls.onCopyText).toHaveBeenLastCalledWith("Supplier ID", "SUP-44821");
    });

    it("shares and copies the kept PDF, or this session's export", async () => {
        const calls = mount(DELIVERY_105);
        await fireEvent.click(screen.getByTestId("send-share"));
        expect(calls.onShare).toHaveBeenCalledWith("kept", "2026-11-10 Customer X Invoice 105.pdf", expect.objectContaining({x: expect.any(Number)}));
        await fireEvent.click(screen.getByTestId("send-copy-file"));
        expect(calls.onCopyFile).toHaveBeenCalledWith("kept", "2026-11-10 Customer X Invoice 105.pdf");
        expect(screen.getAllByTestId("send-kept-pdf")).toHaveLength(2);
    });

    it("offers no Share… before a PDF exists, and the draft export once there is one", async () => {
        mount(DELIVERY_108);
        expect(screen.queryByTestId("send-share")).toBeNull();
        expect(screen.queryByTestId("send-copy-file")).toBeNull();
    });

    it("says where this session's export went, with its warnings", async () => {
        const calls = mount(DELIVERY_108, {
            exported: {
                number: "108",
                document: DELIVERY_108.document,
                destination: "temp",
                file: "2026-11-17 Customer X Invoice 108.pdf",
                bytes: 1000,
                changed_since_sent: null,
                warnings: ["the page did not finish rendering"],
            },
        });
        expect(screen.getByTestId("send-exported").textContent).toContain("temporary");
        expect(screen.getByTestId("send-exported").textContent).toContain("did not finish");
        await fireEvent.click(screen.getByTestId("send-copy-file"));
        expect(calls.onCopyFile).toHaveBeenCalledWith("temp", "2026-11-17 Customer X Invoice 108.pdf");
    });

    it("hides what this process can't do", () => {
        mount({...DELIVERY_105, capabilities: {pdf: false, share: false, copy_file: false, open_url: false}});
        expect(screen.queryByTestId("send-export")).toBeNull();
        expect(screen.getByTestId("send-no-pdf").textContent).toContain("desktop app");
        expect(screen.queryByTestId("send-share")).toBeNull();
    });

    it("shows an error and holds the buttons while busy", () => {
        mount(DELIVERY_105, {busy: true, error: "printing invoice 105: the print system said no"});
        expect(screen.getByRole("alert").textContent).toContain("the print system said no");
        expect((screen.getByTestId("send-export") as HTMLButtonElement).disabled).toBe(true);
    });
});
