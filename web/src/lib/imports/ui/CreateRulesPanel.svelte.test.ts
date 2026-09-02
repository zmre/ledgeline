// The Create-a-rules-file panel: what is on screen before anything is written.
//
// This is the one screen in the imports feature that creates a file in the
// user's journal directory, and it is reached only when NOTHING could read
// their statement — so the person looking at it has, by definition, just been
// told the app could not help. What it shows them therefore matters more than
// most panels, and the assertions below are about exactly that:
//
//   - the guesses are visible AND correctable (a mis-detected column is the real
//     failure mode, and it is silent in hledger — a misspelled `fields` name
//     produces blank descriptions at exit 0);
//   - the shaky guesses are marked, and the confident ones are not, because a
//     badge on every row is a badge nobody reads;
//   - the file's own text is shown before it is written, not after;
//   - Create is not pressable until the one thing no CSV can supply is supplied.

import {render, screen} from "@testing-library/svelte";
import {describe, expect, it, vi} from "vitest";
import {decodeRulesDraft} from "$lib/api/nativeDecode";
import type {RulesDraft} from "../types";
import {draftForm} from "../createModel";
import {withFieldNames, withSetting, type FormItem, type RulesForm} from "../model";
import CreateRulesPanel from "./CreateRulesPanel.svelte";

/** The engine's own answer for a three-column export, one column of it uncertain. */
function draft(): RulesDraft {
    return decodeRulesDraft({
        doc: {
            id: "import/2026/bank.csv.rules",
            label: "bank",
            revision: "",
            editable: true,
            newline: "lf",
            settings: {
                skip: {value: 1, itemId: 0},
                dateFormat: {value: "%m/%d/%Y", itemId: 1},
                fields: {names: ["date", "description", "amount"], itemId: 2},
                account1: {value: "", itemId: 3},
                account2: {value: "expenses:unknown", itemId: 4},
            },
            items: [
                {id: 0, line: 1, lines: 1, kind: "directive", name: "skip", value: "1"},
                {id: 1, line: 2, lines: 1, kind: "directive", name: "date-format", value: "%m/%d/%Y"},
                {id: 2, line: 3, lines: 1, kind: "fields", names: ["date", "description", "amount"]},
                {id: 3, line: 4, lines: 1, kind: "assignment", field: "account1", value: ""},
                {id: 4, line: 5, lines: 1, kind: "assignment", field: "account2", value: "expenses:unknown"},
            ],
            warnings: [],
        },
        preview: {
            available: true,
            separator: ",",
            header: ["Posted Date", "Memo", "Gross"],
            rows: [["01/02/2026", "COFFEE ROASTERS", "-4.50"]],
            columns: 3,
            truncated: false,
        },
        columns: [
            {index: 0, field: "date", confidence: 0.95},
            {index: 1, field: "description", confidence: 0.85},
            // Read from its VALUES rather than from its header — the guess most
            // often wrong, and the one that has to be marked.
            {index: 2, field: "amount", confidence: 0.35},
        ],
        warnings: ["The amounts carry no currency symbol."],
    });
}

/**
 * The working form, DEEPLY reactive — the mapping and account panels bind into
 * these objects, and a plain array renders once and then stops agreeing with
 * itself (the trap `RulesList.svelte.test.ts` and `AliasPanel` both document).
 */
function form(items?: FormItem[]): RulesForm {
    const base = draftForm(draft().doc);
    const list = $state(items ?? base.items);
    return {...base, items: list};
}

function mount(overrides: Record<string, unknown> = {}) {
    const onItems = vi.fn();
    const onSave = vi.fn();
    const onId = vi.fn();
    const onCancel = vi.fn();
    const onRetry = vi.fn();
    render(CreateRulesPanel, {
        draft: draft(),
        form: form(),
        id: "import/2026/bank.csv.rules",
        drafting: false,
        saving: false,
        error: null,
        accountNames: ["assets:bank:checking", "expenses:unknown"],
        onId,
        onItems,
        onSave,
        onRetry,
        onCancel,
        ...overrides,
    });
    return {onItems, onSave, onId, onCancel, onRetry};
}

