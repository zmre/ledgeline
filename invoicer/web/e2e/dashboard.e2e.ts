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
//   - one draft (108, $13,600.00); 10 recognition entries due, 1 more closed;
//   - paid in the last 30 days (2026-10-19 to 2026-11-17): $3,900.00 into 2
//     invoices (102 in full on 10-20, $1,000 of 107 on 11-05); 7 days:
//     nothing; 365 days: $16,250.00 into 4 (101 and 104 too);
//   - recently updated, latest first: 108 (dated 11-17, draft), 105 (sent
//     11-10, open), 107 (paid 11-05, overdue), 102 (paid 10-20), 103 (voided
//     10-10), 104. The scratch copy's file times are the run's (`cp -R`),
//     after the clock, so they don't count (R15);
//   - the bar's axis: 0, 5k … 25k.
//
// Screenshots for review (and, in Phase 6, the docs) go to
// target/e2e-invoicer-shots/ (gitignored): wide and narrow, dark and light,
// in the app's own theme (`openThemed`: the prefs answer the variant, a full
// reload, transitions settled), with the OS scheme emulated to match. A
// contrast check runs each theme against the opposite OS scheme.
//
// Written with 5a, reworked with R13–R18; NOT run by the agent that wrote
// it (agents never run Playwright); the lead runs it.

import {resolve} from "node:path";
import {expect, test, type Locator, type Page} from "@playwright/test";
import {API_TOKEN} from "../playwright.config";

const FIXED_NOW = new Date(2026, 10, 17, 12, 0, 0);
const SHOTS = resolve(import.meta.dirname, "../../../target/e2e-invoicer-shots");

