// The invoice editor's model (invoicer PLAN 4a): the shapes the editor API
// answers, and the pure operations on a bill table the grid makes. No Svelte.
//
// The form is the server's (`GET /api/invoices/{num}/form`): header fields,
// custom fields, the bill table as columns plus raw markdown cells, and the
// prose above and below. The grid edits raw cells — what the file holds — so
// nothing a hand edit wrote is lost; the server rebuilds the file from them,
// recomputing every computed amount (ARCHITECTURE §3.3).
import {DEFAULT_TOTAL_WORDS, isBlank, isBold, plain, type Role, type RowKind} from "./arithmetic.js";
import type {NumberSource} from "./invoices.js";

export type {Role, RowKind} from "./arithmetic.js";

/** A column of the bill table. */
export interface Column {
    header: string;
    role: Role;
}

/** The bill table as the form carries it. */
export interface GridForm {
    columns: Column[];
    rows: string[][];
}

/** A custom frontmatter field. */
export interface CustomField {
    key: string;
    value: unknown;
}

/** `GET /api/invoices/{num}/form`'s form: what the editor edits. */
export interface EditorForm {
    customer: string;
    template: string | null;
    number: string;
    date: string;
    terms: number | null;
    due_date: string | null;
    reference: string | null;
    deposit_for: string | null;
    fields: CustomField[];
    table: GridForm | null;
    prose_before: string;
    prose_after: string;
}

/** A deferral as the server reads a books cell. */
export interface DeferralView {
    target_account: string;
    months: number | null;
    through: string | null;
    service_start: string | null;
    calendar: boolean;
}

/** One computed row (`show`'s table row). */
export interface RowView {
    row: number;
    line: number;
    kind: RowKind;
    description: string;
    code: string | null;
    quantity: string | null;
    unit: string | null;
    rate: string | null;
    amount: string | null;
    written: string;
    computed: boolean;
    applies_to: number | null;
    books: string | null;
    account: string | null;
    deferral: DeferralView | null;
}

/** The bill table, computed by the server. */
export interface TableView {
    commodity: string;
    total: string;
    rows: RowView[];
    discrepancies: {row: number; line: number; written: string; expected: string}[];
}

/** A problem `check` would report. */
export interface EditorProblem {
    severity: "error" | "warning";
    code: string;
    message: string;
    line: number | null;
}

/** What sending would book, and what is booked. */
export interface WouldBook {
    basis: "accrual" | "cash";
    would_book: string | null;
    booked: {date: string; description: string; text: string} | null;
    in_step: boolean | null;
    receivable: string | null;
    error: string | null;
}

/** `POST /api/invoices/{num}/compute`. */
export interface Computed {
    text: string;
    changed: boolean;
    total: string | null;
    commodity: string | null;
    table: TableView | null;
    table_error: string | null;
    problems: EditorProblem[];
    books: WouldBook | null;
}

/** `GET /api/invoices/{num}/form`. */
export interface EditorState {
    path: string;
    title: string;
    number: string;
    status: "draft" | "sent" | "void" | string;
    revision: string;
    form: EditorForm;
    computed: Computed;
}

/** `GET /api/editor/options`. */
export interface EditorOptions {
    customers: {
        title: string;
        link: string;
        path: string;
        kind: "person" | "organization" | string;
        terms: number | null;
        invoice_template: string | null;
        open_pos: {reference: string; amount: string | null; expires: string | null}[];
    }[];
    catalog: CatalogItem[];
    accounts: string[];
    /** Those of `accounts` that are revenue, by declared type first (a customer's revenue account suggests them first). */
    revenue_accounts?: string[];
    deposit_account: string;
    revenue_account: string | null;
    fields: string[];
    words: Words;
    next_number: string;
    /** The number `next_number` follows, null at the start (Phase 4 feedback H19). */
    next_number_after?: string | null;
    /** Where that is: `document`, `books` (only the journal books it) or `start`. */
    next_number_source?: NumberSource | null;
    customers_dir: string;
    /** Every word a final total row may be labelled with (`[invoicer.words] total`), the default first. */
    total_words?: string[];
    /** Every subtotal word (`[invoicer.words] subtotal`). */
    subtotal_words?: string[];
    /** Notes `deposit_for` may link to: estimates first. */
    link_notes?: {title: string; link: string; kind: string | null}[];
    /** The books' default payment terms (Settings → New invoices), for a customer with none; null: no default. */
    default_terms?: number | null;
}

