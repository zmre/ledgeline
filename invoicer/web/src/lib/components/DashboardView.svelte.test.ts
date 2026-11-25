// The receivables dashboard, mounted on what the server answers for the
// accrual and cash fixtures (its golden): the funnel, the hero total, the
// outstanding bar on its axis, aging, recently updated, the tiles, and every
// figure a link to the filtered invoice list.
import {fireEvent, render, screen, within} from "@testing-library/svelte";
import {describe, expect, it, vi} from "vitest";
import DashboardView from "./DashboardView.svelte";
import {golden} from "../dashboardGolden.js";
import type {Dashboard} from "../dashboard.js";

const listBase = "/app/invoices";
const invoiceHref = (number: string) => `/app/invoices/${number}`;
const mount = (dashboard: Dashboard, extra: Record<string, unknown> = {}) => render(DashboardView, {dashboard, listBase, invoiceHref, ...extra});

describe("COMPONENT DashboardView", () => {
    it("leads with the total owed, linked to every open invoice", () => {
        mount(golden("accrual"));
        const total = screen.getByTestId("dashboard-total");
        expect(total.textContent).toBe("$22,850.00");
        expect(total.getAttribute("href")).toBe("/app/invoices?status=open");
        expect(total.className).toContain("text-5xl");
        expect(screen.getByTestId("dashboard-total-count").textContent).toBe("7 items owed");
        expect(screen.getByTestId("receivables-panel").textContent).toContain("Total outstanding");
    });

    it("has no New invoice button of its own: the sidebar's New is the one (R13)", () => {
        mount(golden("accrual"));
        expect(screen.queryByRole("link", {name: "New invoice"})).toBeNull();
        expect(screen.queryByRole("button", {name: "New invoice"})).toBeNull();
    });

    it("draws overdue red and not yet due yellow, each segment a link named with its amount and share", () => {
        mount(golden("accrual"));
        const overdue = screen.getByTestId("bar-overdue");
        expect(overdue.className).toContain("bg-error");
        expect(overdue.getAttribute("href")).toBe("/app/invoices?status=overdue");
        expect(overdue.getAttribute("aria-label")).toBe("Overdue: $12,700.00, 5 items, 56% of what is owed");
        const notDue = screen.getByTestId("bar-not-due");
        expect(notDue.className).toContain("bg-warning");
        expect(notDue.getAttribute("href")).toBe("/app/invoices?aged=current");
        expect(screen.getByTestId("dashboard-overdue").textContent).toContain("$12,700.00");
        expect(screen.getByTestId("dashboard-not-due").textContent).toContain("$10,150.00");
        // Receivables without an invoice file are explained, since the list can't show them.
        expect(screen.getByTestId("dashboard-unfiled").textContent).toContain("$11,300.00");
    });

    it("draws a small balance against the $10,000 minimum axis", () => {
        mount(golden("cash"));
        expect(screen.getByTestId("dashboard-total").textContent).toBe("$9,700.00");
        expect(screen.queryByTestId("bar-overdue")).toBeNull();
        expect(screen.getByTestId("bar-not-due").getAttribute("style")).toContain("width: 97%");
        expect(screen.getAllByTestId("bar-tick").map((t) => t.textContent)).toEqual(["0", "2k", "4k", "6k", "8k", "10k"]);
        expect(screen.queryByTestId("dashboard-unfiled")).toBeNull();
    });

    it("lists the aging buckets, each with a figure linking to its range", () => {
        mount(golden("accrual"));
        const rows = screen.getAllByTestId("aging-row");
        expect(rows.map((r) => r.dataset.key)).toEqual(["current", "1-30", "31-60", "61-90", "91+"]);
        const old = within(rows[4]).getByRole("link", {name: "91+ days past due: $3,000.00, 1 item"});
        expect(old.getAttribute("href")).toBe("/app/invoices?aged=91%2B");
        expect(within(rows[0]).getByRole("link").getAttribute("href")).toBe("/app/invoices?aged=current");
    });

    it("leaves an empty bucket unlinked", () => {
        mount(golden("cash"));
        const rows = screen.getAllByTestId("aging-row");
        expect(within(rows[0]).getByRole("link").getAttribute("href")).toBe("/app/invoices?aged=current");
        expect(within(rows[1]).queryByRole("link")).toBeNull();
        expect(rows[1].textContent).toContain("$0.00");
    });

    it("shows the tiles: oldest overdue, DSO, days to pay (drafts are the funnel's)", () => {
        mount(golden("accrual"));
        const oldest = screen.getByTestId("tile-oldest-overdue");
        expect(oldest.textContent).toContain("130 days");
        expect(oldest.textContent).toContain("No. 2001 · Tailspin · $3,000.00 · due 2026-07-15");
        // 2001 has no file (booked in the journal alone): its customer's overdue invoices.
        expect(oldest.getAttribute("href")).toBe("/app/invoices?status=overdue&customer=Tailspin");
        const dso = screen.getByTestId("tile-dso");
        expect(dso.textContent).toContain("58.9 days");
        expect(dso.textContent).toContain("$22,850.00 owed for $34,900.00 billed in the last 90 days");
        expect(dso.getAttribute("href")).toBe("/app/invoices?status=open");
        const paid = screen.getByTestId("tile-days-to-pay");
        expect(paid.textContent).toContain("25.3 days");
        expect(paid.textContent).toContain("Over 3 invoices paid in full");
        expect(paid.getAttribute("href")).toBe("/app/invoices?status=paid");
        expect(screen.queryByTestId("tile-drafts")).toBeNull();
    });

    it("opens the oldest overdue invoice itself when it has a file", () => {
        const d = golden("accrual");
        const usd = {...d.commodities[0], oldest_overdue: {...d.commodities[0].oldest_overdue!, number: "106", path: "invoices/106.md"}};
        mount({...d, commodities: [usd]});
        expect(screen.getByTestId("tile-oldest-overdue").getAttribute("href")).toBe("/app/invoices/106");
    });

    it("counts recognition due, with Open in Ledgeline only where the server can", async () => {
        const {unmount} = mount(golden("accrual"));
        const tile = screen.getByTestId("tile-recognition");
        expect(screen.getByTestId("recognition-due").textContent).toBe("10 entries");
        expect(tile.textContent).toContain("from 2026-11-01");
        expect(tile.textContent).toContain("1 more is due in a closed period.");
        expect(tile.textContent).toContain("Post them in Ledgeline: Journal → Recognition.");
        expect(screen.queryByTestId("open-ledgeline")).toBeNull();
        unmount();

        const onOpenLedgeline = vi.fn();
        mount({...golden("accrual"), open_ledgeline: true}, {onOpenLedgeline, openError: "Ledgeline isn't installed on this Mac"});
        await fireEvent.click(screen.getByRole("button", {name: "Open in Ledgeline"}));
        expect(onOpenLedgeline).toHaveBeenCalledOnce();
        expect(screen.getByRole("alert").textContent).toBe("Ledgeline isn't installed on this Mac");
        expect(screen.getByTestId("tile-recognition").textContent).not.toContain("Journal → Recognition");
    });

    it("has no recognition tile without schedules, and says when nothing is owed", () => {
        mount({...golden("cash"), commodities: []});
        expect(screen.queryByTestId("tile-recognition")).toBeNull();
        expect(screen.getByTestId("dashboard-nothing-owed").textContent).toContain("Nothing owed");
        expect(screen.getByTestId("tile-oldest-overdue").textContent).toContain("None");
    });

    it("leads with the funnel and shows recently updated invoices", () => {
        mount(golden("accrual"));
        const sections = screen.getByTestId("dashboard").children;
        expect(sections[0].getAttribute("data-testid")).toBe("funnel");
        expect(screen.getAllByTestId("recent-card")[0].dataset.number).toBe("106");
    });

    it("reaches every figure by Tab: the funnel, the bar, aging, the cards and the tiles are links", () => {
        mount(golden("accrual"));
        const links = screen.getAllByRole("link");
        // Three funnel cards, two bar segments, two legend figures, the total,
        // five buckets, six recent cards, three tiles.
        expect(links).toHaveLength(22);
        expect(links.every((l) => l.getAttribute("tabindex") !== "-1")).toBe(true);
        expect(screen.getByTestId("paid-window").tagName).toBe("SELECT");
    });
});
