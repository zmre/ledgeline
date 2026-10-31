// The review overlay AFTER ACTIONS (plan 32 Phase 3): what a save sends, the
// one-keystroke suggestion path, the stack changing as you work (D5a), the
// run's summary, failures, and motion (D29). At-rest behaviour is
// `ReviewOverlay.svelte.test.ts`.

import {fireEvent, screen} from "@testing-library/svelte";
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import {keymap} from "#lib/keys/keymap.svelte.js";
import {localToday} from "#lib/stores/filters.svelte.js";
import {resetReview, review} from "#lib/stores/review.svelte.js";
import {connectFakeEngine} from "#lib/testing/fakeEngine.js";
import {deferred, queueItemWire, queuePageWire, reviewTxnWire, serveReview, similarWire, txnFixture} from "#lib/testing/reviewFixture.js";
import {accountField, card, MOD, mountOverlay, press, settle, suggestionWire} from "#lib/testing/reviewOverlay.js";
import type {QueueKind} from "../types";
import {resetReviewLaunch, reviewLaunch} from "./launch.svelte";

const TODAY = localToday();
const REVIEWED = {op: "setTxnTag", name: "reviewed", value: TODAY};
const TO_COFFEE = {op: "setPostingAccount", posting: 0, account: "expenses:food:coffee"};
const blue = (index: number, account = "expenses:unknown") => txnFixture(index, `SQ *BLUE BOTTLE ${index} OAKLAND`, account, "2026-10-12");
type Edit = {index: number; fingerprint: string; ops: Record<string, unknown>[]};

/** A transaction as an edit left it: the ops' account and description applied. */
function edited(ops: Record<string, unknown>[], before: {account: string; description: string; reviewed: boolean}) {
    const account = (ops.find((op) => op.op === "setPostingAccount")?.account as string | undefined) ?? before.account;
    const description = (ops.find((op) => op.op === "setDescription")?.description as string | undefined) ?? before.description;
    return {account, description, reviewed: before.reviewed || ops.some((op) => op.name === "reviewed")};
}

/** The batch's 200 for edits over untouched fixtures. */
function landed(edits: Edit[], generation = "g2") {
    return {
        generation,
        edited: edits.map(({index, ops}) => {
            const now = edited(ops, {account: "expenses:unknown", description: `SQ *BLUE BOTTLE ${index} OAKLAND`, reviewed: false});
            return reviewTxnWire(txnFixture(index, now.description, now.account, "2026-10-12"), {
                unknown: now.account === "expenses:unknown",
                reviewed: now.reviewed,
                fingerprint: `fp-${index}-1`,
            });
        }),
    };
}

/**
 * A small engine over Blue Bottle transactions: the queue is computed from
 * what the batches wrote (Unknowns: still unknown; Recents: not reviewed),
 * less any index in `deleted` once the first save has landed.
 */
function serve(
    kind: QueueKind,
    queued: number[],
    options: {suggestions?: Record<string, unknown>[]; similar?: number[]; inQueue?: number[]; deleted?: number[]} = {}
) {
    const held = new Map<number, {account: string; description: string; reviewed: boolean; version: number}>();
    for (const index of [...queued, ...(options.similar ?? [])])
        held.set(index, {account: "expenses:unknown", description: `SQ *BLUE BOTTLE ${index} OAKLAND`, reviewed: false, version: 0});
    let generation = 1;
    const wire = (index: number) => {
        const now = held.get(index)!;
        return {
            txn: txnFixture(index, now.description, now.account, "2026-10-12"),
            marks: {unknown: now.account === "expenses:unknown", reviewed: now.reviewed, fingerprint: `fp-${index}-${now.version}`},
        };
    };
    const batches: Edit[][] = [];
    const seen = serveReview({
        queue: () => {
            const gone = generation > 1 ? new Set(options.deleted ?? []) : new Set<number>();
            const indices = queued.filter((index) => !gone.has(index) && (kind === "unknowns" ? wire(index).marks.unknown : !wire(index).marks.reviewed));
            return queuePageWire(
                kind,
                indices.map((index) => {
                    const {txn, marks} = wire(index);
                    return queueItemWire(txn, {...marks, suggestions: index === queued[0] ? (options.suggestions ?? []) : []});
                }),
                {generation: `g${generation}`}
            );
        },
        // Answered at the run's generation, which is what makes the store keep it.
        similar: (index) =>
            similarWire(index, index === queued[0] ? (options.similar ?? []).map((sibling) => reviewTxnWire(wire(sibling).txn, wire(sibling).marks)) : [], {
                inQueue: options.inQueue ?? [],
                generation: `g${generation}`,
            }),
        batch: (body) => {
            batches.push(body.edits as Edit[]);
            generation += 1;
            for (const {index, ops} of body.edits as Edit[]) {
                const before = held.get(index)!;
                held.set(index, {...edited(ops, before), version: before.version + 1});
            }
            return {generation: `g${generation}`, edited: (body.edits as Edit[]).map(({index}) => reviewTxnWire(wire(index).txn, wire(index).marks))};
        },
    });
    return {seen, batches};
}

