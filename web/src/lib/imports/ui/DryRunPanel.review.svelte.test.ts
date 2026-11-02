// The dry run's "Review before writing" (plan 32 D26): Review Unknowns and
// Review All above Write changes, with their counts and `u`/`r`; what review
// has queued beside the write, flagged edits included; and the command line's
// note that the CLI imports without them.
//
// Props, as `DryRunPanel.svelte.test.ts` does: the counts and the edits come
// from the stores, which `stagedReview.test.ts` and `importStore.review.test.ts`
// cover. This is about what the panel shows of them and what its keys do.

import {fireEvent, render, screen} from "@testing-library/svelte";
import {afterEach, describe, expect, it, vi} from "vitest";
import {decodeCsvStagedTxn} from "#lib/api/importReviewDecode.js";
import {keymap} from "#lib/keys/keymap.svelte.js";
import {dryRun, ROTH_ALIAS} from "#lib/testing/aliasFixtures.js";
import {stagedTxnWire, txnFixture} from "#lib/testing/reviewFixture.js";
import type {DryRunResult} from "../importTypes";
import type {StagedOverride, StagedTxn} from "../reviewTypes";
import type {StagedReviewProps} from "../stagedReview";
import DryRunPanel from "./DryRunPanel.svelte";

const staged = (ordinal: number, description: string): StagedTxn =>
    decodeCsvStagedTxn(stagedTxnWire(txnFixture(ordinal + 1, description), {ordinal}), "staged");
const TXNS = [staged(0, "HARDWARE STORE"), staged(1, "PEETS")];

function run(overrides: Partial<Extract<DryRunResult, {ok: true}>> = {}): DryRunResult {
    const base = dryRun(null);
    if (!base.ok) throw new Error("unreachable");
    return {...base, txns: TXNS, ...overrides};
}

function reviewProps(
    overrides: Partial<StagedReviewProps> = {}
): StagedReviewProps & {onReview: ReturnType<typeof vi.fn>; onDiscard: ReturnType<typeof vi.fn>} {
    return {
        unknowns: 2,
        all: 2,
        overrideCount: 0,
        journalEditCount: 0,
        flagged: [],
        onReview: vi.fn(),
        onDiscard: vi.fn(),
        ...overrides,
    } as StagedReviewProps & {
        onReview: ReturnType<typeof vi.fn>;
        onDiscard: ReturnType<typeof vi.fn>;
    };
}

const mount = (result: DryRunResult, review: StagedReviewProps | null) =>
    render(DryRunPanel, {
        view: "data" as const,
        result,
        error: null,
        aliases: [ROTH_ALIAS],
        writing: false,
        editable: true,
        confWriting: false,
        confWritten: null,
        confError: null,
        onRetry: () => {},
        onWrite: () => {},
        onInstallConf: () => {},
        review,
    });

const press = (key: string): void => keymap.handle(new KeyboardEvent("keydown", {key, bubbles: true, cancelable: true}));
const button = (testid: string): HTMLButtonElement => screen.getByTestId(testid) as HTMLButtonElement;

afterEach(() => keymap.reset());

