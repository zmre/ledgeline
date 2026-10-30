// The dialog's draft (plan 31 C3, C11): deep copies, one-mutation commits,
// and the "Writes:" preview's debounce and stale-answer handling. Named
// `.svelte.test.ts` because `EntryPreview` holds `$state`.

import {afterEach, describe, expect, it, vi} from "vitest";
import {ValidationError} from "#lib/api/native.js";
import {dec} from "#lib/domain/money.js";
import {addDraft, applyDraft, draftOf, draftProblems, draftRenderBody, duplicateDraft, EntryPreview, PREVIEW_DEBOUNCE_MS, stepDraft} from "./entryDraft.svelte";
import {blankEvent, emptyScenario, renderBody} from "./scenarioModel";
import type {LayoutItem, Scenario, ScenarioLine} from "./types";

function line(overrides: Partial<ScenarioLine> = {}): ScenarioLine {
    return {
        id: "rule:0:0",
        group: "rule:0",
        role: "flow",
        account: "expenses:rent",
        amount: {commodity: "$", quantity: dec(420000, 2), precision: 2},
        period: {raw: "monthly", simple: "monthly", base: null, from: null, to: null},
        growth: null,
        opening: null,
        note: "rent",
        counter: "assets:checking",
        gains: null,
        source: "journal",
        comment: "",
        ruleComment: "",
        ...overrides,
    };
}

const item = (kind: LayoutItem["kind"], key: string, overrides: Partial<LayoutItem> = {}): LayoutItem => ({
    kind,
    key,
    origin: 0,
    disabled: false,
    attached: "",
    raw: "",
    ...overrides,
});

function file(): Scenario {
    return {
        ...emptyScenario(),
        lines: [line(), line({id: "rule:1:0", group: "rule:1", account: "expenses:fun", note: "fun"})],
        events: [
            {
                ...blankEvent("rule:2", "2027-03-01", "$"),
                description: "raise",
                postings: [{account: "assets:cash", amount: {commodity: "$", quantity: dec(5, 0), precision: 0}, comment: ""}],
            },
        ],
        layout: [
            item("rule", "rule:0", {attached: "; The lease.\n", raw: "; The lease.\n~ monthly  rent\n"}),
            item("rule", "rule:1", {origin: 1}),
            item("event", "rule:2", {origin: 2}),
        ],
    };
}

describe("UNIT entryDraft — a draft is a copy", () => {
    it("deep-copies: mutating the draft never reaches the scenario", () => {
        const scenario = file();
        const before = structuredClone(scenario);
        const draft = draftOf(scenario, "rule:0");
        expect(draft).not.toBeNull();
        if (draft === null || draft.payload.kind !== "rule") return;
        draft.payload.lines[0].account = "expenses:other";
        draft.payload.lines[0].amount.quantity.m = 1n;
        draft.item.disabled = true;
        draft.attached = "changed";
        expect(scenario).toEqual(before);
    });

    it("opens with the attached comment as display text, and nothing edited", () => {
        expect(draftOf(file(), "rule:0")).toMatchObject({attached: "The lease.", edited: false, origin: "edit"});
    });

    it("commits exactly the entry's lines, layout item and attached comment, in place", () => {
        const draft = draftOf(file(), "rule:0");
        if (draft === null || draft.payload.kind !== "rule") throw new Error("no draft");
        draft.payload.lines[0].note = "rent and power";
        draft.item.disabled = true;
        draft.attached = "The new lease.";
        const next = applyDraft(file(), draft, "; The new lease.\n; ~ monthly  rent and power\n");
        expect(next.lines.map((l) => [l.group, l.note])).toEqual([
            ["rule:0", "rent and power"],
            ["rule:1", "fun"],
        ]);
        expect(next.layout[0]).toEqual({
            kind: "rule",
            key: "rule:0",
            origin: 0,
            disabled: true,
            attached: "; The lease.\n",
            attachedText: "The new lease.",
            raw: "; The new lease.\n; ~ monthly  rent and power\n",
        });
        expect(next.layout.slice(1)).toEqual(file().layout.slice(1));
        expect(next.events).toEqual(file().events);
    });

    it("an attached comment left as it was stays the file's own", () => {
        const draft = draftOf(file(), "rule:0");
        if (draft === null) throw new Error("no draft");
        expect("attachedText" in applyDraft(file(), draft).layout[0]).toBe(false);
    });
});

