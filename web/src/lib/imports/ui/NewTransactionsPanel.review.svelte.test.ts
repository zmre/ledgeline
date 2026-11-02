// The New Transactions screen's review wiring (plan 32 Phase 4), end to end
// against a fake engine: a dry run offers Review Unknowns, the overlay opens
// over the staged transactions, "Remember as a rule" checks the rule live and
// saves it before the card, the dry run then re-runs with the card's override,
// and closing leaves the edits counted beside Write changes. The same story
// `e2e/imports.e2e.ts` tells against the real engine.
//
// Its own file: `importStore` and the review run are module singletons.

import {fireEvent, render, screen} from "@testing-library/svelte";
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import {importStore} from "#lib/imports/importStore.svelte.js";
import {keymap} from "#lib/keys/keymap.svelte.js";
import {resetReviewLaunch} from "#lib/review/ui/launch.svelte.js";
import {resetReview, review} from "#lib/stores/review.svelte.js";
import {settings} from "#lib/stores/settings.svelte.js";
import {FAKE_JOURNAL_INFO} from "#lib/testing/fakeEngine.js";
import {CAPABILITIES, STAGE, upload} from "#lib/testing/importFixtures.js";
import {similarWire, stagedTxnWire, txnFixture} from "#lib/testing/reviewFixture.js";
import {accountField, MOD, press, settle} from "#lib/testing/reviewOverlay.js";
import NewTransactionsPanel from "./NewTransactionsPanel.svelte";

const RULES_DOC = {
    id: "import/2026/bank.csv.rules",
    label: "bank",
    revision: "10-abc",
    editable: true,
    newline: "lf",
    settings: {fields: {names: ["date", "description", "amount"], itemId: 0}},
    items: [{id: 0, line: 1, lines: 1, kind: "fields", names: ["date", "description", "amount"]}],
    warnings: [],
};

const hardware = (overridden: boolean) =>
    stagedTxnWire(txnFixture(1, "HARDWARE STORE", overridden ? "expenses:home:supplies" : "expenses:unknown"), {
        ordinal: 0,
        key: "hardware store",
        unknown: !overridden,
        overridden,
        fingerprint: "gen-1",
    });

function dryRunWire(overrides: unknown[] | undefined): Record<string, unknown> {
    const n = overrides?.length ?? 0;
    return {
        ok: true,
        entries:
            n === 0 ? "2024-01-19 HARDWARE STORE\n    expenses:unknown\n" : "2024-01-19 HARDWARE STORE  ; reviewed:2026-10-31\n    expenses:home:supplies\n",
        count: 1,
        status: "would import 1",
        skipped: null,
        balance: null,
        cliCommand: "ledgeline import -i bank.csv",
        idMatches: null,
        txns: [hardware(n > 0)],
        records: {
            aligned: true,
            header: ["Date", "Description", "Amount"],
            fields: ["date", "description", "amount"],
            rows: [["01/19/2024", "HARDWARE STORE", "-62.13"]],
        },
        overrides: n,
        cliNote: n === 0 ? null : `plus ${n} reviewed edit`,
    };
}

interface Seen {
    method: string;
    path: string;
    body: Record<string, unknown> | undefined;
}

