// The catalog and snippets editors' pure parts: the grid's own checks, what
// counts as a change, the snippet form's changed fields, and the requests
// (with the hash, and the server's refusals as EditorApiErrors).
import {afterEach, describe, expect, it, vi} from "vitest";
import type {GridForm} from "./editor.js";
import {EditorApiError} from "./editorApi.js";
import {
    addCatalogRow,
    catalogProblems,
    cellProblemsOf,
    deleteSnippet,
    gridsDiffer,
    isBlankRow,
    noteUrl,
    saveCatalog,
    saveSnippet,
    setCatalogCell,
    snippetChanges,
    snippetDirty,
    usageText,
    type SnippetEntry,
} from "./library.js";

const CATALOG: GridForm = {
    columns: [
        {header: "Code", role: "item"},
        {header: "Description", role: "description"},
        {header: "Unit", role: "unit"},
        {header: "Rate", role: "rate"},
        {header: "*Account*", role: "books"},
    ],
    rows: [
        ["ARCH", "Senior architect hours", "h", "$800", "revenue:consulting"],
        ["JR", "Junior engineer", "h", "$200", "revenue:consulting:delivery"],
    ],
};

const NET_30: SnippetEntry = {
    path: "invoices/tpl/snippets/net-30.md",
    title: "Net 30 ACH",
    kind: "payment",
    aliases: [],
    body: "## Payment\n\nPay by ACH.\n",
    hash: "h1",
    used_by: [
        {path: "customers/Customer X.md", title: "Customer X", how: "default", number: null},
        {path: "invoices/105.md", title: "Invoice 105", how: "copy", number: "105"},
        {path: "invoices/108.md", title: "Invoice 108", how: "copy", number: "108"},
    ],
};

describe("UNIT catalog grid", () => {
    it("adds an empty row after the focused one, or at the end", () => {
        const after = addCatalogRow(CATALOG, 0);
        expect(after.at).toBe(1);
        expect(after.table.rows[1]).toEqual(["", "", "", "", ""]);
        expect(CATALOG.rows).toHaveLength(2);
        expect(addCatalogRow(CATALOG, null).at).toBe(2);
        expect(setCatalogCell(CATALOG, 1, 3, "$250").rows[1][3]).toBe("$250");
    });

    it("counts a change only when a save would write one", () => {
        expect(gridsDiffer(CATALOG, addCatalogRow(CATALOG, null).table)).toBe(false);
        expect(gridsDiffer(CATALOG, setCatalogCell(CATALOG, 0, 3, " $800 "))).toBe(false);
        expect(gridsDiffer(CATALOG, setCatalogCell(CATALOG, 0, 3, "$850"))).toBe(true);
        expect(gridsDiffer(CATALOG, {...CATALOG, rows: [CATALOG.rows[1], CATALOG.rows[0]]})).toBe(true);
        expect(isBlankRow(["", " ", "**", ""])).toBe(true);
    });

    it("marks missing and duplicate codes, codes the CLI can't name, and missing descriptions", () => {
        const table: GridForm = {
            ...CATALOG,
            rows: [
                ...CATALOG.rows,
                ["arch", "Again", "", "", ""],
                ["", "No code", "", "", ""],
                ["A:B", "", "", "", ""],
                ["TWO WORDS", "x", "", "", ""],
                ["", "", "", "", ""],
            ],
        };
        expect(catalogProblems(table).map((p) => [p.row, p.column])).toEqual([
            [2, 0],
            [3, 0],
            [4, 0],
            [4, 1],
            [5, 0],
        ]);
        expect(catalogProblems(table)[0].message).toBe("the code arch is used twice");
        expect(catalogProblems(CATALOG)).toEqual([]);
    });

    it("reads the server's cell problems from a 422", () => {
        expect(cellProblemsOf({problems: [{row: 1, column: 3, message: "the rate ten: not a number"}, {row: "x"}]})).toEqual([
            {row: 1, column: 3, message: "the rate ten: not a number"},
        ]);
        expect(cellProblemsOf({})).toEqual([]);
    });
});

