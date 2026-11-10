// docs/journal/transfer-matching.md — the Find Matches card and a linked
// transfer row, collapsed and expanded. Engine: the staged copy of
// fixtures/matching.journal (and its four includes), read as of 2027-01-31.
//
// Fixture facts at that date (crates/ledgeline-server/tests/match_fixture.rs
// pins them): Find Matches proposes 8 pairs and one bill-pay combination (9
// proposals: 6 clear, 3 to check), served in date order, so the year-end
// Amex payment (#16, Dec 31 → #33, Jan 2, labelled clear, 2 days apart) is
// the 7th card and the combination (Jan 12) the 9th. Two pairs are
// linked: savings (#3/#19, Oct 5–6) and Amex (#4 → #28, Oct 20–22, match
// k7f3a2), each one row anchored on its checking half.
//
// Nothing here saves. The card shot only skips (`n`), which writes nothing;
// expanding a row is view state. Even a stray save would land in
// target/docs-screenshots/matching/, never in fixtures/.
import type {Page} from "@playwright/test";
import {around, expect, finishMotion, shot, test, type Rect} from "./harness";

const SECTION = "journal";

test.use({journal: "matching", now: new Date(2027, 0, 31, 12, 0, 0), settings: {insightsOpen: false}});

test.beforeEach(async ({page}) => {
    await page.goto("/");
    await expect(page.locator("tbody tr[data-txn]").first()).toBeVisible();
    await expect(page.getByTestId("find-matches-count")).toHaveText("9");
});

/**
 * The overlay's top bar and the deck. The peeks sit lower than the card by a
 * CSS transform, which the deck's own box does not include.
 */
const deckClip = async (page: Page): Promise<Rect> => {
    const box = await around([page.getByRole("heading", {name: "Find Matches"}), page.getByTestId("match-stack")]);
    return {...box, height: box.height + 34};
};

test("match card", async ({page}) => {
    await page.setViewportSize({width: 1280, height: 1000});
    await page.keyboard.press("m");
    await expect(page.getByTestId("match-overlay")).toBeVisible();
    await expect(page.getByTestId("match-counts")).toContainText("6 clear");
    await page.keyboard.press("Enter");
    await expect(page.getByTestId("match-card")).toBeVisible();
    // Skip (writes nothing) until the year-end Amex payment is on top.
    const left = () => page.locator("[data-testid='match-card'] [data-testid='match-half-left']").last();
    for (let skips = 0; skips < 8 && !((await left().textContent()) ?? "").includes("Dec 31, 2026"); skips += 1) {
        await page.keyboard.press("n");
        await finishMotion(page);
    }
    const card = page.getByTestId("match-card").last();
    await expect(card.getByTestId("match-half-left")).toContainText("Dec 31, 2026");
    await expect(card.getByTestId("match-header")).toContainText("checking");
    await expect(card.getByTestId("match-header")).toContainText("amex");
    await expect(card.getByTestId("match-confidence")).toHaveText("clear");
    await expect(card.getByTestId("match-half-right").getByTestId("match-half-file")).toHaveText("amex.journal");
    await finishMotion(page);
    await shot(page, SECTION, "match-card", {clip: await deckClip(page), padding: 12});
});

test.describe("transfer rows", () => {
    // October 2026: two linked transfers among ordinary rows.
    test.beforeEach(async ({page}) => {
        await page.goto("/?from=2026-10-01&to=2026-10-31");
        await expect(page.locator("tbody tr[data-transfer='k7f3a2']")).toBeVisible();
        await expect(page.locator("tbody tr[data-transfer='a1b2c3']")).toBeVisible();
    });

    test("collapsed", async ({page}) => {
        const rows = page.locator("tbody tr[data-txn]");
        await expect(rows).toHaveCount(5);
        await shot(page, SECTION, "transfer-row", {clip: [rows.first(), rows.last()], padding: 8});
    });

    test("expanded", async ({page}) => {
        const transfer = page.locator("tbody tr[data-transfer='k7f3a2']");
        await transfer.getByTestId("transfer-toggle").click();
        // The halves: checking's #4 and Amex's #28, as ordinary rows under it.
        const amex = page.locator("tbody tr[data-txn='28']");
        await expect(amex).toBeVisible();
        await finishMotion(page);
        await shot(page, SECTION, "transfer-row-expanded", {clip: await around([transfer, amex]), padding: 8});
    });
});
