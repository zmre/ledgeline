// A customer's open POs in the view (PLAN 5b, S5–S6), mounted: what each has
// left, and Add, Edit and Close each saving the whole list at once.
import {fireEvent, render, screen, within} from "@testing-library/svelte";
import {describe, expect, it, vi} from "vitest";
import type {PoUsage} from "#lib/customers.js";
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

describe("COMPONENT OpenPos", () => {
    it("shows what each PO has left and the invoices against it", () => {
        render(OpenPos, {usage: USAGE, onSave: vi.fn(), invoiceHref});
        const rows = screen.getAllByTestId("po-row");
        expect(rows[0].textContent).toContain("$30,000.00 left of $40,000");
        expect(within(rows[0]).getByRole("link", {name: "104"}).getAttribute("href")).toBe("/app/invoices/104");
        expect(within(rows[1]).getByText("Expired")).toBeDefined();
    });

    it("adds a PO", async () => {
        const onSave = vi.fn().mockResolvedValue(true);
        render(OpenPos, {usage: USAGE, onSave, invoiceHref});
        await fireEvent.click(screen.getByTestId("po-add"));
        await fireEvent.input(screen.getByTestId("po-reference"), {target: {value: " PO #900 "}});
        await fireEvent.input(screen.getByTestId("po-amount"), {target: {value: "$2,500"}});
        await fireEvent.click(screen.getByTestId("po-save"));
        expect(onSave).toHaveBeenCalledExactlyOnceWith([
            {reference: "PO #PY75334", amount: "$40,000", expires: "2027-06-30", was: "PO #PY75334"},
            {reference: "PO #9", amount: null, expires: "2026-01-31", was: "PO #9"},
            {reference: "PO #900", amount: "$2,500", expires: null, was: null},
        ]);
    });

    it("refuses a PO without its reference, before saving", async () => {
        const onSave = vi.fn();
        render(OpenPos, {usage: [], onSave, invoiceHref});
        expect(screen.getByTestId("po-none")).toBeDefined();
        await fireEvent.click(screen.getByTestId("po-add"));
        await fireEvent.keyDown(screen.getByTestId("po-reference"), {key: "Enter"});
        expect(onSave).not.toHaveBeenCalled();
        expect(screen.getByTestId("po-error").textContent).toContain("reference");
    });

    it("edits one, naming the reference it had", async () => {
        const onSave = vi.fn().mockResolvedValue(true);
        render(OpenPos, {usage: USAGE, onSave, invoiceHref});
        await fireEvent.click(screen.getByRole("button", {name: "Edit PO #9"}));
        await fireEvent.input(screen.getByTestId("po-reference"), {target: {value: "PO #9-B"}});
        await fireEvent.keyDown(screen.getByTestId("po-reference"), {key: "Enter"});
        expect(onSave.mock.calls[0][0][1]).toEqual({reference: "PO #9-B", amount: null, expires: "2026-01-31", was: "PO #9"});
    });

    it("closes one after asking", async () => {
        const onSave = vi.fn().mockResolvedValue(true);
        render(OpenPos, {usage: USAGE, onSave, invoiceHref});
        await fireEvent.click(screen.getByRole("button", {name: "Close PO #PY75334"}));
        expect(screen.getByTestId("po-close-dialog").textContent).toContain("keep their reference");
        await fireEvent.click(screen.getByTestId("po-close-dialog-confirm"));
        expect(onSave).toHaveBeenCalledExactlyOnceWith([{reference: "PO #9", amount: null, expires: "2026-01-31", was: "PO #9"}]);
    });
});
