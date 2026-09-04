import {readFileSync} from "node:fs";
import {describe, expect, it} from "vitest";
import {decodeRulesDoc} from "$lib/api/nativeDecode";
import type {SaveRulesItem} from "$lib/api/native";
import {
    appendRule,
    blankRule,
    columnRole,
    columnRoleHint,
    describeIfBlock,
    describeItem,
    fieldNames,
    fieldsIndex,
    isDirty,
    isHledgerField,
    itemId,
    itemText,
    moveRule,
    ruleIndices,
    settingIndex,
    settingText,
    toForm,
    toSaveRequest,
    validateForm,
    withFieldNames,
    withFlag,
    withSetting,
    type FormItem,
    type IfBlockItem,
    type RulesForm,
} from "./model";
import type {RulesDocument} from "./types";

// The primary sample is the SAME committed body the engine asserts against
// (`fixtures/rules/golden/rules-doc.json`, replayed byte-for-byte by
// `crates/ledgeline-server/tests/rules_endpoints.rs`), decoded through the real
// decoder. A rules file is a format-preserving document whose every item has to
// survive a round trip, and a hand-written literal would only prove the model
// round-trips the shapes its author remembered.
function goldenDoc(): RulesDocument {
    return decodeRulesDoc(JSON.parse(readFileSync(new URL("../../../../fixtures/rules/golden/rules-doc.json", import.meta.url), "utf8")));
}

/** A fresh baseline/live pair, exactly as the page builds them. */
function pair(doc: RulesDocument = goldenDoc()): {baseline: RulesForm; form: RulesForm} {
    return {baseline: toForm(doc), form: toForm(doc)};
}

/** Every id the save request accounts for, saved or deleted. */
function accountedFor(items: SaveRulesItem[], deleted: number[]): Set<number> {
    const ids = new Set(deleted);
    for (const item of items) {
        if (item.id !== undefined) ids.add(item.id);
    }
    return ids;
}

function ruleAt(form: RulesForm, position: number): IfBlockItem {
    const item = form.items[ruleIndices(form.items)[position] ?? -1];
    if (item === undefined || item.kind !== "ifBlock") throw new Error(`no rule at rules-list position ${position}`);
    return item;
}

describe("UNIT imports model — the golden document becomes a form", () => {
    it("maps every wire item to exactly one form item, in order", () => {
        const doc = goldenDoc();
        const form = toForm(doc);
        expect(form.items).toHaveLength(doc.items.length);
        expect(form.items.map(itemId)).toEqual(doc.items.map((item) => item.id));
        expect(form.id).toBe("import/2026/bank.csv.rules");
        expect(form.revision).toBe(doc.revision);
        expect(form.editable).toBe(true);
    });

    it("makes trivia and opaque constructs KEPT — they carry no editable field at all", () => {
        const form = toForm(goldenDoc());
        const kept = form.items.filter((item) => item.kind === "kept");
        expect(kept.map(itemId)).toEqual([0, 11]);
        expect(kept.every((item) => item.kind === "kept" && "source" in item)).toBe(true);
    });

    it("derives each setting's backing item, agreeing with the engine's own itemIds", () => {
        // The engine sends a flattened `settings` projection carrying the item
        // that produced each value. This model derives the same thing from
        // `items`, so asserting the two agree is what stops a panel from
        // silently editing a line hledger does not use.
        const doc = goldenDoc();
        const form = toForm(doc);
        expect(settingIndex(form.items, "date-format")).toBe(doc.settings.dateFormat?.itemId);
        expect(settingIndex(form.items, "skip")).toBe(doc.settings.skip?.itemId);
        expect(settingIndex(form.items, "account1")).toBe(doc.settings.account1?.itemId);
        expect(settingIndex(form.items, "account2")).toBe(doc.settings.account2?.itemId);
        expect(settingIndex(form.items, "currency")).toBe(doc.settings.currency?.itemId);
        expect(fieldsIndex(form.items)).toBe(doc.settings.fields?.itemId);
    });

    it("reads setting values back out", () => {
        const form = toForm(goldenDoc());
        expect(settingText(form.items, "date-format")).toBe("%Y-%m-%d");
        expect(settingText(form.items, "skip")).toBe("1");
        expect(settingText(form.items, "account2")).toBe("expenses:unknown");
        expect(settingText(form.items, "separator")).toBeNull();
        expect(fieldNames(form.items)).toEqual(["date", "description", "amount"]);
    });

    it("gives the rules list everything no panel speaks for, and nothing else", () => {
        const form = toForm(goldenDoc());
        // 0 is the header comment; 7/8/9/10 are the editable blocks (10 being
        // the AND-group one); 11 is the `if` table. 1-6 are the
        // directives/fields/assignments the panels own.
        expect(ruleIndices(form.items)).toEqual([0, 7, 8, 9, 10, 11]);
    });
});

