// The shared empty state (Phase 4 feedback H21), mounted: a titled card with
// one sentence and a primary button, a link or an action.
import {fireEvent, render, screen, within} from "@testing-library/svelte";
import {describe, expect, it, vi} from "vitest";
import EmptyState from "./EmptyState.svelte";

const base = {title: "No snippets yet", text: "Written once for invoices to copy.", action: "New snippet", testid: "empty", actionTestid: "empty-new"};

describe("COMPONENT EmptyState", () => {
    it("is a titled card whose primary button is a link when given an href", () => {
        render(EmptyState, {...base, href: "/app/snippets/new"});
        const card = screen.getByTestId("empty");
        expect(within(card).getByRole("heading", {name: "No snippets yet"})).toBeDefined();
        expect(card.getAttribute("aria-labelledby")).toBe("empty-title");
        expect(card.textContent).toContain("Written once for invoices to copy.");
        const link = within(card).getByRole("link", {name: "New snippet"});
        expect(link.getAttribute("href")).toBe("/app/snippets/new");
        expect(link.className).toContain("btn-primary");
    });

    it("calls its action, and is disabled while busy", async () => {
        const onAction = vi.fn();
        const {rerender} = render(EmptyState, {...base, onAction});
        const button = screen.getByRole("button", {name: "New snippet"});
        expect(button.className).toContain("btn-primary");
        await fireEvent.click(button);
        expect(onAction).toHaveBeenCalledOnce();
        await rerender({...base, onAction, busy: true});
        expect((screen.getByTestId("empty-new") as HTMLButtonElement).disabled).toBe(true);
    });
});
