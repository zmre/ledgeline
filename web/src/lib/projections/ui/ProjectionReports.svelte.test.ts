// The six readings, mounted (plan 28).
//
// What only a mount can answer is what each surface was HANDED and whether it
// said it:
//
//   · The runway sentence is the single most load-bearing string this tab
//     produces, and it has to be on screen ABOVE the chart rather than left for
//     a reader to find by eye on a line.
//   · The three statements are the Reports tab's own views, handed the
//     projection's columns and told where the forecast starts — so the divider
//     and the "Projected" label have to be on screen, over the right columns.
//   · The zero rule and the crossing marker belong to the runway alone.
//
// Which NUMBERS reach the chart is `projectionView.test.ts`'s; how each
// statement lays itself out is its own view's test. Nothing asserts on
// geometry: jsdom has no layout engine.

import {fireEvent, render, screen} from "@testing-library/svelte";
import {tick} from "svelte";
import {afterEach, describe, expect, it} from "vitest";
import {dec} from "#lib/domain/money.js";
import type {AmountStyle} from "#lib/domain/types.js";
import {DEFAULT_AMOUNT_STYLE} from "#lib/format/amounts.js";
import {keymap} from "#lib/keys/keymap.svelte.js";
import {accountsFixture, journalFixture, projectionFixture, usd} from "#lib/testing/projectionFixture.js";
import {expectSharedGrid} from "#lib/testing/statementGrid.js";
import type {ProjectionTab} from "../params";
import type {Projection} from "../types";
import ProjectionReports from "./ProjectionReports.svelte";

const STYLES: ReadonlyMap<string, AmountStyle> = new Map([["$", DEFAULT_AMOUNT_STYLE]]);

const projection = (overrides: Partial<Projection> = {}): Projection => projectionFixture(overrides);

function mount(tab: ProjectionTab, value: Projection = projection(), extra: {accountsMode?: "change" | "balance"; emptyAccounts?: boolean} = {}) {
    return render(ProjectionReports, {tab, projection: value, interval: "monthly" as const, styles: STYLES, fallbackCommodity: "$", ...extra});
}

/** The header cells of the first table inside `testid`, in order. */
function headings(testid: string): string[] {
    const head = screen.getByTestId(testid).querySelector("thead tr:not([data-testid])");
    return [...(head?.querySelectorAll("th") ?? [])].map((th) => th.textContent?.trim() ?? "");
}

describe("COMPONENT ProjectionReports — the runway", () => {
    it("states the crossing point in WORDS above the chart", () => {
        mount("cash", projection({runway: {bucket: 1, label: "Mar 2028", date: "2028-03-31", periods: 18}}));
        expect(screen.getByTestId("projection-runway").textContent).toContain("Cash turns negative in Mar 2028 — 18 months.");
        // A crossing is a warning, and is announced as one.
        expect(screen.getByRole("alert")).toBeDefined();
    });

    it("marks the crossing bucket on the chart as well as in the sentence", () => {
        const {container} = mount("cash", projection({runway: {bucket: 1, label: "Sep 2026", date: "2026-09-30", periods: 2}}));
        expect(container.querySelectorAll('[data-rule="mark"]')).toHaveLength(1);
    });

    it("says so plainly when it never crosses, and keeps the zero rule in frame anyway", () => {
        const {container} = mount("cash");
        expect(screen.getByTestId("projection-runway").textContent).toContain("Cash never turns negative over the 2 months projected.");
        expect(screen.getByTestId("projection-runway").textContent).toContain("builds by");
        expect(container.querySelectorAll('[data-rule="zero"]')).toHaveLength(1);
        expect(container.querySelectorAll('[data-rule="mark"]')).toHaveLength(0);
    });

    it("labels the opening balance beside the heading — it is a real figure, not a projected bucket", () => {
        mount("cash");
        expect(screen.getByTestId("projection-cash").textContent).toContain("opening $10,000.00");
        expect(screen.getByTestId("projection-cash").textContent).toContain("from 2026-08-01");
    });

    it("labels the chart with the FORECAST periods only — the runway counts from the cutoff", () => {
        const {container} = mount("cash");
        const ticks = [...container.querySelectorAll('[data-placement="bottom"] .lc-axis-tick-label')].map((t) => t.textContent?.trim());
        expect(ticks).toEqual(["Aug 2026", "Sep 2026"]);
    });
});

