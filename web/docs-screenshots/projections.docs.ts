// docs/projections/ — the what-if table seeded from the journal, the readings
// below it, and the asset-growth scenario (fixtures/asset-growth-scenario.journal,
// staged beside the sample journal) for asset rows. Nothing is saved.
import type {Page} from "@playwright/test";
import {annotate, around, expect, shot, test} from "./harness";

const SECTION = "projections";

/** The controls bar under the table: Interval, Periods, Actuals, depth. */
const controlsBar = (page: Page) => page.getByLabel("Actual periods before the projection").locator("xpath=ancestor::div[contains(@class,'rounded-box')][1]");
const readingTabs = (page: Page) => page.getByRole("tablist", {name: "Projection report"});

test("seeded table", async ({page}) => {
    await page.goto("/projections");
    await expect(page.getByTestId("projection-cash")).toBeVisible();
    await expect(page.getByTestId("projection-line").first()).toBeVisible();
    await shot(page, SECTION, "seeded");
});

// The counter-account column and the scenario's Default cash account box
// (cash.md, "In the table"): the box boxed, the column circled.
test("counter-account", async ({page}) => {
    await page.goto("/projections");
    const income = page.getByTestId("projection-section-income");
    await expect(income.getByTestId("projection-line").first()).toBeVisible();
    const cashDefault = page.getByTestId("projection-cash-default");
    const header = income.getByRole("columnheader", {name: "Counter-account"});
    const drawn = await annotate(page, [
        {kind: "box", target: cashDefault, pad: 4},
        {kind: "box", target: await around([header, income.getByLabel(/^Counter-account for/).last()])},
    ]);
    await shot(page, SECTION, "counter-account", {clip: [cashDefault, income, drawn], padding: 8});
});

// A row's description tooltip (file-format.md, "Descriptions and order"),
// shown by hovering the speech-bubble icon of the first expense row.
test("description bubble", async ({page}) => {
    await page.goto("/projections");
    const expenses = page.getByTestId("projection-section-expense");
    const row = expenses.getByTestId("projection-line").first();
    await expect(row).toBeVisible();
    await row.getByTestId("note-button").hover();
    const tip = page.getByTestId("note-tooltip");
    await expect(tip).toBeVisible();
    // Account, Amount and Counter-account only, header to third row: the
    // tooltip is the subject, the rest of the row is the same as in "seeded".
    const all = await around([expenses.locator("thead"), expenses.getByTestId("projection-line").nth(2), tip]);
    const right = await around([expenses.getByRole("columnheader", {name: "Counter-account"})]);
    await shot(page, SECTION, "description", {clip: {...all, width: right.x + right.width - all.x}, padding: 4});
});

// One shot per reading, over a short monthly horizon (two actual months, four
// projected) so every column fits at 1280px.
const reading = async (page: Page, tab: string, testid: string, query = "&interval=monthly&count=4&lookback=2") => {
    // Taller than the whole page: `shot` measures each clip target after
    // scrolling it into view, so a page that scrolls mixes coordinates.
    await page.setViewportSize({width: 1280, height: 4000});
    await page.goto(`/projections?tab=${tab}${query}`);
    const panel = page.getByTestId(testid);
    await expect(panel).toBeVisible();
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
    await expect(assets.getByText("Projected").first()).toBeVisible();
    // Assets only: Liabilities and Equity carry the same Actual/Projected band.
    await shot(page, SECTION, "balance-sheet", {clip: [controlsBar(page), readingTabs(page), assets], padding: 8});
});

test("accounts", async ({page}) => {
    const panel = await reading(page, "accounts", "projection-accounts");
    await expect(panel.getByTestId("accounts-table")).toBeVisible();
    const toggles = await around([panel.getByRole("group", {name: "Accounts reading"}), panel.getByText("Show empty accounts")]);
    const drawn = await annotate(page, [{kind: "box", target: toggles}]);
    await shot(page, SECTION, "accounts", {clip: [readingTabs(page), panel, drawn], padding: 8});
});

test("journal", async ({page}) => {
    const panel = await reading(page, "journal", "projection-journal");
    const entries = panel.getByTestId("forecast-entry");
    await expect(entries.first()).toBeVisible();
    // The first few generated transactions; the list runs on for pages.
    await shot(page, SECTION, "journal", {clip: [readingTabs(page), panel.getByTestId("forecast-journal-count"), entries.nth(3)], padding: 8});
});

test.describe("asset growth scenario", () => {
    test.beforeEach(async ({page}) => {
        await page.setViewportSize({width: 1280, height: 1600});
        await page.goto("/projections");
        const picker = page.getByTestId("projection-file-select");
        await expect(page.getByTestId("projection-file-bar")).toBeVisible();
        const value = await picker.locator("option", {hasText: /asset/i}).getAttribute("value");
        if (value === null) throw new Error("asset-growth-scenario.journal is not in the scenario picker");
        await picker.selectOption(value);
        await expect(page.getByTestId("projection-asset-line").first()).toBeVisible();
    });

    test("scenario", async ({page}) => {
        // The editable scenario only, from the picker down to the asset rows; the
        // readings below are the same as for the seeded table.
        await shot(page, SECTION, "asset-scenario", {
            clip: [page.getByTestId("projection-file-bar"), page.getByTestId("projection-section-asset")],
            padding: 10,
        });
    });

    test("asset rows", async ({page}) => {
        const assets = page.getByTestId("projection-section-asset");
        const growth = page.getByTestId("projection-asset-line").first();
        const drawn = await annotate(page, [{kind: "box", target: growth, pad: 1}]);
        await shot(page, SECTION, "asset-rows", {clip: [assets, drawn], padding: 8});
    });
});
