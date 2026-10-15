// Journal-tab xlsx export: the filtered transactions as an accountant-ready
// double-entry journal, laid out like QuickBooks Online's Journal report so it
// reads like what accountants already receive (and so our own QuickBooks
// journal importer, crates/ledgeline-core/src/qb_journal.rs, reads it back):
//
//   rows 1-3  title band: journal name, "Journal", date range; row 4 blank
//   row 5     header; column A is the unlabelled group-marker column
//   groups    `612` alone in column A, one row per posting (date and type
//             repeated on every row), then `Total for 612` with the group's
//             debit and credit totals
//   end       a `TOTAL` row, then a generation-timestamp footer
//
// A transaction with more than two accounts is simply a longer group — that is
// the point of the layout. Amounts are signed the hledger way, so
// `debit = amount > 0`, `credit = -amount` needs no account types.
//
// Loaded with `await import(...)` from the journal route ONLY, so neither this
// module nor exceljs reaches the journal tab's initial bundle.

import {add, mul, neg, roundTo, displayPlaces, MAX_QUANTITY_DECIMALS, formatAmount, toNumber, type Dec} from "#lib/domain/money.js";
import type {Amount, AmountStyle, Posting, Transaction} from "#lib/domain/types.js";
import type {Workbook, Worksheet} from "exceljs"; // type-only: erased at build time
import {downloadWorkbook, numberFormat} from "./xlsx";

/** QuickBooks has no notion of hledger's transaction kinds; every entry is a journal entry. */
const TRANSACTION_TYPE = "Journal Entry";

/** The QuickBooks dimension columns, emitted only when some exported entry carries the tag. */
const DIMENSIONS = [
    {tag: "class", header: "Item class"},
    {tag: "customer", header: "Customer full name"},
    {tag: "vendor", header: "Vendor"},
] as const;

export interface JournalExportMeta {
    /** The journal's display name (company line of the title band); null falls back to "Ledgeline". */
    title: string | null;
    /** Human date range, e.g. "2026-01-01 – 2026-12-31" or "all dates". */
    period: string;
    /** Generation time for the footer; injectable so tests are deterministic. */
    generatedAt?: Date;
}

/** One posting row's money: the value the debit/credit columns carry, and the as-written amount when they differ. */
interface RowValue {
    commodity: string;
    qty: Dec;
    /** Set when the posting is priced (`10 AAPL @ $150`): the as-written amount, for the Quantity column. */
    quantity?: string;
}

/**
 * What a posting amount is worth in the debit/credit columns.
 *
 * A priced amount is carried at its cost, so a share purchase debits the
 * brokerage `$1,500.00` and credits the bank `$1,500.00` — a group that
 * balances, the way an accountant expects — with the `10 AAPL @ $150.00` it
 * came from kept in the Quantity column rather than lost. hledger stores a
 * total cost unsigned, so the sign comes from the amount.
 */
function rowValue(amount: Amount, styleOf: (commodity: string) => AmountStyle): RowValue {
    if (amount.cost === undefined) return {commodity: amount.commodity, qty: amount.qty};
    const magnitude = amount.cost.per ? mul(amount.qty.m < 0n ? neg(amount.qty) : amount.qty, amount.cost.qty) : amount.cost.qty;
    const qty = amount.qty.m < 0n ? neg(magnitude) : magnitude;
    const costStyle = styleOf(amount.cost.commodity);
    const costText = formatAmount({commodity: amount.cost.commodity, qty: amount.cost.qty, style: costStyle}, MAX_QUANTITY_DECIMALS);
    const quantity = `${formatAmount(amount, MAX_QUANTITY_DECIMALS)} ${amount.cost.per ? "@" : "@@"} ${costText}`;
    return {commodity: amount.cost.commodity, qty, quantity};
}

/**
 * Round to what the commodity displays at — its (declared) style precision,
 * under the non-money cap so a share count is never rounded to cents. Totals
 * are summed from these rounded values, so a `Total for` row always equals the
 * cells above it, in Excel as on paper.
 */
function shown(value: Dec, style: AmountStyle): Dec {
    return roundTo(value, displayPlaces(value, style.precision, MAX_QUANTITY_DECIMALS));
}

/** A posting's account as hledger writes it: `(a)` unbalanced virtual, `[a]` balanced virtual. */
function accountLabel(posting: Posting): string {
    if (posting.type === "virtual") return `(${posting.account})`;
    if (posting.type === "balancedVirtual") return `[${posting.account}]`;
    return posting.account;
}