async function similarReady(): Promise<void> {
    await vi.waitFor(() => expect(screen.getByTestId("review-similar")).toBeTruthy());
}

beforeEach(async () => {
    resetReview();
    resetReviewLaunch();
    await connectFakeEngine();
});
afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    keymap.reset();
});

describe("COMPONENT ReviewOverlay saving", () => {
    it("sends one batch: the card with the reviewed tag, the checked candidates without it — from inside a field with ⌘↵", async () => {
        const {batches} = serve("unknowns", [1, 2], {similar: [11, 12]});
        await mountOverlay();
        await similarReady();
        await fireEvent.input(accountField(), {target: {value: "expenses:food:coffee"}});
        card().focus();
        await press("s");
        await fireEvent.click(screen.getAllByTestId("review-similar-row")[1]?.querySelector("input") as HTMLInputElement);
        accountField().focus();
        await press("Enter", MOD);
        // Optimistic: the next card is up before the engine answers.
        expect(review.current?.item.txn.index).toBe(2);
        await review.settled();
        expect(batches).toEqual([
            [
                {index: 1, fingerprint: "fp-1-0", ops: [TO_COFFEE, REVIEWED]},
                {index: 12, fingerprint: "fp-12-0", ops: [TO_COFFEE]},
            ],
        ]);
    });

    it("⌘⇧↵ accepts the selected chip, marks reviewed, saves and advances — from inside a field too", async () => {
        const {batches} = serve("unknowns", [1, 2], {suggestions: [suggestionWire("expenses:food:coffee", true, {description: "Blue Bottle Coffee"})]});
        await mountOverlay();
        (screen.getByLabelText("Description") as HTMLInputElement).focus();
        await press("Enter", {...MOD, shiftKey: true});
        expect(review.current?.item.txn.index).toBe(2);
        await review.settled();
        expect(batches[0]?.[0]?.ops).toEqual([
            {op: "setDescription", description: "Blue Bottle Coffee"},
            TO_COFFEE,
            {op: "setTxnTag", name: "payee-key", value: "payee"},
            REVIEWED,
        ]);
    });

    it("Y in card mode does what ⌘⇧↵ does, and y what ⌘↵ does", async () => {
        const {batches} = serve("unknowns", [1, 2, 3], {suggestions: [suggestionWire("expenses:food:coffee")]});
        await mountOverlay();
        await press("Y");
        expect(review.cursor).toBe(1);
        await review.settled();
        expect(batches[0]?.[0]?.ops).toEqual([TO_COFFEE, REVIEWED]);
        // Card 2 has no suggestion and is unknown: y is the disabled Reviewed.
        await press("y");
        expect(review.cursor).toBe(1);
    });

    it("with no chip, ⌘⇧↵ is ⌘↵", async () => {
        const seen = serveReview({
            queue: () => queuePageWire("recents", [queueItemWire(txnFixture(5, "Safeway", "expenses:food:groceries"), {unknown: false})]),
            batch: (body) => landed(body.edits as Edit[]),
        });
        await mountOverlay("recents");
        await press("Enter", {...MOD, shiftKey: true});
        await review.settled();
        const sent = seen.find((request) => request.url.pathname.endsWith("/batch"))?.body as {edits: Edit[]};
        expect(sent.edits).toEqual([{index: 5, fingerprint: "fp-5-0", ops: [REVIEWED]}]);
    });

    it("shows a saving… dot while a write is in flight", async () => {
        const hold = deferred<unknown>();
        serveReview({
            queue: () =>
                queuePageWire("recents", [queueItemWire(txnFixture(5, "Safeway", "expenses:food:groceries")), queueItemWire(txnFixture(6, "Safeway"))]),
            batch: () => hold.promise,
        });
        await mountOverlay("recents");
        await press("y");
        expect(screen.getByTestId("review-saving").textContent).toContain("saving");
        hold.resolve({generation: "g2", edited: [reviewTxnWire(txnFixture(5, "Safeway", "expenses:food:groceries"), {unknown: false, reviewed: true})]});
        await review.settled();
        await settle();
        expect(screen.queryByTestId("review-saving")).toBeNull();
    });

    it("a failed save rewinds to its card with the edits and the error", async () => {
        serveReview({
            queue: () => queuePageWire("unknowns", [queueItemWire(blue(1)), queueItemWire(blue(2))]),
            batch: () => new Response("transaction #1 changed on disk", {status: 409}),
        });
        await mountOverlay();
        await fireEvent.input(accountField(), {target: {value: "expenses:food:coffee"}});
        card().focus();
        await press("y");
        await review.settled();
        await settle();
        expect(review.current?.item.txn.index).toBe(1);
        expect(accountField().value).toBe("expenses:food:coffee");
        expect(screen.getByTestId("review-failure").textContent).toContain("Not saved");
    });

    it("i marks the card reviewed:ignore and advances", async () => {
        const {batches} = serve("unknowns", [1, 2]);
        await mountOverlay();
        await press("i");
        expect(review.cursor).toBe(1);
        expect(screen.getByTestId("review-stack").dataset.exit).toBe("ignore");
        await review.settled();
        expect(batches[0]?.[0]?.ops).toEqual([{op: "setTxnTag", name: "reviewed", value: "ignore"}]);
    });
});