describe("UNIT imports model — the save request accounts for every item", () => {
    it("sends an untouched document back as nothing but `keep`s", () => {
        const {baseline, form} = pair();
        const body = toSaveRequest(baseline, form);
        expect(body.revision).toBe(baseline.revision);
        expect(body.delete).toEqual([]);
        expect(body.items).toHaveLength(12);
        expect(body.items.every((item) => item.kind === "keep")).toBe(true);
        expect(body.items.map((item) => item.id)).toEqual([0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]);
    });

    it("carries the OPAQUE construct through as `keep`, never as a body", () => {
        // This is the pass-through contract that `editMapping.ts` learned the
        // hard way (DL-2): an item the editor cannot represent must be echoed by
        // id, not dropped and not re-rendered from a guess.
        const {baseline, form} = pair();
        ruleAt(form, 1).assignments[0]!.value = "expenses:food:beans";
        const opaque = toSaveRequest(baseline, form).items.find((item) => item.id === 11);
        expect(opaque).toEqual({kind: "keep", id: 11});
    });

    it("sends only the edited item as a typed body", () => {
        const {baseline, form} = pair();
        ruleAt(form, 1).groups[0]!.matchers[0]!.pattern = "ESPRESSO";
        const body = toSaveRequest(baseline, form);
        const bodies = body.items.filter((item) => item.kind !== "keep");
        expect(bodies).toEqual([
            {
                kind: "ifBlock",
                id: 7,
                groups: [{matchers: [{pattern: "ESPRESSO"}]}],
                assignments: [{field: "account2", value: "expenses:food:coffee"}],
            },
        ]);
        expect(accountedFor(body.items, body.delete)).toEqual(new Set([0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]));
    });

    it("omits `field` for a whole-record matcher and sends it for a scoped one", () => {
        const {baseline, form} = pair();
        const rule = ruleAt(form, 3);
        // An OR list is one matcher per group, which is what makes "add another
        // alternative" an added GROUP rather than an added matcher.
        expect(rule.groups.map((group) => group.matchers.map((matcher) => matcher.field))).toEqual([["description"], ["description"]]);
        rule.groups = [{matchers: [{field: "", pattern: "MARKET"}]}, ...rule.groups];
        const sent = toSaveRequest(baseline, form).items.find((item) => item.id === 9);
        expect(sent).toMatchObject({
            kind: "ifBlock",
            groups: [
                {matchers: [{pattern: "MARKET"}]},
                {matchers: [{field: "description", pattern: "SUPERMARKET"}]},
                {matchers: [{field: "description", pattern: "GROCER"}]},
            ],
        });
        // A whole-record matcher must not carry `field: ""` — the engine reads
        // that as a field NAMED "" and its bare-name check refuses it.
        expect(Object.keys((sent as {groups: {matchers: object[]}[]}).groups[0]?.matchers[0] ?? {})).toEqual(["pattern"]);
    });

    it("inserts a new rule WITHOUT an id and still accounts for every server item", () => {
        const {baseline, form} = pair();
        form.items = appendRule(form.items, blankRule("expenses:unknown"));
        const rule = form.items.find((item) => item.kind === "ifBlock" && item.id === null);
        if (rule?.kind !== "ifBlock") throw new Error("appendRule did not add a rule");
        rule.groups[0]!.matchers[0]!.pattern = "PHARMACY";
        rule.assignments[0]!.value = "expenses:health";

        const body = toSaveRequest(baseline, form);
        const inserted = body.items.filter((item) => item.kind === "ifBlock" && item.id === undefined);
        expect(inserted).toHaveLength(1);
        expect(accountedFor(body.items, body.delete)).toEqual(new Set([0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]));
    });

    it("puts a removed item in `delete`, and nowhere else", () => {
        const {baseline, form} = pair();
        form.items = form.items.filter((item) => itemId(item) !== 8);
        const body = toSaveRequest(baseline, form);
        expect(body.delete).toEqual([8]);
        expect(body.items.some((item) => item.id === 8)).toBe(false);
        expect(accountedFor(body.items, body.delete)).toEqual(new Set([0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]));
    });

    it("expresses a reorder as `keep`s in a new order — no item is re-rendered", () => {
        const {baseline, form} = pair();
        form.items = moveRule(form.items, 1, 2);
        const body = toSaveRequest(baseline, form);
        expect(body.items.every((item) => item.kind === "keep")).toBe(true);
        expect(body.items.map((item) => item.id)).toEqual([0, 1, 2, 3, 4, 5, 6, 8, 7, 9, 10, 11]);
        expect(body.delete).toEqual([]);
    });

    it("re-sends an item as `keep` once its value is typed back to what it was", () => {
        const {baseline, form} = pair();
        const rule = ruleAt(form, 1);
        rule.groups[0]!.matchers[0]!.pattern = "TEA";
        expect(toSaveRequest(baseline, form).items.find((item) => item.id === 7)?.kind).toBe("ifBlock");
        rule.groups[0]!.matchers[0]!.pattern = "COFFEE";
        expect(toSaveRequest(baseline, form).items.find((item) => item.id === 7)).toEqual({kind: "keep", id: 7});
    });

    // The property the whole feature rests on, over a spread of edits rather
    // than one: the engine refuses a plan that does not name every item, so a
    // request that dropped one is a 400 at best and a truncated file at worst.
    it("accounts for every server item under any combination of edits", () => {
        const mutations: ((form: RulesForm) => void)[] = [
            (form) => (form.items = moveRule(form.items, 4, 0)),
            (form) => (form.items = form.items.filter((item) => itemId(item) !== 4)),
            (form) => (form.items = appendRule(form.items, blankRule("expenses:x"))),
            (form) => (form.items = withSetting(form.items, "date-format", "%d/%m/%Y")),
            (form) => (form.items = withSetting(form.items, "separator", ";")),
            (form) => (form.items = withSetting(form.items, "skip", "")),
            (form) => (form.items = withFlag(form.items, "newest-first", true)),
            (form) => (form.items = withFieldNames(form.items, ["date", "description", "amount", "balance"])),
            (form) => (ruleAt(form, 2).assignments[0]!.value = "expenses:rent"),
        ];
        for (const first of mutations) {
            for (const second of mutations) {
                const {baseline, form} = pair();
                first(form);
                second(form);
                const body = toSaveRequest(baseline, form);
                const named = accountedFor(body.items, body.delete);
                for (const id of baseline.items.map(itemId)) {
                    expect(named.has(id as number), `id ${id} went missing`).toBe(true);
                }
                // And nothing is BOTH saved and deleted.
                const saved = body.items.map((item) => item.id).filter((id): id is number => id !== undefined);
                expect(saved.filter((id) => body.delete.includes(id))).toEqual([]);
            }
        }
    });
});

