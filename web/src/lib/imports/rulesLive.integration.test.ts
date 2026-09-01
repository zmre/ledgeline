// INTEGRATION check against a live ledgeline-server. Skipped unless
// LEDGELINE_API_URL is set — `just test-integration` sets it:
//   target/debug/ledgeline --server fixtures/sample.journal --port 5055
//   LEDGELINE_API_URL=http://127.0.0.1:5055 vitest run rulesLive
//
// Drives the SAME pipeline the Imports screen does — LedgelineApi → nativeDecode
// → `toForm` → an edit → `toSaveRequest` → PUT → decode the saved document — so a
// green run is proof that the model produces a request the engine accepts and
// that the bytes on disk end up as the model said they would, short of a DOM.
//
// It is the half of the Imports contract a unit test cannot reach. `model.test.ts`
// proves the save request accounts for every item; only a live engine can prove
// that "accounts for every item" is the same thing the engine means by it, and
// that a `keep` really does re-emit the file's own bytes.
//
// # This test writes to disk, so it writes only to a scratch file of its own
//
// The engine under `just test-integration` is bound to `fixtures/sample.journal`,
// whose directory is the rules discovery scan root — so every committed `*.rules`
// fixture is visible. Those are read here and never written. The mutating half
// runs against a file this test creates under `fixtures/scratch/` (gitignored)
// and deletes afterwards, so a run leaves the tree exactly as it found it.

import {mkdirSync, rmSync, writeFileSync, readFileSync} from "node:fs";
import {afterAll, beforeAll, describe, expect, it} from "vitest";
import {LedgelineApi} from "$lib/api/native";
import {decodeRulesDoc, decodeRulesIndex, decodeRulesPreview} from "$lib/api/nativeDecode";
import {appendRule, fieldNames, isDirty, itemId, moveRule, ruleIndices, settingText, toForm, toSaveRequest, validateForm, withSetting} from "./model";
import type {IfBlockItem, RulesForm} from "./model";

const apiUrl = process.env.LEDGELINE_API_URL;

const SCRATCH_DIR = new URL("../../../../fixtures/scratch/imports-integration/", import.meta.url);
const SCRATCH_FILE = new URL("live.csv.rules", SCRATCH_DIR);
const SCRATCH_ID = "scratch/imports-integration/live.csv.rules";

// One of each shape the round trip has to survive: a standalone comment run, the
// settings the panels own, two editable OR-list rules, an editable AND-group,
// and a conditional TABLE the engine refuses to classify. The table is the point
// — it is the item that only survives because the client echoes it back as
// `{kind:"keep", id}`. The AND-group is the other point: the `&` is grammar the
// ENGINE writes, and only a live save proves the nesting we send comes back as
// the same nesting rather than as two OR-ed alternatives.
const SCRATCH = `# Scratch file for rulesLive.integration.test.ts.

skip 1
fields date, description, amount
date-format %m/%d/%Y
account1 assets:bank:checking
account2 expenses:unknown

if COFFEE
    account2 expenses:food:coffee

if LANDLORD
    account2 expenses:home:rent

# An AND-group: the continuation line's & prefix means BOTH have to match.
if
%description AIRLINE
& %amount ^-
    account2 expenses:travel:airfare

# A conditional TABLE: keep-only, and it must come back byte for byte.
if,account2,comment
ATM WITHDRAWAL,assets:cash,cash out
`;

function api(): LedgelineApi {
    return new LedgelineApi(apiUrl ?? "");
}

/** Read the open document and build the baseline/live pair the page holds. */
async function open(id: string): Promise<{baseline: RulesForm; form: RulesForm}> {
    const doc = decodeRulesDoc(await api().getRules(id));
    return {baseline: toForm(doc), form: toForm(doc)};
}

function ruleAt(form: RulesForm, position: number): IfBlockItem {
    const item = form.items[ruleIndices(form.items)[position] ?? -1];
    if (item === undefined || item.kind !== "ifBlock") throw new Error(`no rule at rules-list position ${position}`);
    return item;
}