function engine(): Seen[] {
    const seen: Seen[] = [];
    const json = (value: unknown): Response => new Response(JSON.stringify(value), {status: 200});
    vi.stubGlobal("fetch", (input: string, init?: RequestInit) => {
        const url = new URL(input);
        const method = init?.method ?? "GET";
        const body = typeof init?.body === "string" ? (JSON.parse(init.body) as Record<string, unknown>) : undefined;
        seen.push({method, path: url.pathname, body});
        const path = url.pathname;
        if (init?.signal?.aborted === true) return Promise.reject(new DOMException("aborted", "AbortError"));
        if (path.endsWith("/version")) return Promise.resolve(json("1.52"));
        if (path.endsWith("/api/journal")) return Promise.resolve(json(FAKE_JOURNAL_INFO));
        if (path.endsWith("/api/diagnostics")) return Promise.resolve(json({count: 0, maxSeverity: null, diagnostics: []}));
        if (path.endsWith("/api/import/capabilities")) return Promise.resolve(json(CAPABILITIES));
        if (path.endsWith("/api/import/stage")) return Promise.resolve(json(STAGE));
        if (path.endsWith("/api/import/dry-run")) return Promise.resolve(json(dryRunWire(body?.overrides as unknown[] | undefined)));
        if (path.endsWith("/api/import/rule-preview")) {
            return Promise.resolve(json({ok: true, matched: 1, total: 1, alreadyCategorised: 0, changed: [], changedTotal: 1}));
        }
        if (path.endsWith("/api/review/similar-key")) return Promise.resolve(json({...similarWire(0, []), key: url.searchParams.get("key")}));
        if (path.includes("/api/rules/")) return Promise.resolve(json(RULES_DOC));
        return Promise.resolve(new Response("no route", {status: 404}));
    });
    return seen;
}

beforeEach(async () => {
    resetReview();
    resetReviewLaunch();
    engine();
    if (settings.serverUrl === null) await settings.setServerUrl("http://engine.test");
    await importStore.reloadCapabilities("http://engine.test");
});
afterEach(() => {
    vi.unstubAllGlobals();
    keymap.reset();
});

describe("COMPONENT NewTransactionsPanel — review before writing", () => {
    it("reviews an unknown, remembers a rule, and leaves the edit for Write changes", async () => {
        const seen = engine();
        render(NewTransactionsPanel);
        await importStore.offerFile(upload("bank.csv"));
        await importStore.runDryRun();
        await vi.waitFor(() => expect(screen.getByTestId("imports-review-unknowns-count").textContent).toBe("1"));

        // `u`, as D17 has it, once the dry run is on screen.
        await press("u");
        await vi.waitFor(() => expect(screen.getByRole("dialog", {name: "Review Unknowns"})).toBeTruthy());
        expect(review.source).toBe("staged");

        await fireEvent.input(accountField(), {target: {value: "expenses:home:supplies"}});
        await fireEvent.click(screen.getByTestId("review-rule-toggle"));
        await vi.waitFor(() => expect(screen.getByTestId("review-rule-count").textContent?.replace(/\s+/g, " ")).toContain("Matches 1 of 1 rows in bank.csv"));
        const preview = seen.find((call) => call.path.endsWith("/api/import/rule-preview"));
        expect(preview?.body).toMatchObject({
            stageId: "stage-1",
            rulesId: "import/2026/bank.csv.rules",
            rule: {matchers: [{field: "description", pattern: "hardware.*store"}], assignments: [{field: "account1", value: "expenses:home:supplies"}]},
        });

        await press("Enter", MOD);
        // The rule went in first, then the dry run re-ran carrying the card's override.
        await vi.waitFor(() => expect(seen.filter((call) => call.path.endsWith("/api/import/dry-run")).at(-1)?.body?.overrides).toBeDefined());
        const put = seen.findIndex((call) => call.method === "PUT" && call.path.includes("/api/rules/"));
        const rerun = seen.findLastIndex((call) => call.path.endsWith("/api/import/dry-run"));
        expect(put).toBeGreaterThan(-1);
        expect(rerun).toBeGreaterThan(put);
        // Nothing was written to the journal by the review itself.
        expect(seen.some((call) => call.path.endsWith("/api/transactions/batch") || call.path.endsWith("/api/import/commit"))).toBe(false);

        await fireEvent.click(screen.getByRole("button", {name: /Done/}));
        await settle();
        expect(screen.queryByTestId("review-overlay")).toBeNull();
        await vi.waitFor(() => expect(screen.getByTestId("imports-review-edits-count").textContent).toBe("1 reviewed edit"));
        expect(screen.getByTestId("imports-cli-note").textContent).toContain("plus 1 reviewed edit");
        // The categorised row no longer counts as unknown; Write changes is there to press.
        expect(screen.getByTestId("imports-review-unknowns-count").textContent).toBe("0");
        expect((screen.getByTestId("imports-write-changes") as HTMLButtonElement).disabled).toBe(false);
    });
});
