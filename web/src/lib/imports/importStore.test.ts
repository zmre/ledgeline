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
// The wire fixtures are shared with the component tests that mount this screen
// — one copy of the engine's contract, asserted on from both projects.
import {CAPABILITIES, STAGE, upload} from "$lib/testing/importFixtures";
import {actionBlocker, importAction} from "./importModel";
import {importStore} from "./importStore.svelte";

const DRY_RUN = {ok: true, entries: "…", count: 3, status: "would import 3", skipped: null, balance: null, blockedByGit: []};

const COMMIT = {csvWritten: "import/2026/bank.csv", journalWritten: "2026/2026.journal", imported: 3, ordering: {inOrder: true, moves: []}, git: null};

const json = (body: unknown): Response => new Response(JSON.stringify(body), {status: 200, headers: {"Content-Type": "application/json"}});

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

/** A response the test releases by hand, so a request can be observed mid-flight. */
function deferred(): {promise: Promise<Response>; release: (response: Response) => void} {
    let release!: (response: Response) => void;
    const promise = new Promise<Response>((resolve) => {
        release = resolve;
    });
    return {promise, release};
}

/** The action button's disabled-reason, computed exactly as `StagedPanel` computes it. */
function blockerNow(): string | null {
    return actionBlocker(importAction(importStore.selectedRulesId), {
        csvPath: importStore.csvPath,
        journalId: importStore.journalId,
        balance: importStore.balance,
        balanceAccount: importStore.balanceAccount,
    });
}

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

