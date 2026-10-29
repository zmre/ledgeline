// The journal's account chips with `--auto` in force (plan 30 Phase 5): a
// generated posting is never a chip — the flow still reads source → dest — and
// a muted "+N auto" badge links to the rule that added them.

import {fireEvent, render, screen} from "@testing-library/svelte";
import {afterEach, describe, expect, it, vi} from "vitest";
import {goto} from "$app/navigation";
import {autoSale} from "#lib/testing/autoRow.js";
import AccountsCell from "./AccountsCell.svelte";

vi.mock("$app/navigation", () => ({goto: vi.fn()}));
vi.mock("$app/paths", () => ({resolve: (path: string) => path}));

afterEach(() => {
    vi.mocked(goto).mockClear();
});

describe("COMPONENT AccountsCell with generated postings", () => {
    it("draws the written flow and a +N auto badge, never a chip for a generated posting", () => {
        const {container} = render(AccountsCell, {txn: autoSale()});
        const text = container.textContent ?? "";
        expect(text).toContain("revenue:sales");
        expect(text).toContain("assets:checking");
        expect(text).not.toContain("liabilities:tax:sales");
        expect(screen.getByTestId("auto-badge").textContent?.trim()).toBe("+2 auto");
        expect(screen.getByTestId("auto-badge").getAttribute("title")).toContain("= revenue:sales");
    });

    it("opens Settings › Automatic postings on the rule when the badge is clicked", async () => {
        render(AccountsCell, {txn: autoSale()});
        await fireEvent.click(screen.getByTestId("auto-badge"));
        expect(goto).toHaveBeenCalledWith("/settings?tab=auto&rule=0");
    });

    it("draws no badge for a transaction no rule touched", () => {
        const sale = autoSale();
        const written = {...sale, postings: sale.postings.filter((posting) => posting.generated === undefined)};
        render(AccountsCell, {txn: written});
        expect(screen.queryByTestId("auto-badge")).toBeNull();
    });
});
