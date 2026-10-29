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
// no layout engine. The row menu is PORTALLED to <body> and mounted only while
// it is open (`RowMenu.svelte`), so every test that wants an item opens the menu
// through its trigger — the route a user takes. The previous version of this
// file read menu items straight out of the row, which a native <details>
// allowed, and that is exactly why nothing here noticed the menus were clipped
// off screen.

import {fireEvent, render, screen, within} from "@testing-library/svelte";
import {tick} from "svelte";
import {beforeEach, describe, expect, it} from "vitest";
import {decodeScenario} from "#lib/api/nativeDecode.js";
import {classesOf} from "#lib/domain/accountClassFixtures.js";
import {scenarioStore} from "../scenarioStore.svelte";
import type {AssetRow, ForecastDefaults, Scenario} from "../types";
import ProjectionsTable from "./ProjectionsTable.svelte";

const NOTE = "unbudgeted — average over 2025-08-01 to 2026-07-08";

// `role` and `opening` are on the wire since plan 23 Phase 2 and the decoder
// DEMANDS both — an absent `role` would silently model an asset row as an
// outflow the size of its contribution, so there is no default. A seeded gap is
// always a flow: `budget_gaps` measures revenue and expense accounts only.
const seedLine = (account: string, mantissa: string) => ({
    id: `gap:${account}:$`,
    group: `gap:${account}:$`,
    role: "flow",
    account,
    amount: {commodity: "$", quantity: {mantissa, places: 2}, precision: 2},
    period: {raw: "monthly", simple: "monthly", base: null, from: null, to: null},
    growth: null,
    opening: null,
    note: NOTE,
    counter: null,
    gains: null,
    source: "unbudgeted",
    // The engine's placement (plan 26 B6), as the seed sends it.
    section: account.startsWith("income:") ? "income" : "expense",
});

const SEED_WIRE = {
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
    cash: null,
    autoRules: [] as unknown[],
    accountTypes: {},
};
const SEED = decodeScenario(SEED_WIRE);

/**
 * An ASSET row on the wire: `role: "asset"`, its `amount` the per-period
 * CONTRIBUTION, and `opening` null so the table shows the journal's own figure.
 */
const assetLine = (account: string, group: string, contribution = "0") => ({
    id: group,
    group,
    role: "asset",
    account,
    amount: {commodity: "$", quantity: {mantissa: contribution, places: 2}, precision: 2},
    period: {raw: "monthly", simple: "monthly", base: null, from: null, to: null},
    growth: {rate: {mantissa: "7", places: 2}, unit: "year"},
    opening: null,
    note: "",
    counter: null,
    gains: null,
    source: "journal",
    section: "asset",
});

const ACCOUNT_NAMES = ["income:salary", "income:dividends", "expenses:housing", "expenses:taxes", "assets:checking", "assets:brokerage", "assets:house"];
const DECLARED = classesOf(["income", "revenue"], ["expenses", "expense"], ["assets", "asset"]);

const STYLES = new Map([["$", {side: "L" as const, spaced: false, precision: 2, decimalPoint: ".", digitGroups: [",", [3]] as [string, number[]]}]]);

/** One attributed asset row, as `Projection.assets` carries it. */
const attributed = (group: string, account: string, held: bigint): AssetRow => ({
    group,
    account,
    journalOpening: new Map([["$", {m: held, p: 2}]]),
    opening: new Map([["$", {m: held, p: 2}]]),
    growth: new Map([["$", {m: 0n, p: 2}]]),
});

function mount(
    scenario: Scenario = scenarioStore.scenario,
    assetBalances: ReadonlyMap<string, AssetRow> = new Map(),
    defaults: ForecastDefaults | null = null
) {
    return render(ProjectionsTable, {
        scenario,
        accountNames: ACCOUNT_NAMES,
        declared: DECLARED,
        commodity: "$",
        stepDate: "2027-04-01",
        assetBalances,
        defaults,
        styles: STYLES,
        onChange: () => scenarioStore.touch(),
    });
}

// The store is a module singleton shared by every test in this FILE, so each
// test re-adopts the seed to get back to a known, not-dirty state.
beforeEach(() => scenarioStore.adopt(SEED));

// --- Reaching a row -------------------------------------------------------
//
// Rows are addressed by their line's `group`, which is unique per segment and
// is on the `<tr>`. The account is the VALUE of a text input, not text in the
// row, so it cannot be searched for.

const rowOf = (group: string): HTMLElement | null => document.querySelector(`[data-line-group="${group}"]`);

const fieldOf = (group: string): HTMLInputElement => {
    const field = rowOf(group)?.querySelector<HTMLInputElement>('input[role="combobox"]');
    if (field === null || field === undefined) throw new Error(`no account field in the row ${group}`);
    return field;
};

const sectionOf = (group: string): string | null => rowOf(group)?.closest("[data-testid^=projection-section-]")?.getAttribute("data-testid") ?? null;

/** Press a section's "+ Add a line", and hand back the line it appended. */
async function addLineIn(section: "income" | "expense") {
    const buttons = [...screen.getByTestId(`projection-section-${section}`).querySelectorAll("button")];
    const add = buttons.find((b) => b.textContent?.includes("Add a line"));
    await fireEvent.click(add as HTMLElement);
    await tick();
    return scenarioStore.scenario.lines[scenarioStore.scenario.lines.length - 1];
}

/** `scheduleRelease` defers by one task, deliberately — see the component header. */
const settle = (): Promise<void> => new Promise((resolve) => setTimeout(resolve, 0));

/** Focus a field the way a browser does, firing the `focusin` the hold listens for. */
async function focus(field: HTMLInputElement): Promise<void> {
    field.focus();
    await fireEvent.focusIn(field);
}

/** Blur a field, then let the deferred release run. */
async function blur(field: HTMLInputElement): Promise<void> {
    field.blur();
    await fireEvent.focusOut(field);
    await settle();
    await tick();
}

/**
 * Type `typed` into `field` ONE CHARACTER AT A TIME, asserting after every
 * keystroke that the row stays in `section`, keeps its DOM node, and keeps the
 * caret.
 *
 * Per keystroke is the whole point: a section re-derived from the account text
 * flips on a single letter (`` is not revenue and neither is `r`), and a flip
 * between two `{#each}` blocks destroys the row's node and takes the caret with
 * it. An assertion only at the end would pass on a row that moved twice.
 *
 * `from` is where the prefix loop starts — the asset row asserts the empty
 * string too, since its section cannot depend on the account at all; the flow
 * row starts at one character, which is where its hold takes over.
 */
