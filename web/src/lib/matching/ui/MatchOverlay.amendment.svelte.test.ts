// Find Matches after Amendment A (plan 35) against a fake engine: the
// combination card (one posting against a group: Link writes every member,
// no Not a match, Ignore this side is the single's, never in Link all), the
// older-than-the-lookback note with Show all, and the view FOCUSED on one
// transaction (`matches.openFor`): exact partners as radios, combinations,
// nearby postings with a running total that gates Link, the empty state,
// links with Unlink, and the widenable window.

import {fireEvent, render, screen} from "@testing-library/svelte";
import {tick} from "svelte";
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";

import {keymap} from "#lib/keys/keymap.svelte.js";
import {resetReviewLaunch} from "#lib/review/ui/launch.svelte.js";
import {editing} from "#lib/stores/editing.svelte.js";
import {journal} from "#lib/stores/journal.svelte.js";
import {matches, resetMatches} from "#lib/stores/matches.svelte.js";
import {settings} from "#lib/stores/settings.svelte.js";
import {connectFakeEngine, FAKE_ENGINE, journalInfoBody, transactionMatchesBody} from "#lib/testing/fakeEngine.js";
import {clearingHalf, combinationWire, matchesPageWire, matchHalfWire, proposalWire, transferHalf} from "#lib/testing/matchFixture.js";
import {reviewTxnWire, type SeenRequest} from "#lib/testing/reviewFixture.js";
import MatchOverlay from "./MatchOverlay.svelte";

// Bill.com (A4): one $2,247.50 debit paid out as a $2,000 bill and a $247.50 bill.
const DEBIT = clearingHalf(1, "2019-02-19", "BILL.COM PAYABLES", "assets:checking", -224750);
const BILL_A = clearingHalf(2, "2019-02-20", "ACME INV 12", "expenses:services", 200000);
const BILL_B = clearingHalf(3, "2019-02-21", "BOLT INV 7", "expenses:software", 24750);
const OTHER = clearingHalf(4, "2019-02-25", "CAFE INV 3", "expenses:food", 5000);
const EXACT = clearingHalf(5, "2019-02-22", "REFUND", "assets:savings", 224750);
const EXACT_FAR = clearingHalf(6, "2019-06-01", "LATE REFUND", "assets:savings", 224750);
const OUT = transferHalf(7, "2026-12-31", "ACH AMEX EPAYMENT", "assets:checking", -50000);
const IN = transferHalf(8, "2027-01-02", "PAYMENT THANK YOU", "liabilities:amex", 50000);

const COMBINATION = combinationWire(DEBIT, [BILL_A, BILL_B]);
const PAIR = proposalWire(OUT, IN);

let seen: SeenRequest[] = [];
let proposals: Record<string, unknown>[] = [];
let older = 0;
let forAnswer: Record<string, unknown> = {};
/** What `/for` answers once a write has landed. */
let forAfter: Record<string, unknown> | null = null;
let generation = 1;

const POSTS = ["link", "reject", "ignore", "unlink", "link-all"] as const;

const partner = (txn: typeof DEBIT, gapDays: number) => ({...matchHalfWire(txn), gapDays});
const forBody = (overrides: Record<string, unknown> = {}) => transactionMatchesBody(reviewTxnWire(DEBIT, {unknown: false}), overrides);

