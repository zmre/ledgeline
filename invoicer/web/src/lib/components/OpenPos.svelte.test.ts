// A customer's open POs in the view (PLAN 5b, S5–S6), mounted: what each has
// left, and Add, Edit and Close each saving the whole list at once.
import {fireEvent, render, screen, within} from "@testing-library/svelte";
import {describe, expect, it, vi} from "vitest";
import type {PoForm, PoUsage} from "#lib/customers.js";
import OpenPos from "./OpenPos.svelte";

const USAGE: PoUsage[] = [
    {
        reference: "PO #PY75334",
        amount: "$40,000",
        expires: "2027-06-30",
        expired: false,
        invoices: ["104"],
        invoiced: "$10,000.00",
        remaining: "$30,000.00",
    },
    {reference: "PO #9", amount: null, expires: "2026-01-31", expired: true, invoices: [], invoiced: null, remaining: null},
];
const invoiceHref = (n: string) => `/app/invoices/${n}`;
const hrefs = {
    invoiceHref,
    noteHref: (path: string) => `/app/notes/${path.replace(/\.md$/, "")}`,
    fileHref: (path: string) => `/${path}`,
};

describe("COMPONENT OpenPos", () => {
    it("shows what each PO has left and the invoices against it", () => {
        render(OpenPos, {usage: USAGE, onSave: vi.fn(), ...hrefs});
        const rows = screen.getAllByTestId("po-row");
        expect(rows[0].textContent).toContain("$30,000.00 left of $40,000");
        expect(within(rows[0]).getByRole("link", {name: "104"}).getAttribute("href")).toBe("/app/invoices/104");
        expect(within(rows[1]).getByText("Expired")).toBeDefined();
    });

    it("adds a PO", async () => {
        const onSave = vi.fn().mockResolvedValue(true);
        render(OpenPos, {usage: USAGE, onSave, ...hrefs});
        await fireEvent.click(screen.getByTestId("po-add"));
        await fireEvent.input(screen.getByTestId("po-reference"), {target: {value: " PO #900 "}});
        await fireEvent.input(screen.getByTestId("po-amount"), {target: {value: "$2,500"}});
        await fireEvent.click(screen.getByTestId("po-save"));
        expect(onSave).toHaveBeenCalledExactlyOnceWith([
            {reference: "PO #PY75334", amount: "$40,000", expires: "2027-06-30", attachment: null, was: "PO #PY75334"},
            {reference: "PO #9", amount: null, expires: "2026-01-31", attachment: null, was: "PO #9"},
            {reference: "PO #900", amount: "$2,500", expires: null, was: null},
        ]);
    });

    it("refuses a PO without its reference, before saving", async () => {
        const onSave = vi.fn();
        render(OpenPos, {usage: [], onSave, ...hrefs});
        expect(screen.getByTestId("po-none")).toBeDefined();
        await fireEvent.click(screen.getByTestId("po-add"));
        await fireEvent.keyDown(screen.getByTestId("po-reference"), {key: "Enter"});
        expect(onSave).not.toHaveBeenCalled();
        expect(screen.getByTestId("po-error").textContent).toContain("reference");
    });

    it("edits one, naming the reference it had", async () => {
        const onSave = vi.fn().mockResolvedValue(true);
        render(OpenPos, {usage: USAGE, onSave, ...hrefs});
        await fireEvent.click(screen.getByRole("button", {name: "Edit PO #9"}));
        await fireEvent.input(screen.getByTestId("po-reference"), {target: {value: "PO #9-B"}});
        await fireEvent.keyDown(screen.getByTestId("po-reference"), {key: "Enter"});
        expect(onSave.mock.calls[0][0][1]).toEqual({reference: "PO #9-B", amount: null, expires: "2026-01-31", was: "PO #9"});
    });

    it("closes one after asking", async () => {
        const onSave = vi.fn().mockResolvedValue(true);
        render(OpenPos, {usage: USAGE, onSave, ...hrefs});
        await fireEvent.click(screen.getByRole("button", {name: "Close PO #PY75334"}));
        expect(screen.getByTestId("po-close-dialog").textContent).toContain("keep their reference");
        expect(screen.getByTestId("po-close-dialog").textContent).toContain("noted under “Closed POs” in the customer's note");
        await fireEvent.click(screen.getByTestId("po-close-dialog-confirm"));
        expect(onSave).toHaveBeenCalledExactlyOnceWith([{reference: "PO #9", amount: null, expires: "2026-01-31", attachment: null, was: "PO #9"}]);
    });

    const ATTACHED: PoUsage[] = [
        {
            ...USAGE[0],
            attachment: {
                kind: "outside",
                path: "~/Documents/PO PY75334.pdf",
                location: "~/Documents/PO PY75334.pdf",
                found: "/Users/me/Documents/PO PY75334.pdf",
                name: "PO PY75334.pdf",
            },
        },
        {...USAGE[1], attachment: {kind: "file", path: "/pos/PO 9.pdf", found: "pos/PO 9.pdf", name: "PO 9.pdf"}},
    ];

    it("shows each PO's attachment, outside the books marked, and keeps it when a PO is edited", async () => {
        const onSave = vi.fn().mockResolvedValue(true);
        const onOpenFile = vi.fn();
        render(OpenPos, {usage: ATTACHED, onSave, ...hrefs, onOpenFile});
        const [first, second] = screen.getAllByTestId("po-attachment");
        expect(within(first).getByTestId("po-attachment-link-outside").textContent).toBe("outside the books");
        expect(first.textContent).not.toContain("[[");
        await fireEvent.click(within(first).getByRole("button", {name: "PO PY75334.pdf"}));
        expect(onOpenFile).toHaveBeenLastCalledWith("~/Documents/PO PY75334.pdf");
        await fireEvent.click(within(second).getByRole("button", {name: "PO 9.pdf"}));
        expect(onOpenFile).toHaveBeenLastCalledWith("pos/PO 9.pdf");
        await fireEvent.click(screen.getByRole("button", {name: "Edit PO #9"}));
        await fireEvent.keyDown(screen.getByTestId("po-reference"), {key: "Enter"});
        const sent = onSave.mock.calls[0][0] as PoForm[];
        expect(sent[0].attachment).toBe("~/Documents/PO PY75334.pdf");
        // The edited row leaves its attachment out: the note keeps it.
        expect(sent[1]).not.toHaveProperty("attachment");
    });

    it("in a browser, an outside attachment is its name and place, as text", () => {
        render(OpenPos, {usage: ATTACHED, onSave: vi.fn(), ...hrefs});
        const first = screen.getAllByTestId("po-attachment")[0];
        expect(within(first).queryByRole("button")).toBeNull();
        expect(within(first).queryByRole("link")).toBeNull();
        expect(first.textContent).toContain("PO PY75334.pdf");
        expect(first.textContent).toContain("~/Documents/PO PY75334.pdf");
        const second = screen.getAllByTestId("po-attachment")[1];
        expect(within(second).getByRole("link", {name: "PO 9.pdf"}).getAttribute("href")).toBe("/pos/PO 9.pdf");
    });
});
