// Home's time row mounted: the three tiles and their links, nothing at all
// without time (or while first loading), the error alert with Retry, and a
// reload when a file changes.
import {fireEvent, render, screen, waitFor} from "@testing-library/svelte";
import {describe, expect, it, vi} from "vitest";
import {noteChange} from "#lib/changes.svelte.js";
import HomeTime from "./HomeTime.svelte";
import type {HomeTime as Figures, HomeTimeLinks} from "./homeTime.js";

const links: HomeTimeLinks = {
    hours: "/app/time?period=this-week",
    billable: "/app/time?period=this-week&split=billable",
    unbilled: "/app/time/entries?period=all&billed=unbilled",
    bill: "/app/to-bill",
    overview: "/app/time",
};

function figures(over: Partial<Figures> = {}): Figures {
    return {
        thisWeek: {from: "2026-11-23", end: "2026-11-29", total: 7200, billable: 5400, share: 0.75},
        lastWeek: {from: "2026-11-16", end: "2026-11-19", total: 3600, billable: 1800, share: 0.5},
        unbilled: {seconds: 4 * 3600 + 5 * 60, customers: 2, money: [{commodity: "$", amount: "1583.33"}], unpriced: false, needsCustomer: 0},
        ...over,
    };
}

const today = (): string => "2026-11-26";

describe("COMPONENT HomeTime", () => {
    it("shows this week against last week, the billable share and what is unbilled, each a link", async () => {
        const load = vi.fn(() => Promise.resolve(figures()));
        render(HomeTime, {links, load, today});
        await screen.findByTestId("home-time-row");
        expect(load).toHaveBeenCalledWith("2026-11-26");
        const hours = screen.getByTestId("home-time-hours");
        expect(hours.getAttribute("href")).toBe(links.hours);
        expect(screen.getByTestId("home-time-hours-value").textContent).toBe("2h");
        expect(screen.getByTestId("home-time-hours-delta").textContent).toContain("+1h (+100%)");
        expect(hours.textContent).toContain("Last week to Thu: 1h");
        const billable = screen.getByTestId("home-time-billable");
        expect(billable.getAttribute("href")).toBe(links.billable);
        expect(screen.getByTestId("home-time-billable-value").textContent).toBe("75%");
        expect(screen.getByTestId("home-time-billable-delta").textContent).toContain("+25 pts");
        expect(billable.textContent).toContain("1h 30m of 2h");
        const value = screen.getByTestId("home-time-unbilled-value");
        expect(value.textContent).toBe("$1,583.33");
        expect(value.getAttribute("href")).toBe(links.unbilled);
        expect(screen.getByTestId("home-time-unbilled-detail").textContent).toBe("4h 05m for 2 customers");
        expect(screen.getByTestId("home-time-bill").getAttribute("href")).toBe(links.bill);
    });

    it("shows hours when no customer's time has a rate, and says what lacks one", async () => {
        const unbilled = {seconds: 3600, customers: 1, money: [], unpriced: true, needsCustomer: 1800};
        render(HomeTime, {links, load: () => Promise.resolve(figures({unbilled})), today});
        expect((await screen.findByTestId("home-time-unbilled-value")).textContent).toBe("1h");
        expect(screen.getByTestId("home-time-unbilled-detail").textContent).toBe("1h for 1 customer · 30m needs a customer");
    });

    it("says nothing is unbilled, with no Bill…, and a dash for a week with no time", async () => {
        const empty = {from: "2026-11-23", end: "2026-11-29", total: 0, billable: 0, share: null};
        const unbilled = {seconds: 0, customers: 0, money: [], unpriced: false, needsCustomer: 0};
        render(HomeTime, {links, load: () => Promise.resolve(figures({thisWeek: empty, unbilled})), today});
        expect((await screen.findByTestId("home-time-unbilled-value")).textContent).toBe("Nothing to bill");
        expect(screen.queryByTestId("home-time-bill")).toBeNull();
        expect(screen.getByTestId("home-time-billable-value").textContent).toBe("—");
        expect(screen.queryByTestId("home-time-billable-delta")).toBeNull();
        expect(screen.getByTestId("home-time-hours-delta").textContent).toContain("−1h");
    });

    it("shows nothing while loading, nor without time", async () => {
        let resolve: (v: Figures | null) => void = () => {};
        const load = vi.fn(() => new Promise<Figures | null>((r) => (resolve = r)));
        const {container} = render(HomeTime, {links, load, today});
        expect(container.textContent?.trim()).toBe("");
        expect(container.querySelector(".loading")).toBeNull();
        resolve(null);
        await waitFor(() => expect(load).toHaveBeenCalled());
        await Promise.resolve();
        expect(screen.queryByTestId("home-time-row")).toBeNull();
        expect(container.textContent?.trim()).toBe("");
    });

    it("shows a failed first load with Retry", async () => {
        const load = vi.fn<(t: string) => Promise<Figures | null>>().mockRejectedValueOnce(new Error("boom")).mockResolvedValueOnce(figures());
        render(HomeTime, {links, load, today});
        const alert = await screen.findByTestId("home-time-error");
        expect(alert.textContent).toContain("Couldn't load your time: boom");
        await fireEvent.click(screen.getByRole("button", {name: "Retry"}));
        await screen.findByTestId("home-time-row");
    });

    it("loads again when a file changes, keeping the figures if that fails", async () => {
        const load = vi
            .fn<(t: string) => Promise<Figures | null>>()
            .mockResolvedValueOnce(figures())
            .mockResolvedValueOnce(figures({thisWeek: {from: "2026-11-23", end: "2026-11-29", total: 9000, billable: 0, share: 0}}))
            .mockRejectedValueOnce(new Error("gone"));
        const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
        render(HomeTime, {links, load, today});
        await screen.findByTestId("home-time-row");
        noteChange(["time/pat/2026-11.timedot"]);
        await waitFor(() => expect(screen.getByTestId("home-time-hours-value").textContent).toBe("2h 30m"));
        noteChange([]);
        await waitFor(() => expect(warn).toHaveBeenCalled());
        expect(screen.getByTestId("home-time-hours-value").textContent).toBe("2h 30m");
        expect(screen.queryByTestId("home-time-error")).toBeNull();
        warn.mockRestore();
    });

    it("hides when the time goes away", async () => {
        const load = vi.fn<(t: string) => Promise<Figures | null>>().mockResolvedValueOnce(figures()).mockResolvedValueOnce(null);
        render(HomeTime, {links, load, today});
        await screen.findByTestId("home-time-row");
        noteChange([]);
        await waitFor(() => expect(screen.queryByTestId("home-time-row")).toBeNull());
    });
});
