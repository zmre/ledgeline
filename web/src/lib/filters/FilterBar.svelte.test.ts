// The filter bar's three inputs are all debounced now, because each change
// re-derives the whole journal view — 279 ms at 150k transactions in node under
// "All time", and this ships in WKWebView. The search box's window went 150 ms →
// 300 ms; the account tree (200 ms) and the date fields (250 ms) had none at all.
//
// The property that makes those delays free is the one worth testing: the
// CONTROL responds on the click, and only the expensive downstream work waits.
// So each test here asserts two things at once — the chip/checkbox is already
// updated, and `filters.value` is not yet.

import {render, screen} from "@testing-library/svelte";
import {tick} from "svelte";
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import FilterBar from "./FilterBar.svelte";
import {filters} from "$lib/stores/filters.svelte";

const ACCOUNTS = ["assets:bank:checking", "expenses:food", "expenses:rent"];

/**
 * Open the account dropdown.
 *
 * Driven through the <details> element rather than by clicking <summary>: jsdom
 * does not implement summary activation, so a synthetic click leaves it shut.
 * `bind:open` listens for `toggle`, which is what actually renders the tree —
 * the rows are built lazily now, so nothing exists to click before this runs.
 * The search input is anchored on because it is mounted either way.
 */
async function openAccountTree(): Promise<void> {
    const details = screen.getByLabelText("Search accounts").closest("details") as HTMLDetailsElement;
    details.open = true;
    details.dispatchEvent(new Event("toggle"));
    await tick();
}

beforeEach(() => {
    vi.useFakeTimers();
    filters.reset();
});

afterEach(() => {
    vi.useRealTimers();
});

describe("COMPONENT FilterBar account selection is debounced but never looks it", () => {
    it("shows the chip immediately and commits to the store only after the window", async () => {
        render(FilterBar, {accountNames: ACCOUNTS});

        // Open the tree (its rows are built lazily, on open) and tick a box.
        await openAccountTree();
        const box = screen.getByRole("checkbox", {name: "expenses"});
        await box.click();
        await tick();

        // Visible straight away: the chip is driven by the component's optimistic
        // set, not by the store.
        expect(screen.getByLabelText("Remove account filter expenses")).toBeTruthy();
        // ...and the expensive half has NOT run yet.
        expect(filters.value.accounts.size).toBe(0);

        await vi.advanceTimersByTimeAsync(200);
        expect([...filters.value.accounts]).toEqual(["expenses"]);
    });

    it("coalesces a burst of toggles into ONE filter change", async () => {
        render(FilterBar, {accountNames: ACCOUNTS});
        await openAccountTree();

        // Three clicks inside one window — the case the debounce exists for.
        // Rows are labelled with the LEAF segment; the store records full names.
        for (const name of ["food", "rent", "checking"]) {
            await screen.getByRole("checkbox", {name}).click();
            await tick();
            await vi.advanceTimersByTimeAsync(50);
        }
        expect(filters.value.accounts.size).toBe(0);

        await vi.advanceTimersByTimeAsync(200);
        expect([...filters.value.accounts].sort()).toEqual(["assets:bank:checking", "expenses:food", "expenses:rent"]);
    });

    it("a queued selection is flushed, not dropped, when the bar unmounts", async () => {
        const {unmount} = render(FilterBar, {accountNames: ACCOUNTS});
        await openAccountTree();
        await screen.getByRole("checkbox", {name: "expenses"}).click();
        await tick();

        unmount();

        // The user saw the box tick; losing that on navigation would be a bug,
        // not an optimization.
        expect([...filters.value.accounts]).toEqual(["expenses"]);
    });
});

describe("COMPONENT FilterBar date fields are debounced", () => {
    it("does not commit the intermediate dates a native date input emits mid-typing", async () => {
        render(FilterBar, {accountNames: ACCOUNTS});
        const from = screen.getByLabelText("From date") as HTMLInputElement;

        // Typing a year fires `change` for each valid intermediate date. Only the
        // last one should ever reach the store.
        for (const value of ["0002-01-01", "0020-01-01", "0202-01-01", "2026-01-01"]) {
            from.value = value;
            from.dispatchEvent(new Event("change", {bubbles: true}));
            await tick();
            await vi.advanceTimersByTimeAsync(30);
        }
        expect(filters.value.from).toBe(defaultFrom);

        await vi.advanceTimersByTimeAsync(250);
        expect(filters.value.from).toBe("2026-01-01");
    });
});

// Captured before any test mutates the store, so the assertion above compares
// against the real default rather than a hard-coded date that ages.
const defaultFrom = filters.value.from;