async function serve(initial: Record<string, unknown>[] = [], options: {older?: number} = {}): Promise<void> {
    await connectFakeEngine();
    seen = [];
    proposals = initial;
    older = options.older ?? 0;
    forAnswer = forBody();
    forAfter = null;
    generation = 1;
    vi.stubGlobal("fetch", (input: RequestInfo | URL, init?: RequestInit) => {
        const url = new URL(String(input));
        const method = init?.method ?? "GET";
        const body = typeof init?.body === "string" ? (JSON.parse(init.body) as Record<string, unknown>) : undefined;
        seen.push({method, url, body});
        const json = (value: unknown, status = 200) => Promise.resolve(new Response(JSON.stringify(value), {status}));
        const path = url.pathname;
        if (path.endsWith("/api/transactions")) return Promise.resolve(new Response("", {status: 405}));
        if (path.endsWith("/api/journal")) return json(journalInfoBody({generation: `g${generation}`}));
        if (path.endsWith("/api/diagnostics")) return json({count: 0, maxSeverity: null, diagnostics: []});
        if (path.endsWith("/api/matches")) {
            const all = url.searchParams.get("lookback") === "0";
            return json(matchesPageWire(proposals, {generation: `g${generation}`, older: all ? 0 : older}));
        }
        if (/\/api\/matches\/for\/\d+$/.test(path)) return json({...forAnswer, generation: `g${generation}`});
        const post = POSTS.find((name) => path.endsWith(`/api/matches/${name}`));
        if (post !== undefined && body !== undefined) {
            generation += 1;
            if (forAfter !== null) forAnswer = forAfter;
            if (post === "link-all") {
                const pairs = body.pairs as unknown[];
                proposals = proposals.filter((proposal) => proposal.clear !== true);
                return json({generation: `g${generation}`, linked: pairs.length, matchIds: pairs.map((_, at) => `id${at}`)});
            }
            proposals = [];
            return json({generation: `g${generation}`, id: "k7f3a2", edited: []});
        }
        return Promise.resolve(new Response(`no route for ${url}`, {status: 404}));
    });
    await editing.probe();
    await journal.refresh({force: true});
    await matches.sync(FAKE_ENGINE, journal.generation ?? "");
}

const posted = (name: (typeof POSTS)[number]) => seen.filter((request) => request.method === "POST" && request.url.pathname.endsWith(`/api/matches/${name}`));
const reads = (pattern: RegExp) => seen.filter((request) => request.method === "GET" && pattern.test(request.url.pathname));
const text = (id: string): string => (screen.getByTestId(id).textContent ?? "").replace(/\s+/g, " ").trim();

async function press(key: string, init: KeyboardEventInit = {}): Promise<void> {
    keymap.handle(new KeyboardEvent("keydown", {key, cancelable: true, ...init}));
    await tick();
}

/** Open the overlay focused on `index` and wait for its view. */
async function focusOn(index = 1): Promise<void> {
    matches.openFor(index);
    render(MatchOverlay);
    await vi.waitFor(() => expect(screen.getByTestId("match-focus-txn")).toBeDefined());
}

beforeEach(() => {
    resetMatches();
    resetReviewLaunch();
});
afterEach(() => {
    vi.unstubAllGlobals();
    keymap.reset();
    resetMatches();
    settings.matchLookback = 365;
    settings.matchWindow = 10;
});

describe("COMPONENT MatchOverlay combination card (A4)", () => {
    it("shows the single posting against its group, and links every member", async () => {
        await serve([COMBINATION]);
        matches.openRun();
        render(MatchOverlay);
        expect(text("match-counts")).toContain("0 clear, 1 to check");
        await fireEvent.click(screen.getByTestId("match-start"));
        await vi.waitFor(() => expect(screen.getByTestId("match-card")).toBeDefined());
        expect(text("match-confidence")).toBe("check");
        expect(text("match-combination")).toContain("one posting against 2");
        expect(text("match-header")).toContain("+1");
        expect(screen.getByTestId("match-half-left").getAttribute("data-txn")).toBe("1");
        const lines = screen.getAllByTestId("match-line");
        expect(lines.map((line) => line.getAttribute("data-txn"))).toEqual(["2", "3"]);
        expect(lines[0]?.textContent).toContain("ACME INV 12");
        expect(lines[0]?.textContent).toContain("expenses:services");
        expect(lines[0]?.textContent).toContain("$2000.00");
        expect(lines[0]?.textContent).toContain("services.journal");
        // No Not a match (a rejection is pairwise), nor its key.
        expect(screen.queryByTestId("match-reject")).toBeNull();
        expect(keymap.active.some((binding) => binding.keys === "x")).toBe(false);

        await fireEvent.click(screen.getByTestId("match-link"));
        await vi.waitFor(() => expect(posted("link")).toHaveLength(1));
        expect(posted("link")[0]?.body).toEqual({
            members: [
                {index: 1, posting: 1, fingerprint: "fp-1-0"},
                {index: 2, posting: 1, fingerprint: "fp-2-0"},
                {index: 3, posting: 1, fingerprint: "fp-3-0"},
            ],
        });
    });

    it("ignores only the single posting", async () => {
        await serve([COMBINATION]);
        matches.openRun();
        render(MatchOverlay);
        await fireEvent.click(screen.getByTestId("match-start"));
        await vi.waitFor(() => expect(screen.getByTestId("match-card")).toBeDefined());
        expect(screen.queryByTestId("match-ignore-right")).toBeNull();
        await fireEvent.click(screen.getByTestId("match-ignore-left"));
        await fireEvent.click(screen.getByTestId("match-confirm-ignore-go"));
        await vi.waitFor(() => expect(posted("ignore")).toHaveLength(1));
        expect(posted("ignore")[0]?.body).toEqual({half: {index: 1, posting: 1, fingerprint: "fp-1-0"}});
    });

    it("is never part of Link all", async () => {
        await serve([PAIR, COMBINATION]);
        matches.openRun();
        render(MatchOverlay);
        expect(text("match-counts")).toContain("1 clear, 1 to check");
        await fireEvent.click(screen.getByTestId("match-link-all"));
        await fireEvent.click(screen.getByTestId("match-confirm-all-go"));
        await vi.waitFor(() => expect(posted("link-all")).toHaveLength(1));
        expect((posted("link-all")[0]?.body as {pairs: unknown[]}).pairs).toEqual([
            {a: {index: 7, posting: 1, fingerprint: "fp-7-0"}, b: {index: 8, posting: 1, fingerprint: "fp-8-0"}},
        ]);
    });
});