async function staysPutWhileTyping(field: HTMLInputElement, group: string, typed: string, section: string, from: number): Promise<void> {
    for (let n = from; n <= typed.length; n += 1) {
        await fireEvent.input(field, {target: {value: typed.slice(0, n)}});
        await tick();

        const after = typed.slice(0, n);
        expect(`${after}: ${sectionOf(group)}`).toBe(`${after}: projection-section-${section}`);
        // Same NODE, still focused: a recreated input would be neither.
        expect(fieldOf(group)).toBe(field);
        expect(document.activeElement).toBe(field);
    }
}

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

    it("says where a seeded number came from in the row's description, not a badge", () => {
        mount();
        expect(screen.queryAllByTestId("estimated-badge")).toHaveLength(0);
        const salary = screen.getByLabelText("Description for income:salary");
        const described = document.getElementById(salary.getAttribute("aria-describedby") ?? "");
        expect(described?.textContent).toContain("average over");
    });

    it("opens not-dirty: seeding is not an edit", () => {
        mount();
        expect(scenarioStore.dirty).toBe(false);
    });
});

describe("COMPONENT ProjectionsTable — editing marks the scenario dirty", () => {
    it("typing an amount keeps it as the line's entry for the engine to sign, and marks the scenario edited", async () => {
        mount();
        expect(scenarioStore.dirty).toBe(false);

        await fireEvent.change(screen.getByLabelText("Amount for income:salary"), {target: {value: "6000"}});

        expect(scenarioStore.dirty).toBe(true);
        const salary = scenarioStore.scenario.lines.find((l) => l.account === "income:salary");
        // A magnitude in the box and in the model; the ENGINE signs it (plan 26 B6).
        expect(salary?.entry).toEqual({m: 6000n, p: 0});
        expect((screen.getByLabelText("Amount for income:salary") as HTMLInputElement).value).toBe("6000");
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
            base: null,
            from: null,
            to: null,
        });
    });

    it("setting a `to` bound spells the period the way a journal would", async () => {
        mount();
        await fireEvent.change(screen.getByLabelText("To date for expenses:housing (exclusive)"), {target: {value: "2028-01-01"}});
        expect(scenarioStore.scenario.lines.find((l) => l.account === "expenses:housing")?.period.raw).toBe("monthly to 2028-01-01");
    });

    it("REGRESSION: an UNSET bound wears a placeholder hint, not WebKit's today", async () => {
        mount();
        const cellOf = (label: string) => screen.getByLabelText(label).closest("td") as HTMLElement;
        const to = "To date for expenses:housing (exclusive)";
        // WebKit paints today into an empty date input; the hint must cover it.
        expect(within(cellOf(to)).queryByTestId("date-unset")).not.toBeNull();
        expect(screen.getByLabelText(to).className).toContain("opacity-0");

        await fireEvent.change(screen.getByLabelText(to), {target: {value: "2028-01-01"}});
        await tick();
        expect(within(cellOf(to)).queryByTestId("date-unset")).toBeNull();
        expect(screen.getByLabelText(to).className).not.toContain("opacity-0");
    });
});

// REGRESSION: `~ monthly from 2026-11-01  <description>` showed its whole `raw`
// in the Per column and locked From/To, because the engine answered `simple`
// only for an UNBOUNDED interval. The periods below are what the engine now
// sends for each form (`projection_endpoints.rs` pins that side).
const periodLine = (account: string, period: Record<string, string | null>, note: string) => ({
    ...seedLine(account, "10000"),
    id: account,
    group: account,
    period,
    note,
    source: "journal",
    section: "expense",
});

const BOUNDED = decodeScenario({
    name: "",
    created: null,
    updated: null,
    lines: [
        periodLine("expenses:a", {raw: "monthly from 2026-11-01", simple: "monthly", base: null, from: "2026-11-01", to: null}, NOTE),
        periodLine("expenses:b", {raw: "monthly to 2027-06-01", simple: "monthly", base: null, from: null, to: "2027-06-01"}, ""),
        periodLine("expenses:c", {raw: "yearly from 2026-11-01 to 2030-01-01", simple: "yearly", base: null, from: "2026-11-01", to: "2030-01-01"}, "ends"),
        periodLine("expenses:d", {raw: "every 3 months from 2026-11-01", simple: null, base: "every 3 months", from: "2026-11-01", to: null}, ""),
        periodLine(
            "expenses:e",
            {raw: "every 15th day of month from 2026-11-01", simple: null, base: "every 15th day of month", from: "2026-11-01", to: null},
            "mid-month"
        ),
        periodLine("expenses:f", {raw: "every weekday", simple: null, base: null, from: null, to: null}, "workdays"),
    ],
    events: [],
    cash: null,
    autoRules: [],
    accountTypes: {},
});

describe("COMPONENT ProjectionsTable — REGRESSION: bounded periods stay editable", () => {
    beforeEach(() => scenarioStore.adopt(BOUNDED));

    const periodOf = (account: string) => scenarioStore.scenario.lines.find((l) => l.account === account)?.period;
    const from = (account: string) => screen.getByLabelText(`From date for ${account}`) as HTMLInputElement;
    const to = (account: string) => screen.getByLabelText(`To date for ${account} (exclusive)`) as HTMLInputElement;

    it.each([
        ["expenses:a", "monthly", "2026-11-01", ""],
        ["expenses:b", "monthly", "", "2027-06-01"],
        ["expenses:c", "yearly", "2026-11-01", "2030-01-01"],
    ])("%s selects %s in Per and shows editable bounds", (account, interval, start, end) => {
        mount();
        expect((screen.getByLabelText(`Period for ${account}`) as HTMLSelectElement).value).toBe(interval);
        expect(from(account).disabled).toBe(false);
        expect(to(account).disabled).toBe(false);
        expect(from(account).value).toBe(start);
        expect(to(account).value).toBe(end);
    });

    it("moving a bounded rule's `from` rewrites `raw` and keeps the description", async () => {
        mount();
        await fireEvent.change(from("expenses:a"), {target: {value: "2026-12-01"}});
        expect(periodOf("expenses:a")?.raw).toBe("monthly from 2026-12-01");
        expect(scenarioStore.scenario.lines.find((l) => l.account === "expenses:a")?.note).toBe(NOTE);
        await fireEvent.change(from("expenses:a"), {target: {value: ""}});
        expect(periodOf("expenses:a")?.raw).toBe("monthly");
    });

    it("changing a bounded rule's interval keeps both bounds", async () => {
        mount();
        await fireEvent.change(screen.getByLabelText("Period for expenses:c"), {target: {value: "quarterly"}});
        expect(periodOf("expenses:c")?.raw).toBe("quarterly from 2026-11-01 to 2030-01-01");
    });

    it.each([
        ["expenses:d", "every 3 months"],
        ["expenses:e", "every 15th day of month"],
    ])("%s shows its words as written and still re-bounds around them", async (account, words) => {
        mount();
        const cell = rowOf(account)?.querySelector('[data-testid="period-as-written"]');
        expect(cell?.textContent?.trim()).toBe(words);
        expect(from(account).value).toBe("2026-11-01");
        await fireEvent.change(to(account), {target: {value: "2027-06-01"}});
        expect(periodOf(account)?.raw).toBe(`${words} from 2026-11-01 to 2027-06-01`);
    });

    it("a recurrence the engine cannot rebuild locks its bounds and says why", () => {
        mount();
        const cell = rowOf("expenses:f")?.querySelector('[data-testid="period-as-written"]');
        expect(cell?.textContent?.trim()).toBe("every weekday");
        for (const box of [from("expenses:f"), to("expenses:f")]) {
            expect(box.disabled).toBe(true);
            expect(box.title).toContain("can't be edited here");
            expect(box.closest("td")?.title).toContain("every weekday");
        }
    });
});

