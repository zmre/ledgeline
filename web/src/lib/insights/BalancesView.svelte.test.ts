// The Balances view, mounted.
//
// cashBalances.test.ts already proves the arithmetic and the classification, so
// nothing here re-checks a sum. What it asks is what only a mounted component
// can answer: that the figures reach the screen, that an overdrawn account is
// visibly marked as one, that the zero-balance checkbox and the currency
// selector do what they say, and that liabilities and cash never share a list.
//
// jsdom has no layout engine, so nothing here asks how anything LOOKS — the pie
// is `hidden lg:block` and is checked for presence only.

import {render, screen, within} from "@testing-library/svelte";
import {beforeEach, describe, expect, it, vi} from "vitest";
import type {AccountDecl} from "$lib/domain/accountTypes";
import {dec} from "$lib/domain/money";
import type {Amount, AmountStyle, Posting, Transaction} from "$lib/domain/types";
import BalancesView from "./BalancesView.svelte";

// The view is "as of today" by construction, so the fixture's dates have to sit
// in the past relative to whatever day the suite runs on. Freezing the clock is
// cheaper and clearer than dating the fixture relative to now.
vi.mock("$lib/reports/periods", async (importOriginal) => ({
    ...(await importOriginal<typeof import("$lib/reports/periods")>()),
    today: () => "2026-09-13",
}));

const STYLE: AmountStyle = {side: "L", spaced: false, precision: 2, decimalPoint: ".", digitGroups: [",", [3]]};
const usd = (cents: number): Amount => ({commodity: "$", qty: dec(cents, 2), style: STYLE});
// Deliberately a European style — symbol on the right, comma decimal — so
// switching currency has to pick up the COMMODITY's style and not the primary's.
const eur = (cents: number): Amount => ({
    commodity: "EUR",
    qty: dec(cents, 2),
    style: {side: "R", spaced: true, precision: 2, decimalPoint: ",", digitGroups: [".", [3]]},
});

let nextIndex = 0;
function txn(date: string, postings: [string, Amount][]): Transaction {
    nextIndex += 1;
    const built: Posting[] = postings.map(([account, amount]) => ({account, amounts: [amount], status: "unmarked", comment: "", tags: []}));
    return {index: nextIndex, date, status: "cleared", description: "t", code: "", comment: "", tags: [], postings: built, haystack: "t"};
}

const DECLS: AccountDecl[] = [
    {name: "assets:bank:checking", type: "cash"},
    {name: "assets:bank:savings", type: "cash"},
    {name: "assets:wallet:eur", type: "cash"},
    {name: "assets:broker:taxable", type: "asset"},
    {name: "liabilities:card", type: "liability"},
    {name: "liabilities:mortgage", type: "liability", bsterm: "noncurrent"},
];

const TXNS: Transaction[] = [
    txn("2026-01-02", [
        ["assets:bank:checking", usd(300_000)],
        ["equity:opening", usd(-300_000)],
    ]),
    txn("2026-01-02", [
        ["assets:bank:savings", usd(1_000_000)],
        ["equity:opening", usd(-1_000_000)],
    ]),
    txn("2026-01-02", [
        ["assets:broker:taxable", usd(5_000_000)],
        ["equity:opening", usd(-5_000_000)],
    ]),
    txn("2026-01-03", [
        ["liabilities:mortgage", usd(-40_000_000)],
        ["equity:opening", usd(40_000_000)],
    ]),
    txn("2026-08-20", [
        ["expenses:food", usd(45_000)],
        ["liabilities:card", usd(-45_000)],
    ]),
    txn("2026-09-01", [
        ["assets:wallet:eur", eur(20_000)],
        ["equity:opening", eur(-20_000)],
    ]),
];

const mount = (allTxns = TXNS, decls = DECLS) => render(BalancesView, {allTxns, decls});

