// Imports screen (steps 9-10): list the rules files beside the journal, edit one
// through structured controls, reorder its rules, and save.
//
// # Why this spec writes its own fixture
//
// The e2e engine is launched over `fixtures/sample.journal`, whose directory IS
// the rules discovery scan root — so every `*.rules` file under `fixtures/` is
// already listed, committed corpus included. That is fine to READ and not fine
// to WRITE: a save test that reordered `fixtures/rules/simple/checking.csv.rules`
// would rewrite a committed fixture, leave the working tree dirty, and stop
// being idempotent the second time it ran.
//
// So the file under test is written here, fresh before every test, into a
// scratch directory that is gitignored and removed afterwards. Nothing about
// `fixtures/sample.journal` or the corpus changes, which is what keeps the other
// four specs — every one of which asserts numbers derived from that journal —
// working untouched.
import {rmSync, mkdirSync, readFileSync, writeFileSync} from "node:fs";
import {expect, test, type Page} from "@playwright/test";
import {API_TOKEN} from "../playwright.config";

const SCRATCH_DIR = new URL("../../fixtures/scratch/imports-e2e/", import.meta.url);
const SCRATCH_RULES = new URL("scratch.csv.rules", SCRATCH_DIR);
const SCRATCH_CSV = new URL("scratch.csv", SCRATCH_DIR);

// Deliberately one of each shape the screen has to handle: a standalone comment
// run, the settings the three panels own, three editable OR-list rules, one
// editable AND-group, and one conditional TABLE the engine classifies `opaque`
// and this GUI must show read-only. `%m/%d/%Y` rather than ISO so the
// date-format example is a non-trivial one.
const RULES = `# Written by web/e2e/imports.e2e.ts. Not a committed fixture.

skip 1
fields date, description, amount
date-format %m/%d/%Y
currency $
account1 assets:bank:checking
account2 expenses:unknown

if ACME PAYROLL
    account2 income:salary

if COFFEE
    account2 expenses:food:coffee

if LANDLORD
    account2 expenses:home:rent

# An AND-group: the continuation line's & prefix means BOTH must match, which is
# a different rule from the same two matchers on two plain lines.
if
%description LANDLORD
& %amount ^-1
    account2 expenses:home:rent:large

# A conditional TABLE. hledger reads each row positionally against the header,
# so Ledgeline never offers one for editing.
if,account2,comment
ATM WITHDRAWAL,assets:cash,cash out
`;

const CSV = `Date,Description,Amount
01/15/2024,ACME PAYROLL,3000.00
01/16/2024,COFFEE HOUSE,-6.45
01/17/2024,LANDLORD LLC,-1850.00
`;

test.beforeEach(async ({page}) => {
    // Rewritten per test: the reorder test edits this file, and every test has
    // to start from the same bytes.
    mkdirSync(SCRATCH_DIR, {recursive: true});
    writeFileSync(SCRATCH_CSV, CSV);
    writeFileSync(SCRATCH_RULES, RULES);
    await page.addInitScript((token) => {
        localStorage.setItem("ledgeline.settings.v1", JSON.stringify({serverUrl: "http://127.0.0.1:5099", serverToken: token}));
    }, API_TOKEN);
});

test.afterAll(() => {
    rmSync(SCRATCH_DIR, {recursive: true, force: true});
});

/**
 * Open the Edit Rules tab and select the scratch file.
 *
 * `?tab=rules` rather than a click: Imports opens on New Transactions now, and
 * the tab is restored from the query string on mount — so this is both the
 * shortest route to the editor and a check that a shared/reloaded URL lands
 * where it says it does. The tab STRIP is exercised by a click below.
 */
async function openScratch(page: Page): Promise<void> {
    await page.goto("/imports?tab=rules");
    await page.getByRole("button", {name: /^scratch/}).click();
    await expect(page.getByTestId("imports-open-file")).toHaveText("scratch");
}

/** The card of each editable rule, in the order they are listed — a one-line summary until it is opened. */
function ruleSummaries(page: Page) {
    return page.getByTestId("imports-rule");
}

/**
 * Open one rule for editing.
 *
 * Rules-list POSITION, not rule number: position 1 is the file's leading comment
 * run, so the first editable rule is 2. That is deliberate — a reorder moves a
 * rule past whatever is next, comment or not, and the numbering on screen says
 * so.
 */
async function openRule(page: Page, position: number): Promise<void> {
    await page.getByRole("button", {name: `Edit rule ${position}`}).click();
    await expect(page.getByRole("button", {name: `Close rule ${position}`})).toBeVisible();
}

