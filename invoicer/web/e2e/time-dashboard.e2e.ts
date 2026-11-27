// Time tracking's dashboard end to end (plan 46 T-dash): the figures for
// the time fixture's October against September, the URL as the filter
// bar's state (a filter is a URL, Back undoes it, `[`/`]` step the period),
// every figure a link into Entries with the same filters, the hledger
// command in each (i), and the tree's own toggles.
//
// Its own server, shared with time-track.e2e.ts: `--server` on :5194 over
// a scratch copy of invoicer/fixtures/time (TIME_SERVER_URL/TIME_COPY in
// playwright.config.ts, added by the Track work; mirrored here until the
// merge, so this file type-checks on its own). `vite preview` proxies only
// to :5199, so this spec loads the SPA the server embeds and skips loudly
// when that is build.rs's placeholder (REQUIRE_REAL_SPA=1 makes it fail).
//
// The browser's clock is the fixture's (2026-11-26). Track's spec writes
// minutes into the copy's CURRENT month on the real clock, so the exact
// figures here are October's (a custom range), which nothing writes.
//
// Written with the feature and NOT run by the agent that wrote it (agents
// never run Playwright); the lead runs it. E2E WIRING: replace the two
// constants below with imports of TIME_SERVER_URL from playwright.config.ts
// once both branches are merged; the dashboard is mounted at /time.

import {expect, test, type Page} from "@playwright/test";
import {API_TOKEN} from "../playwright.config";

const TIME_SERVER_URL = "http://127.0.0.1:5194";
const FIXED_NOW = new Date(2026, 10, 26, 12, 0, 0); // local 2026-11-26, the fixture's clock
const PLACEHOLDER_MARKER = "Ledgeline Invoicer SPA not built";
const OCTOBER = "from=2026-10-01&to=2026-10-31";

test.use({baseURL: TIME_SERVER_URL});

async function open(page: Page, search = ""): Promise<void> {
    const shell = await page.request.get(`${TIME_SERVER_URL}/app/`);
    const body = await shell.text();
    if (body.includes(PLACEHOLDER_MARKER) || !body.includes("__sveltekit")) {
        const why = "The server embeds build.rs's PLACEHOLDER. Run `just invoicer-e2e` (it builds invoicer/web before the binary).";
        if (process.env.REQUIRE_REAL_SPA === "1") throw new Error(`REQUIRE_REAL_SPA=1: ${why}`);
        console.warn(`\n*** SKIPPING invoicer time-dashboard.e2e: ${why}\n`);
        test.skip(true, why);
    }
    await page.clock.setFixedTime(FIXED_NOW);
    await page.goto(`${TIME_SERVER_URL}/app/time${search}#token=${API_TOKEN}`);
    await expect(page.getByTestId("time-dashboard")).toBeVisible();
}

test.describe("time dashboard", () => {
    test("shows October against September: tiles, bars, tree and lists", async ({page}) => {
        await open(page, `?${OCTOBER}`);
        await expect(page.getByTestId("tile-total-value")).toHaveText("9h");
        await expect(page.getByTestId("tile-total-delta")).toContainText("+7h 30m");
        await expect(page.getByTestId("tile-billable-value")).toHaveText("8h 15m");
        await expect(page.getByTestId("tile-days-value")).toHaveText("2 of 22");
        await expect(page.getByTestId("tile-unbilled-value")).toHaveText("3h");
        await expect(page.getByTestId("time-dash-range")).toContainText("compared with Sep 1 – 30, 2026");
        // A column per day; the 6th's bar is clients and internal.
        await expect(page.getByTestId("time-dash-column")).toHaveCount(31);
        const sixth = page.getByTestId("time-dash-column").nth(5);
        await expect(sixth).toHaveAttribute("aria-label", /^Tue, Oct 6, 2026: 4h 35m/);
        await sixth.hover();
        await expect(page.getByTestId("time-dash-tooltip")).toContainText("4h 35m");
        // The tree at depth 2, and the top accounts linked into Entries.
        await expect(page.locator('[data-testid="time-dash-tree-row"][data-account="clients:acme"]')).toBeVisible();
        await expect(page.locator('[data-testid="time-dash-tree-row"][data-account="clients:acme:tax-return"]')).toHaveCount(0);
        await page.getByRole("button", {name: "Expand clients:acme"}).click();
        await expect(page.locator('[data-testid="time-dash-tree-row"][data-account="clients:acme:tax-return"]')).toBeVisible();
        const first = page.getByTestId("time-dash-top-accounts-item").first();
        await expect(first).toHaveAttribute("data-key", "clients:acme");
        await expect(first.getByRole("link")).toHaveAttribute("href", `/app/time/entries?${OCTOBER}&account=clients%3Aacme`);
    });

    test("each figure's (i) gives the hledger command", async ({page}) => {
        await open(page, `?${OCTOBER}`);
        await page.getByRole("button", {name: "About total hours"}).focus();
        await expect(page.getByRole("tooltip").filter({hasText: "hledger"})).toContainText("hledger -f time.journal bal -p 2026-10 --depth 0");
        await page.getByRole("button", {name: "About the account tree"}).focus();
        await expect(page.getByRole("tooltip").filter({hasText: "--tree"})).toContainText("bal -p 2026-10 --tree --depth 2");
    });

    test("filters live in the URL: a choice is a step, Back undoes it, [ and ] step the period", async ({page}) => {
        await open(page, `?${OCTOBER}`);
        await page.getByTestId("time-dash-billable").selectOption("yes");
        await expect(page).toHaveURL(/billable=yes/);
        await expect(page.getByTestId("tile-total-value")).toHaveText("8h 15m");
        await page.goBack();
        await expect(page).not.toHaveURL(/billable=/);
        await expect(page.getByTestId("tile-total-value")).toHaveText("9h");

        // `/` focuses the account filter; typing narrows after a pause.
        await page.keyboard.press("/");
        await expect(page.getByTestId("time-dash-account")).toBeFocused();
        await page.keyboard.type("internal");
        await expect(page).toHaveURL(/account=internal/);
        await expect(page.getByTestId("tile-total-value")).toHaveText("45m");
        await page.keyboard.press("Escape");
        await expect(page).not.toHaveURL(/account=/);
        await page.keyboard.press("Escape");

        // October's next is November: this month, the default, so a bare URL.
        await page.keyboard.press("]");
        await expect(page).not.toHaveURL(/from=|period=/);
        await page.keyboard.press("[");
        await page.keyboard.press("[");
        await expect(page).toHaveURL(/from=2026-09-01&to=2026-09-30/);
        await expect(page.getByTestId("tile-total-value")).toHaveText("1h 30m");
    });

    test("drills into Entries with the same filters", async ({page}) => {
        await open(page, `?${OCTOBER}&customer=Acme+Corp`);
        await expect(page.getByTestId("tile-unbilled-value")).toHaveAttribute("href", `/app/time/entries?${OCTOBER}&customer=Acme+Corp&billed=unbilled`);
        await page.getByTestId("tile-total-value").click();
        await expect(page).toHaveURL(new RegExp(`/app/time/entries\\?${OCTOBER}&customer=Acme\\+Corp`));
    });
});
