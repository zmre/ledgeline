// The Balances view, mounted.
//
// The engine computes the view (`/api/balances/cash`; its arithmetic and its
// classification are pinned by crates/ledgeline-core/tests/cash_balances_golden.rs,
// against goldens captured from the TypeScript it replaced). What only a mounted
// component can answer is what is asked here: that the figures reach the
// screen, that an overdrawn account is visibly marked as one, that the
// zero-balance checkbox and the currency selector do what they say, and that
// liabilities and cash never share a list.
//
// jsdom has no layout engine, so nothing here asks how anything LOOKS — the pie
// is `hidden lg:block` and is checked for presence only.

import {render, screen, within} from "@testing-library/svelte";
import {tick} from "svelte";
import {beforeEach, describe, expect, it} from "vitest";
import {add, dec, type Dec} from "#lib/domain/money.js";
import type {AmountStyle} from "#lib/domain/types.js";
import type {BalanceRow, BalanceSummary, CashBalancesView} from "#lib/journalView/types.js";
import {filters} from "#lib/stores/filters.svelte.js";
import {balanceSeries} from "#lib/testing/balanceSeriesFixture.js";
import BalancesView from "./BalancesView.svelte";

const USD: AmountStyle = {side: "L", spaced: false, precision: 2, decimalPoint: ".", digitGroups: [",", [3]]};
// Deliberately a European style — symbol on the right, comma decimal — so
// switching currency has to pick up the COMMODITY's style and not the primary's.
const EUR: AmountStyle = {side: "R", spaced: true, precision: 2, decimalPoint: ",", digitGroups: [".", [3]]};

const row = (account: string, cents: number, asOf = "2026-01-02", confirmed = false): BalanceRow => ({account, qty: dec(cents, 2), asOf, confirmed});
const sum = (rows: BalanceRow[]): Dec => rows.reduce((total, r) => add(total, r.qty), dec(0, 0));

function summary(commodity: string, style: AmountStyle, cash: BalanceRow[], liabilities: BalanceRow[]): BalanceSummary {
    const totalCash = sum(cash);
    const totalLiabilities = sum(liabilities);
    return {commodity, style, cash, liabilities, totalCash, totalLiabilities, net: add(totalCash, totalLiabilities)};
}

function view(summaries: BalanceSummary[], guessedLongTerm: string[] = []): CashBalancesView {
    return {asOf: "2026-09-13", commodities: summaries.map((s) => s.commodity), summaries, guessedLongTerm};
}

const DOLLARS = summary(
    "$",
    USD,
    [row("assets:bank:savings", 1_000_000), row("assets:bank:checking", 300_000)],
    [row("liabilities:card", -45_000, "2026-08-20")]
);
const EUROS = summary("EUR", EUR, [row("assets:wallet:eur", 20_000, "2026-09-01")], []);
const VIEW = view([DOLLARS, EUROS]);

const mount = (v: CashBalancesView = VIEW) => render(BalancesView, {view: v});

