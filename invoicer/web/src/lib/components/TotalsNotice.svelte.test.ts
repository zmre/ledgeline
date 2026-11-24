// The totals notice, mounted: stored next to computed, and Recalculate, which
// asks first for a sent invoice (it re-books it) and isn't offered on a void one.
import {fireEvent, render, screen} from "@testing-library/svelte";
import {describe, expect, it, vi} from "vitest";
import TotalsNotice from "./TotalsNotice.svelte";
import type {Mismatch} from "#lib/invoices.js";

const ITEMS: Mismatch[] = [
    {row: 2, label: "Senior architect hours", written: "$1,600.00", computed: "$2,400.00"},
    {row: 3, label: "Total due", written: "**$13,600.00**", computed: "$14,400.00"},
];

function mount(status: string | null, items: Mismatch[] = ITEMS, extra: {busy?: boolean; error?: string | null} = {}) {
    const onRecalculate = vi.fn();
    render(TotalsNotice, {number: "108", status, items, onRecalculate, ...extra});
    return onRecalculate;
}

describe("COMPONENT TotalsNotice", () => {
    it("is nothing when the amounts agree", () => {
        mount("draft", []);
        expect(screen.queryByTestId("totals-notice")).toBeNull();
    });

    it("shows stored and computed, and recalculates a draft at once", async () => {
        const onRecalculate = mount("draft");
        expect(screen.getByTestId("totals-notice").textContent).toContain("Totals don't match the rows");
        const rows = screen.getAllByTestId("totals-row").map((r) => [...r.querySelectorAll("td")].map((td) => td.textContent?.trim()).join(" | "));
        expect(rows).toEqual(["Senior architect hours | $1,600.00 | $2,400.00", "Total due | **$13,600.00** | $14,400.00"]);
        await fireEvent.click(screen.getByTestId("totals-recalc"));
        expect(onRecalculate).toHaveBeenCalledWith(false);
    });

    it("asks before recalculating a sent invoice, since that re-books it", async () => {
        const onRecalculate = mount("sent");
        await fireEvent.click(screen.getByTestId("totals-recalc"));
        expect(onRecalculate).not.toHaveBeenCalled();
        expect(screen.getByRole("alertdialog").textContent).toContain("Recalculate and re-book invoice 108?");
        await fireEvent.click(screen.getByRole("button", {name: "Cancel"}));
        expect(screen.queryByRole("alertdialog")).toBeNull();

        await fireEvent.click(screen.getByTestId("totals-recalc"));
        await fireEvent.click(screen.getByTestId("totals-rebook"));
        expect(onRecalculate).toHaveBeenCalledWith(true);
        expect(screen.queryByRole("alertdialog")).toBeNull();
    });

    it("offers nothing on a void invoice, and shows a refusal", () => {
        mount("void", ITEMS, {error: "on or before closed-through"});
        expect(screen.queryByTestId("totals-recalc")).toBeNull();
        expect(screen.getByTestId("totals-notice").textContent).toContain("void");
        expect(screen.getByTestId("totals-error").textContent).toContain("closed-through");
    });
});
