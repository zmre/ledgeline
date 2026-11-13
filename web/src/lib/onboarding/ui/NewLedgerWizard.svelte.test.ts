// The File → New wizard, mounted over a fake engine: the real store, the real
// requests, literal wire JSON back. What matters: the fresh path walks to a
// create whose body carries the reviewed chart; a folder that isn't empty
// blocks the create; a browser without the desktop app is told so; the sample
// link opens the sample.

import {connectFakeEngine} from "#lib/testing/fakeEngine.js";
import {fireEvent, render, screen} from "@testing-library/svelte";
import {afterEach, describe, expect, it, vi} from "vitest";
import NewLedgerWizard from "./NewLedgerWizard.svelte";

type Sent = {method: string; path: string; body: unknown};

const TEMPLATE = {
    accounts: [
        {name: "assets", tags: [{name: "type", value: "A"}]},
        {name: "assets:bank", tags: [{name: "type", value: "C"}]},
        {name: "expenses", tags: [{name: "type", value: "X"}]},
        {name: "expenses:fun", tags: []},
    ],
};

/** A fake engine that tells GET from POST on the same path, and records what was sent. */
async function engine(options: {desktop?: boolean; journalOpen?: boolean; folder?: Record<string, unknown> | null} = {}): Promise<Sent[]> {
    await connectFakeEngine();
    const sent: Sent[] = [];
    const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), {status, headers: {"Content-Type": "application/json"}});
    vi.stubGlobal("fetch", (input: unknown, init?: RequestInit) => {
        const url = new URL(String(input));
        const method = init?.method ?? "GET";
        sent.push({method, path: url.pathname, body: init?.body === undefined ? undefined : JSON.parse(String(init.body))});
        if (method === "GET" && url.pathname === "/api/new-ledger") {
            return Promise.resolve(json({desktop: options.desktop ?? true, journalOpen: options.journalOpen ?? true}));
        }
        if (url.pathname === "/api/new-ledger/template") return Promise.resolve(json(TEMPLATE));
        if (url.pathname === "/api/new-ledger/folder") {
            return Promise.resolve(json({folder: options.folder === undefined ? {id: "f-1", name: "Acme Books", empty: true} : options.folder}));
        }
        if (method === "POST" && url.pathname === "/api/new-ledger")
            return Promise.resolve(json({folder: "Acme Books", files: ["main.journal", "accounts.journal"]}));
        if (url.pathname === "/api/new-ledger/sample") return Promise.resolve(json({folder: "sample", files: ["sample.journal"]}));
        return Promise.resolve(new Response(`no route for ${url.pathname}`, {status: 404}));
    });
    return sent;
}

/** Press Next and wait for the step it leads to (`next()` may fetch first). */
async function next(arrive: string): Promise<void> {
    await vi.waitFor(() => expect((screen.getByTestId("new-ledger-next") as HTMLButtonElement).disabled).toBe(false));
    await fireEvent.click(screen.getByTestId("new-ledger-next"));
    await vi.waitFor(() => expect(screen.getAllByTestId(arrive).length).toBeGreaterThan(0));
}

afterEach(() => vi.unstubAllGlobals());