/** The labels total rows may carry: the books' words, else the engine's defaults. */
export interface TotalWords {
    total: string[];
    subtotal: string[];
}

/** The total and subtotal words `options` names (the defaults until they load). */
export function totalWords(options: EditorOptions | null, words: Words = DEFAULT_WORDS): TotalWords {
    const total = options?.total_words?.length
        ? options.total_words
        : [words.total, ...DEFAULT_TOTAL_WORDS.filter((w) => w !== "Subtotal" && w !== words.total)];
    const subtotal = options?.subtotal_words?.length ? options.subtotal_words : [words.subtotal];
    return {total, subtotal};
}

/** Every label that makes a row a total row (subtotal or final). */
export const allTotalWords = (w: TotalWords): string[] => [...new Set([...w.subtotal, ...w.total, ...DEFAULT_TOTAL_WORDS])];

/** A catalog item (`GET /api/editor/options`). */
export interface CatalogItem {
    code: string;
    description: string;
    unit: string | null;
    rate: string | null;
    books: string | null;
}

/** The words new rows are written with (`[invoicer.words]`, the first of each). */
export interface Words {
    description: string;
    quantity: string;
    rate: string;
    amount: string;
    unit: string;
    item: string;
    subtotal: string;
    total: string;
    deposit: string;
    tax: string;
}

/** The English defaults, for tests and before the options load. */
export const DEFAULT_WORDS: Words = {
    description: "Description",
    quantity: "Qty",
    rate: "Rate",
    amount: "Amount",
    unit: "Unit",
    item: "Code",
    subtotal: "Subtotal",
    total: "Total due",
    deposit: "Less deposit",
    tax: "Sales tax",
};

/** `GET /api/invoices/{num}/deposits`. */
export interface OpenDeposits {
    customer: string;
    account: string;
    commodity: string;
    balance: string;
    deposits: {
        number: string | null;
        path: string | null;
        date: string;
        description: string;
        credited: string;
        remaining: string;
        row: {description: string; amount: string; books: string};
    }[];
}

/** The row types the grid's buttons add. */
export type NewRowKind = "item" | "discount-percent" | "discount-fixed" | "subtotal" | "tax" | "spacer";

const clone = (table: GridForm): GridForm => ({columns: table.columns.map((c) => ({...c})), rows: table.rows.map((r) => [...r])});

/** The column holding `role`, or -1. */
export const columnOf = (table: GridForm, role: Role): number => table.columns.findIndex((c) => c.role === role);

/** Row `row`'s cell in the `role` column ("" when there is none). */
export function cellOf(table: GridForm, row: number, role: Role): string {
    const at = columnOf(table, role);
    return at < 0 ? "" : (table.rows[row]?.[at] ?? "");
}

/** `table` with row `row`'s `role` cell set to `text` (a no-op when there's no such column). */
export function setCell(table: GridForm, row: number, role: Role, text: string): GridForm {
    const at = columnOf(table, role);
    if (at < 0 || row < 0 || row >= table.rows.length) return table;
    const next = clone(table);
    next.rows[row][at] = text.replace(/\r?\n/g, " ");
    return next;
}

/** The default header a new column of `role` gets. */
function headerFor(role: Role, words: Words): string {
    switch (role) {
        case "item":
            return words.item;
        case "unit":
            return words.unit;
        case "quantity":
            return words.quantity;
        case "rate":
            return words.rate;
        case "amount":
            return words.amount;
        case "description":
            return words.description;
        case "date":
            return "Date";
        case "books":
            return "*Account*";
        default:
            return role;
    }
}

