// docs/reports/ — Insights, Balance Sheet, Subscriptions.
//
// P&L and Cash Flow are deliberately NOT captured yet: their Interval / Total
// controls are being reworked (plans/24-report-intervals-and-totals.md), and a
// shot taken now would be stale on arrival. The stubs at the bottom say what
// each one should show; un-skip and fill in once plan 24 lands.
import {annotate, expect, shot, test} from "./harness";

const SECTION = "reports";

test("insights", async ({page}) => {
    await page.goto("/reports");
    await expect(page.getByTestId("insights-dashboard")).toBeVisible();
    await expect(page.getByTestId("insights-box-networth")).toContainText("$");
    await shot(page, SECTION, "insights");
});

test("balance sheet", async ({page}) => {
    await page.goto("/reports?tab=bs");
    await expect(page.getByTestId("bs-section-assets")).toBeVisible();
    await shot(page, SECTION, "balance-sheet");
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

// TODO(plan 24): P&L with the Interval dropdown circled and the % change column visible.
//   name: "income-statement" — circle the Interval <select>, box the "% change" column header.
test.skip("income statement", async ({page}) => {
    await page.goto("/reports?tab=is");
    await expect(page.getByTestId("income-statement")).toBeVisible();
    await annotate(page, [{kind: "circle", target: page.getByLabel("Interval")}]);
    await shot(page, SECTION, "income-statement");
});

// TODO(plan 24): P&L at a monthly interval with the Total column boxed.
//   name: "income-statement-monthly" — Interval = Monthly, box the Total column.
test.skip("income statement, monthly", async ({page}) => {
    await page.goto("/reports?tab=is");
    await page.getByLabel("Interval").selectOption({label: "Monthly"});
    await annotate(page, [{kind: "box", target: page.getByRole("columnheader", {name: "Total"})}]);
    await shot(page, SECTION, "income-statement-monthly");
});

// TODO(plan 24): Cash Flow with From/To and Interval circled and the Total column boxed.
//   name: "cash-flow" — circle From, To and Interval; box the Total column.
test.skip("cash flow", async ({page}) => {
    await page.goto("/reports?tab=cf");
    await annotate(page, [
        {kind: "circle", target: page.getByLabel("From", {exact: true})},
        {kind: "circle", target: page.getByLabel("To", {exact: true})},
        {kind: "circle", target: page.getByLabel("Interval")},
        {kind: "box", target: page.getByRole("columnheader", {name: "Total"})},
    ]);
    await shot(page, SECTION, "cash-flow");
});
