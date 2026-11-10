// The Settings tab strip's digits (plan 33 D22): Projections joined LAST, so
// keys 1–4 keep the tabs they always had, and 5 is the new one.

import {render, screen} from "@testing-library/svelte";
import {tick} from "svelte";
import {afterEach, describe, expect, it} from "vitest";
import {keymap} from "#lib/keys/keymap.svelte.js";
import SettingsTabs from "./SettingsTabs.svelte";

async function press(key: string): Promise<void> {
    keymap.handle(new KeyboardEvent("keydown", {key, cancelable: true}));
    await tick();
}

const selected = (): string | null => screen.getByRole("tablist").querySelector('[aria-selected="true"]')?.textContent?.trim() ?? null;

afterEach(() => {
    keymap.reset();
});

describe("COMPONENT SettingsTabs", () => {
    it("5 selects Projections, and 1–4 keep their tabs", async () => {
        render(SettingsTabs, {tab: "accounts"});
        await press("5");
        expect(selected()).toBe("Projections");
        for (const [key, label] of [
            ["1", "Accounts"],
            ["2", "Account Aliases"],
            ["3", "hledger config"],
            ["4", "Automatic postings"],
        ]) {
            await press(key);
            expect(selected()).toBe(label);
        }
    });
});
