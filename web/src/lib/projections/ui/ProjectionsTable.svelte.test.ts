// The what-if table, mounted, driving the REAL scenario store.
//
// The two claims only a mount can answer (web/README.md):
//
//   1. WHAT THE TABLE WAS HANDED. A seeded scenario's rows carry
//      `source: "unbudgeted"`, and the whole point of that flag is that the user
//      is told the figure is an average of their history rather than something
//      they wrote. Every pure function behind it can be green while the badge is
//      never rendered.
//   2. WHAT AN EDIT DOES TO THE STORE. "Edited" is a store fact reached through
//      a DOM event, and the wiring between the two — `onChange` → `touch()` —
//      is exactly the seam Phase 3's save button will hang off.
//
// The scenario goes in as WIRE BYTES through the real `decodeScenario`, not as
// a hand-built domain object: the figures below are copied from the committed
// `fixtures/native/v1/projections-seed.json`, so this file exercises the same
// decoding path the tab does. (The bytes are inlined rather than read from
// disk: this project runs under jsdom, where `import.meta.url` is not a `file:`
// URL and `readFileSync` cannot resolve one. The golden itself is swept by
// `nativeDecode.test.ts`, which does run under node.)
//
// Nothing asserts on geometry, visibility-by-overlap or computed CSS: jsdom has
// no layout engine. The row menu is a native <details>, so its items are in the
// document whether or not it is open, which is what makes them clickable here.

import {fireEvent, render, screen, within} from "@testing-library/svelte";
import {beforeEach, describe, expect, it} from "vitest";
import {decodeScenario} from "$lib/api/nativeDecode";
import type {AccountType} from "$lib/domain/accountTypes";
import {scenarioStore} from "../scenarioStore.svelte";
import type {Scenario} from "../types";
import ProjectionsTable from "./ProjectionsTable.svelte";

const NOTE = "unbudgeted — average over 2025-08-01 to 2026-07-08";

const seedLine = (account: string, mantissa: string) => ({
    id: `gap:${account}:$`,
    group: `gap:${account}:$`,
    account,
    amount: {commodity: "$", quantity: {mantissa, places: 2}, precision: 2},
    period: {raw: "monthly", simple: "monthly", from: null, to: null},
    growth: null,
    note: NOTE,
    source: "unbudgeted",
});

const SEED = decodeScenario({
    name: "",
    created: null,
    updated: null,
    lines: [
        // Revenue is NEGATIVE on the wire, exactly as the posting is written.
        seedLine("income:salary", "-518833"),
        seedLine("income:dividends", "-729"),
        seedLine("expenses:housing", "187500"),
        seedLine("expenses:taxes", "133833"),
    ],
    events: [],
});

const ACCOUNT_NAMES = ["income:salary", "income:dividends", "expenses:housing", "expenses:taxes", "assets:checking"];
const DECLARED: ReadonlyMap<string, AccountType> = new Map<string, AccountType>([
    ["income", "revenue"],
    ["expenses", "expense"],
    ["assets", "asset"],
]);

function mount(scenario: Scenario = scenarioStore.scenario) {
    return render(ProjectionsTable, {
        scenario,
        accountNames: ACCOUNT_NAMES,
        declared: DECLARED,
        commodity: "$",
        stepDate: "2027-04-01",
        onChange: () => scenarioStore.touch(),
    });
}

// The store is a module singleton shared by every test in this FILE, so each
// test re-adopts the seed to get back to a known, not-dirty state.
beforeEach(() => scenarioStore.adopt(SEED));

