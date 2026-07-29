// Read-back verification for the xlsx builder (WP-07 DoD): build a workbook,
// serialize it, re-load it with exceljs, and assert title row, headers, cell
// values, and number formats survive the round trip.

import {describe, expect, it} from "vitest";
import {Workbook} from "exceljs";
import {dec, type MixedAmount} from "$lib/domain/money";
import type {AccountType} from "$lib/domain/accountTypes";
import type {Holding, HoldingsReport} from "$lib/holdings/types";
import type {BudgetCell, BudgetReport, PeriodReport, SectionedReport} from "$lib/reports/types";
import {buildBudgetWorkbook, buildHoldingsWorkbook, buildWorkbook, numberFormat} from "./xlsx";

const usd = (cents: number): MixedAmount => new Map([["$", dec(cents, 2)]]);
const amt = (m: number, p: number): MixedAmount => new Map([["$", dec(m, p)]]);

async function readBack(built: Workbook, title: string) {
    const buffer = await built.xlsx.writeBuffer();
    const loaded = new Workbook();
    await loaded.xlsx.load(buffer as never);
    const ws = loaded.getWorksheet(title);
    if (ws === undefined) throw new Error(`worksheet "${title}" missing after round trip`);
    return ws;
}

async function roundTrip(report: SectionedReport | PeriodReport, meta: {title: string; params: string}) {
    return readBack(await buildWorkbook(report, meta), meta.title);
}