/**
 * Open the menu named for `row` and click the item named `label`.
 *
 * Through the trigger, because the menu is mounted only while open and lives in
 * <body> when it is. `getByLabelText` also enforces the other half of defect 2:
 * a row whose name is ambiguous cannot be found here at all.
 */
async function chooseInMenu(row: string, label: string): Promise<void> {
    await fireEvent.click(screen.getByLabelText(`Row menu for ${row}`));
    await tick();
    await fireEvent.click(within(screen.getByTestId("row-menu")).getByRole("menuitem", {name: label}));
    await tick();
}

describe("COMPONENT ProjectionsTable — the row menu", () => {
    it("adds a step: one logical row becomes two bounded segments meeting on the date", async () => {
        mount();
        await chooseInMenu("expenses:housing", "Add a step on 2027-04-01");

        const segments = scenarioStore.scenario.lines.filter((l) => l.account === "expenses:housing");
        expect(segments.map((s) => s.period.raw)).toEqual(["monthly to 2027-04-01", "monthly from 2027-04-01"]);
        // Two `~` RULES, so two groups — the engine's cash guard is scoped per group.
        expect(new Set(segments.map((s) => s.group)).size).toBe(2);
        expect(scenarioStore.dirty).toBe(true);
    });

    it("removes the step again, giving back the row that was split", async () => {
        mount();
        const before = scenarioStore.scenario.lines.filter((l) => l.account === "expenses:housing").map((l) => l.period.raw);
        await chooseInMenu("expenses:housing", "Add a step on 2027-04-01");
        await chooseInMenu("expenses:housing", "Remove the step");
        expect(scenarioStore.scenario.lines.filter((l) => l.account === "expenses:housing").map((l) => l.period.raw)).toEqual(before);
    });

    it("deletes a row and everything under its id", async () => {
        mount();
        await chooseInMenu("expenses:housing", "Add a step on 2027-04-01");
        await chooseInMenu("expenses:housing", "Delete row");
        expect(scenarioStore.scenario.lines.some((l) => l.account === "expenses:housing")).toBe(false);
    });

    it("adds a line to the section the button is in, on a prefix of the right type", async () => {
        mount();
        const before = scenarioStore.scenario.lines.length;
        const added = await addLineIn("income");
        expect(scenarioStore.scenario.lines).toHaveLength(before + 1);
        expect(added.account).toBe("income:");
        // Authored, not estimated — the user is writing it.
        expect(added.source).toBe("journal");
        expect(added.period.raw).toBe("monthly");
    });

    it("REGRESSION: every row's menu has a name of its OWN, blank account or not", async () => {
        // Defect 2. Two rows added to Income share the seeded `income:` prefix
        // and every blank row used to be "a new row", so neither a screen
        // reader nor a click could say which `⋯` it meant — leaving the
        // half-typed row a user most wants gone with no reachable delete.
        mount();
        const first = await addLineIn("income");
        const second = await addLineIn("income");

        expect(screen.getByLabelText("Row menu for income: (row 3 of Income)")).toBeDefined();
        expect(screen.getByLabelText("Row menu for income: (row 4 of Income)")).toBeDefined();

        await chooseInMenu("income: (row 4 of Income)", "Delete row");
        expect(scenarioStore.scenario.lines.some((l) => l.id === second.id)).toBe(false);
        expect(scenarioStore.scenario.lines.some((l) => l.id === first.id)).toBe(true);
    });

    it("REGRESSION: a row with an EMPTY account is still deletable", async () => {
        // The row a user most wants to get rid of, and the one that used to
        // have no addressable delete at all.
        mount();
        const added = await addLineIn("income");
        const field = fieldOf(added.group);
        await focus(field);
        await fireEvent.input(field, {target: {value: ""}});
        await tick();

        await chooseInMenu("row 3 of Income", "Delete row");

        expect(scenarioStore.scenario.lines.some((l) => l.id === added.id)).toBe(false);
    });
});

