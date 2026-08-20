// The Other holdings table, mounted.
//
// The pure half is covered in `view.test.ts` — `formatHeldCommodities` blanks a
// base-only balance, `sortOtherHoldings` keeps nulls last. Neither says the
// screen renders them, and that gap is exactly what the `components` vitest
// project exists for (see the rationale in `vite.config.ts`): the two bugs it
// was created after were both "the logic was right and the component was handed
// the wrong values".
//
// The claims worth mounting for here are: the Holding column really is blank for
// a dollar-booked asset, the tfoot really shows the ENGINE's totals rather than a
// sum of the rows, and the row cursor really reaches a row that exists.
//
// jsdom has no layout engine, so nothing below asks how anything LOOKS.

import {render, screen} from "@testing-library/svelte";
import {tick} from "svelte";
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import {decodeOtherHoldingsReport} from "$lib/api/nativeDecode";
import {formatDec, type Dec} from "$lib/domain/money";
import type {AmountStyle} from "$lib/domain/types";
import {keymap} from "$lib/keys/keymap.svelte";
import {formatUnitsWith} from "./view";
import OtherHoldingsTable from "./OtherHoldingsTable.svelte";

// The drill-down navigates, and a router is neither available nor the subject
// here — what matters is that Enter asks for THAT account.
const openJournal = vi.hoisted(() => vi.fn(() => Promise.resolve()));
vi.mock("$lib/journal/openJournal", () => ({openJournal}));

const MONEY: AmountStyle = {side: "L", spaced: false, precision: 2, decimalPoint: ".", digitGroups: [",", [3]]};
const UNITS: AmountStyle = {side: "R", spaced: true, precision: 0, decimalPoint: ".", digitGroups: null};

const format = (v: Dec): string => `$${formatDec(v, MONEY)}`;
const formatUnits = formatUnitsWith(() => UNITS);

const dec = (mantissa: number, places: number) => ({mantissa: String(mantissa), places});

// Decoded from a wire literal rather than hand-built, so this exercises the
// decode → render seam the way the page does.
const REPORT = decodeOtherHoldingsReport({
    asOf: "2026-07-08",
    base: "$",
    holdings: [
        {
            // Commodity-booked: the unit is what lets it revalue, so it must show.
            account: "assets:property:house",
            name: "Family home",
            commodities: {HOUSE: dec(1, 0)},
            value: dec(17500000, 2),
            cost: dec(15000000, 2),
            change: dec(2500000, 2),
            changePct: 16.67,
        },
        {
            // Dollar-booked: its Holding cell would only repeat the Value column.
            account: "assets:vehicles:van",
            name: "Van",
            commodities: {$: dec(1800000, 2)},
            value: dec(1800000, 2),
            cost: dec(1800000, 2),
            change: dec(0, 2),
            changePct: 0,
        },
        {
            // Unpriced: contributes to no total, raises the warning on the page.
            account: "assets:partners:acme",
            name: "Acme LP",
            commodities: {ACME: dec(5, 0)},
            value: null,
            cost: null,
            change: null,
            changePct: null,
        },
    ],
    // The scope chooser's options, not this table's rows — wider than `holdings`
    // by contract, and unused here (the page hands it to ScopeBar).
    accounts: ["assets:partners:acme", "assets:property:house", "assets:vehicles:van"],
    // Deliberately NOT the sum of the rows above: the point is that this table
    // prints what the engine sent and never recomputes it.
    totals: {value: dec(19300000, 2), cost: dec(16800000, 2), change: dec(2500000, 2), changePct: 14.88},
    warnings: [],
});

const mount = (gainPeriod: "all" | "ytd" | "12mo" = "all") =>
    render(OtherHoldingsTable, {holdings: [...REPORT.holdings], totals: REPORT.totals, base: REPORT.base, format, formatUnits, gainPeriod});

/** Press a key the way the app's window listener would. */
async function press(key: string): Promise<void> {
    keymap.handle(new KeyboardEvent("keydown", {key, cancelable: true}));
    await tick();
}

const cell = (account: string, at: number): string => screen.getByTestId(`other-holding-${account}`).children[at].textContent?.trim() ?? "";

beforeEach(() => {
    openJournal.mockClear();
});

afterEach(() => {
    keymap.reset();
});

