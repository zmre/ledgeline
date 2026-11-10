// The Find Matches overlay (plan 35 D9) against a fake engine: the counts
// screen and Link all (asks first), one card per pair with its header,
// confidence and gap, Link / Link and Save / Not a match / Skip / Ignore this
// side, an ambiguous pair's alternatives, and a 409. Every write re-reads the
// journal (its generation moves) and then the proposals, which is what moves
// the deck on.

import {fireEvent, render, screen, within} from "@testing-library/svelte";
import {tick} from "svelte";
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";

import {keymap} from "#lib/keys/keymap.svelte.js";
import {resetReviewLaunch} from "#lib/review/ui/launch.svelte.js";
import {editing} from "#lib/stores/editing.svelte.js";
import {journal} from "#lib/stores/journal.svelte.js";
import {matches, resetMatches} from "#lib/stores/matches.svelte.js";
import {connectFakeEngine, FAKE_ENGINE, journalInfoBody} from "#lib/testing/fakeEngine.js";
import {matchesPageWire, matchHalfWire, proposalWire, transferHalf} from "#lib/testing/matchFixture.js";
import type {SeenRequest} from "#lib/testing/reviewFixture.js";
import {MOD} from "#lib/testing/reviewOverlay.js";
import MatchOverlay from "./MatchOverlay.svelte";

const OUT = transferHalf(1, "2026-12-31", "ACH AMEX EPAYMENT", "assets:checking", -50000);
const IN = transferHalf(2, "2027-01-02", "PAYMENT THANK YOU", "liabilities:amex", 50000);
const SAVE = transferHalf(3, "2027-02-01", "TRANSFER TO SAVINGS", "assets:checking", -10000);
const SAVED = transferHalf(4, "2027-02-03", "ONLINE TRANSFER FROM CHK", "assets:savings", 10000);
const SAVED_LATE = transferHalf(5, "2027-02-03", "ANOTHER TRANSFER IN", "assets:brokerage", 10000);

const CLEAR = proposalWire(OUT, IN);
const CHECK = proposalWire(SAVE, SAVED, {
    clear: false,
    ambiguous: true,
    alternatives: [{a: matchHalfWire(SAVE), b: matchHalfWire(SAVED_LATE), gapDays: 2, score: 2, taken: false}],
});

let seen: SeenRequest[] = [];
let proposals: Record<string, unknown>[] = [];
let generation = 1;
/** Answer the next match write with this status instead (a 409, a 400). */
let failNext: {status: number; error: string; then?: Record<string, unknown>[]} | null = null;

const POSTS = ["link", "reject", "ignore", "link-all"] as const;

/** The proposals that do not involve transaction `index`. */
const without = (indices: number[]): Record<string, unknown>[] =>
    proposals.filter((proposal) => !indices.some((index) => [proposal.a, proposal.b].some((half) => (half as {index: number}).index === index)));

async function serve(initial: Record<string, unknown>[]): Promise<void> {
    await connectFakeEngine();
    seen = [];
    proposals = initial;
    generation = 1;
    failNext = null;
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
        if (path.endsWith("/api/matches")) return json(matchesPageWire(proposals, {generation: `g${generation}`}));
        const post = POSTS.find((name) => path.endsWith(`/api/matches/${name}`));
        if (post !== undefined && body !== undefined) {
            if (failNext !== null) {
                const {status, error, then} = failNext;
                failNext = null;
                if (status === 409) generation += 1;
                if (then !== undefined) proposals = then;
                return json({error}, status);
            }
            generation += 1;
            if (post === "link-all") {
                const pairs = body.pairs as {a: {index: number}}[];
                proposals = without(pairs.map((pair) => pair.a.index));
                return json({generation: `g${generation}`, linked: pairs.length, matchIds: pairs.map((_, at) => `id${at}`)});
            }
            const half = (post === "ignore" ? body.half : body.a) as {index: number};
            proposals = without([half.index]);
            return json({generation: `g${generation}`, id: "k7f3a2", edited: []});
        }
        return Promise.resolve(new Response(`no route for ${url}`, {status: 404}));
    });
    await editing.probe();
    await journal.refresh({force: true});
    await matches.sync(FAKE_ENGINE, journal.generation ?? "");
    matches.openRun();
}

const posted = (name: (typeof POSTS)[number]) => seen.filter((request) => request.method === "POST" && request.url.pathname.endsWith(`/api/matches/${name}`));

async function press(key: string, init: KeyboardEventInit = {}): Promise<void> {
    keymap.handle(new KeyboardEvent("keydown", {key, cancelable: true, ...init}));
    await tick();
}

async function startCards(): Promise<void> {
    await fireEvent.click(screen.getByTestId("match-start"));
    await vi.waitFor(() => expect(screen.getByTestId("match-card")).toBeDefined());
}

const header = (): string => (screen.getByTestId("match-header").textContent ?? "").replace(/\s+/g, " ").trim();

