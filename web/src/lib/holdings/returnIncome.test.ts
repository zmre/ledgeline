import {describe, expect, it} from "vitest";
import type {AccountClass} from "../domain/accountClass";
import {matchesReturnIncome, splitIncomeAccounts} from "./returnIncome";

const cls = (type: AccountClass["type"]): AccountClass => ({type}) as AccountClass;

describe("UNIT returnIncome", () => {
    it("matches a fragment anywhere in the name, ignoring case and blanks", () => {
        expect(matchesReturnIncome("income:Dividends:VTI", ["dividend"])).toBe(true);
        expect(matchesReturnIncome("income:staking", [" STAKING "])).toBe(true);
        expect(matchesReturnIncome("income:rsu", ["dividend", "staking"])).toBe(false);
        expect(matchesReturnIncome("income:dividends", [""])).toBe(false);
        expect(matchesReturnIncome("income:dividends", [])).toBe(false);
    });

    it("splits only income accounts, by the filters", () => {
        const classes = new Map([
            ["income:dividends", cls("revenue")],
            ["income:rsu", cls("revenue")],
            ["income:staking", cls("revenue")],
            ["expenses:dividend-fees", cls("expense")],
        ]);
        expect(splitIncomeAccounts(classes, ["dividend", "staking"])).toEqual({
            asReturn: ["income:dividends", "income:staking"],
            asNewMoney: ["income:rsu"],
        });
    });
});
