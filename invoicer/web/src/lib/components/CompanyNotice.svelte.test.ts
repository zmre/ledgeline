// The company notice, mounted: a quiet line on a sent invoice whose copy of
// your company is out of date, and Update from company, which asks first (it
// re-books). Drafts never show it (they use the company's current details).
import {fireEvent, render, screen} from "@testing-library/svelte";
import {describe, expect, it, vi} from "vitest";
import CompanyNotice from "./CompanyNotice.svelte";

function mount(status: string | null, changed: boolean, extra: {busy?: boolean; error?: string | null; confirming?: boolean} = {}) {
    const onUpdate = vi.fn();
    render(CompanyNotice, {number: "105", status, changed, onUpdate, ...extra});
    return onUpdate;
}

describe("COMPONENT CompanyNotice", () => {
    it("is nothing for a draft, a void invoice, or a sent one whose details are current", () => {
        for (const [status, changed] of [
            ["draft", true],
            ["void", true],
            ["sent", false],
            [null, true],
        ] as const) {
            const {unmount} = render(CompanyNotice, {number: "105", status, changed, onUpdate: vi.fn(), confirming: true});
            expect(screen.queryByTestId("company-notice"), `${status} ${changed}`).toBeNull();
            if (status !== "sent") expect(screen.queryByRole("alertdialog"), `${status}`).toBeNull();
            unmount();
        }
    });

    it("offers the update quietly, and asks before it rewrites a sent invoice", async () => {
        const onUpdate = mount("sent", true);
        const notice = screen.getByTestId("company-notice");
        expect(notice.textContent).toContain("Your company's details have changed since this invoice was sent.");
        expect(screen.queryByRole("alert")).toBeNull();
        // No field keys or implementation words in what it says.
        expect(notice.textContent).not.toMatch(/invoicer_|\[\[|frontmatter/);

        await fireEvent.click(screen.getByTestId("company-update"));
        expect(onUpdate).not.toHaveBeenCalled();
        const dialog = screen.getByRole("alertdialog");
        expect(dialog.textContent).toContain("Update invoice 105 from your company?");
        expect(dialog.textContent).toContain("re-booked");
        expect(dialog.textContent).not.toMatch(/invoicer_|\[\[|frontmatter/);
        // A step that writes the books: the focus is on the safe choice.
        expect(document.activeElement).toBe(screen.getByRole("button", {name: "Cancel"}));

        await fireEvent.click(screen.getByRole("button", {name: "Cancel"}));
        expect(screen.queryByRole("alertdialog")).toBeNull();
        expect(onUpdate).not.toHaveBeenCalled();

        await fireEvent.click(screen.getByTestId("company-update"));
        await fireEvent.click(screen.getByTestId("company-confirm-confirm"));
        expect(onUpdate).toHaveBeenCalledTimes(1);
        expect(screen.queryByRole("alertdialog")).toBeNull();
    });

    it("asks when the page's key does, and Escape cancels", async () => {
        const onUpdate = mount("sent", true, {confirming: true});
        expect(screen.getByRole("alertdialog").textContent).toContain("Update invoice 105 from your company?");
        await fireEvent.keyDown(document.activeElement ?? document.body, {key: "Escape"});
        expect(screen.queryByRole("alertdialog")).toBeNull();
        expect(onUpdate).not.toHaveBeenCalled();
    });

    it("shows a refusal, and can't be pressed while busy", () => {
        mount("sent", true, {busy: true, error: "invoice 101 is dated on or before closed-through"});
        expect(screen.getByTestId("company-error").textContent).toContain("closed-through");
        expect((screen.getByTestId("company-update") as HTMLButtonElement).disabled).toBe(true);
    });
});
