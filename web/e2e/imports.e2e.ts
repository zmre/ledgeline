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
// run, the settings the three panels own, three editable OR-list rules, and one
// conditional TABLE the engine classifies `opaque` and this GUI must show
// read-only. `%m/%d/%Y` rather than ISO so the date-format example is a
// non-trivial one.
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

/** Open Imports and select the scratch file. */
async function openScratch(page: Page): Promise<void> {
    await page.goto("/imports");
    await page.getByRole("button", {name: /^scratch/}).click();
    await expect(page.getByTestId("imports-open-file")).toHaveText("scratch");
}

/** The matcher summary of each editable rule card, in the order they are listed. */
function ruleSummaries(page: Page) {
    return page.getByTestId("imports-rule");
}

test("navigates to Imports and lists the rules files beside the journal", async ({page}) => {
    await page.goto("/");
    await page.getByRole("link", {name: "Imports"}).click();
    await expect(page).toHaveTitle("Ledgeline — Imports");

    // The scan root is named by LABEL — the engine deliberately never sends a path.
    await expect(page.getByText("the folder your journal is in").first()).toBeVisible();
    await expect(page.getByRole("button", {name: /^scratch/})).toBeVisible();

    // A real ledger has `2025/imports/capitalone.csv.rules` next to
    // `2026/imports/capitalone.csv.rules`, so the row has to say which folder it
    // came from — a list of bare labels shows the same name twice.
    const scratchRow = page.getByRole("button", {name: /^scratch/});
    await expect(scratchRow).toContainText("scratch/imports-e2e");

    // Every assertion below is SCOPED to that row on purpose. Two unrelated
    // files summarizing identically ("4 rules, 1 advanced" also describes the
    // committed tree fixture) is precisely the ambiguity this change exists to
    // fix, so a page-wide matcher would resolve to both and fail.
    //
    // The counts moved off the row into the tooltip, which `data-tip` draws as a
    // pseudo-element — invisible to assistive technology, so `sr-only` text
    // mirrors it into the accessible name. Assert the attribute for what is
    // drawn and the name for what is announced; they are two different
    // mechanisms and either can break alone.
    //
    // Four conditionals, one of which is the table: `ifBlockCount` counts every
    // conditional, `opaqueItemCount` only the ones this GUI will not edit.
    const scratchItem = page.locator("li.tooltip", {has: scratchRow});
    await expect(scratchItem).toHaveAttribute("data-tip", /^scratch\/imports-e2e\/scratch\.csv\.rules · 4 rules, 1 advanced/);
    await expect(scratchRow).toHaveAccessibleName(/scratch\/imports-e2e\/scratch\.csv\.rules · 4 rules, 1 advanced/);
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
    await expect(page.getByRole("button", {name: "Save"})).toBeDisabled();

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

    await expect(ruleSummaries(page)).toHaveCount(3);
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
    await expect(ruleSummaries(page).nth(1).getByLabel("Account").first()).toHaveValue("expenses:food:cafe");
});

test("switching files with unsaved changes asks before discarding them", async ({page}) => {
    await openScratch(page);

    await page.getByRole("button", {name: "Move rule 2 down"}).click();
    await expect(page.getByTestId("imports-dirty")).toBeVisible();

    // The inline two-step confirm, not a `beforeunload` guard: the click that
    // discards the edit is never the click that asked to switch.
    await page.getByRole("button", {name: /^checking/}).click();
    await expect(page.getByText("Discard your unsaved changes?")).toBeVisible();
    await expect(page.getByTestId("imports-open-file")).toHaveText("scratch");

    await page.getByRole("button", {name: "Keep editing"}).click();
    await expect(page.getByTestId("imports-open-file")).toHaveText("scratch");
    await expect(page.getByTestId("imports-dirty")).toBeVisible();

    await page.getByRole("button", {name: /^checking/}).click();
    await page.getByRole("button", {name: "Discard"}).click();
    await expect(page.getByTestId("imports-open-file")).toHaveText("checking");
});