describe("UNIT entryDraft — new-entry drafts insert at their point (plan 31 C11)", () => {
    it("Duplicate: the copy lands directly below the original", () => {
        const draft = duplicateDraft(file(), "rule:0", false);
        if (draft === null) throw new Error("no draft");
        expect(draft).toMatchObject({origin: "duplicate", after: "rule:0", disableOriginal: null, edited: true, attached: "The lease."});
        const next = applyDraft(file(), draft);
        expect(next.layout.map((candidate) => candidate.key)).toEqual(["rule:0", draft.key, "rule:1", "rule:2"]);
        expect(next.layout[0].disabled).toBe(false);
    });

    it("Duplicate as alternative: the copy below AND the original off, in one scenario", () => {
        const draft = duplicateDraft(file(), "rule:2", true);
        if (draft === null) throw new Error("no draft");
        const next = applyDraft(file(), draft);
        expect(next.layout.map((candidate) => [candidate.key, candidate.disabled])).toEqual([
            ["rule:0", false],
            ["rule:1", false],
            ["rule:2", true],
            [draft.key, false],
        ]);
    });

    it("Add: a blank entry after the cursor's, or at the end", () => {
        const context = {commodity: "$", date: "2027-01-01", assetPrefix: "assets:", cash: null};
        const flow = addDraft(file(), "flow", "rule:0", context);
        expect(applyDraft(file(), flow).layout.map((candidate) => candidate.key)[1]).toBe(flow.key);
        const note = addDraft(file(), "note", null, context);
        expect(applyDraft(file(), note).layout.at(-1)?.key).toBe(note.key);
    });

    it("Add a step change: the later half below, and the original bounded at the split", () => {
        const draft = stepDraft(file(), "rule:0", "2027-06-01");
        if (draft === null) throw new Error("no draft");
        const next = applyDraft(file(), draft);
        expect(next.layout.map((candidate) => candidate.key)).toEqual(["rule:0", draft.key, "rule:1", "rule:2"]);
        expect(next.lines.filter((l) => l.group === "rule:0").map((l) => l.period.raw)).toEqual(["monthly to 2027-06-01"]);
        expect(next.lines.filter((l) => l.group === draft.key).map((l) => [l.id, l.period.raw])).toEqual([["rule:0:0", "monthly from 2027-06-01"]]);
    });
});

describe("UNIT entryDraft — what stops Done", () => {
    it("names a missing account, a bad description, an undated one-off and an empty note", () => {
        const rule = draftOf(file(), "rule:0");
        if (rule === null || rule.payload.kind !== "rule") throw new Error("no draft");
        expect(draftProblems(rule)).toEqual([]);
        rule.payload.lines[0].account = " ";
        rule.payload.lines[0].note = "a ; b";
        expect(draftProblems(rule)).toHaveLength(2);
        const event = draftOf(file(), "rule:2");
        if (event === null || event.payload.kind !== "event") throw new Error("no draft");
        event.payload.event.date = "";
        expect(draftProblems(event)).toEqual(["The one-off needs a date."]);
        const note = addDraft(file(), "section", null, {commodity: "$", date: "2027-01-01", assetPrefix: "assets:", cash: null});
        expect(draftProblems(note)).toEqual(["The heading needs a title."]);
    });

    it("a render body is the scenario with the draft applied, and the draft's own item", () => {
        const draft = draftOf(file(), "rule:0");
        if (draft === null) throw new Error("no draft");
        draft.item.disabled = true;
        draft.attached = "Edited.";
        const body = draftRenderBody(file(), draft);
        expect(body.key).toBe("rule:0");
        expect(body.scenario.layout).toEqual([{kind: "rule", key: "rule:0", origin: 0, disabled: true, attachedText: "Edited."}]);
    });
});

describe("COMPONENT EntryPreview — the Writes: box", () => {
    afterEach(() => vi.useRealTimers());
    const body = renderBody(file(), "rule:0");

    it("shows a piece's own text until asked, then calls ONCE per debounce window", async () => {
        vi.useFakeTimers();
        const render = vi.fn(async () => "~ monthly  rent\n");
        const preview = new EntryPreview(render);
        preview.show("raw text");
        expect(preview.text).toBe("raw text");
        preview.request(body);
        preview.request(body);
        preview.request(body);
        expect(preview.pending).toBe(true);
        await vi.advanceTimersByTimeAsync(PREVIEW_DEBOUNCE_MS);
        expect(render).toHaveBeenCalledTimes(1);
        expect(preview).toMatchObject({text: "~ monthly  rent\n", rendered: true, pending: false, refusal: null});
    });

    it("a 400 shows the writer's sentence, and keeps the last good text", async () => {
        vi.useFakeTimers();
        const preview = new EntryPreview(async () => {
            throw new ValidationError("every line of one rule must share its recurrence");
        });
        preview.show("before");
        preview.request(body);
        await vi.advanceTimersByTimeAsync(PREVIEW_DEBOUNCE_MS);
        expect(preview.refusal).toBe("every line of one rule must share its recurrence");
        expect(preview.text).toBe("before");
        expect(preview.rendered).toBe(false);
    });

    it("ignores a stale answer: an older request resolving last never overwrites a newer one", async () => {
        vi.useFakeTimers();
        const answers: ((text: string) => void)[] = [];
        const preview = new EntryPreview(() => new Promise<string>((resolve) => answers.push(resolve)));
        preview.request(body);
        await vi.advanceTimersByTimeAsync(PREVIEW_DEBOUNCE_MS);
        preview.request(body);
        await vi.advanceTimersByTimeAsync(PREVIEW_DEBOUNCE_MS);
        answers[1]("newer");
        await vi.advanceTimersByTimeAsync(0);
        answers[0]("older");
        await vi.advanceTimersByTimeAsync(0);
        expect(preview.text).toBe("newer");
    });

    it("any other failure is said, but is not a refusal", async () => {
        vi.useFakeTimers();
        const preview = new EntryPreview(async () => {
            throw new Error("network down");
        });
        preview.request(body);
        await vi.advanceTimersByTimeAsync(PREVIEW_DEBOUNCE_MS);
        expect(preview).toMatchObject({refusal: null, failure: "network down", pending: false});
    });
});
