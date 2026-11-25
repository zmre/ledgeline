// The receivables dashboard end to end (invoicer PLAN 5a): the app opens on
// it, its figures are the accrual fixture's, every figure opens the invoice
// list filtered by the documented query parameters (R5), it works from the
// keyboard, and it fits a narrow window.
//
// Against `--server` on :5199 over the shared scratch copy of
// invoicer/fixtures/accrual (playwright.config.ts). It only reads, and runs
// first (its own project, before `customers`): invoices.e2e.ts sends 108 and
// editor.e2e.ts adds 2005, which would change these figures. Fixture facts at
// the clock, 2026-11-17 (the API golden is the same books on 2026-11-22;
// invoicer-core's tests/dashboard.rs pins these on this date):
//   - $22,850.00 outstanding (7 items): $12,700.00 overdue (5), $10,150.00 not
//     yet due (2); $11,300.00 of it (4) booked in the journal without an
//     invoice file (2001, opening balances);
//   - aging Current $10,150.00 · 1–30 $7,000.00 · 31–60 $1,500.00 · 61–90
//     $1,200.00 · 91+ $3,000.00;
//   - oldest overdue: 2001, Tailspin, due 2026-07-15, 125 days, $3,000.00;
//   - DSO 58.9 days over 90; average days to pay 25.3 over 3 invoices;
//   - one draft (108, $13,600.00); 10 recognition entries due, 1 more closed.
//
// Screenshots for review (and, in Phase 6, the docs) go to
// target/e2e-invoicer-shots/ (gitignored): wide and narrow, dark and light.
//
// Written with 5a and NOT run by the agent that wrote it (agents never run
// Playwright); the lead runs it.

import {resolve} from "node:path";
import {expect, test, type Page} from "@playwright/test";
import {API_TOKEN} from "../playwright.config";

const FIXED_NOW = new Date(2026, 10, 17, 12, 0, 0);
const SHOTS = resolve(import.meta.dirname, "../../../target/e2e-invoicer-shots");

/** Open the app the way `--server` prints it: the token in the launch fragment, on the root. */
async function open(page: Page): Promise<void> {
    await page.clock.setFixedTime(FIXED_NOW);
    await page.goto(`/app/#token=${API_TOKEN}`);
    await expect(page).toHaveURL(/\/app\/$/);
    await expect(page.getByTestId("dashboard")).toBeVisible();
}

