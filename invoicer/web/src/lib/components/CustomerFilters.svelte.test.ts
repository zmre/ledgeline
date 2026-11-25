// The customer list's filter bar (PLAN 5b, S2), mounted: toggles say whether
// they are on (aria-pressed), each choice is a history step.
import {fireEvent, render, screen} from "@testing-library/svelte";
import {describe, expect, it, vi} from "vitest";
import {parseCustomerQuery} from "#lib/listQuery.js";
import CustomerFilters from "./CustomerFilters.svelte";

const query = (qs: string) => parseCustomerQuery(new URLSearchParams(qs));

describe("COMPONENT CustomerFilters", () => {
    it("shows the URL's filters and toggles them", async () => {
        const onChange = vi.fn();
        render(CustomerFilters, {query: query("owes=true&kind=organization&q=con"), onChange, onJump: vi.fn(), onClear: vi.fn()});
        const owes = screen.getByTestId("customer-filter-owes");
        const overdue = screen.getByTestId("customer-filter-overdue");
        expect(owes.getAttribute("aria-pressed")).toBe("true");
        expect(overdue.getAttribute("aria-pressed")).toBe("false");
        expect((screen.getByTestId("customer-filter-kind") as HTMLSelectElement).value).toBe("organization");
        expect((screen.getByTestId("customer-search") as HTMLInputElement).value).toBe("con");
        await fireEvent.click(overdue);
        expect(onChange).toHaveBeenLastCalledWith(expect.objectContaining({owes: true, overdue: true}), {replace: false});
        await fireEvent.click(owes);
        expect(onChange).toHaveBeenLastCalledWith(expect.objectContaining({owes: false}), {replace: false});
        await fireEvent.change(screen.getByTestId("customer-filter-kind"), {target: {value: "person"}});
        expect(onChange).toHaveBeenLastCalledWith(expect.objectContaining({kind: "person"}), {replace: false});
    });

    it("offers Clear filters only when filtered, and Enter in the search jumps", async () => {
        const onJump = vi.fn();
        const onChange = vi.fn();
        render(CustomerFilters, {query: query(""), onChange, onJump, onClear: vi.fn()});
        expect(screen.queryByTestId("customer-filters-clear")).toBeNull();
        const search = screen.getByTestId("customer-search");
        await fireEvent.input(search, {target: {value: "fred"}});
        await fireEvent.keyDown(search, {key: "Enter"});
        expect(onChange).toHaveBeenCalledExactlyOnceWith(expect.objectContaining({q: "fred"}), {replace: true});
        expect(onJump).toHaveBeenCalledExactlyOnceWith("fred");
    });
});