describe("UNIT export/xlsx", () => {
    describe("numberFormat", () => {
        it("derives decimals from the Dec's places (capped at 2) and affixes the commodity", () => {
            expect(numberFormat("$", 2)).toBe('"$"#,##0.00');
            expect(numberFormat("EUR", 0)).toBe('#,##0 "EUR"');
            expect(numberFormat("", 3)).toBe("#,##0.00"); // display cap: never more than 2 decimals
        });

        // SEC-13: commodities are journal-derived (user-controlled) and are
        // interpolated into styles.xml. Everything outside the allowlist must
        // degrade to the bare numeric format rather than emit a format code.
        describe("commodity allowlist (SEC-13)", () => {
            it("accepts the currency symbols and ticker shapes it is meant to", () => {
                expect(numberFormat("€", 2)).toBe('"€"#,##0.00');
                expect(numberFormat("£", 2)).toBe('"£"#,##0.00');
                expect(numberFormat("¥", 0)).toBe('"¥"#,##0');
                expect(numberFormat("USD", 2)).toBe('#,##0.00 "USD"');
                expect(numberFormat("AAPL", 2)).toBe('#,##0.00 "AAPL"');
                expect(numberFormat("BRK.B", 2)).toBe('#,##0.00 "BRK.B"'); // `.` is allowlisted
                expect(numberFormat("VTSAX", 4)).toBe('#,##0.00 "VTSAX"');
                expect(numberFormat("0DTE", 2)).toBe('#,##0.00 "0DTE"'); // digits allowed
            });

            // The metacharacter set named in CLEANUP.md, one case each, plus the
            // quote that the old code tried (and was alone in trying) to escape.
            it.each([
                ["backslash", "US\\D"],
                ["open bracket", "US[D"],
                ["close bracket", "US]D"],
                ["semicolon", "US;D"],
                ["underscore", "US_D"],
                ["asterisk", "US*D"],
                ["at sign", "US@D"],
                ["double quote", 'US"D'],
            ])("rejects %s and falls back to the bare format", (_name, commodity) => {
                expect(numberFormat(commodity, 2)).toBe("#,##0.00");
            });

            it("rejects a crafted section-splitting payload outright", () => {
                // Pre-fix this produced `#,##0.00 "a";;;"` — a format code with
                // attacker-chosen sections and an unbalanced trailing quote.
                expect(numberFormat('a";;;"', 2)).toBe("#,##0.00");
                expect(numberFormat('";[Red]General;"', 2)).toBe("#,##0.00");
            });

            it("rejects other structural and control characters", () => {
                for (const c of ["USD EUR", "US\nD", "US\tD", "US/D", "US?D", "US#D", "US%D", "US-D", "US:D", "US,D", "US(D)"]) {
                    expect(numberFormat(c, 2)).toBe("#,##0.00");
                }
            });

            it("still returns the bare format for an empty commodity", () => {
                expect(numberFormat("", 2)).toBe("#,##0.00");
                expect(numberFormat("", 0)).toBe("#,##0");
            });

            it("keeps the fallback consistent with the requested decimal places", () => {
                expect(numberFormat("US;D", 0)).toBe("#,##0");
                expect(numberFormat("US;D", 1)).toBe("#,##0.0");
                expect(numberFormat("US;D", 9)).toBe("#,##0.00"); // display cap still applies
            });

            it("never emits an unbalanced quote for any single-character commodity", () => {
                // Exhaustive over ASCII: whatever comes back must have an even
                // number of quote characters, i.e. every literal is closed.
                for (let i = 0; i < 128; i += 1) {
                    const fmt = numberFormat(String.fromCharCode(i), 2);
                    const quotes = (fmt.match(/"/g) ?? []).length;
                    expect(quotes % 2, `unbalanced quotes for charCode ${i}: ${fmt}`).toBe(0);
                }
            });
        });
    });

    it("sectioned report: title, params, headers, section rows, totals, numFmt", async () => {
        const report: SectionedReport = {
            asOf: "2026-07-08",
            sections: [
                {
                    title: "Assets",
                    rows: [
                        {account: "assets", depth: 1, own: new Map(), inclusive: usd(123456)},
                        {account: "assets:bank", depth: 2, own: new Map(), inclusive: usd(123456)},
                        {account: "assets:bank:checking", depth: 3, own: usd(123456), inclusive: usd(123456)},
                    ],
                    total: usd(123456),
                },
                {
                    title: "Liabilities",
                    rows: [{account: "liabilities", depth: 1, own: usd(20000), inclusive: usd(20000)}],
                    total: usd(20000),
                },
            ],
            grandTotal: usd(103456),
        };
        const ws = await roundTrip(report, {title: "Balance Sheet", params: "as of 2026-07-08, depth 3"});

        expect(ws.getCell("A1").value).toBe("Balance Sheet");
        expect(ws.getCell("A1").font.bold).toBe(true);
        expect(ws.getCell("A2").value).toBe("as of 2026-07-08, depth 3");
        expect([ws.getCell("A4").value, ws.getCell("B4").value]).toEqual(["Account", "Amount"]);
        expect(ws.getCell("A4").font.bold).toBe(true);

        // Section content: single-child chain compressed to one row.
        expect(ws.getCell("A5").value).toBe("Assets");
        expect(ws.getCell("A6").value).toBe("assets:bank:checking");
        expect(ws.getCell("B6").value).toBe(1234.56);
        expect(ws.getCell("B6").numFmt).toBe('"$"#,##0.00');
        expect(ws.getCell("A7").value).toBe("Total Assets");
        expect(ws.getCell("B7").value).toBe(1234.56);
        expect(ws.getCell("A8").value).toBe("Liabilities");
        expect(ws.getCell("A9").value).toBe("liabilities");
        expect(ws.getCell("B9").value).toBe(200);
        expect(ws.getCell("A10").value).toBe("Total Liabilities");
        expect(ws.getCell("A11").value).toBe("Net");
        expect(ws.getCell("B11").value).toBe(1034.56);
        expect(ws.getCell("B11").font.bold).toBe(true);
    });

    it("period report: bucket header labels, per-bucket values, Net totals row", async () => {
        const report: PeriodReport = {
            buckets: ["2026-06", "2026-07"],
            rows: [
                {account: "assets", depth: 1, values: [usd(-5000), usd(10050)]},
                {account: "assets:bank", depth: 2, values: [usd(-5000), usd(10050)]},
            ],
            totals: [usd(-5000), usd(10050)],
        };
        const ws = await roundTrip(report, {title: "Cash Flow", params: "last 2 monthly periods ending 2026-07-08, depth 2"});

        expect([ws.getCell("A4").value, ws.getCell("B4").value, ws.getCell("C4").value]).toEqual(["Account", "Jun 2026", "Jul 2026"]);
        // chain compressed to the single leaf row
        expect(ws.getCell("A5").value).toBe("assets:bank");
        expect(ws.getCell("B5").value).toBe(-50);
        expect(ws.getCell("C5").value).toBe(100.5);
        expect(ws.getCell("C5").numFmt).toBe('"$"#,##0.00');
        expect(ws.getCell("A6").value).toBe("Net");
        expect(ws.getCell("B6").value).toBe(-50);
        expect(ws.getCell("C6").value).toBe(100.5);
    });

    // FE-6: the cell value must be the number the SCREEN shows. Excel applying
    // the number format to an unrounded float re-rounds a BINARY approximation
    // (1005/1e3 is stored as 1.00499999999999989…), so it printed 1.00 where
    // formatDec printed 1.01. These three mantissas are exactly the half-way
    // cases that expose it.
    describe("exact decimal rounding at the export boundary", () => {
        it.each([
            [1005, 1.01, "1.01"],
            [1015, 1.02, "1.02"],
            [-1005, -1.01, "-1.01"],
        ])("writes %i/1e3 as %f, matching the screen's %s", async (mantissa, expected) => {
            const report: PeriodReport = {
                buckets: ["2026-07"],
                rows: [{account: "assets", depth: 1, values: [amt(mantissa, 3)]}],
                totals: [amt(mantissa, 3)],
            };
            const ws = await roundTrip(report, {title: "Cash Flow", params: "p"});
            expect(ws.getCell("B5").value).toBe(expected);
            expect(ws.getCell("B5").numFmt).toBe('"$"#,##0.00'); // still two places: the ROUNDING moved, not the format
            expect(ws.getCell("B6").value).toBe(expected); // the Net row too
        });

        it("rounds the multi-commodity TEXT fallback on the Dec, not via toFixed on a float", async () => {
            const mixed: MixedAmount = new Map([
                ["$", dec(1005, 3)],
                ["EUR", dec(-1005, 3)],
            ]);
            const report: PeriodReport = {buckets: ["2026"], rows: [{account: "assets", depth: 1, values: [mixed]}], totals: [new Map()]};
            const ws = await roundTrip(report, {title: "Net Worth", params: "p"});
            expect(ws.getCell("B5").value).toBe("1.01 $, -1.01 EUR"); // was "1.00 $, -1.00 EUR"
        });

        it("does not silently drop a sub-cent value to zero", async () => {
            const report: PeriodReport = {buckets: ["2026"], rows: [{account: "assets", depth: 1, values: [amt(12345, 8)]}], totals: [amt(12345, 8)]};
            const ws = await roundTrip(report, {title: "Net Worth", params: "p"});
            expect(ws.getCell("B5").value).toBe(0.00012345); // was 0.00012345 shown through a 2-place format, i.e. "0.00"
            expect(ws.getCell("B5").numFmt).toBe('"$"#,##0.00000000');
        });
    });

    it("multi-commodity cells fall back to text; empty cells write 0", async () => {
        const mixed: MixedAmount = new Map([
            ["EUR", dec(1000, 2)],
            ["$", dec(2500, 2)],
        ]);
        const report: PeriodReport = {
            buckets: ["2026"],
            rows: [{account: "assets", depth: 1, values: [mixed]}],
            totals: [new Map()],
        };
        const ws = await roundTrip(report, {title: "Net Worth", params: "last 1 yearly periods ending 2026-07-08"});
        expect(ws.getCell("B5").value).toBe("25.00 $, 10.00 EUR"); // sorted by commodity ("$" < "EUR")
        expect(ws.getCell("B6").value).toBe(0);
    });

    it("holdings workbook: headers, data rows, nulls → empty cells, percent format, honest totals row", async () => {
        const aapl: Holding = {
            symbol: "AAPL",
            name: "Apple Inc.",
            accounts: ["assets:broker"],
            shares: dec(105n, 1), // 10.5
            basis: dec(100000n, 2), // $1,000.00
            firstBasisDate: "2024-05-01",
            price: {qty: dec(20000n, 2), date: "2026-06-30", source: "directive"},
            marketValue: dec(210000n, 2),
            gain: dec(110000n, 2),
            gainPct: 110,
        };
        const gld: Holding = {
            symbol: "GLD",
            name: "GLD",
            accounts: [],
            shares: dec(5n, 0),
            basis: null,
            firstBasisDate: null,
            price: null,
            marketValue: null,
            gain: null,
            gainPct: null,
        };
        const report: HoldingsReport = {
            asOf: "2026-07-08",
            base: "$",
            holdings: [aapl, gld],
            totals: {marketValue: dec(210000n, 2), basis: null, gain: null, gainPct: null}, // honest: GLD is tainted/unpriced
            topGainers: [],
            topLosers: [],
            warnings: [],
        };
        const ws = await readBack(await buildHoldingsWorkbook(report, {title: "Holdings", params: "As of 2026-07-08"}), "Holdings");

        expect(ws.getCell("A1").value).toBe("Holdings");
        expect(ws.getCell("A2").value).toBe("As of 2026-07-08");
        const headers = Array.from({length: 10}, (_, i) => ws.getCell(4, i + 1).value);
        expect(headers).toEqual(["Name", "Symbol", "Shares", "Basis", "First basis", "Price", "Price date", "Market value", "Gain", "Gain %"]);

        // AAPL data row: shares numFmt from its own precision, money via the base commodity, dates as text.
        expect(ws.getCell("A5").value).toBe("Apple Inc.");
        expect(ws.getCell("B5").value).toBe("AAPL");
        expect(ws.getCell("C5").value).toBe(10.5);
        expect(ws.getCell("C5").numFmt).toBe("#,##0.0");
        expect(ws.getCell("D5").value).toBe(1000);
        expect(ws.getCell("D5").numFmt).toBe('"$"#,##0.00');
        expect(ws.getCell("E5").value).toBe("2024-05-01");
        expect(ws.getCell("F5").value).toBe(200);
        expect(ws.getCell("G5").value).toBe("2026-06-30");
        expect(ws.getCell("H5").value).toBe(2100);
        expect(ws.getCell("I5").value).toBe(1100);
        expect(ws.getCell("J5").value).toBeCloseTo(1.1, 12); // 110% stored as a real ratio; Excel's % format ×100s it back
        expect(ws.getCell("J5").numFmt).toBe("+0.0%;-0.0%");

        // GLD row: every null field is an empty cell, not 0 or an em-dash.
        expect(ws.getCell("A6").value).toBe("GLD");
        expect(ws.getCell("C6").value).toBe(5);
        for (const col of ["D", "E", "F", "G", "H", "I", "J"]) expect(ws.getCell(`${col}6`).value, `${col}6`).toBeNull();

        // Totals row: bold label, values ONLY in Basis and Market value — and the null basis stays blank.
        expect(ws.getCell("A7").value).toBe("Total (2 holdings)");
        expect(ws.getCell("A7").font.bold).toBe(true);
        expect(ws.getCell("H7").value).toBe(2100);
        expect(ws.getCell("H7").font.bold).toBe(true);
        for (const col of ["B", "C", "D", "E", "F", "G", "I", "J"]) expect(ws.getCell(`${col}7`).value, `${col}7`).toBeNull();
    });

    // FE-6: shares are a unit count, not money, and a holding's marketValue has
    // places = shares.p + price.p — routinely above the money cap, which is
    // where the float re-rounding bit hardest.
    it("holdings workbook: fractional units survive, and market value rounds exactly", async () => {
        const btc: Holding = {
            symbol: "BTC",
            name: "Bitcoin",
            accounts: ["assets:crypto"],
            shares: dec(123456n, 8), // 0.00123456 BTC — read "0" under the money cap
            basis: dec(7000n, 2),
            firstBasisDate: "2025-01-02",
            price: {qty: dec(1005n, 3), date: "2026-06-30", source: "directive"}, // sub-cent-sensitive price
            marketValue: dec(1005n, 3), // shares.p + price.p → 3 places; 1.005 → screen "1.01"
            gain: dec(-1015n, 3), // −1.015 → screen "-1.02"
            gainPct: null,
        };
        const report: HoldingsReport = {
            asOf: "2026-07-08",
            base: "$",
            holdings: [btc],
            totals: {marketValue: dec(1005n, 3), basis: dec(7000n, 2), gain: dec(-1015n, 3), gainPct: null},
            topGainers: [],
            topLosers: [],
            warnings: [],
        };
        const ws = await readBack(await buildHoldingsWorkbook(report, {title: "Holdings", params: "As of 2026-07-08"}), "Holdings");

        expect(ws.getCell("C5").value).toBe(0.00123456); // was 0.00123456 under "#,##0.00", i.e. "0.00"
        expect(ws.getCell("C5").numFmt).toBe("#,##0.00000000");
        expect(ws.getCell("F5").value).toBe(1.01); // price: exact half-up, was 1.005 → Excel "1.00"
        expect(ws.getCell("H5").value).toBe(1.01); // market value
        expect(ws.getCell("I5").value).toBe(-1.02); // gain: −1.015 away from zero, was Excel "-1.01"
        expect(ws.getCell("H6").value).toBe(1.01); // totals row
        expect(ws.getCell("H6").numFmt).toBe('"$"#,##0.00');
    });

    // FE-6: the sheet used to end in one "Total" row that summed |income| +
    // |expenses| into a figure shown nowhere on the page, and divided one such
    // sum by another for "% of budget". BudgetSummary.svelte renders two
    // sections with two totals; so does the export now.
    describe("budget workbook mirrors the two on-screen sections", () => {
        const cell = (actual: MixedAmount, goal: MixedAmount | null): BudgetCell => ({actual, goal});
        const declared: ReadonlyMap<string, AccountType> = new Map();

        // Income is credit-normal (negative on the wire); expenses positive.
        const report: BudgetReport = {
            kind: "budget",
            buckets: ["2026-06", "2026-07"],
            rows: [
                {account: "income:salary", depth: 2, cells: [cell(usd(-300000), usd(-250000)), cell(usd(-300000), usd(-250000))]},
                {account: "expenses:rent", depth: 2, cells: [cell(usd(200000), usd(200000)), cell(usd(200000), usd(200000))]},
                {account: "expenses:food", depth: 2, cells: [cell(usd(30000), usd(40000)), cell(usd(30000), usd(40000))]},
            ],
            totals: [cell(new Map(), null), cell(new Map(), null)],
        };

        it("writes an Income section then an Expenses section, each with its own total", async () => {
            const ws = await readBack(await buildBudgetWorkbook(report, {title: "Budget", params: "2026-06 – 2026-07"}, declared), "Budget");

            expect(Array.from({length: 5}, (_, i) => ws.getCell(4, i + 1).value)).toEqual(["Account", "Spent", "Budget", "Remaining", "% of budget"]);
            expect(Array.from({length: 7}, (_, i) => ws.getCell(5 + i, 1).value)).toEqual([
                "Income",
                "income:salary",
                "Total Income",
                "Expenses",
                "expenses:rent",
                "expenses:food",
                "Total Expenses",
            ]);
            expect(ws.getCell("A5").font.bold).toBe(true);
            expect(ws.getCell("A8").font.bold).toBe(true);
        });

        it("totals each section on its own scale — never |income| + |expenses|", async () => {
            const ws = await readBack(await buildBudgetWorkbook(report, {title: "Budget", params: "p"}, declared), "Budget");

            // Income: earned $6,000 of a $5,000 target (magnitudes, as the page shows them).
            expect([ws.getCell("B7").value, ws.getCell("C7").value, ws.getCell("D7").value]).toEqual([6000, 5000, -1000]);
            expect(ws.getCell("E7").value).toBe(1.2);
            expect(ws.getCell("B7").font.bold).toBe(true);

            // Expenses: spent $4,600 of $4,800.
            expect([ws.getCell("B11").value, ws.getCell("C11").value, ws.getCell("D11").value]).toEqual([4600, 4800, 200]);
            expect(ws.getCell("E11").value).toBeCloseTo(4600 / 4800, 12);

            // The sheet ends there: no row summing 6000 + 4600 = 10600 against 5000 + 4800 = 9800.
            expect(ws.getCell("A12").value).toBeNull();
            expect(ws.getCell("B12").value).toBeNull();
        });

        it("omits a section entirely when the period has no leaf of that type", async () => {
            const expensesOnly: BudgetReport = {...report, rows: report.rows.filter((r) => r.account.startsWith("expenses"))};
            const ws = await readBack(await buildBudgetWorkbook(expensesOnly, {title: "Budget", params: "p"}, declared), "Budget");
            expect(Array.from({length: 4}, (_, i) => ws.getCell(5 + i, 1).value)).toEqual(["Expenses", "expenses:rent", "expenses:food", "Total Expenses"]);
        });
    });
});
