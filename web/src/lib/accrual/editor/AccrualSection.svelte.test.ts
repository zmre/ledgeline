// The editor's Accrual section (plan 45 D-editor) on the real overlay against
// a fake engine: a deferral scheduled from the card (its tags on the posting,
// the preview, the file prompt, then the journal save BEFORE the rules save
// with the same id), the renewal banner, a scheduled line's summary and End
// contract's restatement preview, and nothing at all with the gate off.

import {fireEvent, render, screen, within} from "@testing-library/svelte";
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import {txnModal} from "#lib/journal/edit/modalState.svelte.js";
import {keymap} from "#ui/keys/keymap.svelte.js";
import type {Amount, Posting, Transaction} from "#lib/domain/types.js";
import {resetReviewLaunch} from "#lib/review/ui/launch.svelte.js";
import ReviewOverlay from "#lib/review/ui/ReviewOverlay.svelte";
import {editing} from "#lib/stores/editing.svelte.js";
import {journal} from "#lib/stores/journal.svelte.js";
import {resetReview, review} from "#lib/stores/review.svelte.js";
import {connectFakeEngine, FAKE_ENGINE, journalInfoBody} from "#lib/testing/fakeEngine.js";
import {queueItemWire, reviewTxnWire, similarWire, type SeenRequest} from "#lib/testing/reviewFixture.js";
import {settle} from "#lib/testing/reviewOverlay.js";
import {accrual, resetAccrual} from "../store.svelte";

vi.mock("$app/navigation", () => ({goto: vi.fn()}));
vi.mock("$app/paths", () => ({resolve: (path: string) => path}));

const STYLE = {side: "L", spaced: false, precision: 2, decimalPoint: ".", digitGroups: null} as const;
const money = (cents: bigint): Amount => ({commodity: "$", qty: {m: cents, p: 2}, style: STYLE});
const leg = (account: string, cents: bigint, comment = "", tags: [string, string][] = []): Posting => ({
    account,
    amounts: [money(cents)],
    status: "unmarked",
    comment,
    tags,
});
const dec = (cents: number): {m: string; p: number} => ({m: String(cents), p: 2});

/** Customer X prepays a year: cash in, deferred revenue out. */
function prepay(comment = "", tags: [string, string][] = []): Transaction {
    return {
        index: 4,
        date: "2026-11-17",
        status: "unmarked",
        description: "Customer X Payment",
        code: "",
        comment: "",
        tags: [],
        postings: [leg("assets:checking", 1200000n), leg("liabilities:deferred-revenue", -1200000n, comment, tags)],
    };
}

const ACCOUNTS: Record<string, string> = {
    "assets:checking": "cash",
    "liabilities:deferred-revenue": "liability",
    "revenue:sales": "revenue",
};

function statusWire(fileExists: boolean): Record<string, unknown> {
    return {
        enabled: true,
        generation: "g1",
        basis: {value: "accrual", source: "inferred"},
        reason: {kind: "declared", account: "liabilities:deferred-revenue", role: "deferred-revenue"},
        roles: {"liabilities:deferred-revenue": {role: "deferred-revenue", declared: true}},
        file: {path: "recognition.journal", exists: fileExists, included: false},
        counts: {due: 0, dueClosed: 0, needsSchedule: 1, invoices: 0},
        warnings: [],
    };
}

function specWire(id: string, extra: Record<string, unknown> = {}): Record<string, unknown> {
    return {
        id,
        lines: [
            {
                sourceAccount: "liabilities:deferred-revenue",
                targetAccount: "revenue:sales",
                total: {commodity: "$", quantity: dec(-1200000)},
                party: {kind: "customer", name: "Customer X"},
            },
        ],
        start: "2025-11-17",
        term: {months: 12},
        proration: "anniversary",
        description: "Customer X Recognized",
        headerTags: [],
        ...extra,
    };
}

