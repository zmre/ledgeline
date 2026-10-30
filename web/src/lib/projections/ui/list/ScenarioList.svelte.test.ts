// The scenario list, mounted over the REAL store (plan 31 C3).
//
// What only a mount can answer: what the list was HANDED (every piece of the
// file, in order, each reading like its text), and what an action does to the
// store — one commit, one revision, nothing until Done. The dialog's own
// fields are `EntryDialog.svelte.test.ts`'s.
//
// Nothing asserts on geometry: jsdom has no layout engine (web/README.md). The
// row menu is portalled to <body> and mounted only while open, so every test
// that wants an item opens the menu through its trigger.

import {fireEvent, render, screen, within} from "@testing-library/svelte";
import {tick} from "svelte";
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import {keymap} from "#lib/keys/keymap.svelte.js";
import ScenarioListHarness from "#lib/testing/ScenarioListHarness.svelte";
import {ACCOUNT_NAMES, DECLARED, FILE, SEED, STYLES, TYPES, TYPES_DECLARED} from "#lib/testing/scenarioListFixture.js";
import {emptyScenario, runBody} from "../../scenarioModel";
import {scenarioStore} from "../../scenarioStore.svelte";
import type {Scenario} from "../../types";

const RENDERED = "~ monthly  rendered\n    expenses:x  $1.00\n    assets:checking\n";
const renderMock = vi.fn(async () => RENDERED);

function mount(props: Record<string, unknown> = {}) {
    return render(ScenarioListHarness, {accountNames: ACCOUNT_NAMES, declared: DECLARED, styles: STYLES, render: renderMock, ...props});
}

beforeEach(() => {
    scenarioStore.adopt(FILE);
    renderMock.mockClear();
});
afterEach(() => keymap.reset());

/** Every entry card, in list order. */
const cards = (): HTMLElement[] => screen.queryAllByTestId("projection-entry");
const card = (key: string): HTMLElement => {
    const found = cards().find((element) => element.dataset.key === key);
    if (found === undefined) throw new Error(`no entry ${key}`);
    return found;
};
const keysShown = (): string[] => cards().map((element) => element.dataset.key ?? "");

async function press(key: string): Promise<void> {
    keymap.handle(new KeyboardEvent("keydown", {key, cancelable: true}));
    await tick();
    await tick();
}

/** Open an entry's `⋯` menu and choose `label`. */
async function choose(key: string, label: string): Promise<void> {
    await fireEvent.click(within(card(key)).getByTestId("row-menu-trigger"));
    await fireEvent.click(within(screen.getByTestId("row-menu")).getByRole("menuitem", {name: label}));
    await tick();
    await tick();
}

/** The account-type badges a card draws, in order: one per posting, the counter-account's last. */
const typeBadges = (element: HTMLElement): string[] =>
    [...element.querySelectorAll<HTMLElement>("[data-type-badge]")].map((badge) => badge.dataset.typeBadge ?? "");

/** Real timers: the preview's 300ms debounce, then its answer. */
const settle = (ms = 360): Promise<void> => new Promise((resolve) => setTimeout(resolve, ms));

/** A deep snapshot of the store's scenario, for "nothing changed" checks. */
const snapshot = (): Scenario => snapshotOf(scenarioStore.scenario);
function snapshotOf<T>(value: T): T {
    return JSON.parse(JSON.stringify(value, (_key, inner: unknown) => (typeof inner === "bigint" ? `${inner}n` : inner))) as T;
}