describe("UNIT imports model — an OR of AND-groups", () => {
    // The distinction the whole grouped shape exists for. `A OR B` and `A AND B`
    // are written with the same two matchers and import different rows, and the
    // only thing that tells them apart on either side of the wire is the nesting.
    it("keeps an AND-group as one group and an OR list as one group each", () => {
        const form = toForm(goldenDoc());
        expect(ruleAt(form, 3).groups.map((group) => group.matchers.map((matcher) => matcher.pattern))).toEqual([["SUPERMARKET"], ["GROCER"]]);
        expect(ruleAt(form, 4).groups.map((group) => group.matchers.map((matcher) => matcher.pattern))).toEqual([["AIRLINE", "^-"]]);
    });

    it("sends an edited AND-group back with its nesting, and no combinator anywhere", () => {
        const {baseline, form} = pair();
        ruleAt(form, 4).groups[0]!.matchers[1]!.pattern = "^-1";
        const sent = toSaveRequest(baseline, form).items.find((item) => item.id === 10);
        expect(sent).toEqual({
            kind: "ifBlock",
            id: 10,
            groups: [
                {
                    matchers: [
                        {field: "description", pattern: "AIRLINE"},
                        {field: "amount", pattern: "^-1"},
                    ],
                },
            ],
            assignments: [{field: "account2", value: "expenses:travel:airfare"}],
        });
        // The `&` is the engine's to write. Nothing in the body may carry one.
        expect(JSON.stringify(sent)).not.toContain("&");
    });

    it("adds an AND condition to an existing group and an OR group beside it", () => {
        const {baseline, form} = pair();
        const rule = ruleAt(form, 1);
        rule.groups[0]!.matchers = [...rule.groups[0]!.matchers, {field: "card", pattern: "personal"}];
        rule.groups = [...rule.groups, {matchers: [{field: "", pattern: "ESPRESSO"}]}];
        expect(toSaveRequest(baseline, form).items.find((item) => item.id === 7)).toMatchObject({
            groups: [{matchers: [{pattern: "COFFEE"}, {field: "card", pattern: "personal"}]}, {matchers: [{pattern: "ESPRESSO"}]}],
        });
    });

    // The signature has to see the SHAPE, not just the values: regrouping two
    // matchers changes which rows the file matches, and a signature that missed
    // it would send the item back as an unchanged `keep` — the file would keep
    // importing the old way while the screen showed the new one.
    it("is dirty when the same matchers are regrouped, with no character changed", () => {
        const {baseline, form} = pair();
        const rule = ruleAt(form, 3);
        const [first, second] = rule.groups;
        rule.groups = [{matchers: [...first!.matchers, ...second!.matchers]}];
        expect(isDirty(baseline, form)).toBe(true);
        expect(toSaveRequest(baseline, form).items.find((item) => item.id === 9)).toMatchObject({
            groups: [
                {
                    matchers: [
                        {field: "description", pattern: "SUPERMARKET"},
                        {field: "description", pattern: "GROCER"},
                    ],
                },
            ],
        });
    });

    // The write wire OMITS an absent control rather than sending `null`, the
    // same way it omits an absent `id` — the save body is `deny_unknown_fields`
    // and the engine reads the missing key as "no control word". A `null` on
    // the wire would be a third value it has no case for.
    it("sends a skip as a key and an unset control as no key at all", () => {
        const {baseline, form} = pair();
        ruleAt(form, 1).control = "skip";
        const sent = toSaveRequest(baseline, form).items.find((item) => item.id === 7);
        expect(sent).toEqual({
            kind: "ifBlock",
            id: 7,
            groups: [{matchers: [{pattern: "COFFEE"}]}],
            assignments: [{field: "account2", value: "expenses:food:coffee"}],
            control: "skip",
        });

        const untouched = pair();
        untouched.form.items = appendRule(untouched.form.items, blankRule("expenses:x"));
        const inserted = toSaveRequest(untouched.baseline, untouched.form).items.at(-1)!;
        expect(inserted).not.toHaveProperty("control");
    });

    it("goes clean again when a regrouping is undone", () => {
        const {baseline, form} = pair();
        const rule = ruleAt(form, 4);
        const [group] = rule.groups;
        rule.groups = group!.matchers.map((matcher) => ({matchers: [matcher]}));
        expect(isDirty(baseline, form)).toBe(true);
        rule.groups = [{matchers: rule.groups.flatMap((each) => each.matchers)}];
        expect(isDirty(baseline, form)).toBe(false);
    });

    it("renders the AND-group the way the file spells it, `&` prefix and all", () => {
        const form = toForm(goldenDoc());
        expect(itemText(ruleAt(form, 4))).toBe("if %description AIRLINE\n& %amount ^-\n    account2 expenses:travel:airfare");
    });

    it("validates every matcher of every group, naming the group only when there is more than one", () => {
        const form = toForm(goldenDoc());
        expect(validateForm(form)).toEqual([]);

        const oneGroup = toForm(goldenDoc());
        ruleAt(oneGroup, 4).groups[0]!.matchers[1]!.pattern = "& SNEAKY";
        expect(validateForm(oneGroup)[0]).toContain("Rule 4, match 2:");

        const twoGroups = toForm(goldenDoc());
        const rule = ruleAt(twoGroups, 3);
        rule.groups[1]!.matchers[0]!.pattern = "GROC(ER|ERY)";
        expect(validateForm(twoGroups)[0]).toContain("Rule 3, group 2, match 1:");
    });

    // The engine refuses an empty group ("a conditional block's OR-group needs
    // at least one matcher") because it would vanish on flattening and silently
    // re-group its neighbours. Saying so here is friendlier than a 400.
    it("refuses an empty OR-group, exactly as the engine does", () => {
        const form = toForm(goldenDoc());
        ruleAt(form, 3).groups[1]!.matchers = [];
        expect(validateForm(form)[0]).toContain("Rule 3, group 2 needs at least one thing to match");
    });

    it("seeds a new rule with one group holding one whole-record matcher", () => {
        expect(blankRule().groups).toEqual([{matchers: [{field: "", pattern: ""}]}]);
    });
});

