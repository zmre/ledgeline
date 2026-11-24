// The template picker, mounted: which template the invoice uses, previewing
// another, and "Use this template".
import {fireEvent, render, screen} from "@testing-library/svelte";
import {describe, expect, it, vi} from "vitest";
import TemplatePicker from "./TemplatePicker.svelte";
import type {TemplateEntry} from "#lib/invoices.js";

const MYCO = "/invoices/tpl/myco-default.html";
const MODERN = "/invoices/tpl/modern.html";
const TEMPLATES: TemplateEntry[] = [
    {path: MYCO, name: "MyCo default", kinds: ["invoice", "estimate"], starter: false, installed: true},
    {path: "/invoices/tpl/classic.html", name: "Classic", kinds: ["invoice"], starter: true, installed: false},
    {path: MODERN, name: "Modern", kinds: ["invoice"], starter: true, installed: false},
];

function mount(own: string | null, extra: {chosen?: string; busy?: boolean; error?: string | null} = {}) {
    const onUse = vi.fn();
    render(TemplatePicker, {templates: TEMPLATES, own, onUse, ...extra});
    return onUse;
}

describe("COMPONENT TemplatePicker", () => {
    it("says which template the invoice uses, and offers nothing to use while it shows that one", () => {
        mount(MYCO);
        expect(screen.getByTestId("template-current").textContent).toContain(`This invoice uses MyCo default (${MYCO})`);
        expect((screen.getByTestId("template-choice") as HTMLSelectElement).value).toBe("");
        expect(screen.queryByTestId("template-use")).toBeNull();
    });

    it("previews another, then makes it the invoice's own", async () => {
        const onUse = mount(MYCO);
        await fireEvent.change(screen.getByTestId("template-choice"), {target: {value: MODERN}});
        const note = screen.getByTestId("template-current").textContent ?? "";
        expect(note).toContain("Previewing Modern: the invoice still uses MyCo default.");
        expect(note).toContain("copies the starter into the books");
        await fireEvent.click(screen.getByTestId("template-use"));
        expect(onUse).toHaveBeenCalledWith(MODERN);
    });

    it("says when the invoice names a template the books don't have", () => {
        mount("/invoices/tpl/gone.html");
        expect(screen.getByTestId("template-current").textContent).toContain("names /invoices/tpl/gone.html, which isn't in the books");
        const options = [...(screen.getByTestId("template-choice") as HTMLSelectElement).options].map((o) => o.textContent?.trim());
        expect(options[0]).toBe("/invoices/tpl/gone.html (this invoice's, not found)");
    });

    it("shows a failure, and is busy while it writes", () => {
        mount(MYCO, {chosen: MODERN, busy: true, error: "the file changed on disk"});
        expect((screen.getByTestId("template-use") as HTMLButtonElement).disabled).toBe(true);
        expect(screen.getByTestId("template-error").textContent).toContain("the file changed on disk");
    });
});
