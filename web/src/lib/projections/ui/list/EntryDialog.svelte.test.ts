// The entry dialog over its DRAFT (plan 31 C3), mounted through the list and
// the real store, with the render call mocked (the "Writes:" preview).
//
// The draft contract is the thing under test: nothing reaches the store while
// the dialog is open (no revision, no run), Done applies everything in ONE
// commit, and Cancel, Escape and the backdrop drop it. Then the fields each
// kind exposes, under the accessible names the old table gave them where a
// field survived ("Amount for income:salary", "Balance for …").

import {fireEvent, render, screen, within} from "@testing-library/svelte";
import {tick} from "svelte";
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import {ValidationError, type RenderProjectionBody} from "#lib/api/native.js";
import {keymap} from "#lib/keys/keymap.svelte.js";
import ScenarioListHarness from "#lib/testing/ScenarioListHarness.svelte";
import {ACCOUNT_NAMES, DECLARED, FILE, STYLES, TYPES, TYPES_DECLARED} from "#lib/testing/scenarioListFixture.js";
import {scenarioStore} from "../../scenarioStore.svelte";
import type {AssetRow, ForecastDefaults} from "../../types";

const RENDERED = "; Paid monthly.\n~ monthly  salary and bonus\n    income:salary  $-6500.00\n    assets:checking\n";
let renderMock = vi.fn<(body: RenderProjectionBody) => Promise<string>>(async () => RENDERED);

function mount(props: Record<string, unknown> = {}) {
    return render(ScenarioListHarness, {accountNames: ACCOUNT_NAMES, declared: DECLARED, styles: STYLES, render: renderMock, ...props});
}

beforeEach(() => {
    scenarioStore.adopt(FILE);
    renderMock = vi.fn<(body: RenderProjectionBody) => Promise<string>>(async () => RENDERED);
});
afterEach(() => {
    keymap.reset();
    vi.restoreAllMocks();
});

const card = (key: string): HTMLElement => {
    const found = screen.getAllByTestId("projection-entry").find((element) => element.dataset.key === key);
    if (found === undefined) throw new Error(`no entry ${key}`);
    return found;
};
const dialog = (): HTMLElement => screen.getByTestId("entry-dialog");
const done = (): HTMLButtonElement => screen.getByTestId("entry-dialog-done") as HTMLButtonElement;
const writes = (): string => screen.getByTestId("entry-dialog-writes").textContent ?? "";
const settle = (ms = 360): Promise<void> => new Promise((resolve) => setTimeout(resolve, ms));
const snapshot = (): string => JSON.stringify(scenarioStore.scenario, (_key, inner: unknown) => (typeof inner === "bigint" ? `${inner}n` : inner));

async function open(key: string): Promise<void> {
    await fireEvent.click(within(card(key)).getByTestId("entry-edit"));
    await tick();
}

async function change(label: string | RegExp, value: string): Promise<void> {
    await fireEvent.change(within(dialog()).getByLabelText(label), {target: {value}});
    await tick();
}

