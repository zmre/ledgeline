// The four readings, mounted (plan 28).
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
import {projectionFixture, usd} from "#lib/testing/projectionFixture.js";
import type {ProjectionTab} from "../params";
import type {Projection} from "../types";
import ProjectionReports from "./ProjectionReports.svelte";

const STYLES: ReadonlyMap<string, AmountStyle> = new Map([["$", DEFAULT_AMOUNT_STYLE]]);

const projection = (overrides: Partial<Projection> = {}): Projection => projectionFixture(overrides);

function mount(tab: ProjectionTab, value: Projection = projection()) {
    return render(ProjectionReports, {tab, projection: value, interval: "monthly" as const, styles: STYLES, fallbackCommodity: "$"});
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

    it("is Runway, then the three statements", () => {
        mount("cash");
        expect(screen.getAllByRole("tab").map((tab) => tab.textContent?.trim())).toEqual(["Runway", "Balance sheet", "Income statement", "Cash flow"]);
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
        await press("1");
        expect(screen.getByTestId("projection-cash")).toBeDefined();

        const section = keymap.help.find((s) => s.group === "Projections");
        expect(section?.rows.map((b) => `${b.keys} ${b.label}`)).toEqual(["1 Runway", "2 Balance sheet", "3 Income statement", "4 Cash flow"]);
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
