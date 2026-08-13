// The New Transactions store, driven end to end against injected responses.
//
// There is no server to talk to — the Rust half of lane E is being built
// concurrently — so the whole flow is exercised by stubbing the global `fetch`
// with the contract's own wire JSON, which is the same seam `native.test.ts`
// already uses. A `.svelte.ts` module is importable from a plain `.test.ts`
// (see `stores/resource.test.ts`), so the runes work under the node project.
//
// What this covers is the WIRING, not the decisions: the decisions all live in
// `importModel.ts` and are tested in `importModel.test.ts`. What can only go
// wrong here is sequencing — seeding the form from the wrong answer, letting a
// superseded payload render, forgetting to invalidate a dry run when the
// destination moved.

import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import {settings} from "$lib/stores/settings.svelte";
import {importStore} from "./importStore.svelte";

const CAPABILITIES = {
    hledger: {available: true, version: "1.52"},
    formats: ["csv", "ofx"],
    journals: [
        {id: "2026/2026.journal", label: "2026.journal", txnCount: 412, lastTxnDate: "2026-08-01", isRoot: false, writable: true},
        {id: "2025/2025.journal", label: "2025.journal", txnCount: 900, lastTxnDate: "2025-12-31", isRoot: false, writable: true},
    ],
    git: {available: true, autocommit: true},
    editable: true,
};

const STAGE = {
    stageId: "stage-1",
    format: "csv",
    preview: {header: ["date"], rows: [["2026-06-24"]], rowCount: 1, truncated: false},
    statement: {ledgerBalance: "-3238.65"},
    notes: [],
    candidates: [
        {
            id: "import/2026/bank.csv.rules",
            label: "bank",
            score: 0.98,
            signals: {txns: 1, postings: 2, amountlessPostings: 0, bareCommodityAmounts: 0, unknownAccounts: 0},
            sample: [],
        },
        {
            id: "import/2026/card.csv.rules",
            label: "card",
            score: 0.4,
            signals: {txns: 1, postings: 2, amountlessPostings: 0, bareCommodityAmounts: 0, unknownAccounts: 0},
            sample: [],
        },
    ],
    defaults: {csvPath: "import/2026/whatever.csv", journalId: "2026/2026.journal"},
};

const DRY_RUN = {ok: true, entries: "…", count: 3, status: "would import 3", skipped: null, balance: null, blockedByGit: []};

const COMMIT = {csvWritten: "import/2026/bank.csv", journalWritten: "2026/2026.journal", imported: 3, ordering: {inOrder: true, moves: []}, git: null};

const json = (body: unknown): Response => new Response(JSON.stringify(body), {status: 200, headers: {"Content-Type": "application/json"}});

/** A `File` without a DOM: node's `File` is enough for `.name` and `.arrayBuffer()`. */
const upload = (name: string): File => new File(["date\n2026-06-24\n"], name, {type: "text/csv"});

type FetchCall = [string, RequestInit | undefined];

/**
 * Route the stubbed fetch by URL suffix, so one mock serves a whole flow.
 *
 * `/version` is always answered because `settings.setServerUrl` verifies it
 * before it will persist an address.
 */
function routes(table: Record<string, unknown>): ReturnType<typeof vi.fn> {
    return vi.fn((url: string) => {
        const key = Object.keys(table).find((route) => url.endsWith(route));
        const body = key === undefined ? undefined : table[key];
        if (body === undefined) return Promise.resolve(new Response("no route", {status: 404}));
        return Promise.resolve(body instanceof Response ? body : json(body));
    });
}

const BASE_ROUTES = {"/version": "1.52", "/api/import/capabilities": CAPABILITIES};

/** The (url, init) pairs a stubbed fetch saw, for asserting on bodies and headers. */
function calls(mock: ReturnType<typeof vi.fn>): FetchCall[] {
    return mock.mock.calls as FetchCall[];
}

function bodyOf(mock: ReturnType<typeof vi.fn>, suffix: string): unknown {
    const call = calls(mock).find(([url]) => url.endsWith(suffix));
    if (call === undefined) throw new Error(`no request to ${suffix}`);
    return JSON.parse(call[1]?.body as string);
}

beforeEach(async () => {
    vi.stubGlobal("fetch", routes(BASE_ROUTES));
    if (settings.serverUrl === null) await settings.setServerUrl("http://engine.test");
    await importStore.reloadCapabilities("http://engine.test");
});

afterEach(() => vi.unstubAllGlobals());

