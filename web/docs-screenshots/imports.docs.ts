// docs/imports/ — New Transactions, the QuickBooks Journal import, and the
// rules editor. Engine: the staged copy of fixtures/sample.journal, whose
// `imports/` folder holds the two rules files stage.ts copies in.
//
// Nothing here commits: dropping a file only stages a preview in the engine's
// session, and no shot presses Import, Write changes, Map accounts or Save.
// Test Import is allowed: it only runs the dry run. Even if one did more, it
// would land in target/docs-screenshots/, never in fixtures/.
import {readFileSync} from "node:fs";
import {fileURLToPath} from "node:url";
import type {Locator, Page} from "@playwright/test";
import {annotate, around, expect, finishMotion, shot, test} from "./harness";

const SECTION = "imports";
const QB_REPORT = fileURLToPath(new URL("../../fixtures/import/qb-journal/report.xlsx", import.meta.url));
/** The statement stage.ts copies beside `imports/checking.csv.rules`: five rows, four categorised by the rules. */
const CHECKING_CSV = new URL("../../fixtures/rules/simple/checking.csv", import.meta.url);

test("new transactions drop zone", async ({page}) => {
    await page.goto("/imports");
    const drop = page.getByTestId("imports-drop-target");
    await expect(drop).toBeVisible();
    await shot(page, SECTION, "new-transactions", {clip: [page.getByRole("tab", {name: "New Transactions"}), drop], padding: 12});
});

test.describe("quickbooks journal import", () => {
    test.beforeEach(async ({page}) => {
        // The preview's sample block and the eleven-row mapping table run far
        // past 800px; a tall viewport keeps every excerpt on screen at once.
        await page.setViewportSize({width: 1280, height: 2400});
        await page.goto("/imports");
        await expect(page.getByTestId("imports-drop-target")).toBeVisible();
        await page.locator('input[type="file"]').setInputFiles(QB_REPORT);
        await expect(page.getByTestId("qb-journal-panel")).toBeVisible();
        await expect(page.getByTestId("qb-unmapped")).toBeVisible();
    });

    test("preview", async ({page}) => {
        // Heading, the export instructions, the count and the first few sample
        // transactions; the rest of the sample is the same shape.
        const section = page.getByRole("region", {name: "QuickBooks Journal export"});
        const box = await around([section]);
        await shot(page, SECTION, "quickbooks-preview", {clip: {...box, height: Math.min(box.height, 330)}, padding: 0});
    });

    test("mapping table", async ({page}) => {
        // The heading and the first three rows, the first row's "Your account"
        // field circled: that is the field the page tells the reader to fill.
        const unmapped = page.getByTestId("qb-unmapped");
        const rows = unmapped.locator(":scope > div.rounded-box");
        const accountInput = (i: number) => rows.nth(i).locator("label").filter({hasText: "Your account"}).locator("input");
        // Filled, not left empty: an empty field shows its placeholder, which
        // reads as if every QuickBooks account maps to the same one.
        const targets = ["assets:bank:checking", "equity:owner", "liabilities:credit card"];
        for (const [i, target] of targets.entries()) {
            await accountInput(i).fill(target);
            await accountInput(i).press("Escape");
        }
        await page.locator("body").click({position: {x: 1, y: 1}});
        const yourAccount = accountInput(0);
        const drawn = await annotate(page, [{kind: "circle", target: yourAccount, pad: 2}]);
        await shot(page, SECTION, "quickbooks-mapping", {clip: [unmapped.getByRole("heading"), rows.nth(2), drawn], padding: 10});
    });

    test("import button waits for the mapping", async ({page}) => {
        const actions = page.getByTestId("qb-actions");
        await expect(page.getByTestId("qb-commit")).toBeDisabled();
        const mapButton = page.getByTestId("qb-map-accounts");
        const drawn = await annotate(page, [{kind: "box", target: await around([actions.locator("button"), actions.getByRole("status")]), pad: 6}]);
        await shot(page, SECTION, "quickbooks-import-button", {clip: [mapButton, actions, drawn], padding: 10});
    });
});

test("rules editor", async ({page}) => {
    await page.goto("/imports?tab=rules");
    await expect(page.getByTestId("imports-open-file")).toHaveText("checking");
    await expect(page.getByTestId("imports-rule").first()).toBeVisible();
    await shot(page, SECTION, "rules-editor");
});

test.describe("review before writing", () => {
    // The dry-run panel runs far past 800px; a tall window keeps the strip, the
    // write button and the overlay's card on screen at once.
    test.beforeEach(async ({page}) => {
        await page.setViewportSize({width: 1280, height: 1600});
        await page.goto("/imports");
        await expect(page.getByTestId("imports-drop-target")).toBeVisible();
        await page.locator('input[type="file"]').setInputFiles({name: "checking.csv", mimeType: "text/csv", buffer: readFileSync(CHECKING_CSV)});
        await expect(page.getByTestId("imports-csv-path")).toHaveValue("imports/checking.csv");
        await expect(page.getByTestId("imports-submit")).toHaveText("Test Import");
        await page.getByTestId("imports-submit").click();
        await expect(page.getByTestId("imports-dry-run").getByRole("heading", {name: "Nothing has been written yet"})).toBeVisible();
        // HARDWARE STORE is the one row no rule categorises.
        await expect(page.getByTestId("imports-review-unknowns-count")).toHaveText("1");
        await expect(page.getByTestId("imports-review-all-count")).toHaveText("5");
    });

    test("the review strip above Write changes", async ({page}) => {
        const strip = page.getByTestId("imports-review");
        const write = page.getByTestId("imports-write-changes");
        await expect(write).toBeEnabled();
        const drawn = await annotate(page, [{kind: "box", target: strip, pad: 4}]);
        await shot(page, SECTION, "review-before-write", {clip: [strip, write, drawn], padding: 12});
    });

    test("remember as a rule", async ({page}) => {
        await page.keyboard.press("u");
        const overlay = page.getByTestId("review-overlay");
        await expect(overlay.getByRole("heading", {name: "Review Unknowns"})).toBeVisible();
        const card = page.locator('[data-card-index="5"]');
        await expect(card).toBeFocused();
        await expect(card.getByTestId("review-payee")).toContainText("HARDWARE STORE");

        // Recategorise posting 2, back to card mode, then ⇧R.
        await card.getByLabel("Account for posting 2").fill("expenses:home:supplies");
        await leaveField(page, card);
        await expect(card.getByTestId("review-rule")).toBeVisible();
        await page.keyboard.press("Shift+R");
        await expect(card.getByTestId("review-rule-toggle")).toBeChecked();
        await expect(card.getByTestId("review-rule-pattern")).toHaveValue("hardware.*store");
        await expect(card.getByTestId("review-rule-count")).toHaveText(/Matches\s+1\s+of\s+5\s+rows in checking\.csv/);
        await card.getByTestId("review-rule-changes-toggle").click();
        await expect(card.getByTestId("review-rule-change")).toContainText("+ expenses:home:supplies");

        await card.evaluate((element) => (element as HTMLElement).focus());
        await page.mouse.move(2, 2);
        await finishMotion(page);
        await shot(page, SECTION, "review-rule", {clip: card, padding: 12});
    });
});

/** Escape closes an open completion list first, then leaves the field (D16): press until the card has focus. */
async function leaveField(page: Page, card: Locator): Promise<void> {
    for (let tries = 0; tries < 3 && !(await card.evaluate((element) => element === document.activeElement)); tries += 1) {
        await page.keyboard.press("Escape");
    }
    await expect(card).toBeFocused();
}