describe("COMPONENT ProjectionsTable — THE SECTION-STABILITY RULE", () => {
    it("REGRESSION: a new row stays put, and keeps focus, through EVERY keystroke", async () => {
        // THE defect, and the worst of the six. A line's section was re-derived
        // from the account text on every render: `` is not a revenue account
        // and neither is `r`, so a row added under Income was born in Expenses
        // and only flipped back on the final letter of `revenues:…`. Income and
        // Expenses are two different `{#each}` blocks, so each flip DESTROYED
        // the row's DOM node and rebuilt it elsewhere — taking the caret with
        // it. The user hit this on a single letter.
        mount();
        const added = await addLineIn("income");
        const field = fieldOf(added.group);
        await focus(field);
        expect(sectionOf(added.group)).toBe("projection-section-income");

        await staysPutWhileTyping(field, added.group, "revenues:consulting", "income", 1);
    });

    it("REGRESSION: retyping an EXISTING row's account does not move it mid-word either", async () => {
        // The same bug from the other end. `expenses:housing` retyped towards
        // `income:…` passes through `i`, `in`, `inc` — none of them revenue,
        // all of them a different section from where it is heading.
        mount();
        const housing = scenarioStore.scenario.lines.find((l) => l.account === "expenses:housing");
        const field = fieldOf(housing?.group ?? "");
        await focus(field);

        for (const text of ["", "i", "in", "inco", "income:consulting"]) {
            await fireEvent.input(field, {target: {value: text}});
            await tick();
            expect(`${text}: ${sectionOf(housing?.group ?? "")}`).toBe(`${text}: projection-section-expense`);
            expect(document.activeElement).toBe(field);
        }
    });

    it("moves the row once focus LEAVES it, and says so out loud", async () => {
        mount();
        const housing = scenarioStore.scenario.lines.find((l) => l.account === "expenses:housing");
        const group = housing?.group ?? "";
        const field = fieldOf(group);
        await focus(field);
        await fireEvent.input(field, {target: {value: "income:consulting"}});
        await tick();
        expect(sectionOf(group)).toBe("projection-section-expense");

        await blur(field);

        expect(sectionOf(group)).toBe("projection-section-income");
        // Perceivable, not silent: a row that relocates without a word is
        // indistinguishable from one that was lost.
        expect(screen.getByTestId("section-move-notice").textContent?.trim()).toContain("Moved income:consulting to Income");
    });

    it("does NOT announce a move that did not happen", async () => {
        mount();
        const salary = scenarioStore.scenario.lines.find((l) => l.account === "income:salary");
        const field = fieldOf(salary?.group ?? "");
        await focus(field);
        await fireEvent.input(field, {target: {value: "income:consulting"}});
        await tick();

        await blur(field);

        expect(sectionOf(salary?.group ?? "")).toBe("projection-section-income");
        expect(screen.getByTestId("section-move-notice").textContent?.trim()).toBe("");
    });

    it("REGRESSION: a row left BLANK is not reclassified into Expenses on the way out", async () => {
        // A row with no account cannot be classified, and filing it under
        // Expenses because of that is the original defect arriving one event
        // later — on exactly the row that is still being written.
        mount();
        const added = await addLineIn("income");
        const field = fieldOf(added.group);
        await focus(field);
        await fireEvent.input(field, {target: {value: ""}});
        await tick();

        await blur(field);

        expect(sectionOf(added.group)).toBe("projection-section-income");
    });

    it("holds a new row in the section whose button made it", async () => {
        // The hold is written at birth, not at first focus: a row added under
        // Income has to be under Income before anybody types anything, and the
        // seeded prefix cannot promise that — the next keystroke can delete it.
        mount();
        const added = await addLineIn("income");

        expect(added.section).toBe("income");
        expect(sectionOf(added.group)).toBe("projection-section-income");
    });

    it("a row the user never touched is still placed by its account's TYPE", async () => {
        // The hold is for editing, not a replacement for the type rule. A
        // loaded scenario carries no holds at all.
        mount();
        const income = within(screen.getByTestId("projection-section-income"));
        expect(income.getAllByTestId("projection-line")).toHaveLength(2);
        expect(scenarioStore.scenario.lines.every((l) => l.section === undefined)).toBe(true);
    });
});

// --- Assets and balances (plan 23, Phase 3) --------------------------------

/** The seed, plus asset rows, decoded through the real wire decoder. */
function withAssets(...lines: ReturnType<typeof assetLine>[]): Scenario {
    return decodeScenario({
        name: "",
        created: null,
        updated: null,
        lines: [seedLine("expenses:housing", "187500"), ...lines],
        events: [],
        cash: null,
        autoRules: [],
        accountTypes: {},
    });
}

const assetRowOf = (group: string): HTMLElement | null => document.querySelector(`[data-testid="projection-asset-line"][data-line-group="${group}"]`);

describe("COMPONENT ProjectionsTable — the Assets and balances section", () => {
    it("renders a THIRD section, keyed off the row's role and not its account", () => {
        // `assets:` is not a revenue account, so before this section existed
        // every one of these rows landed under "Expenses and other outflows".
        scenarioStore.adopt(withAssets(assetLine("assets:brokerage", "brok"), assetLine("assets:house", "house")));
        mount();

        const assets = within(screen.getByTestId("projection-section-asset"));
        expect(assets.getAllByTestId("projection-asset-line")).toHaveLength(2);
        expect(assets.getByLabelText("Contribution for assets:brokerage")).toBeDefined();
        expect(assets.getByLabelText("Growth rate for the assets:house balance, in percent")).toHaveProperty("value", "7");
        // …and neither has leaked into the outflow half.
        const expenses = within(screen.getByTestId("projection-section-expense"));
        expect(expenses.getAllByTestId("projection-line")).toHaveLength(1);
        expect(expenses.queryByLabelText("Contribution for assets:brokerage")).toBeNull();
    });

    it("a FLOW row posting to an asset account is still an outflow", () => {
        // The discriminator is `role`, both ways round. `assets:checking` is a
        // legitimate destination for a recurring transfer, and reading the
        // account instead would reclassify it into a compounding balance.
        scenarioStore.adopt(
            decodeScenario({
                name: "",
                created: null,
                updated: null,
                lines: [seedLine("assets:checking", "50000")],
                events: [],
                cash: null,
                autoRules: [],
                accountTypes: {},
            })
        );
        mount();
        expect(within(screen.getByTestId("projection-section-expense")).getAllByTestId("projection-line")).toHaveLength(1);
        expect(within(screen.getByTestId("projection-section-asset")).queryAllByTestId("projection-asset-line")).toHaveLength(0);
    });

    it("adds an asset row with role: asset, while the flow sections keep making flows", async () => {
        mount();
        await fireEvent.click(screen.getByRole("button", {name: "+ Add an asset"}));
        await tick();
        const added = scenarioStore.scenario.lines[scenarioStore.scenario.lines.length - 1];

        expect(added.role).toBe("asset");
        // Seeded into the asset tree, and with no section HOLD: this row is
        // held here by its role, which nothing on it can edit.
        expect(added.account).toBe("assets:");
        expect(added.section).toBeUndefined();
        expect(added.opening).toBeNull();
        expect(added.amount.quantity).toEqual({m: 0n, p: 2});
        expect(assetRowOf(added.group)).not.toBeNull();

        // …and the existing sections are untouched.
        const flow = await addLineIn("income");
        expect(flow.role).toBe("flow");
    });

    it("names an asset row's controls distinctly from a FLOW row on the same account", async () => {
        // A monthly transfer into `assets:checking` beside an `assets:checking`
        // row earning interest is an ordinary thing to write, and `rowNames`
        // can only make a name unique within its own section. Two identically
        // named "Growth rate for assets:checking" boxes would leave a screen
        // reader — and `getByLabelText` — with no way to say which was which.
        scenarioStore.adopt(
            decodeScenario({
                name: "",
                created: null,
                updated: null,
                lines: [seedLine("assets:checking", "50000"), assetLine("assets:checking", "brok")],
                events: [],
                cash: null,
                autoRules: [],
                accountTypes: {},
            })
        );
        mount();

        expect(screen.getByLabelText("Growth rate for assets:checking, in percent")).toBeDefined();
        expect(screen.getByLabelText("Growth rate for the assets:checking balance, in percent")).toBeDefined();
        expect(screen.getByLabelText("Row menu for assets:checking")).toBeDefined();
        expect(screen.getByLabelText("Row menu for the assets:checking balance")).toBeDefined();
        // `getByLabelText` throws on more than one match, so reaching each of
        // those at all IS the assertion.
    });

    it("offers no Add a step on an asset row — two segments would compound the same balance twice", async () => {
        scenarioStore.adopt(withAssets(assetLine("assets:brokerage", "brok")));
        mount();
        await fireEvent.click(screen.getByLabelText("Row menu for the assets:brokerage balance"));
        await tick();
        const items = within(screen.getByTestId("row-menu"))
            .getAllByRole("menuitem")
            .map((item) => item.textContent?.trim());
        expect(items).toEqual(["Duplicate", "Delete row"]);
    });
});

