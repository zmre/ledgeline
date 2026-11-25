// The templates list and the template editor (invoicer PLAN 4d): the shapes
// the API answers, the pure helpers the pages read them through, and the
// calls.
//
// A template is an MBR page template in the books' templates folder: a small
// child of the shipped invoice base (`{% extends "_invoice_base.v1.html" %}`
// and only the blocks it changes) or a whole page. Its first line names it
// for the invoicer's pickers (`{# invoicer: name="…" kinds="…" #}`); the
// editor shows that line as the Name and Kinds fields, and the rest (the
// body) as code. Saves name the content hash the page read, as the other
// editors do (a `409` `changed`, an EditorApiError with the current
// `revision`). Starters not copied into the books are read-only until
// Customize copies one in.
import type {InvoiceEntry, TemplateEntry} from "./invoices.js";
import {noteUrl} from "./library.js";
import {send} from "./editorApi.js";

/** A note naming a template. */
export interface TemplateUse {
    path: string;
    title: string;
    /** `invoice`: a document's `template:`; `customer`: its `invoice_template:` (new invoices start with it). */
    how: "invoice" | "customer";
    number: string | null;
}

/** One template in `GET /api/templates`, with what the templates page shows. */
export interface TemplateListEntry extends TemplateEntry {
    /** Its content hash; null for a starter not copied in. */
    hash: string | null;
    /** The file it extends (`_invoice_base.v1.html`), if any. */
    extends: string | null;
    /** The invoice base's version it extends. */
    base_version: number | null;
    /** The blocks it overrides. */
    overrides: string[];
    used_by: TemplateUse[];
}

/** One of the base's blocks. */
export interface BaseBlock {
    name: string;
    /** What it is for, from the base's own comment. */
    about: string;
}

/** `GET /api/templates/{path}`. */
export interface TemplateDetail extends TemplateListEntry {
    /** The Tera text after the name line. */
    body: string;
    /** A starter not copied in: Customize first. */
    read_only: boolean;
    /** The blocks of the base it extends. */
    base_blocks: BaseBlock[];
}

/** What a save sends: absent fields are left. */
export interface TemplateFields {
    name?: string;
    kinds?: string[];
    body?: string;
}

/** What a create or a save wrote. */
export interface TemplateSaved {
    /** The file in the books folder. */
    path: string;
    /** The `template:` value. */
    template: string;
    name: string;
    hash: string;
    written: boolean;
    changed: string[];
    /** The base, when it was copied in now. */
    base: string | null;
}

/** Why a template didn't render, in words, and where in the body. */
export interface TemplateProblem {
    message: string;
    line: number | null;
    column: number | null;
}

/** `POST /api/templates/preview`: the page to frame, or the problem. */
export interface TemplatePreview {
    url: string | null;
    problem: TemplateProblem | null;
}

/** The editor's form: every field present. */
export interface TemplateForm {
    name: string;
    kinds: string[];
    body: string;
}

/** The kinds of document a template can be for, in order. */
export const TEMPLATE_KINDS: readonly {id: string; label: string}[] = [
    {id: "invoice", label: "Invoices"},
    {id: "estimate", label: "Estimates"},
    {id: "credit-note", label: "Credit notes"},
];

const isObject = (body: unknown): body is Record<string, unknown> => typeof body === "object" && body !== null && !Array.isArray(body);
const isSaved = (body: unknown): boolean => isObject(body) && typeof body.path === "string" && typeof body.hash === "string";

/** The file a `template:` value names, without its leading `/`. */
export function templateFile(template: string): string {
    return template.replace(/^\/+/, "");
}

/** The SPA's route parameter for a template: its file without `.html` (`invoices/tpl/blue`). */
export function templateParam(template: string): string {
    return templateFile(template)
        .replace(/\.html$/i, "")
        .split("/")
        .map(encodeURIComponent)
        .join("/");
}

