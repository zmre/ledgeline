// The running-balance columns (plan 36), mounted: the real journal view store
// fed wire rows that carry `running` cells, exactly as the engine serves
// `/api/journal/rows?balance=`. What only a mounted table can show is asked
// here — that the two columns appear with a pin and only then, that a carried
// balance is marked as carried, that assertion marks say pass or fail, that a
// posting-dated leg is labelled, and that ✗ narrows the dates.

import {render, screen, within} from "@testing-library/svelte";
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import {dec} from "#lib/domain/money.js";
import type {Amount, AmountStyle, ISODate, Transaction} from "#lib/domain/types.js";
import {filters} from "#lib/stores/filters.svelte.js";
import {running} from "#lib/stores/running.svelte.js";
import {FAKE_ENGINE} from "#lib/testing/fakeEngine.js";
import {showRows, VIEW_GENERATION, wireAmount} from "#lib/testing/journalViewFixture.js";
import TransactionTable from "./TransactionTable.svelte";

const USD: AmountStyle = {side: "L", spaced: false, precision: 2, decimalPoint: ".", digitGroups: [",", [3]]};
const usd = (cents: number): Amount => ({commodity: "$", qty: dec(cents, 2), style: USD});
const ACCOUNT = "assets:checking";

function txn(index: number, date: ISODate, description: string, accounts: [string, string], cents: number, postingDate?: ISODate): Transaction {
    return {
        index,
        date,
        status: "unmarked",
        description,
        code: "",
        comment: "",
        tags: [],
        postings: [
            {account: accounts[0], amounts: [usd(cents)], status: "unmarked", comment: "", tags: [], ...(postingDate === undefined ? {} : {date: postingDate})},
            {account: accounts[1], amounts: [usd(-cents)], status: "unmarked", comment: "", tags: []},
        ],
    };
}

// Display order: newest first.
const TXNS = [
    txn(4, "2026-04-05", "Statement check", [ACCOUNT, "equity:adjust"], 0),
    txn(3, "2026-04-03", "Dividend", ["assets:broker", "income:dividends"], 1200),
    txn(2, "2026-03-28", "Rent", [ACCOUNT, "expenses:rent"], -25000, "2026-04-02"),
    txn(1, "2026-03-01", "Paycheck", [ACCOUNT, "income:salary"], 500000),
];

const RUNNING = [
    {
        change: wireAmount(usd(0)),
        balance: wireAmount(usd(475000)),
        touches: true,
        assertions: [{account: ACCOUNT, op: "=", asserted: wireAmount(usd(500000)), calculated: wireAmount(usd(475000)), ok: false}],
    },
    {change: null, balance: wireAmount(usd(475000)), touches: false, assertions: []},
    {change: wireAmount(usd(-25000)), balance: wireAmount(usd(475000)), touches: true, postedOn: "2026-04-02", assertions: []},
    {
        change: wireAmount(usd(500000)),
        balance: wireAmount(usd(500000)),
        touches: true,
        assertions: [{account: `${ACCOUNT}:joint`, op: "=", asserted: wireAmount(usd(500000)), ok: true}],
    },
];

const PIN = {account: ACCOUNT, commodity: null};

/** The table row for a description. */
const rowOf = (description: string): HTMLElement => screen.getByText(description).closest("tr") as HTMLElement;
const cellOf = (description: string, testid: string): HTMLElement => within(rowOf(description)).getByTestId(testid);

beforeEach(() => {
    filters.reset();
    // jsdom reports 0px; the table needs a width to choose table over cards.
    Object.defineProperty(HTMLElement.prototype, "clientWidth", {configurable: true, get: () => 1024});
});

afterEach(() => {
    vi.unstubAllGlobals();
    Reflect.deleteProperty(HTMLElement.prototype, "clientWidth");
});