/** Where a new `role` column goes, as the CLI's `new` writes tables: Code first, Unit after Qty, the books column last. */
function insertionPoint(table: GridForm, role: Role): number {
    const at = (r: Role): number => columnOf(table, r);
    const books = at("books");
    const end = books < 0 ? table.columns.length : books;
    switch (role) {
        case "item":
        case "date":
            return Math.max(0, at("description"));
        case "unit":
            return at("quantity") >= 0 ? at("quantity") + 1 : at("rate") >= 0 ? at("rate") : end;
        case "quantity":
            return at("description") >= 0 ? at("description") + 1 : end;
        case "rate":
            return at("amount") >= 0 ? at("amount") : end;
        case "books":
            return table.columns.length;
        default:
            return end;
    }
}

/** `table` with a `role` column (added empty when missing). */
export function ensureColumn(table: GridForm, role: Role, words: Words = DEFAULT_WORDS): GridForm {
    if (columnOf(table, role) >= 0) return table;
    const at = insertionPoint(table, role);
    const next = clone(table);
    next.columns.splice(at, 0, {header: headerFor(role, words), role});
    next.rows.forEach((row) => row.splice(at, 0, ""));
    return next;
}

const fold = (text: string): string => text.trim().replace(/\s+/g, " ").toLowerCase();

/** The column of row `row`'s total label (a bold total word outside the amount and books columns), or -1. */
export function totalLabelColumn(table: GridForm, row: number, words: Words = DEFAULT_WORDS, extra: readonly string[] = []): number {
    const known = [words.subtotal, words.total, ...DEFAULT_TOTAL_WORDS, ...extra].map(fold);
    return (table.rows[row] ?? []).findIndex((text, at) => {
        const role = table.columns[at]?.role;
        return role !== "amount" && role !== "books" && isBold(text) && known.includes(fold(plain(text)));
    });
}

/** Whether row `row` is a total row (a bold total word outside the amount and books columns). */
export function isTotalRow(table: GridForm, row: number, words: Words = DEFAULT_WORDS, extra: readonly string[] = []): boolean {
    return totalLabelColumn(table, row, words, extra) >= 0;
}

/** Row `row`'s total label, unbolded ("" when it isn't a total row). */
export function totalLabel(table: GridForm, row: number, words: Words = DEFAULT_WORDS, extra: readonly string[] = []): string {
    const at = totalLabelColumn(table, row, words, extra);
    return at < 0 ? "" : plain(table.rows[row][at]);
}

/** `table` with total row `row` relabelled `label` (bold, in the cell its label is in). Blank labels are refused: the row stays a total. */
export function setTotalLabel(table: GridForm, row: number, label: string, words: Words = DEFAULT_WORDS, extra: readonly string[] = []): GridForm {
    const at = totalLabelColumn(table, row, words, extra);
    const text = label.trim();
    if (at < 0 || text === "") return table;
    const next = clone(table);
    next.rows[row][at] = `**${text}**`;
    return next;
}

/** The last total row (the amount due), or -1. */
export function finalTotal(table: GridForm, words: Words = DEFAULT_WORDS, extra: readonly string[] = []): number {
    for (let i = table.rows.length - 1; i >= 0; i -= 1) if (isTotalRow(table, i, words, extra)) return i;
    return -1;
}

/** Whether row `row` is a spacer: every cell blank. */
export const isSpacerRow = (table: GridForm, row: number): boolean => (table.rows[row] ?? []).every((text) => isBlank(text));

/**
 * `table` with a final total row: unchanged when it has one, else a
 * `**Total due**` row (the first total word) appended, its label in the
 * rate column (the description when there's none), as `new` writes it.
 */
export function ensureTotal(table: GridForm, words: Words = DEFAULT_WORDS, extra: readonly string[] = []): GridForm {
    if (finalTotal(table, words, extra) >= 0) return table;
    return insertCells(table, table.rows.length, {rate: `**${words.total}**`});
}

/** Where a new row goes: at the bottom, just above the final total (rows below it can't be booked), else at the end. */
export function insertionRow(table: GridForm, words: Words = DEFAULT_WORDS, extra: readonly string[] = []): number {
    const total = finalTotal(table, words, extra);
    return total >= 0 ? total : table.rows.length;
}

