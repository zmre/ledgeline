// The templates list (invoicer PLAN 4d), mounted.
import {fireEvent, render, screen, within} from "@testing-library/svelte";
import {describe, expect, it, vi} from "vitest";
import type {TemplateListEntry} from "#lib/templates.js";
import TemplateList from "./TemplateList.svelte";

const entries: TemplateListEntry[] = [
    {
        path: "/invoices/tpl/myco-default.html",
        name: "MyCo default",
        kinds: ["invoice", "estimate"],
        starter: false,
        installed: true,
        hash: "h1",
        extends: "_invoice_base.v1.html",
        base_version: 1,
        overrides: ["ready"],
        used_by: [
            {path: "invoices/101.md", title: "Invoice 101", how: "invoice", number: "101"},
            {path: "customers/Contoso.md", title: "Contoso", how: "customer", number: null},
        ],
    },
    {
        path: "/invoices/tpl/whole.html",
        name: "Whole",
        kinds: ["invoice"],
        starter: false,
        installed: true,
        hash: "h2",
        extends: null,
        base_version: null,
        overrides: [],
        used_by: [],
    },
    {
        path: "/invoices/tpl/classic.html",
        name: "Classic",
        kinds: ["invoice", "estimate", "credit-note"],
        starter: true,
        installed: false,
        hash: null,
        extends: "_invoice_base.v1.html",
        base_version: 1,
        overrides: [],
        used_by: [],
    },
];

describe("COMPONENT TemplateList", () => {
    it("shows each template's kinds, base, changes and uses, starters marked", () => {
        render(TemplateList, {entries, href: (e: TemplateListEntry) => `/app/templates${e.path.replace(/\.html$/, "")}`});
        const rows = screen.getAllByTestId("template-row");
        expect(rows).toHaveLength(3);
        const cells = (row: HTMLElement): string[] => [...row.querySelectorAll("td")].map((td) => td.textContent?.replace(/\s+/g, " ").trim() ?? "");
        expect(cells(rows[0])).toEqual(["MyCo default", "Invoices, estimates", "Invoice base, version 1", "ready", "1 invoice, 1 customer"]);
        expect(within(rows[0]).getByTestId("template-usage").getAttribute("title")).toBe("Invoice 101, Contoso (new invoices)");
        expect(cells(rows[1])).toEqual(["Whole", "Invoices", "A whole page", "—", "Not used"]);
        expect(cells(rows[2])[0]).toBe("Classic Starter");
        expect(cells(rows[2])[3]).toBe("Nothing yet");
        expect(within(rows[0]).getByRole("link", {name: "MyCo default"}).getAttribute("href")).toBe("/app/templates/invoices/tpl/myco-default");
        expect(within(rows[0]).queryByTestId("template-starter")).toBeNull();
    });

    it("opens a template from a click anywhere on its row, the name its one link", async () => {
        const onOpen = vi.fn();
        render(TemplateList, {entries, href: () => "#", onOpen});
        const rows = screen.getAllByTestId("template-row");
        await fireEvent.click(within(rows[1]).getByText("A whole page"));
        expect(onOpen).toHaveBeenCalledOnce();
        expect(onOpen.mock.calls[0][0].name).toBe("Whole");
        // The name is a link of its own: the row leaves its click alone.
        const link = within(rows[0]).getByRole("link", {name: "MyCo default"});
        link.addEventListener("click", (e) => e.preventDefault());
        await fireEvent.click(link);
        expect(onOpen).toHaveBeenCalledOnce();
        for (const row of rows) expect(within(row).getAllByRole("link")).toHaveLength(1);
    });

    it("follows the row's link when no handler is given", async () => {
        render(TemplateList, {entries, href: () => "#row"});
        const link = within(screen.getAllByTestId("template-row")[2]).getByRole("link", {name: "Classic"});
        const followed = vi.fn((e: Event) => e.preventDefault());
        link.addEventListener("click", followed);
        await fireEvent.click(within(screen.getAllByTestId("template-row")[2]).getByText("Nothing yet"));
        expect(followed).toHaveBeenCalledOnce();
    });
});