describe("COMPONENT EntryDialog — the draft (plan 31 C3)", () => {
    it("opens on the entry, focused inside, showing the piece's own text until an edit", async () => {
        mount();
        await open("rule:0");
        expect(dialog().getAttribute("aria-modal")).toBe("true");
        expect(within(dialog()).getByRole("heading", {name: "Edit recurring rule"})).toBeTruthy();
        expect(dialog().contains(document.activeElement)).toBe(true);
        expect(writes()).toContain("~ monthly  salary");
        expect(renderMock).not.toHaveBeenCalled();
    });

    it("editing fields leaves the store untouched: same scenario, same revision, no run", async () => {
        mount();
        const before = snapshot();
        const revision = scenarioStore.revision;
        const schedule = vi.spyOn(scenarioStore, "schedule");
        const touch = vi.spyOn(scenarioStore, "touch");
        await open("rule:0");
        await change("Description", "salary and bonus");
        await change("Amount for income:salary", "6500");
        await fireEvent.click(within(dialog()).getByLabelText("Enabled"));
        await change("Comment above this entry", "Paid monthly, on the last day.");
        expect(snapshot()).toBe(before);
        expect(scenarioStore.revision).toBe(revision);
        expect(schedule).not.toHaveBeenCalled();
        expect(touch).not.toHaveBeenCalled();
    });

    it("Done applies every field in ONE edit", async () => {
        mount();
        const touch = vi.spyOn(scenarioStore, "touch");
        await open("rule:0");
        await change("Description", "salary and bonus");
        await change("Amount for income:salary", "6500");
        await change("Growth rate for income:salary, in percent", "4");
        await change("Comment above this entry", "Paid on the last day.");
        await fireEvent.click(within(dialog()).getByLabelText("Enabled"));
        await settle();
        await fireEvent.click(done());
        await settle(20);
        expect(touch).toHaveBeenCalledTimes(1);
        expect(screen.queryByTestId("entry-dialog")).toBeNull();
        const [salary] = scenarioStore.scenario.lines;
        expect(salary.note).toBe("salary and bonus");
        // A magnitude: the engine signs it by the account's type.
        expect(salary.entry).toEqual({m: 6500n, p: 0});
        expect(salary.growth).toEqual({rate: {m: 4n, p: 2}, unit: "year"});
        const item = scenarioStore.scenario.layout.find((candidate) => candidate.key === "rule:0");
        expect(item).toMatchObject({disabled: true, attachedText: "Paid on the last day.", raw: RENDERED});
        expect(card("rule:0").dataset.disabled).toBe("true");
    });

    it("Done with nothing edited closes without an edit", async () => {
        mount();
        const revision = scenarioStore.revision;
        await open("rule:0");
        await fireEvent.click(done());
        await tick();
        expect(screen.queryByTestId("entry-dialog")).toBeNull();
        expect(scenarioStore.revision).toBe(revision);
    });

    it("Cancel, Escape and the backdrop each discard the draft", async () => {
        mount();
        const before = snapshot();
        for (const dismiss of [
            () => fireEvent.click(screen.getByTestId("entry-dialog-cancel")),
            () => fireEvent.keyDown(document.activeElement ?? document.body, {key: "Escape"}),
            () => fireEvent.click(dialog().querySelector(".modal-backdrop") as HTMLElement),
        ]) {
            await open("rule:3");
            await change("Description", "something else");
            await dismiss();
            await tick();
            expect(screen.queryByTestId("entry-dialog")).toBeNull();
            expect(snapshot()).toBe(before);
        }
    });

    it("closing returns focus to the Edit button that opened it", async () => {
        mount();
        const edit = within(card("rule:3")).getByTestId("entry-edit");
        edit.focus();
        await fireEvent.click(edit);
        await tick();
        await fireEvent.click(screen.getByTestId("entry-dialog-cancel"));
        await tick();
        expect(document.activeElement).toBe(edit);
    });

    it("Cmd/Ctrl+Enter is Done", async () => {
        mount();
        await open("rule:3");
        await change("Description", "Series A, priced");
        await settle();
        await fireEvent.keyDown(dialog(), {key: "Enter", metaKey: true});
        await settle(20);
        expect(scenarioStore.scenario.events[0].description).toBe("Series A, priced");
    });
});

describe("COMPONENT EntryDialog — the Writes: preview (plan 31 C15)", () => {
    it("after an edit, asks the server ONCE per debounce window with the draft, and shows its text verbatim", async () => {
        mount();
        await open("rule:0");
        await change("Description", "salary and");
        await change("Description", "salary and bonus");
        expect(renderMock).not.toHaveBeenCalled();
        await settle();
        expect(renderMock).toHaveBeenCalledTimes(1);
        const body = renderMock.mock.calls[0][0];
        expect(body.key).toBe("rule:0");
        expect(body.scenario.lines.find((line) => line.group === "rule:0")?.note).toBe("salary and bonus");
        expect(body.scenario.layout).toEqual([{kind: "rule", key: "rule:0", origin: 2, disabled: false, attached: "; Paid monthly.\n"}]);
        expect(writes()).toBe(RENDERED.replace(/\n$/, ""));
    });

    it("a refusal shows the writer's sentence and keeps Done shut", async () => {
        renderMock = vi.fn(async () => {
            throw new ValidationError("every line of one rule must share its recurrence");
        });
        mount();
        await open("rule:0");
        await change("Description", "salary and bonus");
        await settle();
        expect(screen.getByTestId("entry-dialog-refusal").textContent).toBe("every line of one rule must share its recurrence");
        expect(done().disabled).toBe(true);
    });

    it("a draft it can already say is wrong keeps Done shut without asking", async () => {
        mount();
        await open("rule:0");
        await change("Description", "salary ; bonus");
        await settle();
        expect(done().disabled).toBe(true);
        expect(renderMock).not.toHaveBeenCalled();
        expect(screen.getByTestId("entry-dialog-problems").textContent).toContain("comment");
    });

    it("Done pressed while the render is in flight lands once it answers", async () => {
        let answer: (text: string) => void = () => {};
        renderMock = vi.fn(() => new Promise<string>((resolve) => (answer = resolve)));
        mount();
        await open("rule:3");
        await change("Description", "Series A, priced");
        await fireEvent.click(done());
        expect(screen.getByTestId("entry-dialog")).toBeTruthy();
        await settle();
        answer("~ 2027-03-01  Series A, priced\n");
        await settle(20);
        expect(screen.queryByTestId("entry-dialog")).toBeNull();
        expect(scenarioStore.scenario.events[0].description).toBe("Series A, priced");
    });
});