test.describe("the dashboard over the accrual fixture (:5199)", () => {
    test("is the app's home and the sidebar's first section", async ({page}) => {
        await open(page);
        await expect(page.getByRole("heading", {name: "Dashboard", exact: true})).toBeVisible();
        const sections = page.getByTestId("sidebar").getByRole("navigation", {name: "Sections"}).getByRole("link");
        // Section names carry their counts for screen readers ("Invoices 2 overdue …"): a prefix match.
        await expect(sections.first()).toHaveText(/^Dashboard/);
        await expect(sections.first()).toHaveAttribute("aria-current", "page");
        await expect(page.getByTestId("sidebar-invoices")).not.toHaveAttribute("aria-current", "page");
    });

    test("shows the fixture's figures", async ({page}) => {
        await open(page);
        await expect(page.getByTestId("dashboard-total")).toHaveText("$22,850.00");
        await expect(page.getByTestId("dashboard-total-count")).toHaveText("7 items owed");
        await expect(page.getByTestId("dashboard-overdue")).toContainText("$12,700.00");
        await expect(page.getByTestId("dashboard-not-due")).toContainText("$10,150.00");
        await expect(page.getByTestId("dashboard-unfiled")).toContainText("$11,300.00");
        await expect(page.getByTestId("bar-overdue")).toBeVisible();
        await expect(page.getByTestId("bar-not-due")).toBeVisible();
        // Above $10,000 the bar's scale is the total: no grey rest.
        await expect(page.getByTestId("bar-rest")).toHaveCount(0);

        const rows = page.getByTestId("aging-row");
        await expect(rows).toHaveCount(5);
        for (const [at, text] of ["$10,150.00", "$7,000.00", "$1,500.00", "$1,200.00", "$3,000.00"].entries()) {
            await expect(rows.nth(at)).toContainText(text);
        }

        await expect(page.getByTestId("tile-oldest-overdue")).toContainText("125 days");
        await expect(page.getByTestId("tile-oldest-overdue")).toContainText("No. 2001 · Tailspin · $3,000.00 · due 2026-07-15");
        await expect(page.getByTestId("tile-dso")).toContainText("58.9 days");
        await expect(page.getByTestId("tile-days-to-pay")).toContainText("25.3 days");
        await expect(page.getByTestId("tile-drafts")).toContainText("$13,600.00");
        await expect(page.getByTestId("recognition-due")).toHaveText("10 entries");
        // `--server` has no window: no Open in Ledgeline, the way to post them instead.
        await expect(page.getByTestId("open-ledgeline")).toHaveCount(0);
        await expect(page.getByTestId("tile-recognition")).toContainText("Journal → Recognition");
    });

    test("every figure opens the invoice list, filtered", async ({page}) => {
        const cases: [string, RegExp][] = [
            ["dashboard-total", /\/app\/invoices\?status=open$/],
            ["bar-overdue", /\/app\/invoices\?status=overdue$/],
            ["dashboard-not-due", /\/app\/invoices\?aged=current$/],
            ["tile-dso", /\/app\/invoices\?status=open$/],
            ["tile-days-to-pay", /\/app\/invoices\?status=paid$/],
            ["tile-drafts", /\/app\/invoices\?status=draft$/],
            // 2001 has no invoice file: its customer's overdue invoices.
            ["tile-oldest-overdue", /\/app\/invoices\?status=overdue&customer=Tailspin$/],
        ];
        for (const [testid, url] of cases) {
            await open(page);
            await page.getByTestId(testid).click();
            await expect(page).toHaveURL(url);
            await expect(page.getByRole("heading", {name: "Invoices", exact: true})).toBeVisible();
        }
        await open(page);
        await page.getByRole("link", {name: "31–60 days past due: $1,500.00, 1 item", exact: true}).click();
        await expect(page).toHaveURL(/\/app\/invoices\?aged=31-60$/);
        await open(page);
        await page.getByRole("link", {name: "91+ days past due: $3,000.00, 1 item", exact: true}).click();
        await expect(page).toHaveURL(/\/app\/invoices\?aged=91-$/);
    });

    test("works from the keyboard", async ({page}) => {
        await open(page);
        // Every figure is a link: focus the total, Enter follows it.
        const total = page.getByTestId("dashboard-total");
        await total.focus();
        await expect(total).toBeFocused();
        await page.keyboard.press("Tab");
        await expect(page.getByTestId("bar-overdue")).toBeFocused();
        await page.keyboard.press("Tab");
        await expect(page.getByTestId("bar-not-due")).toBeFocused();
        await page.keyboard.press("Enter");
        await expect(page).toHaveURL(/\/app\/invoices\?aged=current$/);
    });

    test("fits a narrow window: one column, nothing wider than the window", async ({page}) => {
        await page.setViewportSize({width: 420, height: 900});
        await open(page);
        const main = page.locator("main");
        const overflow = await main.evaluate((el) => el.scrollWidth - el.clientWidth);
        expect(overflow).toBeLessThanOrEqual(0);
        const bar = await page.getByTestId("dashboard-bar").boundingBox();
        expect(bar).not.toBeNull();
        expect(bar!.x + bar!.width).toBeLessThanOrEqual(420);
        // The tiles stack: each starts where the first does.
        const tiles = page.getByTestId("dashboard-tiles").locator("> li");
        const first = await tiles.nth(0).boundingBox();
        const second = await tiles.nth(1).boundingBox();
        expect(second!.x).toBeCloseTo(first!.x, 0);
        expect(second!.y).toBeGreaterThan(first!.y);
        await expect(page.getByTestId("dashboard-total")).toBeVisible();
    });

    test("screenshots for review: wide and narrow, dark and light", async ({page}) => {
        for (const scheme of ["dark", "light"] as const) {
            await page.emulateMedia({colorScheme: scheme});
            for (const [name, width] of [
                ["wide", 1280],
                ["narrow", 420],
            ] as const) {
                await page.setViewportSize({width, height: 1000});
                await open(page);
                await expect(page.getByTestId("dashboard-total")).toHaveText("$22,850.00");
                await page.screenshot({path: `${SHOTS}/dashboard-${name}-${scheme}.png`, fullPage: true});
            }
        }
    });
});
