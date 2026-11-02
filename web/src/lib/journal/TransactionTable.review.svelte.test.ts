// The journal's review affordances (plan 32 D17, D18, D20): the two toolbar
// buttons and their count badges, the `u`/`r` keys disabled exactly when the
// buttons are, the row stripes, and the cursor landing on the last reviewed
// transaction when a run closes. A separate file: the keymap, the launch store
// and the journal view are module singletons.

import {fireEvent, render, screen} from "@testing-library/svelte";
import {tick} from "svelte";
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import type {Transaction} from "#lib/domain/types.js";
import {keymap} from "#lib/keys/keymap.svelte.js";
import {minusDays} from "#lib/review/card.js";
import {resetReviewLaunch, reviewLaunch} from "#lib/review/ui/launch.svelte.js";
import {editing} from "#lib/stores/editing.svelte.js";
import {localToday} from "#lib/stores/filters.svelte.js";
import {journalView, resetJournalView} from "#lib/stores/journalView.svelte.js";
import {settings} from "#lib/stores/settings.svelte.js";
import {connectFakeEngine, FAKE_ENGINE} from "#lib/testing/fakeEngine.js";
import {summaryBody, VIEW_GENERATION, wireRow} from "#lib/testing/journalViewFixture.js";
import {txnFixture} from "#lib/testing/reviewFixture.js";
import TransactionTable from "./TransactionTable.svelte";

const TODAY = localToday();

interface Row {
    txn: Transaction;
    unknown: boolean;
    reviewed: boolean;
}

const ROWS: Row[] = [
    {txn: txnFixture(6, "Future unknown", "expenses:unknown", "2999-01-01"), unknown: true, reviewed: false},
    {txn: txnFixture(5, "Coffee", "expenses:unknown", TODAY), unknown: true, reviewed: false},
    {txn: txnFixture(4, "Safeway", "expenses:food", minusDays(TODAY, 1)), unknown: false, reviewed: false},
    {txn: txnFixture(3, "Rent", "expenses:rent", minusDays(TODAY, 2)), unknown: false, reviewed: true},
    {txn: txnFixture(2, "Edge of the window", "expenses:food", minusDays(TODAY, 365)), unknown: false, reviewed: false},
    {txn: txnFixture(1, "Old", "expenses:food", minusDays(TODAY, 366)), unknown: true, reviewed: false},
];

let summaryUrls: URL[] = [];

async function show(review: {unknown: number; unreviewedRecent: number}): Promise<void> {
    await connectFakeEngine();
    summaryUrls = [];
    const rows = ROWS.map((row) => ({...wireRow(row.txn), unknown: row.unknown, reviewed: row.reviewed}));
    vi.stubGlobal("fetch", (input: RequestInfo | URL) => {
        const url = new URL(String(input));
        const json = (body: unknown, status = 200) => Promise.resolve(new Response(JSON.stringify(body), {status}));
        if (url.pathname.endsWith("/api/transactions")) return Promise.resolve(new Response("", {status: 405}));
        if (url.pathname.endsWith("/api/journal/rows")) return json({generation: VIEW_GENERATION, total: rows.length, offset: 0, rows});
        if (url.pathname.endsWith("/api/journal/position")) {
            const index = Number(url.searchParams.get("index"));
            return json({generation: VIEW_GENERATION, position: ROWS.findIndex((row) => row.txn.index === index), date: null, total: rows.length});
        }
        if (url.pathname.endsWith("/api/journal/summary")) {
            summaryUrls.push(url);
            return json(summaryBody(rows.length, {review: {...review, notEditable: 0, notEditableRecent: 0}}));
        }
        return Promise.resolve(new Response("no route", {status: 404}));
    });
    await editing.probe();
    resetJournalView();
    journalView.sync(FAKE_ENGINE, VIEW_GENERATION, {from: null, to: null, accounts: new Set(), query: ""});
    await vi.waitFor(() => {
        if (journalView.summary === null || journalView.total === null) throw new Error("not loaded");
    });
}

async function press(key: string): Promise<void> {
    keymap.handle(new KeyboardEvent("keydown", {key, cancelable: true}));
    await tick();
}

const marker = (description: string): string | null =>
    screen
        .getAllByRole("row")
        .find((row) => row.textContent?.includes(description))
        ?.getAttribute("data-review-marker") ?? null;