beforeEach(() => {
    resetMatches();
    resetReviewLaunch();
});
afterEach(() => {
    vi.unstubAllGlobals();
    keymap.reset();
    resetMatches();
});

describe("COMPONENT MatchOverlay counts screen", () => {
    it("shows the counts and links every clear pair, after a confirm, in one request", async () => {
        await serve([CLEAR, CHECK]);
        render(MatchOverlay);
        expect(screen.getByTestId("match-counts").textContent?.replace(/\s+/g, " ")).toContain("1 clear, 1 to check");
        await fireEvent.click(screen.getByTestId("match-link-all"));
        expect(screen.getByTestId("match-confirm-all").textContent).toContain("Link all 1 clear matches?");
        expect(posted("link-all")).toHaveLength(0);
        await fireEvent.click(screen.getByTestId("match-confirm-all-go"));
        await vi.waitFor(() => expect(screen.getByTestId("match-linked-all").textContent).toContain("Linked 1 pair."));
        expect(posted("link-all")[0]?.body).toEqual({
            pairs: [{a: {index: 1, posting: 1, fingerprint: "fp-1-0"}, b: {index: 2, posting: 1, fingerprint: "fp-2-0"}}],
        });
        expect(screen.getByTestId("match-counts").textContent?.replace(/\s+/g, " ")).toContain("0 clear, 1 to check");
        expect(screen.queryByTestId("match-link-all")).toBeNull();
    });

    it("says there is nothing to match", async () => {
        await serve([]);
        render(MatchOverlay);
        expect(screen.getByTestId("match-empty").textContent).toContain("Nothing to match");
    });
});

