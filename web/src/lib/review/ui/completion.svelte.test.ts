// Autocomplete on the review card (plan 32 D15): descriptions complete from
// the journal's payees, tags complete their name and then their value.

import {fireEvent, render, screen} from "@testing-library/svelte";
import {tick} from "svelte";
import {describe, expect, it} from "vitest";
import DescriptionInput from "./DescriptionInput.svelte";
import TagChips from "./TagChips.svelte";

const keydown = async (target: Element, key: string): Promise<KeyboardEvent> => {
    const event = new KeyboardEvent("keydown", {key, bubbles: true, cancelable: true});
    target.dispatchEvent(event);
    await tick();
    return event;
};

describe("COMPONENT DescriptionInput", () => {
    const payees = [
        {payee: "Blue Bottle Coffee", count: 3, last: "2026-10-01"},
        {payee: "SQ *BLUE BOTTLE 4412 OAKLAND", count: 1, last: "2026-10-12"},
        {payee: "Safeway", count: 9, last: "2026-10-25"},
    ];

    it("opens on typing, ranks prefixes first, and Enter accepts without saving", async () => {
        render(DescriptionInput, {props: {value: "", payees, today: "2026-10-30"}});
        const field = screen.getByLabelText("Description") as HTMLInputElement;
        expect(screen.queryByRole("listbox")).toBeNull();
        await fireEvent.input(field, {target: {value: "blue"}});
        await tick();
        expect(screen.getAllByRole("option").map((option) => option.textContent?.replace(/\s+/g, " ").trim())).toEqual([
            "Blue Bottle Coffee 3×",
            "SQ *BLUE BOTTLE 4412 OAKLAND 1×",
        ]);
        const event = await keydown(field, "Enter");
        expect(event.defaultPrevented).toBe(true);
        expect(field.value).toBe("Blue Bottle Coffee");
        expect(screen.queryByRole("listbox")).toBeNull();
    });

    it("keeps a | note while completing the payee, and Escape closes the popup first", async () => {
        render(DescriptionInput, {props: {value: "", payees, today: "2026-10-30"}});
        const field = screen.getByLabelText("Description") as HTMLInputElement;
        await fireEvent.input(field, {target: {value: "Saf | weekly"}});
        await tick();
        expect(screen.queryByRole("listbox")).toBeNull();
        await fireEvent.input(field, {target: {value: "Saf"}});
        await tick();
        const escape = await keydown(field, "Escape");
        expect(escape.defaultPrevented).toBe(true);
        expect(screen.queryByRole("listbox")).toBeNull();
        const second = await keydown(field, "Escape");
        expect(second.defaultPrevented).toBe(false);
    });
});

describe("COMPONENT TagChips", () => {
    const vocabulary = [
        {
            name: "trip",
            count: 4,
            values: [
                {value: "paris", count: 3},
                {value: "rome", count: 1},
            ],
        },
        {name: "receipt", count: 1, values: []},
    ];

    it("completes the name, then its value, and adds the chip", async () => {
        const props = $state({tags: [] as [string, string][], vocabulary});
        render(TagChips, {props});
        const field = screen.getByLabelText("Add a tag") as HTMLInputElement;
        await fireEvent.input(field, {target: {value: "tr"}});
        await tick();
        expect(screen.getAllByRole("option").map((option) => option.textContent?.replace(/\s+/g, " ").trim())).toEqual(["trip: 4×"]);
        await keydown(field, "Enter");
        expect(field.value).toBe("trip:");
        await tick();
        expect(screen.getAllByRole("option")[0]?.textContent).toContain("paris");
        await keydown(field, "Enter");
        await tick();
        expect(screen.getAllByTestId("review-tag").map((chip) => chip.textContent?.replace(/\s+/g, "").trim())).toEqual(["trip:paris"]);
        expect(field.value).toBe("");
        await keydown(field, "Backspace");
        await tick();
        expect(screen.queryAllByTestId("review-tag")).toHaveLength(0);
    });

    it("refuses a tag the engine could not write", async () => {
        render(TagChips, {props: {tags: [], vocabulary}});
        const field = screen.getByLabelText("Add a tag") as HTMLInputElement;
        await fireEvent.input(field, {target: {value: "two words:x"}});
        await keydown(field, "Escape");
        await keydown(field, "Enter");
        await tick();
        expect(screen.getByRole("alert").textContent).toContain("name:value");
        expect(screen.queryAllByTestId("review-tag")).toHaveLength(0);
    });
});