describe("COMPONENT ScenarioList — a seeded scenario", () => {
    beforeEach(() => scenarioStore.adopt(SEED, 2));

    it("one card per seeded rule, badged by the account's TYPE, in the seed's order", () => {
        mount({seeded: true});
        expect(keysShown()).toEqual(["gap:income:salary:$", "gap:income:dividends:$", "gap:expenses:housing:$", "gap:expenses:taxes:$"]);
        // Before the first run the counter's default is unknown, so only the posting is typed.
        expect(typeBadges(card("gap:income:salary:$"))).toEqual(["income"]);
        expect(typeBadges(card("gap:expenses:housing:$"))).toEqual(["expense"]);
        // No section badge in the header: the kind is all it says.
        expect(card("gap:income:salary:$").querySelector("[data-badge=income]")).toBeNull();
    });

    it("shows the amount the file holds, revenue signed negative, and marks it estimated", () => {
        mount({seeded: true});
        const salary = card("gap:income:salary:$");
        expect(within(salary).getByTestId("entry-postings").textContent).toContain("−$5,188.33");
        expect(within(salary).getByText("estimated")).toBeTruthy();
        // Where the number came from is the rule's description, read in full on the card.
        expect(salary.textContent).toContain("unbudgeted — average over");
    });

    it("draws a negative amount in the neutral text colour, never as an error (amendment C3-29)", () => {
        scenarioStore.adopt(TYPES);
        mount({declared: TYPES_DECLARED});
        const amounts = (key: string): HTMLElement[] => within(card(key)).getAllByTestId("entry-amount");
        // The capex rule reads as the file does: money out of checking, into the computers.
        expect(amounts("rule:c").map((amount) => amount.textContent)).toEqual(["−$3,000.00", "$3,000.00"]);
        expect(amounts("rule:d").map((amount) => amount.textContent)).toEqual(["$5,000.00", "−$5,000.00"]);
        const negatives = cards()
            .flatMap((entry) => within(entry).queryAllByTestId("entry-amount"))
            .filter((amount) => amount.textContent?.startsWith("−"));
        expect(negatives.length).toBeGreaterThan(2);
        for (const amount of negatives) expect(amount.className).not.toMatch(/text-error/);
    });

    it("says nothing is saved until Save As, and opens not-dirty", () => {
        mount({seeded: true});
        expect(screen.getByTestId("projection-seeded").textContent).toContain("nothing is saved until you Save as");
        expect(scenarioStore.dirty).toBe(false);
    });
});