const tile = (testid: string): string => within(screen.getByTestId(testid)).getByText(/[$€0-9]/).textContent ?? "";
/** The list row whose account label is `account`, as a list item. */
const listRow = (account: string): HTMLElement => screen.getByText(account, {selector: "bdi"}).closest("li") as HTMLElement;

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
        expect(within(listRow("assets:bank:checking")).queryByText(/As of 2026-01-02/)).not.toBeNull();
        expect(within(listRow("liabilities:card")).queryByText(/As of 2026-08-20/)).not.toBeNull();
    });

    it("marks a figure a balance assertion confirmed", () => {
        mount(view([summary("$", USD, [row("assets:bank:checking", 300_100, "2026-02-01", true), row("assets:bank:savings", 100)], [])]));
        expect(within(listRow("assets:bank:checking")).queryByText(/checked/)).not.toBeNull();
        expect(within(listRow("assets:bank:savings")).queryByText(/checked/)).toBeNull();
    });

    it("highlights an overdrawn cash account, but not an ordinary card balance", () => {
        mount(view([summary("$", USD, [row("assets:bank:checking", -100_000)], [row("liabilities:card", -45_000)])]));
        expect(within(listRow("assets:bank:checking")).getByText("$-1,000.00").className).toContain("text-error");
        // Owing money on a card is the normal state of a card.
        expect(within(listRow("liabilities:card")).getByText("$-450.00").className).not.toContain("text-error");
    });

    it("hides zero balances by default, and says how many are hidden", async () => {
        mount(view([summary("$", USD, [row("assets:bank:checking", 0), row("assets:bank:savings", 100)], [])]));
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
        mount(view([DOLLARS]));
        expect(screen.queryByRole("combobox", {name: "Balance currency"})).toBeNull();
    });

    describe("the long-term name guess", () => {
        it("says so, and says how to override it", () => {
            mount(view([DOLLARS], ["liabilities:mortgage"]));
            const note = screen.getByTestId("balances-guessed");
            expect(note.textContent).toContain("liabilities:mortgage");
            expect(note.textContent).toContain("bsterm: current");
        });

        it("stays quiet when nothing was excluded on a guess", () => {
            mount();
            expect(screen.queryByTestId("balances-guessed")).toBeNull();
        });
    });

    // jsdom's ResizeObserver stub never fires (componentSetup.ts), so these two
    // exercise the UNMEASURED fallback — the frame before a real browser
    // measures, and the floor everything degrades to. The measured path is
    // AccountLabel's own concern and is tested there.
    describe("account names use the panel's width", () => {
        const named = (account: string) => () => mount(view([summary("$", USD, [row(account, 100_000, "2026-03-01")], [])]));

        it("does not abbreviate a name the journal table's 30-character budget would have cut", () => {
            const account = "liabilities:creditcards:chase-sapphire";
            named(account)();
            expect(screen.queryByText(account, {selector: "bdi"})).not.toBeNull();
        });

        it("still abbreviates from the ancestors when a name is longer than any fallback", () => {
            const account = "assets:morganstanley:patrick-roth-ira:sweep-cash-account";
            named(account)();
            const shown = screen.getByText(/sweep-cash-account$/, {selector: "bdi"}).textContent;
            expect(shown).not.toBe(account);
            expect(shown).toContain(":sweep-cash-account");
            expect(screen.queryByText(account, {selector: ".sr-only"})).not.toBeNull();
        });
    });

    it("says what to do instead of showing an empty box when nothing is declared cash", () => {
        mount(view([]));
        expect(screen.queryByText(/No cash or short-term liability accounts/)).not.toBeNull();
    });

    describe("a row opens the account in the journal with its running balance (plan 36)", () => {
        beforeEach(() => filters.reset());

        it("filters to the account and pins it on click, keeping the date range", () => {
            filters.setRange("2026-02-01", "2026-02-28");
            mount();
            (listRow("assets:bank:checking").querySelector("button") as HTMLButtonElement).click();
            expect([...filters.value.accounts]).toEqual(["assets:bank:checking"]);
            expect(filters.value.pin?.account).toBe("assets:bank:checking");
            expect({from: filters.value.from, to: filters.value.to}).toEqual({from: "2026-02-01", to: "2026-02-28"});
        });

        it("is a real button, so Enter and Tab reach it", () => {
            mount();
            const button = listRow("liabilities:card").querySelector("button") as HTMLButtonElement;
            expect(button.type).toBe("button");
            button.focus();
            expect(document.activeElement).toBe(button);
        });

        it("carries the currency on screen when there is a choice", async () => {
            mount();
            const select = screen.getByRole("combobox", {name: "Balance currency"}) as HTMLSelectElement;
            select.value = "EUR";
            select.dispatchEvent(new Event("change", {bubbles: true}));
            await tick();
            (listRow("assets:wallet:eur").querySelector("button") as HTMLButtonElement).click();
            expect(filters.value.pin).toEqual({account: "assets:wallet:eur", commodity: "EUR"});
        });

        it("leaves the commodity to the engine when there is only one", () => {
            mount(view([DOLLARS]));
            (listRow("assets:bank:savings").querySelector("button") as HTMLButtonElement).click();
            expect(filters.value.pin).toEqual({account: "assets:bank:savings", commodity: null});
        });
    });

    describe("a pinned account's balance line replaces the pie (amendment A1)", () => {
        beforeEach(() => filters.reset());

        /** The line for `assets:bank:savings`, the account the tests pin. */
        const SAVINGS = balanceSeries({account: "assets:bank:savings"});

        it("shows the pie with no pin, the line with one, and the pie again on unpin", async () => {
            const {rerender} = render(BalancesView, {view: VIEW, trend: null});
            expect(screen.queryByTestId("balances-pie")).not.toBeNull();
            expect(screen.queryByTestId("balances-line")).toBeNull();

            (listRow("assets:bank:savings").querySelector("button") as HTMLButtonElement).click();
            await rerender({view: VIEW, trend: SAVINGS});
            expect(screen.queryByTestId("balances-pie")).toBeNull();
            expect(screen.getByTestId("balances-line").querySelector('[data-testid="balance-trend"]')).not.toBeNull();
            expect(screen.getByTestId("balances-line").textContent).toContain("assets:bank:savings");

            filters.unpin();
            await tick();
            expect(screen.queryByTestId("balances-line")).toBeNull();
            expect(screen.queryByTestId("balances-pie")).not.toBeNull();
        });

        it("hides the whole-journal totals while pinned, and brings them back on unpin", async () => {
            render(BalancesView, {view: VIEW, trend: null});
            const totals = ["balances-cash", "balances-liabilities", "balances-net"];
            for (const id of totals) expect(screen.queryByTestId(id), id).not.toBeNull();

            filters.pin("assets:bank:savings");
            await tick();
            for (const id of totals) expect(screen.queryByTestId(id), id).toBeNull();

            filters.unpin();
            await tick();
            for (const id of totals) expect(screen.queryByTestId(id), id).not.toBeNull();
        });

        it("highlights the pinned row, and drops the pie's colour dots with the pie", async () => {
            filters.pin("assets:bank:savings");
            render(BalancesView, {view: VIEW, trend: SAVINGS});
            await tick();
            const pinned = listRow("assets:bank:savings").querySelector("button") as HTMLButtonElement;
            const other = listRow("assets:bank:checking").querySelector("button") as HTMLButtonElement;
            expect(pinned.getAttribute("aria-current")).toBe("true");
            expect(pinned.className).toContain("bg-primary/15");
            expect(other.getAttribute("aria-current")).toBeNull();
            expect(other.className).not.toContain("bg-primary/15");
            expect(listRow("assets:bank:checking").querySelector(".rounded-full")).toBeNull();
        });

        it("charts an account the list does not show, such as one pinned from a report", () => {
            filters.pin("assets:brokerage");
            render(BalancesView, {view: VIEW, trend: balanceSeries({account: "assets:brokerage"})});
            expect(screen.getByTestId("balances-line").textContent).toContain("assets:brokerage");
        });

        it("waits rather than showing another account's line", () => {
            filters.pin("assets:bank:checking");
            render(BalancesView, {view: VIEW, trend: SAVINGS});
            const slot = screen.getByTestId("balances-line");
            expect(slot.querySelector('[data-testid="balance-trend"]')).toBeNull();
            expect(slot.querySelector('[aria-label="Loading balance line"]')).not.toBeNull();
            // ... and never falls back to the pie while it waits.
            expect(screen.queryByTestId("balances-pie")).toBeNull();
        });

        it("says why when the line could not be loaded", () => {
            filters.pin("assets:bank:checking");
            render(BalancesView, {view: VIEW, trend: null, trendError: new Error("no posting is in the account")});
            expect(screen.getByTestId("balances-line").textContent).toContain("Couldn't chart assets:bank:checking");
        });

        it("still charts a pin when no cash account holds the commodity", () => {
            filters.pin("assets:bank:savings");
            render(BalancesView, {view: view([]), trend: SAVINGS});
            expect(screen.queryByText(/No cash or short-term liability accounts/)).not.toBeNull();
            expect(screen.getByTestId("balances-line").querySelector('[data-testid="balance-trend"]')).not.toBeNull();
        });
    });
});
