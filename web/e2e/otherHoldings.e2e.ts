// E2E for the Holdings → Other sub-tab (plans/14) against the REAL stack: the
// ledgeline binary serving fixtures/sample.journal on :5099
// (playwright.config.ts webServer) with the built SPA in front. Same pinned
// clock + localStorage seeding as holdings.e2e.ts, whose conventions this file
// follows deliberately — the two tabs live one click apart and should be tested
// the same way.
//
// Fixture facts at asOf 2026-07-08, verified against hledger 1.52 and the
// committed engine golden `fixtures/native/v1/holdings-other.json`:
//
//   assets:property:home  "Family home"  1 HOME
//       bought 2024-07-01 as `1 HOME @ $420,000.00`, revalued by
//       `P 2026-06-30 HOME $468,000.00` → value $468,000.00, cost $420,000.00,
//       change $48,000.00, +11.4%.
//       It is tagged `holdings: other` and that tag is load-bearing: it holds a
//       NON-currency commodity, so without the tag the stock engine would claim
//       it. Booking it as its own commodity is the only way a dollar journal
//       makes a house revalue, which is the whole point of the Change column.
//
//   assets:vehicles:car   "Honda CR-V"   (no Holding cell — dollar-booked)
//       opened 2024-07-01 at $28,000.00 into `…:car:cost`, and written down by
//       explicit entries into the sibling `…:car:depreciation` (tagged
//       `valuation: depreciation`): $4,000.00 on 2025-06-30 and $3,500.00 on
//       2026-06-30. The two roll into ONE row at `assets:vehicles:car` → value
//       $20,500.00 against cost $28,000.00, change -$7,500.00, -26.8%. Before the
//       contra-asset split the two moved together and the loss read $0.00. The
//       change is honestly zero — the correct answer, not a missing feature.
//
//   Totals: value $488,500.00, cost $448,000.00, change $40,500.00, +9.0%.
//
// Note on formatting, because it is easy to assert the wrong string: the Change
// column renders through the same money formatter as Value, so a gain reads
// "$48,000.00" (coloured, not signed). Only the PERCENT column carries an
// explicit sign — "+11.4%" — and zero carries none at all, so the car's percent
// is "0.0%" rather than "+0.0%".

import {expect, test, type Page} from "@playwright/test";
import {API_TOKEN} from "../playwright.config";

const API_URL = "http://127.0.0.1:5099";
const FIXED_NOW = new Date(2026, 6, 8, 12, 0, 0); // local 2026-07-08

const HOME = "assets:property:home";
const CAR = "assets:vehicles:car";

test.beforeEach(async ({page}) => {
    await page.clock.setFixedTime(FIXED_NOW); // Date is fake, timers keep running (URL-sync debounce)
    await page.addInitScript(
        ([url, token]) => {
            localStorage.setItem("ledgeline.settings.v1", JSON.stringify({serverUrl: url, serverToken: token}));
        },
        [API_URL, API_TOKEN]
    );
});

/** Open /holdings and switch to the Other tab the way a user does. */
async function openOther(page: Page): Promise<void> {
    await page.goto("/holdings");
    await page.getByRole("tab", {name: "Other"}).click();
    await expect(page.getByTestId("other-holdings-table")).toBeVisible();
}

test("other holdings: the tab strip defaults to Stocks and reports selection through aria-selected", async ({page}) => {
    await page.goto("/holdings");

    const stocks = page.getByRole("tab", {name: "Stocks"});
    const other = page.getByRole("tab", {name: "Other"});
    await expect(stocks).toBeVisible();
    await expect(other).toBeVisible();

    // Stocks is the tab Holdings has always opened on, so a fresh visit must not
    // move anyone: the strip is additive.
    await expect(stocks).toHaveAttribute("aria-selected", "true");
    await expect(other).toHaveAttribute("aria-selected", "false");
    await expect(page.getByTestId("holdings-table")).toBeVisible();

    await other.click();
    await expect(other).toHaveAttribute("aria-selected", "true");
    await expect(stocks).toHaveAttribute("aria-selected", "false");
});