describe("COMPONENT ReviewOverlay dynamic stack (D5a)", () => {
    it("Unknowns: applying to 9 queued siblings ends the run on '1 reviewed · 9 fixed via similar'; ↵ closes onto the last reviewed", async () => {
        const siblings = [2, 3, 4, 5, 6, 7, 8, 9, 10];
        serve("unknowns", [1, ...siblings], {similar: siblings, inQueue: siblings});
        reviewLaunch.open("unknowns");
        await mountOverlay();
        expect(screen.getByTestId("review-position").textContent?.replace(/\s+/g, " ").trim()).toBe("1 of 10");
        await similarReady();
        await fireEvent.input(accountField(), {target: {value: "expenses:food:coffee"}});
        card().focus();
        await press("s");
        await press("A");
        card().focus();
        await press("Enter", MOD);
        await review.settled();
        await settle();
        expect(screen.getByTestId("review-tally").textContent).toBe("1 reviewed · 9 fixed via similar · 0 skipped");
        await press("Enter");
        expect(reviewLaunch.kind).toBeNull();
        expect(reviewLaunch.returnTo?.txnIndex).toBe(1);
    });

    it("Recents: touched siblings stay, showing the change and 'updated from card 1'; the total recomputes", async () => {
        serve("recents", [1, 2, 3], {similar: [2, 3], inQueue: [2, 3], deleted: [3]});
        await mountOverlay("recents");
        await similarReady();
        await fireEvent.input(screen.getByLabelText("Description"), {target: {value: "Blue Bottle Coffee"}});
        card().focus();
        await press("s");
        await press("A");
        card().focus();
        await press("Enter", MOD);
        await review.settled();
        await vi.waitFor(() => expect(review.count).toBe(2));
        await settle();
        // The background revalidation dropped card 3, deleted elsewhere.
        expect(review.cards.map((held) => held.item.txn.index)).toEqual([1, 2]);
        expect(screen.getByTestId("review-position").textContent?.replace(/\s+/g, " ").trim()).toBe("2 of 2");
        expect(screen.getByTestId("review-payee").textContent).toContain("Blue Bottle Coffee");
        // …and the form follows the refreshed text: nothing is pending on it
        // (it is still uncategorised — only the rename was applied).
        expect((screen.getByLabelText("Description") as HTMLInputElement).value).toBe("Blue Bottle Coffee");
        expect(screen.getByTestId("review-footer").dataset.state).toBe("clean-unknown");
        expect(screen.getByTestId("review-updated-from").textContent).toContain("updated from card 1");
    });

    it("shows the run's tally with skips", async () => {
        serve("unknowns", [1, 2]);
        await mountOverlay();
        await press("n");
        await press("n");
        expect(screen.getByTestId("review-tally").textContent).toBe("0 reviewed · 0 fixed via similar · 2 skipped");
        // The summary is what lay under the last card: the deck holds nothing else now.
        expect(screen.queryByTestId("review-card")).toBeNull();
        expect(screen.queryAllByTestId("review-peek")).toHaveLength(0);
        expect(document.activeElement).toBe(screen.getByTestId("review-summary"));
    });
});