describe("UNIT imports model — the one-line rule summary", () => {
    const rule = (
        groups: IfBlockItem["groups"],
        assignments: IfBlockItem["assignments"] = [{field: "account2", value: "expenses:x"}],
        control: IfBlockItem["control"] = null
    ): IfBlockItem => ({
        kind: "ifBlock",
        id: null,
        groups,
        assignments,
        control,
    });

    it("reads a single whole-record matcher as one line", () => {
        expect(describeIfBlock(rule([{matchers: [{field: "", pattern: "COFFEE"}]}]))).toBe("IF row ~ COFFEE → account2 = expenses:x");
    });

    it("names the column a scoped matcher is scoped to", () => {
        expect(describeIfBlock(rule([{matchers: [{field: "description", pattern: "AMAZON"}]}]))).toBe("IF description ~ AMAZON → account2 = expenses:x");
    });

    it("joins an AND-group with AND and needs no brackets for one branch", () => {
        const summary = describeIfBlock(
            rule([
                {
                    matchers: [
                        {field: "description", pattern: "AMAZON"},
                        {field: "card", pattern: "personal"},
                    ],
                },
            ])
        );
        expect(summary).toBe("IF description ~ AMAZON AND card ~ personal → account2 = expenses:x");
    });

    it("joins a plain OR list with OR and still needs no brackets", () => {
        const summary = describeIfBlock(rule([{matchers: [{field: "", pattern: "SHELL"}]}, {matchers: [{field: "", pattern: "CHEVRON"}]}]));
        expect(summary).toBe("IF row ~ SHELL OR row ~ CHEVRON → account2 = expenses:x");
    });

    // Without the brackets `A AND B OR C` reads as either grouping, and the two
    // match different rows — so they appear exactly where the ambiguity is.
    it("brackets an AND-group once it shares the line with another branch", () => {
        const summary = describeIfBlock(
            rule([
                {
                    matchers: [
                        {field: "", pattern: "GROCER"},
                        {field: "card", pattern: "personal"},
                    ],
                },
                {matchers: [{field: "", pattern: "FARMERS"}]},
            ])
        );
        expect(summary).toBe("IF (row ~ GROCER AND card ~ personal) OR row ~ FARMERS → account2 = expenses:x");
    });

    it("lists what the rule sets, and counts the rest", () => {
        const summary = describeIfBlock(
            rule(
                [{matchers: [{field: "", pattern: "X"}]}],
                [
                    {field: "account2", value: "expenses:a"},
                    {field: "comment", value: "note"},
                    {field: "code", value: "42"},
                ]
            )
        );
        expect(summary).toBe("IF row ~ X → account2 = expenses:a, comment = note, +1 more");
    });

    it("names an assignment that has no value yet without an empty `=`", () => {
        expect(describeIfBlock(rule([{matchers: [{field: "", pattern: "X"}]}], [{field: "account2", value: ""}]))).toBe("IF row ~ X → account2");
    });

    // Scannability is the whole point of the collapsed list, so a monstrous rule
    // is summarized rather than allowed to grow the card.
    it("counts the branches and conditions it does not show, instead of growing", () => {
        const many = rule(
            [
                {
                    matchers: [
                        {field: "a", pattern: "1"},
                        {field: "b", pattern: "2"},
                        {field: "c", pattern: "3"},
                        {field: "d", pattern: "4"},
                    ],
                },
                {matchers: [{field: "e", pattern: "5"}]},
                {matchers: [{field: "f", pattern: "6"}]},
                {matchers: [{field: "g", pattern: "7"}]},
            ],
            [{field: "account2", value: "expenses:x"}]
        );
        expect(describeIfBlock(many)).toBe("IF (a ~ 1 AND b ~ 2 AND c ~ 3 AND +1 more) OR e ~ 5 OR +2 more → account2 = expenses:x");
    });

    it("clips a pattern and a value that would run off the line", () => {
        const long = "SUPERMARKET|GROCER|CORNERSHOP|MARKET|DELI";
        const summary = describeIfBlock(rule([{matchers: [{field: "description", pattern: long}]}], [{field: "comment", value: long}]));
        expect(summary).toBe("IF description ~ SUPERMARKET|GROCER|CORNERSHOP|M… → comment = SUPERMARKET|GROCER|CORNERSHOP|M…");
        expect(summary.length).toBeLessThan(100);
    });

    it("calls a rule with nothing typed in it a new rule", () => {
        expect(describeIfBlock(blankRule("expenses:unknown"))).toBe("New rule");
        expect(describeIfBlock(rule([]))).toBe("New rule");
    });

    it("summarizes every editable rule in the golden document", () => {
        const form = toForm(goldenDoc());
        expect(
            ruleIndices(form.items)
                .map((at) => form.items[at]!)
                .filter((item) => item.kind === "ifBlock")
                .map(describeIfBlock)
        ).toEqual([
            "IF row ~ COFFEE → account2 = expenses:food:coffee",
            "IF row ~ LANDLORD → account2 = expenses:home:rent",
            "IF description ~ SUPERMARKET OR description ~ GROCER → account2 = expenses:food:groceries, comment = weekly shop",
            "IF description ~ AIRLINE AND amount ~ ^- → account2 = expenses:travel:airfare",
        ]);
    });

    // A control word has no field and no value, so the `field = value` phrasing
    // has nothing to put on either side of the `=`. Saying what happens to the
    // ROW is the only reading that is both true and scannable.
    it("says what a skip/end does to the row rather than naming the keyword", () => {
        expect(describeIfBlock(rule([{matchers: [{field: "description", pattern: "PENDING"}]}], [], "skip"))).toBe("IF description ~ PENDING → skip this row");
        expect(describeIfBlock(rule([{matchers: [{field: "description", pattern: "PENDING"}]}], [], "end"))).toBe(
            "IF description ~ PENDING → stop reading here"
        );
    });

    // hledger accepts both in one block (the assignment is simply never used on
    // a skipped row), so the summary has to show both. The control word goes
    // last because it is what finally happens to the row.
    it("shows an assignment and a control word together, control last", () => {
        expect(describeIfBlock(rule([{matchers: [{field: "", pattern: "X"}]}], [{field: "account2", value: "expenses:y"}], "skip"))).toBe(
            "IF row ~ X → account2 = expenses:y, skip this row"
        );
    });
});

