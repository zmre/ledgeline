// docs/journal/review.md — the review buttons and row stripes, the card deck,
// the settings popover and the editor's badge. Engine: the staged copy of
// fixtures/review/, read as of its TODAY, 2026-10-30.
//
// Fixture facts at that date (web/e2e/review.e2e.ts reads the same files):
// Review Unknowns has 8 cards, newest first #25 Venmo, #23 NFLX, #22 SQ *BLUE
// BOTTLE 4412 OAKLAND (a strong coffee suggestion with the rename "Blue Bottle
// Coffee", 7 similar), #21, #20, …; Review Recents has 17.
//
// Nothing here saves. The card shots edit a card and check boxes, and leave it
// unsaved; Skip (`n`) writes nothing. Even a stray save would land in
// target/docs-screenshots/review/, never in fixtures/.
import type {Page} from "@playwright/test";
import {annotate, around, expect, finishMotion, shot, test, type Rect} from "./harness";

const SECTION = "journal";

test.use({journal: "review", now: new Date(2026, 9, 30, 12, 0, 0), settings: {insightsOpen: false}});

/** A card by its transaction index. During an exit the outgoing card is in the DOM too. */
const cardOf = (page: Page, index: number) => page.locator(`[data-card-index="${index}"]`);
const position = (page: Page) => page.getByTestId("review-position");

test.beforeEach(async ({page}) => {
    await page.goto("/");
    await expect(page.locator("tbody tr[data-txn]").first()).toBeVisible();
    await expect(page.getByTestId("review-unknowns-count")).toHaveText("8");
    await expect(page.getByTestId("review-recents-count")).toHaveText("17");
});

test("review buttons and row stripes", async ({page}) => {
    // Newest first: #26 Safeway (unreviewed: the faint stripe), #25 Venmo
    // (unknown: amber), #24 Safeway (reviewed:yes: none), then unknowns.
    await expect(page.locator("tr[data-txn='26']")).toHaveAttribute("data-review-marker", "unreviewed");
    await expect(page.locator("tr[data-txn='25']")).toHaveAttribute("data-review-marker", "unknown");
    await expect(page.locator("tr[data-txn='24']")).not.toHaveAttribute("data-review-marker");
    const add = page.getByRole("button", {name: "Add transaction"});
    const settingsToggle = page.locator("summary[aria-label='Review settings']");
    const rows = page.locator("tbody tr[data-txn]");
    const drawn = await annotate(page, [{kind: "box", target: await around([page.getByTestId("review-unknowns-button"), settingsToggle]), pad: 5}]);
    await shot(page, SECTION, "review-buttons", {clip: [add, drawn, rows.nth(0), rows.nth(5)], padding: 8});
});

test.describe("the card", () => {
    // Tall enough that the card, with Similar open, never scrolls inside itself.
    test.beforeEach(async ({page}) => {
        await page.setViewportSize({width: 1280, height: 1300});
        // `u` opens Review Unknowns; two Skips bring up the Blue Bottle card.
        await page.keyboard.press("u");
        await expect(page.getByTestId("review-overlay")).toBeVisible();
        await expect(position(page)).toHaveText("1 of 8");
        await page.keyboard.press("n");
        await expect(position(page)).toHaveText("2 of 8");
        await page.keyboard.press("n");
        await expect(position(page)).toHaveText("3 of 8");
        const card = cardOf(page, 22);
        await expect(card).toBeFocused();
        await expect(card.getByTestId("review-payee")).toContainText("SQ *BLUE BOTTLE 4412 OAKLAND");
        await expect(card.getByTestId("review-similar-count")).toHaveText("7");
        await finishMotion(page);
        await expect(page.getByTestId("review-peek")).toHaveCount(2);
    });

    /**
     * The top bar and the deck. The peeks sit lower than the card by a CSS
     * transform (14px and 28px), which the deck's own box does not include.
     */
    const deckClip = async (page: Page): Promise<Rect> => {
        const box = await around([position(page), page.getByTestId("review-stack")]);
        return {...box, height: box.height + 34};
    };

    test("clean unknown card", async ({page}) => {
        const card = cardOf(page, 22);
        // The strong chip is pre-selected; Reviewed waits for a category.
        await expect(card.getByTestId("review-chip").first()).toHaveAttribute("aria-pressed", "true");
        await expect(card.getByTestId("review-footer")).toHaveAttribute("data-state", "clean-unknown");
        await shot(page, SECTION, "review-card", {clip: await deckClip(page), padding: 12});
    });

    test("dirty card with similar transactions checked", async ({page}) => {
        const card = cardOf(page, 22);
        // Tab in the account takes the chip: the category and its rename.
        await page.keyboard.press("a");
        const account = card.getByLabel("Account for posting 1");
        await expect(account).toBeFocused();
        await page.keyboard.press("Tab");
        await expect(account).toHaveValue("expenses:food:coffee");
        await expect(card.getByLabel("Description")).toHaveValue("Blue Bottle Coffee");
        await page.keyboard.press("Escape");
        await expect(card).toBeFocused();
        await expect(card.getByTestId("review-footer")).toHaveAttribute("data-state", "dirty");

        // Open Similar and check the first two rows.
        await page.keyboard.press("s");
        await expect(card.getByTestId("review-similar")).toHaveAttribute("data-open", "");
        const similarRows = card.getByTestId("review-similar-row");
        await similarRows.nth(0).getByRole("checkbox").check();
        await similarRows.nth(1).getByRole("checkbox").check();
        await expect(card.getByTestId("review-similar-row").locator("input:checked")).toHaveCount(2);

        // No focus ring on the last checkbox, no hover on its row.
        await card.evaluate((element) => (element as HTMLElement).focus());
        await page.mouse.move(2, 2);
        await finishMotion(page);
        await shot(page, SECTION, "review-card-dirty", {clip: await deckClip(page), padding: 12});
    });
});

test("review settings popover", async ({page}) => {
    const toggle = page.locator("summary[aria-label='Review settings']");
    await toggle.click();
    const popover = page.getByTestId("review-settings");
    await expect(popover).toBeVisible();
    await page.mouse.move(2, 2);
    await shot(page, SECTION, "review-settings", {clip: [page.getByTestId("review-recents-button"), toggle, popover], padding: 14});
});

test("editor: Mark reviewed", async ({page}) => {
    // #17, "Blue Bottle Coffee", carries reviewed:2026-09-02: the editor's
    // checkbox starts checked, and leaving it so keeps that date (D32).
    const row = page.locator("tr[data-txn='17']");
    await row.hover();
    await row.getByRole("button", {name: "Edit whole transaction"}).click();
    const dialog = page.getByRole("dialog", {name: "Edit transaction"});
    const card = dialog.getByTestId("review-card");
    await expect(card).toBeFocused();
    const box = dialog.getByTestId("review-mark-reviewed");
    await expect(box).toBeChecked();
    await page.mouse.move(2, 2);
    await finishMotion(page);
    const footer = card.getByTestId("review-footer");
    const drawn = await annotate(page, [{kind: "circle", target: box.locator("xpath=ancestor::label[1]"), pad: 4}]);
    await shot(page, SECTION, "transaction-editor-reviewed", {clip: [footer, drawn], padding: 8});
});