describe("COMPONENT MatchOverlay cards", () => {
    it("shows a pair: Checking → Amex, the amount, clear, the gap, both halves, via and the file", async () => {
        await serve([CLEAR]);
        render(MatchOverlay);
        await startCards();
        expect(header()).toBe("checking → amex");
        expect(screen.getByTestId("match-amount").textContent?.trim()).toBe("$500.00");
        expect(screen.getByTestId("match-confidence").textContent).toBe("clear");
        expect(screen.getByTestId("match-gap").textContent).toBe("2 days apart");
        const left = screen.getByTestId("match-half-left");
        expect(left.textContent).toContain("Dec 31, 2026");
        expect(left.textContent).toContain("assets:checking");
        expect(left.textContent).toContain("via transfers:cc");
        expect((left.querySelector("input") as HTMLInputElement).value).toBe("ACH AMEX EPAYMENT");
        expect(within(left).getByTestId("match-half-file").textContent).toBe("checking.journal");
        const right = screen.getByTestId("match-half-right");
        expect(right.textContent).toContain("liabilities:amex");
        expect(within(right).getByTestId("match-half-file").textContent).toBe("amex.journal");
    });

    it("links, and the re-read moves the deck on to the summary", async () => {
        await serve([CLEAR]);
        render(MatchOverlay);
        await startCards();
        await fireEvent.click(screen.getByTestId("match-link"));
        await vi.waitFor(() => expect(screen.getByTestId("match-tally").textContent).toContain("1 linked"));
        expect(posted("link")[0]?.body).toEqual({a: {index: 1, posting: 1, fingerprint: "fp-1-0"}, b: {index: 2, posting: 1, fingerprint: "fp-2-0"}});
    });

    it("turns Link into Link and Save once a description changes, and saves it in the same write", async () => {
        await serve([CLEAR]);
        render(MatchOverlay);
        await startCards();
        expect(screen.getByTestId("match-link").textContent).not.toContain("Save");
        const field = screen.getByTestId("match-half-left").querySelector("input") as HTMLInputElement;
        await fireEvent.input(field, {target: {value: "Amex payment"}});
        expect(screen.getByTestId("match-link").textContent).toContain("Link and Save");
        // Revert restores the original.
        await fireEvent.click(screen.getByTestId("match-revert"));
        expect((screen.getByTestId("match-half-left").querySelector("input") as HTMLInputElement).value).toBe("ACH AMEX EPAYMENT");
        await fireEvent.input(screen.getByTestId("match-half-left").querySelector("input") as HTMLInputElement, {target: {value: "Amex payment"}});
        await press("Enter", MOD);
        await vi.waitFor(() => expect(posted("link")).toHaveLength(1));
        expect(posted("link")[0]?.body).toMatchObject({a: {index: 1, description: "Amex payment"}, b: {index: 2}});
        expect((posted("link")[0]?.body as {b: Record<string, unknown>}).b.description).toBeUndefined();
    });

    it("remembers Not a match, and Skip writes nothing", async () => {
        await serve([CLEAR, CHECK]);
        render(MatchOverlay);
        await startCards();
        await press("ArrowRight");
        expect(header()).toBe("checking → savings");
        expect(seen.filter((request) => request.method === "POST")).toHaveLength(0);
        await fireEvent.click(screen.getByTestId("match-reject"));
        await vi.waitFor(() => expect(posted("reject")).toHaveLength(1));
        expect(posted("reject")[0]?.body).toEqual({a: {index: 3, posting: 1, fingerprint: "fp-3-0"}, b: {index: 4, posting: 1, fingerprint: "fp-4-0"}});
        await vi.waitFor(() => expect(screen.getByTestId("match-tally").textContent).toContain("1 not a match · 0 ignored · 1 skipped"));
    });

    it("ignores one side only after a confirm", async () => {
        await serve([CLEAR]);
        render(MatchOverlay);
        await startCards();
        await fireEvent.click(screen.getByTestId("match-ignore-right"));
        expect(screen.getByTestId("match-confirm-ignore").textContent).toContain("PAYMENT THANK YOU");
        expect(posted("ignore")).toHaveLength(0);
        await fireEvent.click(screen.getByTestId("match-confirm-ignore-go"));
        await vi.waitFor(() => expect(posted("ignore")).toHaveLength(1));
        expect(posted("ignore")[0]?.body).toEqual({half: {index: 2, posting: 1, fingerprint: "fp-2-0"}});
    });

    it("offers an ambiguous pair's alternatives, the nearest selected, and links the one picked", async () => {
        await serve([CHECK]);
        render(MatchOverlay);
        await startCards();
        expect(screen.getByTestId("match-confidence").textContent).toBe("check");
        const radios = screen.getByTestId("match-choices").querySelectorAll<HTMLInputElement>("input[type='radio']");
        expect(radios).toHaveLength(2);
        expect(radios[0]?.checked).toBe(true);
        expect(screen.getByTestId("match-choices").textContent).toContain("ANOTHER TRANSFER IN");
        await press("c");
        expect(header()).toBe("checking → brokerage");
        expect(screen.getByTestId("match-half-right").textContent).toContain("assets:brokerage");
        await press("y");
        await vi.waitFor(() => expect(posted("link")).toHaveLength(1));
        expect(posted("link")[0]?.body).toEqual({a: {index: 3, posting: 1, fingerprint: "fp-3-0"}, b: {index: 5, posting: 1, fingerprint: "fp-5-0"}});
    });

    it("says so on a 409, writes nothing, and shows the pair as read again", async () => {
        await serve([CLEAR]);
        render(MatchOverlay);
        await startCards();
        failNext = {status: 409, error: "stale"};
        await fireEvent.click(screen.getByTestId("match-link"));
        await vi.waitFor(() => expect(screen.getByTestId("match-notice").textContent).toContain("changed"));
        expect(screen.getByTestId("match-card")).toBeDefined();
        expect(screen.queryByTestId("match-tally")).toBeNull();
        // The journal and the proposals were read again at the new generation.
        expect(seen.filter((request) => request.url.pathname.endsWith("/api/matches") && request.method === "GET").length).toBeGreaterThan(1);
    });

    it("keeps the descriptions typed when a 409 re-reads the pair at new fingerprints", async () => {
        await serve([CLEAR]);
        render(MatchOverlay);
        await startCards();
        await fireEvent.input(screen.getByTestId("match-half-left").querySelector("input") as HTMLInputElement, {target: {value: "Amex payment"}});
        failNext = {status: 409, error: "stale", then: [{...CLEAR, a: matchHalfWire(OUT, "fp-1-1"), b: matchHalfWire(IN, "fp-2-1")}]};
        await fireEvent.click(screen.getByTestId("match-link"));
        await vi.waitFor(() => expect(screen.getByTestId("match-notice")).toBeDefined());
        expect((screen.getByTestId("match-half-left").querySelector("input") as HTMLInputElement).value).toBe("Amex payment");
        await fireEvent.click(screen.getByTestId("match-link"));
        await vi.waitFor(() => expect(posted("link")).toHaveLength(2));
        expect(posted("link")[1]?.body).toMatchObject({a: {fingerprint: "fp-1-1", description: "Amex payment"}, b: {fingerprint: "fp-2-1"}});
    });

    it("keeps a refused write's reason on the card", async () => {
        await serve([CLEAR]);
        render(MatchOverlay);
        await startCards();
        failNext = {status: 400, error: "the halves do not cancel"};
        await fireEvent.click(screen.getByTestId("match-link"));
        await vi.waitFor(() => expect(screen.getByTestId("match-failure").textContent).toContain("do not cancel"));
    });
});

describe("COMPONENT MatchOverlay closing", () => {
    it("asks before closing over edited descriptions", async () => {
        await serve([CLEAR]);
        render(MatchOverlay);
        await startCards();
        await fireEvent.input(screen.getByTestId("match-half-right").querySelector("input") as HTMLInputElement, {target: {value: "Paid"}});
        (screen.getByTestId("match-card") as HTMLElement).focus();
        await press("Escape");
        expect(screen.getByTestId("match-confirm-close")).toBeDefined();
        expect(matches.open).toBe(true);
        await fireEvent.click(screen.getByText("Discard and close"));
        expect(matches.open).toBe(false);
    });
});