describe("COMPONENT ProjectionsTable — the Balance column", () => {
    it("shows the JOURNAL's figure, greyed, with the box left empty", () => {
        scenarioStore.adopt(withAssets(assetLine("assets:brokerage", "brok")));
        mount(scenarioStore.scenario, new Map([["brok", attributed("brok", "assets:brokerage", 51_230_000n)]]));

        const box = screen.getByLabelText("Balance for assets:brokerage") as HTMLInputElement;
        // Empty value + the ledger's figure as the placeholder IS "greyed until
        // you type over it". Nothing has been overridden, so the model is null.
        expect(box.value).toBe("");
        expect(box.placeholder).toBe("512300.00");
        expect(box.title).toBe("Your journal says $512,300.00");
        expect(scenarioStore.scenario.lines.find((l) => l.group === "brok")?.opening).toBeNull();
    });

    it("a typed balance is DISTINGUISHABLE, and the ledger's own figure stays on screen beside it", async () => {
        scenarioStore.adopt(withAssets(assetLine("assets:brokerage", "brok")));
        mount(scenarioStore.scenario, new Map([["brok", attributed("brok", "assets:brokerage", 51_230_000n)]]));

        await fireEvent.change(screen.getByLabelText("Balance for assets:brokerage"), {target: {value: "640000"}});
        await tick();

        const line = scenarioStore.scenario.lines.find((l) => l.group === "brok");
        expect(line?.opening).toEqual({commodity: "$", quantity: {m: 640000n, p: 0}, precision: 2});
        expect(scenarioStore.dirty).toBe(true);
        // Visually distinct: the box wears the warning style the untouched one
        // does not…
        const box = screen.getByLabelText("Balance for assets:brokerage");
        expect(box.closest("label")?.className).toContain("input-warning");
        // …and THE FAILURE MODE THIS COLUMN EXISTS TO PREVENT: the ledger's own
        // number is still readable beside the number the user typed.
        expect(screen.getByTestId("ledger-balance").textContent?.trim()).toBe("ledger $512,300.00");
    });

    it("clears back to the journal's, from the button and from an emptied box", async () => {
        scenarioStore.adopt(withAssets(assetLine("assets:brokerage", "brok")));
        mount(scenarioStore.scenario, new Map([["brok", attributed("brok", "assets:brokerage", 51_230_000n)]]));
        const box = screen.getByLabelText("Balance for assets:brokerage");

        await fireEvent.change(box, {target: {value: "640000"}});
        await tick();
        await fireEvent.click(screen.getByLabelText("Use the journal's balance for assets:brokerage"));
        await tick();

        expect(scenarioStore.scenario.lines.find((l) => l.group === "brok")?.opening).toBeNull();
        expect(screen.queryByTestId("ledger-balance")).toBeNull();

        // Select-all-and-delete means the same thing, and is the way most
        // people will reach for it.
        await fireEvent.change(box, {target: {value: "640000"}});
        await tick();
        await fireEvent.change(screen.getByLabelText("Balance for assets:brokerage"), {target: {value: ""}});
        await tick();
        expect(scenarioStore.scenario.lines.find((l) => l.group === "brok")?.opening).toBeNull();
    });

    it("says nothing rather than zero before anything has been projected", () => {
        // A row the engine has not walked yet, and a row it declined to walk
        // (a non-asset account — the warnings above the charts say why), reach
        // this the same way: no entry at all.
        scenarioStore.adopt(withAssets(assetLine("assets:brokerage", "brok")));
        mount();
        expect(screen.getByLabelText("Balance for assets:brokerage")).toHaveProperty("placeholder", "—");
    });

    it("refuses an attribution computed for the account this row USED to name", () => {
        // The stale case. A figure shown under a freshly-typed account is a
        // claim about the ledger that the ledger is not making — and the
        // recompute that would correct it is a debounce away.
        scenarioStore.adopt(withAssets(assetLine("assets:brokerage", "brok")));
        mount(scenarioStore.scenario, new Map([["brok", attributed("brok", "assets:house", 51_230_000n)]]));
        expect(screen.getByLabelText("Balance for assets:brokerage")).toHaveProperty("placeholder", "—");
    });
});

describe("COMPONENT ProjectionsTable — an asset row and THE SECTION-STABILITY RULE", () => {
    it("REGRESSION: an asset row stays put, and keeps focus, through EVERY keystroke", async () => {
        // The same regression as the Income one above, written the same way,
        // for the section added after it. It passes for a stronger reason: an
        // asset row's section is its `role`, which no box on the row edits, so
        // there is no hold involved and nothing for a half-typed account to
        // flip.
        scenarioStore.adopt(withAssets(assetLine("assets:brokerage", "brok")));
        mount(scenarioStore.scenario, new Map([["brok", attributed("brok", "assets:brokerage", 51_230_000n)]]));

        const field = fieldOf("brok");
        await focus(field);
        expect(sectionOf("brok")).toBe("projection-section-asset");

        // Typed here is a route that would move a FLOW row twice — through
        // `income:…`, which is revenue, and out the other side.
        await staysPutWhileTyping(field, "brok", "income:consulting", "asset", 0);

        // …and it does not move on the way OUT either, which is where the flow
        // sections do their reconciling.
        await blur(field);
        expect(sectionOf("brok")).toBe("projection-section-asset");
        expect(screen.getByTestId("section-move-notice").textContent?.trim()).toBe("");
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
            lines: [
                {...dated, id: "buy", group: "buy", account: "expenses:housing", period: {raw: "2027-03-01", simple: null, base: null, from: null, to: null}},
            ],
        });
        mount();
        const oneoff = within(screen.getByTestId("projection-section-oneoff"));
        expect(oneoff.getByLabelText("Amount for the one-off expenses:housing")).toBeDefined();
        expect(within(screen.getByTestId("projection-section-expense")).queryByTestId("projection-line")).toBeNull();
        expect(oneoff.getByLabelText("Date of the one-off expenses:housing")).toHaveProperty("value", "2027-03-01");
    });
});