describe("CreateRulesPanel", () => {
    it("shows the mapping with the CSV's own header and a sample value beside each column", () => {
        // The entire reason the draft carries a preview: `fields date,
        // description, amount` tells you nothing about whether it is RIGHT.
        // `Posted Date | 01/02/2026 | date` turns checking it from an act of
        // memory into an act of reading.
        mount();
        expect(screen.getByText("Posted Date")).toBeTruthy();
        expect(screen.getByText("01/02/2026")).toBeTruthy();
        expect(screen.getByLabelText("Field name for column 1")).toHaveProperty("value", "date");
        expect(screen.getByLabelText("Field name for column 3")).toHaveProperty("value", "amount");
    });

    it("marks the uncertain guess and leaves the confident ones unmarked", () => {
        mount();
        // Whitespace-normalised: the count and its noun are separate text nodes
        // in the template, so the DOM carries the indentation between them.
        const notice = screen.getByTestId("imports-create-uncertain").textContent?.replace(/\s+/g, " ");
        expect(notice).toMatch(/2 columns were guessed/);
        // The value-derived guess is the loud one; the 0.95 header match is not
        // marked at all.
        expect(screen.getByText("Column 3: guess")).toBeTruthy();
        expect(screen.queryByText("Column 1: guess")).toBeNull();
        expect(screen.queryByText("Column 1: check this")).toBeNull();
    });

    it("shows what the file will say, before it is written", () => {
        const lines = screen.queryByTestId("imports-create-lines");
        expect(lines).toBeNull();
        mount();
        expect(screen.getByTestId("imports-create-lines").textContent).toBe(
            ["skip 1", "date-format %m/%d/%Y", "fields date, description, amount", "account1", "account2 expenses:unknown"].join("\n")
        );
    });

    it("surfaces every engine warning verbatim", () => {
        // Each one names a way the draft can be wrong that hledger will NOT
        // mention, so paraphrasing or summarising them loses the only warning
        // the user could have acted on.
        mount();
        expect(screen.getByTestId("imports-create-warnings").textContent).toContain("no currency symbol");
    });

    it("will not create until the account this statement is for is set", () => {
        const {onSave} = mount();
        const button = screen.getByTestId("imports-create-save");
        expect(button).toHaveProperty("disabled", true);
        expect(screen.getByTestId("imports-create-blocker").textContent).toMatch(/which account/i);
        button.click();
        expect(onSave).not.toHaveBeenCalled();
    });

    it("becomes pressable once the account is set", () => {
        const ready = form();
        ready.items = withSetting(ready.items, "account1", "assets:bank:checking");
        const {onSave} = mount({form: ready});
        const button = screen.getByTestId("imports-create-save");
        expect(button).toHaveProperty("disabled", false);
        expect(screen.queryByTestId("imports-create-blocker")).toBeNull();
        button.click();
        expect(onSave).toHaveBeenCalledOnce();
    });

    it("becomes pressable when a column is mapped to account1 instead, and says so", () => {
        // A QuickBooks-style export naming a different account per row: the
        // fixed text field stays blank on purpose, and that has to be enough.
        const mapped = form();
        mapped.items = withFieldNames(mapped.items, ["date", "account1", "amount"]);
        const {onSave} = mount({form: mapped});
        const button = screen.getByTestId("imports-create-save");
        expect(button).toHaveProperty("disabled", false);
        expect(screen.queryByTestId("imports-create-blocker")).toBeNull();
        expect(screen.getByTestId("imports-create-account1-mapped").textContent).toContain("account1");
        button.click();
        expect(onSave).toHaveBeenCalledOnce();
    });

    it("refuses a name that is not a rules id, and says which rule it broke", () => {
        mount({id: "bank.csv"});
        expect(screen.getByTestId("imports-create-save")).toHaveProperty("disabled", true);
        expect(screen.getByTestId("imports-create-blocker").textContent).toMatch(/\.rules/);
    });

    it("reports a correction to the column mapping", () => {
        // The failure this screen exists for. A mis-detected column has to be
        // fixable HERE — after the write it is an edit of a file that already
        // imported things wrongly.
        const {onItems} = mount();
        const input = screen.getByLabelText("Field name for column 3") as HTMLInputElement;
        input.value = "amount-out";
        input.dispatchEvent(new Event("input", {bubbles: true}));
        expect(onItems).toHaveBeenCalled();
        const next = onItems.mock.calls.at(-1)?.[0] as FormItem[];
        const fields = next.find((item) => item.kind === "fields");
        expect(fields).toMatchObject({names: ["date", "description", "amount-out"]});
    });

    it("offers the currency control its own warning points at", () => {
        // The engine warns "set Currency below" when the amounts carry no
        // symbol — hledger reads a commodity-less amount as a commodity of its
        // own, so those rows never add up with the journal's `$` ones. A
        // warning naming a control that did not exist would be worse than no
        // warning, and this panel had exactly that gap until it was driven for
        // real.
        const {onItems} = mount();
        const input = screen.getByTestId("imports-create-currency") as HTMLInputElement;
        expect(input.value).toBe("");
        input.value = "$";
        input.dispatchEvent(new Event("change", {bubbles: true}));
        const next = onItems.mock.calls.at(-1)?.[0] as FormItem[];
        expect(next.find((item) => item.kind === "assignment" && item.field === "currency")).toMatchObject({value: "$"});
    });

    it("says it is reading rather than showing an empty form while the draft is in flight", () => {
        mount({drafting: true, draft: null, form: null});
        expect(screen.getByRole("status").textContent).toMatch(/Reading your file/);
        expect(screen.queryByTestId("imports-create-save")).toBeNull();
    });

    it("shows the engine's own sentence when the draft fails, with a way back", () => {
        // Verbatim: it is the one that says whether the name was taken, the
        // upload expired, or something else entirely.
        const {onRetry} = mount({
            drafting: false,
            draft: null,
            form: null,
            error: '"bank.csv.rules" already exists.',
        });
        expect(screen.getByTestId("imports-create-error").textContent).toContain("already exists");
        screen.getByTestId("imports-create-retry").click();
        expect(onRetry).toHaveBeenCalledOnce();
    });

    it("freezes every control while the write is in flight", () => {
        const ready = form();
        ready.items = withSetting(ready.items, "account1", "assets:bank:checking");
        mount({form: ready, saving: true});
        expect(screen.getByTestId("imports-create-save")).toHaveProperty("disabled", true);
        expect(screen.getByTestId("imports-create-cancel")).toHaveProperty("disabled", true);
        expect(screen.getByTestId("imports-create-id")).toHaveProperty("disabled", true);
    });
});
