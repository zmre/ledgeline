// Regenerates ledgeline-export.xlsx with the real journal-tab exporter (see README.md):
//   cd web && bun ../fixtures/import/qb-journal/ledgeline-export.gen.ts ../fixtures/import/qb-journal/ledgeline-export.xlsx

import {writeFileSync} from "node:fs";
import {buildJournalWorkbook} from "../../../web/src/lib/export/journalXlsx.ts";

const D = {side: "L", spaced: false, precision: 2, decimalPoint: ".", digitGroups: [",", [3]]} as const;
const S = {side: "R", spaced: true, precision: 4, decimalPoint: ".", digitGroups: null} as const;
const styleOf = (c: string) => (c === "$" ? D : S) as never;
const dec = (m: number, p: number) => ({m: BigInt(m), p});
const amt = (c: string, q: ReturnType<typeof dec>, cost?: unknown) => ({commodity: c, qty: q, style: styleOf(c), ...(cost ? {cost} : {})});
const post = (account: string, amounts: unknown[], extra = {}) => ({account, amounts, status: "unmarked", comment: "", tags: [], ...extra});
const txn = (index: number, date: string, description: string, postings: unknown[], extra = {}) => ({index, date, status: "unmarked", description, code: "", comment: "", tags: [], postings, haystack: "", ...extra});

const txns = [
    txn(7, "2026-03-15", "Acme payroll", [
        post("income:salary", [amt("$", dec(-500000, 2))]),
        post("expenses:taxes:federal", [amt("$", dec(90000, 2))]),
        post("expenses:health", [amt("$", dec(25000, 2))], {comment: "HSA"}),
        post("assets:checking", [amt("$", dec(3850, 0))]),
    ], {code: "1042", tags: [["vendor", "Acme Corp"], ["class", "Household"]]}),
    txn(9, "2026-03-20", "Buy AAPL", [
        post("assets:brokerage", [amt("AAPL", dec(10, 0), {commodity: "$", qty: dec(15000, 2), per: true})]),
        post("assets:checking", [amt("$", dec(-150000, 2))]),
    ]),
    txn(12, "2026-03-28", "Grocery store", [
        post("expenses:food", [amt("$", dec(8720, 2))]),
        post("liabilities:card", [amt("$", dec(-8720, 2))]),
    ], {tags: [["customer", "Self"]]}),
];
const wb = await buildJournalWorkbook(txns as never, styleOf, {title: "Acme Books", period: "2026-03-01 – 2026-03-31", generatedAt: new Date(Date.UTC(2026, 3, 1))});
writeFileSync(process.argv[2], Buffer.from(await wb.xlsx.writeBuffer()));
