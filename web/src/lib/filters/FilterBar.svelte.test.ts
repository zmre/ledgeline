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
import {keymap} from "#lib/keys/keymap.svelte.js";
import {filters} from "#lib/stores/filters.svelte.js";
import {problems} from "#lib/stores/problems.svelte.js";
import {running} from "#lib/stores/running.svelte.js";
import {FAKE_ENGINE, routes} from "#lib/testing/fakeEngine.js";

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

describe("COMPONENT FilterBar running-balance pin (plan 36)", () => {
    const ACCOUNT = "assets:bank:checking";

    /** Serve `/api/journal/running` with `body` (or a refusal) and load the store, as the journal page does. */
    async function loadRunning(body: Record<string, unknown> | Response): Promise<void> {
        vi.stubGlobal("fetch", (input: RequestInfo | URL) => routes({"/api/journal/running": body})(String(input)));
        await running.load(FAKE_ENGINE, {account: ACCOUNT, generation: "g1"});
    }

    const info = (commodities: string[], assertions: Record<string, unknown> = {}): Record<string, unknown> => ({
        generation: "g1",
        account: ACCOUNT,
        commodities: commodities.map((commodity, i) => ({commodity, postings: 10 - i})),
        assertions,
    });

    function press(key: string): void {
        const event = new KeyboardEvent("keydown", {key, bubbles: true, cancelable: true});
        document.body.dispatchEvent(event);
        keymap.handle(event);
    }

    afterEach(() => {
        vi.unstubAllGlobals();
        keymap.reset();
        problems.clearFocus();
    });

    it("shows its own chip, and ✕ removes the pin but keeps the account filter", async () => {
        filters.focusAccount(ACCOUNT);
        render(FilterBar, {accountNames: ACCOUNTS});
        await tick();
        expect(screen.getByTestId("running-pin-chip").textContent).toContain(`Running balance: ${ACCOUNT}`);

        screen.getByLabelText("Remove running balance").click();
        await tick();
        expect(screen.queryByTestId("running-pin-chip")).toBeNull();
        expect(filters.value.pin).toBeNull();
        expect([...filters.value.accounts]).toEqual([ACCOUNT]);
    });

    it("puts a ⚖ on each account chip that pins that account and keeps the filter", async () => {
        filters.setAccounts(new Set(["expenses:food", ACCOUNT]));
        render(FilterBar, {accountNames: ACCOUNTS});
        await tick();
        screen.getByLabelText(`Show running balance of ${ACCOUNT}`).click();
        await tick();
        expect(filters.value.pin).toEqual({account: ACCOUNT, commodity: null});
        expect(filters.value.accounts.size).toBe(2);
        // The pinned account's own chip no longer offers to pin it.
        expect(screen.queryByLabelText(`Show running balance of ${ACCOUNT}`)).toBeNull();
        expect(screen.getByLabelText("Show running balance of expenses:food")).toBeTruthy();
    });

    it("survives changing the account filter", async () => {
        filters.focusAccount(ACCOUNT);
        render(FilterBar, {accountNames: ACCOUNTS});
        await tick();
        screen.getByLabelText(`Remove account filter ${ACCOUNT}`).click();
        await vi.advanceTimersByTimeAsync(200);
        expect(filters.value.accounts.size).toBe(0);
        expect(screen.getByTestId("running-pin-chip")).toBeTruthy();
    });

    it("offers the commodity picker only when the subtree holds more than one", async () => {
        await loadRunning(info(["$"]));
        filters.focusAccount(ACCOUNT);
        const first = render(FilterBar, {accountNames: ACCOUNTS});
        await tick();
        expect(screen.queryByLabelText("Running balance commodity")).toBeNull();
        first.unmount();

        await loadRunning(info(["$", "EUR"]));
        render(FilterBar, {accountNames: ACCOUNTS});
        await tick();
        const select = screen.getByLabelText("Running balance commodity") as HTMLSelectElement;
        expect([...select.options].map((o) => o.value)).toEqual(["$", "EUR"]);
        select.value = "EUR";
        select.dispatchEvent(new Event("change", {bubbles: true}));
        expect(filters.value.pin).toEqual({account: ACCOUNT, commodity: "EUR"});
        // Back to the default stores no commodity, so the URL stays short.
        select.value = "$";
        select.dispatchEvent(new Event("change", {bubbles: true}));
        expect(filters.value.pin).toEqual({account: ACCOUNT, commodity: null});
    });

    it("links the first failing assertion, widening the dates only as needed, and asks the table to scroll", async () => {
        await loadRunning(
            info(["$"], {
                $: [
                    {tindex: 3, date: "2025-12-31", ok: true},
                    {tindex: 8, date: "2026-01-31", ok: false},
                    {tindex: 11, date: "2026-02-28", ok: false},
                ],
            })
        );
        filters.focusAccount(ACCOUNT);
        filters.setRange("2026-03-01", "2026-03-31");
        render(FilterBar, {accountNames: ACCOUNTS});
        await tick();
        const link = screen.getByTestId("running-first-failure");
        expect(link.textContent?.trim()).toBe("✗ first failure 2026-01-31");
        link.click();
        expect({from: filters.value.from, to: filters.value.to}).toEqual({from: "2026-01-31", to: "2026-03-31"});
        expect(problems.focusRequest?.txnIndex).toBe(8);
        expect([...filters.value.accounts]).toEqual([ACCOUNT]);
    });

    it("has no failure link when every assertion holds", async () => {
        await loadRunning(info(["$"], {$: [{tindex: 3, date: "2025-12-31", ok: true}]}));
        filters.focusAccount(ACCOUNT);
        render(FilterBar, {accountNames: ACCOUNTS});
        await tick();
        expect(screen.queryByTestId("running-first-failure")).toBeNull();
    });

    it("says so when the engine does not know the account", async () => {
        await loadRunning(new Response("no account matches", {status: 400}));
        filters.focusAccount(ACCOUNT);
        render(FilterBar, {accountNames: ACCOUNTS});
        await tick();
        expect(screen.getByTestId("running-pin-refused").textContent).toBe("not in this journal");
    });

    it("b pins the one filtered account, and unpins", async () => {
        filters.setAccounts(new Set([ACCOUNT]));
        render(FilterBar, {accountNames: ACCOUNTS});
        await tick();
        press("b");
        expect(filters.value.pin).toEqual({account: ACCOUNT, commodity: null});
        press("b");
        expect(filters.value.pin).toBeNull();
    });

    it("b is off with two accounts and no pin: there is no telling which was meant", async () => {
        filters.setAccounts(new Set([ACCOUNT, "expenses:food"]));
        render(FilterBar, {accountNames: ACCOUNTS});
        await tick();
        expect(keymap.active.some((binding) => binding.keys === "b")).toBe(false);
        press("b");
        expect(filters.value.pin).toBeNull();
    });
});
