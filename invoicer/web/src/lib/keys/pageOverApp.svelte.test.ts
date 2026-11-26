// A page's key over the app's (the customer page's `n`, New invoice for this
// customer, over the app's New invoice), mounted in the real order: the
// layout's app layer and the page's layer under it. The page's binding must
// answer the key and be the only `n` on the `?` sheet.
import {render} from "@testing-library/svelte";
import {describe, expect, it, vi} from "vitest";

const nav = vi.hoisted(() => ({goto: vi.fn()}));
vi.mock("$app/state", () => ({page: {url: new URL("http://localhost/app/customers/customers/Customer%20X")}}));
vi.mock("$app/navigation", () => ({goto: nav.goto}));
vi.mock("$app/paths", () => ({resolve: (path: string) => `/app${path}`}));

import {keymap} from "#ui/keys/keymap.svelte.js";
import {listenForKeys, press} from "#lib/testing/keys.js";
import LayoutWithPage from "#lib/testing/LayoutWithPage.svelte";

describe("COMPONENT a page's key over the app's", () => {
    listenForKeys();

    it("n on the customer page starts an invoice for this customer, and the sheet shows one n", async () => {
        const forCustomer = vi.fn();
        render(LayoutWithPage, {
            props: {
                pageLayer: () => ({
                    id: "customer-page",
                    bindings: [{keys: "n", label: "New invoice for this customer", group: "Customer", run: forCustomer}],
                }),
            },
        });
        // The page registered first, the layout after it (child effects run first).
        await press("n");
        expect(forCustomer).toHaveBeenCalledOnce();
        expect(nav.goto).not.toHaveBeenCalled();
        const rows = keymap.help.flatMap((section) => section.rows.map((row) => ({...row, group: section.group})));
        const ns = rows.filter((row) => row.keys === "n");
        expect(ns).toEqual([expect.objectContaining({label: "New invoice for this customer", group: "Customer"})]);
        // The app's other keys are still there.
        expect(rows.some((row) => row.keys === "d" && row.label === "New deposit invoice")).toBe(true);
    });

    it("without a page's n, the app's n still opens a new invoice", async () => {
        render(LayoutWithPage, {props: {pageLayer: () => ({id: "other-page", bindings: []})}});
        await press("n");
        expect(nav.goto).toHaveBeenLastCalledWith("/app/new");
    });
});
