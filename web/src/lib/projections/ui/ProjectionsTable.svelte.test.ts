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
import {decodeScenario} from "$lib/api/nativeDecode";
import type {AccountType} from "$lib/domain/accountTypes";
import {scenarioStore} from "../scenarioStore.svelte";
import type {AssetRow, Scenario} from "../types";
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
    period: {raw: "monthly", simple: "monthly", from: null, to: null},
    growth: null,
    opening: null,
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
    period: {raw: "monthly", simple: "monthly", from: null, to: null},
    growth: {rate: {mantissa: "7", places: 2}, unit: "year"},
    opening: null,
    note: "",
    source: "journal",
});

const ACCOUNT_NAMES = ["income:salary", "income:dividends", "expenses:housing", "expenses:taxes", "assets:checking", "assets:brokerage", "assets:house"];
const DECLARED: ReadonlyMap<string, AccountType> = new Map<string, AccountType>([
    ["income", "revenue"],
    ["expenses", "expense"],
    ["assets", "asset"],
]);

const STYLES = new Map([["$", {side: "L" as const, spaced: false, precision: 2, decimalPoint: ".", digitGroups: [",", [3]] as [string, number[]]}]]);

/** One attributed asset row, as `Projection.assets` carries it. */
const attributed = (group: string, account: string, held: bigint): AssetRow => ({
    group,
    account,
    journalOpening: new Map([["$", {m: held, p: 2}]]),
    opening: new Map([["$", {m: held, p: 2}]]),
    growth: new Map([["$", {m: 0n, p: 2}]]),
});

function mount(scenario: Scenario = scenarioStore.scenario, assetBalances: ReadonlyMap<string, AssetRow> = new Map()) {
    return render(ProjectionsTable, {
        scenario,
        accountNames: ACCOUNT_NAMES,
        declared: DECLARED,
        commodity: "$",
        stepDate: "2027-04-01",
        assetBalances,
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

        const typed = "revenues:consulting";
        for (let n = 1; n <= typed.length; n += 1) {
            await fireEvent.input(field, {target: {value: typed.slice(0, n)}});
            await tick();

            const after = typed.slice(0, n);
            expect(`${after}: ${sectionOf(added.group)}`).toBe(`${after}: projection-section-income`);
            // Same NODE, still focused: a recreated input would be neither.
            expect(fieldOf(added.group)).toBe(field);
            expect(document.activeElement).toBe(field);
        }
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
        // flip. Typed here is a route that would move a FLOW row twice —
        // through `income:…`, which is revenue, and out the other side.
        scenarioStore.adopt(withAssets(assetLine("assets:brokerage", "brok")));
        mount(scenarioStore.scenario, new Map([["brok", attributed("brok", "assets:brokerage", 51_230_000n)]]));

        const field = fieldOf("brok");
        await focus(field);
        expect(sectionOf("brok")).toBe("projection-section-asset");

        const typed = "income:consulting";
        for (let n = 0; n <= typed.length; n += 1) {
            await fireEvent.input(field, {target: {value: typed.slice(0, n)}});
            await tick();

            const after = typed.slice(0, n);
            expect(`${after}: ${sectionOf("brok")}`).toBe(`${after}: projection-section-asset`);
            // Same NODE, still focused: a recreated input would be neither.
            expect(fieldOf("brok")).toBe(field);
            expect(document.activeElement).toBe(field);
        }

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
            lines: [{...dated, id: "buy", group: "buy", account: "expenses:housing", period: {raw: "2027-03-01", simple: null, from: null, to: null}}],
        });
        mount();
        const oneoff = within(screen.getByTestId("projection-section-oneoff"));
        expect(oneoff.getByLabelText("Amount for the one-off expenses:housing")).toBeDefined();
        expect(within(screen.getByTestId("projection-section-expense")).queryByTestId("projection-line")).toBeNull();
        expect(oneoff.getByLabelText("Date of the one-off expenses:housing")).toHaveProperty("value", "2027-03-01");
    });
});
