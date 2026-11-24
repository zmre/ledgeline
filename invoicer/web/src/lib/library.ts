// The catalog and snippets editors (invoicer PLAN 4c): the shapes the API
// answers, the pure helpers the pages read them through, and the calls.
//
// The catalog is a note whose body is one table (ARCHITECTURE §3.6); the
// editor edits it as the invoice editor edits a bill table, as raw markdown
// cells under the table's own headers, and the server writes only the
// table's bytes back (`invoicer_core::catalog_edit`). Snippets are notes whose
// body an invoice copies (§3.7); their title and kind are edited by key and
// the body replaced (`invoicer_core::snippet_edit`). Every write names the
// content hash the page was shown, so an outside edit is never overwritten
// (a `409` `changed`, an EditorApiError with the current `revision`).
import type {GridForm, Role} from "./editor.js";
import {send} from "./editorApi.js";

/** A problem `check` reports about a note. */
export interface NoteProblem {
    severity: "error" | "warning";
    code: string;
    message: string;
    line: number | null;
}

/** One catalog note (`GET /api/catalog`). */
export interface CatalogNote {
    path: string;
    title: string;
    /** Its content hash: what a save names. */
    hash: string;
    /** Its table; null when it has none the invoicer reads (edit it by hand). */
    table: GridForm | null;
    problems: NoteProblem[];
}

/** `GET /api/catalog`. */
export interface CatalogList {
    catalogs: CatalogNote[];
    /** Where a new catalog goes (`[invoicer] catalog`). */
    path: string;
}

/** A cell a save refuses (the server's, or this app's own check). */
export interface CellProblem {
    /** The row in the grid (0-based). */
    row: number;
    /** The column, when one cell is at fault. */
    column: number | null;
    message: string;
}

/** What a catalog save or create wrote. */
export interface CatalogSaved {
    path: string;
    hash: string;
    written: boolean;
    items: number;
}

/** One note using a snippet. */
export interface SnippetUse {
    path: string;
    title: string;
    /** `default`: a customer's `snippets:` names it; `copy`: a document holds its body. */
    how: "default" | "copy";
    number: string | null;
}

/** One snippet (`GET /api/snippets`). */
export interface SnippetEntry {
    path: string;
    title: string;
    kind: string | null;
    aliases: string[];
    body: string;
    hash: string;
    used_by: SnippetUse[];
}

/** What a snippet save sends: absent fields are left; `kind: ""` removes it. */
export interface SnippetFields {
    title?: string;
    kind?: string;
    body?: string;
}

/** What a snippet create or save wrote. */
export interface SnippetSaved {
    path: string;
    title: string;
    link: string;
    kind: string | null;
    hash: string;
    written: boolean;
    changed: string[];
}

/** The snippet form, every field present. */
export interface SnippetForm {
    title: string;
    kind: string;
    body: string;
}

/** The kinds ARCHITECTURE §3.7 names; any other word is allowed too. */
export const SNIPPET_KINDS = ["payment", "terms", "note"] as const;

const isObject = (body: unknown): body is Record<string, unknown> => typeof body === "object" && body !== null && !Array.isArray(body);
const isSaved = (body: unknown): boolean => isObject(body) && typeof body.path === "string" && typeof body.hash === "string";

/** `base/<path>`, each segment percent-encoded. */
export function noteUrl(base: string, path: string): string {
    return `${base}/${path.replace(/^\/+/, "").split("/").map(encodeURIComponent).join("/")}`;
}

/** `GET /api/catalog`: every catalog note, read now. */
export function fetchCatalog(): Promise<CatalogList> {
    return send<CatalogList>("GET", "/api/catalog", (b) => isObject(b) && Array.isArray(b.catalogs));
}

/** `POST /api/catalog`: an empty catalog at `[invoicer] catalog` (`409` `exists` when the file is there). */
export function createCatalog(): Promise<CatalogSaved> {
    return send<CatalogSaved>("POST", "/api/catalog", isSaved, {});
}

/** `PUT /api/catalog/{path}`: write the note's table, if it is still the bytes `hash` names. */
export function saveCatalog(path: string, hash: string, table: GridForm): Promise<CatalogSaved> {
    return send<CatalogSaved>("PUT", noteUrl("/api/catalog", path), isSaved, {hash, table});
}

/** `GET /api/snippets`: every snippet, with its body and uses. */
export function fetchSnippets(): Promise<SnippetEntry[]> {
    return send<SnippetEntry[]>("GET", "/api/snippets", Array.isArray);
}

/** `POST /api/snippets`: a new snippet in `<templates-dir>/snippets/` (`409` `name-taken`). */
export function createSnippet(fields: SnippetFields): Promise<SnippetSaved> {
    return send<SnippetSaved>("POST", "/api/snippets", isSaved, fields);
}

/** `PUT /api/snippets/{path}`: write `fields` (only those), if the note is still the bytes `hash` names. */
export function saveSnippet(path: string, hash: string, fields: SnippetFields): Promise<SnippetSaved> {
    return send<SnippetSaved>("PUT", noteUrl("/api/snippets", path), isSaved, {hash, fields});
}

/** `DELETE /api/snippets/{path}`: `409` `in-use` (with `customers`) while customers name it, unless `force`. */
export async function deleteSnippet(path: string, hash: string, force = false): Promise<void> {
    await send<unknown>("DELETE", noteUrl("/api/snippets", path), isObject, {hash, force});
}

