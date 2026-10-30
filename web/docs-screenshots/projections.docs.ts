// docs/projections/ — the scenario list seeded from the journal, the readings
// below it, and the demo scenario (fixtures/docs/scenario-demo.journal, staged
// beside the sample journal) for the list, the editor and asset rows. Nothing
// is saved.
import type {Page} from "@playwright/test";
import {annotate, expect, shot, test} from "./harness";

const SECTION = "projections";

/** The controls bar under the table: Interval, Periods, Actuals, depth. */
const controlsBar = (page: Page) => page.getByLabel("Actual periods before the projection").locator("xpath=ancestor::div[contains(@class,'rounded-box')][1]");
const readingTabs = (page: Page) => page.getByRole("tablist", {name: "Projection report"});

test("seeded table", async ({page}) => {
    await page.goto("/projections");
    await expect(page.getByTestId("projection-cash")).toBeVisible();
    await expect(page.getByTestId("projection-entry").first()).toBeVisible();
    await shot(page, SECTION, "seeded");
});

// The scenario's Default cash account box and a rule's counter-account line
// (cash.md, "In the list"): the box boxed, the first card's greyed default
// counter-account line boxed.
test("counter-account", async ({page}) => {
    await page.goto("/projections");
    const list = page.getByTestId("projection-list");
    const first = list.getByTestId("projection-entry").first();
    await expect(first).toBeVisible();
    const cashDefault = page.getByTestId("projection-cash-default");
    const drawn = await annotate(page, [
        {kind: "box", target: cashDefault, pad: 4},
        {kind: "box", target: first.getByTestId("entry-counter"), pad: 2},
    ]);
    await shot(page, SECTION, "counter-account", {clip: [cashDefault, first, drawn], padding: 8});
});

// A rule's description (file-format.md, "Descriptions and order"): read in
// full on its card's header line, beside the period.
test("description bubble", async ({page}) => {
    await page.goto("/projections");
    const expense = page
        .locator('[data-testid="projection-entry"]')
        .filter({has: page.locator('[data-type-badge="expense"]')})
        .first();
    await expect(expense).toBeVisible();
    await shot(page, SECTION, "description", {clip: [expense], padding: 4});
});

// One shot per reading, over a short monthly horizon (two actual months, four
// projected) so every column fits at 1280px.
// The readings run a projection over the whole sample journal; under a full
// docs run's load the default 30s has timed out (the balance sheet, plan 31 C4).
test.describe.configure({timeout: 90_000});

const reading = async (page: Page, tab: string, testid: string, query = "&interval=monthly&count=4&lookback=2") => {
    // Taller than the whole page: `shot` measures each clip target after
    // scrolling it into view, so a page that scrolls mixes coordinates.
    await page.setViewportSize({width: 1280, height: 4000});
    await page.goto(`/projections?tab=${tab}${query}`);
    const panel = page.getByTestId(testid);
    // A full docs run starts three engines at once; the first projection can be slow.
    await expect(panel).toBeVisible({timeout: 30_000});
    return panel;
};

test("runway", async ({page}) => {
    // The tab's own default horizon (monthly × 24): a runway over four months says little.
    const panel = await reading(page, "cash", "projection-cash", "");
    await expect(page.getByTestId("projection-runway")).toBeVisible();
    await shot(page, SECTION, "runway", {clip: [controlsBar(page), readingTabs(page), panel], padding: 8});
});

test("balance sheet", async ({page}) => {
    const panel = await reading(page, "bs", "projection-bs");
    const assets = panel.getByTestId("bs-section-assets");
    await expect(assets.getByText("Projected").first()).toBeVisible({timeout: 30_000});
    // Assets only: Liabilities and Equity carry the same Actual/Projected band.
    await shot(page, SECTION, "balance-sheet", {clip: [controlsBar(page), readingTabs(page), assets], padding: 8});
});

test("accounts", async ({page}) => {
    const panel = await reading(page, "accounts", "projection-accounts");
    await expect(panel.getByTestId("accounts-table")).toBeVisible();
    const drawn = await annotate(page, [{kind: "box", target: panel.getByText("Show empty accounts")}]);
    await shot(page, SECTION, "accounts", {clip: [readingTabs(page), panel, drawn], padding: 8});
});

test("journal", async ({page}) => {
    const panel = await reading(page, "journal", "projection-journal");
    const entries = panel.getByTestId("forecast-entry");
    await expect(entries.first()).toBeVisible();
    // The first few generated transactions; the list runs on for pages.
    await shot(page, SECTION, "journal", {clip: [readingTabs(page), panel.getByTestId("forecast-journal-count"), entries.nth(3)], padding: 8});
});

// The demo scenario (fixtures/docs/scenario-demo.journal, staged as
// projection-series-a.journal): a file written by hand, with section headings,
// a note, a turned-off alternative, a step change, an asset rule, a one-off,
// an automatic posting and two rules that balance on their own.
test.describe("demo scenario", () => {
    test.beforeEach(async ({page}) => {
        await page.setViewportSize({width: 1280, height: 2800});
        await page.goto("/projections");
        const picker = page.getByTestId("projection-file-select");
        await expect(page.getByTestId("projection-file-bar")).toBeVisible();
        const value = await picker.locator("option", {hasText: /series a/i}).getAttribute("value");
        if (value === null) throw new Error("projection-series-a.journal is not in the scenario picker");
        await picker.selectOption(value);
        await expect(page.locator('[data-testid="projection-entry"][data-disabled="true"]').first()).toBeVisible();
        await expect(page.locator('[data-testid="projection-entry"] [data-testid="entry-balance"]').first()).toContainText("bal");
    });

    const investing = (page: Page) => page.getByRole("article", {name: "~ monthly investing"});

    test("list", async ({page}) => {
        // From the picker to the end of the list; the readings below are the seeded table's.
        await shot(page, SECTION, "list", {clip: [page.getByTestId("projection-file-bar"), page.getByTestId("projection-list")], padding: 10});
    });

    test("editor", async ({page}) => {
        await page.setViewportSize({width: 1280, height: 1500});
        await investing(page).getByTestId("entry-edit").click();
        const dialog = page.getByTestId("entry-dialog");
        await expect(dialog.getByTestId("entry-dialog-writes")).toContainText("Savings plan");
        await shot(page, SECTION, "editor", {clip: [dialog.locator(".modal-box")], padding: 0});
    });

    test("asset rows", async ({page}) => {
        const card = investing(page);
        const drawn = await annotate(page, [{kind: "box", target: card.getByTestId("entry-balance").nth(1), pad: 3}]);
        await shot(page, SECTION, "asset-rows", {clip: [card, drawn], padding: 8});
    });
});