function scheduleWire(id: string, statuses: string[] = []): Record<string, unknown> {
    const dates = ["2026-11-17", "2026-12-17", "2027-01-17"];
    return {
        spec: specWire(id, {
            start: "2026-11-17",
            term: {months: 3},
            lines: [{sourceAccount: "liabilities:deferred-revenue", targetAccount: "revenue:sales", total: {commodity: "$", quantity: dec(-300000)}}],
        }),
        periods: dates.map((date, at) => ({
            date,
            part: "regular",
            amounts: [dec(-100000)],
            ...(statuses[at] === undefined ? {} : {status: statuses[at]}),
        })),
        rules: [
            {
                role: "main",
                period: "~ monthly from 2026-11-17 to 2027-02-17",
                text: "~ monthly from 2026-11-17 to 2027-02-17  Customer X Recognized\n    liabilities:deferred-revenue  $1000.00\n    revenue:sales",
            },
        ],
        renewalDate: "2027-02-17",
    };
}

interface Engine {
    txn: Transaction;
    status?: Record<string, unknown>;
    forBody?: Record<string, unknown>;
    preview?: (body: Record<string, unknown>) => unknown;
}

function serve(engine: Engine): SeenRequest[] {
    const seen: SeenRequest[] = [];
    const json = (value: unknown, status = 200): Promise<Response> => Promise.resolve(new Response(JSON.stringify(value), {status}));
    vi.stubGlobal("fetch", (input: RequestInfo | URL, init?: RequestInit) => {
        const url = new URL(String(input));
        const method = init?.method ?? "GET";
        const body = typeof init?.body === "string" ? (JSON.parse(init.body) as Record<string, unknown>) : undefined;
        seen.push({method, url, body});
        const path = url.pathname;
        if (path.endsWith("/version")) return json("1.52");
        if (path.endsWith("/api/journal")) {
            const accounts = Object.fromEntries(
                Object.entries(ACCOUNTS).map(([name, type]) => [name, {type, typeDeclared: true, term: null, termGuessed: false}])
            );
            return json(journalInfoBody({accounts, generation: "g1"}));
        }
        if (path.endsWith("/api/diagnostics")) return json({count: 0, maxSeverity: null, diagnostics: []});
        if (path.endsWith("/api/payees")) return json({generation: "g1", payees: []});
        if (path.endsWith("/api/tags")) return json({generation: "g1", tags: []});
        if (/\/api\/review\/item\/\d+$/.test(path)) return json({generation: "g1", item: queueItemWire(engine.txn, {unknown: false})});
        if (/\/api\/review\/similar\/\d+$/.test(path)) return json(similarWire(engine.txn.index, []));
        if (path.endsWith("/api/accrual")) return json(engine.status ?? statusWire(false));
        if (/\/api\/recognition\/for\/\d+$/.test(path)) {
            return json(engine.forBody ?? {enabled: true, generation: "g1", schedules: [], unscheduled: [1]});
        }
        if (path.endsWith("/api/recognition/preview") && body !== undefined) {
            if (engine.preview !== undefined) return json(engine.preview(body));
            const spec = (body.schedules as Record<string, unknown>[])[0] ?? {};
            return json({enabled: true, accrualGeneration: "a1", appended: [scheduleWire(String(spec.id))], restatements: [], createsFile: true});
        }
        if (path.endsWith("/api/recognition/save"))
            return json({enabled: true, accrualGeneration: "a2", appended: [], restatements: [], createsFile: true, generation: "g3"});
        if (path.endsWith("/api/transactions/batch") && body !== undefined) {
            return json({generation: "g2", edited: (body.edits as unknown[]).map(() => reviewTxnWire(engine.txn, {unknown: false}))});
        }
        return Promise.resolve(new Response(`no route for ${url}`, {status: 404}));
    });
    return seen;
}

async function openEdit(): Promise<void> {
    await journal.refresh({force: true});
    await accrual.sync(FAKE_ENGINE, "g1");
    txnModal.openEdit(4);
    render(ReviewOverlay, {props: {kind: "edit", index: 4}});
    await vi.waitFor(() => {
        if (review.loading || review.current === null) throw new Error("card not loaded");
    });
    await settle();
}

const section = (): HTMLElement => screen.getByTestId("accrual-section");
const paths = (seen: SeenRequest[]): string[] => seen.map((request) => request.url.pathname);