describe("UNIT importStore — capabilities", () => {
    it("decodes the probe and exposes it as data", () => {
        expect(importStore.capabilitiesView).toBe("data");
        expect(importStore.capabilities?.hledger.version).toBe("1.52");
    });

    it("reports a probe failure as an error surface rather than an empty screen", async () => {
        vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response("boom", {status: 500})));
        await importStore.reloadCapabilities("http://engine.test");
        expect(importStore.capabilitiesView).toBe("error");
        expect(importStore.capabilitiesError).not.toBeNull();
    });
});

describe("UNIT importStore — staging seeds the form", () => {
    beforeEach(() => {
        vi.stubGlobal("fetch", routes({...BASE_ROUTES, "/api/import/stage": STAGE}));
    });

    it("selects the top-ranked candidate and derives the CSV path from IT, not from the default", async () => {
        await importStore.offerFile(upload("bank.csv"));
        expect(importStore.selectedRulesId).toBe("import/2026/bank.csv.rules");
        expect(importStore.csvPath).toBe("import/2026/bank.csv");
        expect(importStore.journalId).toBe("2026/2026.journal");
    });

    it("prefills the balance from the statement the format volunteered", async () => {
        await importStore.offerFile(upload("bank.csv"));
        expect(importStore.balance).toBe("-3238.65");
    });

    it("follows the chosen candidate's CSV path until the user types one", async () => {
        await importStore.offerFile(upload("bank.csv"));
        importStore.selectCandidate("import/2026/card.csv.rules");
        expect(importStore.csvPath).toBe("import/2026/card.csv");
        importStore.setCsvPath("mine/statement.csv");
        importStore.selectCandidate("import/2026/bank.csv.rules");
        // A hand-chosen destination survives: it decides which `.latest` state
        // file the next import reads.
        expect(importStore.csvPath).toBe("mine/statement.csv");
    });

    it("refuses a PDF without uploading it", async () => {
        const fetchMock = routes({...BASE_ROUTES, "/api/import/stage": STAGE});
        vi.stubGlobal("fetch", fetchMock);
        await importStore.offerFile(upload("statement.pdf"));
        expect(importStore.rejection).toMatch(/PDF/);
        expect(calls(fetchMock).some(([url]) => url.endsWith("/stage"))).toBe(false);
    });

    it("sends the filename as a bare, header-safe name", async () => {
        const fetchMock = routes({...BASE_ROUTES, "/api/import/stage": STAGE});
        vi.stubGlobal("fetch", fetchMock);
        await importStore.offerFile(upload("../relevé.csv"));
        const stage = calls(fetchMock).find(([url]) => url.endsWith("/stage"));
        expect(stage?.[1]?.headers as Record<string, string>).toMatchObject({"X-Ledgeline-Filename": "relev_.csv"});
    });
});

describe("UNIT importStore — a stale answer can never render", () => {
    it("stops showing a dry run the moment the destination it answered for changes", async () => {
        vi.stubGlobal("fetch", routes({...BASE_ROUTES, "/api/import/stage": STAGE, "/api/import/dry-run": DRY_RUN}));
        await importStore.offerFile(upload("bank.csv"));
        await importStore.runDryRun();
        expect(importStore.dryRunView).toBe("data");

        importStore.setJournalId("2025/2025.journal");
        // The payload is still held — it is simply no longer an answer to the
        // question on screen, which is the FE-1 distinction.
        expect(importStore.dryRun).not.toBeNull();
        expect(importStore.dryRunView).toBe("loading");
        expect(importStore.dryRunRequested).toBe(false);
    });

    it("does not render the previous file's preview while a second upload is in flight", async () => {
        vi.stubGlobal("fetch", routes({...BASE_ROUTES, "/api/import/stage": STAGE}));
        await importStore.offerFile(upload("bank.csv"));
        expect(importStore.stagedView).toBe("data");

        let release!: (response: Response) => void;
        const pending = new Promise<Response>((resolve) => {
            release = resolve;
        });
        vi.stubGlobal(
            "fetch",
            vi.fn((url: string) => (url.endsWith("/stage") ? pending : Promise.resolve(json(CAPABILITIES))))
        );

        const second = importStore.offerFile(upload("card.csv"));
        expect(importStore.stagedView).toBe("loading");
        release(json({...STAGE, stageId: "stage-2"}));
        await second;
        expect(importStore.staged?.stageId).toBe("stage-2");
        expect(importStore.stagedView).toBe("data");
    });
});