describe("COMPONENT ScenarioList — the file, in file order (plan 31 C1)", () => {
    it("lists every piece where the file has it, each as its own kind", () => {
        mount();
        expect(cards().map((element) => [element.dataset.kind, element.dataset.key])).toEqual([
            ["fixed", "fixed:0"],
            ["comment", "comment:0"],
            ["rule", "rule:0"],
            ["rule", "off:0"],
            ["rule", "rule:1"],
            ["rule", "rule:2"],
            ["comment", "comment:1"],
            ["event", "rule:3"],
            ["rule", "rule:4"],
            ["auto", "auto:0"],
            ["auto", "auto:1"],
        ]);
        expect(screen.getByRole("list", {name: /file order/}).children.length).toBe(11);
    });

    it("a `;;` divider block is a section heading; a note reads as its comment", () => {
        mount();
        expect(within(card("comment:0")).getByRole("heading").textContent).toContain("Income");
        expect(card("comment:0").dataset.style).toBe("section");
        expect(card("comment:1").textContent).toContain("; Assumes no price rises.");
    });

    it("file text is read-only: no Edit button, no grip, and it says so", () => {
        mount();
        const text = card("fixed:0");
        expect(within(text).queryByTestId("entry-edit")).toBeNull();
        expect(text.textContent).toContain("file text · read-only");
        expect(text.textContent).toContain("account equity:gains");
    });

    it("a rule reads like its text: period, description, attached comment, postings, counter", () => {
        mount();
        const salary = card("rule:0");
        expect(within(salary).getByRole("heading").textContent).toMatch(/~\s*monthly\s*salary/);
        expect(within(salary).getByTestId("entry-attached").textContent).toContain("; Paid monthly.");
        expect(within(salary).getByTestId("entry-postings").textContent).toContain("income:salary");
        expect(salary.textContent).toContain("↗ 3%/yr");
        expect(within(salary).getByTestId("entry-counter").textContent).toContain("assets:checking");
    });

    it("an asset rule says where its balance comes from and where its gains go", () => {
        mount();
        const asset = card("rule:4");
        expect(within(asset).getByTestId("entry-balance").textContent).toContain("(override)");
        expect(asset.textContent).toContain("gains → equity:gains");
        // The house, and its counter-account (assets:checking under a declared `assets`).
        expect(typeBadges(asset)).toEqual(["asset", "asset"]);
        expect(asset.querySelector("[data-badge]")).toBeNull();
    });

    it("a one-off reads with its date and SIGNED amount", () => {
        mount();
        const event = card("rule:3");
        expect(within(event).getByRole("heading").textContent).toContain("2027-03-01");
        expect(event.textContent).toContain("$2,000,000.00");
        expect(within(event).getByText("one-off")).toBeTruthy();
    });

    it("an `=` rule reads as a sentence, and one the form cannot state shows its text, locked", () => {
        mount();
        const auto = card("auto:0");
        expect(within(auto).getByRole("heading").textContent).toMatch(/When\s*income\s*income:salary\s*is projected/);
        // The matched account, each generated posting and the counter, each by its type.
        expect(typeBadges(auto)).toEqual(["income", "expense", "asset"]);
        expect(within(auto).getByTestId("entry-postings").textContent).toContain("3% of it");
        const locked = card("auto:1");
        expect(locked.textContent).toContain("edit in the file");
        expect(locked.textContent).toContain("(budget:rent)  *-1");
    });

    it("a step pair says which step each is, and the chip goes to the other", async () => {
        mount();
        const chip = within(card("rule:1")).getByTestId("step-chip");
        expect(chip.textContent).toContain("step 1 of 2");
        await fireEvent.click(chip);
        await tick();
        await tick();
        expect(card("rule:2").getAttribute("aria-current")).toBe("true");
    });

    it("a turned-off entry is dimmed in place, and a screen reader hears it is turned off", () => {
        mount();
        const off = card("off:0");
        expect(off.dataset.disabled).toBe("true");
        expect(
            within(off)
                .getByText(/turned/)
                .closest("[data-badge]")?.textContent
        ).toMatch(/turned\s*off/);
    });

    it("every Edit button and menu has a name of its own", () => {
        mount();
        const edits = screen.getAllByTestId("entry-edit").map((button) => button.getAttribute("aria-label"));
        const menus = screen.getAllByTestId("row-menu-trigger").map((button) => button.getAttribute("aria-label"));
        expect(new Set(edits).size).toBe(edits.length);
        expect(new Set(menus).size).toBe(menus.length);
        expect(edits).toContain("Edit payroll (entry 5)");
    });

    it("a read-only file says so beside the list, and stays editable", () => {
        mount({readOnly: true});
        expect(screen.getByTestId("projection-read-only").textContent).toContain("part of your main journal");
        expect(screen.getAllByTestId("entry-edit").length).toBeGreaterThan(0);
    });
});

