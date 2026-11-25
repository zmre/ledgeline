// The journal's recognition chrome around the rows (plan 45 D8, Q7, Q11):
// the filter bar's Recognition chip (only with the helpers on; it cycles
// Grouped → Hidden → Shown into the `recognitionRows` preference), the
// footer's "incl. N hidden recognition entries", Top Transactions' one line
// per month of entries (opening that month's group in the journal), and the
// URL's `expand=` seed.

import {fireEvent, render, screen} from "@testing-library/svelte";
import {goto} from "$app/navigation";
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import {accrual, resetAccrual} from "#lib/accrual/store.svelte.js";
import {dec} from "#lib/domain/money.js";
import FilterBar from "#lib/filters/FilterBar.svelte";
import {keymap} from "#ui/keys/keymap.svelte.js";
import TopTxnsList from "#lib/reports/ui/insights/TopTxnsList.svelte";
import {filters} from "#lib/stores/filters.svelte.js";
import {settings} from "#lib/stores/settings.svelte.js";
import {connectFakeEngine, FAKE_ENGINE} from "#lib/testing/fakeEngine.js";
import {expandKeyOf, parseExpand, recognitionExpand, resetRecognitionExpand} from "./recognitionExpand.svelte";
import TotalsFooter from "./TotalsFooter.svelte";

vi.mock("$app/navigation", () => ({goto: vi.fn()}));
vi.mock("$app/paths", () => ({resolve: (path: string) => path}));

/** A minimal `/api/accrual` with the helpers on. */
const ACCRUAL_ON = {
    enabled: true,
    generation: "g1",
    basis: {value: "accrual", source: "inferred"},
    reason: {kind: "declared", account: "liabilities:deferred-revenue", role: "deferred-revenue"},
    roles: {"liabilities:deferred-revenue": {role: "deferred-revenue", declared: true}},
    file: {path: "recognition.journal", exists: true, included: false},
    counts: {due: 0, dueClosed: 0, needsSchedule: 0, invoices: 0},
    warnings: [],
};

async function gate(body: unknown): Promise<void> {
    await connectFakeEngine({"/api/accrual": body});
    await accrual.sync(FAKE_ENGINE, `g-${Math.random()}`);
}

beforeEach(() => {
    resetAccrual();
    resetRecognitionExpand();
});
afterEach(() => {
    vi.unstubAllGlobals();
    vi.mocked(goto).mockClear();
    keymap.reset();
    resetAccrual();
    resetRecognitionExpand();
    settings.recognitionRows = "group";
});

describe("COMPONENT the Recognition chip (D8)", () => {
    it("is not there when the helpers are off", async () => {
        await gate({enabled: false});
        render(FilterBar, {accountNames: []});
        expect(accrual.enabled).toBe(false);
        expect(screen.queryByTestId("recognition-chip")).toBeNull();
    });

    it("cycles Grouped → Hidden → Shown into the recognitionRows preference", async () => {
        await gate(ACCRUAL_ON);
        render(FilterBar, {accountNames: []});
        const chip = await screen.findByTestId("recognition-chip");
        expect(chip.textContent).toContain("Recognition: Grouped");
        await fireEvent.click(chip);
        expect(settings.recognitionRows).toBe("hide");
        expect(screen.getByTestId("recognition-chip").textContent).toContain("Hidden");
        await fireEvent.click(screen.getByTestId("recognition-chip"));
        expect(settings.recognitionRows).toBe("show");
        await fireEvent.click(screen.getByTestId("recognition-chip"));
        expect(settings.recognitionRows).toBe("group");
    });
});

describe("COMPONENT the footer's hidden entries (Q11)", () => {
    it("says how many entries the totals include but the rows hide, and nothing when none are", () => {
        const {rerender} = render(TotalsFooter, {count: 12, period: "Nov 2026", total: [], noun: "row", hiddenRecognition: 412});
        expect(screen.getByTestId("footer-recognition-hidden").textContent).toBe("incl. 412 hidden recognition entries");
        void rerender({count: 12, period: "Nov 2026", total: [], noun: "row", hiddenRecognition: 0});
        return Promise.resolve().then(() => expect(screen.queryByTestId("footer-recognition-hidden")).toBeNull());
    });
});

describe("COMPONENT Top Transactions' recognition line (Q7)", () => {
    const rows = [
        {index: 9, date: "2026-11-02", description: "Big invoice", amount: dec(5_000_000, 2), recognition: null},
        {index: 20, date: "2026-11-17", description: "Customer X Recognized", amount: dec(4_125_000, 2), recognition: {month: "2026-11", count: 37}},
    ];

    it("reads as the month, its entries and their total, and opens the month's group in the journal", async () => {
        render(TopTxnsList, {rows, base: "$", styles: new Map()});
        const line = screen.getByTestId("top-txn-recognition");
        expect(line.textContent).toContain("Recognition, Nov");
        expect(line.textContent).toContain("37 entries");
        expect(screen.getByText("Big invoice")).toBeTruthy();
        await fireEvent.click(line);
        expect(filters.value.from).toBe("2026-11-01");
        expect(filters.value.to).toBe("2026-11-30");
        expect(recognitionExpand.months).toEqual(["2026-11"]);
        expect(vi.mocked(goto)).toHaveBeenCalledWith(expect.stringContaining("expand=2026-11"));
        // The seed belongs to that filter: the page's first follow keeps it, a new filter drops it.
        recognitionExpand.follow(expandKeyOf(filters.value));
        expect(recognitionExpand.months).toEqual(["2026-11"]);
        recognitionExpand.follow(expandKeyOf({...filters.value, query: "rent"}));
        expect(recognitionExpand.months).toEqual([]);
    });
});

describe("UNIT parseExpand", () => {
    it("keeps valid months once, sorted, and drops the rest", () => {
        expect(parseExpand("2026-11,2026-10,2026-11, nope,2026-1")).toEqual(["2026-10", "2026-11"]);
        expect(parseExpand(null)).toEqual([]);
    });
});
