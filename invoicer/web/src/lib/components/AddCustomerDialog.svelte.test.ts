// The editor's inline Add customer (invoicer PLAN E13), mounted: it creates
// through the customers API (4b's `POST /api/customers`), sending only what
// was filled in, and hands back the saved customer's link.
import {fireEvent, render, screen, waitFor} from "@testing-library/svelte";
import {afterEach, describe, expect, it, vi} from "vitest";
import AddCustomerDialog from "./AddCustomerDialog.svelte";

const saved = {
    path: "customers/Acme Corp.md",
    title: "Acme Corp",
    link: "[[Acme Corp]]",
    ledger_customer: "Acme Corp",
    kind: "organization",
    hash: "h",
    written: true,
    changed: [],
};

describe("COMPONENT AddCustomerDialog", () => {
    afterEach(() => {
        vi.unstubAllGlobals();
    });

    it("posts the filled-in fields to /api/customers and answers the link", async () => {
        const fetch = vi.fn(async () => new Response(JSON.stringify(saved), {status: 201}));
        vi.stubGlobal("fetch", fetch);
        const onCreated = vi.fn();
        render(AddCustomerDialog, {name: "Acme", onCreated, onCancel: vi.fn()});
        await fireEvent.input(screen.getByTestId("add-customer-name"), {target: {value: "Acme Corp"}});
        await fireEvent.input(screen.getByLabelText("Billing email"), {target: {value: "ap@acme.example"}});
        await fireEvent.input(screen.getByRole("spinbutton", {name: "Terms (net days)"}), {target: {value: "45"}});
        await fireEvent.click(screen.getByTestId("add-customer-save"));
        await waitFor(() => expect(onCreated).toHaveBeenCalledOnce());
        expect(onCreated.mock.calls[0][0]).toMatchObject({link: "[[Acme Corp]]"});
        const [url, init] = fetch.mock.calls[0] as unknown as [string, RequestInit];
        expect(url).toBe("/api/customers");
        expect(init.method).toBe("POST");
        // No address was given, so none is sent.
        expect(JSON.parse(String(init.body))).toEqual({
            kind: "organization",
            title: "Acme Corp",
            emails: [{label: "billing", value: "ap@acme.example"}],
            terms: 45,
        });
    });

    it("shows a refusal and stays open", async () => {
        vi.stubGlobal(
            "fetch",
            vi.fn(async () => new Response(JSON.stringify({error: "a note is already called Acme Corp"}), {status: 409}))
        );
        const onCreated = vi.fn();
        render(AddCustomerDialog, {name: "Acme Corp", onCreated, onCancel: vi.fn()});
        await fireEvent.click(screen.getByTestId("add-customer-save"));
        expect((await screen.findByRole("alert")).textContent).toContain("Acme Corp");
        expect(onCreated).not.toHaveBeenCalled();
    });
});
