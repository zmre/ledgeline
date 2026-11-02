// The account picker for an upload that holds several accounts — a "download
// all accounts" `.qbo`. What matters: every account is offered, the one being
// read is marked, and choosing another asks for THAT index.

import {fireEvent, render, screen} from "@testing-library/svelte";
import {describe, expect, it, vi} from "vitest";
import StatementPicker from "./StatementPicker.svelte";

const STATEMENTS = [
    {accountHint: "7592", accountType: "CHECKING", currency: "USD", ledgerBalance: "7033.77", transactions: 12},
    {accountHint: "3656", accountType: "CHECKING", currency: "USD", ledgerBalance: "29016.80", transactions: 33},
    {accountHint: "8485", accountType: "SAVINGS", currency: "USD", ledgerBalance: "25.00", transactions: 0},
];

describe("COMPONENT StatementPicker", () => {
    it("offers every account and marks the one being read", () => {
        render(StatementPicker, {statements: STATEMENTS, selected: 1, disabled: false, onChoose: () => {}});

        const choices = screen.getAllByRole("radio");
        expect(choices.map((choice) => choice.textContent?.replace(/\s+/g, " ").trim())).toEqual([
            "Checking …7592 12 transactions · 7033.77 USD",
            "Checking …3656 33 transactions · 29016.80 USD",
            "Savings …8485 0 transactions · 25.00 USD",
        ]);
        expect(choices.map((choice) => choice.getAttribute("aria-checked"))).toEqual(["false", "true", "false"]);
    });

    it("asks for the account that was chosen", async () => {
        const onChoose = vi.fn();
        render(StatementPicker, {statements: STATEMENTS, selected: 0, disabled: false, onChoose});

        await fireEvent.click(screen.getByText("Savings …8485"));

        expect(onChoose).toHaveBeenCalledWith(2);
    });

    it("cannot be changed while an import is in flight", () => {
        render(StatementPicker, {statements: STATEMENTS, selected: 0, disabled: true, onChoose: () => {}});

        expect(screen.getAllByRole("radio").every((choice) => (choice as HTMLButtonElement).disabled)).toBe(true);
    });
});