test("other holdings: switching tabs swaps the table, and swaps it back", async ({page}) => {
    await openOther(page);

    // One table at a time — a house counted on both tabs is worse than a house
    // counted nowhere, and the two reports are disjoint by construction.
    await expect(page.getByTestId("holdings-table")).toHaveCount(0);
    await expect(page.getByTestId("other-holding-" + HOME)).toBeVisible();
    await expect(page.getByTestId("other-holding-" + CAR)).toBeVisible();
    // The stock symbols do not leak across.
    await expect(page.getByTestId("holding-AAPL")).toHaveCount(0);

    await page.getByRole("tab", {name: "Stocks"}).click();
    await expect(page.getByTestId("holdings-table")).toBeVisible();
    await expect(page.getByTestId("other-holdings-table")).toHaveCount(0);
    await expect(page.getByTestId("holding-AAPL")).toBeVisible();
});

test("other holdings: rows carry name, value, cost and change from the engine", async ({page}) => {
    await openOther(page);

    const home = page.getByTestId("other-holding-" + HOME);
    await expect(home).toContainText("Family home"); // the `name:` tag, not the last segment
    await expect(home).toContainText(HOME);
    await expect(home).toContainText("$468,000.00"); // value at the 2026-06-30 price
    await expect(home).toContainText("$420,000.00"); // cost, from the `@` annotation
    await expect(home).toContainText("$48,000.00"); // change (unsigned; the percent carries the sign)
    await expect(home).toContainText("+11.4%");

    const car = page.getByTestId("other-holding-" + CAR);
    await expect(car).toContainText("Honda CR-V");
    await expect(car).toContainText("$20,500.00"); // value, net of depreciation
    await expect(car).toContainText("$28,000.00"); // cost, gross of it
    // Accumulated depreciation is a sibling account tagged `valuation:
    // depreciation`, so it counts against value and NOT against cost — which is
    // the whole reason this row can report a loss at all.
    await expect(car).toContainText("$-7,500.00");
    await expect(car).toContainText("-26.8%");
});

test("other holdings: the Holding column shows the unit as written, and is blank for a dollar-booked asset", async ({page}) => {
    await openOther(page);

    // The unit is the evidence that this row can revalue at all.
    await expect(page.getByTestId("held-" + HOME)).toHaveText("1 HOME");
    // The car holds only the base commodity, so printing it would just repeat the
    // Value column immediately to its right.
    await expect(page.getByTestId("held-" + CAR)).toHaveText("");
});

test("other holdings: the totals row is the engine's, not a sum of the visible rows", async ({page}) => {
    await openOther(page);
    const totals = page.getByTestId("other-holdings-totals");

    await expect(totals).toContainText("Total (2 holdings):");
    await expect(totals).toContainText("$488,500.00"); // value
    await expect(totals).toContainText("$448,000.00"); // cost
    // Everything in scope is priced, so nothing is refused and no warning shows.
    await expect(page.getByTestId("other-holdings-warnings")).toHaveCount(0);
});

test("other holdings: the value-over-time trend renders on this tab too", async ({page}) => {
    await openOther(page);

    // Same component and same testid as the Stocks tab: the Other series reuses
    // the stock series' wire shape byte for byte, which is why there is no new
    // chart code. The June revaluation ($445,000 → $468,000) is inside the window.
    await expect(page.getByTestId("holdings-trend")).toBeVisible();
});

test("other holdings: the tab round-trips through the query string", async ({page}) => {
    await page.goto("/holdings");
    // Stocks is the default, so a fresh visit carries no `tab` key at all.
    await expect(page).not.toHaveURL(/tab=/);

    await page.getByRole("tab", {name: "Other"}).click();
    // The URL mirror is debounced (250ms) and the clock is pinned with timers
    // still running, so this resolves on its own.
    await expect(page).toHaveURL(/[?&]tab=other/);

    await page.getByRole("tab", {name: "Stocks"}).click();
    await expect(page).not.toHaveURL(/tab=/);
});

