// A whole `Projection` for the web tests (plan 28): two forecast months after
// two months of actuals, with the three statements filled in.
//
// The statements are the Reports tab's own fixtures, laid out as a projection's
// columns would be: the grouped balance sheet repeated across four month-ends,
// the engine's own monthly income statement bytes, and the hand-built cash flow
// statement. Their figures are not a projection of each other — no test here
// reads them as one — they are there so every reading has something real to
// render, and `projectedFrom` has columns on both sides of it.

import {readFileSync} from "node:fs";
import {decodeBalanceSheetReport, decodeIncomeStatementReport} from "#lib/api/nativeDecode.js";
import {dec, type MixedAmount} from "#lib/domain/money.js";
import type {ForecastEntry, ForecastJournal, ForecastOrigin, Projection} from "#lib/projections/types.js";
import type {AccountReport, AccountReportRow, BalanceSheetColumns, IsBucket} from "#lib/reports/types.js";
import {asColumns} from "#lib/reports/ui/balanceSheetRows.js";
import {GROUPED_BALANCE_SHEET} from "./balanceSheetFixture";
import {CASH_FLOW} from "./cashFlowFixture";

export const usd = (n: number): MixedAmount => new Map([["$", dec(n, 0)]]);

/** The four month-ends a two-plus-two projection's balance sheet is laid out at. */
export const PROJECTION_COLUMNS: IsBucket[] = [
    {key: "2026-06", label: "Jun 2026", from: "2026-06-01", to: "2026-06-30"},
    {key: "2026-07", label: "Jul 2026", from: "2026-07-01", to: "2026-07-31"},
    {key: "2026-08", label: "Aug 2026", from: "2026-08-01", to: "2026-08-31"},
    {key: "2026-09", label: "Sep 2026", from: "2026-09-01", to: "2026-09-30"},
];

/** A one-column sheet repeated across `columns`, every figure the same in each. */
export function widenSheet(sheet: BalanceSheetColumns, columns: IsBucket[]): BalanceSheetColumns {
    const across = <T>(figures: readonly T[]): T[] => columns.map(() => figures[0]);
    return {
        ...sheet,
        columns,
        sections: sheet.sections.map((section) => ({
            ...section,
            groups: section.groups.map((group) => ({
                ...group,
                rows: group.rows.map((row) => ({...row, own: across(row.own), inclusive: across(row.inclusive)})),
                total: across(group.total),
            })),
            subsections: section.subsections.map((band) => ({...band, total: across(band.total)})),
            total: across(section.total),
        })),
        netWorth: across(sheet.netWorth),
        check: across(sheet.check),
        balanced: across(sheet.balanced),
    };
}

/** A committed native golden, read relative to `web/` (vitest's cwd). */
const golden = (name: string): unknown => JSON.parse(readFileSync(`../fixtures/native/v1/${name}.json`, "utf8"));

/** The engine's own interval-mode income statement bytes. */
const MONTHLY_INCOME = decodeIncomeStatementReport(golden("incomestatement-grouped-monthly"));

export function projectionFixture(overrides: Partial<Projection> = {}): Projection {
    return {
        buckets: ["2026-08", "2026-09"],
        lookback: ["2026-06", "2026-07"],
        actualsEnd: "2026-08-01",
        projectedFrom: 2,
        cash: {opening: usd(10000), values: [usd(14800), usd(19600)]},
        runway: null,
        assets: [],
        warnings: [],
        balanceSheet: widenSheet(asColumns(decodeBalanceSheetReport(GROUPED_BALANCE_SHEET)), PROJECTION_COLUMNS),
        incomeStatement: MONTHLY_INCOME,
        cashFlow: CASH_FLOW,
        defaults: {cash: "assets:bank:checking", gains: "equity:unrealized gains", lines: new Map(), events: new Map()},
        accounts: null,
        journal: null,
        ...overrides,
    };
}

/**
 * A `POST /api/projections/run` response body AS IT ARRIVES — wire JSON, for a
 * test that serves it from a fake engine and decodes it the real way. One
 * forecast month, no actuals; the statements are real goldens of their types.
 */
export function rawProjectionRun(): Record<string, unknown> {
    const nothing = [{}];
    const section = (kind: string, title: string) => ({kind, title, groups: [], subsections: [], total: nothing});
    return {
        buckets: ["2026-08"],
        lookback: [],
        actualsEnd: "2026-08-01",
        projectedFrom: 0,
        cash: {opening: {$: {mantissa: "500000", places: 2}}, values: [{$: {mantissa: "500000", places: 2}}]},
        runway: null,
        // `[]` is a real answer and the key is never absent — this scenario holds
        // no asset rows. `decodeProjection` refuses a missing one.
        assets: [],
        warnings: [],
        balanceSheet: {
            columns: [{key: "2026-08", label: "Aug 2026", from: "2026-08-01", to: "2026-08-31"}],
            base: "$",
            value: "market",
            sections: [section("assets", "Assets"), section("liabilities", "Liabilities"), section("equity", "Equity")],
            netWorth: nothing,
            check: nothing,
            balanced: [true],
            meta: {unpriced: []},
        },
        incomeStatement: golden("incomestatement-grouped-monthly"),
        cashFlow: golden("cashflow"),
        defaults: {cash: "assets:bank:checking", gains: "equity:unrealized gains", lines: {}, events: {}},
        // Asked for by neither: `null`, never absent.
        accounts: null,
        journal: null,
    };
}