test("navigates to Imports and lists the rules files beside the journal", async ({page}) => {
    await page.goto("/");
    await page.getByRole("link", {name: "Imports"}).click();
    await expect(page).toHaveTitle("Ledgeline — Imports");

    // Imports opens on New Transactions, so the rules editor is one tab away.
    // Asserted rather than clicked past: which tab is the landing one is a
    // decision, and a silent flip back to the editor would go unnoticed.
    await expect(page.getByRole("tab", {name: "New Transactions"})).toHaveAttribute("aria-selected", "true");
    // The panel root, not any section inside it. Which sections render depends
    // on whether the test machine has hledger and whether a journal is bound,
    // and neither is what this test is about.
    await expect(page.getByTestId("imports-new")).toBeVisible();

    await page.getByRole("tab", {name: "Edit Rules"}).click();
    await expect(page.getByRole("tab", {name: "Edit Rules"})).toHaveAttribute("aria-selected", "true");
    // The tab is mirrored into the URL, debounced — which is what makes the
    // `?tab=rules` entry point every other test uses reachable by sharing a link.
    await expect(page).toHaveURL(/\?tab=rules$/);

    // The scan root is named by LABEL — the engine deliberately never sends a path.
    await expect(page.getByText("the folder your journal is in").first()).toBeVisible();
    await expect(page.getByRole("button", {name: /^scratch/})).toBeVisible();

    // A real ledger has `2025/imports/capitalone.csv.rules` next to
    // `2026/imports/capitalone.csv.rules`, so the row has to say which folder it
    // came from — a list of bare labels shows the same name twice.
    const scratchRow = page.getByRole("button", {name: /^scratch/});
    await expect(scratchRow).toContainText("scratch/imports-e2e");

    // Every assertion below is SCOPED to that row on purpose. Two unrelated
    // files summarizing identically ("5 rules, 1 advanced" also describes the
    // committed tree fixture) is precisely the ambiguity this change exists to
    // fix, so a page-wide matcher would resolve to both and fail.
    //
    // The counts moved off the row into the tooltip, which `data-tip` draws as a
    // pseudo-element — invisible to assistive technology, so `sr-only` text
    // mirrors it into the accessible name. Assert the attribute for what is
    // drawn and the name for what is announced; they are two different
    // mechanisms and either can break alone.
    //
    // Five conditionals, one of which is the table: `ifBlockCount` counts every
    // conditional, `opaqueItemCount` only the ones this GUI will not edit.
    const scratchItem = page.locator("li.tooltip", {has: scratchRow});
    await expect(scratchItem).toHaveAttribute("data-tip", /^scratch\/imports-e2e\/scratch\.csv\.rules · 5 rules, 1 advanced/);
    await expect(scratchRow).toHaveAccessibleName(/scratch\/imports-e2e\/scratch\.csv\.rules · 5 rules, 1 advanced/);
    await expect(scratchRow).toHaveAccessibleName(/assets:bank:checking → expenses:unknown/);

    // The folder belongs UNDER the name, and this asserts the pixels rather than
    // the class list because the class list lied: daisyUI styles a menu item as
    // `display:grid; grid-auto-flow:column`, which makes a lone `flex-col` a
    // no-op and lays the two lines out side by side, the folder rendering as a
    // small grey suffix hanging off the name. Only a geometry check catches
    // that — `toHaveClass` passed the whole time it was broken.
    const parts = scratchRow.locator("> span");
    const nameBox = await parts.nth(0).boundingBox();
    const folderBox = await parts.nth(1).boundingBox();
    if (nameBox === null || folderBox === null) throw new Error("both row lines must be laid out");
    expect(folderBox.y).toBeGreaterThanOrEqual(nameBox.y + nameBox.height);
});

test("shows the settings, the date-format example and the real CSV columns", async ({page}) => {
    await openScratch(page);

    // The whole reason `date-format` gets a picker: seeing what it does to one
    // known date is what tells month-first from day-first.
    await expect(page.getByTestId("date-format-example")).toHaveText("01/15/2026");

    await page.getByRole("tab", {name: "Row mapping"}).click();
    // Each column labelled with the CSV's own header AND a real sample value —
    // which is what turns checking a mapping into an act of reading.
    const columns = page.getByRole("table").locator("tbody tr");
    await expect(columns).toHaveCount(3);
    await expect(columns.nth(1)).toContainText("Description");
    await expect(columns.nth(1)).toContainText("ACME PAYROLL");
    await expect(columns.nth(2)).toContainText("Amount");
    await expect(columns.nth(2)).toContainText("3000.00");
    await expect(page.getByLabel("Field name for column 3")).toHaveValue("amount");

    await page.getByRole("tab", {name: "Accounts"}).click();
    await expect(page.getByLabel("Account").first()).toHaveValue("assets:bank:checking");
});

