// "Remember as a rule" (plan 32 D25) mounted on its own: the prefilled match,
// what it sets, and the live check — debounced, each newer check aborting the
// last, and every answer it can give (a count and the rows it changes,
// hledger's refusal, a failed request, no rules file). Saving it is the
// overlay's job, in `ReviewOverlay.staged.svelte.test.ts`.

import {fireEvent, render, screen} from "@testing-library/svelte";
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import {cardChange, reviewFormOf, type CardChange} from "#lib/api/editMapping.js";
import {decodeCsvStagedTxn} from "#lib/api/importReviewDecode.js";
import type {RulePreview, RuleSource, RuleSpec, StagedRecords, StagedTxn} from "#lib/imports/reviewTypes.js";
import {stagedTxnWire, txnFixture} from "#lib/testing/reviewFixture.js";
import {RULE_PREVIEW_DEBOUNCE_MS} from "#lib/imports/ruleBuilder.js";
import RuleBuilder from "./RuleBuilder.svelte";

const RECORDS: StagedRecords = {
    aligned: true,
    header: ["Date", "Description", "Amount"],
    fields: ["date", "description", "amount"],
    rows: [
        ["01/15/2024", "ACME PAYROLL", "3000.00"],
        ["01/19/2024", "HARDWARE STORE #0042", "-62.13"],
    ],
};

const ITEM: StagedTxn = decodeCsvStagedTxn(stagedTxnWire(txnFixture(2, "HARDWARE STORE #0042"), {ordinal: 1, key: "hardware store"}), "item");

/** The card's change: the unknown posting recategorised, and optionally renamed. */
function changeOf(account = "expenses:home:supplies", description: string | null = null): CardChange {
    const form = reviewFormOf(ITEM.txn);
    (form.postings[0] as {account: string}).account = account;
    if (description !== null) form.description = description;
    const change = cardChange(ITEM.txn, form);
    if (change === null) throw new Error("expected a change");
    return change;
}

const OK: RulePreview = {
    ok: true,
    matched: 2,
    total: 5,
    alreadyCategorised: 1,
    changed: [
        {
            ordinal: 1,
            before: "2024-01-19 HARDWARE STORE #0042\n    assets:bank:checking  $-62.13\n    expenses:unknown\n",
            after: "2024-01-19 HARDWARE STORE #0042\n    assets:bank:checking  $-62.13\n    expenses:home:supplies\n",
        },
    ],
    changedTotal: 1,
};

function sourceWith(preview: RuleSource["preview"], records: StagedRecords = RECORDS): RuleSource {
    return {records, fileLabel: "checking.csv", rulesLabel: "checking.csv.rules", preview, save: vi.fn(), refresh: vi.fn(async () => null)};
}

function mount(props: {source: RuleSource; on?: boolean; change?: CardChange; item?: StagedTxn}) {
    return render(RuleBuilder, {
        props: {source: props.source, item: props.item ?? ITEM, original: (props.item ?? ITEM).txn, change: props.change ?? changeOf(), on: props.on ?? true},
    });
}

const pattern = (): HTMLInputElement => screen.getByTestId("review-rule-pattern") as HTMLInputElement;
const field = (): HTMLSelectElement => screen.getByTestId("review-rule-field") as HTMLSelectElement;
const check = (): string => (screen.getByTestId("review-rule-check").textContent ?? "").replace(/\s+/g, " ").trim();

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

describe("COMPONENT RuleBuilder — the prefilled rule", () => {
    it("matches the description field with the key's tokens, escaped and joined", () => {
        mount({source: sourceWith(vi.fn().mockResolvedValue(OK))});
        expect(field().value).toBe("description");
        expect(pattern().value).toBe("hardware.*store");
    });

    it("matches the whole record when the dry run could not line records up", () => {
        mount({source: sourceWith(vi.fn().mockResolvedValue(OK), {aligned: false, header: null, fields: ["date", "description", "amount"], rows: []})});
        expect(field().value).toBe("");
        expect(pattern().value).toBe("hardware.*store");
    });

    it("sets the account slot that held the unknown, and offers the rename unchecked", () => {
        const {component} = mount({source: sourceWith(vi.fn().mockResolvedValue(OK)), change: changeOf("expenses:home:supplies", "Hardware Store")});
        const targets = screen.getAllByTestId("review-rule-target") as HTMLInputElement[];
        expect(targets.map((box) => [box.dataset.field, box.checked])).toEqual([
            ["account1", true],
            ["description", false],
        ]);
        expect(component.currentRule()).toEqual({
            matchers: [{field: "description", pattern: "hardware.*store"}],
            assignments: [{field: "account1", value: "expenses:home:supplies"}],
        } satisfies RuleSpec);
    });

    it("follows the user's ticks, scope and pattern", async () => {
        const {component} = mount({source: sourceWith(vi.fn().mockResolvedValue(OK)), change: changeOf("expenses:home:supplies", "Hardware Store")});
        const [account, description] = screen.getAllByTestId("review-rule-target") as HTMLInputElement[];
        await fireEvent.click(description as HTMLInputElement);
        await fireEvent.change(field(), {target: {value: ""}});
        await fireEvent.input(pattern(), {target: {value: "hardware"}});
        expect(component.currentRule()).toEqual({
            matchers: [{field: null, pattern: "hardware"}],
            assignments: [
                {field: "account1", value: "expenses:home:supplies"},
                {field: "description", value: "Hardware Store"},
            ],
        });
        await fireEvent.click(account as HTMLInputElement);
        await fireEvent.click(description as HTMLInputElement);
        expect(component.currentRule()).toBeNull();
        expect(check()).toContain("Give the rule a pattern and something to set");
    });
});