describe("COMPONENT OtherHoldingsTable", () => {
    it("renders the seven columns in the contracted order", () => {
        mount();
        const headers = [...screen.getByTestId("other-holdings-table").querySelectorAll("thead tr > *")].map((h) => h.textContent?.trim());

        expect(headers).toEqual(["Name", "Account", "Holding", "Value", "Cost", "Change", "Change %"]);
    });

    it("puts all seven headers in <th scope='col'> cells, twinning HoldingsTable's header contract", async () => {
        mount();
        const cells = [...screen.getByTestId("other-holdings-table").querySelectorAll("thead tr > *")];

        // aria-sort is only valid on a columnheader/rowheader; as <td>s these
        // announced no sort state and gave the numeric cells below no column
        // association. HoldingsTable.svelte.test.ts pins the same claim on the twin.
        expect(cells.map((cell) => cell.tagName)).toEqual(Array(7).fill("TH"));
        expect(cells.map((cell) => cell.getAttribute("scope"))).toEqual(Array(7).fill("col"));

        screen.getByRole("button", {name: "Value"}).click();
        await tick();
        // The role query IS the assertion: on the old <td> markup no
        // columnheader named "Value" exists, so this line fails there.
        expect(screen.getByRole("columnheader", {name: "Value"}).getAttribute("aria-sort")).toBe("descending");
    });

    it("tags the Change header with the active window so a YTD figure isn't read as all-time", () => {
        mount("ytd");

        expect(screen.getByRole("button", {name: "Change (YTD)"})).toBeDefined();
    });

    it("shows the balance as written for a commodity-booked asset", () => {
        mount();

        expect(screen.getByTestId("held-assets:property:house").textContent?.trim()).toBe("1 HOUSE");
        expect(screen.getByTestId("held-assets:partners:acme").textContent?.trim()).toBe("5 ACME");
    });

    it("leaves the Holding cell blank for a dollar-booked asset, rather than repeating the Value column", () => {
        mount();

        expect(screen.getByTestId("held-assets:vehicles:van").textContent?.trim()).toBe("");
        // The value itself is still there, once, in its own column.
        expect(cell("assets:vehicles:van", 3)).toBe("$18,000.00");
    });

    it("renders an em-dash, not a zero, for an unpriced row's value/cost/change", () => {
        mount();

        expect(cell("assets:partners:acme", 3)).toBe("—");
        expect(cell("assets:partners:acme", 4)).toBe("—");
        expect(cell("assets:partners:acme", 5)).toBe("—");
        expect(cell("assets:partners:acme", 6)).toBe("—");
    });

    it("prints the ENGINE's totals in the tfoot and never a sum of the visible rows", () => {
        mount();
        const foot = screen.getByTestId("other-holdings-totals");

        expect(foot.children[0].textContent?.trim()).toBe("Total (3 holdings):");
        // The rows shown sum to $193,000.00 only because the engine says so —
        // $175,000 + $18,000 + (unpriced, contributing nothing).
        expect(foot.children[3].textContent?.trim()).toBe("$193,000.00");
        expect(foot.children[4].textContent?.trim()).toBe("$168,000.00");
    });

    it("keeps the engine's row order until a header is clicked, then sorts and says so", async () => {
        mount();
        const rows = (): string[] =>
            [...screen.getByTestId("other-holdings-table").querySelectorAll("tbody tr")].map((r) => r.getAttribute("data-testid") ?? "");

        expect(rows()).toEqual(["other-holding-assets:property:house", "other-holding-assets:vehicles:van", "other-holding-assets:partners:acme"]);

        screen.getByRole("button", {name: "Name"}).click();
        await tick();
        // Text columns start ascending; the unpriced row is not special here.
        expect(rows()).toEqual(["other-holding-assets:partners:acme", "other-holding-assets:property:house", "other-holding-assets:vehicles:van"]);
        expect(screen.getByRole("button", {name: /^Name/}).closest("th")?.getAttribute("aria-sort")).toBe("ascending");
    });

    it("sorts Value descending on the first click and keeps the unpriced row last in both directions", async () => {
        mount();
        const accounts = (): string[] =>
            [...screen.getByTestId("other-holdings-table").querySelectorAll("tbody tr")].map((r) =>
                (r.getAttribute("data-testid") ?? "").replace("other-holding-", "")
            );

        screen.getByRole("button", {name: "Value"}).click();
        await tick();
        expect(accounts()).toEqual(["assets:property:house", "assets:vehicles:van", "assets:partners:acme"]);

        screen.getByRole("button", {name: /^Value/}).click();
        await tick();
        expect(accounts()).toEqual(["assets:vehicles:van", "assets:property:house", "assets:partners:acme"]);
    });

    it("moves a row cursor with j/k and marks the current row", async () => {
        mount();

        await press("j");
        expect(screen.getByTestId("other-holding-assets:property:house").getAttribute("aria-current")).toBe("true");

        await press("j");
        expect(screen.getByTestId("other-holding-assets:vehicles:van").getAttribute("aria-current")).toBe("true");

        await press("k");
        expect(screen.getByTestId("other-holding-assets:property:house").getAttribute("aria-current")).toBe("true");
    });

    it("drills into the journal for the cursored ACCOUNT — one per row, unlike a stock spanning two brokerages", async () => {
        mount();

        await press("j");
        await press("Enter");

        expect(openJournal).toHaveBeenCalledWith({accounts: ["assets:property:house"], preset: "all"});
    });

    it("files its keys under Holdings, so one feature is not split across two help headings", async () => {
        mount();
        await tick();
        const groups = keymap.help.map((section) => section.group);

        expect(groups).toContain("Holdings");
        expect(groups).not.toContain("Journal");
    });
});