const tile = (testid: string): string => within(screen.getByTestId(testid)).getByText(/[$€0-9]/).textContent ?? "";
/** The list row whose account label is `account`, as a list item. */
const row = (account: string): HTMLElement => screen.getByText(account, {selector: "bdi"}).closest("li") as HTMLElement;

beforeEach(() => {
    localStorage.clear();
});

describe("COMPONENT BalancesView", () => {
    it("shows cash, short-term liabilities and net cash for the primary commodity", () => {
        mount();
        expect(tile("balances-cash")).toBe("$13,000.00");
        expect(tile("balances-liabilities")).toBe("$-450.00");
        expect(tile("balances-net")).toBe("$12,550.00");
    });

    it("leaves non-cash assets and non-current liabilities out entirely", () => {
        mount();
        // The brokerage and the mortgage are both in the fixture and both larger
        // than anything shown; neither belongs to this question.
        expect(screen.queryByText("assets:broker:taxable", {selector: "bdi"})).toBeNull();
        expect(screen.queryByText("liabilities:mortgage", {selector: "bdi"})).toBeNull();
    });

    it("never intermingles the two lists", () => {
        mount();
        const cash = screen.getByRole("heading", {name: "Cash"}).closest("section") as HTMLElement;
        const debt = screen.getByRole("heading", {name: "Liabilities"}).closest("section") as HTMLElement;
        expect(within(cash).queryByText("assets:bank:savings", {selector: "bdi"})).not.toBeNull();
        expect(within(cash).queryByText("liabilities:card", {selector: "bdi"})).toBeNull();
        expect(within(debt).queryByText("liabilities:card", {selector: "bdi"})).not.toBeNull();
    });

    it("dates each account by its own last movement, not the journal's", () => {
        mount();
        expect(within(row("assets:bank:checking")).queryByText(/As of 2026-01-02/)).not.toBeNull();
        expect(within(row("liabilities:card")).queryByText(/As of 2026-08-20/)).not.toBeNull();
    });

    it("marks a figure a balance assertion confirmed", () => {
        const checked = txn("2026-02-01", [
            ["assets:bank:checking", usd(100)],
            ["equity:opening", usd(-100)],
        ]);
        checked.postings[0] = {...checked.postings[0], balanceAssertion: {amount: usd(3_001_00), inclusive: false, total: false}};
        mount([...TXNS, checked]);
        expect(within(row("assets:bank:checking")).queryByText(/checked/)).not.toBeNull();
        expect(within(row("assets:bank:savings")).queryByText(/checked/)).toBeNull();
    });

    it("highlights an overdrawn cash account, but not an ordinary card balance", () => {
        const overdrawn = txn("2026-09-02", [
            ["expenses:food", usd(400_000)],
            ["assets:bank:checking", usd(-400_000)],
        ]);
        mount([...TXNS, overdrawn]);
        expect(within(row("assets:bank:checking")).getByText("$-1,000.00").className).toContain("text-error");
        // Owing money on a card is the normal state of a card.
        expect(within(row("liabilities:card")).getByText("$-450.00").className).not.toContain("text-error");
    });

    it("hides zero balances by default, and says how many are hidden", async () => {
        const emptied = txn("2026-09-03", [
            ["assets:bank:checking", usd(-300_000)],
            ["expenses:food", usd(300_000)],
        ]);
        mount([...TXNS, emptied]);
        expect(screen.queryByText("assets:bank:checking", {selector: "bdi"})).toBeNull();
        const checkbox = screen.getByRole("checkbox", {name: /Hide zero balances/}) as HTMLInputElement;
        expect(checkbox.checked).toBe(true);
        checkbox.click();
        expect(await screen.findByText("assets:bank:checking", {selector: "bdi"})).not.toBeNull();
    });

    it("switches currency without ever summing across them", async () => {
        mount();
        const select = screen.getByRole("combobox", {name: "Balance currency"}) as HTMLSelectElement;
        expect([...select.options].map((o) => o.value)).toEqual(["$", "EUR"]);
        select.value = "EUR";
        select.dispatchEvent(new Event("change", {bubbles: true}));
        expect(await screen.findByText("assets:wallet:eur", {selector: "bdi"})).not.toBeNull();
        // The dollar accounts are gone rather than converted.
        expect(screen.queryByText("assets:bank:savings", {selector: "bdi"})).toBeNull();
        expect(tile("balances-cash")).toBe("200,00 EUR");
    });

    it("offers no currency selector when the journal uses one commodity", () => {
        mount(TXNS.filter((t) => t.postings.every((p) => p.amounts.every((a) => a.commodity === "$"))));
        expect(screen.queryByRole("combobox", {name: "Balance currency"})).toBeNull();
    });

    describe("the long-term name guess", () => {
        const untagged: AccountDecl[] = [
            {name: "assets:bank:checking", type: "cash"},
            {name: "liabilities:mortgage", type: "liability"},
            {name: "liabilities:card", type: "liability"},
        ];
        const borrowed = txn("2026-04-01", [
            ["liabilities:mortgage", usd(-40_000_000)],
            ["assets:bank:checking", usd(40_000_000)],
        ]);

        it("keeps an untagged mortgage out of the short-term figure", () => {
            mount([...TXNS, borrowed], untagged);
            expect(screen.queryByText("liabilities:mortgage", {selector: "bdi"})).toBeNull();
            // $450 on the card, and not a cent of the $400,000 mortgage.
            expect(tile("balances-liabilities")).toBe("$-450.00");
        });

        it("says so, and says how to override it", () => {
            mount([...TXNS, borrowed], untagged);
            const note = screen.getByTestId("balances-guessed");
            expect(note.textContent).toContain("liabilities:mortgage");
            expect(note.textContent).toContain("bsterm: current");
        });

        it("stays quiet when the exclusion was the owner's own tag", () => {
            // DECLS tags the mortgage `bsterm: noncurrent`; repeating the user's
            // own decision back at them is noise, not disclosure.
            mount();
            expect(screen.queryByTestId("balances-guessed")).toBeNull();
        });
    });

    // jsdom's ResizeObserver stub never fires (componentSetup.ts), so these two
    // exercise the UNMEASURED fallback — the frame before a real browser
    // measures, and the floor everything degrades to. The measured path is
    // AccountLabel's own concern and is tested there.
    describe("account names use the panel's width", () => {
        const named = (account: string) => {
            const t = txn("2026-03-01", [
                [account, usd(100_000)],
                ["equity:opening", usd(-100_000)],
            ]);
            return () => mount([t], [{name: account, type: "cash"}]);
        };

        it("does not abbreviate a name the journal table's 30-character budget would have cut", () => {
            // 38 characters. This is the reported bug: the insights box is far
            // wider than the accounts column, and the column's budget was
            // shortening names that had hundreds of spare pixels.
            const account = "liabilities:creditcards:chase-sapphire";
            named(account)();
            expect(screen.queryByText(account, {selector: "bdi"})).not.toBeNull();
        });

        it("still abbreviates from the ancestors when a name is longer than any fallback", () => {
            const account = "assets:morganstanley:patrick-roth-ira:sweep-cash-account";
            named(account)();
            // The leaf survives; the ancestors are what get spent.
            const shown = screen.getByText(/sweep-cash-account$/, {selector: "bdi"}).textContent;
            expect(shown).not.toBe(account);
            expect(shown).toContain(":sweep-cash-account");
            // The full name is still available to a screen reader and the tooltip.
            expect(screen.queryByText(account, {selector: ".sr-only"})).not.toBeNull();
        });
    });

    it("says what to do instead of showing an empty box when nothing is declared cash", () => {
        mount(TXNS, [
            {name: "assets", type: "asset"},
            {name: "liabilities", type: "equity"},
        ]);
        expect(screen.queryByText(/No cash or short-term liability accounts/)).not.toBeNull();
    });
});
