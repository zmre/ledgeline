// docs/holdings/ — the Stocks tab (and its price fetch) and the Other tab.
// The Update prices button is pointed at, never pressed: it calls Yahoo.
import {annotate, expect, shot, test} from "./harness";

const SECTION = "holdings";

test("stocks", async ({page}) => {
    await page.goto("/holdings");
    await expect(page.getByTestId("holdings-table")).toBeVisible();
    await shot(page, SECTION, "stocks");
});

test("stocks table", async ({page}) => {
    await page.setViewportSize({width: 1280, height: 1600});
    await page.goto("/holdings");
    const table = page.getByTestId("holdings-table");
    await expect(table).toBeVisible();
    await shot(page, SECTION, "stocks-table", {clip: table, padding: 8});
});

test("update prices button", async ({page}) => {
    await page.goto("/holdings");
    await expect(page.getByTestId("holdings-table")).toBeVisible();
    const button = page.getByRole("button", {name: "Update prices"});
    const drawn = await annotate(page, [{kind: "arrow", target: button, from: {dx: -300, dy: 0}, label: "Fetch prices"}]);
    await shot(page, SECTION, "update-prices", {clip: [button, drawn], padding: 10});
});

test("other holdings", async ({page}) => {
    await page.goto("/holdings");
    await page.getByRole("tab", {name: "Other"}).click();
    await expect(page.getByTestId("other-holdings-table")).toBeVisible();
    await shot(page, SECTION, "other");
});