describe("COMPONENT ProjectionsTable — counter-accounts, gains accounts and the cash default (plan 28)", () => {
    const DEFAULTS: ForecastDefaults = {
        cash: "assets:checking",
        gains: "equity:unrealized gains",
        lines: new Map([["gap:income:salary:$", "assets:checking"]]),
        events: new Map(),
    };

    /** Type into an account combobox and leave it, which is what commits it. */
    async function typeAccount(field: HTMLElement, value: string): Promise<void> {
        await fireEvent.input(field, {target: {value}});
        await fireEvent.blur(field);
        await tick();
    }

    it("shows the engine's inferred counter-account GREYED, with the box left empty", () => {
        mount(scenarioStore.scenario, new Map(), DEFAULTS);
        const box = screen.getByLabelText("Counter-account for income:salary") as HTMLInputElement;
        expect(box.value).toBe("");
        expect(box.placeholder).toBe("assets:checking");
        // Nothing overridden, so nothing is stated.
        expect(scenarioStore.scenario.lines.find((l) => l.account === "income:salary")?.counter).toBeNull();
        // A rule the engine has not answered for yet says so rather than guessing.
        expect((screen.getByLabelText("Counter-account for expenses:housing") as HTMLInputElement).placeholder).toBe("default");
    });

    it("typing one names it, marks the scenario edited, and clearing goes back to the default", async () => {
        mount(scenarioStore.scenario, new Map(), DEFAULTS);
        const box = screen.getByLabelText("Counter-account for income:salary");
        await typeAccount(box, "assets:savings");
        expect(scenarioStore.scenario.lines.find((l) => l.account === "income:salary")?.counter).toBe("assets:savings");
        expect(scenarioStore.dirty).toBe(true);

        await typeAccount(screen.getByLabelText("Counter-account for income:salary"), "");
        expect(scenarioStore.scenario.lines.find((l) => l.account === "income:salary")?.counter).toBeNull();
    });

    it("SPLITS a line out of a shared rule when its counter differs, so the file can still be written", async () => {
        const [salary, dividends] = SEED.lines;
        scenarioStore.adopt({
            ...SEED,
            lines: [
                {...salary, group: "rule:0", counter: "assets:checking"},
                {...dividends, group: "rule:0", counter: "assets:checking"},
            ],
        });
        mount();
        await typeAccount(screen.getByLabelText("Counter-account for income:dividends"), "assets:brokerage");

        const lines = scenarioStore.scenario.lines;
        const moved = lines.find((l) => l.account === "income:dividends");
        expect(moved?.counter).toBe("assets:brokerage");
        expect(moved?.group).not.toBe("rule:0");
        expect(lines.find((l) => l.account === "income:salary")).toMatchObject({group: "rule:0", counter: "assets:checking"});
    });

    it("an asset row's gains account is greyed with the default, and typing one sets it", async () => {
        scenarioStore.adopt(withAssets(assetLine("assets:house", "house")));
        mount(scenarioStore.scenario, new Map(), DEFAULTS);
        const box = screen.getByLabelText("Gains account for the assets:house balance") as HTMLInputElement;
        expect(box.value).toBe("");
        expect(box.placeholder).toBe("equity:unrealized gains");
        await typeAccount(box, "equity:home gains");
        expect(scenarioStore.scenario.lines.find((l) => l.group === "house")?.gains).toBe("equity:home gains");
    });

    it("an event takes ONE counter-account, on its first posting", async () => {
        mount(scenarioStore.scenario, new Map(), DEFAULTS);
        await fireEvent.click(screen.getByRole("button", {name: "+ Add an event"}));
        await tick();
        await typeAccount(screen.getByLabelText("Counter-account for the new event 1"), "assets:checking");
        expect(scenarioStore.scenario.events[0].counter).toBe("assets:checking");
    });

    it("the scenario's default cash account is greyed with the journal's fallback, and settable", async () => {
        mount(scenarioStore.scenario, new Map(), DEFAULTS);
        const box = screen.getByLabelText("Default cash account") as HTMLInputElement;
        expect(box.value).toBe("");
        expect(box.placeholder).toBe("assets:checking");
        await typeAccount(box, "  assets:savings ");
        expect(scenarioStore.scenario.cash).toBe("assets:savings");
        expect(scenarioStore.dirty).toBe(true);
    });
});

// ---------------------------------------------------------------------------
// Descriptions, and order (plan 28, follow-ups 2026-10-26)
// ---------------------------------------------------------------------------

/** The groups of a section's rows, top to bottom, as the DOM has them. */
const groupsIn = (section: string): string[] =>
    [...screen.getByTestId(`projection-section-${section}`).querySelectorAll<HTMLElement>("[data-line-group]")].map((row) => row.dataset.lineGroup ?? "");

const HOUSING = "gap:expenses:housing:$";
const TAXES = "gap:expenses:taxes:$";