/** The cells of a new row of `kind`, by role. */
export function newRowCells(kind: NewRowKind, words: Words = DEFAULT_WORDS, commodity = "$"): Partial<Record<Role, string>> {
    const zero = commodity.length <= 1 ? `-${commodity}0.00` : `-0.00 ${commodity}`;
    switch (kind) {
        case "item":
            return {quantity: "1"};
        case "discount-percent":
            return {description: "Discount", rate: "-10%"};
        case "discount-fixed":
            return {description: "Discount", amount: zero};
        case "subtotal":
            return {rate: `**${words.subtotal}**`};
        case "tax":
            return {description: words.tax, rate: "0%"};
        case "spacer":
            return {};
    }
}

/** `table` with `cells` inserted as a new row at `at`. A label for the rate column goes in the description when there is no rate column. */
export function insertCells(table: GridForm, at: number, cells: Partial<Record<Role, string>>): GridForm {
    const next = clone(table);
    const row = next.columns.map(() => "");
    for (const [role, text] of Object.entries(cells) as [Role, string][]) {
        let col = columnOf(next, role);
        if (col < 0 && role === "rate") col = columnOf(next, "description");
        if (col >= 0) row[col] = text;
    }
    next.rows.splice(Math.max(0, Math.min(at, next.rows.length)), 0, row);
    return next;
}

/** `table` with a new `kind` row at the bottom, above the final total (added when missing). Returns the table and the new row's index. */
export function insertRow(
    table: GridForm,
    kind: NewRowKind,
    words: Words = DEFAULT_WORDS,
    commodity = "$",
    extra: readonly string[] = []
): {table: GridForm; at: number} {
    const totalled = ensureTotal(table, words, extra);
    const at = insertionRow(totalled, words, extra);
    return {table: insertCells(totalled, at, newRowCells(kind, words, commodity)), at};
}

/** `table` with row `from` moved to `to` (clamped). */
export function moveRow(table: GridForm, from: number, to: number): GridForm {
    if (from < 0 || from >= table.rows.length) return table;
    const target = Math.max(0, Math.min(to, table.rows.length - 1));
    if (target === from) return table;
    const next = clone(table);
    const [row] = next.rows.splice(from, 1);
    next.rows.splice(target, 0, row);
    return next;
}

/** Where a row dragged from `from` lands when dropped before row `before` (in the table as it is): the `moveRow` target. */
export const dropIndex = (from: number, before: number): number => (before > from ? before - 1 : before);

/** `table` without row `row`. */
export function removeRow(table: GridForm, row: number): GridForm {
    if (row < 0 || row >= table.rows.length) return table;
    const next = clone(table);
    next.rows.splice(row, 1);
    return next;
}

/** `table` with a catalog item filled into row `row` (code, description, unit, rate, books), adding Code/Unit columns it needs. */
export function fillFromCatalog(table: GridForm, row: number, item: CatalogItem, words: Words = DEFAULT_WORDS): GridForm {
    let next = ensureColumn(table, "item", words);
    if (item.unit !== null && item.unit !== "") next = ensureColumn(next, "unit", words);
    next = setCell(next, row, "item", item.code);
    next = setCell(next, row, "description", item.description);
    if (item.unit !== null) next = setCell(next, row, "unit", item.unit);
    if (item.rate !== null) next = setCell(next, row, "rate", item.rate);
    if (item.books !== null && columnOf(next, "books") >= 0) next = setCell(next, row, "books", item.books);
    if (isBlank(cellOf(next, row, "quantity"))) next = setCell(next, row, "quantity", "1");
    return next;
}

/** A books cell, read: an account, or a deferral. */
export interface BooksCell {
    account: string;
    deferral: {target: string; months: number; from: string | null; calendar: boolean} | null;
}

const DEFERRAL = /^(.+?)\s*(?:→|->)\s*(.+?)\s*,\s*(\d+)\s+months?(?:\s+from\s+(\d{4}-\d{2}-\d{2}))?(\s+calendar)?\s*$/i;

/**
 * Read a books cell (ARCHITECTURE §3.3): `account`, or
 * `deferred → recognize-into, N months [from YYYY-MM-DD] [calendar]`. Null
 * for text this form can't show (a `through` term): edit it as text.
 */
