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
    // The bars only: the Expenses roll-up and one bar per budgeted account. The
    // tick on each bar is where spending "should" be by today.
    const rollup = page.getByText(/^Expenses · Spent/).locator("xpath=ancestor::div[contains(@class,'rounded-box')][1]");
    const lastBar = page.getByText("expenses:food", {exact: true}).first().locator("xpath=ancestor::div[1]/..");
    const drawn = await annotate(page, [{kind: "box", target: page.getByText(/\$\d+ \/ \$\d+/).first(), pad: 4}]);
    await shot(page, SECTION, "bars", {clip: [rollup, lastBar, drawn], padding: 10});
});

test("goals", async ({page}) => {
    const heading = page.getByRole("heading", {name: "Budget goals"});
    await heading.scrollIntoViewIfNeeded();
    const section = heading.locator("xpath=ancestor::section[1]");
    await shot(page, SECTION, "goals", {clip: (await section.count()) > 0 ? section : heading, padding: 10});
});
