// The transaction editor's transfer line (plan 35 Amendment A2): ⇄ Find match
// on a transaction with an unlinked clearing posting (opens Find Matches
// focused on it; waits for a clean card), its links with Unlink (asks first,
// names every member of the link, then re-opens the editor), and nothing at
// all on an ordinary transaction. Read from the transaction as served: no
// match request until a button is pressed.

import {fireEvent, render, screen} from "@testing-library/svelte";
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import type {Transaction} from "#lib/domain/types.js";
import {txnModal} from "#lib/journal/edit/modalState.svelte.js";
import {keymap} from "#lib/keys/keymap.svelte.js";
import {editing} from "#lib/stores/editing.svelte.js";
import {matches, resetMatches} from "#lib/stores/matches.svelte.js";
import {resetReview, review} from "#lib/stores/review.svelte.js";
import {connectFakeEngine, FAKE_JOURNAL_INFO, transactionMatchesBody} from "#lib/testing/fakeEngine.js";
import {clearingHalf, matchHalfWire, transferHalf} from "#lib/testing/matchFixture.js";
import {queueItemWire, reviewTxnWire, similarWire, type SeenRequest} from "#lib/testing/reviewFixture.js";
import {settle} from "#lib/testing/reviewOverlay.js";
import {resetReviewLaunch} from "./launch.svelte";
import ReviewOverlay from "./ReviewOverlay.svelte";

vi.mock("$app/navigation", () => ({goto: vi.fn()}));
vi.mock("$app/paths", () => ({resolve: (path: string) => path}));

const OPEN = clearingHalf(4, "2026-11-02", "CAR PAYMENT", "assets:checking", -50655);
const LINKED = clearingHalf(4, "2026-11-01", "BILL.COM PAYABLES", "assets:checking", -224750, [["match", "g1bill"]]);
const BILL_A = clearingHalf(5, "2026-11-02", "ACME INV 12", "expenses:services", 200000, [["match", "g1bill"]]);
const BILL_B = clearingHalf(6, "2026-11-03", "BOLT INV 7", "expenses:software", 24750, [["match", "g1bill"]]);
const PLAIN = transferHalf(4, "2026-11-02", "Safeway", "expenses:food", 500);

let seen: SeenRequest[] = [];

function serve(txn: Transaction): void {
    seen = [];
    vi.stubGlobal("fetch", (input: RequestInfo | URL, init?: RequestInit) => {
        const url = new URL(String(input));
        const method = init?.method ?? "GET";
        seen.push({method, url, body: typeof init?.body === "string" ? (JSON.parse(init.body) as unknown) : undefined});
        const json = (value: unknown, status = 200) => Promise.resolve(new Response(JSON.stringify(value), {status}));
        const path = url.pathname;
        if (path.endsWith("/version")) return json("1.52");
        if (path.endsWith("/api/journal")) return json(FAKE_JOURNAL_INFO);
        if (path.endsWith("/api/diagnostics")) return json({count: 0, maxSeverity: null, diagnostics: []});
        if (path.endsWith("/api/payees")) return json({generation: "g1", payees: []});
        if (path.endsWith("/api/tags")) return json({generation: "g1", tags: []});
        if (/\/api\/review\/item\/\d+$/.test(path)) return json({generation: "g1", item: queueItemWire(txn, {unknown: false})});
        const similar = /\/api\/review\/similar\/(\d+)$/.exec(path);
        if (similar !== null) return json(similarWire(Number(similar[1]), []));
        if (path.endsWith("/api/matches")) return json({generation: "g1", clearing: true, counts: {clear: 0, check: 0}, older: 0, proposals: []});
        if (/\/api\/matches\/for\/\d+$/.test(path)) {
            const members = [LINKED, BILL_A, BILL_B].map((member) => matchHalfWire(member));
            return json(transactionMatchesBody(reviewTxnWire(txn, {unknown: false}), {links: [{matchId: "g1bill", broken: false, members}]}));
        }
        if (path.endsWith("/api/matches/unlink")) return json({generation: "g2", id: "g1bill", edited: []});
        return Promise.resolve(new Response(`no route for ${url}`, {status: 404}));
    });
}

async function openEdit(txn: Transaction): Promise<void> {
    serve(txn);
    await editing.probe();
    txnModal.openEdit(txn.index);
    render(ReviewOverlay, {props: {kind: "edit", index: txn.index}});
    await vi.waitFor(() => {
        if (review.loading || review.current === null) throw new Error("card not loaded");
    });
    await settle();
}

const matchRequests = () => seen.filter((request) => request.url.pathname.includes("/api/matches"));

beforeEach(async () => {
    resetReview();
    resetReviewLaunch();
    resetMatches();
    txnModal.close();
    editing.clearConflict();
    await connectFakeEngine();
});
afterEach(() => {
    vi.unstubAllGlobals();
    keymap.reset();
    resetMatches();
});

describe("COMPONENT ReviewOverlay editor: Find match (A2)", () => {
    it("offers Find match on an unlinked clearing posting without asking the engine, and opens it focused", async () => {
        await openEdit(OPEN);
        expect(screen.getByTestId("review-match-strip")).toBeDefined();
        expect(screen.queryByTestId("review-unlink")).toBeNull();
        expect(matchRequests()).toHaveLength(0);
        await fireEvent.click(screen.getByTestId("review-find-match"));
        expect(matches.open).toBe(true);
        expect(matches.focus?.index).toBe(4);
    });

    it("waits for a clean card", async () => {
        await openEdit(OPEN);
        const description = screen.getByLabelText("Description") as HTMLInputElement;
        await fireEvent.input(description, {target: {value: "CAR LOAN"}});
        const button = screen.getByTestId("review-find-match") as HTMLButtonElement;
        expect(button.disabled).toBe(true);
        expect(button.title).toContain("Save or revert");
    });

    it("shows a link with Unlink, which names every member, then re-opens the editor", async () => {
        await openEdit(LINKED);
        expect(screen.queryByTestId("review-find-match")).toBeNull();
        expect(screen.getByTestId("review-match-link").textContent).toContain("match:g1bill");
        await fireEvent.click(screen.getByTestId("review-unlink"));
        expect(matchRequests()).toHaveLength(0);
        const nonce = txnModal.nonce;
        await fireEvent.click(screen.getByTestId("review-unlink-go"));
        await vi.waitFor(() => expect(seen.some((request) => request.url.pathname.endsWith("/api/matches/unlink"))).toBe(true));
        expect(seen.find((request) => request.url.pathname.endsWith("/api/matches/unlink"))?.body).toEqual({
            matchId: "g1bill",
            halves: [
                {index: 4, posting: 1, fingerprint: "fp-4-0"},
                {index: 5, posting: 1, fingerprint: "fp-5-0"},
                {index: 6, posting: 1, fingerprint: "fp-6-0"},
            ],
        });
        await vi.waitFor(() => expect(txnModal.nonce).toBe(nonce + 1));
    });

    it("shows nothing on a transaction with no transfer half", async () => {
        await openEdit(PLAIN);
        expect(screen.queryByTestId("review-match-strip")).toBeNull();
    });
});