test("names a column with an arbitrary field name and saves it", async ({page}) => {
    await openScratch(page);
    await page.getByRole("tab", {name: "Row mapping"}).click();

    // A `fields` name is not drawn from a fixed set: a name hledger does not
    // know simply labels the column, which is what makes `%cat` usable in a
    // later rule. The control has to accept free text for that to be possible
    // at all — a `<select>` could not express it.
    const columns = page.getByRole("table").locator("tbody tr");
    await expect(columns.nth(2)).toContainText("sets amount");

    const custom = page.getByLabel("Field name for column 3");
    await custom.fill("cat");
    await expect(columns.nth(2)).toContainText("available as %cat");

    await page.getByRole("button", {name: "Save"}).click();
    // The `saved` badge, NOT `Save` going disabled. Save is disabled by
    // `!dirty || rulesStore.saving`, so it goes disabled the instant the PUT is
    // sent — waiting on it resolves while the request is still in flight and
    // the read below races the write. `savedAt` is only set once the engine has
    // answered, which is the thing this test is actually about.
    await expect(page.getByTestId("imports-saved")).toBeVisible();

    // It is the FILE that has to carry the name, not just the form.
    expect(readFileSync(SCRATCH_RULES, "utf8")).toContain("fields date, description, cat");
});

test("an advanced construct is listed, locked, and has no edit control", async ({page}) => {
    await openScratch(page);

    const advanced = page.getByTestId("imports-locked-item").filter({has: page.getByTestId("imports-locked-badge")});
    await expect(advanced).toHaveCount(1);
    // Shown, with its raw text — hiding it would make the file appear to contain
    // less than it does, and a reorder would move rules past something invisible.
    await expect(advanced).toContainText("if,account2,comment");
    await expect(advanced).toContainText("ATM WITHDRAWAL,assets:cash,cash out");
    await expect(advanced).toContainText("edit in terminal");

    // The point of the whole `opaque` classification: no way to rewrite it here.
    await expect(advanced.getByRole("textbox")).toHaveCount(0);
    await expect(advanced.getByRole("combobox")).toHaveCount(0);
    // It is still MOVABLE, which is the other half of the contract.
    await expect(advanced.getByRole("button", {name: /^Move item \d+ up$/})).toBeEnabled();
});

test("reorders two rules, saves, and the new order survives a reload", async ({page}) => {
    await openScratch(page);

    await expect(ruleSummaries(page)).toHaveCount(4);
    await expect(ruleSummaries(page).nth(0)).toContainText("ACME PAYROLL");
    await expect(ruleSummaries(page).nth(1)).toContainText("COFFEE");

    // Save is inert until there is something to save.
    await expect(page.getByRole("button", {name: "Save"})).toBeDisabled();

    // Position 1 in the rules list is the file's leading comment run, so the
    // first editable rule sits at position 2.
    await page.getByRole("button", {name: "Move rule 2 down"}).click();
    await expect(ruleSummaries(page).nth(0)).toContainText("COFFEE");
    await expect(ruleSummaries(page).nth(1)).toContainText("ACME PAYROLL");
    await expect(page.getByTestId("imports-dirty")).toBeVisible();

    await page.getByRole("button", {name: "Save"}).click();
    await expect(page.getByTestId("imports-saved")).toBeVisible();
    await expect(page.getByTestId("imports-dirty")).toHaveCount(0);

    // Re-read from disk: the reorder is in the file, not just in the DOM.
    await page.reload();
    await openScratch(page);
    await expect(ruleSummaries(page).nth(0)).toContainText("COFFEE");
    await expect(ruleSummaries(page).nth(1)).toContainText("ACME PAYROLL");
    // And nothing else was lost — the advanced construct is still there.
    await expect(page.getByTestId("imports-locked-badge")).toHaveCount(1);
});

