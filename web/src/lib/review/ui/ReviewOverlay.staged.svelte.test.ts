// The review overlay over an import's transactions before they are written
// (plan 32 Phase 4): its title, saves into the import's sink and never to the
// network, the two Similar groups (D24), the run summary, closing, and
// "Remember as a rule" (D25) — offered only on a CSV card whose description or
// account changed, saved BEFORE the card, then the import regenerated.

import {fireEvent, render, screen, within} from "@testing-library/svelte";
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import {decodeCsvStagedTxn, decodeQbStagedTxn} from "#lib/api/importReviewDecode.js";
import type {EditResult} from "#lib/api/editFailure.js";
import {keymap} from "#lib/keys/keymap.svelte.js";
import type {RulePreview, RuleSource, StagedSave, StagedTxn} from "#lib/imports/reviewTypes.js";
import {importPrefill} from "#lib/imports/ruleBuilder.js";
import {resetReview, review, type StagedKind} from "#lib/stores/review.svelte.js";
import {connectFakeEngine} from "#lib/testing/fakeEngine.js";
import {
    qbStagedTxnWire,
    queuePageWire,
    reviewTxnWire,
    serveReview,
    similarWire,
    stagedTxnWire,
    txnFixture,
    type SeenRequest,
} from "#lib/testing/reviewFixture.js";
import {accountField, card, MOD, press, primaryButton, settle} from "#lib/testing/reviewOverlay.js";
import {reviewLaunch, resetReviewLaunch, type StagedLaunch} from "./launch.svelte";
import ReviewOverlay from "./ReviewOverlay.svelte";

const staged = (ordinal: number, description: string, key: string): StagedTxn =>
    decodeCsvStagedTxn(stagedTxnWire(txnFixture(ordinal + 1, description), {ordinal, key}), "staged");

/** HARDWARE STORE 1 and 2 share a key; PEETS stands alone. */
const ITEMS = (): StagedTxn[] => [
    staged(0, "HARDWARE STORE 1", "hardware store"),
    staged(1, "PEETS", "peets"),
    staged(2, "HARDWARE STORE 2", "hardware store"),
];
const JOURNAL_SIBLING = reviewTxnWire(txnFixture(40, "HARDWARE STORE 9"));

const RECORDS = {
    aligned: true,
    header: ["Date", "Description", "Amount"],
    fields: ["date", "description", "amount"],
    rows: [
        ["01/19/2024", "HARDWARE STORE 1", "-62.13"],
        ["01/20/2024", "PEETS", "-4.20"],
        ["01/21/2024", "HARDWARE STORE 2", "-9.99"],
    ],
} as const;
const PREVIEW: RulePreview = {ok: true, matched: 2, total: 3, alreadyCategorised: 0, changed: [], changedTotal: 2};

interface Harness {
    saves: StagedSave[];
    order: string[];
    closed: boolean[];
    seen: SeenRequest[];
    rules: RuleSource & {save: ReturnType<typeof vi.fn>; refresh: ReturnType<typeof vi.fn>};
}

/** Serve the journal side (similar by key), and open a staged run through the launch store, as the import panels do. */
async function open(kind: StagedKind, options: {items?: StagedTxn[]; rules?: boolean; ruleSave?: EditResult; writeLabel?: string} = {}): Promise<Harness> {
    const seen = serveReview({
        queue: () => queuePageWire("unknowns", []),
        similarKey: (key) => ({...similarWire(0, key === "hardware store" ? [JOURNAL_SIBLING] : []), key}),
    });
    const order: string[] = [];
    const saves: StagedSave[] = [];
    const closed: boolean[] = [];
    const rules = {
        prefill: importPrefill(RECORDS),
        fileLabel: "checking.csv",
        rulesLabel: "checking.csv.rules",
        preview: vi.fn().mockResolvedValue(PREVIEW),
        save: vi.fn(async (): Promise<EditResult> => (order.push("rule"), options.ruleSave ?? {ok: true})),
        refresh: vi.fn(async () => (order.push("refresh"), null)),
    };
    const launch: StagedLaunch = {
        kind,
        items: options.items ?? ITEMS(),
        sink: {stage: (save) => (order.push("card"), saves.push(save), {ok: true})},
        rules: options.rules === false ? null : rules,
        writeLabel: options.writeLabel,
        onClose: (saved) => closed.push(saved),
    };
    reviewLaunch.openStaged(launch);
    render(ReviewOverlay, {props: {kind, staged: reviewLaunch.staged}});
    await settle();
    return {saves, order, closed, seen, rules};
}