describe("COMPONENT ProjectionsTable — a row's description", () => {
    it("every row has a description icon, and hovering or focusing it SHOWS the description", async () => {
        mount();
        const icon = screen.getByRole("button", {name: "Description for expenses:housing"});
        expect(screen.queryByTestId("note-tooltip")).toBeNull();

        await fireEvent.pointerEnter(icon);
        expect(screen.getByTestId("note-tooltip").textContent).toContain(NOTE);
        await fireEvent.pointerLeave(icon);
        expect(screen.queryByTestId("note-tooltip")).toBeNull();

        // Keyboard focus too — a `title` would never show for a keyboard user.
        await fireEvent.focus(icon);
        expect(screen.getByTestId("note-tooltip").textContent).toContain(NOTE);
        await fireEvent.blur(icon);
        expect(screen.queryByTestId("note-tooltip")).toBeNull();

        // And the text is the button's DESCRIPTION, for a screen reader, always.
        const describedBy = icon.getAttribute("aria-describedby") ?? "";
        expect(document.getElementById(describedBy)?.textContent).toContain(NOTE);
    });

    it("click opens an editor; Enter saves the new text to the rule and marks the scenario edited", async () => {
        mount();
        const icon = screen.getByRole("button", {name: "Description for expenses:housing"});
        await fireEvent.click(icon);
        const editor = screen.getByRole("dialog", {name: "Edit the description for expenses:housing"});
        const box = within(editor).getByRole("textbox");
        expect(box).toHaveProperty("value", NOTE);

        await fireEvent.input(box, {target: {value: "rent after the move"}});
        await fireEvent.keyDown(box, {key: "Enter"});
        await tick();

        expect(screen.queryByTestId("note-editor")).toBeNull();
        expect(scenarioStore.scenario.lines.find((l) => l.group === HOUSING)?.note).toBe("rent after the move");
        expect(scenarioStore.dirty).toBe(true);
    });

    it("Escape cancels: nothing changes and nothing is edited", async () => {
        mount();
        await fireEvent.click(screen.getByRole("button", {name: "Description for expenses:housing"}));
        const box = within(screen.getByTestId("note-editor")).getByRole("textbox");
        await fireEvent.input(box, {target: {value: "never mind"}});
        await fireEvent.keyDown(box, {key: "Escape"});
        await tick();

        expect(screen.queryByTestId("note-editor")).toBeNull();
        expect(scenarioStore.scenario.lines.find((l) => l.group === HOUSING)?.note).toBe(NOTE);
        expect(scenarioStore.dirty).toBe(false);
    });

    it("what the file would refuse is said in the editor, and Save stays shut", async () => {
        mount();
        await fireEvent.click(screen.getByRole("button", {name: "Description for expenses:housing"}));
        const editor = screen.getByTestId("note-editor");
        const box = within(editor).getByRole("textbox");
        await fireEvent.input(box, {target: {value: "rent ; utilities"}});
        await tick();

        expect(within(editor).getByRole("alert").textContent).toMatch(/comment/);
        expect(within(editor).getByRole("button", {name: "Save"})).toHaveProperty("disabled", true);
        await fireEvent.keyDown(box, {key: "Enter"});
        await tick();
        expect(screen.getByTestId("note-editor")).toBeDefined();
        expect(scenarioStore.dirty).toBe(false);
    });

    it("rows that share ONE rule share its description, and say so", async () => {
        // A budget rule with two postings: one `~` header between them.
        scenarioStore.adopt({
            ...SEED,
            lines: SEED.lines.map((line) => (line.account.startsWith("expenses:") ? {...line, group: "rule:0", note: "the budget"} : line)),
        });
        mount();
        const icon = screen.getByRole("button", {name: "Description for expenses:taxes"});
        await fireEvent.focus(icon);
        expect(screen.getByTestId("note-tooltip").textContent).toContain("Shared by the 2 lines of this rule");
        await fireEvent.blur(icon);

        await fireEvent.click(icon);
        const box = within(screen.getByTestId("note-editor")).getByRole("textbox");
        await fireEvent.input(box, {target: {value: "household"}});
        await fireEvent.keyDown(box, {key: "Enter"});
        await tick();
        const notes = scenarioStore.scenario.lines.filter((l) => l.group === "rule:0").map((l) => l.note);
        expect(notes).toEqual(["household", "household"]);
    });

    it("marks the seed's average text as STALE once the row's amount changes — and never rewrites it", async () => {
        mount();
        expect(screen.queryAllByTestId("note-stale")).toHaveLength(0);
        const amount = screen.getByLabelText("Amount for expenses:housing");
        await fireEvent.change(amount, {target: {value: "2400"}});
        await tick();

        const icon = screen.getByRole("button", {name: "Description for expenses:housing"});
        expect(within(icon).getByTestId("note-stale")).toBeDefined();
        await fireEvent.focus(icon);
        expect(screen.getByTestId("note-tooltip").textContent).toMatch(/out of date/);
        expect(scenarioStore.scenario.lines.find((l) => l.group === HOUSING)?.note).toBe(NOTE);
    });

    it("an asset row and a one-off line have one too", () => {
        scenarioStore.adopt(
            decodeScenario({
                name: "",
                created: null,
                updated: null,
                lines: [
                    assetLine("assets:brokerage", "rule:1"),
                    {
                        ...seedLine("expenses:housing", "500000"),
                        id: "rule:2:0",
                        group: "rule:2",
                        period: {raw: "2027-03-01", simple: null, base: null, from: null, to: null},
                        section: "oneoff",
                    },
                ],
                events: [],
                cash: null,
                autoRules: [],
                accountTypes: {},
            })
        );
        mount();
        expect(screen.getByRole("button", {name: "Description for the assets:brokerage balance"})).toBeDefined();
        expect(screen.getByRole("button", {name: "Description for the one-off expenses:housing"})).toBeDefined();
    });
});

describe("COMPONENT ProjectionsTable — reordering rows", () => {
    it("Move up and Move down reorder a section, and mark the scenario edited", async () => {
        mount();
        expect(groupsIn("expense")).toEqual([HOUSING, TAXES]);

        await chooseInMenu("expenses:taxes", "Move up");
        expect(groupsIn("expense")).toEqual([TAXES, HOUSING]);
        expect(scenarioStore.dirty).toBe(true);

        await chooseInMenu("expenses:taxes", "Move down");
        expect(groupsIn("expense")).toEqual([HOUSING, TAXES]);
    });

    it("offers a move only where there is somewhere to go", async () => {
        mount();
        await fireEvent.click(screen.getByLabelText("Row menu for expenses:housing"));
        const first = within(screen.getByTestId("row-menu"));
        expect(first.queryByRole("menuitem", {name: "Move up"})).toBeNull();
        expect(first.getByRole("menuitem", {name: "Move down"})).toBeDefined();
        await fireEvent.keyDown(document.activeElement ?? document.body, {key: "Escape"});
        await tick();

        await fireEvent.click(screen.getByLabelText("Row menu for expenses:taxes"));
        const last = within(screen.getByTestId("row-menu"));
        expect(last.getByRole("menuitem", {name: "Move up"})).toBeDefined();
        expect(last.queryByRole("menuitem", {name: "Move down"})).toBeNull();
    });

    it("a step change moves as ONE row, its segments together", async () => {
        mount();
        await chooseInMenu("expenses:housing", "Add a step on 2027-04-01");
        const segments = scenarioStore.scenario.lines.filter((l) => l.account === "expenses:housing").map((l) => l.group);
        expect(segments).toHaveLength(2);

        await chooseInMenu("expenses:taxes", "Move up");
        expect(groupsIn("expense")).toEqual([TAXES, ...segments]);
    });

    it("dragging a row by its handle onto another moves it there", async () => {
        mount();
        const handle = within(rowOf(TAXES) as HTMLElement).getByTestId("drag-handle");
        const target = rowOf(HOUSING) as HTMLElement;
        await fireEvent.dragStart(handle);
        await fireEvent.dragOver(target);
        await fireEvent.drop(target);
        await fireEvent.dragEnd(handle);
        await tick();

        expect(groupsIn("expense")).toEqual([TAXES, HOUSING]);
        expect(scenarioStore.dirty).toBe(true);
    });

    it("a drag never crosses sections: an income row dropped on an expense goes nowhere", async () => {
        mount();
        const handle = within(rowOf("gap:income:salary:$") as HTMLElement).getByTestId("drag-handle");
        await fireEvent.dragStart(handle);
        await fireEvent.drop(rowOf(HOUSING) as HTMLElement);
        await tick();
        expect(groupsIn("expense")).toEqual([HOUSING, TAXES]);
        expect(groupsIn("income")).toEqual(["gap:income:salary:$", "gap:income:dividends:$"]);
        expect(scenarioStore.dirty).toBe(false);
    });

    it("THE SECTION-STABILITY RULE holds across a move: a held, half-typed row moves within its section", async () => {
        mount();
        const added = await addLineIn("income");
        const field = fieldOf(added.group);
        await focus(field);
        await fireEvent.input(field, {target: {value: "i"}});
        await tick();

        await chooseInMenu("i", "Move up");
        expect(sectionOf(added.group)).toBe("projection-section-income");
        expect(groupsIn("income")).toEqual(["gap:income:salary:$", added.group, "gap:income:dividends:$"]);
    });

    it("events reorder through their menu too", async () => {
        scenarioStore.adopt(
            decodeScenario({
                name: "",
                created: null,
                updated: null,
                lines: [],
                events: [
                    {
                        id: "rule:0",
                        date: "2027-01-01",
                        description: "first",
                        postings: [{account: "expenses:a", amount: {commodity: "$", quantity: {mantissa: "100", places: 2}, precision: 2}}],
                        counter: null,
                    },
                    {
                        id: "rule:1",
                        date: "2027-02-01",
                        description: "second",
                        postings: [{account: "expenses:b", amount: {commodity: "$", quantity: {mantissa: "100", places: 2}, precision: 2}}],
                        counter: null,
                    },
                ],
                cash: null,
                autoRules: [],
                accountTypes: {},
            })
        );
        mount();
        await chooseInMenu("second", "Move up");
        expect(scenarioStore.scenario.events.map((e) => e.description)).toEqual(["second", "first"]);
    });
});