test("edits a rule's category and saves it", async ({page}) => {
    await openScratch(page);

    // The list is summaries until a rule is opened, so editing one is now two
    // steps: find it by its line, then open it.
    await expect(ruleSummaries(page).nth(1)).toContainText("IF row ~ COFFEE → account2 = expenses:food:coffee");
    await openRule(page, 3);

    const coffee = ruleSummaries(page).nth(1);
    const category = coffee.getByLabel("Account").first();
    await expect(category).toHaveValue("expenses:food:coffee");
    await category.fill("expenses:food:cafe");
    await category.blur();

    await expect(page.getByTestId("imports-dirty")).toBeVisible();
    await page.getByRole("button", {name: "Save"}).click();
    await expect(page.getByTestId("imports-saved")).toBeVisible();

    await page.reload();
    await openScratch(page);
    // Collapsed again after the reload, and the new category is on the line —
    // which is the whole point of the summary being the thing you read.
    await expect(ruleSummaries(page).nth(1)).toContainText("IF row ~ COFFEE → account2 = expenses:food:cafe");
    await openRule(page, 3);
    await expect(ruleSummaries(page).nth(1).getByLabel("Account").first()).toHaveValue("expenses:food:cafe");
});

// The redesign end to end: a scannable list, one rule opened, an AND condition
// added to the group it already has, saved, and read back off disk. The AND is
// grammar the ENGINE writes — the `&` prefix in the file is the only thing that
// distinguishes "both of these" from "either of these", and nothing typed here
// produces it.
test("opens a rule, adds an AND condition, and the file keeps the AND after a reload", async ({page}) => {
    await openScratch(page);

    // Every rule is one line, and no rule has an editable field until it is
    // opened. This is what the list looked like before the split: four rules of
    // stacked selects and inputs, and nothing to scan.
    await expect(ruleSummaries(page)).toHaveCount(4);
    await expect(ruleSummaries(page).nth(3)).toContainText("IF description ~ LANDLORD AND amount ~ ^-1 → account2 = expenses:home:rent:large");
    // Scoped to the rule cards: the Preferences panel above the list always has
    // fields of its own, and it is the RULES that have none until one is opened.
    await expect(ruleSummaries(page).getByRole("textbox")).toHaveCount(0);

    await openRule(page, 5);
    await expect(page.getByLabel("Rule 5, group 1, match 1 text")).toHaveValue("LANDLORD");
    await expect(page.getByLabel("Rule 5, group 1, match 2 text")).toHaveValue("^-1");

    await page.getByRole("button", {name: "Add an AND condition to group 1 of rule 5"}).click();
    await page.getByLabel("Rule 5, group 1, match 3 column").selectOption("description");
    await page.getByLabel("Rule 5, group 1, match 3 text").fill("LLC");

    await page.getByRole("button", {name: "Save"}).click();
    await expect(page.getByTestId("imports-saved")).toBeVisible();
    // A save hands the list back: the rule that was open is a summary again,
    // now carrying the condition that was just added.
    await expect(page.getByRole("button", {name: "Close rule 5"})).toHaveCount(0);
    await expect(ruleSummaries(page).nth(3)).toContainText("AND description ~ LLC");

    // The `&` prefix is in the FILE, on its own line — an added OR branch would
    // be the same three matchers without it, and would import differently.
    expect(readFileSync(SCRATCH_RULES, "utf8")).toContain("%description LANDLORD\n& %amount ^-1\n& %description LLC\n");

    await page.reload();
    await openScratch(page);
    await expect(ruleSummaries(page)).toHaveCount(4);
    await expect(ruleSummaries(page).nth(3)).toContainText("IF description ~ LANDLORD AND amount ~ ^-1 AND description ~ LLC");
    await expect(ruleSummaries(page).getByRole("textbox")).toHaveCount(0);
});

// Adding a rule opens it, because a blank summary line has nothing to scan and
// the next thing anyone does is type into it.
test("adds a rule, which opens for editing straight away", async ({page}) => {
    await openScratch(page);

    await page.getByRole("button", {name: "+ Add rule"}).click();

    // Position 7: the comment run, four rules, the conditional table, then this.
    await expect(page.getByRole("button", {name: "Close rule 7"})).toBeVisible();
    await page.getByLabel("Rule 7, group 1, match 1 text").fill("PHARMACY");
    await page.getByRole("button", {name: "Add an OR group to rule 7"}).click();
    await page.getByLabel("Rule 7, group 2, match 1 text").fill("CHEMIST");
    // `account2` gets the account autocomplete rather than a plain field, which
    // is the control this rule was seeded with the file's fallback in.
    await ruleSummaries(page).last().getByLabel("Account").first().fill("expenses:health");

    await page.getByRole("button", {name: "Save"}).click();
    await expect(page.getByTestId("imports-saved")).toBeVisible();

    // Two OR branches are two plain matcher lines — no `&` anywhere, which is
    // exactly the difference the nesting carries.
    const text = readFileSync(SCRATCH_RULES, "utf8");
    expect(text).toContain("PHARMACY\nCHEMIST\n");
    expect(text).toContain("account2 expenses:health");
});

