// Empty books end to end (Phase 4 feedback H20–H22): with no invoices,
// customers, catalog or snippets, the four lists show the one shared empty
// state (a titled card, one sentence, a primary button; no terminal
// instructions), each button starts the right thing, and the sidebar's New
// button reaches every create flow from the keyboard, in a narrow window too.
//
// Its own server: `--server` on :5198 over a scratch copy of
// invoicer/fixtures/empty (target/e2e-invoicer-empty/, staged fresh per run;
// playwright.config.ts). `vite preview` proxies only to :5199, so this spec
// loads the SPA the server embeds, as csp.same-origin.e2e.ts does, and skips
// loudly when the binary embeds build.rs's placeholder (REQUIRE_REAL_SPA=1
// makes that a failure). `[invoicer] number-start = 1001`.
//
// Serial: the catalog's button writes the catalog note, so it comes after
// the checks that need it absent.
//
// Written with H22 and NOT run by the agent that wrote it (agents never run
// Playwright); the lead runs it.

import {expect, test, type Page} from "@playwright/test";
import {API_TOKEN, EMPTY_SERVER_URL} from "../playwright.config";

const FIXED_NOW = new Date(2026, 10, 17, 12, 0, 0);
const PLACEHOLDER_MARKER = "Ledgeline Invoicer SPA not built";

test.use({baseURL: EMPTY_SERVER_URL});
test.describe.configure({mode: "serial"});

async function open(page: Page): Promise<void> {
    const shell = await page.request.get("/app/");
    const body = await shell.text();
    if (body.includes(PLACEHOLDER_MARKER) || !body.includes("__sveltekit")) {
        const why = "The server on :5198 embeds build.rs's PLACEHOLDER. Run `just invoicer-e2e` (it builds invoicer/web before the binary).";
        if (process.env.REQUIRE_REAL_SPA === "1") throw new Error(`REQUIRE_REAL_SPA=1: ${why}`);
        console.warn(`\n*** SKIPPING invoicer empty.e2e: ${why}\n`);
        test.skip(true, why);
    }
    await page.clock.setFixedTime(FIXED_NOW);
    await page.goto(`/app/#token=${API_TOKEN}`);
    await expect(page).toHaveURL(/\/app\/invoices$/);
}

async function section(page: Page, name: string): Promise<void> {
    await page.getByTestId("sidebar").getByRole("link", {name, exact: true}).click();
}

/** The shared empty state: its heading, and its one button primary and inside the card. */
async function expectEmpty(page: Page, testid: string, title: string, action: string): Promise<void> {
    const card = page.getByTestId(testid);
    await expect(card).toBeVisible();
    await expect(card.getByRole("heading", {name: title, exact: true})).toBeVisible();
    const button = card.getByRole(action === "New invoice" || action === "New customer" ? "link" : "button", {name: action, exact: true});
    await expect(button).toBeVisible();
    await expect(button).toHaveClass(/btn-primary/);
    await expect(card).not.toContainText("ledgeline-invoicer");
}

