// The review overlay while an EARLIER card's save lands underneath the one on
// screen: the run moves to a new generation, and every similar set it holds
// is due a refetch. The card on screen must not notice — its Similar section
// stays mounted, open or closed as the user left it, with focus where it was.
//
// This is the e2e flake in `review.e2e.ts` step 3 (2026-11-23): ⌘⇧↵ saved
// card 23 and `s` on card 22 ran ~50 ms later, as the save landed. The store
// dropped every similar set on the new generation, so card 22's rows went to
// [] until the refetch came back: the section unmounted (focus fell to
// <body>), and an `s` in that gap found no rows and did nothing.

import {fireEvent, screen} from "@testing-library/svelte";
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import {keymap} from "#lib/keys/keymap.svelte.js";
import {resetReview, review} from "#lib/stores/review.svelte.js";
import {connectFakeEngine} from "#lib/testing/fakeEngine.js";
import {deferred, queueItemWire, queuePageWire, reviewTxnWire, serveReview, similarWire, txnFixture} from "#lib/testing/reviewFixture.js";
import {accountField, card, MOD, mountOverlay, press, settle} from "#lib/testing/reviewOverlay.js";
import {resetReviewLaunch} from "./launch.svelte";

const blue = (index: number, account = "expenses:unknown") => txnFixture(index, `SQ *BLUE BOTTLE ${index} OAKLAND`, account, "2026-10-12");
type Edit = {index: number; fingerprint: string; ops: Record<string, unknown>[]};

/**
 * Two queued cards. Card 1's Similar holds #11; card 2's holds #12 and #11.
 * The batch answers only when the test lets it, and so does every similar
 * request made after it — the gap the e2e fell into, held open.
 */
function serve() {
    let generation = 1;
    const moved = new Set<number>();
    const wire = (index: number) =>
        reviewTxnWire(blue(index, moved.has(index) ? "expenses:food:coffee" : "expenses:unknown"), {
            unknown: !moved.has(index),
            fingerprint: `fp-${index}-${moved.has(index) ? 1 : 0}`,
        });
    const batch = deferred<void>();
    const refetch = deferred<void>();
    const seen = serveReview({
        queue: () =>
            queuePageWire(
                "unknowns",
                [1, 2].filter((index) => !moved.has(index)).map((index) => queueItemWire(blue(index))),
                {generation: `g${generation}`}
            ),
        similar: async (index) => {
            const at = generation;
            if (at > 1) await refetch.promise;
            const siblings = (index === 1 ? [11] : [12, 11]).filter((sibling) => !moved.has(sibling));
            return similarWire(index, siblings.map(wire), {generation: `g${at}`});
        },
        batch: async (body) => {
            await batch.promise;
            generation += 1;
            for (const {index} of body.edits as Edit[]) moved.add(index);
            return {generation: `g${generation}`, edited: (body.edits as Edit[]).map(({index}) => wire(index))};
        },
    });
    return {seen, batch, refetch};
}

const similar = (): HTMLElement => screen.getByTestId("review-similar");
const rowLabels = (): string[] => screen.getAllByTestId("review-similar-row").map((row) => row.dataset.txn ?? "");

/** Card 1: move it to coffee, check #11, ⌘↵ — the batch is held, card 2 is up. */
async function saveFirstCard(): Promise<void> {
    await vi.waitFor(() => expect(similar()).toBeTruthy());
    await fireEvent.input(accountField(), {target: {value: "expenses:food:coffee"}});
    card().focus();
    await press("s");
    await fireEvent.click(screen.getAllByTestId("review-similar-row")[0]?.querySelector("input") as HTMLInputElement);
    accountField().focus();
    await press("Enter", MOD);
    expect(review.current?.item.txn.index).toBe(2);
    // Card 2's set was prefetched with card 1's (D12).
    await vi.waitFor(() => expect(rowLabels()).toEqual(["12", "11"]));
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

describe("COMPONENT ReviewOverlay when an earlier save lands", () => {
    it("keeps the card on screen's Similar open, with focus on its row, while the sets are refetched", async () => {
        const {seen, batch, refetch} = serve();
        await mountOverlay();
        await saveFirstCard();
        await fireEvent.input(accountField(), {target: {value: "expenses:food:coffee"}});
        card().focus();
        await press("s");
        expect(similar().dataset.open).toBe("");
        const row = document.activeElement;
        expect(row?.getAttribute("aria-label")).toContain("BLUE BOTTLE 12");

        // Card 1's save lands: a new generation, every held set is stale.
        batch.resolve();
        await review.settled();
        await settle();
        // The refetch is on its way and held: what shows is the set as it
        // was, less #11 — the save just moved it, so its fingerprint is gone.
        expect(review.generation).toBe("g2");
        await vi.waitFor(() => expect(seen.filter((request) => request.url.pathname.endsWith("/similar/2"))).toHaveLength(2));
        expect(similar().dataset.open).toBe("");
        expect(rowLabels()).toEqual(["12"]);
        expect(document.activeElement).toBe(row);

        refetch.resolve();
        await vi.waitFor(() => expect(review.similar(2)?.generation).toBe("g2"));
        await settle();
        expect(similar().dataset.open).toBe("");
        expect(rowLabels()).toEqual(["12"]);
        expect(document.activeElement).toBe(row);
        // ⇧A reaches the section that is on screen.
        card().focus();
        await press("A");
        expect((screen.getByTestId("review-similar-all") as HTMLInputElement).checked).toBe(true);
    });

    it("opens Similar on `s` pressed while the sets are being refetched", async () => {
        const {batch, refetch} = serve();
        await mountOverlay();
        await saveFirstCard();
        batch.resolve();
        await review.settled();
        await settle();
        expect(review.generation).toBe("g2");

        await fireEvent.input(accountField(), {target: {value: "expenses:food:coffee"}});
        card().focus();
        await press("s");
        expect(similar().dataset.open).toBe("");
        expect(document.activeElement?.closest("[data-similar-rows]")).not.toBeNull();

        refetch.resolve();
        await vi.waitFor(() => expect(review.similar(2)?.generation).toBe("g2"));
        await settle();
        expect(similar().dataset.open).toBe("");
        expect(card().contains(document.activeElement)).toBe(true);
    });
});