describe("UNIT importStore — nothing has been asked for yet", () => {
    // THE regression test for the whole class. All three of this screen's
    // resources sit idle until the user acts, and `dataView` reports idle as
    // "loading" — correctly, for the mount-fetching surfaces it was written for.
    // Reading those views as "busy" is what made the screen open with a spinner
    // in the drop zone, a frozen destination form, and a Save button wearing a
    // spinner nobody had pressed. A fresh module is the only honest way to ask:
    // this file's other tests share one module-level store.
    it("is not loading and not busy before a file has been offered", async () => {
        vi.resetModules();
        const {importStore: fresh} = await import("./importStore.svelte");

        expect(fresh.stagingInFlight).toBe(false);
        expect(fresh.dryRunInFlight).toBe(false);
        expect(fresh.writeInFlight).toBe(false);
        expect(fresh.formBusy).toBe(false);
        // And no section claims a staged file, so nothing consults `stagedView`.
        expect(fresh.hasStagedOutcome).toBe(false);
        expect(fresh.dryRunRequested).toBe(false);
        expect(fresh.writeRequested).toBe(false);
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

describe("UNIT importStore — busy is in flight, and only in flight", () => {
    beforeEach(() => {
        vi.stubGlobal("fetch", routes({...BASE_ROUTES, "/api/import/stage": STAGE}));
    });

    it("spins the drop target only while the upload is running", async () => {
        const gate = deferred();
        vi.stubGlobal(
            "fetch",
            vi.fn((url: string) => (url.endsWith("/stage") ? gate.promise : Promise.resolve(json(CAPABILITIES))))
        );
        const offering = importStore.offerFile(upload("bank.csv"));
        expect(importStore.stagingInFlight).toBe(true);
        gate.release(json(STAGE));
        await offering;
        // The file is on screen now, so the drop zone goes back to inviting the
        // next one instead of claiming to still be reading this one.
        expect(importStore.stagingInFlight).toBe(false);
        expect(importStore.hasStagedOutcome).toBe(true);
    });

    it("freezes the form for the length of a dry run and no longer", async () => {
        await importStore.offerFile(upload("bank.csv"));
        expect(importStore.formBusy).toBe(false);

        const gate = deferred();
        vi.stubGlobal(
            "fetch",
            vi.fn((url: string) => (url.endsWith("/dry-run") ? gate.promise : Promise.resolve(json(CAPABILITIES))))
        );
        const running = importStore.runDryRun();
        expect(importStore.dryRunInFlight).toBe(true);
        expect(importStore.formBusy).toBe(true);
        gate.release(json(DRY_RUN));
        await running;
        // Unfrozen the moment it settles: the dry run is a thing to react to,
        // and reacting means editing a destination.
        expect(importStore.dryRunInFlight).toBe(false);
        expect(importStore.formBusy).toBe(false);
    });

    it("freezes the form for the length of a write and no longer", async () => {
        await importStore.offerFile(upload("bank.csv"));
        const gate = deferred();
        vi.stubGlobal(
            "fetch",
            vi.fn((url: string) => (url.endsWith("/commit") ? gate.promise : Promise.resolve(json(CAPABILITIES))))
        );
        const writing = importStore.writeChanges();
        expect(importStore.writeInFlight).toBe(true);
        expect(importStore.formBusy).toBe(true);
        gate.release(json(COMMIT));
        await writing;
        expect(importStore.writeInFlight).toBe(false);
        expect(importStore.formBusy).toBe(false);
    });

    it("leaves the form editable after a dry run fails — fixing it is the only way out", async () => {
        await importStore.offerFile(upload("bank.csv"));
        vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response("boom", {status: 500})));
        await importStore.runDryRun();
        expect(importStore.dryRunView).toBe("error");
        expect(importStore.formBusy).toBe(false);
    });
});

describe("UNIT importStore — the balance and the account it is a balance of", () => {
    beforeEach(() => {
        vi.stubGlobal("fetch", routes({...BASE_ROUTES, "/api/import/stage": STAGE}));
    });

    it("seeds the account from the chosen candidate, so nothing blocks the button", async () => {
        await importStore.offerFile(upload("bank.csv"));
        expect(importStore.balance).toBe("-3238.65");
        expect(importStore.balanceAccount).toBe("assets:bank:checking");
        expect(blockerNow()).toBeNull();
    });

    it("keeps seeding the account after the amount has been edited", async () => {
        // One `balanceTouched` served both fields, so typing the closing balance
        // off a paper statement permanently stopped the account from ever being
        // filled in — and the form then refused to submit because the account it
        // had just stopped filling in was empty.
        await importStore.offerFile(upload("bank.csv"));
        importStore.setBalance("-3238.66");
        importStore.selectCandidate("import/2026/card.csv.rules");
        expect(importStore.balanceAccount).toBe("liabilities:card:visa");
        expect(blockerNow()).toBeNull();
    });

    it("never re-aims an account the user aimed themselves", async () => {
        await importStore.offerFile(upload("bank.csv"));
        importStore.setBalanceAccount("assets:bank:savings");
        importStore.selectCandidate("import/2026/card.csv.rules");
        expect(importStore.balanceAccount).toBe("assets:bank:savings");
        // Editing the amount is not aiming it, and must not re-latch it either.
        importStore.setBalance("12.00");
        importStore.selectCandidate("import/2026/bank.csv.rules");
        expect(importStore.balanceAccount).toBe("assets:bank:savings");
    });

    it("forgets both when a new file is staged", async () => {
        await importStore.offerFile(upload("bank.csv"));
        importStore.setBalanceAccount("assets:bank:savings");
        await importStore.offerFile(upload("card.csv"));
        expect(importStore.balanceAccount).toBe("assets:bank:checking");
    });

    it("does not block Save CSV over a balance that route cannot carry", async () => {
        // The reported case: an OFX volunteers its closing balance, NO rules
        // file matches (so the action is Save CSV and there is no `account1` to
        // seed from), and the save-csv body is `{stageId, csvPath}` — it carries
        // neither field. The only button that path has must be pressable.
        vi.stubGlobal("fetch", routes({...BASE_ROUTES, "/api/import/stage": {...STAGE, candidates: []}}));
        await importStore.offerFile(upload("bank.csv"));
        expect(importStore.selectedRulesId).toBeNull();
        expect(importStore.balance).toBe("-3238.65");
        expect(importStore.balanceAccount).toBe("");
        expect(blockerNow()).toBeNull();
    });
});

describe("UNIT importStore — a first upload that fails", () => {
    it("says so, and retries the file the user offered", async () => {
        // A FRESH module, for the same reason as the test above: by now the
        // shared store holds a payload AND a query from an earlier success, and
        // it is precisely their absence that used to make this failure silent.
        vi.resetModules();
        vi.stubGlobal("fetch", routes({...BASE_ROUTES, "/api/import/stage": new Response("boom", {status: 500})}));
        const [{importStore: fresh}, {settings: freshSettings}] = await Promise.all([import("./importStore.svelte"), import("$lib/stores/settings.svelte")]);
        await freshSettings.setServerUrl("http://engine.test");
        await fresh.reloadCapabilities("http://engine.test");

        await fresh.offerFile(upload("bank.csv"));
        expect(fresh.rejection).toBeNull();
        expect(fresh.stagingInFlight).toBe(false);
        expect(fresh.staged).toBeNull();
        // `staged.query` is written on SUCCESS only, so a section gated on it
        // showed nothing at all here: the drop target's spinner just stopped.
        expect(fresh.hasStagedOutcome).toBe(true);
        expect(fresh.stagedView).toBe("error");

        vi.stubGlobal("fetch", routes({...BASE_ROUTES, "/api/import/stage": STAGE}));
        // And Retry has a file to retry WITH — it used to read `staged.query`,
        // which is null exactly here, so the button did nothing.
        await fresh.retryStage();
        expect(fresh.staged?.stageId).toBe("stage-1");
        expect(fresh.stagedView).toBe("data");
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
        expect(importStore.sortError).toBeNull();
    });

    it("says so when the re-sort landed but git would not commit it", async () => {
        // The dangerous quiet case: the journal is rewritten, the import's own
        // commit is already in history, and `git revert` of it no longer
        // restores the file. Silence here would be the safety net failing
        // without a word.
        const outOfOrder = {...COMMIT, ordering: {inOrder: false, moves: [{date: "2026-01-20", description: "x", fromLine: 812, toLine: 540}]}};
        const fetchMock = routes({
            ...BASE_ROUTES,
            "/api/import/stage": STAGE,
            "/api/import/dry-run": DRY_RUN,
            "/api/import/commit": outOfOrder,
            "/api/import/sort": {moved: 3, git: {committed: false, paths: [], skipped: ["2026/2026.journal"]}},
        });
        vi.stubGlobal("fetch", fetchMock);
        await importStore.offerFile(upload("bank.csv"));
        await importStore.writeChanges();
        await importStore.resort();
        expect(importStore.sortMoved).toBe(3);
        expect(importStore.sortError).toContain("git did not commit");
        expect(importStore.sortError).toContain("2026/2026.journal");
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
