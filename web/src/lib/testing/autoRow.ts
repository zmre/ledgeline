// One journal row as the engine serves it with `--auto` in force (plan 30):
// the January sale of `fixtures/auto-postings/`, whose `= revenue:sales` rule
// adds a sales-tax pair BETWEEN the two written postings. Shared by the
// decoder, edit-mapping, row-model and component tests, so they all agree on
// what a generated posting looks like on the wire.

import {decodeRow} from "#lib/api/journalViewDecode.js";
import type {Transaction} from "#lib/domain/types.js";

const STYLE = {side: "L", spaced: false, precision: 2, decimalPoint: ".", digitGroups: [",", [3]]};

const amount = (m: string, p: number) => ({commodity: "$", qty: {m, p}, style: {...STYLE, precision: Math.max(2, p)}});

const RULE = {rule: 0, query: "revenue:sales"};
const RULE_TAGS = [
    ["generated-posting", "= revenue:sales"],
    ["_generated-posting", "= revenue:sales"],
];

/** `GET /api/transactions/4` (or one entry of `/api/journal/rows`) for the sale. */
export function autoSaleWire(): Record<string, unknown> {
    return {
        index: 4,
        date: "2026-01-10",
        status: "unmarked",
        code: "",
        description: "Acme | sale",
        comment: "",
        tags: [
            ["modified-transaction", ""],
            ["_modified-transaction", ""],
        ],
        postings: [
            {account: "revenue:sales", amounts: [amount("-33333", 2)], status: "unmarked", comment: "", tags: []},
            {account: "liabilities:tax:sales", amounts: [amount("-33333", 3)], status: "unmarked", comment: "", tags: RULE_TAGS, generated: RULE},
            {account: "assets:checking", amounts: [amount("33333", 3)], status: "unmarked", comment: "", tags: RULE_TAGS, generated: RULE},
            {account: "assets:checking", amounts: [amount("33333", 2)], status: "unmarked", comment: "", tags: []},
        ],
        flags: [],
    };
}

/** The same row, decoded. */
export function autoSale(): Transaction {
    return decodeRow(autoSaleWire()).txn;
}

/** `GET /api/auto-rules` over `fixtures/auto-postings/`, abridged to one rule. */
export const AUTO_RULES_LISTING = {
    inForce: true,
    conf: "hledger.conf",
    confHijacked: false,
    editable: true,
    newRule: {journalId: "main.journal", revision: "rev-main"},
    rules: [
        {
            index: 0,
            file: "rules.journal",
            line: 5,
            query: "revenue:sales",
            raw: "= revenue:sales\n    liabilities:tax:sales      *0.1\n    assets:checking\n",
            unsupported: null,
            generated: 4,
            problems: [
                {
                    kind: "unknownAccount",
                    message: "The automatic posting rule `= revenue:sales` (rules.journal:5) posts to liabilities:tax:sales, which …",
                    txnIndex: null,
                    account: "liabilities:tax:sales",
                },
            ],
            journalId: "rules.journal",
            block: 0,
            revision: "rev-rules",
            form: {
                matchAccount: "revenue:sales",
                extraQuery: "",
                postings: [{account: "liabilities:tax:sales", kind: "percent", percent: {mantissa: "10", places: 0}, amount: null}],
                counter: "assets:checking",
                comment: "",
                forecastOnly: false,
            },
        },
        {
            index: 1,
            file: "rules.journal",
            line: 9,
            query: "expr:'food and not lunch'",
            raw: "= expr:'food and not lunch'\n    (budget:food)  *-1\n",
            unsupported: "expr: with a nested boolean is not supported",
            generated: 0,
            problems: [],
            journalId: "rules.journal",
            block: 1,
            revision: "rev-rules",
            form: null,
        },
        {
            index: 2,
            file: "payroll/rules.journal",
            line: 2,
            query: "expenses:payroll",
            raw: "= expenses:payroll\n    expenses:payroll-tax  *0.1\n    assets:checking\n",
            unsupported: null,
            generated: 8,
            problems: [3, 7].map((txnIndex) => ({
                kind: "refused",
                message: "Under --auto, the automatic posting rule `= expenses:payroll` (rules.journal:2) fires 2 times in this transaction …",
                txnIndex,
                account: null,
            })),
            // A file that cannot be written: listed, never editable.
            journalId: null,
            block: 0,
            revision: null,
            form: null,
        },
    ],
};
