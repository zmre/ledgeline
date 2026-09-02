// Mounting the QuickBooks Journal import panel directly — the unmapped-account
// resolution flow and the commit flow, WP-17 Phase C's own component-test
// requirements.
//
// The store is real (`qbJournalStore`, driven through a stubbed `fetch`), for
// the reason `AliasPanel.svelte.test.ts` gives for the same choice: a mocked
// store would prove the panel renders whatever it is handed, and what
// actually breaks in a screen like this is what it does with a real response
// — a mapping save that does not reuse the alias wire correctly, a commit
// whose 400 refusal never reaches the screen, a re-sort offered for a file
// that already sorted.
//
// EVERY test uses its own `stageId`. `qbJournalStore` is a module singleton
// shared by every test in this FILE, and `ensurePreview` fetches once per
// stage by design (`qbJournalStore.test.ts`'s own coverage) — a shared id
// across tests would have the second test silently reading the first test's
// cached preview instead of the response its own fetch stub describes.

import {settings} from "$lib/stores/settings.svelte";
import {connectFakeEngine, FAKE_ENGINE} from "$lib/testing/fakeEngine";
import {fireEvent, render, screen} from "@testing-library/svelte";
import {afterEach, describe, expect, it, vi} from "vitest";
import QbJournalPanel from "./QbJournalPanel.svelte";

const json = (body: unknown): Response => new Response(JSON.stringify(body), {status: 200, headers: {"Content-Type": "application/json"}});

type FetchCall = [string, RequestInit | undefined];

/** One unmapped-account preview, for `stageId`. */
function previewUnmapped(stageId: string): Record<string, unknown> {
    return {
        stageId,
        transactionCount: 2,
        postingCount: 4,
        dateFormat: {format: "%m/%d/%Y", ambiguous: false},
        unmappedAccounts: ["3000 Member Equity"],
        sample: [{id: "441", date: "2026-01-05", description: "Deposit", postings: ["assets:bank:checking  1000.00", "3000 Member Equity  -1000.00"]}],
        idMatches: null,
    };
}

/** The same preview once every account has an alias, so a commit may run. */
function previewMapped(stageId: string): Record<string, unknown> {
    return {
        ...previewUnmapped(stageId),
        unmappedAccounts: [],
        idMatches: {new: 2, unchanged: 0, conflicting: [], conflictingTotal: 0},
    };
}

/** `GET /api/aliases` — one writable file with nothing declared yet. */
const ALIASES = {
    editable: true,
    files: [{journalId: "main.journal", label: "main.journal", revision: "rev-1", writable: true, aliases: []}],
};

afterEach(() => vi.unstubAllGlobals());

describe("COMPONENT QbJournalPanel — the export instructions", () => {
    it("tells the user where the export comes from and that re-downloading is safe", async () => {
        await connectFakeEngine({"/api/import/qb-journal/help-1": previewUnmapped("help-1")});

        render(QbJournalPanel, {props: {stageId: "help-1", accountNames: []}});

        await vi.waitFor(() => expect(screen.getByTestId("qb-export-help")).toBeTruthy());
        const help = screen.getByTestId("qb-export-help").textContent ?? "";
        expect(help).toMatch(/Reports.*Journal/);
        expect(help).toMatch(/Export to Excel/);
        expect(help).toMatch(/safe/i);
    });
});

