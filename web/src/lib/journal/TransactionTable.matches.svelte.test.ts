// The journal's transfer matching (plan 35 Phase 3): the Find Matches button
// (badge, disabled states, `m`), collapsed transfer rows (D11: ⇄, "→ Jan 2",
// "Checking → Amex"), expanding one in place without breaking the windowed
// geometry (▸, `l`), Unlink and Forget rejection through the toolbar
// confirm. A separate file: the keymap, the matches store and the journal
// view are module singletons.

import {fireEvent, render, screen} from "@testing-library/svelte";
import {tick} from "svelte";
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import {keymap} from "#lib/keys/keymap.svelte.js";
import {editing} from "#lib/stores/editing.svelte.js";
import {journal} from "#lib/stores/journal.svelte.js";
import {journalView, resetJournalView} from "#lib/stores/journalView.svelte.js";
import {matches, resetMatches} from "#lib/stores/matches.svelte.js";
import {settings} from "#lib/stores/settings.svelte.js";
import {connectFakeEngine, FAKE_ENGINE, FAKE_JOURNAL_INFO} from "#lib/testing/fakeEngine.js";
import {summaryBody, VIEW_GENERATION, wireRow} from "#lib/testing/journalViewFixture.js";
import {matchesPageWire, proposalWire, transferHalf, transferRowWire} from "#lib/testing/matchFixture.js";
import {queueItemWire, type SeenRequest} from "#lib/testing/reviewFixture.js";
import TransactionTable from "./TransactionTable.svelte";

const OUT = transferHalf(10, "2026-12-31", "ACH AMEX EPAYMENT", "assets:checking", -50000, [["match", "k7f3a2"]]);
const IN = transferHalf(11, "2027-01-02", "PAYMENT THANK YOU", "liabilities:amex", 50000, [["match", "k7f3a2"]]);
const REJECTED = transferHalf(12, "2026-12-20", "TRANSFER TO SAVINGS", "assets:checking", -2500, [["nomatch", "r4x9"]]);
const GROCERIES = transferHalf(13, "2027-01-03", "Whole Foods", "expenses:groceries", 8412);

/**
 * Display order (newest first): Whole Foods (flagged `linked` as a transaction
 * in two links would be: not collapsed, ⇄ all the same), the transfer
 * (anchored on its outgoing half), the rejected transfer.
 */
const ROWS = [{...wireRow(GROCERIES), linked: true}, transferRowWire("k7f3a2", OUT, IN), wireRow(REJECTED)];

let seen: SeenRequest[] = [];
let matchesAnswer: unknown = matchesPageWire([]);

async function show(): Promise<void> {
    await connectFakeEngine();
    seen = [];
    vi.stubGlobal("fetch", (input: RequestInfo | URL, init?: RequestInit) => {
        const url = new URL(String(input));
        const method = init?.method ?? "GET";
        seen.push({method, url, body: typeof init?.body === "string" ? (JSON.parse(init.body) as unknown) : undefined});
        const json = (body: unknown, status = 200) => Promise.resolve(new Response(JSON.stringify(body), {status}));
        const path = url.pathname;
        if (path.endsWith("/api/transactions")) return Promise.resolve(new Response("", {status: 405}));
        if (path.endsWith("/api/journal")) return json(FAKE_JOURNAL_INFO);
        if (path.endsWith("/api/diagnostics")) return json({count: 0, maxSeverity: null, diagnostics: []});
        if (path.endsWith("/api/journal/rows")) return json({generation: VIEW_GENERATION, total: ROWS.length, offset: 0, rows: ROWS});
        if (path.endsWith("/api/journal/summary")) return json(summaryBody(ROWS.length, {transfers: 1}));
        if (path.endsWith("/api/matches")) return json(matchesAnswer);
        const item = /\/api\/review\/item\/(\d+)$/.exec(path);
        if (item !== null) {
            const txn = [OUT, IN, REJECTED].find((candidate) => candidate.index === Number(item[1]));
            return txn === undefined ? json({error: "gone"}, 404) : json({generation: "g1", item: queueItemWire(txn, {unknown: false})});
        }
        if (path.endsWith("/api/matches/unlink") || path.endsWith("/api/matches/forget")) return json({generation: "g2", id: "x", edited: []});
        return Promise.resolve(new Response(`no route for ${url}`, {status: 404}));
    });
    await editing.probe();
    await journal.refresh({force: true});
    resetJournalView();
    journalView.sync(FAKE_ENGINE, VIEW_GENERATION, {from: null, to: null, accounts: new Set(), query: ""});
    await vi.waitFor(() => {
        if (journalView.total === null || journalView.rowAt(0) === null) throw new Error("not loaded");
    });
}