describe("UNIT imports model — dirty tracking", () => {
    it("is clean for an untouched form and dirty for any change", () => {
        const {baseline, form} = pair();
        expect(isDirty(baseline, form)).toBe(false);

        ruleAt(form, 1).groups[0]!.matchers[0]!.pattern = "ESPRESSO";
        expect(isDirty(baseline, form)).toBe(true);
    });

    it("goes clean again when the edit is undone by hand", () => {
        const {baseline, form} = pair();
        const rule = ruleAt(form, 1);
        rule.groups[0]!.matchers[0]!.pattern = "ESPRESSO";
        rule.groups[0]!.matchers[0]!.pattern = "COFFEE";
        expect(isDirty(baseline, form)).toBe(false);
    });

    it("notices a reorder even though every item is unchanged", () => {
        const {baseline, form} = pair();
        form.items = moveRule(form.items, 1, 2);
        expect(isDirty(baseline, form)).toBe(true);
    });

    it("notices an insert and a delete", () => {
        const added = pair();
        added.form.items = appendRule(added.form.items, blankRule());
        expect(isDirty(added.baseline, added.form)).toBe(true);

        const removed = pair();
        removed.form.items = removed.form.items.filter((item) => itemId(item) !== 9);
        expect(isDirty(removed.baseline, removed.form)).toBe(true);
    });

    // The signature has to see the control word for the same reason it has to
    // see the grouping: nothing else about the rule changes, but the file goes
    // from importing the row to dropping it. A signature that missed it would
    // send the rule back as an unchanged `keep` and the file would keep
    // importing rows the card says are skipped.
    it("is dirty when only the skip/end changed, with no character edited", () => {
        const {baseline, form} = pair();
        ruleAt(form, 1).control = "skip";
        expect(isDirty(baseline, form)).toBe(true);

        ruleAt(form, 1).control = null;
        expect(isDirty(baseline, form)).toBe(false);
    });

    it("notices a setting change and a setting removal", () => {
        const changed = pair();
        changed.form.items = withSetting(changed.form.items, "date-format", "%d/%m/%Y");
        expect(isDirty(changed.baseline, changed.form)).toBe(true);

        const cleared = pair();
        cleared.form.items = withSetting(cleared.form.items, "skip", "");
        expect(isDirty(cleared.baseline, cleared.form)).toBe(true);
    });
});