test("other holdings: a direct ?tab=other link opens on the Other tab", async ({page}) => {
    await page.goto("/holdings?tab=other");

    await expect(page.getByRole("tab", {name: "Other"})).toHaveAttribute("aria-selected", "true");
    await expect(page.getByTestId("other-holdings-table")).toBeVisible();
    await expect(page.getByTestId("other-holding-" + HOME)).toBeVisible();
});

test("other holdings: the scope and the tab survive the same link together", async ({page}) => {
    // One writer to this query string: the scope keys and the tab key must both
    // be honoured, and neither may erase the other.
    await page.goto("/holdings?asof=2025-01-01&tab=other");

    await expect(page.getByRole("tab", {name: "Other"})).toHaveAttribute("aria-selected", "true");
    await expect(page.getByLabel("As of date")).toHaveValue("2025-01-01");
    // 2025-01-01: both assets exist (opened 2024-07-01), the home is priced by
    // `P 2024-07-01 HOME $420,000.00` and the car has not yet been written down.
    await expect(page.getByTestId("other-holding-" + HOME)).toContainText("$420,000.00");
    await expect(page.getByTestId("other-holding-" + CAR)).toContainText("$28,000.00");
});

test("other holdings: time travel before the assets existed shows the empty state", async ({page}) => {
    await openOther(page);

    // Both positions open on 2024-07-01, so the day before there is nothing to own.
    await page.getByLabel("As of date").fill("2024-06-30");

    await expect(page.getByTestId("other-holdings-empty")).toBeVisible();
    await expect(page.getByTestId("other-holdings-table")).toHaveCount(0);
    await expect(page.getByTestId("other-holdings-empty")).toContainText("2024-06-30");
});

test("other holdings: the row cursor moves and drills into the journal", async ({page}) => {
    // Reached by URL rather than by clicking the tab: a clicked <button> keeps
    // focus, and Enter on a focused button ALSO activates it. Landing here with
    // focus on the body keeps this test about the keymap and nothing else.
    await page.goto("/holdings?tab=other");
    await expect(page.getByTestId("other-holdings-table")).toBeVisible();

    // The table's own keymap layer, filed under "Holdings" rather than "Journal".
    await page.keyboard.press("j");
    await expect(page.getByTestId("other-holding-" + HOME)).toHaveAttribute("aria-current", "true");
    await page.keyboard.press("j");
    await expect(page.getByTestId("other-holding-" + CAR)).toHaveAttribute("aria-current", "true");

    await page.keyboard.press("Enter");
    await expect(page).toHaveTitle("Ledgeline — Journal");
});

test("other holdings: the digit keys select tabs, and the help sheet files them under Holdings", async ({page}) => {
    await page.goto("/holdings");
    // Wait for the strip to be live before typing at it. `goto` resolves on the
    // load event, but the keymap layer is registered during hydration, and a
    // digit pressed before that lands nowhere and is not retried — the
    // assertion below would then poll a tab that never changes. Every other
    // keyboard test here (and `keys.e2e.ts`'s report-tab twin, via
    // `reportsReady`) waits first for the same reason.
    await expect(page.getByRole("tab", {name: "Stocks"})).toHaveAttribute("aria-selected", "true");

    await page.keyboard.press("2");
    await expect(page.getByRole("tab", {name: "Other"})).toHaveAttribute("aria-selected", "true");
    await page.keyboard.press("1");
    await expect(page.getByRole("tab", {name: "Stocks"})).toHaveAttribute("aria-selected", "true");

    // One feature, one heading: the holdings table's row cursor moved out of
    // "Journal" when the tab strip added a second holdings keyboard surface.
    await page.keyboard.press("?");
    const help = page.getByTestId("key-help");
    await expect(help).toBeVisible();
    await expect(help).toContainText("Holdings");
    await expect(help).toContainText("Next holding");
});
