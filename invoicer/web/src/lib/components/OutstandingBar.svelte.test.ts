// The outstanding bar (R16), mounted on the server's golden: segment
// geometry on the axis, the tick labels, and the tooltip on hover and on
// keyboard focus (the same details either way), hidden by Escape.
import {fireEvent, render, screen} from "@testing-library/svelte";
import {describe, expect, it} from "vitest";
import OutstandingBar from "./OutstandingBar.svelte";
import {golden} from "../dashboardGolden.js";

const listBase = "/app/invoices";
const accrual = () => golden("accrual").commodities[0];

describe("COMPONENT OutstandingBar", () => {
    it("labels a round axis past the total, compactly", () => {
        render(OutstandingBar, {view: accrual(), listBase});
        // $22,850 runs to 25k by 5k.
        expect(screen.getAllByTestId("bar-tick").map((t) => t.textContent)).toEqual(["0", "5k", "10k", "15k", "20k", "25k"]);
        const ticks = screen.getAllByTestId("bar-tick");
        expect(ticks[0].getAttribute("style")).toContain("left: 0%");
        expect(ticks.at(-1)!.getAttribute("style")).toContain("left: 100%");
        expect(ticks.at(-1)!.getAttribute("style")).toContain("translateX(-100%)");
    });

    it("lays overdue then not yet due end to end, as shares of the axis, 2px apart", () => {
        render(OutstandingBar, {view: accrual(), listBase});
        const overdue = screen.getByTestId("bar-overdue");
        const notDue = screen.getByTestId("bar-not-due");
        // $12,700 and $10,150 of 25k.
        expect(overdue.dataset.width).toBe("50.80");
        expect(overdue.getAttribute("style")).toContain("width: 50.8%");
        expect(notDue.dataset.width).toBe("40.60");
        expect(notDue.getAttribute("style")).toContain("left: calc(50.8% + 2px)");
    });

    it("shows a tooltip on hover: the amount first, then the name, count and share", async () => {
        render(OutstandingBar, {view: accrual(), listBase});
        expect(screen.queryByRole("tooltip")).toBeNull();
        const overdue = screen.getByTestId("bar-overdue");
        await fireEvent.pointerEnter(overdue);
        const tip = screen.getByRole("tooltip");
        expect(tip.children[0].textContent).toBe("$12,700.00");
        expect(tip.textContent).toContain("Overdue");
        expect(tip.textContent).toContain("5 items · 56% of what is owed");
        expect(overdue.getAttribute("aria-describedby")).toBe(tip.id);
        // The other segment dims, so the hovered one stands out.
        expect(screen.getByTestId("bar-not-due").className).toContain("opacity-50");
        await fireEvent.pointerLeave(overdue);
        expect(screen.queryByRole("tooltip")).toBeNull();
    });

    it("shows the same tooltip on keyboard focus, and Escape hides it", async () => {
        render(OutstandingBar, {view: accrual(), listBase});
        const notDue = screen.getByTestId("bar-not-due");
        await fireEvent.focus(notDue);
        const tip = screen.getByRole("tooltip");
        expect(tip.textContent).toContain("$10,150.00");
        expect(tip.textContent).toContain("Not yet due");
        expect(tip.textContent).toContain("2 items · 44% of what is owed");
        await fireEvent.keyDown(notDue, {key: "Escape"});
        expect(screen.queryByRole("tooltip")).toBeNull();
        await fireEvent.focus(notDue);
        await fireEvent.blur(notDue);
        expect(screen.queryByRole("tooltip")).toBeNull();
    });

    it("prints every figure in its legend, so nothing rests on colour", () => {
        render(OutstandingBar, {view: accrual(), listBase});
        expect(screen.getByTestId("dashboard-overdue").textContent!.replace(/\s+/g, " ").trim()).toBe("Overdue $12,700.00 5 items");
        expect(screen.getByTestId("dashboard-not-due").textContent!.replace(/\s+/g, " ").trim()).toBe("Not yet due $10,150.00 2 items");
    });

    it("draws nothing on the track when nothing is owed", () => {
        const empty = {...accrual(), overdue: {amount: "0", text: "$0.00", count: 0}, not_due: {amount: "0", text: "$0.00", count: 0}};
        render(OutstandingBar, {view: {...empty, scale: {amount: "10000", text: "$10,000"}}, listBase});
        expect(screen.queryByTestId("bar-overdue")).toBeNull();
        expect(screen.queryByTestId("bar-not-due")).toBeNull();
        expect(screen.getAllByTestId("bar-tick").at(-1)!.textContent).toBe("10k");
    });
});
