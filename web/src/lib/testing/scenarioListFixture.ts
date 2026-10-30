// Wire bytes for the scenario list's component tests (plan 31 C3): a seed as
// the engine sends one, and a scenario FILE with every kind of piece — a
// section heading, a rule with an attached comment, a turned-off alternative,
// a step pair, a note, a one-off, an asset rule, an `=` rule the form can
// state, one it cannot, and a piece of file text. Decoded by the real
// `decodeScenario`, so the tests exercise the tab's own decoding path.

import {decodeScenario} from "#lib/api/nativeDecode.js";
import {classesOf} from "#lib/domain/accountClassFixtures.js";
import type {AmountStyle} from "#lib/domain/types.js";
import type {Scenario} from "#lib/projections/types.js";

export const SEED_NOTE = "unbudgeted — average over 2025-08-01 to 2026-07-08";

const monthly = {raw: "monthly", simple: "monthly", base: null, from: null, to: null};

/** One wire line. Revenue is NEGATIVE on the wire, exactly as the posting is written. */
export function wireLine(account: string, mantissa: string, overrides: Record<string, unknown> = {}): Record<string, unknown> {
    return {
        id: `gap:${account}:$`,
        group: `gap:${account}:$`,
        role: "flow",
        account,
        amount: {commodity: "$", quantity: {mantissa, places: 2}, precision: 2},
        period: monthly,
        growth: null,
        opening: null,
        note: SEED_NOTE,
        counter: null,
        gains: null,
        source: "unbudgeted",
        comment: "",
        ruleComment: "",
        section: account.startsWith("income:") ? "income" : "expense",
        ...overrides,
    };
}

const newItem = (kind: string, key: string) => ({kind, key, origin: null, disabled: false, attached: "", raw: ""});

export const SEED_WIRE = {
    name: "",
    created: null,
    updated: null,
    lines: [
        wireLine("income:salary", "-518833"),
        wireLine("income:dividends", "-729"),
        wireLine("expenses:housing", "187500"),
        wireLine("expenses:taxes", "133833"),
    ],
    events: [],
    cash: null,
    autoRules: [] as unknown[],
    accountTypes: {},
    // A seed's layout is derived by the engine, every entry new (plan 31 amendment C2-2).
    layout: ["income:salary", "income:dividends", "expenses:housing", "expenses:taxes"].map((account) => newItem("rule", `gap:${account}:$`)),
    comments: [],
    fixed: [],
};

export const COMMISSION_FORM = {
    matchAccount: "income:salary",
    extraQuery: "",
    postings: [{account: "expenses:taxes", kind: "percent", percent: {mantissa: "3", places: 0}, amount: null}],
    counter: "assets:checking",
    comment: "",
    forecastOnly: true,
};

const authored = (account: string, mantissa: string, overrides: Record<string, unknown>) =>
    wireLine(account, mantissa, {source: "journal", note: "", counter: "assets:checking", ...overrides});