describe("COMPONENT ScenarioList — every posting badged by its account's type (2026-10-29)", () => {
    beforeEach(() => scenarioStore.adopt(TYPES));
    // The run named a default for the one rule that needs one, and none for the balanced ones.
    const defaults = {cash: "assets:checking", gains: "equity:gains", lines: new Map([["rule:r", "assets:checking"]]), events: new Map()};

    it("a deferred-revenue rule reads as a liability AND income, with no counter line: it balances", () => {
        mount({declared: TYPES_DECLARED, defaults});
        const deferred = card("rule:d");
        expect(typeBadges(deferred)).toEqual(["liability", "income"]);
        expect(within(deferred).getByText("liability").closest("[data-type-badge]")).toBeTruthy();
        expect(within(deferred).queryByTestId("entry-counter")).toBeNull();
        expect(deferred.querySelector("[data-badge]")).toBeNull();
    });

    it("a capital purchase stating both legs has no default counter, nor does a balanced one-off", () => {
        mount({declared: TYPES_DECLARED, defaults});
        expect(typeBadges(card("rule:c"))).toEqual(["cash", "asset"]);
        expect(within(card("rule:c")).queryByTestId("entry-counter")).toBeNull();
        expect(within(card("move")).queryByTestId("entry-counter")).toBeNull();
    });

    it("a rule left to its default counter shows it greyed, its badge dimmed like its text", () => {
        mount({declared: TYPES_DECLARED, defaults});
        const counter = within(card("rule:r")).getByTestId("entry-counter");
        expect(counter.textContent).toContain("assets:checking");
        expect(counter.textContent).toContain("(default)");
        const badge = counter.querySelector<HTMLElement>("[data-type-badge]");
        expect(badge?.dataset.typeBadge).toBe("cash");
        expect(badge?.className).toContain("opacity-60");
    });

    it("a type the scenario file declares wins; an untyped account says so, in words", () => {
        mount({declared: TYPES_DECLARED, defaults});
        expect(typeBadges(card("rule:h"))).toEqual(["liability", "cash"]);
        const unknown = card("rule:m").querySelector<HTMLElement>("[data-type-badge=none]");
        expect(unknown?.textContent).toContain("no account type");
        expect(unknown?.title).toContain("misc:thing has no account type");
    });

    it("one chip per type present; an entry matches a type by ANY of its accounts", async () => {
        mount({declared: TYPES_DECLARED, defaults});
        expect(screen.getByTestId("projection-filter-income").textContent).toMatch(/Income\s*1/);
        expect(screen.getByTestId("projection-filter-liability").textContent).toMatch(/Liability\s*2/);
        expect(screen.queryByTestId("projection-filter-equity")).toBeNull();
        await fireEvent.click(screen.getByTestId("projection-filter-income"));
        expect(keysShown()).toEqual(["rule:d"]);
        await fireEvent.click(screen.getByTestId("projection-filter-liability"));
        expect(keysShown()).toEqual(["rule:d", "rule:h"]);
    });
});

describe("COMPONENT ScenarioList — empty states", () => {
    it("a new, empty scenario says so, with the four Add buttons", async () => {
        scenarioStore.adopt(emptyScenario());
        mount({declared: TYPES_DECLARED});
        const empty = screen.getByTestId("projection-empty");
        expect(empty.textContent).toContain("No entries yet");
        const buttons = within(empty)
            .getAllByRole("button")
            .map((button) => button.textContent?.trim());
        expect(buttons).toEqual(["+ Recurring flow", "+ Asset", "+ One-off", "+ Automatic posting"]);
        await fireEvent.click(within(empty).getByRole("button", {name: "+ One-off"}));
        await tick();
        expect(screen.getByTestId("entry-dialog")).toBeTruthy();
    });

    it("filtered to nothing says which, and clears", async () => {
        scenarioStore.adopt(FILE);
        scenarioStore.commit({
            ...scenarioStore.scenario,
            lines: scenarioStore.scenario.lines.filter((l) => l.role !== "asset"),
            layout: scenarioStore.scenario.layout.filter((item) => item.key !== "rule:4"),
        });
        mount({filter: "liability"});
        expect(screen.getByTestId("projection-filter-empty").textContent).toContain("No liability entries.");
        // Section headings stay, for context, but say nothing matched.
        expect(keysShown()).toEqual(["comment:0"]);
        await fireEvent.click(within(screen.getByTestId("projection-filter-empty")).getByRole("button", {name: "Clear filter"}));
        expect(cards()).toHaveLength(10);
    });
});