describe("UNIT imports model — settings", () => {
    it("rewrites a setting in place, keeping its id so the save is a replace", () => {
        const form = toForm(goldenDoc());
        const items = withSetting(form.items, "date-format", "%d/%m/%Y");
        expect(items).toHaveLength(form.items.length);
        expect(settingText(items, "date-format")).toBe("%d/%m/%Y");
        expect(settingIndex(items, "date-format")).toBe(3);
        expect(itemId(items[3] as FormItem)).toBe(3);
    });

    it("removes the line when a setting is cleared, rather than writing an empty one", () => {
        const form = toForm(goldenDoc());
        const items = withSetting(form.items, "skip", "");
        expect(items).toHaveLength(form.items.length - 1);
        expect(settingText(items, "skip")).toBeNull();
    });

    it("adds a missing setting after the settings already there, not above the file's header comment", () => {
        const form = toForm(goldenDoc());
        const items = withSetting(form.items, "separator", ";");
        // The golden opens with a comment run (item 0) and its last settings
        // item is the `account2` assignment (item 6).
        expect(items.findIndex((item) => item.kind === "directive" && item.name === "separator")).toBe(7);
        expect(itemId(items[7] as FormItem)).toBeNull();
        expect(items[0]?.kind).toBe("kept");
    });

    it("toggles a valueless directive by adding and removing the line", () => {
        const form = toForm(goldenDoc());
        const on = withFlag(form.items, "newest-first", true);
        expect(settingText(on, "newest-first")).toBe("");
        expect(on).toHaveLength(form.items.length + 1);

        const off = withFlag(on, "newest-first", false);
        expect(settingText(off, "newest-first")).toBeNull();
        expect(off).toHaveLength(form.items.length);

        // Toggling to the state it is already in changes nothing.
        expect(withFlag(form.items, "newest-first", false)).toHaveLength(form.items.length);
    });

    it("replaces the column mapping in place", () => {
        const form = toForm(goldenDoc());
        const items = withFieldNames(form.items, ["date", "description", "amount", ""]);
        expect(fieldNames(items)).toEqual(["date", "description", "amount", ""]);
        expect(itemId(items[2] as FormItem)).toBe(2);
    });

    // Last-one-wins, because that is what the engine's own projection does. The
    // earlier duplicate stays in the rules list rather than being hidden behind
    // a panel that silently speaks for it.
    it("speaks for the LAST duplicate of a setting and leaves the earlier one in the rules list", () => {
        const items: FormItem[] = [
            {kind: "directive", id: 0, name: "skip", value: "1"},
            {kind: "directive", id: 1, name: "skip", value: "2"},
        ];
        expect(settingIndex(items, "skip")).toBe(1);
        expect(settingText(items, "skip")).toBe("2");
        expect(ruleIndices(items)).toEqual([0]);
    });

    it("treats a keep-only `source` directive as a setting it can show but never rewrite", () => {
        const items: FormItem[] = [
            {
                kind: "kept",
                id: 0,
                source: {kind: "directive", id: 0, line: 1, lines: 1, name: "source", value: "statement.csv"},
            },
        ];
        expect(settingText(items, "source")).toBe("statement.csv");
        // Claimed by the preferences panel, so it is not also in the rules list.
        expect(ruleIndices(items)).toEqual([]);
    });
});

describe("UNIT imports model — reordering the rules list", () => {
    it("moves a rule within the list and leaves every other item where it was", () => {
        const form = toForm(goldenDoc());
        const moved = moveRule(form.items, 1, 2);
        expect(moved.map(itemId)).toEqual([0, 1, 2, 3, 4, 5, 6, 8, 7, 9, 10, 11]);
        // The settings items never move — they are not in the rules list.
        expect(moved.slice(1, 7).map(itemId)).toEqual([1, 2, 3, 4, 5, 6]);
    });

    it("can move the advanced item, which is listed and movable but not editable", () => {
        const form = toForm(goldenDoc());
        // Rules-list position 5 is the `if` table; 4 is the AND-group block.
        expect(moveRule(form.items, 5, 1).map(itemId)).toEqual([0, 1, 2, 3, 4, 5, 6, 11, 7, 8, 9, 10]);
    });

    it("is a no-op at the bounds", () => {
        const form = toForm(goldenDoc());
        expect(moveRule(form.items, 0, -1).map(itemId)).toEqual(form.items.map(itemId));
        expect(moveRule(form.items, 5, 6).map(itemId)).toEqual(form.items.map(itemId));
    });

    it("appends a new rule LAST, below every rule in the document", () => {
        const form = toForm(goldenDoc());
        const items = appendRule(form.items, blankRule("expenses:unknown"));
        expect(items).toHaveLength(form.items.length + 1);
        // Later matches win, so a new rule belongs below every rule it should be
        // able to override — including the trailing advanced one.
        const added = items[items.length - 1];
        expect(added?.kind).toBe("ifBlock");
        expect(itemId(added as FormItem)).toBeNull();
        expect(items.map(itemId)).toEqual([0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, null]);
    });

    // A conditional table's extent is terminated by a BLANK LINE, so a table
    // ending at EOF has none to move with it and used to swallow whatever was
    // written beneath it. The GUI dodged that by landing above the trailing
    // opaque run; the engine now supplies the blank line, so the rule goes where
    // the user asked for it — which is also the position "later matches win"
    // makes the useful one.
    it("lands BELOW a trailing advanced construct, which the engine now keeps separate", () => {
        const items: FormItem[] = [
            {kind: "assignment", id: 0, field: "account2", value: "expenses:unknown"},
            {
                kind: "ifBlock",
                id: 1,
                groups: [{matchers: [{field: "", pattern: "COFFEE"}]}],
                assignments: [{field: "account2", value: "expenses:food"}],
                control: null,
            },
            {
                kind: "kept",
                id: 2,
                source: {kind: "opaque", id: 2, line: 5, lines: 2, reason: "ifTable", label: "if,account2", text: "if,account2\nX,y\n", truncated: false},
            },
        ];
        const out = appendRule(items, blankRule());
        expect(out.map(itemId)).toEqual([0, 1, 2, null]);
    });

    it("appends at the end when nothing is in the way", () => {
        const items: FormItem[] = [
            {kind: "assignment", id: 0, field: "account2", value: "expenses:unknown"},
            {
                kind: "ifBlock",
                id: 1,
                groups: [{matchers: [{field: "", pattern: "COFFEE"}]}],
                assignments: [{field: "account2", value: "expenses:food"}],
                control: null,
            },
        ];
        expect(appendRule(items, blankRule()).map(itemId)).toEqual([0, 1, null]);
    });

    it("seeds a new rule with the file's fallback account, so the commonest edit is one field", () => {
        expect(blankRule("expenses:unknown").assignments).toEqual([{field: "account2", value: "expenses:unknown"}]);
        expect(blankRule().assignments[0]?.value).toBe("");
    });
});