test.describe("empty books (:5198)", () => {
    test("every list shows the same empty state, and no second New button", async ({page}) => {
        await open(page);
        await expectEmpty(page, "invoice-list-empty", "No invoices yet", "New invoice");
        await expect(page.getByTestId("invoices-new")).toHaveCount(0);
        // No invoice counts at all; the other sections say what they count.
        await expect(page.getByTestId("sidebar-invoices-overdue")).toHaveCount(0);
        await expect(page.getByTestId("sidebar-invoices-count")).toHaveCount(0);
        await expect(page.getByTestId("sidebar-customers-count")).toHaveText("0 customers");
        await expect(page.getByTestId("sidebar-catalog-count")).toHaveText("0 items");
        await expect(page.getByTestId("sidebar-snippets-count")).toHaveText("0 snippets");

        await section(page, "Customers");
        await expect(page).toHaveURL(/\/app\/customers$/);
        await expectEmpty(page, "customer-list-empty", "No customers yet", "New customer");
        await expect(page.getByTestId("new-customer")).toHaveCount(0);

        await section(page, "Catalog");
        await expect(page).toHaveURL(/\/app\/catalog$/);
        await expectEmpty(page, "catalog-empty", "No catalog yet", "New item");
        await expect(page.getByTestId("catalog-new-item")).toHaveCount(0);

        await section(page, "Snippets");
        await expect(page).toHaveURL(/\/app\/snippets$/);
        await expectEmpty(page, "snippets-empty", "No snippets yet", "New snippet");
        await expect(page.getByTestId("new-snippet")).toHaveCount(0);
    });

    test("each empty state's button starts the right thing", async ({page}) => {
        await open(page);
        await page.getByTestId("invoice-list-new").click();
        await expect(page).toHaveURL(/\/app\/new$/);
        await expect(page.getByTestId("new-invoice")).toContainText("number 1001");
        await expect(page.getByTestId("new-number-reason")).toContainText("the first number");

        await section(page, "Customers");
        await page.getByTestId("customer-list-new").click();
        await expect(page).toHaveURL(/\/app\/customers\/new$/);

        await section(page, "Snippets");
        await page.getByTestId("snippets-empty-new").click();
        await expect(page.getByTestId("new-snippet-form")).toBeVisible();
        await expect(page.getByTestId("new-snippet-title")).toBeFocused();
        await expect(page.getByTestId("snippets-empty")).toHaveCount(0);

        // Last: this one writes the catalog note, then adds a row, focused.
        await section(page, "Catalog");
        await page.getByTestId("catalog-empty-new").click();
        await expect(page.getByTestId("catalog-row")).toHaveCount(1);
        await expect(page.getByLabel("Row 1 code", {exact: true})).toBeFocused();
        await expect(page.getByTestId("catalog-empty")).toHaveCount(0);
        await expect(page.getByTestId("catalog-new-item")).toHaveClass(/btn-primary/);
    });

    test("the New button's menu reaches every create flow from the keyboard", async ({page}) => {
        await open(page);
        const toggle = page.getByTestId("new-toggle");
        const menu = page.getByRole("menu");
        await expect(page.getByTestId("new-main")).toHaveText("New invoice");

        // ArrowDown opens on the first item; arrows move; Escape closes, focus back on the arrow.
        await toggle.focus();
        await page.keyboard.press("ArrowDown");
        await expect(menu).toBeVisible();
        await expect(menu.getByRole("menuitem")).toHaveText(["New invoice", "New customer", "New catalog item", "New snippet"]);
        await expect(menu.getByRole("menuitem", {name: "New invoice", exact: true})).toBeFocused();
        await page.keyboard.press("Escape");
        await expect(menu).toHaveCount(0);
        await expect(toggle).toBeFocused();

        // New customer.
        await page.keyboard.press("ArrowDown");
        await page.keyboard.press("ArrowDown");
        await expect(menu.getByRole("menuitem", {name: "New customer", exact: true})).toBeFocused();
        await page.keyboard.press("Enter");
        await expect(page).toHaveURL(/\/app\/customers\/new$/);

        // New snippet: the form opens and the parameter is dropped.
        await toggle.focus();
        await page.keyboard.press("ArrowUp");
        await expect(menu.getByRole("menuitem", {name: "New snippet", exact: true})).toBeFocused();
        await page.keyboard.press("Enter");
        await expect(page).toHaveURL(/\/app\/snippets$/);
        await expect(page.getByTestId("new-snippet-form")).toBeVisible();

        // New catalog item: the catalog the last test wrote gets a row, focused.
        await toggle.focus();
        await page.keyboard.press("Enter");
        await page.keyboard.press("End");
        await page.keyboard.press("ArrowUp");
        await expect(menu.getByRole("menuitem", {name: "New catalog item", exact: true})).toBeFocused();
        await page.keyboard.press("Enter");
        await expect(page).toHaveURL(/\/app\/catalog$/);
        await expect(page.getByTestId("catalog-row")).toHaveCount(1);
        await expect(page.getByLabel("Row 1 code", {exact: true})).toBeFocused();

        // The main part: New invoice. (The catalog row is unsaved: discard it on the way out.)
        page.once("dialog", (dialog) => void dialog.accept());
        await page.getByTestId("new-main").click();
        await expect(page).toHaveURL(/\/app\/new$/);
    });

    test("a narrow window: the New button sits in the top bar and its menu stays on screen", async ({page}) => {
        await page.setViewportSize({width: 480, height: 800});
        await open(page);
        const toggle = page.getByTestId("new-toggle");
        await expect(page.getByTestId("new-main")).toBeVisible();
        await toggle.click();
        const menu = page.getByRole("menu");
        await expect(menu).toBeVisible();
        const box = await menu.boundingBox();
        expect(box).not.toBeNull();
        expect(box!.x).toBeGreaterThanOrEqual(0);
        expect(box!.x + box!.width).toBeLessThanOrEqual(480);
        await expect(menu.getByRole("menuitem")).toHaveCount(4);
        // A click outside closes it.
        await page.getByRole("heading", {name: "Invoices", exact: true}).click();
        await expect(menu).toHaveCount(0);
        await expectEmpty(page, "invoice-list-empty", "No invoices yet", "New invoice");
    });
});