// ---------------------------------------------------------------------------
// The catalog grid.
// ---------------------------------------------------------------------------

/** A deep copy of a grid (plain JSON; a `$state` proxy copies too). */
export function cloneGrid(table: GridForm): GridForm {
    return JSON.parse(JSON.stringify(table)) as GridForm;
}

const blank = (cell: string | undefined): boolean => (cell ?? "").replace(/\*\*|__/g, "").trim() === "";

/** Whether every cell of a row is blank (a save drops such rows). */
export function isBlankRow(row: readonly string[]): boolean {
    return row.every((cell) => blank(cell));
}

/** Whether two grids differ in anything a save would write (blank rows don't count). */
export function gridsDiffer(a: GridForm, b: GridForm): boolean {
    const shape = (t: GridForm): string =>
        JSON.stringify({columns: t.columns.map((c) => c.header), rows: t.rows.filter((r) => !isBlankRow(r)).map((r) => r.map((c) => c.trim()))});
    return shape(a) !== shape(b);
}

/** The column holding `role`, or -1. */
export function roleColumn(table: GridForm, role: Role): number {
    return table.columns.findIndex((c) => c.role === role);
}

/** `table` with an empty row after `after` (at the end when null); and where it went. */
export function addCatalogRow(table: GridForm, after: number | null): {table: GridForm; at: number} {
    const at = after === null ? table.rows.length : Math.min(after + 1, table.rows.length);
    const rows = table.rows.map((r) => [...r]);
    rows.splice(
        at,
        0,
        table.columns.map(() => "")
    );
    return {table: {...table, rows}, at};
}

/** `table` with cell (`row`, `column`) set to `text`. */
export function setCatalogCell(table: GridForm, row: number, column: number, text: string): GridForm {
    return {...table, rows: table.rows.map((cells, r) => (r === row ? cells.map((c, i) => (i === column ? text : c)) : cells))};
}

/**
 * What this app can tell before the server does: every non-blank row needs a
 * code and a description; a code is one word without `:` (the CLI's
 * `--from-catalog CODE:QTY`) and used once, case-insensitively. Rates and
 * accounts are the server's to judge (the books' money style, hledger's
 * account rules); its answer arrives as a `422` with `problems`.
 */
export function catalogProblems(table: GridForm): CellProblem[] {
    const code = roleColumn(table, "item");
    const description = roleColumn(table, "description");
    const out: CellProblem[] = [];
    const seen = new Set<string>();
    table.rows.forEach((row, r) => {
        if (isBlankRow(row)) return;
        const text = (row[code] ?? "").replace(/\*\*|__/g, "").trim();
        if (code >= 0 && text === "") out.push({row: r, column: code, message: "a row needs a code"});
        else if (/\s/.test(text)) out.push({row: r, column: code, message: `the code ${JSON.stringify(text)} can't hold a space`});
        else if (/[:|\\]/.test(text)) out.push({row: r, column: code, message: `the code ${text} can't hold ":", "|" or "\\"`});
        else if (seen.has(text.toLowerCase())) out.push({row: r, column: code, message: `the code ${text} is used twice`});
        else seen.add(text.toLowerCase());
        if (description >= 0 && blank(row[description])) out.push({row: r, column: description, message: "a row needs a description"});
    });
    return out;
}

/** The problems a 422 carried, as cell problems. */
export function cellProblemsOf(body: Record<string, unknown>): CellProblem[] {
    const list = body.problems;
    if (!Array.isArray(list)) return [];
    return list.filter(
        (p): p is CellProblem =>
            isObject(p) && typeof p.row === "number" && typeof p.message === "string" && (p.column === null || typeof p.column === "number")
    );
}

// ---------------------------------------------------------------------------
// Snippets.
// ---------------------------------------------------------------------------

/** The form for a snippet as listed. */
export function snippetForm(entry: SnippetEntry): SnippetForm {
    return {title: entry.title, kind: entry.kind ?? "", body: entry.body};
}

/** The fields `current` changes from `original`, and only those: what a save sends. */
export function snippetChanges(original: SnippetForm, current: SnippetForm): SnippetFields {
    const out: SnippetFields = {};
    if (current.title.trim() !== original.title.trim()) out.title = current.title.trim();
    if (current.kind.trim() !== original.kind.trim()) out.kind = current.kind.trim();
    if (current.body.trim() !== original.body.trim()) out.body = current.body;
    return out;
}

/** Whether the snippet form has unsaved changes. */
export function snippetDirty(original: SnippetForm, current: SnippetForm): boolean {
    return Object.keys(snippetChanges(original, current)).length > 0;
}

/** The customers naming a snippet as a default. */
export function namingCustomers(entry: SnippetEntry): SnippetUse[] {
    return entry.used_by.filter((u) => u.how === "default");
}

/** "Customer X; 2 invoices" — how a snippet is used, in a few words. */
export function usageText(entry: SnippetEntry): string {
    const customers = namingCustomers(entry).length;
    const copies = entry.used_by.length - customers;
    const parts: string[] = [];
    if (customers > 0) parts.push(`${customers} ${customers === 1 ? "customer" : "customers"}`);
    if (copies > 0) parts.push(`${copies} ${copies === 1 ? "invoice" : "invoices"}`);
    return parts.length === 0 ? "Not used" : parts.join(", ");
}