describe("COMPONENT ProjectionsTable — automatic postings (plan 30)", () => {
    const locked = (raw: string, query: string, unsupported: string | null = null) => ({raw, query, unsupported, form: null, block: 0});
    const commissionForm = {
        matchAccount: "income:salary",
        extraQuery: "",
        postings: [{account: "expenses:taxes", kind: "percent", percent: {mantissa: "3", places: 0}, amount: null}],
        counter: "assets:checking",
        comment: "",
        forecastOnly: true,
    };
    const COMMISSION = "= ^income:salary(:|$) tag:_generated-transaction\n    expenses:taxes   *-0.03\n    assets:checking  *0.03\n";
    const editable = (raw = COMMISSION, query = "^income:salary(:|$) tag:_generated-transaction", block = 0) => ({
        raw,
        query,
        unsupported: null,
        form: commissionForm,
        block,
    });

    it("is always there to add a rule to, and says what one is for while there are none", () => {
        mount();
        const section = screen.getByTestId("projection-section-auto");
        expect(section.textContent).toContain("Nothing here yet");
        expect(within(section).queryAllByTestId("projection-auto-rule")).toHaveLength(0);
    });

    it("edits a rule the form can state, and shows one it cannot as its text, locked", () => {
        mount(
            decodeScenario({
                ...SEED_WIRE,
                autoRules: [
                    editable(),
                    locked("= expenses:rent\n    (budget:rent)  *-1\n", "expenses:rent"),
                    locked("= expr:'a and b'\n    (x)  1\n", "expr:'a and b'", "`expr:` queries are not supported yet"),
                ],
            })
        );
        const rules = within(screen.getByTestId("projection-section-auto")).getAllByTestId("projection-auto-rule");
        expect(rules).toHaveLength(3);
        expect((within(rules[0]!).getByLabelText("Matched account for rule 1") as HTMLInputElement).value).toBe("income:salary");
        expect(within(rules[0]!).getByTestId("auto-rule-preview").textContent).toBe("≈ $155.65 to expenses:taxes on each $5,188.33 to income:salary");
        expect(rules[1]!.textContent).toContain("(budget:rent)  *-1");
        expect(rules[1]!.textContent).toContain("edit in the file");
        expect(rules[1]!.textContent).toContain("projections only here");
        expect(within(rules[1]!).queryAllByRole("textbox")).toHaveLength(0);
        expect(rules[2]!.textContent).toContain("never fires");
        expect(rules[2]!.textContent).toContain("`expr:` queries are not supported yet, so it generates nothing.");
    });

    it("an edit marks the rule edited and the scenario dirty, so a save sends its form", async () => {
        scenarioStore.adopt(decodeScenario({...SEED_WIRE, autoRules: [editable()]}));
        mount();
        expect(scenarioStore.dirty).toBe(false);
        const box = screen.getByLabelText("Percent for posting 1 of rule 1");
        await fireEvent.change(box, {target: {value: "4"}});
        await tick();
        const rule = scenarioStore.scenario.autoRules[0]!;
        expect(rule.edited).toBe(true);
        expect(rule.form?.postings[0]?.percent).toEqual({m: 4n, p: 0});
        expect(scenarioStore.dirty).toBe(true);
    });

    it("adds a rule balanced against the scenario's cash account, and deletes one through its menu", async () => {
        scenarioStore.adopt(decodeScenario({...SEED_WIRE, cash: "assets:checking", autoRules: [editable()]}));
        mount();
        await fireEvent.click(screen.getByRole("button", {name: "+ Add a rule"}));
        await tick();
        expect(scenarioStore.scenario.autoRules).toHaveLength(2);
        const added = scenarioStore.scenario.autoRules[1]!;
        expect(added).toMatchObject({raw: "", block: null, edited: true});
        expect(added.form?.counter).toBe("assets:checking");

        await chooseInMenu("rule 1", "Delete rule");
        expect(scenarioStore.scenario.autoRules).toHaveLength(1);
        expect(scenarioStore.scenario.autoRules[0]!.raw).toBe("");
    });

    it("reorders rules, because a later one sees what an earlier one posted", async () => {
        scenarioStore.adopt(decodeScenario({...SEED_WIRE, autoRules: [editable(), locked("=revenue:other\n    (x)  *0.05\n", "revenue:other")]}));
        mount();
        await chooseInMenu("rule 2", "Move up");
        expect(scenarioStore.scenario.autoRules.map((rule) => rule.query)).toEqual(["revenue:other", "^income:salary(:|$) tag:_generated-transaction"]);
    });

    it("offers one click to make a hand-written rule forecast-only in the file", async () => {
        const untagged = {...editable("= income:salary\n    expenses:taxes  *-0.03\n", "income:salary"), form: {...commissionForm, counter: null}};
        scenarioStore.adopt(decodeScenario({...SEED_WIRE, autoRules: [untagged]}));
        mount();
        await fireEvent.click(screen.getByRole("button", {name: "Limit to forecasts in the file"}));
        await tick();
        expect(scenarioStore.scenario.autoRules[0]!.edited).toBe(true);
        expect(screen.queryByText("projections only here")).toBeNull();
    });
});
