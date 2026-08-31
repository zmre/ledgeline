// The Budget tab: its own top-level route since the editor landed, serving the
// bars and the goals from one page.
//
// # Why nothing here writes
//
// The e2e engine is launched over `fixtures/sample.journal` with editing
// enabled, and a budget goal lives IN that journal — there is no scratch file to
// redirect a save into, the way `imports.e2e.ts` redirects a rules-file save.
// A spec that added a goal would rewrite a committed fixture that five other
// specs assert exact numbers from, leave the working tree dirty, and stop being
// idempotent the second time it ran.
//
// So the write path has no e2e, deliberately, and is covered instead by
// `crates/ledgeline-server/tests/budget_endpoints.rs` — twenty-three tests over
// a temp journal, asserting the written BYTES, which is a stronger check than
// clicking a button and reading a bar. What this spec is for is the half that
// only exists in the browser: that the tab is reachable, that the old URL still
// works, and that the empty state offers the right next step.
import {expect, test, type Page} from "@playwright/test";
import {API_TOKEN} from "../playwright.config";

const API_URL = "http://127.0.0.1:5099";
const FIXED_NOW = new Date(2026, 6, 8, 12, 0, 0); // local 2026-07-08

// The settings store keeps ONE key holding both halves. Seeding two separate
// keys leaves the app with no server, and the first-run "Connect to hledger-web"
// modal then overlays the page and swallows every click.
test.beforeEach(async ({page}) => {
    await page.clock.setFixedTime(FIXED_NOW); // so "This year" is a fixed year
    await page.addInitScript(
        ([url, token]) => {
            localStorage.setItem("ledgeline.settings.v1", JSON.stringify({serverUrl: url, serverToken: token}));
        },
        [API_URL, API_TOKEN]
    );
});

// `exact`, because Playwright's label matching is substring-and-case-insensitive
// by default: a bare "To" also matches "Connect to hledger-web" and "Access
// token (Ledgeline)". Pinning it keeps the locator honest even when something
// else is on screen.
const dateField = (page: Page, label: "From" | "To") => page.getByLabel(label, {exact: true});

test("budget: is a top-level tab, not a report tab", async ({page}) => {
    await page.goto("/");
    await page.getByRole("link", {name: "Budget"}).click();
    await expect(page).toHaveTitle("Ledgeline — Budget");
    await expect(page.getByRole("heading", {name: "Budget", exact: true})).toBeVisible();

    // And it is gone from the Reports strip, which is the other half of the move.
    await page.getByRole("link", {name: "Reports"}).click();
    await expect(page.getByRole("tab", {name: "Balance Sheet"})).toBeVisible();
    await expect(page.getByRole("tab", {name: "Budget"})).toHaveCount(0);
});

test("budget: a bookmarked ?tab=budget URL is forwarded, keeping its range", async ({page}) => {
    // Budget was a report tab for several releases. A link somebody saved then
    // must not quietly land on Insights, which is where an unknown `tab` value
    // otherwise falls back to.
    await page.goto("/reports?tab=budget&from=2026-01-01&to=2026-06-30&depth=2");
    await expect(page).toHaveTitle("Ledgeline — Budget");
    await expect(page).toHaveURL(/\/budget\?/);
    await expect(dateField(page, "From")).toHaveValue("2026-01-01");
    await expect(dateField(page, "To")).toHaveValue("2026-06-30");
});

test("budget: the period presets drive the range and the URL", async ({page}) => {
    await page.goto("/budget");
    await page.getByRole("button", {name: "This year"}).click();
    await expect(dateField(page, "From")).toHaveValue(/^\d{4}-01-01$/);
    await expect(dateField(page, "To")).toHaveValue(/^\d{4}-12-31$/);
    await expect(page).toHaveURL(/from=\d{4}-01-01&to=\d{4}-12-31/);
});

test("budget: a journal with no goals offers to start one, and says exactly what it will do", async ({page}) => {
    // `fixtures/sample.journal` declares no `~` rules, so this is the state every
    // new user opens the tab in — and the one where getting the wording wrong
    // costs the most, because the button writes two files.
    await page.goto("/budget");

    const empty = page.getByTestId("no-goals");
    await expect(empty).toBeVisible();
    await expect(empty).toContainText("No budget goals yet");

    const create = page.getByTestId("create-budget-file");
    await expect(create).toBeVisible();
    await expect(create).toContainText("budget.journal");
    // The offer names both effects — the new file AND the include line — before
    // it is taken. Deliberately not clicked: see the header.
    await expect(empty).toContainText("include");

    // The bars render alongside, and say the same thing in their own words.
    await expect(page.getByTestId("budget-empty")).toBeVisible();
});