describe("COMPONENT MatchOverlay older than the lookback (A1)", () => {
    it("says how many, and Show all sets the lookback to All time and reads again", async () => {
        await serve([PAIR], {older: 3});
        matches.openRun();
        render(MatchOverlay);
        expect(text("match-older")).toContain("3 more older than the lookback");
        await fireEvent.click(screen.getByTestId("match-show-all"));
        expect(settings.matchLookback).toBe(0);
        await vi.waitFor(() => expect(reads(/\/api\/matches$/).some((request) => request.url.searchParams.get("lookback") === "0")).toBe(true));
        await vi.waitFor(() => expect(screen.queryByTestId("match-older")).toBeNull());
    });

    it("says so on the empty screen too", async () => {
        await serve([], {older: 5});
        matches.openRun();
        render(MatchOverlay);
        expect(screen.getByTestId("match-empty")).toBeDefined();
        expect(text("match-older")).toContain("5 more older than the lookback");
    });
});

describe("COMPONENT MatchOverlay focused on one transaction (A2)", () => {
    it("offers exact partners as radios, the nearest picked, flagged when outside the window; Link writes the pair", async () => {
        await serve();
        // These are years old: All time, so only the window flags them.
        settings.matchLookback = 0;
        forAnswer = forBody({postings: [{posting: 1, exact: [partner(EXACT, 3), partner(EXACT_FAR, 102)], combinations: [], nearby: []}]});
        forAfter = forBody({postings: []});
        await focusOn();
        expect(screen.getByTestId("match-focus-badge")).toBeDefined();
        expect(reads(/\/api\/matches\/for\/1$/)[0]?.url.searchParams.get("window")).toBe("10");
        expect(text("match-focus-amount")).toBe("$2247.50");
        const radios = screen.getAllByTestId("match-focus-exact-option") as HTMLInputElement[];
        expect(radios.map((radio) => radio.checked)).toEqual([true, false]);
        const lines = screen.getAllByTestId("match-line");
        expect(lines[0]?.textContent).toContain("3 days apart");
        expect(lines[1]?.querySelector("[data-testid='match-line-flag']")?.textContent).toBe("outside the 10-day window");
        expect(lines[0]?.querySelector("[data-testid='match-line-flag']")).toBeNull();
        expect(text("match-focus-total")).toContain("$2247.50 of $2247.50");
        // `c` cycles the radios; Not a match is offered for one exact partner.
        await press("c");
        expect(radios.map((radio) => radio.checked)).toEqual([false, true]);
        await press("c");
        expect(screen.getByTestId("match-reject")).toBeDefined();

        await fireEvent.click(screen.getByTestId("match-link"));
        await vi.waitFor(() => expect(posted("link")).toHaveLength(1));
        expect(posted("link")[0]?.body).toEqual({a: {index: 1, posting: 1, fingerprint: "fp-1-0"}, b: {index: 5, posting: 1, fingerprint: "fp-5-0"}});
        // Nothing left to match on it: the overlay closes.
        await vi.waitFor(() => expect(matches.open).toBe(false));
    });

    it("rejects one exact partner with Not a match", async () => {
        await serve();
        forAnswer = forBody({postings: [{posting: 1, exact: [partner(EXACT, 3)], combinations: [], nearby: []}]});
        await focusOn();
        await press("x");
        await vi.waitFor(() => expect(posted("reject")).toHaveLength(1));
        expect(posted("reject")[0]?.body).toEqual({a: {index: 1, posting: 1, fingerprint: "fp-1-0"}, b: {index: 5, posting: 1, fingerprint: "fp-5-0"}});
    });

    it("builds a group from nearby postings, Link enabled only once they add up exactly", async () => {
        await serve();
        forAnswer = forBody({postings: [{posting: 1, exact: [], combinations: [], nearby: [partner(BILL_A, 1), partner(BILL_B, 2), partner(OTHER, 6)]}]});
        await focusOn();
        const link = () => screen.getByTestId("match-link") as HTMLButtonElement;
        const boxes = screen.getAllByTestId("match-focus-nearby-option") as HTMLInputElement[];
        expect(link().disabled).toBe(true);
        expect(screen.queryByTestId("match-focus-total")).toBeNull();
        await fireEvent.click(boxes[0] as HTMLInputElement);
        expect(text("match-focus-total")).toBe("$2000.00 of $2247.50");
        expect(screen.getByTestId("match-focus-total").getAttribute("data-balanced")).toBe("false");
        expect(link().disabled).toBe(true);
        await fireEvent.click(boxes[2] as HTMLInputElement);
        expect(text("match-focus-total")).toBe("$2050.00 of $2247.50");
        await fireEvent.click(boxes[2] as HTMLInputElement);
        await fireEvent.click(boxes[1] as HTMLInputElement);
        expect(text("match-focus-total")).toBe("$2247.50 of $2247.50");
        expect(screen.getByTestId("match-focus-total").getAttribute("data-balanced")).toBe("true");
        expect(link().disabled).toBe(false);
        expect(link().textContent).toContain("Link 3 postings");
        // Not a match is for a single exact partner only.
        expect(screen.queryByTestId("match-reject")).toBeNull();

        await press("y");
        await vi.waitFor(() => expect(posted("link")).toHaveLength(1));
        expect(posted("link")[0]?.body).toEqual({
            members: [
                {index: 1, posting: 1, fingerprint: "fp-1-0"},
                {index: 2, posting: 1, fingerprint: "fp-2-0"},
                {index: 3, posting: 1, fingerprint: "fp-3-0"},
            ],
        });
    });

    it("offers a suggested combination as a radio choice that picks its group", async () => {
        await serve();
        forAnswer = forBody({
            postings: [{posting: 1, exact: [], combinations: [COMBINATION], nearby: [partner(BILL_A, 1), partner(BILL_B, 2)]}],
        });
        await focusOn();
        const radio = screen.getByTestId("match-focus-combination-option") as HTMLInputElement;
        expect(radio.checked).toBe(true);
        expect((screen.getAllByTestId("match-focus-nearby-option") as HTMLInputElement[]).map((box) => box.checked)).toEqual([true, true]);
        expect(text("match-focus-total")).toBe("$2247.50 of $2247.50");
        await fireEvent.click(screen.getByTestId("match-link"));
        await vi.waitFor(() => expect(posted("link")).toHaveLength(1));
        expect((posted("link")[0]?.body as {members: {index: number}[]}).members.map((member) => member.index)).toEqual([1, 2, 3]);
    });

    it("says why when there is nothing to offer, and offers Ignore this side", async () => {
        await serve();
        forAnswer = forBody({postings: [{posting: 1, exact: [], combinations: [], nearby: []}]});
        forAfter = forBody({postings: []});
        await focusOn();
        expect(text("match-focus-empty")).toContain("No unlinked $-2247.50 on transfers:cc at any date, and nothing opposite within ±10 days.");
        expect(screen.queryByTestId("match-link")).toBeNull();
        await fireEvent.click(screen.getByTestId("match-focus-ignore"));
        expect(posted("ignore")).toHaveLength(0);
        await fireEvent.click(screen.getByTestId("match-confirm-ignore-go"));
        await vi.waitFor(() => expect(posted("ignore")).toHaveLength(1));
        expect(posted("ignore")[0]?.body).toEqual({half: {index: 1, posting: 1, fingerprint: "fp-1-0"}});
        await vi.waitFor(() => expect(matches.open).toBe(false));
    });

    it("widens the nearby window from the view, and reads again with it", async () => {
        await serve();
        forAnswer = forBody({postings: [{posting: 1, exact: [], combinations: [], nearby: []}]});
        await focusOn();
        const input = screen.getByTestId("match-focus-window") as HTMLInputElement;
        input.value = "30";
        await fireEvent.change(input);
        await vi.waitFor(() =>
            expect(
                reads(/\/api\/matches\/for\/1$/)
                    .at(-1)
                    ?.url.searchParams.get("window")
            ).toBe("30")
        );
        input.value = "500";
        await fireEvent.change(input);
        await vi.waitFor(() =>
            expect(
                reads(/\/api\/matches\/for\/1$/)
                    .at(-1)
                    ?.url.searchParams.get("window")
            ).toBe("60")
        );
        expect(settings.matchWindow).toBe(10);
    });

    it("lists the transaction's links, and Unlink (after a confirm) names every member", async () => {
        await serve();
        const linked = [DEBIT, BILL_A, BILL_B].map((txn) => matchHalfWire(txn));
        forAnswer = forBody({postings: [], links: [{matchId: "g1bill", broken: false, members: linked}]});
        await focusOn();
        expect(text("match-focus-none")).toContain("Every transfer half of this transaction is linked.");
        expect(text("match-focus-link")).toContain("match:g1bill");
        expect(text("match-focus-link")).toContain("3 postings");
        await fireEvent.click(screen.getByTestId("match-focus-unlink"));
        expect(posted("unlink")).toHaveLength(0);
        await fireEvent.click(screen.getByTestId("match-focus-unlink-go"));
        await vi.waitFor(() => expect(posted("unlink")).toHaveLength(1));
        expect(posted("unlink")[0]?.body).toEqual({
            matchId: "g1bill",
            halves: [
                {index: 1, posting: 1, fingerprint: "fp-1-0"},
                {index: 2, posting: 1, fingerprint: "fp-2-0"},
                {index: 3, posting: 1, fingerprint: "fp-3-0"},
            ],
        });
        // The view stays open, read again.
        expect(matches.open).toBe(true);
        await vi.waitFor(() => expect(reads(/\/api\/matches\/for\/1$/).length).toBeGreaterThan(1));
    });

    it("unlinks every BROKEN link a transaction carries from the store (the Problems drawer's fix), naming all members", async () => {
        await serve();
        forAnswer = forBody({
            postings: [],
            links: [
                {matchId: "good01", broken: false, members: [DEBIT, BILL_A].map((txn) => matchHalfWire(txn))},
                {matchId: "bad001", broken: true, members: [DEBIT, OTHER, BILL_B].map((txn) => matchHalfWire(txn))},
            ],
        });
        const outcome = await matches.unlinkFrom(1, null);
        expect(outcome.ok).toBe(true);
        expect(posted("unlink").map((request) => request.body)).toEqual([
            {
                matchId: "bad001",
                halves: [
                    {index: 1, posting: 1, fingerprint: "fp-1-0"},
                    {index: 4, posting: 1, fingerprint: "fp-4-0"},
                    {index: 3, posting: 1, fingerprint: "fp-3-0"},
                ],
            },
        ]);
    });

    it("Escape cancels an armed Ignore first, then closes", async () => {
        await serve();
        forAnswer = forBody({postings: [{posting: 1, exact: [], combinations: [], nearby: []}]});
        await focusOn();
        await press("i");
        expect(screen.getByTestId("match-confirm-ignore")).toBeDefined();
        await press("Escape");
        expect(screen.queryByTestId("match-confirm-ignore")).toBeNull();
        expect(matches.open).toBe(true);
        await press("Escape");
        expect(matches.open).toBe(false);
        expect(posted("ignore")).toHaveLength(0);
    });
});