describe("COMPONENT ProjectionReports — the statements", () => {
    it("renders the balance sheet through the Reports tab's own view, one column per period", () => {
        mount("bs");
        expect(screen.getByTestId("projection-bs").querySelector('[data-testid="balance-sheet"]')).not.toBeNull();
        expect(headings("bs-section-assets").slice(1)).toEqual(["Jun 2026", "Jul 2026", "Aug 2026", "Sep 2026"]);
    });

    it("draws the divider at the first projected column and says 'Projected' over the forecast", () => {
        mount("bs");
        const box = screen.getByTestId("bs-section-assets");
        const label = box.querySelector('[data-testid="projected-label"]');
        expect(label?.textContent).toBe("Projected");
        // Over the two forecast columns, beside the two actual ones.
        expect(label?.getAttribute("colspan")).toBe("2");
        expect(box.querySelector('[data-testid="projected-band"]')?.textContent).toContain("Actual");
        // The rule is on the first projected column's cells and nowhere else.
        const head = [...(box.querySelector("thead tr:not([data-testid])")?.querySelectorAll("th") ?? [])];
        const ruled = head.map((th) => th.className.includes("border-l-2"));
        expect(ruled).toEqual([false, false, false, true, false]);
        const firstRow = box.querySelector("tbody tr");
        expect([...(firstRow?.querySelectorAll("td") ?? [])].map((td) => td.className.includes("border-l-2"))).toEqual([false, false, true, false]);
    });

    it("renders the income statement through its own view, with the divider", () => {
        mount("is");
        const view = screen.getByTestId("projection-is");
        expect(view.querySelector('[data-testid="income-statement"]')).not.toBeNull();
        expect(view.querySelector('[data-testid="projected-label"]')?.textContent).toBe("Projected");
    });

    it("renders the cash flow statement through its own view, with the divider", () => {
        mount("cf");
        const view = screen.getByTestId("projection-cf");
        expect(view.querySelector('[data-testid="cash-flow-statement"]')).not.toBeNull();
        expect(view.querySelector('[data-testid="projected-label"]')?.textContent).toBe("Projected");
    });

    it("marks nothing as projected when the whole window is forecast and there are no actuals", () => {
        mount("bs", projection({projectedFrom: 0}));
        const band = screen.getByTestId("bs-section-assets").querySelector('[data-testid="projected-band"]');
        // The band still says the columns are projected, and claims no actuals.
        expect(band?.textContent).not.toContain("Actual");
        expect(band?.querySelector('[data-testid="projected-label"]')?.getAttribute("colspan")).toBe("4");
    });
});

describe("COMPONENT ProjectionReports — the tab strip", () => {
    afterEach(() => keymap.reset());

    it("is Runway, then the three statements, then Accounts and Journal", () => {
        mount("cash");
        expect(screen.getAllByRole("tab").map((tab) => tab.textContent?.trim())).toEqual([
            "Runway",
            "Balance sheet",
            "Income statement",
            "Cash flow",
            "Accounts",
            "Journal",
        ]);
    });

    it("switches which reading is on screen", async () => {
        mount("cash");
        expect(screen.queryByTestId("projection-bs")).toBeNull();
        await fireEvent.click(screen.getByRole("tab", {name: "Balance sheet"}));
        expect(screen.getByTestId("projection-bs")).toBeDefined();
        expect(screen.queryByTestId("projection-cash")).toBeNull();
    });

    it("binds one digit per reading, in tab order, filed under Projections in `?`", async () => {
        mount("cash");
        const press = async (key: string) => {
            keymap.handle(new KeyboardEvent("keydown", {key, cancelable: true}));
            await tick();
        };
        await press("2");
        expect(screen.getByTestId("projection-bs")).toBeDefined();
        await press("3");
        expect(screen.getByTestId("projection-is")).toBeDefined();
        await press("4");
        expect(screen.getByTestId("projection-cf")).toBeDefined();
        await press("5");
        expect(screen.getByTestId("projection-accounts")).toBeDefined();
        await press("6");
        expect(screen.getByTestId("projection-journal")).toBeDefined();
        await press("1");
        expect(screen.getByTestId("projection-cash")).toBeDefined();

        const section = keymap.help.find((s) => s.group === "Projections");
        expect(section?.rows.map((b) => `${b.keys} ${b.label}`)).toEqual([
            "1 Runway",
            "2 Balance sheet",
            "3 Income statement",
            "4 Cash flow",
            "5 Accounts",
            "6 Journal",
        ]);
    });

    it("brings the strip to the top of the window when a reading is picked, by click or by digit", async () => {
        const calls: {strip: Element; options: unknown}[] = [];
        const original = Element.prototype.scrollIntoView;
        Element.prototype.scrollIntoView = function (this: Element, options?: unknown): void {
            calls.push({strip: this, options});
        };
        try {
            mount("cash");
            const strip = screen.getByRole("tablist");
            await fireEvent.click(screen.getByRole("tab", {name: "Journal"}));
            keymap.handle(new KeyboardEvent("keydown", {key: "2", cancelable: true}));
            await tick();
            expect(calls).toEqual([
                {strip, options: {block: "start", behavior: "smooth"}},
                {strip, options: {block: "start", behavior: "smooth"}},
            ]);
        } finally {
            Element.prototype.scrollIntoView = original;
        }
    });

    it("is at least a window tall, so a short panel cannot pull the strip back down", () => {
        mount("journal");
        expect(screen.getByRole("tablist").parentElement?.classList.contains("min-h-dvh")).toBe(true);
    });

    it("marks the active tab for a screen reader, not only with a class", () => {
        mount("is");
        expect(screen.getByRole("tab", {name: "Income statement"}).getAttribute("aria-selected")).toBe("true");
        expect(screen.getByRole("tab", {name: "Runway"}).getAttribute("aria-selected")).toBe("false");
    });
});