/** The `template:` value a route parameter names (`invoices/tpl/blue` → `/invoices/tpl/blue.html`). */
export function templateOfParam(param: string): string {
    const file = param
        .replace(/^\/+/, "")
        .split("/")
        .map((s) => {
            // SvelteKit hands params decoded already; a stray `%` stays as is.
            try {
                return decodeURIComponent(s);
            } catch {
                return s;
            }
        })
        .join("/");
    return `/${/\.html$/i.test(file) ? file : `${file}.html`}`;
}

/** `GET /api/templates`: the books' templates by name, then the starters not copied in. */
export function fetchTemplateList(): Promise<TemplateListEntry[]> {
    return send<TemplateListEntry[]>("GET", "/api/templates", Array.isArray);
}

/** `GET /api/templates/{path}`: one template and its body. */
export function fetchTemplate(template: string): Promise<TemplateDetail> {
    return send<TemplateDetail>("GET", noteUrl("/api/templates", templateFile(template)), (b) => isObject(b) && typeof b.body === "string");
}

/** `POST /api/templates`: a new child of the base, or a copy of `from` (`409` `name-taken`). */
export function createTemplate(request: {name: string; kinds?: string[]; from?: string}): Promise<TemplateSaved> {
    return send<TemplateSaved>("POST", "/api/templates", isSaved, request);
}

/** `POST /api/templates/customize`: copy a starter into the books (with the base it extends). */
export function customizeTemplate(template: string): Promise<{path: string; written: boolean; base: string | null}> {
    return send("POST", "/api/templates/customize", (b) => isObject(b) && typeof b.path === "string", {template});
}

/** `PUT /api/templates/{path}`: write `fields` (only those), if the file is still the bytes `hash` names. */
export function saveTemplate(template: string, hash: string, fields: TemplateFields): Promise<TemplateSaved> {
    return send<TemplateSaved>("PUT", noteUrl("/api/templates", templateFile(template)), isSaved, {hash, fields});
}

/** `DELETE /api/templates/{path}`: `409` `in-use` (with `invoices` and `customers`) while any name it. */
export async function deleteTemplate(template: string, hash: string): Promise<void> {
    await send<unknown>("DELETE", noteUrl("/api/templates", templateFile(template)), isObject, {hash});
}

/** `POST /api/templates/preview`: render unsaved `body` as `template` for invoice `number`. */
export function previewTemplate(number: string, template: string, body: string): Promise<TemplatePreview> {
    return send<TemplatePreview>("POST", "/api/templates/preview", (b) => isObject(b) && "url" in b, {number, template, body});
}

/** The form for a template as read. */
export function templateForm(detail: Pick<TemplateDetail, "name" | "kinds" | "body">): TemplateForm {
    return {name: detail.name, kinds: [...detail.kinds], body: detail.body};
}

const sameKinds = (a: readonly string[], b: readonly string[]): boolean => [...a].sort().join(",") === [...b].sort().join(",");

/** The fields `current` changes from `original`, and only those: what a save sends. */
export function templateChanges(original: TemplateForm, current: TemplateForm): TemplateFields {
    const out: TemplateFields = {};
    if (current.name.trim() !== original.name.trim()) out.name = current.name.trim();
    if (!sameKinds(current.kinds, original.kinds)) out.kinds = TEMPLATE_KINDS.map((k) => k.id).filter((k) => current.kinds.includes(k));
    if (current.body !== original.body) out.body = current.body;
    return out;
}

/** Whether the form has unsaved changes. */
export function templateDirty(original: TemplateForm, current: TemplateForm): boolean {
    return Object.keys(templateChanges(original, current)).length > 0;
}

/** "Invoices, estimates": the kinds in words. */
export function kindsText(kinds: readonly string[]): string {
    const words = TEMPLATE_KINDS.filter((k) => kinds.includes(k.id)).map((k) => k.label.toLowerCase());
    if (words.length === 0) return "—";
    const text = words.join(", ");
    return text.charAt(0).toUpperCase() + text.slice(1);
}

