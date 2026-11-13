import {describe, expect, it} from "vitest";
import type {AccountClass, AccountClasses} from "#lib/domain/accountClass.js";
import {NO_ACCOUNT_CLASSES} from "#lib/domain/accountClass.js";
import {dec} from "#lib/domain/money.js";
import type {Amount, AmountStyle, ISODate, Posting, Transaction} from "#lib/domain/types.js";
import type {JournalRow} from "#lib/journalView/types.js";
import {calendarItem, calendarMonth, fitDay, groupByDay, isWholeMonth, monthGrid, monthLabel, monthRange, shiftMonth} from "./calendarModel";

const usdStyle: AmountStyle = {side: "L", spaced: false, precision: 2, decimalPoint: ".", digitGroups: [",", [3]]};
const shareStyle: AmountStyle = {side: "R", spaced: true, precision: 0, decimalPoint: ".", digitGroups: null};
const usd = (cents: number): Amount => ({commodity: "$", qty: dec(cents, 2), style: usdStyle});
const shares = (n: number, cost?: Amount["cost"]): Amount => ({commodity: "AAPL", qty: dec(n, 0), style: shareStyle, ...(cost === undefined ? {} : {cost})});

function txn(index: number, date: ISODate, description: string, postings: [string, Amount[]][]): Transaction {
    const full: Posting[] = postings.map(([account, amounts]) => ({account, amounts, status: "unmarked", comment: "", tags: []}));
    return {index, date, status: "unmarked", description, code: "", comment: "", tags: [], postings: full};
}

const row = (t: Transaction, flags: JournalRow["flags"] = []): JournalRow => ({txn: t, flags, unknown: false, reviewed: false});

const opts = {classes: NO_ACCOUNT_CLASSES, styleOf: () => usdStyle, preferred: "$"};

describe("month math", () => {
    it("bounds a month, leap February included", () => {
        expect(monthRange("2026-11")).toEqual({from: "2026-11-01", to: "2026-11-30"});
        expect(monthRange("2028-02")).toEqual({from: "2028-02-01", to: "2028-02-29"});
    });

    it("shifts across year ends", () => {
        expect(shiftMonth("2026-12", 1)).toBe("2027-01");
        expect(shiftMonth("2026-01", -1)).toBe("2025-12");
        expect(shiftMonth("2026-05", -17)).toBe("2024-12");
    });

    it("labels a month", () => {
        expect(monthLabel("2026-11")).toBe("November 2026");
    });

    it("knows a whole month from anything else", () => {
        expect(isWholeMonth("2026-11-01", "2026-11-30")).toBe(true);
        expect(isWholeMonth("2026-11-01", "2026-11-29")).toBe(false);
        expect(isWholeMonth("2026-11-02", "2026-11-30")).toBe(false);
        expect(isWholeMonth(null, "2026-11-30")).toBe(false);
    });

    it("reads a range's month from its end, pulling a future end back to today", () => {
        const today = "2026-11-13";
        expect(calendarMonth("2026-08-16", "2026-11-13", today)).toBe("2026-11"); // last 90 days
        expect(calendarMonth("2025-01-01", "2025-12-31", today)).toBe("2025-12"); // last year
        expect(calendarMonth("2026-01-01", "2026-12-31", today)).toBe("2026-11"); // this year
        expect(calendarMonth("2027-03-01", "2027-03-31", today)).toBe("2027-03"); // a future month stays put
        expect(calendarMonth("2024-04-10", null, today)).toBe("2024-04");
        expect(calendarMonth(null, null, today)).toBe("2026-11"); // all dates
    });

    it("lays a month out in Sunday-first whole weeks", () => {
        // 1 November 2026 is a Sunday; the 30th a Monday.
        const weeks = monthGrid("2026-11");
        expect(weeks).toHaveLength(5);
        expect(weeks[0][0]).toBe("2026-11-01");
        expect(weeks[4]).toEqual(["2026-11-29", "2026-11-30", "2026-12-01", "2026-12-02", "2026-12-03", "2026-12-04", "2026-12-05"]);
        // 1 August 2026 is a Saturday: six weeks, led by July's last days.
        const august = monthGrid("2026-08");
        expect(august).toHaveLength(6);
        expect(august[0][0]).toBe("2026-07-26");
        expect(august[0][6]).toBe("2026-08-01");
        expect(august.every((week) => week.length === 7)).toBe(true);
    });
});