describe("UNIT imports model — validation", () => {
    const withRule = (rule: Partial<IfBlockItem>): RulesForm => ({
        id: "x.rules",
        label: "x",
        revision: "r",
        editable: true,
        items: [
            {
                kind: "ifBlock",
                id: null,
                groups: [{matchers: [{field: "", pattern: "OK"}]}],
                assignments: [{field: "account2", value: "expenses:x"}],
                control: null,
                ...rule,
            },
        ],
    });

    it("passes the golden document unchanged", () => {
        expect(validateForm(toForm(goldenDoc()))).toEqual([]);
    });

    it("requires a rule to have something to match and something to set", () => {
        expect(validateForm(withRule({groups: []}))[0]).toContain("at least one thing to match");
        expect(validateForm(withRule({assignments: []}))[0]).toContain("at least one field to set");
    });

    // hledger accepts `if COND / skip` with no assignment at all, and so does
    // the engine's `check_body` — so a rule that only drops the row is complete,
    // not half-written. Refusing it here would block a legal file from saving.
    it("accepts a rule that only skips the row, with no field set at all", () => {
        expect(validateForm(withRule({assignments: [], control: "skip"}))).toEqual([]);
        expect(validateForm(withRule({assignments: [], control: "end"}))).toEqual([]);
    });

    it("refuses the matcher shapes hledger would read as something else", () => {
        const refused: [string, string][] = [
            ["", "empty"],
            [" leading", "cannot start with a space"],
            ["& AND", "+ AND condition"],
            ["! NOT", "needs the command line"],
            ["; looks like a comment", "`;`, `#` or `*`"],
            ["# also", "`;`, `#` or `*`"],
            ["A && B", "`&&` joins"],
            ["GROC(ER|ERY)", "literal `(`"],
            ["back \\1 reference", "backreferences"],
            ["%description NARROW", "%field pattern"],
        ];
        for (const [pattern, expected] of refused) {
            const errors = validateForm(withRule({groups: [{matchers: [{field: "", pattern}]}]}));
            expect(errors.join(" "), `pattern ${JSON.stringify(pattern)}`).toContain(expected);
        }
    });

    it("accepts an escaped parenthesis, which is a literal one", () => {
        expect(validateForm(withRule({groups: [{matchers: [{field: "", pattern: "ACME \\(UK\\)"}]}]}))).toEqual([]);
    });

    it("accepts a scoped matcher whose pattern begins with `%` once it is scoped", () => {
        // Scoped, so hledger reads the pattern as a pattern — the refusal only
        // applies to a WHOLE-RECORD matcher that would be silently narrowed.
        expect(validateForm(withRule({groups: [{matchers: [{field: "description", pattern: "%something else"}]}]}))).toEqual([]);
    });

    it("refuses assignment fields hledger does not have, and the ones it will not take here", () => {
        expect(validateForm(withRule({assignments: [{field: "", value: "x"}]}))[0]).toContain("pick a field");
        expect(validateForm(withRule({assignments: [{field: "skip", value: "1"}]}))[0]).toContain("`skip` is a setting");
        expect(validateForm(withRule({assignments: [{field: "end", value: ""}]}))[0]).toContain("control flow");
        expect(validateForm(withRule({assignments: [{field: "acount2", value: "x"}]}))[0]).toContain("not an hledger CSV field name");
    });

    it("refuses an assignment value hledger would read back differently", () => {
        expect(validateForm(withRule({assignments: [{field: "account2", value: "   padded"}]}))[0]).toContain("cannot start with a space");
        expect(validateForm(withRule({assignments: [{field: "comment", value: "see \\1"}]}))[0]).toContain("backreferences");
    });

    it("checks the settings the panels write", () => {
        const settings = (items: FormItem[]): string[] => validateForm({id: "x", label: "x", revision: "r", editable: true, items});
        expect(settings([{kind: "directive", id: null, name: "skip", value: "one"}])[0]).toContain("whole number");
        expect(settings([{kind: "directive", id: null, name: "separator", value: ",,"}])[0]).toContain("one character");
        expect(settings([{kind: "directive", id: null, name: "separator", value: "TAB"}])).toEqual([]);
        expect(settings([{kind: "directive", id: null, name: "decimal-mark", value: ".."}])[0]).toContain("exactly one character");
        expect(settings([{kind: "directive", id: null, name: "balance-type", value: "~"}])[0]).toContain("must be one of");
        expect(settings([{kind: "directive", id: null, name: "date-format", value: ""}])[0]).toContain("empty");
        // A value that begins with a space is refused before it is judged
        // empty, because hledger reads that run as the separator and the value
        // would be written and then read back without it.
        expect(settings([{kind: "directive", id: null, name: "date-format", value: " %Y"}])[0]).toContain("cannot start with a space");
    });

    it("requires the column mapping to name at least two columns", () => {
        const form: RulesForm = {id: "x", label: "x", revision: "r", editable: true, items: [{kind: "fields", id: null, names: ["date"]}]};
        expect(validateForm(form)[0]).toContain("at least two columns");
    });

    it("allows an empty column name, which is how hledger spells `ignore this column`", () => {
        const form: RulesForm = {id: "x", label: "x", revision: "r", editable: true, items: [{kind: "fields", id: null, names: ["date", "", "amount"]}]};
        expect(validateForm(form)).toEqual([]);
    });
});

