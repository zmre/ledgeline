// J/K reorder rules from the keyboard, through the same `moveRule` the ↑/↓
// buttons use.
//
// The load-bearing assertion is that the cursor FOLLOWS the moved rule. Order is
// semantics in an hledger rules file ("later matches win"), so a user is
// normally moving one rule several places — and a cursor that stayed put would
// mean the second J moved a different rule than the first.

import {render, screen} from "@testing-library/svelte";
import {tick} from "svelte";
import {afterEach, describe, expect, it} from "vitest";
import {keymap} from "$lib/keys/keymap.svelte";
import type {FormItem} from "../model";
import RulesList from "./RulesList.svelte";

afterEach(() => keymap.reset());

function rule(pattern: string, account: string): FormItem {
    return {kind: "ifBlock", id: null, matchers: [{field: null, pattern}], assignments: {account2: account}, raw: []} as unknown as FormItem;
}

const ITEMS: FormItem[] = [rule("ACME PAYROLL", "income:salary"), rule("COFFEE", "expenses:coffee"), rule("SHELL", "expenses:gas")];

function mount(onChange: (items: FormItem[]) => void = () => {}) {
    return render(RulesList, {
        props: {
            items: ITEMS,
            accountNames: ["income:salary", "expenses:coffee", "expenses:gas"],
            csvFields: [],
            fallbackAccount: "expenses:misc",
            onChange,
            disabled: false,
        },
    });
}

async function press(key: string): Promise<void> {
    keymap.handle(new KeyboardEvent("keydown", {key, cancelable: true}));
    await tick();
}

const cursorAt = (): string | null => document.querySelector("[aria-current='true']")?.getAttribute("data-rule") ?? null;

describe("COMPONENT RulesList cursor", () => {
    it("has no cursor until a key moves it", () => {
        mount();

        expect(cursorAt()).toBeNull();
    });

    it("moves between cards with j and k", async () => {
        mount();

        await press("j");
        expect(cursorAt()).toBe("0");

        await press("j");
        expect(cursorAt()).toBe("1");

        await press("k");
        expect(cursorAt()).toBe("0");
    });

    it("jumps to the ends with G and gg", async () => {
        mount();

        await press("G");
        expect(cursorAt()).toBe("2");

        await press("g");
        await press("g");
        expect(cursorAt()).toBe("0");
    });

    it("clears on Escape", async () => {
        mount();
        await press("j");

        await press("Escape");

        expect(cursorAt()).toBeNull();
    });
});

describe("COMPONENT RulesList reorder", () => {
    it("moves the rule down on J, through the same moveRule the buttons use", async () => {
        let moved: FormItem[] | null = null;
        mount((items) => (moved = items));
        await press("j");

        await press("J");

        expect(moved).not.toBeNull();
        // ACME was first; after J it is second.
        expect(screen.getByRole("button", {name: "Move rule 2 up"})).toBeDefined();
    });

    it("takes the cursor with the moved rule", async () => {
        // Otherwise the second J moves a DIFFERENT rule than the first, which is
        // how you silently scramble a rules file.
        mount();
        await press("j");
        expect(cursorAt()).toBe("0");

        await press("J");

        expect(cursorAt()).toBe("1");
    });

    it("does nothing at the ends rather than wrapping", async () => {
        // Wrapping a reorder would move a rule from the top of the file to the
        // bottom on one keystroke — a semantic change wearing a cosmetic's
        // clothes, and not what the ↑/↓ buttons do (they disable at the bounds).
        let calls = 0;
        mount(() => (calls += 1));
        await press("j");

        await press("K");

        expect(calls).toBe(0);
    });

    it("does not reorder while the panel is disabled", async () => {
        let calls = 0;
        render(RulesList, {
            props: {items: ITEMS, accountNames: [], csvFields: [], fallbackAccount: "expenses:misc", onChange: () => (calls += 1), disabled: true},
        });
        await press("j");

        await press("J");

        expect(calls).toBe(0);
    });
});