describe("COMPONENT DryRunPanel — review before writing", () => {
    it("offers Review Unknowns and Review All with their counts, above Write changes", () => {
        mount(run(), reviewProps({unknowns: 9, all: 112}));
        expect(screen.getByTestId("imports-review-unknowns-count").textContent).toBe("9");
        expect(screen.getByTestId("imports-review-all-count").textContent).toBe("112");
        const strip = screen.getByTestId("imports-review");
        // Above the write: the strip comes first in document order.
        expect(strip.compareDocumentPosition(button("imports-write-changes")) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
        // Optional: Write changes stays pressable with nothing reviewed (D26).
        expect(button("imports-write-changes").disabled).toBe(false);
    });

    it("wears the Journal toolbar's review style", () => {
        mount(run(), reviewProps());
        expect(button("imports-review-unknowns").className).toContain("bg-base-content/10");
        expect(button("imports-review-all").className).toContain("bg-base-content/10");
    });

    it("opens the run each button names", async () => {
        const review = reviewProps();
        mount(run(), review);
        await fireEvent.click(button("imports-review-unknowns"));
        await fireEvent.click(button("imports-review-all"));
        expect(review.onReview.mock.calls).toEqual([["unknowns"], ["all"]]);
    });

    it("binds u and r while the dry run is on screen, and lists them in the ? sheet", () => {
        const review = reviewProps();
        mount(run(), review);
        press("u");
        press("r");
        expect(review.onReview.mock.calls).toEqual([["unknowns"], ["all"]]);
        const help = JSON.stringify(keymap.help);
        expect(help).toContain("Review this import's unknowns");
        expect(help).toContain("Review all of this import");
    });

    it("disables a button, and its key, at zero", () => {
        const review = reviewProps({unknowns: 0});
        mount(run(), review);
        expect(button("imports-review-unknowns").disabled).toBe(true);
        expect(button("imports-review-all").disabled).toBe(false);
        press("u");
        expect(review.onReview).not.toHaveBeenCalled();
        expect(JSON.stringify(keymap.help)).not.toContain("Review this import's unknowns");
    });

    it("is absent, and so are its keys, when hledger refused the import", () => {
        const review = reviewProps();
        mount({ok: false, stderr: "hledger: bad"}, review);
        expect(screen.queryByTestId("imports-review")).toBeNull();
        press("u");
        expect(review.onReview).not.toHaveBeenCalled();
    });

    it("is absent when the import would write nothing to review", () => {
        mount(run({txns: []}), reviewProps());
        expect(screen.queryByTestId("imports-review")).toBeNull();
    });

    it("is absent when the host passes no review", () => {
        mount(run(), null);
        expect(screen.queryByTestId("imports-review")).toBeNull();
        expect(screen.getByTestId("imports-write-changes")).toBeDefined();
    });
});

describe("COMPONENT DryRunPanel — what review has queued", () => {
    it("says nothing when the write carries no review edits", () => {
        mount(run(), reviewProps());
        expect(screen.queryByTestId("imports-review-edits")).toBeNull();
    });

    it("counts the overrides and the journal edits beside Write changes", () => {
        mount(run(), reviewProps({overrideCount: 3, journalEditCount: 2}));
        expect(screen.getByTestId("imports-review-edits-count").textContent).toBe("3 reviewed edits · 2 existing transactions edited");
        expect(screen.queryByTestId("imports-review-flagged-toggle")).toBeNull();
    });

    it("lists flagged edits on request, and discards one or all of them", async () => {
        const flagged: StagedOverride[] = TXNS.map((txn) => ({
            ref: txn.ref,
            fingerprint: txn.fingerprint ?? "",
            ops: [{op: "setPostingAccount", posting: 0, account: "expenses:home"}],
            original: txn,
            flagged: true,
        }));
        const review = reviewProps({overrideCount: 2, flagged});
        mount(run(), review);
        expect(screen.getByTestId("imports-review-flagged-toggle").textContent?.replace(/\s+/g, " ").trim()).toBe("2 need another look");
        expect(screen.queryByTestId("imports-review-flagged")).toBeNull();

        await fireEvent.click(screen.getByTestId("imports-review-flagged-toggle"));
        const rows = screen.getAllByTestId("imports-review-flagged-row");
        expect(rows).toHaveLength(2);
        expect(rows[0]?.textContent).toContain("HARDWARE STORE");
        expect(rows[0]?.textContent).toContain("posting 1 → expenses:home");

        await fireEvent.click(screen.getByRole("button", {name: /Discard the edit to .* HARDWARE STORE/}));
        await fireEvent.click(screen.getByTestId("imports-review-discard-flagged"));
        expect(review.onDiscard.mock.calls).toEqual([
            [[{kind: "csv", ordinal: 0}]],
            [
                [
                    {kind: "csv", ordinal: 0},
                    {kind: "csv", ordinal: 1},
                ],
            ],
        ]);
    });
});

describe("COMPONENT DryRunPanel — the command line's note (Q8)", () => {
    it("says the command line imports without the review edits", () => {
        mount(run({cliNote: "plus 2 reviewed edits"}), reviewProps());
        expect(screen.getByTestId("imports-cli-note").textContent).toContain("plus 2 reviewed edits");
        // Under the command, which is still verbatim.
        expect(
            screen.getByTestId("imports-cli-command").compareDocumentPosition(screen.getByTestId("imports-cli-note")) & Node.DOCUMENT_POSITION_FOLLOWING
        ).toBeTruthy();
    });

    it("is absent without review edits", () => {
        mount(run(), reviewProps());
        expect(screen.queryByTestId("imports-cli-note")).toBeNull();
    });
});
