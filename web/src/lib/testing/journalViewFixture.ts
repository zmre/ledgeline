// Feeding the windowed journal (plan 26 B5) to a mounted component: domain
// transactions served through the fake engine's `/api/journal/rows` and
// `/api/journal/summary`, exactly as the engine would serve them, so a test
// drives the REAL journal view store rather than a mock of it.

import {vi} from "vitest";
import type {Dec} from "#lib/domain/money.js";
import type {Amount, Transaction} from "#lib/domain/types.js";
import type {Problem} from "#lib/checks/engine.js";
import {journalView, resetJournalView} from "#lib/stores/journalView.svelte.js";
import {FAKE_ENGINE, routes} from "./fakeEngine";

const wireDec = (d: Dec): {m: string; p: number} => ({m: d.m.toString(), p: d.p});

function wireAmount(amount: Amount): Record<string, unknown> {
    const out: Record<string, unknown> = {commodity: amount.commodity, qty: wireDec(amount.qty), style: amount.style};
    if (amount.cost !== undefined) out.cost = {commodity: amount.cost.commodity, qty: wireDec(amount.cost.qty), per: amount.cost.per};
    return out;
}

/** A domain transaction as `/api/journal/rows` serves it. */
export function wireRow(txn: Transaction, flags: Pick<Problem, "rule" | "severity" | "message">[] = []): Record<string, unknown> {
    return {
        index: txn.index,
        date: txn.date,
        ...(txn.date2 === undefined ? {} : {date2: txn.date2}),
        status: txn.status,
        code: txn.code,
        description: txn.description,
        comment: txn.comment,
        tags: txn.tags,
        postings: txn.postings.map((posting) => ({
            account: posting.account,
            amounts: posting.amounts.map(wireAmount),
            status: posting.status,
            comment: posting.comment,
            tags: posting.tags,
            ...(posting.date === undefined ? {} : {date: posting.date}),
            ...(posting.type === undefined ? {} : {type: posting.type}),
            ...(posting.balanceAssertion === undefined
                ? {}
                : {
                      balanceAssertion: {
                          amount: wireAmount(posting.balanceAssertion.amount),
                          inclusive: posting.balanceAssertion.inclusive,
                          total: posting.balanceAssertion.total,
                      },
                  }),
        })),
        flags,
    };
}

export const VIEW_GENERATION = "fake-1";

/** An empty-but-valid summary for `total` rows. */
export function summaryBody(total: number, overrides: Record<string, unknown> = {}): Record<string, unknown> {
    return {generation: VIEW_GENERATION, total, visibleNet: [], commodities: [], perCommodity: [], maxDepth: 1, primary: null, ...overrides};
}

/**
 * Stub `fetch` so the windowed routes serve `txns` (already in display order)
 * and point the journal view at them; resolves once the first block is in.
 * `extra` adds or overrides routes. Pair with `afterEach(() => vi.unstubAllGlobals())`.
 */
export async function showRows(txns: readonly Transaction[], extra: Record<string, unknown> = {}): Promise<void> {
    const rows = txns.map((txn) => wireRow(txn));
    const table = routes({"/api/journal/summary": summaryBody(txns.length), ...extra});
    vi.stubGlobal("fetch", (input: RequestInfo | URL) => {
        const url = new URL(String(input));
        if (url.pathname.endsWith("/api/journal/rows")) {
            const offset = Number(url.searchParams.get("offset") ?? 0);
            const limit = Number(url.searchParams.get("limit") ?? 200);
            const ordered = url.searchParams.get("order") === "asc" ? [...rows].reverse() : rows;
            const body = {generation: VIEW_GENERATION, total: rows.length, offset, rows: ordered.slice(offset, offset + limit)};
            return Promise.resolve(new Response(JSON.stringify(body), {status: 200}));
        }
        return table(String(input));
    });
    resetJournalView();
    journalView.sync(FAKE_ENGINE, VIEW_GENERATION, {from: null, to: null, accounts: new Set(), query: ""});
    await vi.waitFor(() => {
        if (journalView.total === null) throw new Error("rows not loaded yet");
    });
}
