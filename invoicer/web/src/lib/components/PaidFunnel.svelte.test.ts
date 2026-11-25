// Getting paid (R14, R17), mounted on the server's golden: three cards with
// amounts and count badges, each a link to the filtered list, and the
// period selector asking the page for another window.
import {fireEvent, render, screen} from "@testing-library/svelte";
import {describe, expect, it, vi} from "vitest";
import PaidFunnel from "./PaidFunnel.svelte";
import {golden} from "../dashboardGolden.js";

const listBase = "/app/invoices";

describe("COMPONENT PaidFunnel", () => {
    it("shows drafts, outstanding and paid in the last 30 days, each a link to its invoices", () => {
        render(PaidFunnel, {dashboard: golden("accrual"), listBase});
        const drafts = screen.getByTestId("funnel-drafts");
        expect(drafts.getAttribute("href")).toBe("/app/invoices?status=draft");
        expect(screen.getByTestId("funnel-drafts-amount").textContent).toBe("$13,600.00");
        expect(screen.getByTestId("funnel-drafts-count").textContent).toBe("1 draft");
        const owed = screen.getByTestId("funnel-outstanding");
        expect(owed.getAttribute("href")).toBe("/app/invoices?status=open");
        expect(screen.getByTestId("funnel-outstanding-amount").textContent).toBe("$22,850.00");
        expect(screen.getByTestId("funnel-outstanding-count").textContent).toBe("7 owed");
        expect(screen.getByTestId("funnel-overdue").textContent).toContain("$12,700.00 overdue");
        const paid = screen.getByTestId("funnel-paid");
        expect(paid.getAttribute("href")).toBe("/app/invoices?paid_from=2026-10-24&paid_to=2026-11-22");
        expect(paid.textContent).toContain("Paid · last 30 days");
        expect(screen.getByTestId("funnel-paid-amount").textContent).toBe("$1,000.00");
        expect(screen.getByTestId("funnel-paid-count").textContent).toBe("1 paid");
        expect(paid.textContent).toContain("since 2026-10-24");
    });

    it("says so when nothing came in", () => {
        const d = golden("cash");
        render(PaidFunnel, {dashboard: {...d, paid: {...d.paid, commodities: []}}, listBase});
        expect(screen.getByTestId("funnel-paid-amount").textContent).toBe("Nothing yet");
        expect(screen.getByTestId("funnel-paid-count").textContent).toBe("0 paid");
    });

    it("offers the windows, on the current one, and asks for another", async () => {
        const onPaidDays = vi.fn();
        render(PaidFunnel, {dashboard: golden("cash"), listBase, onPaidDays});
        const select = screen.getByLabelText(/Paid in the/) as HTMLSelectElement;
        expect([...select.options].map((o) => o.textContent)).toEqual(["last 7 days", "last 30 days", "last 90 days", "last 365 days"]);
        expect(select.value).toBe("30");
        await fireEvent.change(select, {target: {value: "90"}});
        expect(onPaidDays).toHaveBeenCalledWith(90);
    });

    it("offers the setting's own window too, and dims the paid card while a new one loads", () => {
        const d = golden("cash");
        render(PaidFunnel, {dashboard: {...d, paid_window_days: 14, paid: {...d.paid, days: 14}}, listBase, loadingPaid: true});
        const select = screen.getByTestId("paid-window") as HTMLSelectElement;
        expect([...select.options].map((o) => o.value)).toEqual(["7", "14", "30", "90", "365"]);
        expect(select.value).toBe("14");
        const paid = screen.getByTestId("funnel-paid");
        expect(paid.className).toContain("opacity-60");
        expect(paid.getAttribute("aria-busy")).toBe("true");
    });

    it("lifts a card on hover and on keyboard focus, but not under reduced motion", () => {
        render(PaidFunnel, {dashboard: golden("accrual"), listBase});
        const card = screen.getByTestId("funnel-outstanding");
        for (const cls of ["hover:-translate-y-0.5", "focus-visible:-translate-y-0.5", "motion-reduce:hover:translate-none", "focus-visible:outline-2"]) {
            expect(card.className).toContain(cls);
        }
        // Only movement and shadow animate: colours follow a theme switch at once (no half-switched card).
        expect(card.className).toContain("transition-[translate,box-shadow,opacity]");
        expect(card.className).not.toMatch(/(^|\s)transition(\s|$)|transition-colors|transition-all/);
    });
});
