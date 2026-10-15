// Read-back verification for the journal export: build, serialize, re-load with
// exceljs, and check the QuickBooks Journal layout cell by cell.

import {describe, expect, it} from "vitest";
import {Workbook, type Worksheet} from "exceljs";
import {dec, type Dec} from "#lib/domain/money.js";
import type {Amount, AmountStyle, Posting, Transaction} from "#lib/domain/types.js";
import {buildJournalWorkbook, journalGroups} from "./journalXlsx";

const DOLLARS: AmountStyle = {side: "L", spaced: false, precision: 2, decimalPoint: ".", digitGroups: [",", [3]]};
const SHARES: AmountStyle = {side: "R", spaced: true, precision: 4, decimalPoint: ".", digitGroups: null};
const styleOf = (commodity: string): AmountStyle => (commodity === "$" ? DOLLARS : SHARES);

const amount = (commodity: string, qty: Dec, cost?: Amount["cost"]): Amount => ({commodity, qty, style: styleOf(commodity), ...(cost ? {cost} : {})});
const posting = (account: string, amounts: Amount[], extra: Partial<Posting> = {}): Posting => ({
    account,
    amounts,
    status: "unmarked",
    comment: "",
    tags: [],
    ...extra,
});
const txn = (index: number, date: string, description: string, postings: Posting[], extra: Partial<Transaction> = {}): Transaction => ({
    index,
    date,
    status: "unmarked",
    description,
    code: "",
    comment: "",
    tags: [],
    postings,
    haystack: "",
    ...extra,
});

// A split paycheck: one credit, three debits — the "more than two accounts" case.
const PAYCHECK = txn(
    7,
    "2026-03-15",
    "Acme payroll",
    [
        posting("income:salary", [amount("$", dec(-500000, 2))]),
        posting("expenses:taxes:federal", [amount("$", dec(90000, 2))]),
        posting("expenses:health", [amount("$", dec(25000, 2))], {comment: "HSA"}),
        posting("assets:checking", [amount("$", dec(3850, 0))]), // written `$3850`: declared style pads it
    ],
    {code: "1042", tags: [["vendor", "Acme Corp"]]}
);

const BUY = txn(9, "2026-03-20", "Buy AAPL", [
    posting("assets:brokerage", [amount("AAPL", dec(10, 0), {commodity: "$", qty: dec(15000, 2), per: true})]),
    posting("assets:checking", [amount("$", dec(-150000, 2))]),
]);

const META = {title: "Acme Books", period: "2026-03-01 – 2026-03-31", generatedAt: new Date(Date.UTC(2026, 3, 1))};

async function sheet(txns: Transaction[]): Promise<Worksheet> {
    const built = await buildJournalWorkbook(txns, styleOf, META);
    const loaded = new Workbook();
    await loaded.xlsx.load((await built.xlsx.writeBuffer()) as never);
    const ws = loaded.getWorksheet("Journal");
    if (ws === undefined) throw new Error("Journal sheet missing");
    return ws;
}

const values = (ws: Worksheet, row: number): unknown[] => (ws.getRow(row).values as unknown[]).slice(1);

describe("UNIT export/journalXlsx", () => {
    it("lays out the QuickBooks title band and header row", async () => {
        const ws = await sheet([PAYCHECK]);
        expect(ws.getCell(1, 1).value).toBe("Acme Books");
        expect(ws.getCell(2, 1).value).toBe("Journal");
        expect(ws.getCell(3, 1).value).toBe("2026-03-01 – 2026-03-31");
        expect(values(ws, 4)).toEqual([]);
        // Column A is the unlabelled marker column; Vendor appears because a txn carries the tag.
        expect(values(ws, 5)).toEqual([
            undefined,
            "Transaction date",
            "Transaction type",
            "Num",
            "Name",
            "Description",
            "Account Name",
            "Debit",
            "Credit",
            "Vendor",
        ]);
    });

    it("writes a multi-account entry as one group: marker, a row per posting, Total for", async () => {
        const ws = await sheet([PAYCHECK]);
        expect(ws.getCell(6, 1).value).toBe("7");
        const accounts = [7, 8, 9, 10].map((r) => ws.getCell(r, 7).value);
        expect(accounts).toEqual(["income:salary", "expenses:taxes:federal", "expenses:health", "assets:checking"]);
        // Date and type repeat on every posting row, as QuickBooks writes them.
        for (const r of [7, 8, 9, 10]) {
            expect(ws.getCell(r, 2).value).toEqual(new Date(Date.UTC(2026, 2, 15)));
            expect(ws.getCell(r, 3).value).toBe("Journal Entry");
            expect(ws.getCell(r, 4).value).toBe("1042");
            expect(ws.getCell(r, 10).value).toBe("Acme Corp");
        }
        expect(ws.getCell(7, 8).value).toBeNull();
        expect(ws.getCell(7, 9).value).toBe(5000);
        expect(ws.getCell(8, 8).value).toBe(900);
        expect(ws.getCell(9, 6).value).toBe("HSA");
        // `$3850` as written, shown in the declared two-place style.
        expect(ws.getCell(10, 8).value).toBe(3850);
        expect(ws.getCell(10, 8).numFmt).toBe('"$"#,##0.00');

        expect(ws.getCell(11, 1).value).toBe("Total for 7");
        expect(ws.getCell(11, 8).value).toEqual({formula: "SUM(H7:H10)", result: 5000});
        expect(ws.getCell(11, 9).value).toEqual({formula: "SUM(I7:I10)", result: 5000});
        expect(ws.getCell(12, 1).value).toBe("TOTAL");
        expect(ws.getCell(12, 8).value).toBe(5000);
    });

    it("carries a priced posting at cost, keeping the as-written quantity", async () => {
        const ws = await sheet([BUY]);
        expect(values(ws, 5)).toContain("Quantity");
        expect(ws.getCell(7, 7).value).toBe("assets:brokerage");
        expect(ws.getCell(7, 8).value).toBe(1500);
        expect(ws.getCell(7, 10).value).toBe("10.0000 AAPL @ $150.00"); // AAPL's declared four places
        expect(ws.getCell(8, 9).value).toBe(1500);
        expect(ws.getCell(9, 8).value).toEqual({formula: "SUM(H7:H8)", result: 1500});
    });

    it("totals an unpriced multi-commodity entry as text, per commodity", () => {
        const swap = txn(3, "2026-03-02", "Swap", [posting("assets:a", [amount("AAPL", dec(25, 1))]), posting("assets:b", [amount("$", dec(-100, 0))])]);
        const [group] = journalGroups([swap], styleOf);
        expect(group.rows.map((r) => [r.commodity, r.debit, r.credit])).toEqual([
            ["AAPL", dec(25000, 4), null],
            ["$", null, dec(10000, 2)],
        ]);
    });

    it("brackets virtual postings the way hledger writes them", () => {
        const virtual = txn(4, "2026-03-03", "Budget", [
            posting("budget:food", [amount("$", dec(50, 0))], {type: "virtual"}),
            posting("assets:envelope", [amount("$", dec(50, 0))], {type: "balancedVirtual"}),
        ]);
        expect(journalGroups([virtual], styleOf)[0].rows.map((r) => r.account)).toEqual(["(budget:food)", "[assets:envelope]"]);
    });
});