describe("COMPONENT ProjectionReports — a scenario in more than one commodity", () => {
    it("names what charting one commodity leaves out rather than dropping it silently", () => {
        mount(
            "cash",
            projection({
                cash: {
                    opening: new Map([
                        ["$", dec(10000, 0)],
                        ["EUR", dec(50, 0)],
                    ]),
                    values: [usd(14800), usd(19600)],
                },
            })
        );
        expect(screen.getByTestId("projection-uncharted").textContent).toContain("EUR");
    });

    it("says nothing when there is nothing left out", () => {
        mount("cash");
        expect(screen.queryByTestId("projection-uncharted")).toBeNull();
    });
});

describe("COMPONENT ProjectionReports — accounts", () => {
    const rowsOf = (): string[] =>
        [...screen.getByTestId("accounts-table").querySelectorAll("tbody tr[data-account]")].map((tr) => tr.getAttribute("data-account") ?? "");

    it("renders every account per period on the statements' column template, with the band and a Total", () => {
        mount("accounts", projection({accounts: accountsFixture("change")}));
        const table = screen.getByTestId("accounts-table");
        expect(headings("accounts-table")).toEqual(["Account", "Jun 2026", "Jul 2026", "Aug 2026", "Sep 2026", "Total"]);
        expect(table.querySelector('[data-testid="projected-label"]')?.getAttribute("colspan")).toBe("2");
        const cells = [...(table.querySelector('tbody tr[data-account="expenses:rent"]')?.querySelectorAll("td") ?? [])];
        expect(cells.map((td) => td.textContent?.trim())).toEqual(["$4,200.00", "$4,200.00", "$4,200.00", "$4,200.00", "$16,800.00"]);
        // The divider down the first projected column, and nowhere else.
        expect(cells.map((td) => td.className.includes("border-l-2"))).toEqual([false, false, true, false, false]);
        // One fixed template, every row filling it: the shared-grid assertion,
        // over a stack of one (plus the same table again so the helper's
        // "more than one block" holds).
        expectSharedGrid([table, table], 5, true);
    });

    it("draws Balance without a Total column — snapshots mean nothing summed", () => {
        mount("accounts", projection({accounts: accountsFixture("balance")}), {accountsMode: "balance"});
        expect(headings("accounts-table")).toEqual(["Account", "Jun 2026", "Jul 2026", "Aug 2026", "Sep 2026"]);
        expect(screen.getByRole("button", {name: "Balance"}).getAttribute("aria-pressed")).toBe("true");
        expect(screen.getByRole("button", {name: "Change"}).getAttribute("aria-pressed")).toBe("false");
    });

    it("indents the tree and folds a parent into its only child", () => {
        mount("accounts", projection({accounts: accountsFixture("change")}));
        // `assets` equals `assets:checking` in every column, so the chain folds.
        expect(rowsOf()).toContain("assets:checking");
        expect(rowsOf()).not.toContain("assets");
    });

    it("toggles Change/Balance and Show empty accounts through its bindings", async () => {
        const view = mount("accounts", projection({accounts: accountsFixture("change")}));
        const checkbox = screen.getByRole("checkbox", {name: "Show empty accounts"}) as HTMLInputElement;
        expect(checkbox.checked).toBe(false);
        await fireEvent.click(screen.getByRole("button", {name: "Balance"}));
        expect(screen.getByRole("button", {name: "Balance"}).getAttribute("aria-pressed")).toBe("true");
        await fireEvent.click(checkbox);
        expect(checkbox.checked).toBe(true);
        view.unmount();
    });

    it("says it is reading while a projection held from another tab has no accounts yet", () => {
        mount("accounts");
        expect(screen.getByTestId("projection-accounts-pending")).toBeDefined();
        expect(screen.queryByTestId("accounts-table")).toBeNull();
    });
});