async function press(key: string): Promise<void> {
    keymap.handle(new KeyboardEvent("keydown", {key, cancelable: true}));
    await tick();
}

const rowTexts = (): string[] => [...document.querySelectorAll("tbody tr[data-txn]")].map((row) => (row.textContent ?? "").replace(/\s+/g, " ").trim());

beforeEach(() => {
    resetMatches();
    matchesAnswer = matchesPageWire([]);
    Object.defineProperty(HTMLElement.prototype, "clientWidth", {configurable: true, get: () => 1024});
});
afterEach(() => {
    vi.unstubAllGlobals();
    Reflect.deleteProperty(HTMLElement.prototype, "clientWidth");
    keymap.reset();
    resetMatches();
});

describe("COMPONENT TransactionTable Find Matches button (D9)", () => {
    it("badges the proposals and opens the overlay from the button and from m", async () => {
        matchesAnswer = matchesPageWire([
            proposalWire(transferHalf(1, "2026-11-01", "OUT", "assets:checking", -100), transferHalf(2, "2026-11-02", "IN", "assets:savings", 100)),
        ]);
        await show();
        render(TransactionTable);
        await vi.waitFor(() => expect(screen.getByTestId("find-matches-count").textContent).toBe("1"));
        const url = seen.find((request) => request.url.pathname.endsWith("/api/matches"))?.url;
        expect(url?.searchParams.get("window")).toBe(String(settings.matchWindow));
        expect(url?.searchParams.get("lookback")).toBe(String(settings.matchLookback));
        await fireEvent.click(screen.getByTestId("find-matches-button"));
        expect(matches.open).toBe(true);
        matches.close();
        await press("m");
        expect(matches.open).toBe(true);
    });

    it("is disabled, key included, with nothing to propose", async () => {
        await show();
        render(TransactionTable);
        await vi.waitFor(() => expect(matches.page).not.toBeNull());
        expect((screen.getByTestId("find-matches-button") as HTMLButtonElement).disabled).toBe(true);
        expect(screen.queryByTestId("find-matches-docs")).toBeNull();
        await press("m");
        expect(matches.open).toBe(false);
    });

    it("says how to declare a clearing account when there is none, linking the docs", async () => {
        matchesAnswer = matchesPageWire([], {clearing: false});
        await show();
        render(TransactionTable);
        await vi.waitFor(() => expect(screen.getByTestId("find-matches-docs")).toBeDefined());
        expect((screen.getByTestId("find-matches-button") as HTMLButtonElement).disabled).toBe(true);
        expect(screen.getByTestId("find-matches-button").parentElement?.getAttribute("data-tip")).toContain("clearing:");
        expect(screen.getByTestId("find-matches-docs").getAttribute("href")).toContain("transfer-matching");
    });
});