/** Wait for the card's similar set (fetched by key) to land. */
async function similarLoaded(): Promise<void> {
    await vi.waitFor(() => expect(screen.getByTestId("review-similar-journal")).toBeTruthy());
}

beforeEach(async () => {
    resetReview();
    resetReviewLaunch();
    await connectFakeEngine();
});
afterEach(() => {
    vi.unstubAllGlobals();
    keymap.reset();
});

describe("COMPONENT ReviewOverlay — over an import", () => {
    it("is titled for the run it is", async () => {
        await open("all");
        expect(screen.getByRole("dialog", {name: "Review All"})).toBeTruthy();
        expect(review.source).toBe("staged");
        expect(review.count).toBe(3);
    });

    it("titles an Unknowns run too", async () => {
        await open("unknowns");
        expect(screen.getByRole("dialog", {name: "Review Unknowns"})).toBeTruthy();
    });

    it("saves into the import, never to the network", async () => {
        const {saves, seen} = await open("all");
        await fireEvent.input(accountField(), {target: {value: "expenses:home:supplies"}});
        await press("Enter", MOD);
        expect(saves).toHaveLength(1);
        expect(saves[0]?.overrides.map((entry) => entry.item.ref)).toEqual([{kind: "csv", ordinal: 0}]);
        expect(saves[0]?.overrides[0]?.ops).toContainEqual({op: "setPostingAccount", posting: 0, account: "expenses:home:supplies"});
        expect(seen.some((request) => request.method === "POST")).toBe(false);
        expect(review.current?.item.txn.index).toBe(2);
    });

    it("has no full editor for a transaction that is in no file yet", async () => {
        await open("all");
        expect(screen.queryByRole("button", {name: /Open in full editor/})).toBeNull();
        expect(screen.queryByRole("button", {name: /Settings/})).toBeNull();
        await press("o");
        expect(screen.getByTestId("review-overlay")).toBeTruthy();
        expect(JSON.stringify(keymap.help)).not.toContain("Open in the full editor");
    });

    it("says, at the end, that the edits are applied by Write changes — and closing keeps them", async () => {
        const {closed} = await open("unknowns", {items: [staged(1, "PEETS", "peets")]});
        await fireEvent.input(accountField(), {target: {value: "expenses:food:coffee"}});
        await press("Enter", MOD);
        expect(screen.getByTestId("review-staged-note").textContent?.replace(/\s+/g, " ")).toContain("applied when you press Write changes");
        await fireEvent.click(screen.getByRole("button", {name: /Done/}));
        expect(closed).toEqual([true]);
        expect(reviewLaunch.staged).toBeNull();
    });

    it("names QuickBooks' own write button in the summary", async () => {
        const item = decodeQbStagedTxn(qbStagedTxnWire(txnFixture(1, "QB DEPOSIT"), "441"), "qb");
        await open("all", {items: [item], rules: false, writeLabel: "Import"});
        await press("ArrowRight");
        expect(screen.getByTestId("review-staged-note").textContent?.replace(/\s+/g, " ")).toContain("applied when you press Import");
    });
});