describe("COMPONENT ProjectionReports — journal", () => {
    it("lists the generated transactions as hledger print does, naming each one's origin", () => {
        mount("journal", projection({journal: journalFixture(2)}));
        const entries = screen.getAllByTestId("forecast-entry");
        expect(entries).toHaveLength(3);
        expect(entries.map((entry) => entry.getAttribute("data-origin"))).toEqual(["rule", "rule", "event"]);
        const first = entries[0]!;
        expect(first.textContent).toContain("2026-08-01");
        expect(first.textContent).toContain("projection");
        expect(first.textContent).toContain("~ monthly");
        expect(first.textContent).toContain("expenses:rent");
        expect(first.textContent).toContain("$4,200.00");
        expect(first.textContent).toContain("$-4,200.00");
        expect(entries[2]!.textContent).toContain("new laptop");
        expect(entries[2]!.textContent).toContain("event");
        expect(screen.getByTestId("forecast-journal-count").textContent).toContain("1–3 of 3");
    });

    it("pages a long horizon a hundred at a time", async () => {
        mount("journal", projection({journal: journalFixture(150)}));
        expect(screen.getAllByTestId("forecast-entry")).toHaveLength(100);
        expect(screen.getByTestId("forecast-journal-count").textContent).toContain("1–100 of 151");
        await fireEvent.click(screen.getAllByRole("button", {name: "Next page"})[0]!);
        expect(screen.getAllByTestId("forecast-entry")).toHaveLength(51);
        expect(screen.getByTestId("forecast-journal-count").textContent).toContain("101–151 of 151");
    });

    it("says when the engine capped the list", () => {
        const journal = {...journalFixture(2), total: 9000, truncated: true};
        mount("journal", projection({journal}));
        expect(screen.getByTestId("forecast-journal-count").textContent).toContain("the first 3 are listed");
    });

    it("filters by text across descriptions and accounts, case-insensitively", async () => {
        mount("journal", projection({journal: journalFixture(2)}));
        const box = screen.getByTestId("forecast-journal-search");
        await fireEvent.input(box, {target: {value: "  New LAPTOP "}});
        expect(screen.getAllByTestId("forecast-entry").map((entry) => entry.getAttribute("data-origin"))).toEqual(["event"]);
        expect(screen.getByTestId("forecast-journal-count").textContent).toContain("Matching 1–1 of 1");
        // An account matches too — every entry posts to `expenses:rent`.
        await fireEvent.input(box, {target: {value: "rent"}});
        expect(screen.getAllByTestId("forecast-entry")).toHaveLength(3);
        await fireEvent.input(box, {target: {value: "no such thing"}});
        expect(screen.queryAllByTestId("forecast-entry")).toHaveLength(0);
        expect(screen.getByTestId("forecast-journal-count").textContent).toContain("No generated transaction matches “no such thing”");
        await fireEvent.input(box, {target: {value: ""}});
        expect(screen.getAllByTestId("forecast-entry")).toHaveLength(3);
    });

    it("starts a new search on its first page", async () => {
        mount("journal", projection({journal: journalFixture(150)}));
        await fireEvent.click(screen.getAllByRole("button", {name: "Next page"})[0]!);
        expect(screen.getByTestId("forecast-journal-count").textContent).toContain("101–151 of 151");
        await fireEvent.input(screen.getByTestId("forecast-journal-search"), {target: {value: "projection"}});
        expect(screen.getByTestId("forecast-journal-count").textContent).toContain("Matching 1–100 of 150");
    });

    it("says a search of a capped list covers only what was listed", async () => {
        const journal = {...journalFixture(2), total: 9000, truncated: true};
        mount("journal", projection({journal}));
        await fireEvent.input(screen.getByTestId("forecast-journal-search"), {target: {value: "rent"}});
        expect(screen.getByTestId("forecast-journal-count").textContent).toContain("searching the first 3 of 9000 listed");
    });

    it("says it is listing while a projection held from another tab has no journal yet", () => {
        mount("journal");
        expect(screen.getByTestId("projection-journal-pending")).toBeDefined();
    });
});