describe("COMPONENT TransactionTable running balance", () => {
    it("has no Change or Balance column without a pin", async () => {
        await showRows(TXNS);
        render(TransactionTable);
        expect(screen.queryByRole("columnheader", {name: "Balance"})).toBeNull();
        expect(screen.queryByTestId("running-balance")).toBeNull();
    });

    it("adds both columns after Amount while pinned", async () => {
        await showRows(TXNS, {}, {pin: PIN, running: RUNNING});
        render(TransactionTable);
        const headers = screen.getAllByRole("columnheader").map((th) => th.textContent?.trim());
        expect(headers.slice(-3)).toEqual(["Amount", "Change", "Balance"]);
    });

    it("shows a touching row's change and balance, negatives in the negative colour", async () => {
        await showRows(TXNS, {}, {pin: PIN, running: RUNNING});
        render(TransactionTable);
        expect(cellOf("Paycheck", "running-change").textContent).toContain("$5,000.00");
        expect(cellOf("Paycheck", "running-balance").textContent).toContain("$5,000.00");
        const rent = within(cellOf("Rent", "running-change")).getByText("$-250.00");
        expect(rent.className).toContain("text-error");
    });

    it("leaves Change blank and dims a carried balance on a row that does not touch the account", async () => {
        await showRows(TXNS, {}, {pin: PIN, running: RUNNING});
        render(TransactionTable);
        expect(cellOf("Dividend", "running-change").textContent?.trim()).toBe("");
        const carried = cellOf("Dividend", "running-balance").querySelector("[data-carried]") as HTMLElement;
        expect(carried.textContent).toBe("$4,750.00");
        expect(carried.className).toContain("text-base-content/50");
        expect(cellOf("Paycheck", "running-balance").querySelector("[data-carried]")).toBeNull();
    });

    it("labels a posting-dated leg with the day it posts", async () => {
        await showRows(TXNS, {}, {pin: PIN, running: RUNNING});
        render(TransactionTable);
        expect(within(rowOf("Rent")).getByTestId("running-posted-on").textContent).toBe("posts 2026-04-02");
        expect(within(rowOf("Paycheck")).queryByTestId("running-posted-on")).toBeNull();
    });

    it("marks a passing assertion ✓, naming a child account, and a failing one ✗ with the detail", async () => {
        await showRows(TXNS, {}, {pin: PIN, running: RUNNING});
        render(TransactionTable);
        expect(within(rowOf("Paycheck")).getByTestId("running-assertion-ok").textContent?.trim()).toBe("✓ joint");
        const fail = within(rowOf("Statement check")).getByTestId("running-assertion-fail");
        expect(fail.textContent?.trim()).toBe("✗");
        expect(fail.getAttribute("aria-label")).toBe("Narrow to gap: assets:checking = expected $5,000.00, found $4,750.00, off by $-250.00");
    });

    it("shows a zero change on an internal move, rather than a blank", async () => {
        await showRows(TXNS, {}, {pin: PIN, running: RUNNING});
        render(TransactionTable);
        const zero = within(cellOf("Statement check", "running-change")).getByText("$0.00");
        expect(zero.getAttribute("title")).toBe(`Internal: moves within ${ACCOUNT}`);
    });

    it("narrows the date range to the gap on ✗, leaving the account filter and search alone", async () => {
        await showRows(
            TXNS,
            {
                "/api/journal/running": {
                    generation: VIEW_GENERATION,
                    account: ACCOUNT,
                    commodities: [{commodity: "$", postings: 3}],
                    assertions: {
                        $: [
                            {tindex: 1, date: "2026-03-01", ok: true},
                            {tindex: 4, date: "2026-04-05", ok: false},
                        ],
                    },
                },
            },
            {pin: PIN, running: RUNNING}
        );
        await running.load(FAKE_ENGINE, {account: ACCOUNT, generation: VIEW_GENERATION});
        filters.setAccounts(new Set(["income:dividends"]));
        filters.setQuery("rent");
        render(TransactionTable);
        within(rowOf("Statement check")).getByTestId("running-assertion-fail").click();
        expect({from: filters.value.from, to: filters.value.to}).toEqual({from: "2026-03-02", to: "2026-04-05"});
        expect([...filters.value.accounts]).toEqual(["income:dividends"]);
        expect(filters.value.query).toBe("rent");
    });
});