describe("COMPONENT ScenarioList — filter chips (plan 31 C12)", () => {
    it("count each kind, and a chip shows only its kind with section headings kept", async () => {
        mount({declared: TYPES_DECLARED});
        expect(screen.getByTestId("projection-filter-all").textContent).toMatch(/All\s*11/);
        // Income: the salary rule, the turned-off contract, and the `=` rule matching salary.
        expect(screen.getByTestId("projection-filter-income").textContent).toMatch(/Income\s*3/);
        expect(screen.getByTestId("projection-filter-equity").textContent).toMatch(/Equity\s*1/);
        // A type nothing in the file has gets no chip.
        expect(screen.queryByTestId("projection-filter-liability")).toBeNull();
        expect(screen.getByTestId("projection-filter-off").textContent).toMatch(/Off\s*1/);
        expect(screen.getByTestId("projection-filter-notes").textContent).toMatch(/Notes\s*2/);
        await fireEvent.click(screen.getByTestId("projection-filter-oneoff"));
        expect(keysShown()).toEqual(["comment:0", "rule:3"]);
        expect(screen.getByTestId("projection-filter-oneoff").getAttribute("aria-pressed")).toBe("true");
    });

    it("turns reordering off while filtered, and says how to get it back", async () => {
        mount({declared: TYPES_DECLARED});
        await fireEvent.click(screen.getByTestId("projection-filter-income"));
        expect(screen.getByTestId("projection-filter-hint").textContent).toContain("Clear the filter to reorder");
        await fireEvent.click(within(card("rule:0")).getByTestId("row-menu-trigger"));
        const items = within(screen.getByTestId("row-menu"))
            .getAllByRole("menuitem")
            .map((item) => item.textContent?.trim());
        expect(items).not.toContain("Move up");
        expect(items).not.toContain("Move down");
        expect(within(card("rule:0")).getByTestId("drag-handle").getAttribute("draggable")).toBe("false");
        const before = scenarioStore.revision;
        await fireEvent.click(card("rule:0"));
        await press("J");
        expect(scenarioStore.revision).toBe(before);
    });
});

describe("COMPONENT ScenarioList — turn off, delete, move", () => {
    it("Turn off dims the entry, says so, and takes it out of the run body", async () => {
        mount({declared: TYPES_DECLARED});
        const before = scenarioStore.revision;
        await choose("rule:3", "Turn off");
        expect(card("rule:3").dataset.disabled).toBe("true");
        expect(scenarioStore.revision).toBe(before + 1);
        expect(scenarioStore.dirty).toBe(true);
        await tick();
        expect(screen.getByTestId("projection-list-status").textContent).toBe("Turned off Series A");
        const body = runBody(scenarioStore.scenario, {interval: "monthly", count: 3, lookback: 0, depth: 2});
        expect(body.scenario.events).toEqual([]);
        await choose("rule:3", "Turn on");
        expect(card("rule:3").dataset.disabled).toBe("false");
    });

    it("Delete removes the entry and what it holds", async () => {
        mount({declared: TYPES_DECLARED});
        await choose("comment:1", "Delete");
        expect(keysShown()).not.toContain("comment:1");
        expect(scenarioStore.scenario.comments.map((c) => c.key)).toEqual(["comment:0"]);
    });

    it("Move up and Move down move ANY entry, one place, past any kind", async () => {
        mount({declared: TYPES_DECLARED});
        await choose("rule:3", "Move up");
        expect(keysShown().slice(5, 8)).toEqual(["rule:2", "rule:3", "comment:1"]);
        await choose("comment:0", "Move down");
        expect(keysShown().slice(1, 3)).toEqual(["rule:0", "comment:0"]);
    });

    it("dragging an entry onto another puts it there, past file text", async () => {
        mount({declared: TYPES_DECLARED});
        const handle = within(card("auto:0")).getByTestId("drag-handle");
        await fireEvent.dragStart(handle, {dataTransfer: {setData: () => {}, effectAllowed: "move"}});
        const target = card("fixed:0").parentElement as HTMLElement;
        await fireEvent.dragOver(target);
        await fireEvent.drop(target);
        await tick();
        expect(keysShown().slice(0, 2)).toEqual(["auto:0", "fixed:0"]);
    });

    it("file text has no grip to drag and no menu to move it", () => {
        mount({declared: TYPES_DECLARED});
        expect(within(card("fixed:0")).queryByTestId("row-menu-trigger")).toBeNull();
        expect(within(card("fixed:0")).queryByTestId("drag-handle")).toBeNull();
    });
});

