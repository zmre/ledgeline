// Read-back verification for the xlsx builder (WP-07 DoD): build a workbook,
// serialize it, re-load it with exceljs, and assert title row, headers, cell
// values, and number formats survive the round trip.

import {describe, expect, it} from "vitest";
import {Workbook, type Worksheet} from "exceljs";
import {decodeBalanceSheetReport, decodeIncomeStatementReport} from "$lib/api/nativeDecode";
import {dec, type MixedAmount} from "$lib/domain/money";
import type {AccountType} from "$lib/domain/accountTypes";
import type {Holding, HoldingsReport} from "$lib/holdings/types";
import type {BudgetCell, BudgetReport, PeriodReport, SectionedReport} from "$lib/reports/types";
import {CLASSIFIED_BALANCE_SHEET, GROUPED_BALANCE_SHEET, UNBALANCED_BALANCE_SHEET} from "$lib/testing/balanceSheetFixture";
import {GROUPED_INCOME_STATEMENT, MULTI_STEP_INCOME_STATEMENT, UNCOMPARED_INCOME_STATEMENT} from "$lib/testing/incomeStatementFixture";
import {buildBalanceSheetWorkbook, buildBudgetWorkbook, buildHoldingsWorkbook, buildIncomeStatementWorkbook, buildWorkbook, numberFormat} from "./xlsx";

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

    // plans/12: the balance sheet is valued into ONE commodity, so its Amount
    // column is finally real numbers with a number format instead of the
    // comma-joined text fallback. What could not be valued goes in its own
    // column rather than disappearing.
    describe("grouped balance-sheet workbook", () => {
        const report = decodeBalanceSheetReport(GROUPED_BALANCE_SHEET);
        const build = (r = report) => buildBalanceSheetWorkbook(r, {title: "Balance Sheet", params: "as of 2026-07-08, depth 3"});
        const column = (ws: Worksheet, from: number, count: number, col = 1) => Array.from({length: count}, (_, i) => ws.getCell(from + i, col).value);

        it("lays out a coloured section header, bold groups, indented accounts, a ruled total", async () => {
            const ws = await readBack(await build(), "Balance Sheet");

            expect(ws.getCell("A1").value).toBe("Balance Sheet");
            expect([ws.getCell("A4").value, ws.getCell("B4").value, ws.getCell("C4").value]).toEqual(["Account", "Amount ($)", "Other commodities"]);

            expect(column(ws, 5, 10)).toEqual([
                "Assets",
                "Cash and cash equivalents",
                "assets:bank",
                "checking",
                "savings",
                // One row, not two: the report is unclamped now, and
                // `compressSectionRows` folds the postingless `assets:bank:wise`
                // into its only child — the workbook shows exactly what the
                // screen does.
                "wise:eur",
                "Investments",
                "assets:broker:taxable",
                "Total Assets",
                null, // a blank row between boxes, matching the gap on screen
            ]);

            // The section header is filled, not merely bold.
            expect(ws.getCell("A5").fill).toMatchObject({type: "pattern", fgColor: {argb: "FF14532D"}});
            expect(ws.getCell("A5").font.bold).toBe(true);
            expect(ws.getCell("A6").font.bold).toBe(true); // group row
            expect(ws.getCell("A8").alignment?.indent).toBe(3); // account row, indented under its group
            expect(ws.getCell("A13").border?.top?.style).toBe("thin"); // the subtotal rule
        });

        it("writes every figure as a real number rounded exactly as the screen rounds it", async () => {
            const ws = await readBack(await build(), "Balance Sheet");

            expect(ws.getCell("B6").value).toBe(42450.24);
            expect(ws.getCell("B6").numFmt).toBe('"$"#,##0.00');
            expect(ws.getCell("B8").value).toBe(28292.81);
            // FE-6: $17,162.375 exactly. Rounded on the Dec (half away from zero)
            // before it becomes a float, so the workbook cannot print .37 where
            // the screen printed .38.
            expect(ws.getCell("B11").value).toBe(17162.38);
            expect(ws.getCell("B13").value).toBe(59612.62); // $59,612.615
            expect(ws.getCell("B13").font.bold).toBe(true);
        });

        it("surfaces the commodities the valuation could not convert instead of dropping them", async () => {
            const ws = await readBack(await build(), "Balance Sheet");

            // GLD and TSLA have no `P` directive in the fixture journal. They are
            // not in the Amount column and they must not be nowhere.
            expect(ws.getCell("C11").value).toBe("5 GLD, -2 TSLA");
            expect(ws.getCell("C13").value).toBe("5 GLD, -2 TSLA");
            expect(ws.getCell("C13").font.bold).toBe(true);
        });

        it("writes a real $0.00 for a line with no base-commodity part", async () => {
            const ws = await readBack(await build(), "Balance Sheet");

            // "Transfers" is 5 GLD and no dollars. A blank cell would read as
            // "no data" rather than "no dollars".
            expect(ws.getCell("A23").value).toBe("Transfers");
            expect(ws.getCell("B23").value).toBe(0);
            expect(ws.getCell("B23").numFmt).toBe('"$"#,##0.00');
            expect(ws.getCell("C23").value).toBe("5 GLD");
        });

        it("degrades to the shared text fallback under a bare Amount header when there is no base", async () => {
            // The same no-base shape the screen's own test mounts: `base` is
            // `Option<Commodity>` on the wire and arrives null for a journal
            // with no base commodity. There is then no figure to promote and no
            // leftovers to demote, so `setBsAmount` hands every cell to
            // `setAmount` and the header drops the "($)".
            const ws = await readBack(await build(decodeBalanceSheetReport({...(GROUPED_BALANCE_SHEET as object), base: null})), "Balance Sheet");

            expect([ws.getCell("A4").value, ws.getCell("B4").value]).toEqual(["Account", "Amount"]); // bare: no commodity to parenthesize

            // A single-commodity line is still a real number with its format…
            expect(ws.getCell("B6").value).toBe(42450.24);
            expect(ws.getCell("B6").numFmt).toBe('"$"#,##0.00');
            // …including one whose only commodity is no currency at all — the
            // sole commodity leads, exactly as the screen's `amountCell`
            // promotes the first in sort order.
            expect(ws.getCell("A23").value).toBe("Transfers");
            expect(ws.getCell("B23").value).toBe(5);
            expect(ws.getCell("B23").numFmt).toBe('#,##0 "GLD"');
            // A multi-commodity line joins every commodity in the ONE Amount
            // cell, sorted, rounded on the Dec — the same figures the screen
            // renders as headline-plus-footnotes for this fixture.
            expect(ws.getCell("B11").value).toBe("17162.38 $, 5 GLD, -2 TSLA");
            expect(ws.getCell("B13").value).toBe("59612.62 $, 5 GLD, -2 TSLA"); // Total Assets
            expect(ws.getCell("B25").value).toBe("42998.91 $, -933.25 EUR"); // Retained earnings
            // And the extras column is written NOWHERE: the leftovers are in the
            // Amount cells, and repeating them would print them twice. (Each of
            // these C cells carries text when base is "$" — see the tests above.)
            for (const row of [11, 13, 23, 25, 36]) expect(ws.getCell(row, 3).value, `C${row}`).toBeNull();
        });

        it("writes the computed equity lines, which have a total and no accounts", async () => {
            const ws = await readBack(await build(), "Balance Sheet");

            expect(column(ws, 20, 8)).toEqual([
                "Equity",
                "Opening",
                "equity:opening",
                "Transfers",
                "equity:transfers",
                "Retained earnings", // computed: no account rows beneath it
                "Valuation adjustment",
                "Total Equity",
            ]);
            expect(ws.getCell("B25").value).toBe(42998.91);
            expect(ws.getCell("C25").value).toBe("-933.25 EUR");
        });

        it("ends with the same tie-out the screen shows, then net worth, and freezes the header", async () => {
            const ws = await readBack(await build(), "Balance Sheet");

            expect(column(ws, 29, 8)).toEqual([
                "Total Assets",
                "Total Liabilities",
                "Total Equity",
                "Liabilities + Equity",
                "Total Assets",
                "Balanced",
                null, // a blank row, as on screen between the tie-out and net worth
                "Net worth (assets − liabilities)",
            ]);
            expect(ws.getCell("A29").border?.top?.style).toBe("medium");
            expect(ws.getCell("A32").border?.top?.style).toBe("thin"); // the tie-out rule
            expect(ws.getCell("A36").border?.top?.style).toBe("medium");
            expect(ws.views[0]).toMatchObject({state: "frozen", ySplit: 4});
        });

        it("sums Liabilities + equity from the exact Decs, so it ties to Total assets", async () => {
            const ws = await readBack(await build(), "Balance Sheet");

            // $531.15 + $59,081.465 = $59,612.615 → $59,612.62, the same cell
            // Total assets holds. The unpriced holdings tie out too and must be
            // on the tie-out rows, not only up in the boxes.
            expect(ws.getCell("B32").value).toBe(59612.62);
            expect(ws.getCell("C32").value).toBe("5 GLD, -2 TSLA");
            expect(ws.getCell("B33").value).toBe(59612.62);
            expect(ws.getCell("B36").value).toBe(59081.47); // net worth, $59,081.465
        });

        it("writes the check line when it is non-zero, at a precision that shows the imbalance", async () => {
            const ws = await readBack(await build(decodeBalanceSheetReport(UNBALANCED_BALANCE_SHEET)), "Balance Sheet");

            // The two tie-out figures still PRINT the same $59,612.62 — the
            // imbalance is half a cent. The verdict comes from the engine's exact
            // `check`, which is why the workbook says so anyway.
            expect(ws.getCell("B32").value).toBe(59612.62);
            expect(ws.getCell("B33").value).toBe(59612.62);
            expect(ws.getCell("A34").value).toBe("Out of balance (assets − liabilities − equity)");
            // Half a cent, rounded away from zero to $0.01 — visible, rather than
            // rounded into the $0.00 that would read as balanced.
            expect(ws.getCell("B34").value).toBe(0.01);
            expect(ws.getCell("A34").font.bold).toBe(true);
        });

        // The current/non-current axis. Indentation is the workbook's only
        // structural language, so it is what has to carry the hierarchy the
        // screen carries with type: heading over group over account.
        describe("current / non-current bands", () => {
            const classified = () => build(decodeBalanceSheetReport(CLASSIFIED_BALANCE_SHEET));

            it("heads each band and closes it with the engine's own subtotal", async () => {
                const ws = await readBack(await classified(), "Balance Sheet");

                expect(column(ws, 5, 16)).toEqual([
                    "Assets",
                    "Current",
                    "Cash and cash equivalents",
                    "assets:bank",
                    "checking",
                    "savings",
                    "Accounts receivable",
                    "assets:ar",
                    "Total current assets",
                    "Non-current",
                    "Property",
                    // One row, not two: `compressSectionRows` folds the chain
                    // inside a band exactly as it does outside one.
                    "assets:property:house",
                    "Long-term investments",
                    "assets:broker:ira",
                    "Total non-current assets",
                    "Total Assets", // the section total, below both bands
                ]);
            });

            it("writes the band subtotals as real numbers, from the engine and not from the group cells", async () => {
                const ws = await readBack(await classified(), "Balance Sheet");

                expect(ws.getCell("B13").value).toBe(62500); // Total current assets
                expect(ws.getCell("B19").value).toBe(537500); // Total non-current assets
                expect(ws.getCell("B20").value).toBe(600000); // the section total, still the whole
                expect(ws.getCell("A13").font.bold).toBe(true);
                expect(ws.getCell("A13").border?.top?.style).toBe("thin"); // ruled like a subtotal
            });

            it("indents the band, then its groups, then their accounts", async () => {
                const ws = await readBack(await classified(), "Balance Sheet");

                expect(ws.getCell("A6").alignment?.indent).toBe(1); // "Current"
                expect(ws.getCell("A7").alignment?.indent).toBe(2); // a group inside it
                expect(ws.getCell("A8").alignment?.indent).toBe(3); // an account inside that
                expect(ws.getCell("A13").alignment?.indent).toBe(1); // the band's subtotal, back at its heading
            });

            it("leaves equity unbanded and its groups where they have always been", async () => {
                const ws = await readBack(await classified(), "Balance Sheet");

                // Equity's groups sit at indent 1 — the unbanded level — in the
                // same workbook where the other two boxes are at 2.
                expect(column(ws, 35, 5)).toEqual(["Equity", "Opening", "equity:opening", "Retained earnings", "Total Equity"]);
                expect(ws.getCell("A36").alignment?.indent).toBe(1);
                expect(ws.getCell("A37").alignment?.indent).toBe(2);
            });

            // The adaptive guarantee, in the workbook: the untagged layout is
            // pinned cell-for-cell by the tests above this describe, which still
            // pass unchanged. This one states the claim outright.
            it("writes no band rows at all for a journal that classifies nothing", async () => {
                const ws = await readBack(await build(), "Balance Sheet");
                const labels = column(ws, 5, 40);

                expect(labels).not.toContain("Current");
                expect(labels).not.toContain("Non-current");
                expect(labels.filter((v) => typeof v === "string" && v.startsWith("Total current"))).toEqual([]);
                expect(ws.getCell("A6").alignment?.indent).toBe(1); // groups stay at the unbanded level
            });
        });

        it("writes 'Balanced' for sub-cent cost dust, agreeing with the page it came from", async () => {
            // Same `bsSummary` as the screen, so the workbook cannot reach a
            // different conclusion from the same report — including on the
            // non-empty-but-balanced `check` a journal of fractional lots yields.
            const dusty = {...(GROUPED_BALANCE_SHEET as object), check: {$: {mantissa: "22797", places: 7}}, balanced: true};
            const ws = await readBack(await build(decodeBalanceSheetReport(dusty)), "Balance Sheet");

            expect(ws.getCell("A34").value).toBe("Balanced");
            expect(ws.getCell("B34").value).toBeNull();
        });
    });

    // plans/13: the income statement is valued into ONE commodity and compares
    // against a prior window, so its money columns are real numbers with a number
    // format and its percentage column is a real percent — not pre-rendered text.
    describe("grouped income-statement workbook", () => {
        const report = decodeIncomeStatementReport(GROUPED_INCOME_STATEMENT);
        const build = (r = report) => buildIncomeStatementWorkbook(r, {title: "Income Statement", params: "2026-01-01 to 2026-07-08"});
        const column = (ws: Worksheet, from: number, count: number, col = 1) => Array.from({length: count}, (_, i) => ws.getCell(from + i, col).value);

        it("lays out a coloured section header, bold groups, indented accounts, a ruled total", async () => {
            const ws = await readBack(await build(), "Income Statement");

            expect(ws.getCell("A1").value).toBe("Income Statement");
            // The prior column is headed with its own DATES: the previous
            // equal-length window is not a period a reader can infer from the
            // range in the title.
            expect(column(ws, 4, 5, 1).length).toBe(5);
            expect([ws.getCell("A4").value, ws.getCell("B4").value, ws.getCell("C4").value, ws.getCell("D4").value, ws.getCell("E4").value]).toEqual([
                "Account",
                "Amount ($)",
                "2025-06-26 … 2025-12-31",
                "% of revenue",
                "Other commodities",
            ]);

            expect(column(ws, 5, 8)).toEqual([
                "Revenue",
                // Groups sort by name, so Dividends precedes Salary — the
                // engine's order, not `hledger is`'s. See the fixture's note.
                "Dividends",
                "income:dividends",
                "Salary",
                "income:salary",
                "Total Revenue",
                null, // a blank row between boxes, matching the gap on screen
                "Expenses",
            ]);

            // The section header is filled, not merely bold.
            expect(ws.getCell("A5").fill).toMatchObject({type: "pattern", fgColor: {argb: "FF14532D"}});
            expect(ws.getCell("A5").font.bold).toBe(true);
            expect(ws.getCell("A6").font.bold).toBe(true); // group row
            expect(ws.getCell("A7").alignment?.indent).toBe(2); // account row, indented under its group
            expect(ws.getCell("A10").border?.top?.style).toBe("thin"); // the subtotal rule
            expect(ws.views[0]).toMatchObject({state: "frozen", ySplit: 4});
        });

        it("writes every figure as a real number rounded exactly as the screen rounds it", async () => {
            const ws = await readBack(await build(), "Income Statement");

            expect(ws.getCell("B10").value).toBe(34010); // Total Revenue
            expect(ws.getCell("B10").numFmt).toBe('"$"#,##0.00');
            expect(ws.getCell("C10").value).toBe(39397.5); // the prior window's
            // Row 34 → 36: the Depreciation group inserts its heading and its one
            // account row directly under the Expenses header.
            expect(ws.getCell("B36").value).toBe(28626.48); // Total Expenses
            expect(ws.getCell("C36").value).toBe(28516.71);
            expect(ws.getCell("B36").font.bold).toBe(true);
        });

        it("writes the percentage as a real percent, not as pre-rendered text", async () => {
            const ws = await readBack(await build(), "Income Statement");

            // Excel's % format multiplies by 100, so 0.842 renders "84.2%" — and
            // stays sortable, chartable and re-totalable, which "84.2%" as a
            // string is not.
            expect(ws.getCell("D36").value).toBe(0.842);
            expect(ws.getCell("D36").numFmt).toBe("0.0%");
            expect(ws.getCell("D10").value).toBe(1); // revenue is 100.0% of revenue
            // …and it is free of the float noise `pct / 100` would leave behind:
            // Salary is 99.9% of revenue, which that division stores as
            // 0.9990000000000001.
            expect(String(ws.getCell("D8").value)).toBe("0.999");
        });

        it("compresses single-child chains exactly as the screen does", async () => {
            const ws = await readBack(await build(), "Income Statement");

            // `expenses:housing` has one child with the same figure in both
            // windows, so the workbook shows the one row the screen shows.
            expect(column(ws, 19, 2)).toEqual(["Housing", "expenses:housing:rent"]);
        });

        it("writes every group EXPANDED, whatever is collapsed on screen", async () => {
            const ws = await readBack(await build(), "Income Statement");

            // A disclosure triangle is a way to read a long statement on a
            // screen; an exported statement missing the accounts the reader
            // happened to have closed is just an incomplete document.
            expect(column(ws, 21, 4)).toEqual(["Taxes", "expenses:taxes", "federal", "state"]);
        });

        it("keeps a line that ran in only one period, with an explicit zero on the other side", async () => {
            const ws = await readBack(await build(), "Income Statement");

            // `expenses:travel:flights` is current-only, `…:activities` prior-only.
            expect(column(ws, 29, 3)).toEqual(["activities", "flights", "lodging"]);
            expect([ws.getCell("B29").value, ws.getCell("C29").value]).toEqual([0, 39.6]);
            expect([ws.getCell("B30").value, ws.getCell("C30").value]).toEqual([412.8, 0]);
        });

        it("ends on net income, and restates nothing above it", async () => {
            const ws = await readBack(await build(), "Income Statement");

            // A condensed restatement of every section total stood between the
            // last box and this row, and went with the screen's: each of those
            // figures is already a `Total …` row above, so the block was
            // duplicated totals in a document whose whole point is that nothing
            // is printed twice.
            expect(column(ws, 38, 3)).toEqual(["Net income (revenue − expenses)", null, null]);
            expect(ws.getCell("A38").border?.top?.style).toBe("medium");
            // `isSummary` is shared with the view precisely so the workbook cannot
            // claim a different bottom line — or a different margin — from the
            // page it was exported from.
            expect(ws.getCell("B38").value).toBe(5383.52);
            expect(ws.getCell("C38").value).toBe(10880.79);
            expect(ws.getCell("D38").value).toBe(0.158);
            expect(ws.getCell("A38").font.bold).toBe(true);
        });

        it("writes each section total exactly once, in its own box's footer", async () => {
            const ws = await readBack(await build(), "Income Statement");
            const amounts = Array.from({length: 42}, (_, i) => ws.getCell(i + 1, 2).value);

            // The claim the summary block used to break: the expense total
            // appeared as "Total Expenses" AND as "Less: Expenses" four rows later.
            expect(amounts.filter((v) => v === 28626.48)).toHaveLength(1);
            expect(amounts.filter((v) => v === 34010)).toHaveLength(1);
        });

        describe("a multi-step statement", () => {
            const multi = decodeIncomeStatementReport(MULTI_STEP_INCOME_STATEMENT);
            const buildMulti = () => buildIncomeStatementWorkbook(multi, {title: "Income Statement", params: "2026"});

            it("writes each ladder rung between its boxes, ruled and blank-separated", async () => {
                const ws = await readBack(await buildMulti(), "Income Statement");

                expect(column(ws, 18, 5)).toEqual([
                    "Total Cost of revenue",
                    null, // the rung reads as separate from the box above it
                    "Gross profit",
                    null,
                    "Operating expenses", // …and from the box below it
                ]);
                // A heavier rule than a section total's, because a rung spans
                // everything above it rather than one box.
                expect(ws.getCell("A20").border?.top?.style).toBe("medium");
                expect(ws.getCell("A18").border?.top?.style).toBe("thin");
                expect(ws.getCell("B20").value).toBe(517400);
                expect(ws.getCell("C20").value).toBe(0.835);
            });

            it("orders EBITDA above D&A and Operating income below it", async () => {
                const ws = await readBack(await buildMulti(), "Income Statement");

                expect(ws.getCell("A29").value).toBe("EBITDA");
                expect(ws.getCell("A31").value).toBe("Depreciation & amortization");
                expect(ws.getCell("A36").value).toBe("Operating income");
                // Each rung is a running total of everything printed above it.
                expect([ws.getCell("B29").value, ws.getCell("B36").value]).toEqual([160900, 136900]);
            });

            it("gives every section its own fill, so the boxes are as distinct as on screen", async () => {
                const ws = await readBack(await buildMulti(), "Income Statement");
                const fills = [5, 12, 22, 31, 38, 45, 52].map((row) => (ws.getCell(row, 1).fill as {fgColor?: {argb?: string}}).fgColor?.argb);

                expect(new Set(fills).size).toBe(fills.length);
                expect(fills[0]).toBe("FF14532D"); // revenue, the one green box
            });

            it("writes the mixed `other` box below zero, alone among the sections", async () => {
                const ws = await readBack(await buildMulti(), "Income Statement");

                expect(ws.getCell("A43").value).toBe("Total Other income & expense");
                expect(ws.getCell("B43").value).toBe(-15000);
                expect(ws.getCell("C43").value).toBe(-0.024);
            });
        });

        it("drops the prior column entirely when the report is not comparing", async () => {
            const ws = await readBack(await build(decodeIncomeStatementReport(UNCOMPARED_INCOME_STATEMENT)), "Income Statement");

            // A blank column headed "Prior" would read as "the prior period was
            // zero" rather than as "there is no prior period", so every column
            // after it shifts.
            expect([ws.getCell("A4").value, ws.getCell("B4").value, ws.getCell("C4").value, ws.getCell("D4").value]).toEqual([
                "Account",
                "Amount ($)",
                "% of revenue",
                "Other commodities",
            ]);
            expect(ws.getCell("C10").value).toBe(1); // the percentage, now in C
            expect(ws.getCell("C10").numFmt).toBe("0.0%");
            expect(ws.getCell("D10").value).toBeNull();
        });

        it("surfaces the commodities the valuation could not convert instead of dropping them", async () => {
            // "unpriced commodities are surfaced, never silently dropped" is the
            // rule the whole redesign rests on, and a workbook that quietly
            // omitted them would be the worst place to break it.
            const unpriced = decodeIncomeStatementReport({
                ...(UNCOMPARED_INCOME_STATEMENT as object),
                netIncome: {current: {$: {mantissa: "538352", places: 2}, GLD: {mantissa: "5", places: 0}}},
            });
            const ws = await readBack(await build(unpriced), "Income Statement");

            expect(ws.getCell("A38").value).toBe("Net income (revenue − expenses)");
            expect(ws.getCell("B38").value).toBe(5383.52);
            expect(ws.getCell("C38").value).toBe(0.158); // no prior column here, so % is in C
            expect(ws.getCell("D38").value).toBe("5 GLD");
        });

        it("attributes each period's unconverted commodities to that period's own column", async () => {
            // 5 GLD unpriced in the current window, 3 GLD in the prior. The
            // screen hangs each figure's extras under the column the figure is
            // in (current under Amount, prior under the dated prior column), so
            // one concatenated cell reading "5 GLD, 3 GLD" said neither which
            // figure belonged to which window nor that the pair was not a
            // typo'd duplicate.
            const unpriced = decodeIncomeStatementReport({
                ...(GROUPED_INCOME_STATEMENT as object),
                netIncome: {
                    current: {$: {mantissa: "538352", places: 2}, GLD: {mantissa: "5", places: 0}},
                    prior: {$: {mantissa: "1088079", places: 2}, GLD: {mantissa: "3", places: 0}},
                },
            });
            const ws = await readBack(await build(unpriced), "Income Statement");

            expect(ws.getCell("A38").value).toBe("Net income (revenue − expenses)");
            expect(ws.getCell("E38").value).toBe("5 GLD"); // the current window's, and ONLY the current window's — not "5 GLD, 3 GLD"
            expect(ws.getCell("F38").value).toBe("3 GLD"); // the prior window's
            // A line with nothing unconverted writes neither cell.
            expect(ws.getCell("E10").value).toBeNull();
            expect(ws.getCell("F10").value).toBeNull();

            // The prior extras column repeats the prior window's DATES, exactly
            // as the prior amount column does: a second bare "Other commodities"
            // header would put the ambiguity back one row up.
            expect(ws.getCell("E4").value).toBe("Other commodities");
            expect(ws.getCell("F4").value).toBe("Other commodities (2025-06-26 … 2025-12-31)");
        });

        it("degrades to the shared text fallback under a bare Amount header when there is no base", async () => {
            // `base` is `Option<Commodity>` on the wire and arrives null for a
            // journal with no base commodity. There is then no figure to promote
            // and no leftovers to demote, so `setIsAmounts` hands every cell to
            // `setAmount` and the header drops the "($)".
            const noBase = decodeIncomeStatementReport({
                ...(GROUPED_INCOME_STATEMENT as object),
                base: null,
                netIncome: {
                    current: {$: {mantissa: "538352", places: 2}, GLD: {mantissa: "5", places: 0}},
                    prior: {$: {mantissa: "1088079", places: 2}, GLD: {mantissa: "3", places: 0}},
                },
            });
            const ws = await readBack(await build(noBase), "Income Statement");

            expect(ws.getCell("B4").value).toBe("Amount"); // bare: there is no commodity to parenthesize

            // A single-commodity line is still a real number with its format…
            expect(ws.getCell("B10").value).toBe(34010);
            expect(ws.getCell("B10").numFmt).toBe('"$"#,##0.00');
            // …and a multi-commodity one joins in ONE cell, per period, sorted
            // and rounded on the Dec — the same figures the screen renders as
            // headline-plus-footnotes for a no-base report.
            expect(ws.getCell("B38").value).toBe("5383.52 $, 5 GLD");
            expect(ws.getCell("C38").value).toBe("10880.79 $, 3 GLD");
            // No base: no revenue figure to divide by (`pctOfRevenue` is null,
            // as on screen) and NO extras column written — the leftovers are in
            // the Amount cells, and repeating them would print them twice.
            expect(ws.getCell("D38").value).toBeNull();
            expect(ws.getCell("E38").value).toBeNull();
            expect(ws.getCell("F38").value).toBeNull();
        });
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
            accounts: [], // the scope chooser's options; the workbook does not read them
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
            accounts: [],
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