export function parseBooksCell(text: string): BooksCell | null {
    const t = text.trim();
    if (t === "") return {account: "", deferral: null};
    if (!/→|->/.test(t)) return {account: t, deferral: null};
    const m = DEFERRAL.exec(t);
    if (m === null) return null;
    return {account: m[1].trim(), deferral: {target: m[2].trim(), months: Number(m[3]), from: m[4] ?? null, calendar: m[5] !== undefined}};
}

/** A books cell, written as the engine writes it. */
export function formatBooksCell(cell: BooksCell): string {
    const account = cell.account.trim();
    if (cell.deferral === null) return account;
    const {target, months, from, calendar} = cell.deferral;
    const unit = months === 1 ? "month" : "months";
    return `${account} → ${target.trim()}, ${months} ${unit}${from ? ` from ${from}` : ""}${calendar ? " calendar" : ""}`;
}

/** `date` plus `days`, `YYYY-MM-DD` (UTC arithmetic: no zone can shift it). */
export function addDays(date: string, days: number): string | null {
    const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(date);
    if (m === null || !Number.isFinite(days)) return null;
    const at = new Date(Date.UTC(Number(m[1]), Number(m[2]) - 1, Number(m[3]) + days));
    const pad = (n: number): string => String(n).padStart(2, "0");
    return `${at.getUTCFullYear()}-${pad(at.getUTCMonth() + 1)}-${pad(at.getUTCDate())}`;
}

/**
 * Whether `table` is a deposit invoice's (ARCHITECTURE §4.4): a row books a
 * charge (not a negative "less deposit" row) to the deposit account.
 */
export function isDepositTable(table: GridForm | null, depositAccount: string | null | undefined): boolean {
    if (table === null || !depositAccount) return false;
    const books = columnOf(table, "books");
    if (books < 0) return false;
    const amount = columnOf(table, "amount");
    const rate = columnOf(table, "rate");
    return table.rows.some((row) => {
        const account = (parseBooksCell(row[books] ?? "")?.account ?? "").toLowerCase();
        const negative = [amount, rate].some((at) => at >= 0 && plain(row[at] ?? "").startsWith("-"));
        return account === depositAccount.toLowerCase() && !negative;
    });
}

/** Whether two forms differ (what saving would change). */
export function formsDiffer(a: EditorForm, b: EditorForm): boolean {
    return JSON.stringify(a) !== JSON.stringify(b);
}

/** A deep copy of a form (a JSON round trip: the form is plain JSON data, and this works on Svelte proxies too). */
export const copyForm = (form: EditorForm): EditorForm => JSON.parse(JSON.stringify(form)) as EditorForm;

/** What to do with a change on disk to the open invoice. */
export type DiskChange = "ignore" | "replace" | "conflict";

/**
 * The file changed on disk (an editor, git, our own save coming back): its
 * revision is ours → ignore; theirs, and nothing unsaved here → show theirs;
 * theirs, with unsaved edits here → ask (never clobber either side).
 */
export function onDiskChange(ours: string, theirs: string, dirty: boolean): DiskChange {
    if (ours === theirs) return "ignore";
    return dirty ? "conflict" : "replace";
}

/** A row's base, in words, for an adjustment: "of Senior architect hours, $2,500.00". */
export function baseText(rows: readonly RowView[], row: RowView): string | null {
    if (row.kind !== "adjustment" || row.applies_to === null) return null;
    const target = rows.find((r) => r.row === row.applies_to);
    if (target === undefined) return null;
    const name = target.kind === "total" ? plain(target.rate ?? "") || "the total" : target.description || `row ${target.row}`;
    return `of ${name}${target.amount ? `, ${target.amount}` : ""}`;
}

/** An empty form, for tests. */
export function emptyForm(over: Partial<EditorForm> = {}): EditorForm {
    return {
        customer: "",
        template: null,
        number: "",
        date: "",
        terms: null,
        due_date: null,
        reference: null,
        deposit_for: null,
        fields: [],
        table: null,
        prose_before: "",
        prose_after: "",
        ...over,
    };
}