/** Open the app the way `--server` prints it: the token in the launch fragment, on the root. */
async function open(page: Page): Promise<void> {
    await page.clock.setFixedTime(FIXED_NOW);
    await page.goto(`/app/#token=${API_TOKEN}`);
    // Still the dashboard (no redirect); a repeat visit may keep the `#token=` fragment.
    await expect(page).toHaveURL(/\/app\/(#.*)?$/);
    await expect(page.getByTestId("dashboard")).toBeVisible();
}

/** The two variants the screenshots and the contrast check use. */
const THEMES = ["default-dark", "default-light"] as const;

/**
 * Open the dashboard in the app's own theme `variant`, set the way the app
 * sets it: the saved prefs (`GET /api/ui-prefs`, answered here with
 * `theme: variant`, since this server keeps prefs in the browser and always
 * answers the default), applied to `<html data-theme>` by the layout. A full
 * reload, not a hash-only navigation, so nothing switches theme live; then
 * wait until the theme is on and no transition is running.
 */
async function openThemed(page: Page, variant: (typeof THEMES)[number]): Promise<void> {
    await page.unroute("**/api/ui-prefs");
    await page.route("**/api/ui-prefs", async (route) => {
        if (route.request().method() !== "GET") return route.continue();
        const response = await route.fetch();
        const body = (await response.json()) as {prefs?: Record<string, unknown>};
        await route.fulfill({response, json: {...body, prefs: {...body.prefs, theme: variant}}});
    });
    await open(page);
    await page.reload();
    await expect(page.locator("html")).toHaveAttribute("data-theme", variant);
    await expect(page.getByTestId("dashboard")).toBeVisible();
    await page.waitForFunction(() => document.getAnimations().length === 0);
}

/**
 * WCAG contrast of `locator`'s text against what is behind it: every
 * ancestor's background composited from the root down (on white), the text
 * colour over that, each resolved through a canvas so any CSS colour
 * (oklch, with alpha) reads.
 */
async function contrastOf(locator: Locator): Promise<number> {
    return locator.evaluate((el) => {
        const canvas = document.createElement("canvas");
        canvas.width = canvas.height = 1;
        const ctx = canvas.getContext("2d", {willReadFrequently: true})!;
        const paint = (layers: string[]): number[] => {
            ctx.globalCompositeOperation = "source-over";
            ctx.fillStyle = "#fff";
            ctx.fillRect(0, 0, 1, 1);
            for (const layer of layers) {
                ctx.fillStyle = layer;
                ctx.fillRect(0, 0, 1, 1);
            }
            return [...ctx.getImageData(0, 0, 1, 1).data.slice(0, 3)];
        };
        const layers: string[] = [];
        for (let node: Element | null = el; node !== null; node = node.parentElement) {
            const bg = getComputedStyle(node).backgroundColor;
            if (bg !== "" && bg !== "transparent" && bg !== "rgba(0, 0, 0, 0)") layers.unshift(bg);
        }
        const back = paint(layers);
        const fore = paint([...layers, getComputedStyle(el).color]);
        const luminance = (rgb: number[]): number => {
            const [r, g, b] = rgb.map((c) => {
                const v = c / 255;
                return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
            });
            return 0.2126 * r + 0.7152 * g + 0.0722 * b;
        };
        const [hi, lo] = [luminance(fore), luminance(back)].sort((a, b) => b - a);
        return (hi + 0.05) / (lo + 0.05);
    });
}

/** The invoice list's rows, by number. */
const listed = (page: Page) => page.getByTestId("invoice-row").evaluateAll((rows) => rows.map((r) => (r as HTMLElement).dataset.number ?? ""));

test.describe("the dashboard over the accrual fixture (:5199)", () => {
    test("is the app's home and the sidebar's first section", async ({page}) => {
        await open(page);
        await expect(page.getByRole("heading", {name: "Dashboard", exact: true})).toBeVisible();
        // No New invoice button of its own: the sidebar's New is the one (R13).
        await expect(page.getByTestId("dashboard-new")).toHaveCount(0);
        await expect(page.getByTestId("dashboard").getByRole("link", {name: "New invoice", exact: true})).toHaveCount(0);
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
        // Above $10,000 the axis runs to the round step past the total.
        await expect(page.getByTestId("bar-tick")).toHaveText(["0", "5k", "10k", "15k", "20k", "25k"]);

        const rows = page.getByTestId("aging-row");
        await expect(rows).toHaveCount(5);
        for (const [at, text] of ["$10,150.00", "$7,000.00", "$1,500.00", "$1,200.00", "$3,000.00"].entries()) {
            await expect(rows.nth(at)).toContainText(text);
        }

        await expect(page.getByTestId("tile-oldest-overdue")).toContainText("125 days");
        await expect(page.getByTestId("tile-oldest-overdue")).toContainText("No. 2001 · Tailspin · $3,000.00 · due 2026-07-15");
        await expect(page.getByTestId("tile-dso")).toContainText("58.9 days");
        await expect(page.getByTestId("tile-days-to-pay")).toContainText("25.3 days");
        // The funnel: drafts, outstanding, paid in the last 30 days (the setting's default).
        await expect(page.getByTestId("funnel-drafts-amount")).toHaveText("$13,600.00");
        await expect(page.getByTestId("funnel-drafts-count")).toHaveText("1 draft");
        await expect(page.getByTestId("funnel-outstanding-amount")).toHaveText("$22,850.00");
        await expect(page.getByTestId("funnel-overdue")).toContainText("$12,700.00 overdue");
        await expect(page.getByTestId("funnel-paid-amount")).toHaveText("$3,900.00");
        await expect(page.getByTestId("funnel-paid-count")).toHaveText("2 paid");
        await expect(page.getByTestId("paid-window")).toHaveValue("30");
        // Recently updated, latest first.
        const cards = page.getByTestId("recent-card");
        await expect(cards).toHaveCount(6);
        await expect
            .poll(() => cards.evaluateAll((els) => els.map((e) => (e as HTMLElement).dataset.number)))
            .toEqual(["108", "105", "107", "102", "103", "104"]);
        // A card's text holds its parts and the band's word; match pieces, not the whole.
        await expect(cards.first()).toContainText("Customer X");
        await expect(cards.first()).toContainText("Dated 2026-11-17");
        await expect(cards.first().getByTestId("recent-status")).toHaveText("Draft");
        await expect(cards.nth(2).getByTestId("recent-status")).toHaveText("Overdue");
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
            ["funnel-drafts", /\/app\/invoices\?status=draft$/],
            ["funnel-outstanding", /\/app\/invoices\?status=open$/],
            ["funnel-paid", /\/app\/invoices\?paid_from=2026-10-19&paid_to=2026-11-17$/],
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
        // (31–60 is an item booked without an invoice file: the list has nothing for it.)
        await expect(page).toHaveURL(/\/app\/invoices\?aged=31-60$/);
        await open(page);
        await page.getByRole("link", {name: "91+ days past due: $3,000.00, 1 item", exact: true}).click();
        await expect(page).toHaveURL(/\/app\/invoices\?aged=91%2B$/);
    });

    test("its figures filter the list to those invoices (5b's parser)", async ({page}) => {
        // The invoices behind each figure that have a file, on 2026-11-17:
        // 107 is 3 days late, 106 78 days, 105 not yet due (108 is a draft).
        const numbers = () => listed(page);
        const cases: [string, RegExp, string[]][] = [
            ["61–90", /\?aged=61-90$/, ["106"]],
            ["1–30", /\?aged=1-30$/, ["107"]],
        ];
        for (const [bucket, url, shown] of cases) {
            await open(page);
            await page.getByRole("link", {name: new RegExp(`^${bucket} days past due:`)}).click();
            await expect(page).toHaveURL(url);
            await expect.poll(numbers).toEqual(shown);
            await expect(page.getByTestId("invoice-filter-aged")).toHaveValue(url.source.match(/aged=([^$]+)/)![1]);
        }
        await open(page);
        await page.getByTestId("bar-overdue").click();
        await expect.poll(numbers).toEqual(["107", "106"]);
        await expect(page.getByTestId("invoice-filter-status")).toHaveValue("overdue");
        await open(page);
        await page.getByTestId("dashboard-not-due").click();
        await expect.poll(numbers).toEqual(["105"]);
        // The last bucket's link (91+): only an item without a file, so nothing matches.
        await open(page);
        await page.getByRole("link", {name: /^91\+ days past due:/}).click();
        await expect(page.getByTestId("invoice-list-no-match")).toBeVisible();
        await expect(page.getByTestId("invoice-filter-aged")).toHaveValue("91+");
        // Tailspin's overdue: by the books' name; its one overdue item has no file either.
        await open(page);
        await page.getByTestId("tile-oldest-overdue").click();
        await expect(page.getByTestId("invoice-list-no-match")).toBeVisible();
    });

    test("the paid figure lists the invoices money came in for, by payment date (R14)", async ({page}) => {
        await open(page);
        await page.getByTestId("funnel-paid").click();
        await expect(page).toHaveURL(/\/app\/invoices\?paid_from=2026-10-19&paid_to=2026-11-17$/);
        await expect.poll(() => listed(page)).toEqual(["107", "102"]);
        // The range shows as a chip that removes itself.
        await expect(page.getByTestId("invoice-filter-paid")).toContainText("Paid 2026-10-19 – 2026-11-17");
        await page.getByTestId("invoice-filter-paid-remove").click();
        await expect(page).toHaveURL(/\/app\/invoices$/);
        await expect(page.getByTestId("invoice-filter-paid")).toHaveCount(0);
    });

    test("the period selector changes the paid window, in the URL", async ({page}) => {
        await open(page);
        const select = page.getByTestId("paid-window");
        await select.selectOption("365");
        await expect(page).toHaveURL(/\/app\/\?paid_days=365$/);
        await expect(page.getByTestId("funnel-paid-amount")).toHaveText("$16,250.00");
        await expect(page.getByTestId("funnel-paid-count")).toHaveText("4 paid");
        await expect(page.getByTestId("funnel-paid")).toHaveAttribute("href", /paid_from=2025-11-18&paid_to=2026-11-17$/);
        await select.selectOption("7");
        await expect(page).toHaveURL(/\/app\/\?paid_days=7$/);
        await expect(page.getByTestId("funnel-paid-amount")).toHaveText("Nothing yet");
        // Back steps to the last window; the setting's own is the bare page.
        await page.goBack();
        await expect(page).toHaveURL(/\/app\/\?paid_days=365$/);
        await expect(select).toHaveValue("365");
        await select.selectOption("30");
        await expect(page).toHaveURL(/\/app\/$/);
        await expect(page.getByTestId("funnel-paid-amount")).toHaveText("$3,900.00");
        // A link with a window opens on it.
        await page.goto("/app/?paid_days=90");
        await expect(page.getByTestId("paid-window")).toHaveValue("90");
    });

    test("a recently updated card opens its invoice", async ({page}) => {
        await open(page);
        await page.locator('[data-testid="recent-card"][data-number="105"]').getByRole("link").click();
        await expect(page).toHaveURL(/\/app\/invoices\/105$/);
    });

    test("hovering a bar segment shows its tooltip; so does keyboard focus", async ({page}) => {
        await open(page);
        await page.getByTestId("bar-overdue").hover();
        const tip = page.getByRole("tooltip");
        await expect(tip).toBeVisible();
        await expect(tip).toContainText("$12,700.00");
        await expect(tip).toContainText("Overdue");
        await expect(tip).toContainText("5 items · 56% of what is owed");
        await page.mouse.move(0, 0);
        await expect(tip).toHaveCount(0);
        await page.getByTestId("bar-not-due").focus();
        await expect(page.getByRole("tooltip")).toContainText("$10,150.00");
        await page.keyboard.press("Escape");
        await expect(page.getByRole("tooltip")).toHaveCount(0);
    });

    test("works from the keyboard", async ({page}) => {
        await open(page);
        // Every figure is a link: focus a segment, Tab to the next, Enter follows it.
        const overdue = page.getByTestId("bar-overdue");
        await overdue.focus();
        await expect(overdue).toBeFocused();
        await page.keyboard.press("Tab");
        await expect(page.getByTestId("bar-not-due")).toBeFocused();
        await page.keyboard.press("Enter");
        await expect(page).toHaveURL(/\/app\/invoices\?aged=current$/);
        // The period selector is reachable by Tab from the page's heading.
        await open(page);
        await page.getByRole("heading", {name: "Dashboard", exact: true}).click();
        const select = page.getByTestId("paid-window");
        for (let i = 0; i < 40 && !(await select.evaluate((el) => el === document.activeElement)); i++) await page.keyboard.press("Tab");
        await expect(select).toBeFocused();
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
        // The funnel's cards stack too.
        const drafts = await page.getByTestId("funnel-drafts").boundingBox();
        const owed = await page.getByTestId("funnel-outstanding").boundingBox();
        expect(owed!.x).toBeCloseTo(drafts!.x, 0);
        expect(owed!.y).toBeGreaterThan(drafts!.y);
        // Recently updated scrolls sideways in its own box, not the page.
        const row = page.getByTestId("recent-cards");
        const scrolls = await row.evaluate((el) => el.scrollWidth > el.clientWidth);
        expect(scrolls).toBe(true);
        // The tiles stack: each starts where the first does.
        const tiles = page.getByTestId("dashboard-tiles").locator("> li");
        const first = await tiles.nth(0).boundingBox();
        const second = await tiles.nth(1).boundingBox();
        expect(second!.x).toBeCloseTo(first!.x, 0);
        expect(second!.y).toBeGreaterThan(first!.y);
        await expect(page.getByTestId("dashboard-total")).toBeVisible();
    });

    test("reads in both themes: the app's theme, not the OS scheme, colours every card", async ({page}) => {
        for (const variant of THEMES) {
            // The OS says the opposite: a style keyed to prefers-color-scheme would show here.
            await page.emulateMedia({colorScheme: variant === "default-light" ? "dark" : "light"});
            await page.setViewportSize({width: 1280, height: 1000});
            await openThemed(page, variant);
            const checks: [string, Locator][] = [
                ["the Drafts label", page.getByTestId("funnel-drafts").getByText("Drafts", {exact: true})],
                ["the drafts amount", page.getByTestId("funnel-drafts-amount")],
                ["the paid card's label", page.getByTestId("funnel-paid").getByText(/^Paid · last/)],
                ["a recent card's customer", page.getByTestId("recent-card").first().getByText("Customer X")],
                ["a recent card's status band", page.getByTestId("recent-status").first()],
                ["an aging row", page.getByTestId("aging-row").first()],
                ["the hero total", page.getByTestId("dashboard-total")],
            ];
            for (const [what, locator] of checks) {
                expect(await contrastOf(locator), `${what} in ${variant}`).toBeGreaterThanOrEqual(4.5);
            }
            // A card's surface is the theme's own surface, the panel's too.
            const surface = (l: Locator) => l.evaluate((el) => getComputedStyle(el).backgroundColor);
            expect(await surface(page.getByTestId("funnel-drafts"))).toBe(await surface(page.getByTestId("receivables-panel")));
        }
    });

    test("screenshots for review: wide and narrow, dark and light", async ({page}) => {
        for (const variant of THEMES) {
            const scheme = variant === "default-light" ? "light" : "dark";
            await page.emulateMedia({colorScheme: scheme});
            for (const [name, width] of [
                ["wide", 1280],
                ["narrow", 420],
            ] as const) {
                await page.setViewportSize({width, height: 1000});
                await openThemed(page, variant);
                await expect(page.getByTestId("dashboard-total")).toHaveText("$22,850.00");
                // The narrow cards are as they are wide: nothing dimmed, nothing loading.
                await expect(page.getByTestId("funnel-paid")).not.toHaveAttribute("aria-busy", "true");
                await page.screenshot({path: `${SHOTS}/dashboard-${name}-${scheme}.png`, fullPage: true, animations: "disabled"});
            }
            // The tooltip, wide.
            await page.setViewportSize({width: 1280, height: 1000});
            await openThemed(page, variant);
            await page.getByTestId("bar-overdue").hover();
            await expect(page.getByRole("tooltip")).toBeVisible();
            await page.screenshot({path: `${SHOTS}/dashboard-tooltip-${scheme}.png`, animations: "disabled"});
        }
        // At 150–200 % scale the container queries see a narrower page: emulate with a mid width.
        await page.emulateMedia({colorScheme: "light"});
        await page.setViewportSize({width: 760, height: 1000});
        await openThemed(page, "default-light");
        await page.screenshot({path: `${SHOTS}/dashboard-medium-light.png`, fullPage: true, animations: "disabled"});
    });
});