describe("COMPONENT EntryDialog — a recurring rule's fields", () => {
    it("REGRESSION: a bounded period stays editable — moving From rewrites raw and keeps the description", async () => {
        mount();
        await open("rule:2");
        expect((within(dialog()).getByLabelText("From") as HTMLInputElement).value).toBe("2027-02-01");
        await change("From", "2027-03-01");
        await change("Repeats", "quarterly");
        await settle();
        await fireEvent.click(done());
        await settle(20);
        const line = scenarioStore.scenario.lines.find((candidate) => candidate.group === "rule:2");
        expect(line?.period.raw).toBe("quarterly from 2027-03-01");
        expect(line?.note).toBe("payroll");
    });

    it("a recurrence the simple controls cannot state is edited as its expression", async () => {
        scenarioStore.adopt({
            ...FILE,
            lines: FILE.lines.map((line) =>
                line.group === "rule:0" ? {...line, period: {raw: "every 15th day of month", simple: null, base: null, from: null, to: null}} : line
            ),
        });
        mount();
        await open("rule:0");
        expect((within(dialog()).getByLabelText("Period expression") as HTMLInputElement).value).toBe("every 15th day of month");
        expect(within(dialog()).queryByLabelText("From")).toBeNull();
    });

    it("says it is one step of a step change, and where the other runs", async () => {
        mount();
        await open("rule:1");
        expect(within(dialog()).getByTestId("step-info").textContent).toMatch(/Step 1 of 2.*monthly from 2027-02-01/s);
    });

    it("an asset posting's balance is greyed with the journal's figure; an override shows, and clears", async () => {
        const assets = new Map<string, AssetRow>([
            [
                "rule:4",
                {group: "rule:4", account: "assets:house", journalOpening: new Map([["$", {m: 65000000n, p: 2}]]), opening: new Map(), growth: new Map()},
            ],
        ]);
        mount({assetBalances: assets});
        await open("rule:4");
        const balance = within(dialog()).getByLabelText("Balance for assets:house") as HTMLInputElement;
        expect(balance.value).toBe("700000");
        expect(balance.placeholder).toBe("650000.00");
        await fireEvent.click(within(dialog()).getByRole("button", {name: "Use the journal's balance for assets:house"}));
        expect(balance.value).toBe("");
        expect(within(dialog()).getByTestId("ledger-balance").textContent).toContain("$650,000.00");
        expect((within(dialog()).getByLabelText("Gains account for assets:house") as HTMLInputElement).value).toBe("equity:gains");
        expect(within(dialog()).getByLabelText("Contribution for assets:house")).toBeTruthy();
    });

    it("the counter-account is greyed with the engine's default, and set on every posting", async () => {
        scenarioStore.adopt({...FILE, lines: FILE.lines.map((line) => (line.group === "rule:0" ? {...line, counter: null} : line))});
        const defaults: ForecastDefaults = {
            cash: "assets:checking",
            gains: "equity:unrealized gains",
            lines: new Map([["rule:0", "assets:savings"]]),
            events: new Map(),
        };
        mount({defaults});
        await open("rule:0");
        const counter = within(dialog()).getByLabelText("Counter-account") as HTMLInputElement;
        expect(counter.value).toBe("");
        expect(counter.placeholder).toBe("assets:savings");
        await fireEvent.input(counter, {target: {value: "liabilities:card"}});
        await fireEvent.blur(counter);
        await settle();
        await fireEvent.click(done());
        await settle(20);
        expect(scenarioStore.scenario.lines.filter((line) => line.group === "rule:0").map((line) => line.counter)).toEqual(["liabilities:card"]);
    });

    it("adds a posting, written into the same rule on Done", async () => {
        mount();
        await open("rule:0");
        await fireEvent.click(within(dialog()).getByRole("button", {name: "+ Add a posting"}));
        const account = within(dialog()).getByLabelText("Account for posting 2") as HTMLInputElement;
        await fireEvent.input(account, {target: {value: "income:dividends"}});
        await fireEvent.blur(account);
        await change("Amount for income:dividends", "100");
        // Moving a posting out into a rule of its own was removed (2026-10-29).
        expect(within(dialog()).queryByRole("button", {name: /to a new rule/})).toBeNull();
        await settle();
        await fireEvent.click(done());
        await settle(20);
        expect(scenarioStore.scenario.lines.filter((line) => line.group === "rule:0").map((line) => line.account)).toEqual([
            "income:salary",
            "income:dividends",
        ]);
    });
});

