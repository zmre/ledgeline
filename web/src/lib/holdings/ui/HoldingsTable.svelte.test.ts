// The stock holdings table, mounted — narrowly, for its HEADER semantics.
//
// The sorting arithmetic lives in `view.test.ts` and the rows/totals discipline
// is pinned on OtherHoldingsTable (this table's deliberate twin — see the
// comment atop OtherHoldingsTable.svelte). What THIS file exists for is the
// accessibility contract the header row carries: `aria-sort` is only valid on a
// columnheader/rowheader, so every header cell must be a real `<th scope="col">`.
// As `<td>`s — how they were first written — validators flagged the attribute,
// screen readers announced no sort state, and the numeric cells below had no
// column association at all. jsdom computes ARIA roles without a layout engine,
// which is exactly the half of the claim it can hold.

import {render, screen} from "@testing-library/svelte";
import {tick} from "svelte";
import {afterEach, describe, expect, it, vi} from "vitest";
import {dec, formatDec, type Dec} from "$lib/domain/money";
import type {AmountStyle} from "$lib/domain/types";
import type {Holding, HoldingsReport} from "$lib/holdings/types";
import {keymap} from "$lib/keys/keymap.svelte";
import HoldingsTable from "./HoldingsTable.svelte";

// Enter drills into the journal; a router is neither available nor the subject.
vi.mock("$lib/journal/openJournal", () => ({openJournal: vi.fn(() => Promise.resolve())}));

const MONEY: AmountStyle = {side: "L", spaced: false, precision: 2, decimalPoint: ".", digitGroups: [",", [3]]};
const format = (v: Dec): string => `$${formatDec(v, MONEY)}`;

const AAPL: Holding = {
    symbol: "AAPL",
    name: "Apple",
    accounts: ["assets:broker:taxable"],
    shares: dec(10, 0),
    basis: dec(150000, 2),
    firstBasisDate: "2025-01-15",
    price: {qty: dec(20000, 2), date: "2026-07-01", source: "directive"},
    marketValue: dec(200000, 2),
    gain: dec(50000, 2),
    gainPct: 33.33,
};

// Unpriced, so the table's null branches render too — the header claims must
// hold over em-dash cells as much as over money.
const GLD: Holding = {
    symbol: "GLD",
    name: "GLD",
    accounts: ["assets:broker:ira"],
    shares: dec(5, 0),
    basis: null,
    firstBasisDate: null,
    price: null,
    marketValue: null,
    gain: null,
    gainPct: null,
};

const TOTALS: HoldingsReport["totals"] = {marketValue: dec(200000, 2), basis: null, gain: null, gainPct: null};

const mount = () => render(HoldingsTable, {holdings: [AAPL, GLD], totals: TOTALS, format});

afterEach(() => {
    keymap.reset();
});

describe("COMPONENT HoldingsTable", () => {
    it("puts all nine headers in <th scope='col'> cells, in the contracted order", () => {
        mount();
        const cells = [...screen.getByTestId("holdings-table").querySelectorAll("thead tr > *")];

        // A <td> here is a defect even when it renders identically: aria-sort is
        // undefined on it, and the column loses its header association.
        expect(cells.map((cell) => cell.tagName)).toEqual(Array(9).fill("TH"));
        expect(cells.map((cell) => cell.getAttribute("scope"))).toEqual(Array(9).fill("col"));
        expect(cells.map((cell) => cell.textContent?.trim())).toEqual([
            "Name",
            "Symbol",
            "Shares",
            "Basis",
            "Price",
            "Price date",
            "Market value",
            "Gain",
            "Gain %",
        ]);
    });

    it("announces the sort on a COLUMNHEADER role, the only role aria-sort is valid on", async () => {
        mount();

        // Until a header is clicked, no column claims a sort at all.
        expect(screen.getByTestId("holdings-table").querySelectorAll("[aria-sort]")).toHaveLength(0);

        screen.getByRole("button", {name: "Market value"}).click();
        await tick();

        // The role query IS the assertion: on the old <td> markup no
        // columnheader named "Market value" exists, so this line fails there.
        expect(screen.getByRole("columnheader", {name: "Market value"}).getAttribute("aria-sort")).toBe("descending");

        // The arrow span is aria-hidden, so the accessible name never grows a "▼".
        screen.getByRole("button", {name: "Market value"}).click();
        await tick();
        expect(screen.getByRole("columnheader", {name: "Market value"}).getAttribute("aria-sort")).toBe("ascending");

        // Exactly one column carries a state; the rest stay silent.
        expect(screen.getByTestId("holdings-table").querySelectorAll("[aria-sort]")).toHaveLength(1);
    });
});
