// The Entries table: day headings with subtotals, billing in words, a
// locked line's explanation and invoice link, and the keys (j/k, Enter, x)
// and Shift+click picking a range.
import {createRawSnippet, tick} from "svelte";
import {SvelteSet} from "svelte/reactivity";
import {fireEvent, render, screen, within} from "@testing-library/svelte";
import {describe, expect, it, vi} from "vitest";
import {listenForKeys, press} from "#lib/testing/keys.js";
import {entry, lockedEntry, qty} from "../testing.js";
import EntryTable from "./EntryTable.svelte";
import {groupByDay} from "./group.js";

vi.mock("$app/paths", () => ({resolve: (path: string, params: Record<string, string> = {}) => `/app${path.replace("[number]", params.number ?? "")}`}));

const rows = [
    entry({id: "f@1", date: "2026-11-26", quantity: qty(60)}),
    entry({id: "f@2", date: "2026-11-26", account: "internal:admin", quantity: qty(15), billed: "not-billable", billable: false, customer: null, note: ""}),
    entry({id: "f@3", date: "2026-11-25", account: "clients:acme:tax-return", quantity: qty(130)}),
    lockedEntry(),
];

const editor = createRawSnippet(() => ({render: () => `<div data-testid="the-editor">editing</div>`}));

function mount(over: Record<string, unknown> = {}) {
    const props = {
        groups: groupByDay(rows),
        grouping: "day" as const,
        selected: new SvelteSet<string>(),
        editing: null,
        lockedNotice: null,
        editor,
        onEdit: vi.fn(),
        onDelete: vi.fn(),
        onToggle: vi.fn(),
        onToggleRange: vi.fn(),
        ...over,
    };
    render(EntryTable, props);
    return props;
}

describe("COMPONENT time EntryTable", () => {
    listenForKeys();

    it("groups by day with each day's total, newest first", () => {
        mount();
        const groups = screen.getAllByTestId("entry-group");
        expect(groups.map((g) => g.dataset.key)).toEqual(["2026-11-26", "2026-11-25", "2026-10-06"]);
        expect(within(groups[0]).getByText("Thu, Nov 26")).toBeDefined();
        expect(within(groups[0]).getByTestId("entry-group-total").textContent).toBe("1h 15m");
        const billing = screen.getAllByTestId("entry-billing").map((c) => c.textContent?.replace(/\s+/g, " ").trim());
        expect(billing).toEqual(["Unbilled", "Not billable", "Unbilled", "read-only Invoice 101"]);
    });

    it("a locked line is read-only: no delete, its reason and a link to the invoice", () => {
        mount({lockedNotice: "time/pat/2026-10.timedot@5"});
        const locked = screen.getAllByTestId("entry-row").find((r) => r.dataset.locked === "true");
        expect(locked).toBeDefined();
        expect(within(locked as HTMLElement).queryByTestId("entry-delete")).toBeNull();
        expect(
            within(locked as HTMLElement)
                .getByRole("link", {name: "Invoice 101"})
                .getAttribute("href")
        ).toBe("/app/invoices/101");
        const notice = screen.getByTestId("entry-locked-notice");
        expect(notice.textContent).toContain("can't be changed: it is on invoice 101, which is sent");
        expect(within(notice).getByRole("link", {name: "Open invoice 101"})).toBeDefined();
    });

    it("the line being edited is replaced by the editor", () => {
        mount({editing: "f@2"});
        expect(screen.getByTestId("the-editor")).toBeDefined();
        expect(screen.getAllByTestId("entry-row").map((r) => r.dataset.id)).not.toContain("f@2");
    });

    it("j/k move the cursor in the order shown; Enter edits; x picks", async () => {
        const props = mount();
        await press("j");
        await press("j");
        await tick();
        // Enter is the table's: the cursored row's button holds focus.
        await press("Enter", document.activeElement as HTMLElement);
        expect(props.onEdit).toHaveBeenCalledWith(expect.objectContaining({id: "f@2"}));
        await press("j");
        await press("x");
        expect(props.onToggle).toHaveBeenCalledWith(expect.objectContaining({id: "f@3"}));
    });

    it("Shift+click picks every line between two boxes", async () => {
        const props = mount();
        const boxes = screen.getAllByTestId("entry-pick");
        await fireEvent.click(boxes[0]);
        expect(props.onToggle).toHaveBeenCalledTimes(1);
        await fireEvent.click(boxes[2], {shiftKey: true});
        expect(props.onToggleRange).toHaveBeenCalledWith(["f@1", "f@2", "f@3"]);
    });
});