function tagValue(posting: Posting, txn: Transaction, tag: string): string {
    const found = posting.tags.find(([name]) => name === tag) ?? txn.tags.find(([name]) => name === tag);
    return found?.[1] ?? "";
}

export interface JournalRow {
    date: string;
    num: string;
    name: string;
    description: string;
    account: string;
    commodity: string;
    /** Exactly one of debit/credit is set, both unsigned and already rounded to display. */
    debit: Dec | null;
    credit: Dec | null;
    quantity: string;
    dimensions: string[];
}

export interface JournalGroup {
    id: number;
    rows: JournalRow[];
}

/**
 * The workbook's content, independent of exceljs: one group per transaction,
 * one row per posting amount (a multi-commodity posting gets a row each).
 */
export function journalGroups(txns: readonly Transaction[], styleOf: (commodity: string) => AmountStyle): JournalGroup[] {
    return txns.map((txn) => ({
        id: txn.index,
        rows: txn.postings.flatMap((posting) =>
            posting.amounts.map((amount) => {
                const value = rowValue(amount, styleOf);
                const qty = shown(value.qty, styleOf(value.commodity));
                const negative = qty.m < 0n;
                return {
                    date: posting.date ?? txn.date,
                    num: txn.code,
                    name: txn.description,
                    description: posting.comment.trim() || txn.comment.trim(),
                    account: accountLabel(posting),
                    commodity: value.commodity,
                    debit: negative ? null : qty,
                    credit: negative ? neg(qty) : null,
                    quantity: value.quantity ?? "",
                    dimensions: DIMENSIONS.map(({tag}) => tagValue(posting, txn, tag)),
                };
            })
        ),
    }));
}

type Totals = Map<string, {debit: Dec; credit: Dec}>;

function addTotals(totals: Totals, rows: readonly JournalRow[]): Totals {
    for (const row of rows) {
        const prev = totals.get(row.commodity);
        const zero = {m: 0n, p: 0};
        totals.set(row.commodity, {
            debit: add(prev?.debit ?? zero, row.debit ?? zero),
            credit: add(prev?.credit ?? zero, row.credit ?? zero),
        });
    }
    return totals;
}

type Cell = ReturnType<Worksheet["getCell"]>;

function setMoney(cell: Cell, commodity: string, qty: Dec): void {
    cell.value = toNumber(qty);
    cell.numFmt = numberFormat(commodity, qty.p, qty.p);
    cell.alignment = {horizontal: "right"};
}

/** A totals cell: a real number for one commodity, `1,500.00 $, 10 AAPL`-style text for several. */
function setTotal(cell: Cell, totals: Totals, side: "debit" | "credit", styleOf: (commodity: string) => AmountStyle): void {
    const entries = [...totals.entries()].sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0));
    if (entries.length === 1) {
        setMoney(cell, entries[0][0], entries[0][1][side]);
    } else {
        cell.value = entries.map(([commodity, t]) => formatAmount({commodity, qty: t[side], style: styleOf(commodity)}, MAX_QUANTITY_DECIMALS)).join(", ");
        cell.alignment = {horizontal: "right"};
    }
    cell.font = {bold: true};
}

