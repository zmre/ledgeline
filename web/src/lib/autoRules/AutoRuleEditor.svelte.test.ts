// The rule editor, mounted on a bare form (plan 30 Phase 4).
//
// What only a mount can answer: that each field writes the form it was handed,
// that `onChange` fires on a committed edit and not on an unreadable one, and
// that the preview follows the form. No store: the editor has none, which is
// what lets Settings reuse it.

import {fireEvent, render, screen, within} from "@testing-library/svelte";
import {tick} from "svelte";
import {describe, expect, it, vi} from "vitest";
import AutoRuleEditor from "./AutoRuleEditor.svelte";
import type {AutoRuleForm} from "./ruleForm";

const STYLES = new Map([["$", {side: "L" as const, spaced: false, precision: 2, decimalPoint: ".", digitGroups: [",", [3]] as [string, number[]]}]]);
const ACCOUNTS = ["revenue:sales", "expenses:commission", "expenses:fee", "assets:checking"];

function mount(form: AutoRuleForm, sample: {commodity: string; quantity: {m: bigint; p: number}; precision: number} | null = null) {
    const onChange = vi.fn();
    // The host's `$state`, as the Projections table holds it.
    const state = $state(form);
    render(AutoRuleEditor, {form: state, accountNames: ACCOUNTS, name: "rule 1", commodity: "$", sample, styles: STYLES, onChange});
    return {form: state, onChange};
}

const commission = (): AutoRuleForm => ({
    matchAccount: "revenue:sales",
    extraQuery: "",
    postings: [{account: "expenses:commission", kind: "percent", percent: {m: 3n, p: 0}, amount: null}],
    counter: "assets:checking",
    comment: "",
    forecastOnly: true,
});

async function commit(label: string, value: string): Promise<void> {
    const field = screen.getByLabelText(label);
    await fireEvent.input(field, {target: {value}});
    await fireEvent.change(field, {target: {value}});
    await fireEvent.blur(field);
    await tick();
}

describe("COMPONENT AutoRuleEditor", () => {
    it("shows the form's fields and what it comes to on a typical amount", () => {
        mount(commission(), {commodity: "$", quantity: {m: -4000000n, p: 2}, precision: 2});
        expect((screen.getByLabelText("Matched account for rule 1") as HTMLInputElement).value).toBe("revenue:sales");
        expect((screen.getByLabelText("Account for posting 1 of rule 1") as HTMLInputElement).value).toBe("expenses:commission");
        expect((screen.getByLabelText("Percent for posting 1 of rule 1") as HTMLInputElement).value).toBe("3");
        expect((screen.getByLabelText("Counter-account for rule 1") as HTMLInputElement).value).toBe("assets:checking");
        expect(screen.getByTestId("auto-rule-preview").textContent).toBe("≈ $1,200.00 to expenses:commission on each $40,000.00 to revenue:sales");
        // A single posting cannot be removed: a rule needs one.
        expect(screen.queryByLabelText("Remove posting 1 of rule 1")).toBeNull();
    });

    it("writes a typed percentage into the form, positive as typed, and says so", async () => {
        const {form, onChange} = mount(commission(), {commodity: "$", quantity: {m: 100000n, p: 2}, precision: 2});
        await commit("Percent for posting 1 of rule 1", "3.5");
        expect(form.postings[0]!.percent).toEqual({m: 35n, p: 1});
        expect(onChange).toHaveBeenCalled();
        expect(screen.getByTestId("auto-rule-preview").textContent).toContain("$35.00 to expenses:commission");
    });

    it("keeps an unreadable number in its box, marks it, and leaves the form alone", async () => {
        const {form, onChange} = mount(commission());
        await commit("Percent for posting 1 of rule 1", "three");
        const box = screen.getByLabelText("Percent for posting 1 of rule 1") as HTMLInputElement;
        expect(box.value).toBe("three");
        expect(box.getAttribute("aria-invalid")).toBe("true");
        expect(form.postings[0]!.percent).toEqual({m: 3n, p: 0});
        expect(onChange).not.toHaveBeenCalled();
    });

    it("switches a posting to a fixed amount in a commodity of the user's choosing", async () => {
        const {form} = mount(commission());
        await fireEvent.change(screen.getByLabelText("Amount kind for posting 1 of rule 1"), {target: {value: "fixed"}});
        await tick();
        expect(form.postings[0]).toMatchObject({kind: "fixed", percent: null, amount: null});
        await commit("Commodity for posting 1 of rule 1", "EUR");
        await commit("Amount for posting 1 of rule 1", "25.50");
        expect(form.postings[0]!.amount).toEqual({commodity: "EUR", quantity: {m: 2550n, p: 2}, precision: 2});
    });

    it("adds and removes postings", async () => {
        const {form, onChange} = mount(commission());
        await fireEvent.click(screen.getByRole("button", {name: "+ Add a posting"}));
        await tick();
        expect(form.postings).toHaveLength(2);
        expect(screen.getAllByTestId("auto-rule-posting")).toHaveLength(2);
        await fireEvent.click(screen.getByLabelText("Remove posting 1 of rule 1"));
        await tick();
        expect(form.postings.map((posting) => posting.account)).toEqual([""]);
        expect(onChange).toHaveBeenCalledTimes(2);
    });

    it("edits the matched account, the counter-account and the comment", async () => {
        const {form, onChange} = mount(commission());
        await commit("Matched account for rule 1", "revenue:services");
        await commit("Counter-account for rule 1", "");
        await commit("Comment for rule 1", "sales commission");
        expect(form.matchAccount).toBe("revenue:services");
        expect(form.counter).toBeNull();
        expect(form.comment).toBe("sales commission");
        expect(onChange).toHaveBeenCalled();
    });

    it("offers extra query terms on request, and shows them when the rule has some", async () => {
        const {form} = mount(commission());
        expect(screen.queryByLabelText("Extra query terms for rule 1")).toBeNull();
        await fireEvent.click(screen.getByRole("button", {name: "+ query terms"}));
        await tick();
        await commit("Extra query terms for rule 1", "desc:acme");
        expect(form.extraQuery).toBe("desc:acme");

        document.body.innerHTML = "";
        mount({...commission(), extraQuery: "amt:>1000"});
        expect((screen.getByLabelText("Extra query terms for rule 1") as HTMLInputElement).value).toBe("amt:>1000");
        expect(within(screen.getByTestId("auto-rule-editor")).getByText("forecasts only")).toBeTruthy();
    });
});
