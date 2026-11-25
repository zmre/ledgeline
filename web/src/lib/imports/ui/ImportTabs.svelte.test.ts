// The Imports strip (plan 43): Accounts joined FIRST, so the digits are
// 1 Accounts, 2 New Transactions, 3 Edit Rules.

import {render, screen} from "@testing-library/svelte";
import {tick} from "svelte";
import {afterEach, describe, expect, it} from "vitest";
import {keymap} from "#ui/keys/keymap.svelte.js";
import ImportTabs from "./ImportTabs.svelte";

async function press(key: string): Promise<void> {
    keymap.handle(new KeyboardEvent("keydown", {key, cancelable: true}));
    await tick();
}

const selected = (): string | null => screen.getByRole("tablist").querySelector('[aria-selected="true"]')?.textContent?.trim() ?? null;

afterEach(() => {
    keymap.reset();
});

describe("COMPONENT ImportTabs", () => {
    it("shows Accounts first, and 1–3 select the three tabs", async () => {
        render(ImportTabs, {tab: "new"});
        expect(screen.getAllByRole("tab").map((t) => t.textContent?.trim())).toEqual(["Accounts", "New Transactions", "Edit Rules"]);
        for (const [key, label] of [
            ["1", "Accounts"],
            ["2", "New Transactions"],
            ["3", "Edit Rules"],
        ]) {
            await press(key);
            expect(selected()).toBe(label);
        }
    });
});