describe("fitDay", () => {
    it("shows everything that fits", () => {
        expect(fitDay(3, 3)).toEqual({shown: 3, more: 0});
        expect(fitDay(0, 4)).toEqual({shown: 0, more: 0});
    });

    it("gives the last line to N more", () => {
        expect(fitDay(9, 4)).toEqual({shown: 3, more: 6});
    });

    it("still says something in a cell with no room", () => {
        expect(fitDay(2, 0)).toEqual({shown: 0, more: 2});
        expect(fitDay(1, 0)).toEqual({shown: 1, more: 0});
    });
});

describe("calendarItem", () => {
    it("shows an expense negative and rounded to whole dollars", () => {
        const item = calendarItem(
            row(
                txn(1, "2026-11-03", "Safeway", [
                    ["expenses:food", [usd(5650)]],
                    ["liabilities:visa", [usd(-5650)]],
                ])
            ),
            opts
        );
        expect(item).toMatchObject({index: 1, label: "Safeway", trade: null, amount: "$-57", tone: "negative"});
    });

    it("shows income positive, net of what was withheld", () => {
        const item = calendarItem(
            row(
                txn(2, "2026-11-15", "Paycheck", [
                    ["assets:bank:checking", [usd(320000)]],
                    ["expenses:taxes", [usd(80000)]],
                    ["income:salary", [usd(-400000)]],
                ])
            ),
            opts
        );
        expect(item).toMatchObject({amount: "$3,200", tone: "positive"});
    });

    it("shows a transfer unsigned and neutral", () => {
        const item = calendarItem(
            row(
                txn(3, "2026-11-20", "Pay Visa", [
                    ["liabilities:visa", [usd(120000)]],
                    ["assets:bank:checking", [usd(-120000)]],
                ])
            ),
            opts
        );
        expect(item).toMatchObject({amount: "$1,200", tone: "neutral"});
    });

    it("classifies by declared type, not by name", () => {
        const declared = (type: AccountClass["type"]): AccountClass => ({type, typeDeclared: true, term: null, termGuessed: false});
        const classes: AccountClasses = new Map([
            ["gastos", declared("expense")],
            ["banco", declared("cash")],
        ]);
        const item = calendarItem(
            row(
                txn(4, "2026-11-04", "Mercado", [
                    ["gastos", [usd(1000)]],
                    ["banco", [usd(-1000)]],
                ])
            ),
            {...opts, classes}
        );
        expect(item).toMatchObject({amount: "$-10", tone: "negative"});
    });

    it("pills a sale with its symbol, signed shares and the cash it raised", () => {
        const item = calendarItem(
            row(
                txn(5, "2026-11-05", "Sell AAPL", [
                    ["assets:broker:aapl", [shares(-120, {commodity: "$", qty: dec(15025, 2), per: true})]],
                    ["assets:broker:cash", [usd(1803000)]],
                ])
            ),
            opts
        );
        expect(item.trade).toEqual({side: "sell", symbol: "AAPL", shares: "-120"});
        expect(item).toMatchObject({label: "AAPL -120", amount: "$18,030", tone: "positive"});
    });

    it("pills a purchase at a total cost as money out", () => {
        const item = calendarItem(
            row(
                txn(6, "2026-11-06", "Buy AAPL", [
                    ["assets:broker:aapl", [shares(10, {commodity: "$", qty: dec(150049, 2), per: false})]],
                    ["assets:broker:cash", [usd(-150049)]],
                ])
            ),
            opts
        );
        expect(item.trade).toEqual({side: "buy", symbol: "AAPL", shares: "+10"});
        expect(item).toMatchObject({amount: "$-1,500", tone: "negative"});
    });

    it("carries the worst finding and every message", () => {
        const flags = [
            {rule: "a", severity: "warning", message: "Looks like a duplicate", txnIndex: 7},
            {rule: "b", severity: "error", message: "Unbalanced", txnIndex: 7},
        ] as unknown as JournalRow["flags"];
        const item = calendarItem(row(txn(7, "2026-11-07", "", [["expenses:misc", [usd(100)]]]), flags), opts);
        expect(item.severity).toBe("error");
        expect(item.label).toBe("(no description)");
        expect(item.title).toContain("Looks like a duplicate");
        expect(item.title).toContain("Unbalanced");
    });
});

describe("groupByDay", () => {
    it("groups newest-first rows into oldest-first days", () => {
        const days = groupByDay([
            {date: "2026-11-02", id: "c"},
            {date: "2026-11-01", id: "b"},
            {date: "2026-11-01", id: "a"},
        ]);
        expect(days.get("2026-11-01")?.map((item) => item.id)).toEqual(["a", "b"]);
        expect(days.get("2026-11-02")?.map((item) => item.id)).toEqual(["c"]);
    });
});