export const FILE_WIRE = {
    name: "Series A",
    created: "2026-10-29",
    updated: "2026-10-29",
    lines: [
        authored("income:salary", "-600000", {id: "rule:0:0", group: "rule:0", note: "salary", growth: {rate: {mantissa: "3", places: 2}, unit: "year"}}),
        authored("income:acme", "-4000000", {
            id: "off:0:0",
            group: "off:0",
            note: "acme contract",
            period: {...monthly, raw: "monthly from 2027-01-01", from: "2027-01-01"},
        }),
        authored("expenses:payroll", "500000", {
            id: "payroll",
            group: "rule:1",
            note: "payroll",
            period: {...monthly, raw: "monthly to 2027-02-01", to: "2027-02-01"},
        }),
        authored("expenses:payroll", "800000", {
            id: "payroll",
            group: "rule:2",
            note: "payroll",
            period: {...monthly, raw: "monthly from 2027-02-01", from: "2027-02-01"},
        }),
        authored("assets:house", "0", {
            id: "rule:4:0",
            group: "rule:4",
            role: "asset",
            note: "investing",
            section: "asset",
            growth: {rate: {mantissa: "4", places: 2}, unit: "year"},
            opening: {commodity: "$", quantity: {mantissa: "700000", places: 0}, precision: 0},
            gains: "equity:gains",
        }),
    ],
    events: [
        {
            id: "rule:3",
            date: "2027-03-01",
            description: "Series A",
            postings: [{account: "assets:checking", amount: {commodity: "$", quantity: {mantissa: "200000000", places: 2}, precision: 2}, comment: ""}],
            counter: "equity:capital",
            comment: "",
        },
    ],
    cash: "assets:checking",
    autoRules: [
        {
            raw: "= ^income:salary(:|$) tag:_generated-transaction\n    expenses:taxes  *0.03\n    assets:checking  *-0.03\n",
            query: "^income:salary(:|$) tag:_generated-transaction",
            unsupported: null,
            form: COMMISSION_FORM,
            block: 0,
            key: "auto:0",
        },
        {raw: "= expenses:rent\n    (budget:rent)  *-1\n", query: "expenses:rent", unsupported: null, form: null, block: 1, key: "auto:1"},
    ],
    accountTypes: {},
    layout: [
        {kind: "fixed", key: "fixed:0", origin: 0, disabled: false, attached: "", raw: "account equity:gains  ; type: E\n"},
        {kind: "comment", key: "comment:0", origin: 1, disabled: false, attached: "", raw: ";;;;;;;;;;;;;;;;;;;;;;\n;; Income\n"},
        {
            kind: "rule",
            key: "rule:0",
            origin: 2,
            disabled: false,
            attached: "; Paid monthly.\n",
            raw: "; Paid monthly.\n~ monthly  salary\n    income:salary  $-6000.00  ; growth: 3%/yr\n    assets:checking\n",
        },
        {
            kind: "rule",
            key: "off:0",
            origin: 3,
            disabled: true,
            attached: "",
            raw: "; ~ monthly from 2027-01-01  acme contract\n;     income:acme  $-40000.00\n;     assets:checking\n",
        },
        {kind: "rule", key: "rule:1", origin: 4, disabled: false, attached: "", raw: "~ monthly to 2027-02-01  payroll\n"},
        {kind: "rule", key: "rule:2", origin: 5, disabled: false, attached: "", raw: "~ monthly from 2027-02-01  payroll\n"},
        {kind: "comment", key: "comment:1", origin: 6, disabled: false, attached: "", raw: "; Assumes no price rises.\n"},
        {kind: "event", key: "rule:3", origin: 7, disabled: false, attached: "", raw: "~ 2027-03-01  Series A\n"},
        {kind: "rule", key: "rule:4", origin: 8, disabled: false, attached: "", raw: "~ monthly  investing\n"},
        {kind: "auto", key: "auto:0", origin: 9, disabled: false, attached: "", raw: "= ^income:salary(:|$) tag:_generated-transaction\n"},
        {kind: "auto", key: "auto:1", origin: 10, disabled: false, attached: "", raw: "= expenses:rent\n    (budget:rent)  *-1\n"},
    ],
    comments: [
        {key: "comment:0", style: "section", raw: ";;;;;;;;;;;;;;;;;;;;;;\n;; Income\n", text: "Income"},
        {key: "comment:1", style: "note", raw: "; Assumes no price rises.\n", text: "Assumes no price rises."},
    ],
    fixed: [{key: "fixed:0", raw: "account equity:gains  ; type: E\n"}],
};

export const SEED: Scenario = decodeScenario(SEED_WIRE);
export const FILE: Scenario = decodeScenario(FILE_WIRE);

export const ACCOUNT_NAMES = [
    "income:salary",
    "income:dividends",
    "income:acme",
    "expenses:housing",
    "expenses:taxes",
    "expenses:payroll",
    "assets:checking",
    "assets:house",
    "equity:capital",
];
export const DECLARED = classesOf(["income", "revenue"], ["expenses", "expense"], ["assets", "asset"], ["equity", "equity"]);
export const STYLES = new Map<string, AmountStyle>([["$", {side: "L", spaced: false, precision: 2, decimalPoint: ".", digitGroups: [",", [3]]}]]);