describe("COMPONENT ScenarioList — the keyboard (plan 31 C13)", () => {
    it("j/k move the cursor, G and g g jump to the ends, Escape clears", async () => {
        mount({declared: TYPES_DECLARED});
        await press("j");
        expect(card("fixed:0").getAttribute("aria-current")).toBe("true");
        await press("j");
        await press("j");
        expect(card("rule:0").getAttribute("aria-current")).toBe("true");
        await press("k");
        expect(card("comment:0").getAttribute("aria-current")).toBe("true");
        await press("G");
        expect(card("auto:1").getAttribute("aria-current")).toBe("true");
        await press("g");
        await press("g");
        expect(card("fixed:0").getAttribute("aria-current")).toBe("true");
        await press("Escape");
        expect(cards().some((element) => element.getAttribute("aria-current") === "true")).toBe(false);
    });

    it("J/K move the entry, and the cursor goes with it", async () => {
        mount({declared: TYPES_DECLARED});
        await fireEvent.click(card("rule:3"));
        await press("K");
        expect(keysShown()[6]).toBe("rule:3");
        expect(card("rule:3").getAttribute("aria-current")).toBe("true");
        await press("J");
        expect(keysShown()[7]).toBe("rule:3");
    });

    it("Enter and e open the dialog on the cursor's entry", async () => {
        mount({declared: TYPES_DECLARED});
        await fireEvent.click(card("rule:0"));
        await press("Enter");
        expect(screen.getByTestId("entry-dialog")).toBeTruthy();
        await fireEvent.click(screen.getByTestId("entry-dialog-cancel"));
        await tick();
        expect(screen.queryByTestId("entry-dialog")).toBeNull();
        await press("e");
        expect(screen.getByRole("dialog").textContent).toContain("Edit recurring rule");
    });

    it("t turns the cursor's entry off and on", async () => {
        mount({declared: TYPES_DECLARED});
        await fireEvent.click(card("rule:0"));
        await press("t");
        expect(card("rule:0").dataset.disabled).toBe("true");
        await press("t");
        expect(card("rule:0").dataset.disabled).toBe("false");
    });

    it("x arms a delete, the second x deletes, and Escape disarms", async () => {
        mount({declared: TYPES_DECLARED});
        await fireEvent.click(card("comment:1"));
        await press("x");
        expect(screen.getByTestId("entry-delete-confirm").textContent).toContain("note Assumes no price rises.");
        await press("Escape");
        expect(screen.queryByTestId("entry-delete-confirm")).toBeNull();
        expect(keysShown()).toContain("comment:1");
        await press("x");
        await press("x");
        expect(keysShown()).not.toContain("comment:1");
    });

    it("c opens a Duplicate draft and C an alternative; n opens the Add menu", async () => {
        mount({declared: TYPES_DECLARED});
        await fireEvent.click(card("rule:0"));
        await press("c");
        expect(screen.getByRole("dialog").textContent).toContain("Copy of a recurring rule");
        await fireEvent.click(screen.getByTestId("entry-dialog-cancel"));
        await tick();
        await press("C");
        expect(screen.getByRole("dialog").textContent).toContain("Alternative to a recurring rule");
        await fireEvent.click(screen.getByTestId("entry-dialog-cancel"));
        await tick();
        await press("n");
        expect(within(screen.getByTestId("row-menu")).getByRole("menuitem", {name: "Section heading"})).toBeTruthy();
    });

    it("nothing fires while the dialog is open", async () => {
        mount({declared: TYPES_DECLARED});
        await fireEvent.click(card("rule:0"));
        await press("Enter");
        const before = scenarioStore.revision;
        await press("t");
        await press("J");
        expect(scenarioStore.revision).toBe(before);
    });
});