beforeEach(() => {
    resetReviewLaunch();
    Object.defineProperty(HTMLElement.prototype, "clientWidth", {configurable: true, get: () => 1024});
});
afterEach(() => {
    vi.unstubAllGlobals();
    Reflect.deleteProperty(HTMLElement.prototype, "clientWidth");
    keymap.reset();
    settings.reviewMarkers = true;
    settings.reviewMaxDays = 365;
});

describe("COMPONENT TransactionTable review buttons", () => {
    it("shows each queue's count, and opens it from the button and from u / r", async () => {
        await show({unknown: 3, unreviewedRecent: 12});
        render(TransactionTable);
        await tick();
        expect(screen.getByTestId("review-unknowns-count").textContent).toBe("3");
        expect(screen.getByTestId("review-recents-count").textContent).toBe("12");
        await fireEvent.click(screen.getByTestId("review-unknowns-button"));
        expect(reviewLaunch.kind).toBe("unknowns");
        resetReviewLaunch();
        await press("r");
        expect(reviewLaunch.kind).toBe("recents");
        resetReviewLaunch();
        await press("u");
        expect(reviewLaunch.kind).toBe("unknowns");
    });

    it("disables a button at zero, and its key with it", async () => {
        await show({unknown: 0, unreviewedRecent: 4});
        render(TransactionTable);
        await tick();
        expect((screen.getByTestId("review-unknowns-button") as HTMLButtonElement).disabled).toBe(true);
        expect((screen.getByTestId("review-recents-button") as HTMLButtonElement).disabled).toBe(false);
        await press("u");
        expect(reviewLaunch.kind).toBeNull();
        expect(keymap.active.some((binding) => binding.keys === "u")).toBe(false);
        expect(keymap.active.some((binding) => binding.keys === "r")).toBe(true);
    });

    it("asks the summary for the counts with the review settings, and again when they change", async () => {
        settings.reviewRecentLimit = 20;
        await show({unknown: 1, unreviewedRecent: 1});
        render(TransactionTable);
        await tick();
        const first = summaryUrls.at(-1);
        expect(first?.searchParams.get("today")).toBe(TODAY);
        expect(first?.searchParams.get("reviewLimit")).toBe("20");
        expect(first?.searchParams.get("reviewMaxDays")).toBe("365");
        settings.reviewMaxDays = 30;
        await tick();
        await vi.waitFor(() => expect(summaryUrls.at(-1)?.searchParams.get("reviewMaxDays")).toBe("30"));
        settings.reviewRecentLimit = 50;
    });
});

describe("COMPONENT TransactionTable review stripes (D20)", () => {
    it("marks unknown amber and unreviewed faint, inside the window only", async () => {
        await show({unknown: 1, unreviewedRecent: 1});
        render(TransactionTable);
        await tick();
        expect(marker("Coffee")).toBe("unknown");
        expect(marker("Safeway")).toBe("unreviewed");
        expect(marker("Rent")).toBeNull();
        expect(marker("Edge of the window")).toBe("unreviewed");
        expect(marker("Old")).toBeNull();
        expect(marker("Future unknown")).toBeNull();
        const coffee = screen.getAllByRole("row").find((row) => row.textContent?.includes("Coffee")) as HTMLElement;
        expect(coffee.style.getPropertyValue("--row-shadow")).toContain("var(--color-warning) 80%");
    });

    it("follows the window setting with no refetch, and turns off with reviewMarkers", async () => {
        await show({unknown: 1, unreviewedRecent: 1});
        render(TransactionTable);
        await tick();
        settings.reviewMaxDays = 0;
        await tick();
        expect(marker("Old")).toBe("unknown");
        settings.reviewMarkers = false;
        await tick();
        expect(marker("Coffee")).toBeNull();
        expect(marker("Safeway")).toBeNull();
    });
});

describe("COMPONENT TransactionTable after a review run", () => {
    it("puts the cursor on the last reviewed transaction", async () => {
        await show({unknown: 1, unreviewedRecent: 1});
        render(TransactionTable);
        await tick();
        reviewLaunch.close(4);
        await vi.waitFor(() => expect(document.querySelector("[aria-current='true']")?.textContent).toContain("Safeway"));
        expect(reviewLaunch.returnTo).toBeNull();
    });
});
