// docs/imports/ — New Transactions, the QuickBooks Journal import, and the
// rules editor. Engine: the staged copy of fixtures/sample.journal, whose
// `imports/` folder holds the two rules files stage.ts copies in.
//
// Nothing here commits: dropping a file only stages a preview in the engine's
// session, and no shot presses Import, Map accounts or Save. Even if one did,
// it would land in target/docs-screenshots/, never in fixtures/.
import {fileURLToPath} from "node:url";
import {annotate, around, expect, shot, test} from "./harness";

const SECTION = "imports";
const QB_REPORT = fileURLToPath(new URL("../../fixtures/import/qb-journal/report.xlsx", import.meta.url));

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
        const yourAccount = rows.first().locator("label").filter({hasText: "Your account"}).locator("input");
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
