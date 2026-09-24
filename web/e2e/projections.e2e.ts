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
// Phase 3 adds Save As, and the rule does not change: the write path is proved
// in `crates/ledgeline-core/tests/projection_files.rs` and
// `crates/ledgeline-server/tests/projection_endpoints.rs`, AGAINST THE BYTES,
// which is a far stronger check than clicking a button. What is asserted here
// is the half those cannot reach: that the file controls are on the page, that
// the picker lists what the engine found, and that the Save As dialog shows the
// path it would write BEFORE it writes it. The dialog is opened and cancelled;
// its Save button is never pressed.
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

// UNVERIFIED: written against the markup, never executed — Playwright cannot
// run in the environment this was added in (plan 23, Phase 3).
test("projections: the Assets and balances section is there, empty, on a seeded journal", async ({page}) => {
    await page.goto("/projections");

    // `sample.journal` has no `~` rules at all, so the seed produces flows only
    // — `budget_gaps` measures revenue and expense accounts. The section is
    // still present, with its own empty state and its own Add button, because
    // an asset row is something the user creates deliberately.
    const assets = page.getByTestId("projection-section-asset");
    await expect(assets).toBeVisible();
    await expect(assets).toContainText("A brokerage account, a 401k, a house.");
    await expect(assets.getByRole("button", {name: "+ Add an asset"})).toBeEnabled();
    await expect(page.getByTestId("projection-asset-line")).toHaveCount(0);
});

// UNVERIFIED, as above.
test("projections: adding an asset row shows the ledger's own balance, greyed", async ({page}) => {
    await page.goto("/projections");
    await expect(page.getByTestId("projection-net")).toBeVisible();

    await page.getByRole("button", {name: "+ Add an asset"}).click();
    const row = page.getByTestId("projection-asset-line").first();
    await expect(row).toBeVisible();

    // Seeded into the journal's own asset tree, and a balance box that is EMPTY
    // — the journal's figure is its placeholder, which is what "greyed until
    // you type over it" means.
    await expect(row.getByRole("combobox").first()).toHaveValue("assets:");
    const balance = page.getByLabel(/^Balance for /);
    await expect(balance).toHaveValue("");

    // Naming a real account and letting the debounced recompute land puts the
    // ledger's figure in the placeholder.
    await row.getByRole("combobox").first().fill("assets:broker:taxable");
    await row.getByRole("combobox").first().press("Enter");
    await expect(page.getByLabel("Balance for assets:broker:taxable")).toHaveAttribute("placeholder", /^\d/);

    // Typing over it is visibly an override, and the ledger's figure survives.
    await page.getByLabel("Balance for assets:broker:taxable").fill("640000");
    await page.getByLabel("Balance for assets:broker:taxable").blur();
    await expect(page.getByTestId("ledger-balance")).toContainText("ledger");
    await page.getByLabel("Use the journal's balance for assets:broker:taxable").click();
    await expect(page.getByTestId("ledger-balance")).toHaveCount(0);
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

    await expect(page.getByTestId("projection-dirty")).toBeVisible();
    await expect.poll(() => runs.length).toBeGreaterThan(before);
    // Nothing was written — an edit is an edit, and the "edited" badge stays up
    // until a save clears it. This spec never saves.
    await expect(page.getByTestId("projection-file-select")).toHaveValue("");
});

test("projections: changing the window re-asks the engine and mirrors to the URL", async ({page}) => {
    await page.goto("/projections");
    await expect(page.getByTestId("projection-net")).toBeVisible();

    await page.getByLabel("Interval").selectOption("quarterly");
    await expect(page).toHaveURL(/interval=quarterly/);
    await expect(page.getByTestId("projection-flow-chart")).toBeVisible();
});

// ---------------------------------------------------------------------------
// Scenario files (plan 22, Phase 3) — READ-ONLY
//
// Nothing below writes. The picker is read, the dialog is opened and cancelled,
// and the one assertion that matters — the path shown before the write — is
// made against the dialog's own preview rather than against a file on disk.
// ---------------------------------------------------------------------------

test("projections: the file bar lists the journals beside the fixture", async ({page}) => {
    await page.goto("/projections");
    await expect(page.getByTestId("projection-file-bar")).toBeVisible();

    const picker = page.getByTestId("projection-file-select");
    // A seeded scenario has no file of its own, and the picker says so rather
    // than showing a blank box.
    await expect(picker).toHaveValue("");
    await expect(picker.locator("option", {hasText: "Seeded from your journal"})).toHaveCount(1);
    // The fixture journal itself is a `*.journal` beside itself, so it is
    // listed — "really any journal file with budget-like entries could be used".
    await expect(picker.locator("optgroup")).toHaveCount(1);
    await expect(picker.locator('optgroup[label="Other journals"]')).toHaveCount(1);

    // Save is offered but not possible: there is no file to save OVER yet.
    await expect(page.getByTestId("projection-save")).toBeDisabled();
    await expect(page.getByTestId("projection-save-as")).toBeEnabled();
});

test("projections: the fixture journal is loadable and marked read-only", async ({page}) => {
    // DECISION 5: a file the main journal is parsed from can be READ (the ask
    // wants the active budget file as a default) and can never be saved over.
    await page.goto("/projections");
    await expect(page.getByTestId("projection-file-bar")).toBeVisible();

    await page.getByTestId("projection-file-select").selectOption({label: "sample"});
    await expect(page.getByTestId("projection-read-only")).toContainText("part of your main journal");
    await expect(page.getByTestId("projection-save")).toBeDisabled();
    // The table is still there, still projecting, just from a different source.
    await expect(page.getByTestId("projection-net")).toBeVisible();
});

test("projections: Save as shows the path it would write, and cancels without writing", async ({page}) => {
    await page.goto("/projections");
    await expect(page.getByTestId("projection-file-bar")).toBeVisible();

    await page.getByTestId("projection-save-as").click();
    await expect(page.getByTestId("projection-save-dialog")).toBeVisible();

    // A name with no letters or digits has no file name, and the button says so
    // rather than the server doing it.
    await page.getByTestId("projection-save-name").fill("!!!");
    await expect(page.getByTestId("projection-save-blocker")).toContainText("no letters or digits");
    await expect(page.getByTestId("projection-save-submit")).toBeDisabled();

    // …and a real one previews the exact path, before anything is written.
    await page.getByTestId("projection-save-name").fill("Series A with a hiring ramp");
    await expect(page.getByTestId("projection-save-path")).toContainText("projection-series-a-with-a-hiring-ramp.journal");
    await expect(page.getByTestId("projection-save-submit")).toBeEnabled();

    // Cancelled. NOTHING is written by this suite.
    await page.getByTestId("projection-save-cancel").click();
    await expect(page.getByTestId("projection-save-dialog")).toHaveCount(0);
});