describe("UNIT imports model — hledger field names", () => {
    it("accepts the plain names and the numbered families", () => {
        for (const name of ["date", "date2", "description", "amount", "amount-in", "amount-out", "account1", "account99", "comment2", "amount1-in"]) {
            expect(isHledgerField(name), name).toBe(true);
        }
    });

    it("refuses names hledger's own list does not contain", () => {
        // `account01` is rejected because hledger builds its name list from
        // `show <$> [1..99]`, so a leading zero produces a name not in it.
        for (const name of ["account0", "account01", "account100", "acount2", "date3", "", "Amount", "description-in"]) {
            expect(isHledgerField(name), name).toBe(false);
        }
    });
});

describe("UNIT imports model — describing what the GUI will not edit", () => {
    it("marks the `if` table advanced, and says why in a sentence", () => {
        const form = toForm(goldenDoc());
        const opaque = form.items.find((item) => itemId(item) === 11) as FormItem;
        const summary = describeItem(opaque);
        expect(summary.advanced).toBe(true);
        expect(summary.title).toBe("if,account2,comment");
        expect(summary.detail).toContain("positionally");
        expect(summary.text).toContain("ATM WITHDRAWAL,assets:cash,cash out");
    });

    it("does not call a comment run advanced — it is kept, not refused", () => {
        const form = toForm(goldenDoc());
        const trivia = form.items.find((item) => itemId(item) === 0) as FormItem;
        expect(describeItem(trivia)).toMatchObject({title: "Comment", advanced: false});
    });

    it("renders each item the way it reads in the file", () => {
        const form = toForm(goldenDoc());
        expect(itemText(form.items[1] as FormItem)).toBe("skip 1");
        expect(itemText(form.items[2] as FormItem)).toBe("fields date, description, amount");
        expect(itemText(form.items[5] as FormItem)).toBe("account1 assets:bank:checking");
        expect(itemText(form.items[7] as FormItem)).toBe("if COFFEE\n    account2 expenses:food:coffee");
    });
});

describe("UNIT imports model — arbitrary column names", () => {
    // A `fields` name does two different jobs through one syntax, and the row
    // mapping has to say which one is happening — otherwise a typo'd `dat`
    // looks identical to a deliberate `%cat`.
    it("reports a built-in name as assigning that field", () => {
        expect(columnRole("date")).toEqual({kind: "assigns", name: "date"});
        expect(columnRoleHint("date")).toBe("sets date");
        expect(columnRoleHint("amount-in")).toBe("sets amount-in");
        expect(columnRoleHint("account2")).toBe("sets account2");
    });

    // The case this change exists for: `fields …, cat` makes `%cat` usable in
    // `comment category:%cat`, and no dropdown could ever have offered it.
    it("reports any other name as a label for interpolation", () => {
        expect(columnRole("cat")).toEqual({kind: "names", name: "cat"});
        expect(columnRoleHint("cat")).toBe("available as %cat");
        expect(columnRoleHint("merchant_id")).toBe("available as %merchant_id");
        expect(columnRoleHint("Trans-Date")).toBe("available as %Trans-Date");
    });

    it("treats an empty name as hledger's skip-this-column, not a missing value", () => {
        expect(columnRole("")).toEqual({kind: "ignored"});
        expect(columnRoleHint("")).toBe("not imported");
    });

    // `account01` and `account100` are not hledger names, so they are custom
    // labels rather than assignments — the hint must not claim otherwise.
    it("does not mistake a near-miss numbered field for a built-in", () => {
        expect(columnRoleHint("account01")).toBe("available as %account01");
        expect(columnRoleHint("account100")).toBe("available as %account100");
        expect(columnRoleHint("account99")).toBe("sets account99");
    });

    it("accepts an arbitrary name through the fields list and validates it", () => {
        const form = toForm(goldenDoc());
        const at = fieldsIndex(form.items);
        expect(at).not.toBeNull();
        const next = withFieldNames(form.items, ["date", "description", "amount", "cat"]);
        expect(fieldNames(next)).toEqual(["date", "description", "amount", "cat"]);
        expect(validateForm({...form, items: next})).toEqual([]);
    });

    it("rejects a name hledger's grammar cannot hold, and says what is allowed", () => {
        const form = toForm(goldenDoc());
        const next = withFieldNames(form.items, ["date", "description", "amount", "my category"]);
        const errors = validateForm({...form, items: next});
        expect(errors).toHaveLength(1);
        expect(errors[0]).toContain("letters, digits, hyphens and underscores");
    });
});