describe("COMPONENT ReviewOverlay — Similar over an import (D24)", () => {
    it("shows two groups, each collapsed with its own count, nothing checked", async () => {
        await open("all");
        await similarLoaded();
        const inImport = screen.getByTestId("review-similar-staged");
        const inJournal = screen.getByTestId("review-similar-journal");
        expect(inImport.textContent).toContain("Similar in this import");
        expect(inJournal.textContent).toContain("Similar already in the journal");
        expect(screen.getByTestId("review-similar-staged-count").textContent).toBe("1");
        expect(screen.getByTestId("review-similar-journal-count").textContent).toBe("1");
        expect(inImport.dataset.open).toBeUndefined();
        expect(inJournal.dataset.open).toBeUndefined();
        for (const box of [...inImport.querySelectorAll("input"), ...inJournal.querySelectorAll("input")])
            expect((box as HTMLInputElement).checked).toBe(false);
        expect(screen.queryByTestId("review-similar")).toBeNull();
    });

    it("sends a checked import sibling as an override and a checked journal one as a journal edit", async () => {
        const {saves} = await open("all");
        await similarLoaded();
        await fireEvent.input(accountField(), {target: {value: "expenses:home:supplies"}});
        await fireEvent.click(screen.getByTestId("review-similar-staged-all"));
        await fireEvent.click(screen.getByTestId("review-similar-journal-all"));
        await press("Enter", MOD);
        const move = {op: "setPostingAccount", posting: 0, account: "expenses:home:supplies"};
        expect(saves[0]?.overrides.map((entry) => entry.item.ref)).toEqual([
            {kind: "csv", ordinal: 0},
            {kind: "csv", ordinal: 2},
        ]);
        expect(saves[0]?.overrides[1]?.ops).toEqual([move]);
        expect(saves[0]?.journalEdits).toEqual([{index: 40, fingerprint: "fp-40-0", ops: [move]}]);
    });

    it("opens both with s and checks both with ⇧A", async () => {
        const {saves} = await open("all");
        await similarLoaded();
        await fireEvent.input(accountField(), {target: {value: "expenses:home:supplies"}});
        card().focus();
        await press("s");
        expect(screen.getByTestId("review-similar-staged").dataset.open).toBe("");
        expect(screen.getByTestId("review-similar-journal").dataset.open).toBe("");
        await press("A");
        await press("Enter", MOD);
        expect(saves[0]?.overrides).toHaveLength(2);
        expect(saves[0]?.journalEdits).toHaveLength(1);
    });

    it("checks only what was checked: an unchecked group adds nothing", async () => {
        const {saves} = await open("all");
        await similarLoaded();
        await fireEvent.input(accountField(), {target: {value: "expenses:home:supplies"}});
        await fireEvent.click(screen.getByTestId("review-similar-journal-all"));
        await press("Enter", MOD);
        expect(saves[0]?.overrides).toHaveLength(1);
        expect(saves[0]?.journalEdits).toHaveLength(1);
    });
});

