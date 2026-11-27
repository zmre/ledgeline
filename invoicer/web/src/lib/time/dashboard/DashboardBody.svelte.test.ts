// The time dashboard's figures, mounted on the time fixture's November
// (and October to the 26th): the tiles and their links, the chart's
// columns and tooltip, the tree's toggles, and the top lists with Other.
import {fireEvent, render, screen, within} from "@testing-library/svelte";
import {describe, expect, it, vi} from "vitest";
import DashboardBody from "./DashboardBody.svelte";
import {META, NOVEMBER, NOVEMBER_NOTES, OCTOBER, OCTOBER_NOTES} from "./fixtures.js";
import type {DashMeta} from "./meta.js";
import {factsOf} from "./model.js";
import {presetRange, previousRange} from "../filters/query.js";
import {DEFAULT_QUERY, type DashQuery} from "./query.js";

const range = presetRange("this-month", "2026-11-26");

function mount(query: Partial<DashQuery> = {}, meta: Partial<DashMeta> = {}, compare = true) {
    const onQuery = vi.fn();
    render(DashboardBody, {
        query: {...DEFAULT_QUERY, ...query},
        range,
        previous: compare ? previousRange(range) : null,
        current: factsOf(NOVEMBER),
        prior: compare ? factsOf(OCTOBER) : null,
        notes: NOVEMBER_NOTES,
        priorNotes: compare ? OCTOBER_NOTES : null,
        meta: {...META, ...meta},
        file: "time.journal",
        entriesBase: "/app/time/entries",
        onQuery,
    });
    return onQuery;
}

