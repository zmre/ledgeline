// The generic combobox: matching, the key table, and the "Add new" entry.
// Positioning is anchoredPopup's (pure) and Playwright's, as for AccountInput.
import {fireEvent, render, screen} from "@testing-library/svelte";
import {tick} from "svelte";
import {describe, expect, it, vi} from "vitest";
import Combobox from "./Combobox.svelte";
import {matchOptions, type ComboOption} from "../ui/comboMatch";

const CUSTOMERS: ComboOption[] = [
    {value: "[[Customer X]]", label: "Customer X", detail: "organization"},
    {value: "[[Contoso]]", label: "Contoso", detail: "organization"},
    {value: "[[Fred Smith]]", label: "Fred Smith", detail: "person"},
];

function mount(props: Record<string, unknown> = {}) {
    const view = render(Combobox, {props: {options: CUSTOMERS, label: "Customer", ...props}});
    return {view, field: screen.getByLabelText("Customer") as HTMLInputElement};
}

async function type(field: HTMLInputElement, text: string): Promise<void> {
    await fireEvent.input(field, {target: {value: text}});
    await tick();
}

function key(field: HTMLElement, k: string): boolean {
    const event = new KeyboardEvent("keydown", {key: k, bubbles: true, cancelable: true});
    field.dispatchEvent(event);
    return event.defaultPrevented;
}

const shown = (): string[] => screen.queryAllByRole("option").map((o) => o.textContent?.replace(/\s+/g, " ").trim() ?? "");

describe("matchOptions", () => {
    it("matches label, detail and value case-insensitively; empty text lists all", () => {
        expect(matchOptions("con", CUSTOMERS).map((o) => o.label)).toEqual(["Contoso"]);
        expect(matchOptions("PERSON", CUSTOMERS).map((o) => o.label)).toEqual(["Fred Smith"]);
        expect(matchOptions("", CUSTOMERS)).toHaveLength(3);
        expect(matchOptions("", CUSTOMERS, 2)).toHaveLength(2);
    });
});

describe("COMPONENT Combobox", () => {
    it("opens on typing, not on focus, and filters", async () => {
        const {field} = mount();
        expect(screen.queryByRole("listbox")).toBeNull();
        await type(field, "x");
        expect(shown()).toEqual(["Customer X organization"]);
        expect(field.getAttribute("aria-expanded")).toBe("true");
    });

    it("moves with the arrows and chooses with Enter", async () => {
        const onChoose = vi.fn();
        const {field} = mount({onChoose});
        expect(key(field, "ArrowDown")).toBe(true);
        await tick();
        expect(shown()).toHaveLength(3);
        key(field, "ArrowDown");
        await tick();
        expect(field.getAttribute("aria-activedescendant")).toMatch(/-1$/);
        expect(key(field, "Enter")).toBe(true);
        await tick();
        expect(field.value).toBe("[[Contoso]]");
        expect(onChoose).toHaveBeenCalledWith(CUSTOMERS[1]);
        expect(screen.queryByRole("listbox")).toBeNull();
    });

    it("closes the popup on the first Escape and lets the second through", async () => {
        const {field} = mount();
        await type(field, "c");
        expect(key(field, "Escape")).toBe(true);
        await tick();
        expect(screen.queryByRole("listbox")).toBeNull();
        expect(key(field, "Escape")).toBe(false);
    });

    it("offers Add new for text no option names, and hands it over", async () => {
        const onAddNew = vi.fn();
        const {field} = mount({onAddNew, addNewLabel: (t: string) => `Add new customer “${t}”…`});
        await type(field, "Northwind");
        expect(shown()).toEqual(["Add new customer “Northwind”…"]);
        key(field, "Enter");
        await tick();
        expect(onAddNew).toHaveBeenCalledWith("Northwind");

        await type(field, "Contoso");
        expect(shown()).toEqual(["Contoso organization"]);
    });

    it("commits on Enter with the list closed, and Tab never traps", async () => {
        const onCommit = vi.fn();
        const {field} = mount({onCommit});
        expect(key(field, "Enter")).toBe(false);
        expect(onCommit).toHaveBeenCalledTimes(1);
        await type(field, "c");
        expect(key(field, "Tab")).toBe(false);
        await tick();
        expect(screen.queryByRole("listbox")).toBeNull();
    });
});