describe("COMPONENT ProjectionsTable — a seeded scenario", () => {
    it("splits the seed into Income and Expenses by the account's TYPE", () => {
        mount();
        const income = within(screen.getByTestId("projection-section-income"));
        const expenses = within(screen.getByTestId("projection-section-expense"));
        expect(income.getAllByTestId("projection-line")).toHaveLength(2);
        expect(expenses.getAllByTestId("projection-line")).toHaveLength(2);
        expect(income.getByLabelText("Amount for income:salary")).toBeDefined();
        expect(expenses.getByLabelText("Amount for expenses:housing")).toBeDefined();
        // …and no seeded revenue row has leaked into the outflow half.
        expect(expenses.queryByLabelText("Amount for income:salary")).toBeNull();
    });

    it("shows a seeded row's amount as a MAGNITUDE, though the wire signs revenue negative", () => {
        mount();
        // The golden's `income:salary` is -5188.33 on the wire.
        expect(screen.getByLabelText("Amount for income:salary")).toHaveProperty("value", "5188.33");
        expect(screen.getByLabelText("Amount for expenses:housing")).toHaveProperty("value", "1875.00");
    });

    it("FLAGS every unbudgeted row as estimated, with the window it was averaged over", () => {
        mount();
        const badges = screen.getAllByTestId("estimated-badge");
        expect(badges.length).toBe(SEED.lines.filter((l) => l.source === "unbudgeted").length);
        expect(badges.length).toBeGreaterThan(0);
        expect(badges[0].getAttribute("title")).toContain("average over");
    });

    it("does NOT flag a row the user wrote", () => {
        scenarioStore.adopt({...SEED, lines: SEED.lines.map((line) => ({...line, source: "journal" as const}))});
        mount();
        expect(screen.queryAllByTestId("estimated-badge")).toHaveLength(0);
    });

    it("opens not-dirty: seeding is not an edit", () => {
        mount();
        expect(scenarioStore.dirty).toBe(false);
    });
});

describe("COMPONENT ProjectionsTable — editing marks the scenario dirty", () => {
    it("typing an amount re-signs it for the account and marks the scenario edited", async () => {
        mount();
        expect(scenarioStore.dirty).toBe(false);

        await fireEvent.change(screen.getByLabelText("Amount for income:salary"), {target: {value: "6000"}});

        expect(scenarioStore.dirty).toBe(true);
        const salary = scenarioStore.scenario.lines.find((l) => l.account === "income:salary");
        // A magnitude in the box; the journal's sign in the model.
        expect(salary?.amount.quantity).toEqual({m: -6000n, p: 0});
        // …and the display precision the seed carried is not lowered by it.
        expect(salary?.amount.precision).toBe(2);
    });

    it("an amount that is not a number is ignored rather than zeroing the row", async () => {
        mount();
        const before = scenarioStore.scenario.lines.find((l) => l.account === "expenses:housing")?.amount.quantity;
        await fireEvent.change(screen.getByLabelText("Amount for expenses:housing"), {target: {value: "about a lot"}});
        expect(scenarioStore.scenario.lines.find((l) => l.account === "expenses:housing")?.amount.quantity).toEqual(before);
    });

    it("a growth rate typed as a percent is held as a FRACTION", async () => {
        mount();
        await fireEvent.change(screen.getByLabelText("Growth rate for expenses:housing, in percent"), {target: {value: "2.5"}});
        const housing = scenarioStore.scenario.lines.find((l) => l.account === "expenses:housing");
        expect(housing?.growth).toEqual({rate: {m: 25n, p: 3}, unit: "year"});
        expect(scenarioStore.dirty).toBe(true);
    });

    it("clearing the growth box removes the growth rather than holding a zero one", async () => {
        mount();
        const box = screen.getByLabelText("Growth rate for expenses:housing, in percent");
        await fireEvent.change(box, {target: {value: "3"}});
        await fireEvent.change(box, {target: {value: ""}});
        expect(scenarioStore.scenario.lines.find((l) => l.account === "expenses:housing")?.growth).toBeNull();
    });

    it("changing the interval rewrites `raw`, which is the only field the engine reads back", async () => {
        mount();
        await fireEvent.change(screen.getByLabelText("Period for expenses:housing"), {target: {value: "quarterly"}});
        expect(scenarioStore.scenario.lines.find((l) => l.account === "expenses:housing")?.period).toEqual({
            raw: "quarterly",
            simple: "quarterly",
            from: null,
            to: null,
        });
    });

    it("setting a `to` bound spells the period the way a journal would", async () => {
        mount();
        await fireEvent.change(screen.getByLabelText("To date for expenses:housing (exclusive)"), {target: {value: "2028-01-01"}});
        expect(scenarioStore.scenario.lines.find((l) => l.account === "expenses:housing")?.period.raw).toBe("monthly to 2028-01-01");
    });
});

