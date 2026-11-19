// Wire builders and a fake engine for the accrual recognition UI's tests
// (plan 45 Phase 3). Synthetic names only (Customer X, Northwind, Fabrikam);
// the shapes are exactly Phase 2's routes, so a component test drives the
// REAL `accrual` store through stubbed `fetch`, as the match tests do.

import {vi} from "vitest";
import {editing} from "#lib/stores/editing.svelte.js";
import {journal} from "#lib/stores/journal.svelte.js";
import {connectFakeEngine, journalInfoBody} from "./fakeEngine";
import type {SeenRequest} from "./reviewFixture";

/** `{m, p}`: a quantity in cents by default. */
export function decWire(mantissa: number | string, places = 2): {m: string; p: number} {
    return {m: String(mantissa), p: places};
}

export function moneyWire(cents: number, commodity = "$"): Record<string, unknown> {
    return {commodity, quantity: decWire(cents)};
}

/** `GET /api/accrual` with the helpers on. */
export function accrualStatusWire(overrides: Record<string, unknown> = {}): Record<string, unknown> {
    return {
        enabled: true,
        generation: "g1",
        basis: {value: "accrual", source: "inferred"},
        reason: {kind: "declared", account: "liabilities:deferred-revenue", role: "deferred-revenue"},
        roles: {
            "liabilities:deferred-revenue": {role: "deferred-revenue", declared: true},
            "assets:prepaid": {role: "prepaid", declared: true},
            "assets:receivable": {role: "receivable", declared: false},
        },
        file: {path: "recognition.journal", exists: true, included: false},
        closedThrough: "2026-09-30",
        counts: {due: 3, dueClosed: 1, needsSchedule: 1, invoices: 2, invoicesCheck: 1},
        warnings: [],
        ...overrides,
    };
}

/** One due occurrence of schedule `id`: deferred → revenue for `cents`. */
export function dueEntryWire(id: string, recognized: string, cents: number, description = "Customer X Recognized"): Record<string, unknown> {
    return {
        id,
        recognized,
        description,
        postings: [
            {account: "liabilities:deferred-revenue", amounts: [moneyWire(cents)]},
            {account: "revenue:sales", amounts: [moneyWire(-cents)]},
        ],
    };
}

/** A due month holding `groups` (party name or null → entries). */
export function dueMonthWire(
    month: string,
    groups: {party: string | null; entries: Record<string, unknown>[]}[],
    ties: Record<string, unknown>[] = []
): Record<string, unknown> {
    const entries = groups.flatMap((group) => group.entries);
    const cents = entries.reduce((sum, entry) => sum + Number((entry.postings as {amounts: {quantity: {m: string}}[]}[])[0]?.amounts[0]?.quantity.m ?? "0"), 0);
    return {
        month,
        count: entries.length,
        totals: [moneyWire(cents)],
        groups: groups.map((group) => ({...(group.party === null ? {} : {party: {kind: "customer", name: group.party}}), entries: group.entries})),
        ties,
    };
}

export function tieWire(ties: boolean, overrides: Record<string, unknown> = {}): Record<string, unknown> {
    return {
        account: "liabilities:deferred-revenue",
        commodity: "$",
        before: decWire(-1200000),
        after: decWire(-1100000),
        scheduled: decWire(ties ? -1100000 : -1050000),
        ties,
        ...overrides,
    };
}

/** `GET /api/recognition/due`. */
export function dueWire(
    months: Record<string, unknown>[],
    closed: Record<string, unknown>[] = [],
    overrides: Record<string, unknown> = {}
): Record<string, unknown> {
    return {
        enabled: true,
        generation: "g1",
        today: "2026-11-17",
        closedThrough: "2026-09-30",
        months,
        closed: {count: closed.length, totals: closed.length === 0 ? [] : [moneyWire(83333 * closed.length)], entries: closed},
        ...overrides,
    };
}

/** One side of a settlement (or a Needs-a-schedule deferral). */
export function sideWire(index: number, overrides: Record<string, unknown> = {}): Record<string, unknown> {
    return {
        index,
        posting: 1,
        fingerprint: `fp-${index}`,
        date: "2026-10-15",
        description: `Fabrikam Invoice ${index}`,
        account: "assets:receivable",
        amount: moneyWire(100000),
        invoice: String(2000 + index),
        party: {kind: "customer", name: "Fabrikam"},
        ...overrides,
    };
}

export function proposalWire(
    invoice: Record<string, unknown>,
    payment: Record<string, unknown>,
    overrides: Record<string, unknown> = {}
): Record<string, unknown> {
    return {invoice, payment, fit: {kind: "exact"}, byNumber: true, byParty: true, gapDays: 12, ambiguous: false, clear: true, ...overrides};
}

