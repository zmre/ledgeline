// E2E for the Projections tab against the REAL stack: the ledgeline engine
// serving fixtures/sample.journal on :5099 (playwright.config.ts webServer) with
// the built SPA in front. Same pinned clock + localStorage seeding as the other
// suites.
//
// # READ-ONLY, deliberately
//
// Nothing here writes a file, for the reason `budget.e2e.ts` gives at length:
// the e2e engine is launched over a COMMITTED fixture that five other specs
// assert exact numbers from, there is no scratch journal to redirect a save
// into, and a spec that wrote one would leave the working tree dirty and stop
// being idempotent the second time it ran.
//
// Phase 2 of `plans/22-projections.md` has no write path at all — the scenario
// lives in the tab — so this costs nothing here. When Phase 3 adds Save As, the
// write path belongs in `crates/ledgeline-server/tests/projection_files.rs`,
// against the bytes, which is a stronger check than clicking a button.
//
// # Fixture facts
//
// `fixtures/sample.journal` declares no `~` periodic rules, so EVERY revenue and
// expense category in it is unbudgeted: the seed is entirely
// `source: "unbudgeted"` rows, and the table should therefore be all "estimated"
// badges. That is the same fact `budget.e2e.ts` relies on for its gaps section.
//
// Note the one thing this spec cannot pin: the engine takes opening balances as
// of ITS today, and `page.clock` only moves the browser's. So nothing here
// asserts a projected figure — only that the tab is reachable, seeds, and
// renders all three readings.

import {expect, test} from "@playwright/test";
import {API_TOKEN} from "../playwright.config";

const API_URL = "http://127.0.0.1:5099";
const FIXED_NOW = new Date(2026, 6, 8, 12, 0, 0); // local 2026-07-08

test.beforeEach(async ({page}) => {
    await page.clock.setFixedTime(FIXED_NOW);
    await page.addInitScript(
        ([url, token]) => {
            localStorage.setItem("ledgeline.settings.v1", JSON.stringify({serverUrl: url, serverToken: token}));
        },
        [API_URL, API_TOKEN]
    );
});

test("projections: is a top-level tab, reachable from the nav", async ({page}) => {
    await page.goto("/");
    await page.getByRole("link", {name: "Projections"}).click();
    await expect(page).toHaveTitle("Ledgeline — Projections");
    await expect(page.getByRole("heading", {name: "Projections", exact: true})).toBeVisible();
});

test("projections: seeds the what-if table from the journal, flagging what it estimated", async ({page}) => {
    await page.goto("/projections");

    // Both recurring sections exist, and the seed put rows in them.
    await expect(page.getByTestId("projection-section-income")).toBeVisible();
    await expect(page.getByTestId("projection-section-expense")).toBeVisible();
    await expect(page.getByTestId("projection-line").first()).toBeVisible();

    // sample.journal has no `~` rules, so every seeded row is an average of the
    // trailing twelve months — and says so, rather than reading as something the
    // user wrote.
    const badge = page.getByTestId("estimated-badge").first();
    await expect(badge).toBeVisible();
    await expect(badge).toHaveAttribute("title", /average over/);

    // Income is a MAGNITUDE in the box, though the wire signs revenue negative.
    await expect(page.getByLabel("Amount for income:salary")).toHaveValue(/^\d/);
});

test("projections: opens on net income over two years, and says so in the URL", async ({page}) => {
    await page.goto("/projections");

    await expect(page.getByRole("tab", {name: "Net income"})).toHaveAttribute("aria-selected", "true");
    await expect(page.getByLabel("Number of periods")).toHaveValue("24");
    await expect(page.getByLabel("Interval")).toHaveValue("monthly");
    await expect(page).toHaveURL(/tab=net&interval=monthly&count=24/);
});

test("projections: all three readings render, and the runway is stated in words", async ({page}) => {
    await page.goto("/projections");

    // Net income is a PeriodReport, so it renders through the ordinary report
    // table, above the inflow/outflow chart.
    await expect(page.getByTestId("projection-net")).toBeVisible();
    await expect(page.getByTestId("projection-flow-chart")).toBeVisible();

    await page.getByRole("tab", {name: "Cash & runway"}).click();
    await expect(page.getByTestId("projection-cash")).toBeVisible();
    // The question the tab exists for, answered above the picture of it.
    await expect(page.getByTestId("projection-runway")).toContainText(/Cash (turns negative in|never turns negative)/);
    await expect(page.getByTestId("projection-cash-chart")).toBeVisible();

    await page.getByRole("tab", {name: "Net worth"}).click();
    await expect(page.getByTestId("projection-worth")).toBeVisible();
    await expect(page.getByTestId("projection-worth")).toContainText("opening");
    await expect(page.getByTestId("projection-worth-chart")).toBeVisible();
});

test("projections: a bookmarked URL reopens the same tab and window", async ({page}) => {
    await page.goto("/projections?tab=worth&interval=yearly&count=5&depth=1");

    await expect(page.getByRole("tab", {name: "Net worth"})).toHaveAttribute("aria-selected", "true");
    await expect(page.getByLabel("Interval")).toHaveValue("yearly");
    await expect(page.getByLabel("Number of periods")).toHaveValue("5");
});

test("projections: editing the table marks it edited and recomputes", async ({page}) => {
    // The whole point of carrying the scenario in the request body: an UNSAVED
    // edit projects. Asserted as a request going out and the page saying the
    // table has been edited — never as a figure, which depends on the engine's
    // own today.
    const runs: string[] = [];
    page.on("request", (request) => {
        if (request.url().includes("/api/projections/run")) runs.push(request.url());
    });

    await page.goto("/projections");
    await expect(page.getByTestId("projection-net")).toBeVisible();
    const before = runs.length;

    await page.getByLabel("Amount for expenses:housing").fill("2500");
    await page.getByLabel("Amount for expenses:housing").blur();

    await expect(page.getByText("edited")).toBeVisible();
    await expect.poll(() => runs.length).toBeGreaterThan(before);
    // Nothing was written: this tab has no save path in Phase 2, and the badge
    // says exactly that.
    await expect(page.getByText("edited")).toHaveAttribute("title", /not available yet/);
});

test("projections: changing the window re-asks the engine and mirrors to the URL", async ({page}) => {
    await page.goto("/projections");
    await expect(page.getByTestId("projection-net")).toBeVisible();

    await page.getByLabel("Interval").selectOption("quarterly");
    await expect(page).toHaveURL(/interval=quarterly/);
    await expect(page.getByTestId("projection-flow-chart")).toBeVisible();
});