test("switching files with unsaved changes asks before discarding them", async ({page}) => {
    await openScratch(page);

    await page.getByRole("button", {name: "Move rule 2 down"}).click();
    await expect(page.getByTestId("imports-dirty")).toBeVisible();

    // The folder is part of this locator on purpose. Labels are NOT unique —
    // the corpus holds a `checking` under both `rules/simple` and
    // `import/match`, which is exactly the case `RulesFileList` renders the
    // folder for. Matching on the label alone was a strict-mode violation
    // waiting for the second `checking` to be added, and it got one.
    const otherFile = page.getByRole("button", {name: /^checking rules\/simple/});

    // The inline two-step confirm, not a `beforeunload` guard: the click that
    // discards the edit is never the click that asked to switch.
    await otherFile.click();
    await expect(page.getByText("Discard your unsaved changes?")).toBeVisible();
    await expect(page.getByTestId("imports-open-file")).toHaveText("scratch");

    await page.getByRole("button", {name: "Keep editing"}).click();
    await expect(page.getByTestId("imports-open-file")).toHaveText("scratch");
    await expect(page.getByTestId("imports-dirty")).toBeVisible();

    await otherFile.click();
    await page.getByRole("button", {name: "Discard"}).click();
    await expect(page.getByTestId("imports-open-file")).toHaveText("checking");
});

// ---------------------------------------------------------------------------
// Creating a rules file from a dropped CSV (WP-16 Phase 2)
// ---------------------------------------------------------------------------

/**
 * A statement no committed rules file can read: exactly TWO columns.
 *
 * The narrowness is the mechanism, not a stylistic choice. `matching::prefilter`
 * rejects any rules file whose `fields` list is wider than the data's widest
 * record, and every rules file under `fixtures/` declares at least three — so a
 * two-column drop is guaranteed to score nothing and land on the empty state
 * this flow is reached from. A cleverer CSV would depend on hledger's scoring
 * and would start failing the day someone adds a fixture.
 *
 * The headers say nothing an English synonym table knows, so both columns are
 * read from their VALUES — which is the weakest guess the generator makes, and
 * therefore the one worth driving end to end.
 */
const ALIEN_CSV = `When,HowMuch
2026-03-01,-12.50
2026-03-02,-4.25
2026-03-03,900.00
`;

/** The id the flow writes. Removed before each run so the create is a real create. */
const CREATED_RULES = new URL("dropped.csv.rules", SCRATCH_DIR);

