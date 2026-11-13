// docs/budget.md — a single top-level page, so its images are
// docs/images/budget-*.png. Engine: fixtures/budget/basic.journal, the one
// with monthly goals for food and bus and actuals in Jan/Feb 2026.
import {annotate, expect, shot, test} from "./harness";

const SECTION = "budget";

test.use({journal: "budget"});

test.beforeEach(async ({page}) => {
    await page.goto("/budget");
    await expect(page.getByText("expenses:food", {exact: true}).first()).toBeVisible();
});

test("overview", async ({page}) => {
    await shot(page, SECTION, "overview");
});

test("bars", async ({page}) => {
    // The Expenses table: one row per budgeted account, the Total under them.
    // The tall line on each bar is the goal, the small tick where spending
    // "should" be by today.
    const table = page.getByTestId("budget-table-expense");
    const drawn = await annotate(page, [{kind: "box", target: table.getByTestId("budget-remaining").first(), pad: 4}]);
    await shot(page, SECTION, "bars", {clip: [table, drawn], padding: 10});
});

test("goals", async ({page}) => {
    const heading = page.getByRole("heading", {name: "Budget goals"});
    await heading.scrollIntoViewIfNeeded();
    const section = heading.locator("xpath=ancestor::section[1]");
    await shot(page, SECTION, "goals", {clip: (await section.count()) > 0 ? section : heading, padding: 10});
});