describe("COMPONENT ReviewOverlay motion (D29)", () => {
    it("marks each moment's exit: skip, back, and a stamped Reviewed", async () => {
        serveReview({
            queue: () =>
                queuePageWire(
                    "recents",
                    [1, 2, 3].map((index) => queueItemWire(txnFixture(index, `Shop ${index}`, "expenses:food:groceries"), {unknown: false}))
                ),
            batch: (body) => landed(body.edits as Edit[]),
        });
        await mountOverlay("recents");
        const stack = screen.getByTestId("review-stack");
        await press("n");
        expect(stack.dataset.exit).toBe("skip");
        await press("ArrowLeft");
        expect(stack.dataset.exit).toBe("back");
        const stamps: Element[] = [];
        const observer = new MutationObserver((mutations) => {
            for (const mutation of mutations)
                for (const node of mutation.addedNodes) if (node instanceof Element && node.matches("[data-testid='review-stamp']")) stamps.push(node);
        });
        observer.observe(stack, {subtree: true, childList: true});
        await press("y");
        observer.disconnect();
        expect(stack.dataset.exit).toBe("reviewed");
        expect(stamps).toHaveLength(1);
    });

    it("a keypress mid-animation finishes it, and acts on the next card at once", async () => {
        serve("unknowns", [1, 2, 3]);
        await mountOverlay();
        const finish = vi.fn();
        vi.spyOn(Element.prototype, "getAnimations").mockReturnValue([{finish} as unknown as Animation]);
        await press("n");
        expect(finish).toHaveBeenCalled();
        expect(review.cursor).toBe(1);
        expect(card().dataset.cardIndex).toBe("2");
    });

    it("swaps the motion for a fade under prefers-reduced-motion, the stamp a border flash", async () => {
        vi.spyOn(window, "matchMedia").mockImplementation((query: string) => ({matches: query.includes("reduce"), media: query}) as MediaQueryList);
        serveReview({
            queue: () =>
                queuePageWire(
                    "recents",
                    [1, 2].map((index) => queueItemWire(txnFixture(index, `Shop ${index}`, "expenses:food:groceries"), {unknown: false}))
                ),
            batch: (body) => landed(body.edits as Edit[]),
        });
        await mountOverlay("recents");
        expect(screen.getByTestId("review-stack").dataset.motion).toBe("reduced");
        // The pile still shows, sitting still: no transition between slots.
        const peek = screen.getByTestId("review-peek").getAttribute("style") ?? "";
        expect(peek).toContain("transform: translateY(14px) scale(0.95)");
        expect(peek).not.toContain("transition");
        const flashed: string[] = [];
        const observer = new MutationObserver((mutations) => {
            for (const mutation of mutations)
                if (mutation.target instanceof HTMLElement && mutation.target.matches("[data-review-card]")) flashed.push(mutation.target.className);
        });
        observer.observe(screen.getByTestId("review-stack"), {subtree: true, attributes: true, attributeFilter: ["class"]});
        await press("y");
        observer.disconnect();
        expect(flashed.some((className) => className.includes("border-success"))).toBe(true);
    });
});