describe("COMPONENT ReviewOverlay — Remember as a rule (D25)", () => {
    it("is offered only once the description or an account changed", async () => {
        await open("all");
        expect(screen.queryByTestId("review-rule")).toBeNull();
        await fireEvent.input(accountField(), {target: {value: "expenses:home:supplies"}});
        expect(screen.getByTestId("review-rule")).toBeTruthy();
        expect((screen.getByTestId("review-rule-toggle") as HTMLInputElement).checked).toBe(false);
    });

    it("toggles with R, listed in the ? sheet", async () => {
        await open("all");
        expect(JSON.stringify(keymap.help)).not.toContain("Remember as a rule");
        await fireEvent.input(accountField(), {target: {value: "expenses:home:supplies"}});
        card().focus();
        await press("R");
        expect((screen.getByTestId("review-rule-toggle") as HTMLInputElement).checked).toBe(true);
        expect(JSON.stringify(keymap.help)).toContain("Remember as a rule");
        await press("R");
        expect((screen.getByTestId("review-rule-toggle") as HTMLInputElement).checked).toBe(false);
    });

    it("is not offered for a split", async () => {
        await open("all");
        await fireEvent.input(screen.getByLabelText("Amount for posting 1"), {target: {value: "1.00"}});
        await fireEvent.input(accountField(), {target: {value: "expenses:home:supplies"}});
        expect(screen.queryByTestId("review-rule")).toBeNull();
    });

    it("is not offered over a QuickBooks import (D30)", async () => {
        const item = decodeQbStagedTxn(qbStagedTxnWire(txnFixture(1, "QB DEPOSIT"), "441"), "qb");
        await open("all", {items: [item], rules: false});
        await fireEvent.input(accountField(), {target: {value: "expenses:home:supplies"}});
        expect(screen.queryByTestId("review-rule")).toBeNull();
        card().focus();
        await press("R");
        expect(screen.queryByTestId("review-rule")).toBeNull();
    });

    it("says on top of the deck when the import can't be regenerated after the rule, until dismissed", async () => {
        const {rules} = await open("all");
        rules.refresh.mockImplementation(async () => "hledger: could not parse checking.csv.rules");
        await fireEvent.input(accountField(), {target: {value: "expenses:home:supplies"}});
        await fireEvent.click(screen.getByTestId("review-rule-toggle"));
        await press("Enter", MOD);
        const notice = await screen.findByTestId("review-import-notice");
        expect(notice.textContent).toContain("could not parse checking.csv.rules");
        await fireEvent.click(within(notice).getByRole("button", {name: "Dismiss"}));
        expect(screen.queryByTestId("review-import-notice")).toBeNull();
    });

    it("saves the rule first, then the card, then regenerates the import", async () => {
        const {order, rules, saves} = await open("all");
        await fireEvent.input(accountField(), {target: {value: "expenses:home:supplies"}});
        await fireEvent.click(screen.getByTestId("review-rule-toggle"));
        await press("Enter", MOD);
        await vi.waitFor(() => expect(order).toEqual(["rule", "card", "refresh"]));
        expect(rules.save).toHaveBeenCalledWith({
            matchers: [{field: "description", pattern: "hardware.*store"}],
            assignments: [{field: "account1", value: "expenses:home:supplies"}],
        });
        expect(saves).toHaveLength(1);
        expect(review.current?.item.txn.index).toBe(2);
    });

    it("leaves the card, edits and all, when the rules file refuses the rule", async () => {
        const {order} = await open("all", {ruleSave: {ok: false, failure: {kind: "conflict", message: "the rules file changed on disk"}}});
        await fireEvent.input(accountField(), {target: {value: "expenses:home:supplies"}});
        await fireEvent.click(screen.getByTestId("review-rule-toggle"));
        await fireEvent.click(primaryButton());
        await vi.waitFor(() => expect(screen.getByTestId("review-rule-save-error").textContent).toContain("the rules file changed on disk"));
        expect(order).toEqual(["rule"]);
        expect(review.current?.item.txn.index).toBe(1);
        expect(accountField().value).toBe("expenses:home:supplies");
    });

    it("saves the card alone with the toggle off", async () => {
        const {order} = await open("all");
        await fireEvent.input(accountField(), {target: {value: "expenses:home:supplies"}});
        await press("Enter", MOD);
        expect(order).toEqual(["card"]);
    });

    it("starts off again on the next card", async () => {
        await open("all");
        await fireEvent.input(accountField(), {target: {value: "expenses:home:supplies"}});
        await fireEvent.click(screen.getByTestId("review-rule-toggle"));
        await press("Enter", MOD);
        await vi.waitFor(() => expect(review.current?.item.txn.index).toBe(2));
        await fireEvent.input(accountField(), {target: {value: "expenses:food:coffee"}});
        expect((within(card()).getByTestId("review-rule-toggle") as HTMLInputElement).checked).toBe(false);
    });
});