describe("COMPONENT QbJournalPanel — resolving unmapped accounts", () => {
    it("lists every unmapped account and keeps the Import button disabled while any remain", async () => {
        await connectFakeEngine({"/api/import/qb-journal/unmapped-1": previewUnmapped("unmapped-1")});

        render(QbJournalPanel, {props: {stageId: "unmapped-1", accountNames: []}});

        await vi.waitFor(() => expect(screen.getByTestId("qb-unmapped")).toBeTruthy());
        expect(screen.getByDisplayValue("3000 Member Equity")).toBeDefined();
        expect((screen.getByTestId("qb-commit") as HTMLButtonElement).disabled).toBe(true);
        expect(screen.getByText("Map every account above before importing.")).toBeDefined();
    });

    it("submits a typed mapping through the existing alias wire and removes the account once it resolves", async () => {
        const stageId = "unmapped-2";
        const fetchMock = vi.fn((url: string) => {
            if (url.endsWith("/version")) return Promise.resolve(json("1.52"));
            if (url.endsWith("/api/aliases/main.journal")) return Promise.resolve(json({...ALIASES.files[0], revision: "rev-2"}));
            if (url.endsWith("/api/aliases")) return Promise.resolve(json(ALIASES));
            if (url.endsWith(`/qb-journal/${stageId}`)) return Promise.resolve(json(previewUnmapped(stageId)));
            return Promise.resolve(new Response("no route", {status: 404}));
        });
        vi.stubGlobal("fetch", fetchMock);
        if (settings.serverUrl === null) await settings.setServerUrl(FAKE_ENGINE);

        render(QbJournalPanel, {props: {stageId, accountNames: []}});
        await vi.waitFor(() => expect(screen.getByTestId("qb-unmapped")).toBeTruthy());

        const input = screen.getByRole("combobox", {name: "Account"}) as HTMLInputElement;
        await fireEvent.input(input, {target: {value: "equity:opening"}});

        // From here the preview must answer MAPPED — the account resolved.
        fetchMock.mockImplementation((url: string) => {
            if (url.endsWith("/version")) return Promise.resolve(json("1.52"));
            if (url.endsWith("/api/aliases/main.journal")) return Promise.resolve(json({...ALIASES.files[0], revision: "rev-2"}));
            if (url.endsWith("/api/aliases")) return Promise.resolve(json(ALIASES));
            if (url.endsWith(`/qb-journal/${stageId}`)) return Promise.resolve(json(previewMapped(stageId)));
            return Promise.resolve(new Response("no route", {status: 404}));
        });

        await fireEvent.click(screen.getByTestId("qb-map-accounts"));

        await vi.waitFor(() => expect(screen.queryByTestId("qb-unmapped")).toBeNull());
        // The alias write went to the EXISTING wire, with a plain (non-regex)
        // append naming the QuickBooks account as the pattern.
        const saveCall = (fetchMock.mock.calls as unknown as FetchCall[]).find(([url]) => url.endsWith("/api/aliases/main.journal"));
        expect(saveCall).toBeDefined();
        const body = JSON.parse(saveCall?.[1]?.body as string);
        expect(body).toEqual({
            revision: "rev-1",
            edits: [{kind: "append", pattern: "3000 Member Equity", replacement: "equity:opening", regex: false}],
        });
        expect((screen.getByTestId("qb-commit") as HTMLButtonElement).disabled).toBe(false);
    });

    it("reuses the alias-editing wire, not a second one — no request reaches it before anything is typed", async () => {
        const stageId = "unmapped-3";
        const fetchMock = vi.fn((url: string) => {
            if (url.endsWith("/version")) return Promise.resolve(json("1.52"));
            if (url.endsWith("/api/aliases")) return Promise.resolve(json(ALIASES));
            if (url.endsWith(`/qb-journal/${stageId}`)) return Promise.resolve(json(previewUnmapped(stageId)));
            return Promise.resolve(new Response("no route", {status: 404}));
        });
        vi.stubGlobal("fetch", fetchMock);
        if (settings.serverUrl === null) await settings.setServerUrl(FAKE_ENGINE);

        render(QbJournalPanel, {props: {stageId, accountNames: []}});
        await vi.waitFor(() => expect(screen.getByTestId("qb-unmapped")).toBeTruthy());

        // Nothing typed yet — pressing "Map accounts" must not reach the network at all.
        await fireEvent.click(screen.getByTestId("qb-map-accounts"));

        expect(fetchMock.mock.calls.some(([url]) => (url as string).includes("/api/aliases/"))).toBe(false);
        expect(screen.getByTestId("qb-mapping-error").textContent).toMatch(/type an account/i);
    });
});

