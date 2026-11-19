// docs/journal/accrual-recognition.md and docs/journal/invoice-matching.md —
// the Recognition button and overlay, a month's group row, the editor's
// Accrual section, the Invoices page with a card's Details open, and
// Settings → Books. Engine: the staged copy of fixtures/accrual/, read as of
// 2026-11-17 (its README lists the cases).
//
// Fixture facts at that date (crates/ledgeline-server/tests/accrual_endpoints.rs
// pins them): 10 entries due in November, 1 due in a closed period (Alpine Ski
// House, September; closed through 2026-09-30), 1 deferral that needs a
// schedule (Fourth Coffee Invoice 2004), and 3 invoice proposals: Tailspin
// 2001, $30 short (a fee, to check); Wingtip 2002, exact and clear; and the
// Gusto payroll bill, paid 3 days before it, exact and clear.
// Posted entries run through October, so October's group row exists.
//
// Nothing here saves: the overlay and the editor are opened and closed, the
// schedule form is filled but never saved, and Details only reads. Even a
// stray save would land in target/docs-screenshots/accrual/, never in fixtures/.
import {around, expect, finishMotion, shot, test} from "./harness";

const SECTION = "journal";

test.use({journal: "accrual", now: new Date(2026, 10, 17, 12, 0, 0), settings: {insightsOpen: false}});

test.describe("journal", () => {
    test.beforeEach(async ({page}) => {
        await page.goto("/");
        await expect(page.locator("tbody tr[data-txn], tbody tr[data-testid='recognition-group-row']").first()).toBeVisible();
        await expect(page.getByTestId("recognition-button")).toBeVisible();
    });

    test("recognition button", async ({page}) => {
        await shot(page, SECTION, "recognition-button", {
            clip: await around([page.getByTestId("find-matches-button"), page.getByTestId("recognition-toolbar")]),
            padding: 8,
        });
    });

    test("due tab", async ({page}) => {
        await page.setViewportSize({width: 1280, height: 1000});
        await page.keyboard.press("a");
        const overlay = page.getByTestId("recognition-overlay");
        await expect(overlay).toBeVisible();
        await expect(overlay.getByTestId("due-month").first()).toContainText("Nov 2026");
        await expect(overlay.getByTestId("due-tie").first()).toBeVisible();
        await finishMotion(page);
        await shot(page, SECTION, "recognition-due");
    });
});

test("group row", async ({page}) => {
    await page.goto("/?from=2026-10-01&to=2026-10-31");
    const group = page.getByTestId("recognition-group-row").first();
    await expect(group).toContainText("Recognition · Oct 2026");
    // October collapses to a handful of rows: clip to the first and last.
    const rows = page.locator("tbody tr");
    await shot(page, SECTION, "recognition-group-row", {clip: [rows.first(), rows.last()], padding: 8});
});

test("accrual section", async ({page}) => {
    await page.setViewportSize({width: 1280, height: 1400});
    await page.goto("/");
    // `a` is bound once the accrual status has loaded: wait for the badge.
    await expect(page.getByTestId("recognition-count")).toBeVisible();
    await page.keyboard.press("a");
    const overlay = page.getByTestId("recognition-overlay");
    await overlay.getByTestId("recognition-tab-needs").click();
    await expect(overlay.getByTestId("needs-item")).toContainText("Fourth Coffee Invoice 2004");
    await overlay.getByTestId("needs-edit").click();
    const section = page.getByTestId("accrual-section");
    await expect(section).toBeVisible();
    await section.getByTestId("accrual-recognize").first().click();
    const form = section.getByTestId("accrual-schedule-form");
    await form.getByTestId("accrual-term-12").click();
    await form.getByTestId("accrual-target").fill("revenue:subscriptions");
    await expect(section.getByTestId("accrual-schedule-preview").getByTestId("accrual-period")).toHaveCount(12);
    await finishMotion(page);
    await shot(page, SECTION, "accrual-section", {clip: section, padding: 12});
});

test.describe("invoices", () => {
    test.beforeEach(async ({page}) => {
        await page.setViewportSize({width: 1280, height: 1100});
        await page.goto("/settlements");
        await expect(page.getByTestId("settlement-card")).toHaveCount(3);
    });

    test("cards", async ({page}) => {
        await shot(page, SECTION, "invoice-matching", {clip: page.getByTestId("settlements-panel"), padding: 12});
    });

    test("details", async ({page}) => {
        const card = page.getByTestId("settlement-card").filter({hasText: "2001"});
        await card.getByTestId("details-toggle").first().click();
        await expect(card.getByTestId("detail-postings")).toHaveCount(2);
        await expect(card.locator("[data-highlighted]")).toHaveCount(2);
        await finishMotion(page);
        await shot(page, SECTION, "invoice-details", {clip: card, padding: 12});
    });
});

test("settings books", async ({page}) => {
    await page.goto("/settings?tab=books");
    await expect(page.getByTestId("settings-books")).toBeVisible();
    await expect(page.getByTestId("books-closed-through")).toContainText("2026-09-30");
    await shot(page, "settings", "books", {clip: page.getByTestId("settings-books"), padding: 12});
});