beforeEach(async () => {
    resetReview();
    resetReviewLaunch();
    resetAccrual();
    txnModal.close();
    editing.clearConflict();
    await connectFakeEngine();
});
afterEach(() => {
    vi.unstubAllGlobals();
    keymap.reset();
});

describe("COMPONENT Accrual section: a deferral to schedule", () => {
    it("tags the posting, previews, asks to create the file, then saves the transaction before the rules, with one id", async () => {
        const seen = serve({txn: prepay()});
        await openEdit();

        expect(section().dataset.open).toBe("");
        await fireEvent.click(within(section()).getByTestId("accrual-recognize"));
        const target = within(section()).getByTestId("accrual-target") as HTMLInputElement;
        await fireEvent.input(target, {target: {value: "revenue:sales"}});
        await vi.waitFor(() => expect(within(section()).getByTestId("accrual-schedule-preview")).toBeTruthy(), {timeout: 3000});
        expect(within(section()).getAllByTestId("accrual-period")).toHaveLength(3);
        expect(within(section()).getByTestId("accrual-rule-text").textContent).toContain("~ monthly from 2026-11-17 to 2027-02-17");

        await fireEvent.click(screen.getByTestId("review-primary"));
        await settle();
        // D1: the file doesn't exist, so nothing is written until the user agrees.
        const prompt = screen.getByTestId("accrual-file-prompt");
        expect(prompt.textContent).toContain("Create recognition.journal beside your journal?");
        expect(paths(seen).some((path) => path.endsWith("/api/transactions/batch"))).toBe(false);

        await fireEvent.click(screen.getByTestId("accrual-file-create"));
        await vi.waitFor(() => expect(paths(seen).some((path) => path.endsWith("/api/recognition/save"))).toBe(true));

        const batchAt = paths(seen).findIndex((path) => path.endsWith("/api/transactions/batch"));
        const saveAt = paths(seen).findIndex((path) => path.endsWith("/api/recognition/save"));
        expect(batchAt).toBeGreaterThanOrEqual(0);
        expect(batchAt).toBeLessThan(saveAt);
        const ops = ((seen[batchAt]?.body as {edits: {ops: {op: string; posting?: number; comment?: string}[]}[]}).edits[0]?.ops ?? []).filter(
            (op) => op.op === "setPostingComment"
        );
        expect(ops).toHaveLength(1);
        const comment = ops[0]?.comment ?? "";
        const id = /recognition: ([a-z0-9]{6})/.exec(comment)?.[1];
        expect(comment).toContain("term-months: 12");
        const saved = seen[saveAt]?.body as {schedules: {id: string; lines: {targetAccount: string; quantity: string}[]}[]; createFile: boolean};
        expect(saved.createFile).toBe(true);
        expect(saved.schedules[0]?.id).toBe(id);
        expect(saved.schedules[0]?.lines[0]).toMatchObject({targetAccount: "revenue:sales", quantity: "-12000.00"});
        // The save landed: the editor flashes its tick and closes.
        await vi.waitFor(() => expect(txnModal.open).toBe(false));
    });

    it("cancelling the file prompt writes nothing", async () => {
        const seen = serve({txn: prepay()});
        await openEdit();
        await fireEvent.click(within(section()).getByTestId("accrual-recognize"));
        await fireEvent.input(within(section()).getByTestId("accrual-target"), {target: {value: "revenue:sales"}});
        await fireEvent.click(screen.getByTestId("review-primary"));
        await settle();
        await fireEvent.click(screen.getByTestId("accrual-file-cancel"));
        await settle();
        expect(screen.queryByTestId("accrual-file-prompt")).toBeNull();
        expect(seen.filter((request) => request.method === "POST" && !request.url.pathname.endsWith("/preview"))).toHaveLength(0);
    });

    it("prefills a renewal from the customer's previous contract, marked as such", async () => {
        serve({
            txn: prepay("customer: Customer X"),
            forBody: {
                enabled: true,
                generation: "g1",
                schedules: [],
                unscheduled: [1],
                renewal: {prefill: specWire("k3d8w1", {term: {months: 12}}), contract: "renewal"},
            },
        });
        await openEdit();
        await vi.waitFor(() => expect(section()).toBeTruthy());
        await fireEvent.click(within(section()).getByTestId("accrual-recognize"));
        expect(within(section()).getByTestId("accrual-renewal-banner").textContent).toBe("Prefilled from Customer X · Nov 2025 · 12 months · k3d8w1");
        expect((within(section()).getByTestId("accrual-target") as HTMLInputElement).value).toBe("revenue:sales");
        const contract = within(section()).getByTestId("accrual-contract") as HTMLSelectElement;
        expect(contract.value).toBe("renewal");
        expect(contract.selectedOptions[0]?.textContent).toBe("renewal (inferred)");
    });
});

