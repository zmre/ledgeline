// The bulk bar: the picked count and hours, each action's operation, and no
// Bill button until Phase 5 hands one in.
import {fireEvent, render, screen} from "@testing-library/svelte";
import {describe, expect, it, vi} from "vitest";
import BulkBar from "./BulkBar.svelte";

function mount(onBill?: () => void) {
    const onApply = vi.fn();
    render(BulkBar, {count: 2, seconds: 5400, accounts: ["internal:admin"], onApply, onClear: vi.fn(), onBill});
    return onApply;
}

describe("COMPONENT time BulkBar", () => {
    it("counts the picked lines and their time", () => {
        mount();
        expect(screen.getByTestId("bulk-count").textContent).toBe("2 picked · 1h 30m");
        expect(screen.queryByTestId("bulk-bill")).toBeNull();
    });

    it("marks billable at once, and not billable with a reason", async () => {
        const onApply = mount();
        await fireEvent.click(screen.getByTestId("bulk-billable"));
        expect(onApply).toHaveBeenLastCalledWith({op: "billable"});
        await fireEvent.click(screen.getByTestId("bulk-nobill"));
        await fireEvent.input(screen.getByTestId("bulk-reason"), {target: {value: " courtesy "}});
        await fireEvent.click(screen.getByTestId("bulk-apply"));
        expect(onApply).toHaveBeenLastCalledWith({op: "nobill", reason: "courtesy"});
    });

    it("moves to an account and sets a person", async () => {
        const onApply = mount();
        await fireEvent.click(screen.getByTestId("bulk-move"));
        expect((screen.getByTestId("bulk-apply") as HTMLButtonElement).disabled).toBe(true);
        await fireEvent.input(screen.getByRole("combobox", {name: "To account"}), {target: {value: "internal:admin"}});
        await fireEvent.click(screen.getByTestId("bulk-apply"));
        expect(onApply).toHaveBeenLastCalledWith({op: "move", account: "internal:admin"});
        await fireEvent.click(screen.getByTestId("bulk-person"));
        await fireEvent.input(screen.getByTestId("bulk-person-name"), {target: {value: "sam"}});
        await fireEvent.click(screen.getByTestId("bulk-apply"));
        expect(onApply).toHaveBeenLastCalledWith({op: "person", person: "sam"});
    });

    it("Escape goes back from an action's field", async () => {
        mount();
        await fireEvent.click(screen.getByTestId("bulk-person"));
        await fireEvent.keyDown(screen.getByTestId("bulk-person-name"), {key: "Escape"});
        expect(screen.queryByTestId("bulk-form")).toBeNull();
    });

    it("shows Bill when Phase 5 hands in its action", () => {
        mount(vi.fn());
        expect(screen.getByTestId("bulk-bill")).toBeDefined();
    });
});