describe.runIf(apiUrl !== undefined && apiUrl !== "")("INTEGRATION live ledgeline-server import rules", () => {
    beforeAll(() => {
        mkdirSync(SCRATCH_DIR, {recursive: true});
        writeFileSync(SCRATCH_FILE, SCRATCH);
    });
    afterAll(() => {
        rmSync(SCRATCH_DIR, {recursive: true, force: true});
    });

    it("lists the rules files beside the journal, summarized", async () => {
        const index = decodeRulesIndex(await api().listRules());
        expect(index.rootLabel).toBe("fixtures");
        expect(index.editable).toBe(true);
        const checking = index.files.find((file) => file.id === "rules/simple/checking.csv.rules");
        expect(checking).toMatchObject({label: "checking", parsed: true, account1: "assets:bank:checking", account2: "expenses:unknown", ifBlockCount: 3});
        // The scratch file this suite created is discovered by the SAME scan.
        expect(index.files.some((file) => file.id === SCRATCH_ID)).toBe(true);
    });

    it("opens a committed rules file and reads its settings and rules back", async () => {
        const {form} = await open("rules/simple/checking.csv.rules");
        expect(settingText(form.items, "date-format")).toBe("%m/%d/%Y");
        expect(settingText(form.items, "account2")).toBe("expenses:unknown");
        expect(fieldNames(form.items)).toEqual(["date", "description", "amount"]);
        expect(validateForm(form)).toEqual([]);
        // Three editable rules plus the file's leading comment run.
        expect(ruleIndices(form.items)).toHaveLength(4);
    });

    it("previews the data file so a column can be labelled with what it holds", async () => {
        const preview = decodeRulesPreview(await api().previewRules("rules/simple/checking.csv.rules"));
        expect(preview.available).toBe(true);
        expect(preview.dataLabel).toBe("checking.csv");
        expect(preview.header).toEqual(["Date", "Description", "Amount"]);
        expect(preview.rows[0]).toEqual(["01/15/2024", "ACME PAYROLL", "3000.00"]);
    });

    it("reports the typed reason when there is no data file to preview", async () => {
        const preview = decodeRulesPreview(await api().previewRules("rules/edge/bom.rules"));
        expect(preview.available).toBe(false);
        expect(preview.reason).toBe("noDataFile");
    });

    // An untouched document saves as nothing but `keep`s, and the engine's no-op
    // short-circuit then writes NOTHING — not even a fresh mtime, which somebody
    // else's `entr` or `hledger import` watch loop would see as a change.
    //
    // Run against the SCRATCH file rather than a committed fixture on purpose:
    // if this property ever regressed, the committed version of the test would
    // rewrite a corpus file as a side effect of running the suite.
    it("saves an untouched document without changing a single byte", async () => {
        const before = readFileSync(SCRATCH_FILE, "utf8");
        const {baseline, form} = await open(SCRATCH_ID);
        expect(isDirty(baseline, form)).toBe(false);

        const body = toSaveRequest(baseline, form);
        expect(body.items.every((item) => item.kind === "keep")).toBe(true);
        const saved = decodeRulesDoc(await api().saveRules(SCRATCH_ID, body));
        expect(saved.revision).toBe(baseline.revision);
        expect(readFileSync(SCRATCH_FILE, "utf8")).toBe(before);
    });

    it("reorders two rules and writes the new order, keeping every other byte", async () => {
        const {baseline, form} = await open(SCRATCH_ID);
        expect(ruleAt(form, 1).groups[0]?.matchers[0]?.pattern).toBe("COFFEE");
        expect(ruleAt(form, 2).groups[0]?.matchers[0]?.pattern).toBe("LANDLORD");

        // Position 0 is the leading comment run, so the first rule is at 1.
        form.items = moveRule(form.items, 1, 2);
        expect(isDirty(baseline, form)).toBe(true);

        const saved = decodeRulesDoc(await api().saveRules(SCRATCH_ID, toSaveRequest(baseline, form)));
        const reopened = toForm(saved);
        expect(ruleAt(reopened, 1).groups[0]?.matchers[0]?.pattern).toBe("LANDLORD");
        expect(ruleAt(reopened, 2).groups[0]?.matchers[0]?.pattern).toBe("COFFEE");

        const text = readFileSync(SCRATCH_FILE, "utf8");
        expect(text.indexOf("LANDLORD")).toBeLessThan(text.indexOf("COFFEE"));
        // Nothing else moved or vanished: the settings, the header comment and
        // the conditional table are all still there, in place.
        expect(text).toContain("# Scratch file for rulesLive.integration.test.ts.");
        expect(text).toContain("ATM WITHDRAWAL,assets:cash,cash out");
        expect(text).toContain("date-format %m/%d/%Y");
    });

    it("edits one rule and one setting, leaving the advanced construct byte-identical", async () => {
        const {baseline, form} = await open(SCRATCH_ID);
        ruleAt(form, 1).assignments[0]!.value = "expenses:food:cafe";
        form.items = withSetting(form.items, "date-format", "%d/%m/%Y");

        const body = toSaveRequest(baseline, form);
        // Only the two edited items carry a body; everything else — the comment
        // run, the `fields` line, the accounts, the conditional table — is a
        // `keep`, which is what re-emits the file's own bytes.
        expect(body.items.filter((item) => item.kind !== "keep")).toHaveLength(2);
        expect(body.delete).toEqual([]);

        const saved = decodeRulesDoc(await api().saveRules(SCRATCH_ID, body));
        const reopened = toForm(saved);
        expect(settingText(reopened.items, "date-format")).toBe("%d/%m/%Y");
        expect(ruleAt(reopened, 1).assignments[0]?.value).toBe("expenses:food:cafe");
        expect(readFileSync(SCRATCH_FILE, "utf8")).toContain("ATM WITHDRAWAL,assets:cash,cash out");
    });

    // This scratch file ENDS IN A CONDITIONAL TABLE, whose extent is terminated
    // by a blank line it does not have — so a rule appended after it used to be
    // read back as another row of the table and refused. The engine now supplies
    // that blank line, and this is the live proof: the rule appends LAST,
    // against the real server, and the save is accepted.
    it("adds a rule and deletes one, and the engine accepts the plan", async () => {
        const {baseline, form} = await open(SCRATCH_ID);
        const before = ruleIndices(form.items).length;
        const doomed = itemId(form.items[ruleIndices(form.items)[1] ?? -1]!);

        form.items = appendRule(form.items, {
            kind: "ifBlock",
            id: null,
            groups: [{matchers: [{field: "description", pattern: "PHARMACY"}]}],
            assignments: [{field: "account2", value: "expenses:health"}],
            control: null,
        });
        form.items = form.items.filter((item) => itemId(item) !== doomed);

        const body = toSaveRequest(baseline, form);
        expect(body.delete).toEqual([doomed]);
        const saved = decodeRulesDoc(await api().saveRules(SCRATCH_ID, body));
        const reopened = toForm(saved);
        expect(ruleIndices(reopened.items)).toHaveLength(before);
        const text = readFileSync(SCRATCH_FILE, "utf8");
        expect(text).toContain("%description PHARMACY");
        expect(text).toContain("account2 expenses:health");
        // Last in the file, after the table — where later-matches-win puts it —
        // and read back as a rule of its own rather than as table rows.
        expect(text.indexOf("ATM WITHDRAWAL")).toBeLessThan(text.indexOf("PHARMACY"));
        expect(reopened.items[reopened.items.length - 1]?.kind).toBe("ifBlock");
    });

    // The half of the grouped shape only a live engine can prove: what we send
    // as NESTING has to come back as the same nesting, and be spelled in the
    // file with the `&` prefix hledger reads as an AND. A renderer that wrote
    // the two matchers as two plain lines would pass every unit test in this
    // repo and quietly change the rule from "both" to "either".
    it("reads an AND-group back as one group, and writes the `&` the file needs", async () => {
        const {baseline, form} = await open(SCRATCH_ID);
        // Found by SHAPE, not by position: the tests above this one reorder,
        // delete and append rules in the same scratch file, so a fixed index
        // would be asserting about whichever rule happened to land there.
        const rule = form.items.find((item): item is IfBlockItem => item.kind === "ifBlock" && item.groups.some((group) => group.matchers.length > 1));
        if (rule === undefined) throw new Error("the AND-group rule is missing from the scratch file");
        expect(rule.groups.map((group) => group.matchers.map((matcher) => matcher.pattern))).toEqual([["AIRLINE", "^-"]]);

        // Add a second condition to the SAME group — the "+ AND condition" the
        // card offers — and a whole new OR branch beside it.
        rule.groups[0]!.matchers = [...rule.groups[0]!.matchers, {field: "description", pattern: "SEAT"}];
        rule.groups = [...rule.groups, {matchers: [{field: "description", pattern: "RAILWAY"}]}];
        expect(isDirty(baseline, form)).toBe(true);
        expect(validateForm(form)).toEqual([]);

        const saved = toForm(decodeRulesDoc(await api().saveRules(SCRATCH_ID, toSaveRequest(baseline, form))));
        const reread = saved.items.find((item): item is IfBlockItem => item.kind === "ifBlock" && item.groups.some((group) => group.matchers.length > 1));
        expect(reread?.groups.map((group) => group.matchers.map((matcher) => matcher.pattern))).toEqual([["AIRLINE", "^-", "SEAT"], ["RAILWAY"]]);

        const text = readFileSync(SCRATCH_FILE, "utf8");
        // Continuations carry the `&` and the new OR branch does not — which is
        // the entire difference between "both" and "either" on disk.
        expect(text).toContain("%description AIRLINE\n& %amount ^-\n& %description SEAT\n%description RAILWAY\n");
    });

    // The whole point of `revision`: a save against bytes somebody else has
    // already replaced is a 409 and writes nothing, rather than a silent clobber.
    it("refuses a save against a stale revision", async () => {
        const {baseline, form} = await open(SCRATCH_ID);
        // The revision is checked BEFORE any item id is resolved, so a stale one
        // is refused even for an edit that is otherwise perfectly valid.
        ruleAt(form, 1).assignments[0]!.value = "expenses:something-else";
        const body = {...toSaveRequest(baseline, form), revision: "0-deadbeefdeadbeef"};
        await expect(api().saveRules(SCRATCH_ID, body)).rejects.toThrow(/changed on disk/);
    });

    // Layer 4 of the engine's security model, from the client side: there is no
    // value in the form model that can be rendered into a `source` line, because
    // `source | CMD` is a shell command hledger runs on import.
    it("has no way to express a `source` directive in a save request", async () => {
        const {baseline, form} = await open(SCRATCH_ID);
        const body = toSaveRequest(baseline, form);
        expect(body.items.some((item) => item.kind === "directive" && item.name === "source")).toBe(false);
    });
});