describe("COMPONENT RuleBuilder — the live check", () => {
    it("waits for the rule to sit still, then asks once", async () => {
        const preview = vi.fn().mockResolvedValue(OK);
        mount({source: sourceWith(preview)});
        await vi.advanceTimersByTimeAsync(RULE_PREVIEW_DEBOUNCE_MS - 50);
        await fireEvent.input(pattern(), {target: {value: "hardware"}});
        await vi.advanceTimersByTimeAsync(RULE_PREVIEW_DEBOUNCE_MS - 50);
        await fireEvent.input(pattern(), {target: {value: "hardware.*st"}});
        expect(preview).not.toHaveBeenCalled();
        await vi.advanceTimersByTimeAsync(RULE_PREVIEW_DEBOUNCE_MS);
        expect(preview).toHaveBeenCalledTimes(1);
        expect((preview.mock.calls[0] as [RuleSpec])[0].matchers).toEqual([{field: "description", pattern: "hardware.*st"}]);
    });

    it("aborts a check the rule has moved past, and drops its answer", async () => {
        const signals: AbortSignal[] = [];
        let releaseFirst: (value: RulePreview) => void = () => {};
        const preview = vi.fn((_rule: RuleSpec, signal?: AbortSignal) => {
            signals.push(signal as AbortSignal);
            if (signals.length === 1) return new Promise<RulePreview>((resolve) => (releaseFirst = resolve));
            return Promise.resolve({...OK, matched: 4});
        });
        mount({source: sourceWith(preview)});
        await vi.advanceTimersByTimeAsync(RULE_PREVIEW_DEBOUNCE_MS);
        expect(preview).toHaveBeenCalledTimes(1);
        await fireEvent.input(pattern(), {target: {value: "hardware"}});
        expect(signals[0]?.aborted).toBe(true);
        await vi.advanceTimersByTimeAsync(RULE_PREVIEW_DEBOUNCE_MS);
        expect(preview).toHaveBeenCalledTimes(2);
        expect(signals[1]?.aborted).toBe(false);
        // The superseded answer arrives late and is ignored.
        releaseFirst({...OK, matched: 99});
        await vi.advanceTimersByTimeAsync(0);
        expect(screen.getByTestId("review-rule-count").textContent).toContain("4");
        expect(screen.getByTestId("review-rule-count").textContent).not.toContain("99");
    });

    it("asks nothing while the toggle is off", async () => {
        const preview = vi.fn().mockResolvedValue(OK);
        mount({source: sourceWith(preview), on: false});
        expect((screen.getByTestId("review-rule-toggle") as HTMLInputElement).checked).toBe(false);
        await vi.advanceTimersByTimeAsync(RULE_PREVIEW_DEBOUNCE_MS * 4);
        expect(preview).not.toHaveBeenCalled();
        await fireEvent.click(screen.getByTestId("review-rule-toggle"));
        await vi.advanceTimersByTimeAsync(RULE_PREVIEW_DEBOUNCE_MS);
        expect(preview).toHaveBeenCalledTimes(1);
    });

    it("says how many rows match, and lists the rows it would change", async () => {
        mount({source: sourceWith(vi.fn().mockResolvedValue(OK))});
        await vi.advanceTimersByTimeAsync(RULE_PREVIEW_DEBOUNCE_MS);
        expect(screen.getByTestId("review-rule-count").textContent?.replace(/\s+/g, " ").trim()).toBe(
            "Matches 2 of 5 rows in checking.csv · 1 already categorised"
        );
        expect(screen.queryByTestId("review-rule-changes")).toBeNull();
        await fireEvent.click(screen.getByTestId("review-rule-changes-toggle"));
        const row = screen.getByTestId("review-rule-change").textContent ?? "";
        expect(row).toContain("2024-01-19 HARDWARE STORE #0042");
        expect(row).toContain("− expenses:unknown");
        expect(row).toContain("+ expenses:home:supplies");
    });

    it("warns when nothing matches", async () => {
        mount({source: sourceWith(vi.fn().mockResolvedValue({...OK, matched: 0, changed: [], changedTotal: 0}))});
        await vi.advanceTimersByTimeAsync(RULE_PREVIEW_DEBOUNCE_MS);
        expect(check()).toContain("No row in this import matches");
    });

    it("shows hledger's refusal verbatim", async () => {
        mount({source: sourceWith(vi.fn().mockResolvedValue({ok: false, stderr: "hledger: this rules file includes another"}))});
        await vi.advanceTimersByTimeAsync(RULE_PREVIEW_DEBOUNCE_MS);
        expect(screen.getByTestId("review-rule-refused").textContent).toContain("hledger: this rules file includes another");
    });

    it("shows a failed request inline", async () => {
        mount({source: sourceWith(vi.fn().mockRejectedValue(new Error("the pattern has an unescaped (")))});
        await vi.advanceTimersByTimeAsync(RULE_PREVIEW_DEBOUNCE_MS);
        expect(screen.getByTestId("review-rule-error").textContent).toContain("the pattern has an unescaped (");
    });

    it("says when there is no rules file or journal to check against", async () => {
        mount({source: sourceWith(vi.fn().mockResolvedValue(null))});
        await vi.advanceTimersByTimeAsync(RULE_PREVIEW_DEBOUNCE_MS);
        expect(check()).toContain("needs a rules file and a journal");
    });
});