describe("COMPONENT Accrual section: a scheduled line", () => {
    const restatement = {
        id: "k7f3a2",
        ruleChange: {before: "~ monthly from 2026-11-17 to 2027-02-17", after: "~ monthly from 2026-11-17 to 2026-12-17"},
        rows: [
            {index: 7, date: "2026-11-17", recognized: "2026-11-17", closed: true, before: [dec(100000)], after: [dec(100000)], action: "kept"},
            {index: 9, date: "2026-12-17", recognized: "2026-12-17", closed: false, before: [dec(100000)], action: "delete"},
        ],
        deletes: [9],
        adjustment: {
            date: "2026-11-30",
            description: "Customer X Recognized",
            tags: [
                ["recognition", "k7f3a2"],
                ["restated", "2026-11-30"],
            ],
            postings: [
                {account: "liabilities:deferred-revenue", amounts: [{commodity: "$", quantity: dec(50000)}]},
                {account: "revenue:sales", amounts: [{commodity: "$", quantity: dec(-50000)}]},
            ],
        },
        totalAfter: [dec(-150000)],
    };

    it("summarizes the schedule and previews End contract as a restatement", async () => {
        const seen = serve({
            txn: prepay("recognition: k7f3a2, term-months: 3", [
                ["recognition", "k7f3a2"],
                ["term-months", "3"],
            ]),
            forBody: {enabled: true, generation: "g1", schedules: [scheduleWire("k7f3a2", ["posted", "due", "future"])], unscheduled: []},
            preview: () => ({enabled: true, accrualGeneration: "a1", appended: [], restatements: [restatement], createsFile: false}),
        });
        await openEdit();
        await fireEvent.click(within(section()).getByRole("button", {name: /Accrual/}));
        await vi.waitFor(() => expect(within(section()).getByTestId("accrual-summary").textContent).toContain("3 × $1,000.00 · 1 posted · 1 due"));

        await fireEvent.click(within(section()).getByTestId("accrual-end-contract"));
        await vi.waitFor(() => expect(within(section()).getByTestId("accrual-restatement")).toBeTruthy(), {timeout: 3000});
        const rows = within(section()).getAllByTestId("accrual-restatement-row");
        expect(rows.map((row) => row.dataset.action)).toEqual(["kept", "delete"]);
        expect(rows[0]?.textContent).toContain("(closed)");
        expect(within(section()).getByTestId("accrual-adjustment").textContent).toContain("Catch-up adjustment 2026-11-30 · restated: 2026-11-30");
        expect(within(section()).getByTestId("accrual-rule-after").textContent).toContain("to 2026-12-17");
        // The restatement preview is asked against the edited source (a replace), so it sees the card's amounts.
        const preview = seen.find((request) => request.url.pathname.endsWith("/api/recognition/preview"));
        expect((preview?.body as {source?: {kind: string; index: number}}).source).toMatchObject({kind: "replace", index: 4});
        expect((preview?.body as {schedules: {id: string; term: unknown}[]}).schedules[0]).toMatchObject({id: "k7f3a2", term: {through: expect.any(String)}});
    });
});

describe("COMPONENT Accrual section: the gate", () => {
    it("shows nothing when the helpers are off", async () => {
        serve({txn: prepay(), status: {enabled: false}});
        await openEdit();
        expect(accrual.enabled).toBe(false);
        expect(screen.queryByTestId("accrual-section")).toBeNull();
    });
});