test("drafts a rules file for a CSV nothing can read, and writes it", async ({page}) => {
    rmSync(CREATED_RULES, {force: true});
    await page.goto("/imports");

    // The engine has to be able to run hledger for the candidate list to mean
    // anything. Without it the screen shows only the banner, and this test
    // would be asserting on the machine rather than on the feature.
    const drop = page.getByTestId("imports-drop-target");
    if ((await drop.count()) === 0) test.skip(true, "this engine cannot run hledger");

    await page.locator('input[type="file"]').setInputFiles({
        name: "dropped.csv",
        mimeType: "text/csv",
        buffer: Buffer.from(ALIEN_CSV),
    });

    // Nothing fits — which is the state the whole flow exists for.
    await expect(page.getByTestId("imports-no-candidates")).toBeVisible();
    await page.getByTestId("imports-create-rules").click();

    const panel = page.getByTestId("imports-create-rules-panel");
    await expect(panel).toBeVisible();

    // The name defaults from the CSV destination, because hledger FINDS a rules
    // file by being named after its data file.
    await expect(page.getByTestId("imports-create-id")).toHaveValue(/dropped\.csv\.rules$/);

    // Both columns were read from their values, so both are marked as guesses
    // rather than presented as facts.
    await expect(page.getByTestId("imports-create-uncertain")).toBeVisible();

    // Both columns were read from their VALUES, so the mapping is shown for
    // checking rather than asserted. `HowMuch` really is the amount, so the
    // correction this test makes is the one the panel's own warning asks for:
    // these amounts carry no currency symbol, and hledger reads a
    // commodity-less amount as a commodity of its OWN — so those rows would
    // never add up with the `$` amounts already in the journal, visible only as
    // a balance that does not move.
    await expect(page.getByLabel("Field name for column 1")).toHaveValue("date");
    await expect(page.getByLabel("Field name for column 2")).toHaveValue("amount");
    await expect(page.getByTestId("imports-create-warnings")).toContainText("no currency symbol");

    const currency = page.getByTestId("imports-create-currency");
    await expect(currency).toHaveValue("");
    await currency.fill("$");
    await currency.blur();

    // The one thing no CSV can supply. Until it is there, Create is refused
    // with the reason beside it rather than a disabled button and no comment.
    await expect(page.getByTestId("imports-create-save")).toBeDisabled();
    await expect(page.getByTestId("imports-create-blocker")).toContainText(/which account/i);
    // `AccountInput` carries a fixed `aria-label="Account"` for both accounts,
    // so the PLACEHOLDER is what tells account1 from account2 here.
    const account1 = page.getByPlaceholder("assets:bank:checking");
    await account1.fill("assets:bank:checking");
    await account1.blur();

    // What the file will say, shown BEFORE it is written.
    await expect(page.getByTestId("imports-create-lines")).toContainText("fields date, amount");
    await expect(page.getByTestId("imports-create-lines")).toContainText("currency $");
    await expect(page.getByTestId("imports-create-lines")).toContainText("account1 assets:bank:checking");
    await expect(page.getByTestId("imports-create-lines")).toContainText("account2 expenses:unknown");

    await expect(page.getByTestId("imports-create-save")).toBeEnabled();
    await page.getByTestId("imports-create-save").click();

    // The bytes on disk are the engine's renderer's, and carry no comment line
    // — `ItemBody` has no comment variant, so a draft that had one could not be
    // saved through the create wire at all.
    await expect(async () => {
        expect(readFileSync(CREATED_RULES, "utf8")).toBe(
            "skip 1\n" +
                "date-format %Y-%m-%d\n" +
                "fields date, amount\n" +
                "account1 assets:bank:checking\n" +
                "account2 expenses:unknown\n" +
                // LAST, not beside the other directives: `withSetting` inserts
                // after the last setting already in the document, and a draft's
                // last one is `account2`. Only `skip` is positional to hledger,
                // so this is harmless — and it is pinned by
                // `createModel.test.ts` so this expectation is a measurement
                // rather than a guess.
                "currency $\n"
        );
    }).toPass();

    // And it is readable back through the editor, which knows nothing about any
    // of this: one round trip, two independent routes.
    await page.goto("/imports?tab=rules");
    await page.getByRole("button", {name: /^dropped/}).click();
    await expect(page.getByTestId("imports-open-file")).toHaveText("dropped");
    await expect(page.getByPlaceholder("assets:bank:checking")).toHaveValue("assets:bank:checking");

    rmSync(CREATED_RULES, {force: true});
});

test("refuses to create a rules file over one that already exists", async ({page}) => {
    // Creating and editing stay separate operations, and the check that MATTERS
    // is the write's: the draft route's own 409 is a courtesy that expires the
    // moment it returns, so this drives the name the user actually types and
    // presses Create.
    await page.goto("/imports");
    const drop = page.getByTestId("imports-drop-target");
    if ((await drop.count()) === 0) test.skip(true, "this engine cannot run hledger");

    await page.locator('input[type="file"]').setInputFiles({
        name: "dropped.csv",
        mimeType: "text/csv",
        buffer: Buffer.from(ALIEN_CSV),
    });
    await expect(page.getByTestId("imports-no-candidates")).toBeVisible();
    await page.getByTestId("imports-create-rules").click();
    await expect(page.getByTestId("imports-create-rules-panel")).toBeVisible();

    // `scratch/imports-e2e/scratch.csv.rules` is written fresh by `beforeEach`,
    // so this id is taken. The draft above succeeded under a different name —
    // the panel does not re-draft on a rename, by design, because a re-draft
    // would discard the corrections the user came here to make.
    await page.getByTestId("imports-create-id").fill("scratch/imports-e2e/scratch.csv.rules");
    const account1 = page.getByPlaceholder("assets:bank:checking");
    await account1.fill("assets:bank:checking");
    await account1.blur();

    await expect(page.getByTestId("imports-create-save")).toBeEnabled();
    await page.getByTestId("imports-create-save").click();
    await expect(page.getByTestId("imports-create-save-error")).toContainText(/already exists/i);

    // And the existing file is untouched — the whole point of the refusal.
    expect(readFileSync(SCRATCH_RULES, "utf8")).toBe(RULES);
});
