// "Remember as a rule" on a JOURNAL card (plan 32 D27): the overlay asks for
// each card's rules files in the background as the card appears (aborting it if
// the card leaves first), shows the toggle only when there are some, and puts
// the file line above Match: the engine's recommendation preselected, or —
// when the engine can't decide — a picker with nothing chosen, and no rule
// saved until the user picks. Saving with the toggle on writes the rule, then
// the card (the usual journal batch).
//
// `loadRuleChoice` is the seam: the choice itself is the real `ruleChoice`
// over a fake rules I/O, so the sources the builder gets are the real ones.

import {fireEvent, screen} from "@testing-library/svelte";
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import type {EditResult} from "#lib/api/editFailure.js";
import type {RuleChoice, RulesCandidate, RulesFor} from "#lib/imports/reviewTypes.js";
import {keymap} from "#lib/keys/keymap.svelte.js";
import {resetReview, review} from "#lib/stores/review.svelte.js";
import {connectFakeEngine} from "#lib/testing/fakeEngine.js";
import {deferred, queueItemWire, queuePageWire, reviewTxnWire, serveReview, txnFixture, type SeenRequest} from "#lib/testing/reviewFixture.js";
import {accountField, card, MOD, mountOverlay, press, primaryButton} from "#lib/testing/reviewOverlay.js";
import {loadRuleChoice, ruleChoice, type JournalRuleIo} from "../journalRules";
import {resetReviewLaunch} from "./launch.svelte";

vi.mock("../journalRules", async (importOriginal) => ({...(await importOriginal<typeof import("../journalRules")>()), loadRuleChoice: vi.fn()}));
const load = vi.mocked(loadRuleChoice);

const CITI_2026 = "import/citi-2026.csv.rules";
const CITI_2025 = "import/citi-2025.csv.rules";
const AMEX = "import/amex.csv.rules";
const AMEX_BUSINESS = "import/amex-business.csv.rules";

