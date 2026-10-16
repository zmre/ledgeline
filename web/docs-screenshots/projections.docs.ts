// docs/projections/ — the projection table seeded from the journal, and the
// asset-growth scenario (fixtures/asset-growth-scenario.journal, staged beside
// the sample journal) for asset rows. Nothing is saved.
import {annotate, expect, shot, test} from "./harness";

const SECTION = "projections";

test("seeded table", async ({page}) => {
    await page.goto("/projections");
    await expect(page.getByTestId("projection-net")).toBeVisible();
    await shot(page, SECTION, "seeded");
});

test.describe("asset growth scenario", () => {
    test.beforeEach(async ({page}) => {
        await page.setViewportSize({width: 1280, height: 1400});
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
        // projected table and chart below are the same as in "table".
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
