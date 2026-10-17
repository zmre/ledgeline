// The shared display-format golden (plan 26 B2): `fixtures/display-format/cases.json`.
//
// The engine carries a port of `displayPlaces`/`formatDec`/`formatAmount`
// (crates/ledgeline-core/src/display.rs) because search text, check messages
// and any future non-web UI must render an amount exactly as this UI does.
// Both implementations are held to the same table: this file asserts the
// TypeScript reproduces every row, crates/ledgeline-core/tests/display_format.rs
// asserts the Rust does. A row that only one side agrees with is the drift this
// exists to catch.
//
// The table is GENERATED from this implementation over a fixed grid. Regenerate
// only when the formatting rules change on purpose, and review the diff:
//
//     LEDGELINE_WRITE_DISPLAY_GOLDEN=1 bunx vitest run --project=unit displayFormat.golden

import {readFileSync, writeFileSync} from "node:fs";
import {describe, expect, it} from "vitest";
import {dec, displayPlaces, formatAmount, formatDec, MAX_DISPLAY_DECIMALS, MAX_QUANTITY_DECIMALS} from "./money";
import type {AmountStyle} from "./types";

const GOLDEN = new URL("../../../../fixtures/display-format/cases.json", import.meta.url);

/** The styles exercised: every side/spacing/mark/grouping shape hledger journals produce. */
const STYLES: readonly {commodity: string; style: AmountStyle}[] = [
    {commodity: "$", style: {side: "L", spaced: false, precision: 2, decimalPoint: ".", digitGroups: [",", [3]]}},
    {commodity: "EUR", style: {side: "R", spaced: true, precision: 2, decimalPoint: ",", digitGroups: [".", [3]]}},
    {commodity: "AAPL", style: {side: "R", spaced: true, precision: 4, decimalPoint: ".", digitGroups: null}},
    {commodity: "₹", style: {side: "L", spaced: false, precision: 2, decimalPoint: ".", digitGroups: [",", [3, 2]]}},
    {commodity: "JPY", style: {side: "L", spaced: true, precision: 0, decimalPoint: ".", digitGroups: [",", [3]]}},
    {commodity: "BTC", style: {side: "R", spaced: true, precision: 8, decimalPoint: ".", digitGroups: null}},
    {commodity: "", style: {side: "L", spaced: false, precision: 2, decimalPoint: ".", digitGroups: null}},
    {commodity: "green apples", style: {side: "R", spaced: true, precision: 1, decimalPoint: ".", digitGroups: null}},
    {commodity: "SEK", style: {side: "R", spaced: true, precision: 2, decimalPoint: ",", digitGroups: [" ", [3]]}},
    {commodity: "€", style: {side: "L", spaced: false, precision: 3, decimalPoint: ".", digitGroups: [",", [3]]}},
];

/** Mantissas: zero, signs, the half-way points, big values up to the wire's safe-integer limit. */
const MANTISSAS = [
    "0",
    "1",
    "-1",
    "4",
    "5",
    "-5",
    "49",
    "50",
    "51",
    "-50",
    "99",
    "100",
    "12345",
    "-12345",
    "999995",
    "1000000",
    "12345678",
    "123456789",
    "-987654321",
    "9007199254740991",
    "-9007199254740991",
];

/** Scales, including ones past both caps (the relaxed-decimals case lives at 3–10). */
const PLACES = [0, 1, 2, 3, 4, 8, 10];

const MAX_DECIMALS = [MAX_DISPLAY_DECIMALS, MAX_QUANTITY_DECIMALS];

type Row = [mantissa: string, places: number, style: number, maxDecimals: number, displayPlaces: number, formatDec: string, formatAmount: string];

function generate(): Row[] {
    const rows: Row[] = [];
    for (const m of MANTISSAS) {
        for (const p of PLACES) {
            const qty = dec(BigInt(m), p);
            STYLES.forEach(({commodity, style}, s) => {
                for (const max of MAX_DECIMALS) {
                    rows.push([m, p, s, max, displayPlaces(qty, style.precision, max), formatDec(qty, style, max), formatAmount({commodity, qty, style}, max)]);
                }
            });
        }
    }
    return rows;
}

interface Golden {
    styles: typeof STYLES;
    columns: string[];
    cases: Row[];
}

const COLUMNS = ["mantissa", "places", "style", "maxDecimals", "displayPlaces", "formatDec", "formatAmount"];

if (process.env.LEDGELINE_WRITE_DISPLAY_GOLDEN === "1") {
    const body = {
        _comment: [
            "Shared display-format golden (plan 26 B2), GENERATED from web/src/lib/domain/money.ts.",
            "Asserted by web/src/lib/domain/displayFormat.golden.test.ts AND crates/ledgeline-core/tests/display_format.rs.",
            "Each case is [mantissa, places, styleIndex, maxDecimals, displayPlaces, formatDec, formatAmount].",
            "Regenerate: LEDGELINE_WRITE_DISPLAY_GOLDEN=1 bunx vitest run --project=unit displayFormat.golden",
        ],
        styles: STYLES,
        columns: COLUMNS,
        cases: generate(),
    };
    writeFileSync(
        GOLDEN,
        JSON.stringify(body, null, 1).replace(
            /\[\n\s+("-?\d+"),\n\s+(\d+),\n\s+(\d+),\n\s+(\d+),\n\s+(\d+),\n\s+(".*?"),\n\s+(".*?")\n\s+\]/g,
            "[$1, $2, $3, $4, $5, $6, $7]"
        ) + "\n"
    );
}

describe("UNIT domain/money — the shared display-format golden", () => {
    const golden = JSON.parse(readFileSync(GOLDEN, "utf8")) as Golden;

    it("pins the grid this file generates, styles included", () => {
        expect(golden.columns).toEqual(COLUMNS);
        expect(golden.styles).toEqual(STYLES);
        expect(golden.cases.length).toBe(MANTISSAS.length * PLACES.length * STYLES.length * MAX_DECIMALS.length);
    });

    it("reproduces every row", () => {
        // One assertion over the whole table so a regression reports the first
        // differing row, not 2,940 failures.
        expect(generate()).toEqual(golden.cases);
    });

    it("covers the relaxed-decimals case — a value the money cap would round to zero", () => {
        expect(golden.cases.some(([m, p, s, max, places]) => m === "12345" && p === 8 && s === 0 && max === 2 && places === 8)).toBe(true);
    });
});