describe("UNIT importStore — writing", () => {
    it("carries writeAssertion into the commit body and nothing else new", async () => {
        const fetchMock = routes({...BASE_ROUTES, "/api/import/stage": STAGE, "/api/import/dry-run": DRY_RUN, "/api/import/commit": COMMIT});
        vi.stubGlobal("fetch", fetchMock);
        await importStore.offerFile(upload("bank.csv"));
        importStore.setBalance("100.00");
        importStore.setBalanceAccount("assets:bank:checking");
        await importStore.writeChanges();
        expect(bodyOf(fetchMock, "/commit")).toEqual({
            stageId: "stage-1",
            rulesId: "import/2026/bank.csv.rules",
            csvPath: "import/2026/bank.csv",
            journalId: "2026/2026.journal",
            balance: "100.00",
            balanceAccount: "assets:bank:checking",
            writeAssertion: true,
        });
    });

    it("takes the save-csv route, with a two-field body, when no rules file is chosen", async () => {
        const fetchMock = routes({...BASE_ROUTES, "/api/import/stage": STAGE, "/api/import/save-csv": {csvWritten: "import/2026/whatever.csv", git: null}});
        vi.stubGlobal("fetch", fetchMock);
        await importStore.offerFile(upload("bank.csv"));
        importStore.selectCandidate(null);
        await importStore.writeChanges();

        // A different ROUTE, not a commit with null handles: a dry-run with no
        // rules file has nothing to propose, so the engine's dry-run/commit body
        // has no way to say "no rules file" at all.
        expect(bodyOf(fetchMock, "/save-csv")).toEqual({stageId: "stage-1", csvPath: "import/2026/whatever.csv"});
        expect(calls(fetchMock).some(([url]) => url.endsWith("/commit"))).toBe(false);
        expect(importStore.committed?.csvWritten).toBe("import/2026/whatever.csv");
        // Nothing was imported, and the result says so rather than reporting 0
        // transactions into a journal that was never opened.
        expect(importStore.committed?.journalWritten).toBeNull();
        expect(importStore.committedView).toBe("data");
    });

    it("never dry-runs when there is no rules file to run", async () => {
        const fetchMock = routes({...BASE_ROUTES, "/api/import/stage": STAGE, "/api/import/dry-run": DRY_RUN});
        vi.stubGlobal("fetch", fetchMock);
        await importStore.offerFile(upload("bank.csv"));
        importStore.selectCandidate(null);
        await importStore.runDryRun();
        expect(calls(fetchMock).some(([url]) => url.endsWith("/dry-run"))).toBe(false);
    });

    it("re-sorts the journal it actually wrote", async () => {
        const outOfOrder = {...COMMIT, ordering: {inOrder: false, moves: [{date: "2026-01-20", description: "x", fromLine: 812, toLine: 540}]}};
        const fetchMock = routes({
            ...BASE_ROUTES,
            "/api/import/stage": STAGE,
            "/api/import/dry-run": DRY_RUN,
            "/api/import/commit": outOfOrder,
            "/api/import/sort": {moved: 3},
        });
        vi.stubGlobal("fetch", fetchMock);
        await importStore.offerFile(upload("bank.csv"));
        await importStore.writeChanges();
        await importStore.resort();
        expect(bodyOf(fetchMock, "/sort")).toEqual({journalId: "2026/2026.journal"});
        expect(importStore.sortMoved).toBe(3);
    });
});

describe("UNIT importStore — the hledger path control", () => {
    it("stores the path and re-probes, so the banner disappears on its own", async () => {
        const unavailable = {...CAPABILITIES, hledger: {available: false, reason: "notFound", message: "hledger was not found"}};
        vi.stubGlobal("fetch", routes({...BASE_ROUTES, "/api/import/capabilities": unavailable}));
        await importStore.reloadCapabilities("http://engine.test");
        expect(importStore.capabilities?.hledger.available).toBe(false);

        vi.stubGlobal("fetch", routes({...BASE_ROUTES, "/api/prefs": {hledgerPath: "/opt/hledger", gitAutocommit: null}}));
        await expect(importStore.saveHledgerPath("/opt/hledger")).resolves.toBe(true);
        expect(importStore.capabilities?.hledger.available).toBe(true);
        expect(importStore.prefs?.hledgerPath).toBe("/opt/hledger");
    });

    it("shows the engine's own refusal when it rejects the path", async () => {
        vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response("/nope is not an executable file", {status: 400})));
        await expect(importStore.saveHledgerPath("/nope")).resolves.toBe(false);
        expect(importStore.prefsError).toBe("/nope is not an executable file");
    });
});