/** Build the workbook (exported separately so tests can read it back without a DOM). */
export async function buildJournalWorkbook(
    txns: readonly Transaction[],
    styleOf: (commodity: string) => AmountStyle,
    meta: JournalExportMeta
): Promise<Workbook> {
    const {Workbook: ExcelWorkbook} = await import("exceljs");
    const workbook = new ExcelWorkbook();
    const ws = workbook.addWorksheet("Journal");
    const groups = journalGroups(txns, styleOf);
    const rows = groups.flatMap((group) => group.rows);
    const dimensions = DIMENSIONS.filter((_, i) => rows.some((row) => row.dimensions[i] !== ""));
    const withQuantity = rows.some((row) => row.quantity !== "");

    const headers = [
        "",
        "Transaction date",
        "Transaction type",
        "Num",
        "Name",
        "Description",
        "Account Name",
        "Debit",
        "Credit",
        ...(withQuantity ? ["Quantity"] : []),
        ...dimensions.map((d) => d.header),
    ];
    const col = (header: string): number => headers.indexOf(header) + 1;
    const DEBIT = col("Debit");
    const CREDIT = col("Credit");

    const lastCol = headers.length;
    [meta.title ?? "Ledgeline", "Journal", meta.period].forEach((text, i) => {
        ws.mergeCells(i + 1, 1, i + 1, lastCol);
        const cell = ws.getCell(i + 1, 1);
        cell.value = text;
        cell.alignment = {horizontal: "center"};
        cell.font = {bold: i < 2, size: i === 0 ? 14 : 11};
    });
    headers.forEach((header, i) => {
        const cell = ws.getCell(5, i + 1);
        if (header !== "") cell.value = header; // the marker column has no label at all
        cell.font = {bold: true};
        cell.border = {bottom: {style: "thin"}};
    });

    const styleColumn = (header: string, width: number): void => {
        if (col(header) > 0) ws.getColumn(col(header)).width = width;
    };
    ws.getColumn(1).width = 12;
    styleColumn("Transaction date", 14);
    styleColumn("Transaction type", 14);
    styleColumn("Num", 8);
    styleColumn("Name", 32);
    styleColumn("Description", 32);
    styleColumn("Account Name", 36);
    styleColumn("Debit", 14);
    styleColumn("Credit", 14);
    styleColumn("Quantity", 28);
    for (const d of dimensions) styleColumn(d.header, 20);
    ws.views = [{state: "frozen", ySplit: 5}];

    let rowIx = 6;
    const grand: Totals = new Map();
    for (const group of groups) {
        ws.getCell(rowIx, 1).value = String(group.id);
        rowIx += 1;
        const first = rowIx;
        for (const row of group.rows) {
            const [y, m, d] = row.date.split("-").map(Number);
            const date = ws.getCell(rowIx, col("Transaction date"));
            date.value = new Date(Date.UTC(y, m - 1, d));
            date.numFmt = "mm/dd/yyyy";
            ws.getCell(rowIx, col("Transaction type")).value = TRANSACTION_TYPE;
            if (row.num !== "") ws.getCell(rowIx, col("Num")).value = row.num;
            ws.getCell(rowIx, col("Name")).value = row.name;
            if (row.description !== "") ws.getCell(rowIx, col("Description")).value = row.description;
            ws.getCell(rowIx, col("Account Name")).value = row.account;
            if (row.debit !== null) setMoney(ws.getCell(rowIx, DEBIT), row.commodity, row.debit);
            if (row.credit !== null) setMoney(ws.getCell(rowIx, CREDIT), row.commodity, row.credit);
            if (row.quantity !== "") ws.getCell(rowIx, col("Quantity")).value = row.quantity;
            dimensions.forEach((dimension) => {
                const value = row.dimensions[DIMENSIONS.indexOf(dimension)];
                if (value !== "") ws.getCell(rowIx, col(dimension.header)).value = value;
            });
            rowIx += 1;
        }
        const totals = addTotals(new Map(), group.rows);
        addTotals(grand, group.rows);
        const marker = ws.getCell(rowIx, 1);
        marker.value = `Total for ${group.id}`;
        marker.font = {bold: true};
        setTotal(ws.getCell(rowIx, DEBIT), totals, "debit", styleOf);
        setTotal(ws.getCell(rowIx, CREDIT), totals, "credit", styleOf);
        // Like QuickBooks, a single-commodity total is a live SUM over its rows
        // (with the value cached beside it), so an accountant's edit re-totals.
        if (totals.size === 1 && rowIx > first) {
            for (const [column, side] of [
                [DEBIT, "debit"],
                [CREDIT, "credit"],
            ] as const) {
                const cell = ws.getCell(rowIx, column);
                const letter = ws.getColumn(column).letter;
                cell.value = {formula: `SUM(${letter}${first}:${letter}${rowIx - 1})`, result: toNumber([...totals.values()][0][side])};
            }
        }
        rowIx += 1;
    }

    const total = ws.getCell(rowIx, 1);
    total.value = "TOTAL";
    total.font = {bold: true};
    if (grand.size > 0) {
        setTotal(ws.getCell(rowIx, DEBIT), grand, "debit", styleOf);
        setTotal(ws.getCell(rowIx, CREDIT), grand, "credit", styleOf);
    }
    rowIx += 2;
    const generatedAt = meta.generatedAt ?? new Date();
    ws.getCell(rowIx, 1).value = `Generated by Ledgeline ${generatedAt.toLocaleString()}`;
    ws.getCell(rowIx, 1).font = {italic: true, size: 9, color: {argb: "FF64748B"}};
    return workbook;
}

/** Build the journal .xlsx and trigger a browser download. */
export async function exportJournalXlsx(
    txns: readonly Transaction[],
    styleOf: (commodity: string) => AmountStyle,
    meta: JournalExportMeta,
    filename: string
): Promise<void> {
    await downloadWorkbook(await buildJournalWorkbook(txns, styleOf, meta), filename);
}