/** `GET /api/settlements`. */
export function settlementsWire(
    proposals: Record<string, unknown>[],
    open: {invoices?: unknown[]; payments?: unknown[]} = {},
    generation = "g1"
): Record<string, unknown> {
    const clear = proposals.filter((p) => p.clear === true).length;
    return {
        enabled: true,
        generation,
        counts: {clear, check: proposals.length - clear},
        proposals,
        open: {invoices: open.invoices ?? [], payments: open.payments ?? []},
    };
}

/** `GET /api/recognition/needs`. */
export function needsWire(items: Record<string, unknown>[], lookback = 90): Record<string, unknown> {
    return {enabled: true, generation: "g1", lookback, items};
}

/** What the fake engine answers; mutate between steps. A route set to a `{status, error}` answers that once. */
export interface AccrualServe {
    status: Record<string, unknown>;
    due: Record<string, unknown>;
    needs: Record<string, unknown>;
    settlements: Record<string, unknown>;
    /** The next write's failure, if any (a 409, a 400). */
    failNext: {status: number; error: string} | null;
    generation: number;
}

/**
 * Point the real stores at a fake engine answering the accrual routes from
 * `state` (read at request time), recording every request. Writes bump the
 * generation; `/api/recognition/post` answers what it was asked to post.
 */
export async function serveAccrual(initial: Partial<AccrualServe> = {}): Promise<{seen: SeenRequest[]; state: AccrualServe}> {
    await connectFakeEngine();
    const seen: SeenRequest[] = [];
    const off = {enabled: false};
    const state: AccrualServe = {
        status: accrualStatusWire(),
        due: dueWire([]),
        needs: needsWire([]),
        settlements: settlementsWire([]),
        failNext: null,
        generation: 1,
        ...initial,
    };
    vi.stubGlobal("fetch", (input: RequestInfo | URL, init?: RequestInit) => {
        const url = new URL(String(input));
        const method = init?.method ?? "GET";
        const body = typeof init?.body === "string" ? (JSON.parse(init.body) as Record<string, unknown>) : undefined;
        seen.push({method, url, body});
        const json = (value: unknown, status = 200) => Promise.resolve(new Response(JSON.stringify(value), {status}));
        const path = url.pathname;
        const g = `g${state.generation}`;
        const withGen = (value: Record<string, unknown>) => (value.enabled === false ? value : {...value, generation: g});
        if (path.endsWith("/version")) return Promise.resolve(new Response("1.52"));
        if (path.endsWith("/api/transactions")) return Promise.resolve(new Response("", {status: 405}));
        if (path.endsWith("/api/journal")) return json(journalInfoBody({generation: g}));
        if (path.endsWith("/api/diagnostics")) return json({count: 0, maxSeverity: null, diagnostics: []});
        if (method === "GET") {
            if (path.endsWith("/api/accrual")) return json(withGen(state.status));
            if (path.endsWith("/api/recognition/due")) return json(state.status.enabled === false ? off : withGen(state.due));
            if (path.endsWith("/api/recognition/needs")) return json(state.status.enabled === false ? off : withGen(state.needs));
            if (path.endsWith("/api/settlements")) return json(state.status.enabled === false ? off : withGen(state.settlements));
        }
        if (method === "POST" && body !== undefined) {
            if (state.failNext !== null) {
                const {status, error} = state.failNext;
                state.failNext = null;
                if (status === 409) state.generation += 1;
                return Promise.resolve(new Response(error, {status}));
            }
            state.generation += 1;
            const next = `g${state.generation}`;
            if (path.endsWith("/api/recognition/post")) {
                return json({
                    enabled: true,
                    generation: next,
                    posted: (body.entries as unknown[] | undefined) ?? [],
                    adjustments: body.includeClosed === true ? ["cx1217"] : [],
                });
            }
            if (path.endsWith("/api/settlements/link")) return json({generation: next, matchId: "m1"});
            if (path.endsWith("/api/settlements/link-all")) return json({generation: next, linked: 1, matchIds: ["m1"]});
            if (path.endsWith("/api/books/closed-through")) return json({generation: next, closedThrough: body.date ?? null});
            if (path.endsWith("/api/recognition/file")) return json({enabled: true, generation: next, path: "recognition.journal"});
        }
        return Promise.resolve(new Response(`no route for ${url}`, {status: 404}));
    });
    await editing.probe();
    await journal.refresh({force: true});
    return {seen, state};
}