describe("COMPONENT ScenarioList — Duplicate and Duplicate as alternative (plan 31 C11)", () => {
    it("Duplicate opens a draft with a placeholder below; Done inserts it there in ONE edit", async () => {
        mount({declared: TYPES_DECLARED});
        const touch = vi.spyOn(scenarioStore, "touch");
        await choose("rule:3", "Duplicate");
        const placeholder = screen.getByTestId("entry-placeholder");
        expect(placeholder.textContent).toContain("New copy — editing");
        expect(card("rule:3").contains(placeholder)).toBe(false);
        expect(placeholder.parentElement).toBe(card("rule:3").parentElement);
        expect(touch).not.toHaveBeenCalled();
        await settle();
        await fireEvent.click(screen.getByTestId("entry-dialog-done"));
        await settle(20);
        expect(touch).toHaveBeenCalledTimes(1);
        const at = keysShown().indexOf("rule:3");
        const copy = keysShown()[at + 1];
        expect(card(copy).dataset.kind).toBe("event");
        expect(card(copy).getAttribute("aria-current")).toBe("true");
        touch.mockRestore();
    });

    it("Cancel leaves the scenario exactly as it was, with no run", async () => {
        mount({declared: TYPES_DECLARED});
        const before = snapshot();
        const revision = scenarioStore.revision;
        await choose("rule:0", "Duplicate");
        await fireEvent.click(screen.getByTestId("entry-dialog-cancel"));
        await tick();
        expect(snapshot()).toEqual(before);
        expect(scenarioStore.revision).toBe(revision);
        expect(screen.queryByTestId("entry-placeholder")).toBeNull();
    });

    it("as alternative: the original says it will be turned off; Done adds the copy AND turns it off, in one edit", async () => {
        mount({declared: TYPES_DECLARED});
        const revision = scenarioStore.revision;
        await choose("rule:0", "Duplicate as alternative");
        expect(within(card("rule:0")).getByTestId("entry-hint").textContent).toContain("Will be turned off");
        await settle();
        await fireEvent.click(screen.getByTestId("entry-dialog-done"));
        await settle(20);
        expect(scenarioStore.revision).toBe(revision + 1);
        expect(card("rule:0").dataset.disabled).toBe("true");
        const copy = keysShown()[keysShown().indexOf("rule:0") + 1];
        expect(card(copy).dataset.disabled).toBe("false");
        expect(screen.getByTestId("projection-list-status").textContent).toBe("Original turned off; copy added");
    });

    it("as alternative, Cancel leaves the original enabled and no copy", async () => {
        mount({declared: TYPES_DECLARED});
        await choose("rule:0", "Duplicate as alternative");
        await fireEvent.click(screen.getByTestId("entry-dialog-cancel"));
        await tick();
        expect(card("rule:0").dataset.disabled).toBe("false");
        expect(cards()).toHaveLength(11);
    });
});

