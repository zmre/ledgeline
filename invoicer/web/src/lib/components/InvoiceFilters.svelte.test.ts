// The invoice list's filter bar (PLAN 5b, S1–S3), mounted: it shows the
// URL's state and asks for a new one — choices as a history step, the search
// debounced in place, Enter as the quick jump, Clear filters only when a
// filter is on.
import {fireEvent, render, screen} from "@testing-library/svelte";
import {afterEach, describe, expect, it, vi} from "vitest";
import {parseInvoiceQuery} from "#lib/listQuery.js";
import InvoiceFilters from "./InvoiceFilters.svelte";

const query = (qs: string) => parseInvoiceQuery(new URLSearchParams(qs));
const customers = ["Contoso", "Customer X", "Wingtip"];

describe("COMPONENT InvoiceFilters", () => {
    afterEach(() => vi.useRealTimers());

    it("shows the URL's filters", () => {
        render(InvoiceFilters, {
            query: query("status=overdue&customer=Wingtip&from=2026-08-01&to=2026-08-31&aged=61-90&q=106"),
            customers,
            onChange: vi.fn(),
            onJump: vi.fn(),
            onClear: vi.fn(),
        });
        expect((screen.getByTestId("invoice-filter-status") as HTMLSelectElement).value).toBe("overdue");
        expect((screen.getByTestId("invoice-search") as HTMLInputElement).value).toBe("106");
        expect((screen.getByTestId("invoice-filter-from") as HTMLInputElement).value).toBe("2026-08-01");
        expect((screen.getByTestId("invoice-filter-to") as HTMLInputElement).value).toBe("2026-08-31");
        expect((screen.getByTestId("invoice-filter-aged") as HTMLSelectElement).value).toBe("61-90");
        expect((screen.getByLabelText("Customer") as HTMLInputElement).value).toBe("Wingtip");
        expect(screen.getByTestId("invoice-filters-clear")).toBeDefined();
    });

    it("hides Clear filters when nothing is filtered, and shows an age it has no bucket for", () => {
        render(InvoiceFilters, {query: query("aged=10-20"), customers, onChange: vi.fn(), onJump: vi.fn(), onClear: vi.fn()});
        const aged = screen.getByTestId("invoice-filter-aged") as HTMLSelectElement;
        expect(aged.value).toBe("10-20");
        expect(aged.selectedOptions[0].textContent).toBe("10–20 days late");
        render(InvoiceFilters, {query: query("sort=number&dir=asc"), customers, onChange: vi.fn(), onJump: vi.fn(), onClear: vi.fn()});
        expect(screen.getAllByTestId("invoice-filters-clear")).toHaveLength(1);
    });

    it("asks for a history step per choice", async () => {
        const onChange = vi.fn();
        render(InvoiceFilters, {query: query("q=x"), customers, onChange, onJump: vi.fn(), onClear: vi.fn()});
        await fireEvent.change(screen.getByTestId("invoice-filter-status"), {target: {value: "paid"}});
        expect(onChange).toHaveBeenLastCalledWith(expect.objectContaining({status: "paid", q: "x"}), {replace: false});
        await fireEvent.change(screen.getByTestId("invoice-filter-from"), {target: {value: "2026-10-01"}});
        expect(onChange).toHaveBeenLastCalledWith(expect.objectContaining({from: "2026-10-01"}), {replace: false});
        await fireEvent.change(screen.getByTestId("invoice-filter-aged"), {target: {value: "91+"}});
        expect(onChange).toHaveBeenLastCalledWith(expect.objectContaining({aged: "91+"}), {replace: false});
    });

    it("debounces the search in place, and Enter jumps", async () => {
        vi.useFakeTimers();
        const onChange = vi.fn();
        const onJump = vi.fn();
        render(InvoiceFilters, {query: query(""), customers, onChange, onJump, onClear: vi.fn(), debounceMs: 100});
        const search = screen.getByTestId("invoice-search");
        await fireEvent.input(search, {target: {value: "PY75"}});
        expect(onChange).not.toHaveBeenCalled();
        vi.advanceTimersByTime(100);
        expect(onChange).toHaveBeenCalledExactlyOnceWith(expect.objectContaining({q: "PY75"}), {replace: true});
        await fireEvent.input(search, {target: {value: "105"}});
        await fireEvent.keyDown(search, {key: "Enter"});
        expect(onChange).toHaveBeenLastCalledWith(expect.objectContaining({q: "105"}), {replace: true});
        expect(onJump).toHaveBeenCalledExactlyOnceWith("105");
    });

    it("follows the URL: Back to another search shows its text", async () => {
        const props = $state({query: query("q=contoso"), customers, onChange: vi.fn(), onJump: vi.fn(), onClear: vi.fn()});
        render(InvoiceFilters, props);
        props.query = query("q=wingtip&status=void");
        await Promise.resolve();
        await new Promise((r) => setTimeout(r, 0));
        expect((screen.getByTestId("invoice-search") as HTMLInputElement).value).toBe("wingtip");
        expect((screen.getByTestId("invoice-filter-status") as HTMLSelectElement).value).toBe("void");
    });

    it("clears", async () => {
        const onClear = vi.fn();
        render(InvoiceFilters, {query: query("status=draft"), customers, onChange: vi.fn(), onJump: vi.fn(), onClear});
        await fireEvent.click(screen.getByTestId("invoice-filters-clear"));
        expect(onClear).toHaveBeenCalledOnce();
    });
});