describe("COMPONENT time dashboard figures", () => {
    it("shows each tile with its change, linked into Entries with the same filters", () => {
        mount({person: "pat"});
        expect(screen.getByTestId("tile-billable-value").textContent).toBe("20h 44m");
        expect(screen.getByTestId("tile-billable-share").textContent).toContain("91% of all time");
        expect(screen.getByTestId("tile-average-value").textContent).toBe("1h 12m");
        expect(screen.getByTestId("tile-days-value").textContent).toBe("7 of 19");
        expect(screen.getByTestId("tile-days-delta").textContent).toContain("+5");
        // This month goes across as This month, so Entries steps from it too.
        expect(screen.getByTestId("tile-total-value").getAttribute("href")).toBe("/app/time/entries?person=pat");
        expect(screen.getByTestId("tile-unbilled-value").getAttribute("href")).toBe("/app/time/entries?person=pat&billed=unbilled");
    });

    it("gives every figure its hledger command (the (i) tooltips, portalled to the body)", () => {
        mount();
        const tips = screen.getAllByRole("tooltip", {hidden: true}).map((t) => t.textContent ?? "");
        const has = (text: string) => tips.some((t) => t.includes(text));
        expect(has("hledger -f time.journal bal -p 2026-11 --depth 0")).toBe(true);
        expect(has("hledger -f time.journal bal -p 2026-11 --tree --depth 2")).toBe(true);
        expect(has("hledger -f time.journal bal -p 2026-11 -D --depth 1")).toBe(true);
        expect(has("hledger -f time.journal print -p 2026-11")).toBe(true);
        expect(screen.getByRole("button", {name: "About total hours"}).getAttribute("aria-describedby")).not.toBeNull();
    });

    it("has no Unbilled tile when no billable account names a customer", () => {
        mount({}, {billableCustomers: false});
        expect(screen.queryByTestId("tile-unbilled")).toBeNull();
    });

    it("draws a column per day, each opening that day's entries, with a tooltip on focus", async () => {
        mount();
        const columns = screen.getAllByTestId("time-dash-column");
        expect(columns).toHaveLength(30);
        const fifth = columns[4];
        expect(fifth.getAttribute("href")).toBe("/app/time/entries?from=2026-11-05&to=2026-11-05");
        expect(fifth.getAttribute("aria-label")).toBe("Thu, Nov 5, 2026: 5h, clients 4h 30m, internal 30m");
        expect(
            within(fifth)
                .getAllByTestId("time-dash-segment")
                .map((s) => s.dataset.series)
        ).toEqual(["clients", "internal"]);
        // One tab stop: the first column; the arrows move along.
        expect(columns.filter((c) => c.tabIndex === 0)).toHaveLength(1);
        await fireEvent.focus(fifth);
        expect(screen.getByTestId("time-dash-tooltip").textContent).toContain("4h 30m");
        await fireEvent.keyDown(fifth, {key: "ArrowRight"});
        expect(document.activeElement).toBe(columns[5]);
    });

    it("draws the weekly target's daily share at day grain, and not by month", () => {
        mount();
        expect(screen.getByTestId("time-dash-legend").textContent).toContain("Target 6h a working day");
        expect(screen.getByTestId("time-dash-target")).toBeTruthy();
    });

    it("asks for another split, grain or target through the URL", async () => {
        const onQuery = mount();
        await fireEvent.change(screen.getByTestId("time-dash-split"), {target: {value: "customer"}});
        expect(onQuery).toHaveBeenLastCalledWith(expect.objectContaining({split: "customer"}), {replace: false});
        await fireEvent.click(screen.getByTestId("time-dash-grain-week"));
        expect(onQuery).toHaveBeenLastCalledWith(expect.objectContaining({grain: "week"}), {replace: false});
        await fireEvent.click(screen.getByTestId("time-dash-target"));
        expect(onQuery).toHaveBeenLastCalledWith(expect.objectContaining({target: false}), {replace: false});
    });

    it("opens and closes tree rows on their own, below and above the depth", async () => {
        mount();
        const accounts = () => screen.getAllByTestId("time-dash-tree-row").map((r) => r.dataset.account);
        expect(accounts()).not.toContain("clients:acme:tax-return");
        await fireEvent.click(screen.getByRole("button", {name: "Expand clients:acme"}));
        expect(accounts()).toContain("clients:acme:tax-return");
        await fireEvent.click(screen.getByRole("button", {name: "Collapse clients"}));
        // internal:training had time only last period: left out, and said so.
        expect(accounts()).toEqual(["clients", "internal", "internal:admin", "internal:old"]);
        expect(screen.getByTestId("time-dash-tree-hidden").textContent).toMatch(/Not shown: \d+ accounts? with no time in this period/);
        await fireEvent.click(screen.getByTestId("time-dash-tree-reset"));
        expect(accounts()).toContain("clients:globex");
        expect(accounts()).not.toContain("clients:acme:tax-return");
    });

    it("lists the top accounts at the depth, customers, people and notes, each linked", () => {
        mount({depth: 3});
        const accounts = screen.getAllByTestId("time-dash-top-accounts-item");
        expect(accounts[0].dataset.key).toBe("clients:globex:audit");
        expect(accounts[0].querySelector("a")?.getAttribute("href")).toBe("/app/time/entries?account=clients%3Aglobex%3Aaudit");
        const customers = screen.getAllByTestId("time-dash-top-customers-item");
        expect(customers.at(-1)?.textContent).toContain("No customer");
        expect(customers.at(-1)?.querySelector("a")).toBeNull();
        expect(screen.getAllByTestId("time-dash-top-people-item").map((p) => p.dataset.key)).toEqual(["sam", "pat", "riley"]);
        const notes = screen.getAllByTestId("time-dash-top-notes-item");
        expect(notes[0].textContent).toContain("2 times");
        expect(
            notes
                .find((n) => n.dataset.key === "email")
                ?.querySelector("a")
                ?.getAttribute("href")
        ).toContain("q=email");
    });

    it("folds past ten into Other, and hides people when one person is shown", () => {
        mount({person: "pat"});
        expect(screen.queryByTestId("time-dash-top-people")).toBeNull();
        expect(screen.queryByTestId("time-dash-top-notes-other")).toBeNull();
        mount({}, {people: ["pat"]});
        expect(screen.queryAllByTestId("time-dash-top-people")).toHaveLength(0);
    });

    it("drops the change column and deltas with comparison off", () => {
        mount({}, {}, false);
        expect(screen.queryByTestId("tile-total-delta")).toBeNull();
        expect(within(screen.getByTestId("time-dash-tree")).queryByText("Change")).toBeNull();
    });
});