describe("COMPONENT ProjectionsTable — the row menu", () => {
    /**
     * A named button inside the row whose Amount box belongs to `account`.
     *
     * The account is the VALUE of a text input, not text in the row, so the row
     * is found through its labelled amount box and walked up from there. The
     * FIRST match: once a row is a step, its segments share an account, and the
     * row menu lives on the first of them.
     */
    const menuItem = (account: string, label: string): HTMLElement => {
        const row = screen.getAllByLabelText(`Amount for ${account}`)[0].closest("tr");
        const found = [...(row?.querySelectorAll("button") ?? [])].find((button) => button.textContent?.trim() === label);
        if (found === undefined) throw new Error(`no "${label}" in the row menu for ${account}`);
        return found;
    };

    it("adds a step: one logical row becomes two bounded segments meeting on the date", async () => {
        mount();
        await fireEvent.click(menuItem("expenses:housing", "Add a step on 2027-04-01"));

        const segments = scenarioStore.scenario.lines.filter((l) => l.account === "expenses:housing");
        expect(segments.map((s) => s.period.raw)).toEqual(["monthly to 2027-04-01", "monthly from 2027-04-01"]);
        // Two `~` RULES, so two groups — the engine's cash guard is scoped per group.
        expect(new Set(segments.map((s) => s.group)).size).toBe(2);
        expect(scenarioStore.dirty).toBe(true);
    });

    it("removes the step again, giving back the row that was split", async () => {
        mount();
        const before = scenarioStore.scenario.lines.filter((l) => l.account === "expenses:housing").map((l) => l.period.raw);
        await fireEvent.click(menuItem("expenses:housing", "Add a step on 2027-04-01"));
        await fireEvent.click(menuItem("expenses:housing", "Remove the step"));
        expect(scenarioStore.scenario.lines.filter((l) => l.account === "expenses:housing").map((l) => l.period.raw)).toEqual(before);
    });

    it("deletes a row and everything under its id", async () => {
        mount();
        await fireEvent.click(menuItem("expenses:housing", "Add a step on 2027-04-01"));
        await fireEvent.click(menuItem("expenses:housing", "Delete row"));
        expect(scenarioStore.scenario.lines.some((l) => l.account === "expenses:housing")).toBe(false);
    });

    it("adds a line to the section the button is in, on a prefix of the right type", async () => {
        mount();
        const before = scenarioStore.scenario.lines.length;
        const incomeSection = screen.getByTestId("projection-section-income");
        const add = [...incomeSection.querySelectorAll("button")].find((b) => b.textContent?.includes("Add a line"));
        await fireEvent.click(add as HTMLElement);
        expect(scenarioStore.scenario.lines).toHaveLength(before + 1);
        const added = scenarioStore.scenario.lines[before];
        expect(added.account).toBe("income:");
        // Authored, not estimated — the user is writing it.
        expect(added.source).toBe("journal");
        expect(added.period.raw).toBe("monthly");
    });
});

describe("COMPONENT ProjectionsTable — one-off events", () => {
    it("adds an event on the offered date, with a SIGNED amount box", async () => {
        mount();
        await fireEvent.click(screen.getByRole("button", {name: "+ Add an event"}));
        expect(scenarioStore.scenario.events).toHaveLength(1);
        expect(scenarioStore.scenario.events[0].date).toBe("2027-04-01");

        // Signed, deliberately: an event is where a balance-sheet movement is
        // written and the direction is the user's, not the account type's.
        await fireEvent.change(screen.getByLabelText("Amount for a new posting on 2027-04-01"), {target: {value: "-2000000"}});
        expect(scenarioStore.scenario.events[0].postings[0].amount.quantity).toEqual({m: -2000000n, p: 0});
    });

    it("shows a single-date `~` rule among the one-offs, though the engine keeps it a line", () => {
        const dated = SEED.lines[0];
        scenarioStore.adopt({
            ...SEED,
            lines: [{...dated, id: "buy", group: "buy", account: "expenses:housing", period: {raw: "2027-03-01", simple: null, from: null, to: null}}],
        });
        mount();
        const oneoff = within(screen.getByTestId("projection-section-oneoff"));
        expect(oneoff.getByLabelText("Amount for the one-off expenses:housing")).toBeDefined();
        expect(within(screen.getByTestId("projection-section-expense")).queryByTestId("projection-line")).toBeNull();
        expect(oneoff.getByLabelText("Date of the one-off expenses:housing")).toHaveProperty("value", "2027-03-01");
    });
});