describe("COMPONENT EntryDialog — a one-off's fields", () => {
    it("takes a SIGNED amount, a date and a counter-account", async () => {
        mount();
        await open("rule:3");
        expect(within(dialog()).getByRole("heading", {name: "Edit one-off"})).toBeTruthy();
        await change("Amount for assets:checking (posting 1)", "-2500");
        await change("Date", "2027-04-01");
        await settle();
        await fireEvent.click(done());
        await settle(20);
        const [event] = scenarioStore.scenario.events;
        expect(event.postings[0].amount.quantity).toEqual({m: -2500n, p: 0});
        expect(event.date).toBe("2027-04-01");
    });
});

describe("COMPONENT EntryDialog — an automatic posting", () => {
    it("hosts the rule form; an edit makes the save send the form", async () => {
        mount();
        await open("auto:0");
        await change("Percent for posting 1 of this rule", "4");
        await settle();
        await fireEvent.click(done());
        await settle(20);
        const rule = scenarioStore.scenario.autoRules.find((candidate) => candidate.key === "auto:0");
        expect(rule?.edited).toBe(true);
        expect(rule?.form?.postings[0].percent).toEqual({m: 4n, p: 0});
    });

    it("a rule the form cannot state is shown as its text, locked, and can still be turned off", async () => {
        mount();
        await open("auto:1");
        expect(within(dialog()).getByTestId("auto-locked").textContent).toContain("(budget:rent)  *-1");
        expect(within(dialog()).queryAllByRole("combobox")).toHaveLength(0);
        await fireEvent.click(within(dialog()).getByLabelText("Enabled"));
        await settle();
        await fireEvent.click(done());
        await settle(20);
        expect(scenarioStore.scenario.layout.find((item) => item.key === "auto:1")?.disabled).toBe(true);
    });

    it("offers one click to make a hand-written rule forecast-only in the file", async () => {
        scenarioStore.adopt({
            ...FILE,
            autoRules: FILE.autoRules.map((rule) =>
                rule.key === "auto:0" ? {...rule, query: "income:salary", raw: "= income:salary\n    expenses:taxes  *0.03\n"} : rule
            ),
        });
        mount();
        await open("auto:0");
        await fireEvent.click(within(dialog()).getByRole("button", {name: "Limit to forecasts in the file"}));
        await settle();
        await fireEvent.click(done());
        await settle(20);
        expect(scenarioStore.scenario.autoRules.find((rule) => rule.key === "auto:0")?.edited).toBe(true);
    });
});

describe("COMPONENT EntryDialog — balanced postings need no counter-account (2026-10-29)", () => {
    beforeEach(() => scenarioStore.adopt(TYPES));
    const RENDERED_CAPEX = "~ every 6 months  computers\n    assets:checking  $-3000.00\n    assets:fixed:computers  $3500.00\n";

    it("says none is needed, and Done keeps the rule without one", async () => {
        renderMock = vi.fn<(body: RenderProjectionBody) => Promise<string>>(async () => RENDERED_CAPEX);
        mount({declared: TYPES_DECLARED});
        await open("rule:c");
        const counter = within(dialog()).getByLabelText("Counter-account") as HTMLInputElement;
        expect(counter.placeholder).toBe("none needed");
        expect(within(dialog()).getByTestId("counter-not-needed").textContent).toContain("balance on their own");
        // Each posting row carries its account's type.
        expect([...dialog().querySelectorAll<HTMLElement>("[data-type-badge]")].map((badge) => badge.dataset.typeBadge)).toEqual(["cash", "asset"]);
        await change("Description", "laptops");
        await settle();
        expect(done().disabled).toBe(false);
        await fireEvent.click(done());
        await settle(20);
        expect(scenarioStore.scenario.lines.filter((line) => line.group === "rule:c").map((line) => [line.note, line.counter])).toEqual([
            ["laptops", null],
            ["laptops", null],
        ]);
    });

    it("once an edit unbalances the postings, the box offers the default again", async () => {
        mount({declared: TYPES_DECLARED, defaults: {cash: "assets:checking", gains: "equity:gains", lines: new Map(), events: new Map()}});
        await open("rule:c");
        await change("Amount for assets:fixed:computers", "3500");
        const counter = within(dialog()).getByLabelText("Counter-account") as HTMLInputElement;
        expect(counter.placeholder).toBe("assets:checking");
        expect(within(dialog()).queryByTestId("counter-not-needed")).toBeNull();
    });

    it("a balanced one-off says the same", async () => {
        mount({declared: TYPES_DECLARED});
        await open("move");
        expect((within(dialog()).getByLabelText("Counter-account") as HTMLInputElement).placeholder).toBe("none needed");
        expect(within(dialog()).getByTestId("counter-not-needed")).toBeTruthy();
    });
});
