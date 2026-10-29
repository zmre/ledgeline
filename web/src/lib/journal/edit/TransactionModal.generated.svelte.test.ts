// The edit modal with `--auto` in force (plan 30 Phase 5): the form holds only
// the written postings; the rule's postings are listed read-only beneath it,
// each linking to its rule.

import {fireEvent, render, screen} from "@testing-library/svelte";
import {tick} from "svelte";
import {afterEach, describe, expect, it, vi} from "vitest";
import {goto} from "$app/navigation";
import {connectFakeEngine} from "#lib/testing/fakeEngine.js";
import {autoSaleWire} from "#lib/testing/autoRow.js";
import TransactionModal from "./TransactionModal.svelte";
import {txnModal} from "./modalState.svelte";

vi.mock("$app/navigation", () => ({goto: vi.fn()}));
vi.mock("$app/paths", () => ({resolve: (path: string) => path}));

afterEach(() => {
    txnModal.close();
    vi.mocked(goto).mockClear();
    vi.unstubAllGlobals();
});

async function openSale(): Promise<void> {
    await connectFakeEngine({"/api/transactions/4": autoSaleWire()});
    txnModal.openEdit(4);
    await vi.waitFor(() => expect(txnModal.target).not.toBeNull());
    await tick();
}

describe("COMPONENT TransactionModal generated postings", () => {
    it("edits only the written postings and lists the rule's below, read-only", async () => {
        render(TransactionModal);
        await openSale();
        const accounts = screen.getAllByLabelText("Account") as HTMLInputElement[];
        expect(accounts.map((input) => input.value)).toEqual(["revenue:sales", "assets:checking"]);
        const generated = screen.getAllByTestId("modal-generated-posting");
        expect(generated.map((row) => row.textContent)).toEqual([expect.stringContaining("liabilities:tax:sales"), expect.stringContaining("assets:checking")]);
        expect(generated[0].textContent).toContain("from rule");
        expect(generated[0].textContent).toContain("= revenue:sales");
        expect(generated[0].querySelector("input")).toBeNull();
    });

    it("closes and opens the rule when its link is clicked", async () => {
        render(TransactionModal);
        await openSale();
        await fireEvent.click(screen.getAllByRole("button", {name: /from rule/})[0]);
        expect(txnModal.open).toBe(false);
        expect(goto).toHaveBeenCalledWith("/settings?tab=auto&rule=0");
    });
});