/** "2 invoices, 1 customer", or "Not used". */
export function usageText(entry: Pick<TemplateListEntry, "used_by">): string {
    const invoices = entry.used_by.filter((u) => u.how === "invoice").length;
    const customers = entry.used_by.length - invoices;
    const parts: string[] = [];
    if (invoices > 0) parts.push(`${invoices} ${invoices === 1 ? "invoice" : "invoices"}`);
    if (customers > 0) parts.push(`${customers} ${customers === 1 ? "customer" : "customers"}`);
    return parts.length === 0 ? "Not used" : parts.join(", ");
}

/** What a template builds on, in words: the invoice base and its version, another file, or nothing. */
export function baseText(entry: Pick<TemplateListEntry, "extends" | "base_version">): string {
    if (entry.extends === null) return "A whole page";
    if (entry.base_version !== null) return `Invoice base, version ${entry.base_version}`;
    return `Extends ${entry.extends}`;
}

/** The scaffolding "Override block…" inserts: the base's own content kept (`super()`), ready for more. */
export function blockScaffold(name: string): string {
    return `{% block ${name} %}\n{{ super() }}\n{% endblock ${name} %}\n`;
}

/**
 * `body` with `name`'s scaffolding inserted: at `at` (a caret offset) when
 * it falls between lines, else at the end; on lines of its own. Returns the
 * text and where the caret goes (inside, after `super()`).
 */
export function insertBlock(body: string, name: string, at: number | null = null): {body: string; caret: number} {
    const scaffold = blockScaffold(name);
    let where = at === null || at < 0 || at > body.length ? body.length : at;
    // Never inside a line: move to the end of the line the caret is on.
    const lineEnd = body.indexOf("\n", where);
    if (where > 0 && body[where - 1] !== "\n") where = lineEnd === -1 ? body.length : lineEnd + 1;
    const before = body.slice(0, where);
    const lead = before === "" || before.endsWith("\n") ? "" : "\n";
    const text = `${before}${lead}${scaffold}${body.slice(where)}`;
    const caret = before.length + lead.length + scaffold.indexOf("{% endblock");
    return {body: text, caret};
}

/**
 * The blocks `body` defines at the top level, in order, read as the editor
 * types (the server's `overrides` are as saved): `{% block x %}` outside
 * `{# … #}` comments and `{% raw %}`.
 */
export function definedBlocks(body: string): string[] {
    const found: string[] = [];
    let depth = 0;
    let raw = false;
    const tag = /\{#[\s\S]*?#\}|\{%-?\s*(\w+)(?:\s+(\w+))?[\s\S]*?-?%\}/g;
    for (const match of body.matchAll(tag)) {
        const [whole, word, name] = match;
        if (whole.startsWith("{#")) continue;
        if (raw) {
            raw = word !== "endraw";
            continue;
        }
        if (word === "raw") raw = true;
        else if (word === "block" && name !== undefined) {
            if (depth === 0 && !found.includes(name)) found.push(name);
            depth += 1;
        } else if (word === "endblock") depth = Math.max(0, depth - 1);
    }
    return found;
}

/** The invoice a preview starts with: the latest by date, then by number (drafts included). */
export function latestInvoice(invoices: readonly InvoiceEntry[]): InvoiceEntry | null {
    const numbered = invoices.filter((i) => i.number !== null && (i.doc_type === null || i.doc_type === "invoice"));
    const pool = numbered.length > 0 ? numbered : invoices.filter((i) => i.number !== null);
    let best: InvoiceEntry | null = null;
    for (const invoice of pool) {
        if (best === null) {
            best = invoice;
            continue;
        }
        const date = (invoice.date ?? "").localeCompare(best.date ?? "");
        const number = (invoice.number ?? "").localeCompare(best.number ?? "", undefined, {numeric: true});
        if (date > 0 || (date === 0 && number > 0)) best = invoice;
    }
    return best;
}

/** "108 · Customer X · 2026-11-17": an invoice in the preview's picker. */
export function invoiceLabel(invoice: InvoiceEntry): string {
    const who = invoice.invoicee.name;
    return [invoice.number ?? invoice.title, who, invoice.date].filter((p): p is string => p !== null && p !== "").join(" · ");
}