describe("COMPONENT ScenarioList — notes and section headings", () => {
    async function addFromMenu(label: string): Promise<void> {
        await fireEvent.click(screen.getByTestId("projection-add-menu"));
        await fireEvent.click(within(screen.getByTestId("row-menu")).getByRole("menuitem", {name: label}));
        await tick();
    }

    it("a section heading is created from the Add menu, after the cursor's entry", async () => {
        mount({declared: TYPES_DECLARED});
        await fireEvent.click(card("rule:2"));
        await addFromMenu("Section heading");
        expect(screen.getByRole("dialog").textContent).toContain("New section heading");
        const title = screen.getByLabelText("Section title");
        await fireEvent.change(title, {target: {value: "Capital"}});
        await settle();
        await fireEvent.click(screen.getByTestId("entry-dialog-done"));
        await settle(20);
        const at = keysShown().indexOf("rule:2");
        const heading = card(keysShown()[at + 1]);
        expect(heading.dataset.style).toBe("section");
        expect(heading.textContent).toContain("Capital");
        expect(scenarioStore.scenario.comments.at(-1)).toMatchObject({style: "section", text: "Capital", raw: "", edited: true});
    });

    it("a note is edited in the dialog, and can become a heading", async () => {
        mount({declared: TYPES_DECLARED});
        await fireEvent.click(within(card("comment:1")).getByTestId("entry-edit"));
        await fireEvent.change(screen.getByLabelText("Note text"), {target: {value: "Assumes 2% rises."}});
        await fireEvent.click(screen.getByRole("radio", {name: "Section heading"}));
        await settle();
        await fireEvent.click(screen.getByTestId("entry-dialog-done"));
        await settle(20);
        expect(scenarioStore.scenario.comments[1]).toMatchObject({style: "section", text: "Assumes 2% rises.", edited: true});
        expect(card("comment:1").dataset.style).toBe("section");
    });

    it("a note moves like any entry, and is deleted from its menu", async () => {
        mount({declared: TYPES_DECLARED});
        await choose("comment:1", "Move up");
        expect(keysShown()[5]).toBe("comment:1");
        await choose("comment:1", "Delete");
        expect(keysShown()).not.toContain("comment:1");
    });
});

describe("COMPONENT ScenarioList — step changes", () => {
    it("Add a step change… asks for a date, then opens the new segment; Done adds it below", async () => {
        mount({declared: TYPES_DECLARED});
        await choose("rule:0", "Add a step change…");
        const prompt = screen.getByTestId("step-prompt");
        await fireEvent.change(within(prompt).getByLabelText("Date the step starts"), {target: {value: "2027-06-01"}});
        await fireEvent.click(within(prompt).getByRole("button", {name: "Split here"}));
        await tick();
        expect(screen.getByRole("dialog").textContent).toContain("New step in a step change");
        await settle();
        await fireEvent.click(screen.getByTestId("entry-dialog-done"));
        await settle(20);
        const at = keysShown().indexOf("rule:0");
        const later = keysShown()[at + 1];
        expect(within(card("rule:0")).getByTestId("step-chip").textContent).toContain("step 1 of 2");
        expect(within(card(later)).getByTestId("step-chip").textContent).toContain("step 2 of 2");
        expect(within(card("rule:0")).getByRole("heading").textContent).toContain("monthly to 2027-06-01");
    });

    it("no step change on an asset rule: two segments would compound the same balance twice", async () => {
        mount({declared: TYPES_DECLARED});
        await fireEvent.click(within(card("rule:4")).getByTestId("row-menu-trigger"));
        expect(within(screen.getByTestId("row-menu")).queryByRole("menuitem", {name: "Add a step change…"})).toBeNull();
    });

    it("Merge with the other step gives back one rule", async () => {
        mount({declared: TYPES_DECLARED});
        await choose("rule:2", "Merge with the other step");
        expect(keysShown()).not.toContain("rule:2");
        expect(within(card("rule:1")).queryByTestId("step-chip")).toBeNull();
    });
});

describe("COMPONENT ScenarioList — the default cash account", () => {
    it("is greyed with the engine's fallback, and settable as one edit", async () => {
        scenarioStore.adopt({...FILE, cash: null});
        mount({declared: TYPES_DECLARED, defaults: {cash: "assets:bank", gains: "equity:gains", lines: new Map(), events: new Map()}});
        const box = screen.getByLabelText("Default cash account") as HTMLInputElement;
        expect(box.value).toBe("");
        expect(box.placeholder).toBe("assets:bank");
        await fireEvent.input(box, {target: {value: "assets:checking"}});
        await fireEvent.blur(box);
        await tick();
        expect(scenarioStore.scenario.cash).toBe("assets:checking");
        expect(scenarioStore.dirty).toBe(true);
    });
});