function candidate(rulesId: string, extra: Partial<RulesCandidate> = {}): RulesCandidate {
    return {
        rulesId,
        label: rulesId.replace(/^.*\//, "").replace(/\.csv\.rules$/, ""),
        family: "citi",
        familySize: 2,
        year: 2026,
        // txnFixture's funding posting, written second.
        account1: "liabilities:visa",
        reasons: ["Books to liabilities:visa"],
        recommended: false,
        matchField: "description",
        fields: ["date", "description", "amount"],
        ...extra,
    };
}

/** citi-2026 recommended; citi-2025 (no `fields`, so whole-record matching) the alternative. */
const DECIDED = (index: number): RulesFor => ({
    index,
    needsChoice: false,
    candidates: [
        candidate(CITI_2026, {recommended: true, reasons: ["Books to liabilities:visa", "Named for 2026, this transaction's year"]}),
        candidate(CITI_2025, {year: 2025, matchField: null, fields: null, reasons: ["Books to liabilities:visa", "Named for 2025"]}),
    ],
});

/** Two families for one account: the user must choose. */
const TIE = (index: number): RulesFor => ({
    index,
    needsChoice: true,
    candidates: [
        candidate(AMEX, {family: "amex", familySize: 1, year: null, reasons: ["Another rules file for this account fits as well"]}),
        candidate(AMEX_BUSINESS, {family: "amex-business", familySize: 1, year: null}),
    ],
});

const OAKLAND = txnFixture(1, "SQ *BLUE BOTTLE 4412 OAKLAND", "expenses:unknown", "2026-02-10");
const AMEX_STORE = txnFixture(2, "AMEX STORE", "expenses:unknown", "2026-02-12");

interface Harness {
    order: string[];
    seen: SeenRequest[];
    io: JournalRuleIo & {preview: ReturnType<typeof vi.fn>; save: ReturnType<typeof vi.fn>};
    choiceFor: (found: RulesFor) => RuleChoice | null;
}

/** An Unknowns run over the two cards; each batch records "card" and lands as written. */
function serve(options: {ruleSave?: EditResult} = {}): Harness {
    const order: string[] = [];
    const seen = serveReview({
        queue: () => queuePageWire("unknowns", [queueItemWire(OAKLAND, {key: "blue bottle oakland"}), queueItemWire(AMEX_STORE, {key: "amex store"})]),
        batch: (body) => {
            order.push("card");
            return {generation: "g2", edited: body.edits.map(({index}) => reviewTxnWire(index === 1 ? OAKLAND : AMEX_STORE, {fingerprint: `fp-${index}-1`}))};
        },
    });
    const io = {
        preview: vi.fn(async () => ({ok: true as const, basis: "journal" as const, matched: 3, total: 9})),
        save: vi.fn(async (): Promise<EditResult> => (order.push("rule"), options.ruleSave ?? {ok: true})),
    };
    return {order, seen, io, choiceFor: (found) => ruleChoice(found, io)};
}

const toggle = (): HTMLInputElement => screen.getByTestId("review-rule-toggle") as HTMLInputElement;
const fileSelect = (): HTMLSelectElement => screen.getByTestId("review-rule-file-select") as HTMLSelectElement;
const pattern = (): HTMLInputElement => screen.getByTestId("review-rule-pattern") as HTMLInputElement;
const text = (testid: string): string => (screen.getByTestId(testid).textContent ?? "").replace(/\s+/g, " ").trim();
const batches = (seen: SeenRequest[]): number => seen.filter((request) => request.url.pathname.endsWith("/api/transactions/batch")).length;

/** Recategorise the card on screen, which is what offers the rule. */
async function recategorise(account = "expenses:food:coffee"): Promise<void> {
    await fireEvent.input(accountField(), {target: {value: account}});
}

beforeEach(async () => {
    resetReview();
    resetReviewLaunch();
    load.mockReset();
    await connectFakeEngine();
});
afterEach(() => {
    vi.unstubAllGlobals();
    keymap.reset();
});

describe("COMPONENT ReviewOverlay — Remember as a rule on a journal card (D27)", () => {
    it("is offered once the card changed, when the engine found rules files for it", async () => {
        const {choiceFor} = serve();
        load.mockImplementation(async (index) => choiceFor(DECIDED(index)));
        await mountOverlay();
        expect(load).toHaveBeenCalledWith(1, expect.any(AbortSignal));
        expect(screen.queryByTestId("review-rule")).toBeNull();
        await recategorise();
        expect(toggle().checked).toBe(false);
        card().focus();
        await press("R");
        expect(toggle().checked).toBe(true);
        expect(JSON.stringify(keymap.help)).toContain("Remember as a rule");
    });

    it("is not offered when no rules file books to the transaction's account", async () => {
        serve();
        load.mockResolvedValue(null);
        await mountOverlay();
        await recategorise();
        expect(screen.queryByTestId("review-rule")).toBeNull();
        card().focus();
        await press("R");
        expect(screen.queryByTestId("review-rule")).toBeNull();
        expect(JSON.stringify(keymap.help)).not.toContain("Remember as a rule");
    });

    it("is not offered when the request fails", async () => {
        serve();
        load.mockRejectedValue(new Error("engine unavailable"));
        await mountOverlay();
        await recategorise();
        expect(screen.queryByTestId("review-rule")).toBeNull();
    });

    it("preselects the recommended file and says why", async () => {
        const {choiceFor} = serve();
        load.mockImplementation(async (index) => choiceFor(DECIDED(index)));
        await mountOverlay();
        await recategorise();
        await fireEvent.click(toggle());
        expect(fileSelect().value).toBe(CITI_2026);
        expect(text("review-rule-file")).toContain("Saving to");
        expect(text("review-rule-file-reason")).toBe("(newest of 2 for citi)");
        expect([...fileSelect().options].map((option) => option.value)).toEqual([CITI_2026, CITI_2025]);
        expect(pattern().disabled).toBe(false);
        expect(pattern().value).toBe("blue.*bottle.*oakland");
        await fireEvent.click(screen.getByTestId("review-rule-file-why"));
        expect(text("review-rule-file-reasons")).toContain("Named for 2025");
        // The live check runs on the picked file and shows the journal fallback.
        await vi.waitFor(() =>
            expect(text("review-rule-count")).toBe("Matches 3 of 9 transactions from liabilities:visa (no statement file found to test against)")
        );
    });

    it("starts the rule again from the newly picked file, keeping focus on the picker", async () => {
        const {choiceFor, io} = serve();
        load.mockImplementation(async (index) => choiceFor(DECIDED(index)));
        await mountOverlay();
        await recategorise();
        await fireEvent.click(toggle());
        await fireEvent.input(pattern(), {target: {value: "blue"}});
        expect((screen.getByTestId("review-rule-field") as HTMLSelectElement).value).toBe("description");
        const select = fileSelect();
        select.focus();
        await fireEvent.change(select, {target: {value: CITI_2025}});
        expect(fileSelect()).toBe(select);
        expect(document.activeElement).toBe(select);
        // citi-2025 names no `fields`: the rule matches the whole record, prefilled afresh.
        expect((screen.getByTestId("review-rule-field") as HTMLSelectElement).value).toBe("");
        expect(pattern().value).toBe("blue.*bottle.*oakland");
        expect(text("review-rule")).toContain("Added to the end of citi-2025.csv.rules");
        await vi.waitFor(() => expect(io.preview).toHaveBeenCalledWith(1, CITI_2025, expect.anything(), expect.any(AbortSignal)));
    });

    it("saves the rule to the picked file first, then the card — setting the slot the rules file writes the expense to", async () => {
        const {choiceFor, io, order} = serve();
        load.mockImplementation(async (index) => choiceFor(DECIDED(index)));
        await mountOverlay();
        await recategorise();
        await fireEvent.click(toggle());
        await press("Enter", MOD);
        await vi.waitFor(() => expect(order).toEqual(["rule", "card"]));
        expect(io.save).toHaveBeenCalledWith(CITI_2026, {
            matchers: [{field: "description", pattern: "blue.*bottle.*oakland"}],
            // The card lists the expense first; the rules file writes its account1 (liabilities:visa) first.
            assignments: [{field: "account2", value: "expenses:food:coffee"}],
        });
        expect(screen.queryByTestId("review-import-notice")).toBeNull();
    });

    it("leaves the card, edits and all, when the rule can't be saved", async () => {
        const {choiceFor, order, seen} = serve({ruleSave: {ok: false, failure: {kind: "conflict", message: "the rules file changed on disk"}}});
        load.mockImplementation(async (index) => choiceFor(DECIDED(index)));
        await mountOverlay();
        await recategorise();
        await fireEvent.click(toggle());
        await fireEvent.click(primaryButton());
        await vi.waitFor(() => expect(text("review-rule-save-error")).toContain("the rules file changed on disk"));
        expect(order).toEqual(["rule"]);
        expect(batches(seen)).toBe(0);
        expect(review.current?.item.txn.index).toBe(1);
        expect(accountField().value).toBe("expenses:food:coffee");
    });
});

describe("COMPONENT ReviewOverlay — when the user must choose the rules file (D27)", () => {
    async function openTie(): Promise<Harness> {
        const harness = serve();
        load.mockImplementation(async (index) => harness.choiceFor(TIE(index)));
        await mountOverlay();
        await recategorise();
        return harness;
    }

    it("selects nothing, and locks the rule until a file is picked", async () => {
        await openTie();
        await fireEvent.click(toggle());
        expect(fileSelect().value).toBe("");
        expect(screen.getByTestId("review-rule-file").dataset.needsChoice).toBe("");
        expect(text("review-rule-file")).toContain("Choose a rules file");
        expect(screen.queryByTestId("review-rule-file-reason")).toBeNull();
        expect(pattern().disabled).toBe(true);
        expect(text("review-rule-unpicked")).toContain("Choose a rules file first");
    });

    it("puts focus on the picker when R turns the rule on", async () => {
        await openTie();
        card().focus();
        await press("R");
        expect(toggle().checked).toBe(true);
        expect(document.activeElement).toBe(fileSelect());
    });

    it("refuses to save the card with the rule on until a file is picked, then saves both", async () => {
        const {io, order, seen} = await openTie();
        await fireEvent.click(toggle());
        await press("Enter", MOD);
        expect(text("review-rule-save-error")).toContain("choose a rules file for it first");
        expect(io.save).not.toHaveBeenCalled();
        expect(batches(seen)).toBe(0);
        expect(review.current?.item.txn.index).toBe(1);

        await fireEvent.change(fileSelect(), {target: {value: AMEX_BUSINESS}});
        expect(screen.queryByTestId("review-rule-save-error")).toBeNull();
        expect(pattern().disabled).toBe(false);
        expect(text("review-rule-file")).toContain("Saving to");
        await press("Enter", MOD);
        await vi.waitFor(() => expect(order).toEqual(["rule", "card"]));
        expect(io.save).toHaveBeenCalledWith(AMEX_BUSINESS, expect.anything());
    });

    it("saves the card alone once the rule is turned off", async () => {
        const {io, order} = await openTie();
        await fireEvent.click(toggle());
        await press("Enter", MOD);
        expect(screen.getByTestId("review-rule-save-error")).toBeTruthy();
        await fireEvent.click(toggle());
        await press("Enter", MOD);
        await vi.waitFor(() => expect(order).toEqual(["card"]));
        expect(io.save).not.toHaveBeenCalled();
    });
});

describe("COMPONENT ReviewOverlay — loading a card's rules files (D27)", () => {
    it("aborts the load when the card leaves first, and asks again on coming back", async () => {
        const {choiceFor} = serve();
        const pending = deferred<RuleChoice | null>();
        const signals = new Map<number, AbortSignal[]>();
        load.mockImplementation((index, signal) => {
            signals.set(index, [...(signals.get(index) ?? []), signal as AbortSignal]);
            return index === 1 && signals.get(1)?.length === 1 ? pending.promise : Promise.resolve(choiceFor(DECIDED(index)));
        });
        await mountOverlay();
        card().focus();
        await press("n");
        expect(signals.get(1)?.[0]?.aborted).toBe(true);
        expect(signals.get(2)).toHaveLength(1);
        // The late answer for the card that left is dropped.
        pending.resolve(null);
        await press("ArrowLeft");
        expect(review.current?.item.txn.index).toBe(1);
        expect(signals.get(1)).toHaveLength(2);
        await recategorise();
        await vi.waitFor(() => expect(screen.getByTestId("review-rule")).toBeTruthy());
    });

    it("asks once per card for the run", async () => {
        const {choiceFor} = serve();
        load.mockImplementation(async (index) => choiceFor(DECIDED(index)));
        await mountOverlay();
        card().focus();
        await press("n");
        await press("ArrowLeft");
        await press("n");
        expect(load.mock.calls.map(([index]) => index)).toEqual([1, 2]);
    });
});
