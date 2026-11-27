// Billing's words and requests: what the Unbilled page, the Bill dialog and
// the Pay dialog say and send.
import {describe, expect, it} from "vitest";
import {entry} from "../testing.js";
import {acme, billGroup, payPreview, q, unbilled} from "./fixtures.js";
import {
    bannerText,
    billRequest,
    differenceText,
    entriesBillTarget,
    hoursWords,
    moneyText,
    parseTable,
    payFieldsOf,
    payRequest,
    periodLabel,
    roundingWords,
    rowNeeds,
    savedWords,
    sourceWords,
    withChoice,
    type BillOptions,
} from "./words.js";

const OPTIONS: BillOptions = {group: "account", detail: null, date: "", number: "", terms: "", reference: ""};

describe("UNIT billing words", () => {
    it("names periods, hours, rounding and money in plain words", () => {
        expect(periodLabel("2026-10-01", "2026-10-31")).toBe("October 2026");
        expect(periodLabel("2026-09-29", "2026-10-31")).toBe("Sep 29 – Oct 31, 2026");
        expect(hoursWords(q(18000))).toBe("5 hours");
        expect(hoursWords(q(3600))).toBe("1 hour");
        expect(hoursWords(q(3000))).toBe("0.83 hours");
        expect(differenceText(q(3000), q(3240))).toBe("+4m");
        expect(differenceText(q(3000), q(3000))).toBe("—");
        expect(moneyText("1583.33", "$")).toBe("$1,583.33");
        expect(moneyText(null, "$")).toBeNull();
        expect(roundingWords({increment: 6, direction: "up", scope: "row"})).toBe("Rounded up to 6 minutes, each row. The stored time doesn't change.");
        expect(roundingWords({increment: null, direction: "up", scope: "row"})).toMatch(/^Not rounded/);
        expect(sourceWords("account")).toBe("Set on the time account");
        expect(sourceWords("most-used")).toBe("This customer's most-used item");
    });

    it("writes the banner from each customer's main suggestion", () => {
        expect(bannerText(unbilled().customers)).toBe("2 customers have unbilled time through October: Acme Corp 50m · Globex 3h");
        expect(bannerText([acme()])).toBe("1 customer has unbilled time through October: Acme Corp 50m");
        expect(bannerText([acme({periods: []})])).toBeNull();
    });

    it("says what a row still needs, and nothing for a row left off", () => {
        expect(rowNeeds(billGroup(), undefined)).toEqual([]);
        expect(rowNeeds(billGroup({item: null, rate: null, amount: null}), undefined)).toEqual(["Choose an item or enter a rate."]);
        const missing = billGroup({item: {...billGroup().item!, code: "NOPE", found: false}, rate: null, amount: null});
        expect(rowNeeds(missing, undefined)[0]).toContain("The catalog has no item NOPE");
        expect(rowNeeds(missing, {key: "k", rate: "$1"})).toEqual(["Choose an item or enter a rate."]);
        expect(rowNeeds({...missing, excluded: true}, undefined)).toEqual([]);
        expect(rowNeeds(billGroup({amount: null, rate: "lots"}), undefined)).toEqual(["lots doesn't read as an amount of money."]);
    });
});

describe("UNIT the Bill request", () => {
    it("merges a row's choices, dropping what is back to the default", () => {
        let choices = withChoice([], "a", {rate: "$200"});
        choices = withChoice(choices, "a", {item: "ARCH", rate: undefined});
        expect(choices).toEqual([{key: "a", item: "ARCH"}]);
        expect(withChoice(choices, "a", {item: undefined})).toEqual([]);
    });

    it("sends only what was edited; an emptied field is the default again", () => {
        expect(billRequest({customer: "Acme Corp", from: "2026-10-01", to: "2026-10-31"}, OPTIONS, [])).toEqual({
            customer: "Acme Corp",
            from: "2026-10-01",
            to: "2026-10-31",
            group: "account",
        });
        const full = billRequest(
            {customer: "Acme Corp", entries: ["f@1"]},
            {group: "account-person", detail: "entries", date: "2026-11-30", number: " 200 ", terms: "15", reference: "PO 7"},
            [
                {key: "a", rate: "", description: "Advice"},
                {key: "b", rate: " "},
            ]
        );
        expect(full).toEqual({
            customer: "Acme Corp",
            entries: ["f@1"],
            group: "account-person",
            groups: [{key: "a", description: "Advice"}],
            detail: "entries",
            date: "2026-11-30",
            number: "200",
            terms: 15,
            reference: "PO 7",
        });
        expect(billRequest({customer: "X"}, {...OPTIONS, terms: "soon"}, []).terms).toBeUndefined();
    });

    it("reads the timesheet table back for the preview", () => {
        const table = parseTable("## Timesheet\n\n| Date | Who | Work | Hours |\n|:-----|:----|:-----|------:|\n| 2026-10-06 | pat | planning | 0.83 |\n");
        expect(table).toEqual({
            title: "Timesheet",
            headers: ["Date", "Who", "Work", "Hours"],
            rows: [["2026-10-06", "pat", "planning", "0.83"]],
            right: [false, false, false, true],
        });
        expect(parseTable(null)).toBeNull();
        expect(parseTable("no table here")).toBeNull();
    });
});

describe("UNIT Entries' Bill…", () => {
    it("bills one customer's unbilled lines, and says why not otherwise", () => {
        const a = entry({id: "a@1"});
        const b = entry({id: "a@2"});
        expect(entriesBillTarget([a, b])).toEqual({customer: "Acme Corp", entries: ["a@1", "a@2"]});
        expect(entriesBillTarget([])).toEqual({error: "Pick the time to bill first."});
        expect(entriesBillTarget([a, entry({id: "g@1", customer: "Globex"})])).toMatchObject({
            error: expect.stringContaining("2 customers (Acme Corp, Globex)"),
        });
        expect(entriesBillTarget([a, entry({billed: "invoiced", invoice: "101"})])).toMatchObject({
            error: expect.stringContaining("One picked line is already billed"),
        });
        expect(entriesBillTarget([entry({customer: null})])).toMatchObject({error: expect.stringContaining("no customer")});
    });
});

describe("UNIT the Pay request", () => {
    it("sends only the answers that changed, and the note to use", () => {
        const initial = payFieldsOf(payPreview());
        expect(payRequest({person: "riley"}, initial, initial, false)).toEqual({person: "riley", lines: "account", save_to_note: true});
        const answered = {
            ...initial,
            rate: "$60",
            expense: "expenses:contractors",
            payable: "liabilities:accounts payable",
            terms: "15",
            lines: "total" as const,
        };
        expect(payRequest({person: "riley"}, answered, initial, false)).toEqual({
            person: "riley",
            lines: "total",
            save_to_note: true,
            rate: "$60",
            expense_account: "expenses:contractors",
            terms: 15,
        });
        const picked = {...initial, pickedNote: "Sam Lee"};
        expect(payRequest({person: "x"}, picked, initial, true).note).toBe("Sam Lee");
        const created = {...initial, noteMode: "create" as const, newNote: {title: "Jo Kim", kind: "organization" as const}};
        expect(payRequest({person: "x"}, created, initial, true).create_note).toEqual({title: "Jo Kim", kind: "organization"});
        expect(payRequest({person: "x"}, created, initial, false).create_note).toBeUndefined();
    });

    it("says what was saved to the note in words", () => {
        expect(savedWords(["pay_rate"])).toBe("the hourly rate");
        expect(savedWords(["pay_rate", "expense_account", "pay_terms"])).toBe("the hourly rate, the expense account and the terms");
    });
});
