// The rule FORM's client half (plan 30 Phase 4): copying, completeness, the
// wire shape and the one-line preview. Rendering and signing are the
// engine's, pinned in `auto_form.rs`.

import {describe, expect, it} from "vitest";
import {blankRuleForm, cloneRuleForm, ruleFormComplete, ruleFormNeedsCounter, ruleFormToWire, rulePreview, type AutoRuleForm} from "./ruleForm";

const STYLES = new Map([["$", {side: "L" as const, spaced: false, precision: 2, decimalPoint: ".", digitGroups: [",", [3]] as [string, number[]]}]]);

const commission = (): AutoRuleForm => ({
    matchAccount: "revenue:sales",
    extraQuery: "",
    postings: [{account: "expenses:commission", kind: "percent", percent: {m: 3n, p: 0}, amount: null}],
    counter: "assets:checking",
    comment: "",
    forecastOnly: true,
});

describe("ruleForm", () => {
    it("a blank rule is one empty percentage posting against the given counter, and not complete", () => {
        const form = blankRuleForm("assets:checking");
        expect(form).toEqual({
            matchAccount: "",
            extraQuery: "",
            postings: [{account: "", kind: "percent", percent: null, amount: null}],
            counter: "assets:checking",
            comment: "",
            forecastOnly: true,
        });
        expect(ruleFormComplete(form)).toBe(false);
    });

    it("clones every level writable, even from a frozen decode", () => {
        const frozen = Object.freeze({
            ...commission(),
            postings: Object.freeze([
                Object.freeze({
                    account: "expenses:fee",
                    kind: "fixed" as const,
                    percent: null,
                    amount: Object.freeze({commodity: "$", quantity: Object.freeze({m: 25n, p: 0}), precision: 0}),
                }),
            ]),
        }) as AutoRuleForm;
        const clone = cloneRuleForm(frozen);
        expect(clone).toEqual(frozen);
        for (const level of [clone, clone.postings, clone.postings[0], clone.postings[0]!.amount, clone.postings[0]!.amount!.quantity]) {
            expect(Object.isFrozen(level)).toBe(false);
        }
    });

    it("needs a counter-account when its one posting has nothing else to balance it", () => {
        expect(ruleFormNeedsCounter(commission())).toBe(false);
        expect(ruleFormNeedsCounter({...commission(), counter: null})).toBe(true);
        expect(ruleFormNeedsCounter({...commission(), counter: "  "})).toBe(true);
        // Two postings may cancel once signed; the server judges those.
        const pair = {
            ...commission(),
            counter: null,
            postings: [...commission().postings, {account: "liabilities:payable", kind: "percent" as const, percent: {m: 3n, p: 0}, amount: null}],
        };
        expect(ruleFormNeedsCounter(pair)).toBe(false);
    });

    it("is complete once it has an account to match and every posting its account and number", () => {
        expect(ruleFormComplete(commission())).toBe(true);
        expect(ruleFormComplete({...commission(), matchAccount: "  "})).toBe(false);
        expect(ruleFormComplete({...commission(), postings: []})).toBe(false);
        expect(ruleFormComplete({...commission(), postings: [{account: "x", kind: "percent", percent: null, amount: null}]})).toBe(false);
        expect(ruleFormComplete({...commission(), postings: [{account: "x", kind: "fixed", percent: {m: 1n, p: 0}, amount: null}]})).toBe(false);
    });

    it("goes on the wire trimmed, with only the posting's own number and a blank counter as null", () => {
        const form = commission();
        form.matchAccount = " revenue:sales ";
        form.counter = "  ";
        form.postings.push({
            account: "expenses:fee",
            kind: "fixed",
            percent: {m: 9n, p: 0},
            amount: {commodity: " $", quantity: {m: 2550n, p: 2}, precision: 2},
        });
        expect(ruleFormToWire(form)).toEqual({
            matchAccount: "revenue:sales",
            extraQuery: "",
            postings: [
                {account: "expenses:commission", kind: "percent", percent: {mantissa: "3", places: 0}, amount: null},
                {account: "expenses:fee", kind: "fixed", percent: null, amount: {commodity: "$", quantity: {mantissa: "2550", places: 2}, precision: 2}},
            ],
            counter: null,
            comment: "",
            forecastOnly: true,
        });
    });

    describe("the preview", () => {
        it("says what the rule books on a typical amount, as a magnitude", () => {
            const sample = {commodity: "$", quantity: {m: -4000000n, p: 2}, precision: 2};
            expect(rulePreview(commission(), sample, STYLES)).toBe("≈ $1,200.00 to expenses:commission on each $40,000.00 to revenue:sales");
        });

        it("names the percentage when there is no sample, and a negative one with a minus", () => {
            const form = commission();
            form.postings[0]!.percent = {m: -35n, p: 1};
            expect(rulePreview(form, null, STYLES)).toBe("≈ −3.5% to expenses:commission of each posting to revenue:sales");
        });

        it("shows a fixed amount as typed, and counts what it leaves out", () => {
            const form = commission();
            form.postings.push(
                {account: "expenses:fee", kind: "fixed", percent: null, amount: {commodity: "$", quantity: {m: 25n, p: 0}, precision: 0}},
                {account: "expenses:other", kind: "percent", percent: {m: 1n, p: 0}, amount: null}
            );
            expect(rulePreview(form, {commodity: "$", quantity: {m: 100000n, p: 2}, precision: 2}, STYLES)).toBe(
                "≈ $30.00 to expenses:commission, $25.00 to expenses:fee and 1 more on each $1,000.00 to revenue:sales"
            );
        });

        it("says nothing until there is an account to match and a posting with a number", () => {
            expect(rulePreview(blankRuleForm(null), null, STYLES)).toBeNull();
            expect(rulePreview({...commission(), matchAccount: ""}, null, STYLES)).toBeNull();
        });
    });
});