describe("COMPONENT NewLedgerWizard", () => {
    it("walks the fresh path and creates the reviewed chart", async () => {
        const sent = await engine();
        const onDone = vi.fn();
        render(NewLedgerWizard, {onDone, onCancel: () => {}});
        await vi.waitFor(() => expect(screen.getByTestId("new-ledger-cancel")).toBeDefined());

        await fireEvent.click(screen.getByTestId("new-ledger-path-fresh"));
        await next("new-ledger-kind-business");
        await fireEvent.click(screen.getByTestId("new-ledger-kind-business"));
        await vi.waitFor(() => expect(sent.some((r) => r.path === "/api/new-ledger/template")).toBe(true));
        await next("new-ledger-fiscal");

        // Details: a decimal comma changes the preview.
        expect(screen.getByTestId("new-ledger-sample-amount").textContent).toContain("$1,000.00");
        await fireEvent.change(screen.getByTestId("new-ledger-fiscal"), {target: {value: "7"}});
        await next("new-ledger-account-row");

        // Review: four rows; remove the last.
        await vi.waitFor(() => expect(screen.getAllByTestId("new-ledger-account-row")).toHaveLength(4));
        await fireEvent.click(screen.getByLabelText("Remove expenses:fun"));
        expect(screen.getAllByTestId("new-ledger-account-row")).toHaveLength(3);
        await next("new-ledger-create");

        // Folder: nothing can be created until one is picked; its name becomes the title.
        expect((screen.getByTestId("new-ledger-create") as HTMLButtonElement).disabled).toBe(true);
        await fireEvent.click(screen.getByTestId("new-ledger-pick-folder"));
        await vi.waitFor(() => expect(screen.getByTestId("new-ledger-folder").textContent).toContain("Acme Books"));
        expect((screen.getByTestId("new-ledger-title") as HTMLInputElement).value).toBe("Acme Books");

        await fireEvent.click(screen.getByTestId("new-ledger-create"));
        await vi.waitFor(() => expect(onDone).toHaveBeenCalledWith("journal"));
        const create = sent.find((r) => r.method === "POST" && r.path === "/api/new-ledger");
        expect(create?.body).toEqual({
            folderId: "f-1",
            title: "Acme Books",
            currency: {commodity: "$", side: "left", spaced: false, decimals: 2, decimalMark: "period"},
            fiscalYearStart: 7,
            accounts: TEMPLATE.accounts.slice(0, 3),
        });
    });

    it("will not create in a folder that isn't empty", async () => {
        await engine({folder: {id: "f-2", name: "Busy", empty: false}});
        render(NewLedgerWizard, {onDone: () => {}, onCancel: () => {}});
        await vi.waitFor(() => expect(screen.getByTestId("new-ledger-cancel")).toBeDefined());
        await fireEvent.click(screen.getByTestId("new-ledger-path-fresh"));
        await next("new-ledger-kind-empty");
        await fireEvent.click(screen.getByTestId("new-ledger-kind-empty"));
        await next("new-ledger-fiscal");
        await next("new-ledger-create");

        await fireEvent.click(screen.getByTestId("new-ledger-pick-folder"));
        await vi.waitFor(() => expect(screen.getByTestId("new-ledger-folder").textContent).toContain("not empty"));
        expect((screen.getByTestId("new-ledger-create") as HTMLButtonElement).disabled).toBe(true);
        expect(screen.getByTestId("new-ledger-blocker").textContent).toContain("isn't empty");
    });

    it("says the desktop app is needed, and offers no cancel on a first run", async () => {
        await engine({desktop: false, journalOpen: false});
        render(NewLedgerWizard, {onDone: () => {}, onCancel: () => {}});
        await vi.waitFor(() => expect(screen.getByTestId("new-ledger-needs-desktop")).toBeDefined());
        expect(screen.queryByTestId("new-ledger-cancel")).toBeNull();
        expect(screen.queryByTestId("new-ledger-sample")).toBeNull();
    });

    it("opens the sample", async () => {
        const sent = await engine({journalOpen: false});
        const onDone = vi.fn();
        render(NewLedgerWizard, {onDone, onCancel: () => {}});
        await vi.waitFor(() => expect(screen.getByTestId("new-ledger-sample")).toBeDefined());
        await fireEvent.click(screen.getByTestId("new-ledger-sample"));
        await vi.waitFor(() => expect(onDone).toHaveBeenCalledWith("journal"));
        expect(sent.some((r) => r.method === "POST" && r.path === "/api/new-ledger/sample")).toBe(true);
    });

    it("asks which app and which file on the import path", async () => {
        await engine();
        render(NewLedgerWizard, {onDone: () => {}, onCancel: () => {}});
        await vi.waitFor(() => expect(screen.getByTestId("new-ledger-cancel")).toBeDefined());
        await fireEvent.click(screen.getByTestId("new-ledger-path-import"));
        await next("new-ledger-source");
        expect((screen.getByTestId("new-ledger-next") as HTMLButtonElement).disabled).toBe(true);
        await fireEvent.change(screen.getByTestId("new-ledger-source"), {target: {value: "banktivity"}});
        await vi.waitFor(() => expect(screen.getByTestId("new-ledger-source-howto").textContent).toContain("QIF"));
        const input = screen.getByTestId("new-ledger-source-file") as HTMLInputElement;
        expect(input.accept).toBe(".qif");
        const file = new File(["!Type:Bank\n"], "books.csv");
        await fireEvent.change(input, {target: {files: [file]}});
        expect(screen.getByTestId("new-ledger-source-warning").textContent).toContain("exports .qif");
        expect((screen.getByTestId("new-ledger-next") as HTMLButtonElement).disabled).toBe(false);
    });
});