describe("COMPONENT QbJournalPanel — the commit flow", () => {
    it("shows the imported count once the write succeeds", async () => {
        const stageId = "commit-1";
        const idMatches = {new: 2, unchanged: 0, conflicting: [], conflictingTotal: 0};
        const commit = {imported: 2, idMatches, ordering: {inOrder: true, files: []}, git: null};
        await connectFakeEngine({
            [`/api/import/qb-journal/${stageId}`]: {...previewMapped(stageId), idMatches},
            "/api/import/qb-journal/commit": commit,
        });

        render(QbJournalPanel, {props: {stageId, accountNames: []}});
        await vi.waitFor(() => expect((screen.getByTestId("qb-commit") as HTMLButtonElement).disabled).toBe(false));

        await fireEvent.click(screen.getByTestId("qb-commit"));

        await vi.waitFor(() => expect(screen.getByTestId("qb-result")).toBeTruthy());
        expect(screen.getByTestId("qb-imported").textContent).toContain("2 transactions");
        expect(screen.queryByTestId("qb-out-of-order")).toBeNull();
    });

    it("offers a re-sort when the write leaves a file out of order, and reports how many moved once pressed", async () => {
        const stageId = "commit-2";
        const idMatches = {new: 1, unchanged: 0, conflicting: [], conflictingTotal: 0};
        const commitOutOfOrder = {
            imported: 1,
            idMatches,
            ordering: {
                inOrder: false,
                files: [{journalId: "main.journal", inOrder: false, moves: [{date: "2020-01-01", description: "Old entry", fromLine: 40, toLine: 4}]}],
            },
            git: null,
        };
        const fetchMock = vi.fn((url: string) => {
            if (url.endsWith("/version")) return Promise.resolve(json("1.52"));
            if (url.endsWith(`/qb-journal/${stageId}`)) return Promise.resolve(json({...previewMapped(stageId), idMatches}));
            if (url.endsWith("/qb-journal/commit")) return Promise.resolve(json(commitOutOfOrder));
            if (url.endsWith("/api/import/sort")) return Promise.resolve(json({moved: 1, git: null}));
            return Promise.resolve(new Response("no route", {status: 404}));
        });
        vi.stubGlobal("fetch", fetchMock);
        if (settings.serverUrl === null) await settings.setServerUrl(FAKE_ENGINE);

        render(QbJournalPanel, {props: {stageId, accountNames: []}});
        await vi.waitFor(() => expect((screen.getByTestId("qb-commit") as HTMLButtonElement).disabled).toBe(false));

        await fireEvent.click(screen.getByTestId("qb-commit"));
        await vi.waitFor(() => expect(screen.getByTestId("qb-out-of-order")).toBeTruthy());
        expect(screen.getByText(/main\.journal is no longer in date order/)).toBeDefined();

        await fireEvent.click(screen.getByTestId("qb-sort"));

        await vi.waitFor(() => expect(screen.getByTestId("qb-sorted")).toBeTruthy());
        expect(screen.getByTestId("qb-sorted").textContent).toContain("1 transaction");
        expect(screen.queryByTestId("qb-out-of-order")).toBeNull();
    });

    it("shows the server's 400 refusal naming every unmapped account rather than crashing", async () => {
        const stageId = "commit-3";
        const refusal = "these QuickBooks accounts have no matching alias, so nothing was written: 3000 Member Equity";
        const idMatches = {new: 0, unchanged: 0, conflicting: [], conflictingTotal: 0};
        const fetchMock = vi.fn((url: string) => {
            if (url.endsWith("/version")) return Promise.resolve(json("1.52"));
            if (url.endsWith(`/qb-journal/${stageId}`)) return Promise.resolve(json({...previewMapped(stageId), idMatches}));
            if (url.endsWith("/qb-journal/commit")) return Promise.resolve(new Response(refusal, {status: 400}));
            return Promise.resolve(new Response("no route", {status: 404}));
        });
        vi.stubGlobal("fetch", fetchMock);
        if (settings.serverUrl === null) await settings.setServerUrl(FAKE_ENGINE);

        render(QbJournalPanel, {props: {stageId, accountNames: []}});
        await vi.waitFor(() => expect((screen.getByTestId("qb-commit") as HTMLButtonElement).disabled).toBe(false));

        await fireEvent.click(screen.getByTestId("qb-commit"));

        await vi.waitFor(() => expect(screen.getByTestId("qb-commit-error")).toBeTruthy());
        expect(screen.getByTestId("qb-commit-error").textContent).toContain("3000 Member Equity");
    });
});