const periodInfo = (key: string, label: string, start: string, end: string) => ({key, label, start, end});

/**
 * The Accounts reading over `PROJECTION_COLUMNS`, in Change mode: checking,
 * rent and salary, plus a declared account with nothing in any column (what
 * "Show empty accounts" adds). Balance mode is the same rows with no totals.
 */
/**
 * A projection's Accounts reading: checking as a BALANCE (opening $10,000,
 * then +$800 a month) with its Δ over the window, rent, shopping and salary
 * as each month's CHANGE with a Total — the shape the engine sends since
 * 2026-10-28.
 */
export function accountsFixture(): AccountReport {
    const row = (account: string, values: number[], summary: number): AccountReportRow => ({
        account,
        depth: account.split(":").length,
        values: values.map(usd),
        summary: usd(summary),
    });
    const changes = (account: string, values: number[]) =>
        row(
            account,
            values,
            values.reduce((a, b) => a + b, 0)
        );
    return {
        kind: "accounts",
        buckets: PROJECTION_COLUMNS.map((column) => column.key),
        periods: [
            periodInfo("2026-06", "Jun 2026", "2026-06-01", "2026-06-30"),
            periodInfo("2026-07", "Jul 2026", "2026-07-01", "2026-07-31"),
            periodInfo("2026-08", "Aug 2026", "2026-08-01", "2026-08-31"),
            periodInfo("2026-09", "Sep 2026", "2026-09-01", "2026-09-30"),
        ],
        sections: [
            {
                section: "assets",
                reading: "balance",
                rows: [row("assets", [10800, 11600, 12400, 13200], 3200), row("assets:checking", [10800, 11600, 12400, 13200], 3200)],
                totals: [usd(10800), usd(11600), usd(12400), usd(13200)],
                summary: usd(3200),
            },
            {
                section: "revenue",
                reading: "change",
                rows: [changes("income", [-5000, -5000, -5000, -5000]), changes("income:salary", [-5000, -5000, -5000, -5000])],
                totals: [usd(-5000), usd(-5000), usd(-5000), usd(-5000)],
                summary: usd(-20000),
            },
            {
                section: "expenses",
                reading: "change",
                rows: [
                    changes("expenses", [4200, 4200, 4200, 4200]),
                    changes("expenses:rent", [4200, 4200, 4200, 4200]),
                    changes("expenses:shopping", [0, 0, 0, 0]),
                ],
                totals: [usd(4200), usd(4200), usd(4200), usd(4200)],
                summary: usd(16800),
            },
        ],
    };
}

/** One generated transaction, as the journal tab reads it. */
export function forecastEntry(index: number, date: string, origin: ForecastOrigin, description = "projection", period = "~ monthly"): ForecastEntry {
    const amount = (n: number) => ({
        commodity: "$",
        qty: dec(n * 100, 2),
        style: {side: "L" as const, spaced: false, precision: 2, decimalPoint: ".", digitGroups: [",", [3]] as [string, number[]]},
    });
    return {
        origin,
        // What core's `haystack` builds for this transaction: description,
        // comment, then each posting's account and its amount as displayed.
        haystack: [description, `generated-transaction: ${period}`, "expenses:rent", "$4,200.00", "$", "assets:checking", "$-4,200.00", "$"]
            .join("\n")
            .toLowerCase(),
        txn: {
            index,
            date,
            status: "unmarked",
            description,
            code: "",
            comment: `generated-transaction: ${period}`,
            tags: [
                ["generated-transaction", period],
                ["_generated-transaction", period],
            ],
            postings: [
                {account: "expenses:rent", amounts: [amount(4200)], status: "unmarked", comment: "", tags: []},
                {account: "assets:checking", amounts: [amount(-4200)], status: "unmarked", comment: "", tags: []},
            ],
        },
    };
}

/** `n` monthly rent transactions from 2026-08, then one event. */
export function journalFixture(n = 2): ForecastJournal {
    const entries = Array.from({length: n}, (_, i) => {
        const month = 8 + i;
        const date = `${2026 + Math.floor((month - 1) / 12)}-${String(((month - 1) % 12) + 1).padStart(2, "0")}-01`;
        return forecastEntry(100 + i, date, {kind: "rule", key: "rule:0"});
    });
    entries.push(forecastEntry(100 + n, "2026-08-20", {kind: "event", key: "laptop"}, "new laptop", "2026-08-20"));
    return {total: entries.length, entries, truncated: false};
}