describe("COMPONENT TransactionTable transfer rows (D11)", () => {
    it("shows a linked pair as one row: ⇄, its own description, → the other date, and Checking → Amex", async () => {
        await show();
        render(TransactionTable);
        await tick();
        const row = document.querySelector("tr[data-transfer='k7f3a2']") as HTMLElement;
        expect(row.getAttribute("data-txn")).toBe("10");
        expect(row.textContent).toContain("ACH AMEX EPAYMENT");
        expect(row.textContent).not.toContain("PAYMENT THANK YOU");
        expect(row.querySelector("[data-testid='transfer-icon']")).not.toBeNull();
        expect(row.querySelector("[data-testid='transfer-partner-date']")?.textContent).toContain("Jan 2");
        // Full account names, rendered like every other row's accounts, and read-only:
        // the two accounts live on two different transactions.
        const ends = row.querySelector("[data-testid='transfer-accounts']") as HTMLElement;
        expect(ends.textContent?.replace(/\s+/g, " ").trim()).toBe("assets:checking → liabilities:amex");
        expect(ends.querySelector("button")).toBeNull();
        expect(row.textContent).toContain("$500.00");
        expect(rowTexts()).toHaveLength(3);
        // A linked transaction that is not collapsed still shows ⇄; an unlinked one does not.
        const icon = (index: number) => document.querySelector(`tr[data-txn='${index}'] [data-testid='transfer-icon']`);
        expect(icon(13)).not.toBeNull();
        expect(icon(12)).toBeNull();
    });

    it("expands in place to both halves as ordinary rows, from ▸ and from l, and collapses again", async () => {
        await show();
        render(TransactionTable);
        await tick();
        await fireEvent.click(screen.getByTestId("transfer-toggle"));
        const texts = rowTexts();
        expect(texts).toHaveLength(5);
        expect(texts[2]).toContain("ACH AMEX EPAYMENT");
        expect(texts[3]).toContain("PAYMENT THANK YOU");
        expect(texts[4]).toContain("TRANSFER TO SAVINGS");
        const halves = [...document.querySelectorAll("tbody tr[data-txn]")].slice(2, 4);
        expect(halves.map((row) => row.getAttribute("data-txn"))).toEqual(["10", "11"]);
        expect(halves.every((row) => row.querySelector("[data-testid='transfer-icon']") !== null)).toBe(true);
        expect(screen.getByTestId("transfer-toggle").getAttribute("aria-expanded")).toBe("true");

        // The keyboard: the cursor walks the halves, and `l` on one folds them back.
        // Slots: Whole Foods, the transfer row, its two halves.
        for (let step = 0; step < 4; step += 1) await press("j");
        expect(document.querySelector("[aria-current='true']")?.getAttribute("data-txn")).toBe("11");
        await press("l");
        expect(rowTexts()).toHaveLength(3);
        expect(document.querySelector("[aria-current='true']")?.getAttribute("data-transfer")).toBe("k7f3a2");
        await press("l");
        expect(rowTexts()).toHaveLength(5);
    });

    it("leaves l unbound on an ordinary row", async () => {
        await show();
        render(TransactionTable);
        await tick();
        await press("j");
        expect(keymap.active.some((binding) => binding.keys === "l")).toBe(false);
        await press("j");
        expect(keymap.active.some((binding) => binding.keys === "l")).toBe(true);
    });

    it("unlinks after a confirm in the toolbar, naming both clearing postings at their current fingerprints", async () => {
        await show();
        render(TransactionTable);
        await tick();
        await fireEvent.click(screen.getByTestId("transfer-unlink"));
        expect(screen.getByTestId("match-confirm").textContent).toContain("checking → amex");
        expect(seen.some((request) => request.method === "POST")).toBe(false);
        await fireEvent.click(screen.getByTestId("match-confirm-go"));
        await vi.waitFor(() => expect(seen.some((request) => request.url.pathname.endsWith("/api/matches/unlink"))).toBe(true));
        const unlink = seen.find((request) => request.url.pathname.endsWith("/api/matches/unlink"));
        expect(unlink?.body).toEqual({
            matchId: "k7f3a2",
            halves: [
                {index: 10, posting: 1, fingerprint: "fp-10-0"},
                {index: 11, posting: 1, fingerprint: "fp-11-0"},
            ],
        });
    });

    it("offers Forget rejection on a row whose posting carries nomatch, and forgets it after a confirm", async () => {
        await show();
        render(TransactionTable);
        await tick();
        const forget = screen.getAllByTestId("forget-rejection");
        expect(forget).toHaveLength(1);
        expect(forget[0]?.closest("tr")?.getAttribute("data-txn")).toBe("12");
        await fireEvent.click(forget[0] as HTMLElement);
        expect(screen.getByTestId("match-confirm").textContent).toContain("r4x9");
        await fireEvent.click(screen.getByTestId("match-confirm-go"));
        await vi.waitFor(() => expect(seen.some((request) => request.url.pathname.endsWith("/api/matches/forget"))).toBe(true));
        expect(seen.find((request) => request.url.pathname.endsWith("/api/matches/forget"))?.body).toEqual({
            value: "r4x9",
            halves: [{index: 12, posting: 1, fingerprint: "fp-12-0"}],
        });
    });
});
