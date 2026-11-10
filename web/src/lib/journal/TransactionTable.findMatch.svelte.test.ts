// The journal's Amendment A entry points (plan 35): Find Matches' ▾ (the
// window and the lookback, presets included, clamped, re-read on change),
// the older-than-the-lookback count in the button's tooltip, and Find match
// for one transaction — the row's hover action and `M` — on a row with an
// unlinked clearing posting only. A separate file: the keymap, the matches
// store and the journal view are module singletons.

import {fireEvent, render, screen, within} from "@testing-library/svelte";
import {tick} from "svelte";
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import {keymap} from "#lib/keys/keymap.svelte.js";
import {editing} from "#lib/stores/editing.svelte.js";
import {journal} from "#lib/stores/journal.svelte.js";
import {journalView, resetJournalView} from "#lib/stores/journalView.svelte.js";
import {matches, resetMatches} from "#lib/stores/matches.svelte.js";
import {settings} from "#lib/stores/settings.svelte.js";
import {connectFakeEngine, FAKE_ENGINE, FAKE_JOURNAL_INFO, transactionMatchesBody} from "#lib/testing/fakeEngine.js";
import {summaryBody, VIEW_GENERATION, wireRow} from "#lib/testing/journalViewFixture.js";
import {clearingHalf, matchesPageWire, proposalWire, transferHalf} from "#lib/testing/matchFixture.js";
import {reviewTxnWire, type SeenRequest} from "#lib/testing/reviewFixture.js";
import TransactionTable from "./TransactionTable.svelte";

/** In transit: an unlinked clearing posting. */
const OPEN = clearingHalf(20, "2026-11-02", "CAR PAYMENT", "assets:checking", -50655);
/** Linked, so nothing to find. */
const LINKED = clearingHalf(21, "2026-11-01", "LINKED HALF", "assets:checking", -100, [["match", "k7f3a2"]]);
/** No clearing posting at all. */
const GROCERIES = transferHalf(22, "2026-10-30", "Whole Foods", "expenses:groceries", 8412);
const ROWS = [wireRow(OPEN), {...wireRow(LINKED), linked: true}, wireRow(GROCERIES)];

let seen: SeenRequest[] = [];
let matchesAnswer: unknown = matchesPageWire([]);

async function show(): Promise<void> {
    await connectFakeEngine();
    seen = [];
    vi.stubGlobal("fetch", (input: RequestInfo | URL, init?: RequestInit) => {
        const url = new URL(String(input));
        seen.push({method: init?.method ?? "GET", url, body: undefined});
        const json = (body: unknown, status = 200) => Promise.resolve(new Response(JSON.stringify(body), {status}));
        const path = url.pathname;
        if (path.endsWith("/api/transactions")) return Promise.resolve(new Response("", {status: 405}));
        if (path.endsWith("/api/journal")) return json(FAKE_JOURNAL_INFO);
        if (path.endsWith("/api/diagnostics")) return json({count: 0, maxSeverity: null, diagnostics: []});
        if (path.endsWith("/api/journal/rows")) return json({generation: VIEW_GENERATION, total: ROWS.length, offset: 0, rows: ROWS});
        if (path.endsWith("/api/journal/summary")) return json(summaryBody(ROWS.length));
        if (path.endsWith("/api/matches")) return json(matchesAnswer);
        if (/\/api\/matches\/for\/\d+$/.test(path)) return json(transactionMatchesBody(reviewTxnWire(OPEN, {unknown: false})));
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

const matchReads = () => seen.filter((request) => request.url.pathname.endsWith("/api/matches"));
const tip = (): string => screen.getByTestId("find-matches-button").parentElement?.getAttribute("data-tip") ?? "";

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
    settings.matchLookback = 365;
    settings.matchWindow = 10;
});

describe("COMPONENT TransactionTable Find Matches ▾ (A1)", () => {
    it("holds the window and the lookback, with presets; each change reads again", async () => {
        await show();
        render(TransactionTable);
        await vi.waitFor(() => expect(matchReads()).toHaveLength(1));
        await fireEvent.click(screen.getByTestId("find-matches-menu"));
        // Review Recents' ▾ holds these settings too: work inside Find Matches' own.
        const menu = within(screen.getByTestId("find-matches-menu").closest("details") as HTMLElement);
        expect(menu.getByTestId("setting-matchWindow")).toBeDefined();
        // The ▾ is Find Matches' own: no review settings in it.
        expect(menu.queryByText("Review Recents takes")).toBeNull();

        await fireEvent.click(menu.getByTestId("lookback-preset-730"));
        expect(settings.matchLookback).toBe(730);
        expect(menu.getByTestId("lookback-preset-730").getAttribute("aria-pressed")).toBe("true");
        expect((menu.getByTestId("setting-matchLookback") as HTMLInputElement).value).toBe("730");
        await vi.waitFor(() => expect(matchReads().at(-1)?.url.searchParams.get("lookback")).toBe("730"));

        await fireEvent.click(menu.getByTestId("lookback-preset-0"));
        await vi.waitFor(() => expect(matchReads().at(-1)?.url.searchParams.get("lookback")).toBe("0"));

        const windowInput = menu.getByTestId("setting-matchWindow") as HTMLInputElement;
        windowInput.value = "200";
        await fireEvent.change(windowInput);
        // Clamped to the engine's 1–60.
        expect(settings.matchWindow).toBe(60);
        await vi.waitFor(() => expect(matchReads().at(-1)?.url.searchParams.get("window")).toBe("60"));
    });

    it("says how many pairs are older than the lookback, and opens on them alone", async () => {
        const pair = proposalWire(transferHalf(1, "2026-11-01", "OUT", "assets:checking", -100), transferHalf(2, "2026-11-02", "IN", "assets:savings", 100));
        matchesAnswer = matchesPageWire([pair], {older: 4});
        await show();
        render(TransactionTable);
        await vi.waitFor(() => expect(tip()).toBe("Find Matches (m) · 4 more older than the lookback"));

        matchesAnswer = matchesPageWire([], {older: 4, generation: "fake-2"});
        resetMatches();
        await matches.sync(FAKE_ENGINE, "other");
        await vi.waitFor(() => expect(tip()).toBe("Nothing to pair in the lookback; 4 more older than the lookback"));
        expect((screen.getByTestId("find-matches-button") as HTMLButtonElement).disabled).toBe(false);
        await press("m");
        expect(matches.open).toBe(true);
    });
});

describe("COMPONENT TransactionTable Find match for one transaction (A2)", () => {
    it("offers the row action only on a row with an unlinked clearing posting", async () => {
        await show();
        render(TransactionTable);
        await tick();
        const action = (index: number) => document.querySelector(`tr[data-txn='${index}'] [data-testid='row-find-match']`);
        expect(action(20)).not.toBeNull();
        expect(action(21)).toBeNull();
        expect(action(22)).toBeNull();
        await fireEvent.click(action(20) as HTMLElement);
        expect(matches.open).toBe(true);
        expect(matches.focus?.index).toBe(20);
        await vi.waitFor(() => expect(seen.some((request) => request.url.pathname.endsWith("/api/matches/for/20"))).toBe(true));
    });

    it("binds M on such a row only", async () => {
        await show();
        render(TransactionTable);
        await tick();
        const bound = () => keymap.active.some((binding) => binding.keys === "M");
        expect(bound()).toBe(false);
        await press("j");
        expect(bound()).toBe(true);
        await press("j");
        expect(bound()).toBe(false);
        await press("j");
        expect(bound()).toBe(false);
        await press("k");
        await press("k");
        await press("M");
        expect(matches.focus?.index).toBe(20);
    });
});
