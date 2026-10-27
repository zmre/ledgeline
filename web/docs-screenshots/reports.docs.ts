// docs/reports/ — Insights, Balance Sheet, Subscriptions, P&L, Cash Flow.
import type {Page} from "@playwright/test";
import {annotate, around, expect, shot, test} from "./harness";

const SECTION = "reports";

test("insights", async ({page}) => {
    await page.goto("/reports");
    await expect(page.getByTestId("insights-dashboard")).toBeVisible();
    await expect(page.getByTestId("insights-box-networth")).toContainText("$");
    await shot(page, SECTION, "insights");
});

// The tab strip and the controls bar, Export circled. On the P&L because it
// is the tab with the most controls that every report shares.
test("tabs and export", async ({page}) => {
    await page.goto("/reports?tab=is");
    await expect(page.getByTestId("income-statement")).toBeVisible();
    const exportButton = page.getByRole("button", {name: "Export report as xlsx"});
    const controls = page.getByLabel("Interval").locator("xpath=ancestor::div[contains(@class,'rounded-box')][1]");
    const drawn = await annotate(page, [{kind: "circle", target: exportButton}]);
    await shot(page, SECTION, "tabs", {clip: [page.getByRole("tablist", {name: "Report"}), controls, drawn], padding: 8});
});

test("balance sheet", async ({page}) => {
    await page.goto("/reports?tab=bs");
    await expect(page.getByTestId("bs-section-assets")).toBeVisible();
    await shot(page, SECTION, "balance-sheet");
});

// The multi-period mode: the controls bar and the Assets box, one column per
// quarter ending at As of.
test("balance sheet, quarterly", async ({page}) => {
    await page.goto("/reports?tab=bs&asof=2026-07-08&from=2025-08-01&interval=quarterly");
    const assets = page.getByTestId("bs-section-assets");
    await expect(assets).toBeVisible();
    await expect(assets).toContainText("Q3 2025");
    const controls = page.getByLabel("Interval").locator("xpath=ancestor::div[contains(@class,'rounded-box')][1]");
    await shot(page, SECTION, "balance-sheet-quarterly", {clip: [controls, assets], padding: 8});
});

test.describe("subscriptions", () => {
    test.use({journal: "subscriptions"});

    test("subscriptions", async ({page}) => {
        await page.goto("/reports?tab=subs");
        const panel = page.getByTestId("subscriptions-panel");
        await expect(panel).toBeVisible();
        await expect(page.getByTestId("subs-box-monthly-total")).toContainText("/mo");
        await shot(page, SECTION, "subscriptions");
    });
});

// The labelled control, not just the <select>: a circle around the select
// alone cuts through its "Interval" label.
const intervalControl = (page: Page) => around([page.getByText("Interval", {exact: true}), page.getByLabel("Interval")]);

test.describe("income statement", () => {
    // The Sankey panels are collapsed: open, "Money in" fills the frame between the
    // controls and the figures the shots are about.
    test.use({settings: {flowsInOpen: false, flowsOutOpen: false}});

    // The P&L in its default Total mode (calendar 2026 against 2025): the
    // Interval control circled, the Prior and % change columns boxed.
    test("income statement", async ({page}) => {
        await page.goto("/reports?tab=is");
        const revenue = page.getByTestId("is-section-revenue");
        await expect(revenue.getByRole("columnheader", {name: "% change"})).toBeVisible();
        const drawn = await annotate(page, [
            {kind: "circle", target: await intervalControl(page)},
            {kind: "box", target: await around([revenue.getByRole("columnheader", {name: "Prior"}), revenue.locator("tfoot td[data-column='pct']")])},
        ]);
        await shot(page, SECTION, "income-statement", {clip: [page.getByRole("tablist", {name: "Report"}), revenue, drawn], padding: 8});
    });

    // Monthly buckets for the first half of 2026, the Total column boxed. A
    // six-month range keeps every column on screen at 1280px.
    test("income statement, monthly", async ({page}) => {
        await page.goto("/reports?tab=is&from=2026-01-01&to=2026-06-30&interval=monthly");
        const revenue = page.getByTestId("is-section-revenue");
        const total = revenue.getByRole("columnheader", {name: "Total", exact: true});
        await expect(total).toBeVisible();
        const drawn = await annotate(page, [
            {kind: "circle", target: await intervalControl(page)},
            {kind: "box", target: await around([total, revenue.locator("tfoot td[data-column='total']")])},
        ]);
        await shot(page, SECTION, "income-statement-monthly", {clip: [page.getByRole("tablist", {name: "Report"}), revenue, drawn], padding: 8});
    });
});

// The statement of cash flows over the first half of 2026 (plans/27): the
// From/To range boxed, Interval circled, and the Total column boxed from its
// heading to Net cash from operating activities. Only the Operating section is
// in frame: the whole statement is taller than the viewport, and measuring it
// would scroll the page out from under the fixed annotation overlay.
test.describe("cash flow", () => {
    // The chart collapsed so the statement sits right under the controls.
    test.use({settings: {cashFlowChartOpen: false}});

    test("cash flow", async ({page}) => {
        await page.goto("/reports?tab=cf&from=2026-01-01&to=2026-06-30&interval=monthly");
        const operating = page.getByTestId("cf-section-operating");
        await expect(operating).toBeVisible();
        const totalHead = operating.locator("thead th").last();
        await expect(totalHead).toHaveText("Total");
        const totalFoot = operating.locator("tfoot td").last();
        const drawn = await annotate(page, [
            {kind: "box", target: await around([page.getByText("From", {exact: true}), page.getByLabel("To", {exact: true})])},
            {kind: "circle", target: await intervalControl(page)},
            {kind: "box", target: await around([totalHead, totalFoot])},
        ]);
        await shot(page, SECTION, "cash-flow", {clip: [page.getByRole("tablist", {name: "Report"}), operating, drawn], padding: 8});
    });
});