describe("UNIT snippet form", () => {
    const original = {title: "Net 30 ACH", kind: "payment", body: "## Payment\n\nPay by ACH.\n"};

    it("sends only what changed", () => {
        expect(snippetChanges(original, {...original, body: "## Payment\n\nPay by ACH.  \n\n"})).toEqual({});
        expect(snippetDirty(original, {...original, title: " Net 30 ACH "})).toBe(false);
        expect(snippetChanges(original, {...original, title: "ACH, net 30 ", kind: ""})).toEqual({title: "ACH, net 30", kind: ""});
        expect(snippetChanges(original, {...original, body: "Pay within 30 days."})).toEqual({body: "Pay within 30 days."});
    });

    it("says how a snippet is used", () => {
        expect(usageText(NET_30)).toBe("1 customer, 2 invoices");
        expect(usageText({...NET_30, used_by: []})).toBe("Not used");
    });
});

describe("UNIT catalog and snippet requests", () => {
    afterEach(() => vi.unstubAllGlobals());

    it("encodes note paths segment by segment", () => {
        expect(noteUrl("/api/snippets", "invoices/tpl/snippets/Late fee: 1.5%.md")).toBe("/api/snippets/invoices/tpl/snippets/Late%20fee%3A%201.5%25.md");
    });

    it("saves the catalog with its hash; a conflict carries the current one, a refusal its cells", async () => {
        const fetch = vi.fn(async (_url: string, init?: RequestInit) => {
            const body = JSON.parse(String(init?.body)) as {hash: string};
            if (body.hash === "stale") return new Response(JSON.stringify({error: "changed on disk", code: "changed", revision: "now"}), {status: 409});
            if (body.hash === "bad")
                return new Response(JSON.stringify({error: "row 2: …", code: "invalid", problems: [{row: 1, column: 3, message: "no"}]}), {status: 422});
            return new Response(JSON.stringify({path: "invoices/catalog.md", hash: "new", written: true, items: 2}), {status: 200});
        });
        vi.stubGlobal("fetch", fetch);
        const saved = await saveCatalog("invoices/catalog.md", "abc", CATALOG);
        expect(saved.items).toBe(2);
        const [url, init] = fetch.mock.calls[0];
        expect(url).toBe("/api/catalog/invoices/catalog.md");
        expect(init?.method).toBe("PUT");
        expect(JSON.parse(String(init?.body))).toEqual({hash: "abc", table: CATALOG});

        await expect(saveCatalog("invoices/catalog.md", "stale", CATALOG)).rejects.toMatchObject({status: 409, code: "changed", revision: "now"});
        const refused = await saveCatalog("invoices/catalog.md", "bad", CATALOG).catch((e: unknown) => e);
        expect(refused).toBeInstanceOf(EditorApiError);
        expect(cellProblemsOf((refused as EditorApiError).body)).toEqual([{row: 1, column: 3, message: "no"}]);
    });

    it("saves and deletes snippets with the hash", async () => {
        const fetch = vi.fn(async (_url: string, init?: RequestInit) => {
            if (init?.method === "DELETE") {
                const body = JSON.parse(String(init.body)) as {force: boolean};
                if (!body.force)
                    return new Response(JSON.stringify({error: "Customer X still names it", code: "in-use", customers: ["Customer X"]}), {status: 409});
                return new Response(JSON.stringify({deleted: "invoices/tpl/snippets/net-30.md"}), {status: 200});
            }
            return new Response(JSON.stringify({path: "invoices/tpl/snippets/net-30.md", hash: "h2", written: true, changed: ["body"]}), {status: 200});
        });
        vi.stubGlobal("fetch", fetch);
        await saveSnippet(NET_30.path, "h1", {body: "Pay."});
        expect(JSON.parse(String(fetch.mock.calls[0][1]?.body))).toEqual({hash: "h1", fields: {body: "Pay."}});
        await expect(deleteSnippet(NET_30.path, "h2")).rejects.toMatchObject({code: "in-use", body: {customers: ["Customer X"]}});
        await deleteSnippet(NET_30.path, "h2", true);
        expect(JSON.parse(String(fetch.mock.calls[2][1]?.body))).toEqual({hash: "h2", force: true});
    });
});
