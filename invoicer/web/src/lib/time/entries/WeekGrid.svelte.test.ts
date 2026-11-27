// The week grid: rows, totals, a typed cell sent as seconds, a refusal or a
// bad duration said in words, and adding an account's row.
import {fireEvent, render, screen, within} from "@testing-library/svelte";
import {describe, expect, it, vi} from "vitest";
import {entry, lockedEntry, qty} from "../testing.js";
import WeekGrid from "./WeekGrid.svelte";
import {weekGrid} from "./group.js";

const entries = [
    entry({id: "f@1", date: "2026-11-25", account: "clients:acme:tax-return", quantity: qty(130)}),
    entry({id: "f@2", date: "2026-11-26", quantity: qty(60)}),
    lockedEntry({id: "f@3", date: "2026-11-26", account: "clients:acme:tax-return"}),
];

function mount(onCell = vi.fn().mockResolvedValue(null), onAddRow = vi.fn()) {
    render(WeekGrid, {grid: weekGrid(entries, "2026-11-23"), today: "2026-11-26", accounts: ["internal:admin"], onCell, onAddRow});
    return {onCell, onAddRow};
}

const cell = (account: string, date: string): HTMLInputElement => {
    const row = screen.getAllByTestId("week-row").find((r) => r.dataset.account === account) as HTMLElement;
    return within(row)
        .getAllByTestId("week-cell")
        .find((c) => c.dataset.date === date) as HTMLInputElement;
};

describe("COMPONENT time WeekGrid", () => {
    it("shows accounts × days with the totals", () => {
        mount();
        expect(screen.getAllByTestId("week-day").map((d) => d.textContent?.slice(0, 3))).toEqual(["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]);
        expect(cell("clients:acme:tax-return", "2026-11-25").value).toBe("2h10m");
        expect(cell("clients:acme:tax-return", "2026-11-26").value).toBe("3h");
        expect(cell("clients:acme:tax-return", "2026-11-26").className).toContain("border-dashed");
        expect(screen.getByTestId("week-total").textContent).toBe("6h 10m");
    });

    it("sends a typed cell as seconds", async () => {
        const {onCell} = mount();
        const input = cell("clients:acme:advisory", "2026-11-26");
        await fireEvent.input(input, {target: {value: "1h30"}});
        await fireEvent.keyDown(input, {key: "Enter"});
        expect(onCell).toHaveBeenCalledWith("clients:acme:advisory", expect.objectContaining({date: "2026-11-26", seconds: 3600}), 5400);
    });

    it("an empty cell is zero; an unchanged one sends nothing", async () => {
        const {onCell} = mount();
        const input = cell("clients:acme:advisory", "2026-11-26");
        await fireEvent.change(input, {target: {value: "60m"}});
        expect(onCell).not.toHaveBeenCalled();
        await fireEvent.change(input, {target: {value: ""}});
        expect(onCell).toHaveBeenCalledWith("clients:acme:advisory", expect.anything(), 0);
    });

    it("says why a cell can't change, and puts it back", async () => {
        mount(vi.fn().mockResolvedValue("Only 0m of this cell can be taken off here."));
        const input = cell("clients:acme:tax-return", "2026-11-26");
        await fireEvent.change(input, {target: {value: "1h"}});
        expect((await screen.findByTestId("week-problem")).textContent).toContain("can be taken off");
        expect(input.value).toBe("3h");
    });

    it("refuses a duration it can't read without asking", async () => {
        const {onCell} = mount();
        await fireEvent.change(cell("clients:acme:advisory", "2026-11-24"), {target: {value: "soon"}});
        expect(screen.getByTestId("week-problem").textContent).toContain("isn't a duration");
        expect(onCell).not.toHaveBeenCalled();
    });

    it("adds an account's row", async () => {
        const {onAddRow} = mount();
        await fireEvent.input(screen.getByRole("combobox", {name: "Account to add a row for"}), {target: {value: "internal:admin"}});
        await fireEvent.click(screen.getByTestId("week-add-row"));
        expect(onAddRow).toHaveBeenCalledWith("internal:admin");
    });
});
