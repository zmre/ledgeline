// Keyboard-first (invoicer PLAN 5k): the app without the mouse. The invoice
// list driven by `j`/`k`/`o`, its sort keys, `/` and Escape in the search,
// the `?` sheet; then an invoice end to end on keys alone: `n`, the
// customer, rows by Alt+Shift+letter, Cmd/Ctrl+S, Escape out of the field,
// Shift+S to send, Shift+P for the PDF.
//
// Its own Playwright project, after `lists` (playwright.config.ts): it runs
// last because it adds and sends an invoice for Fabrikam, which every
// earlier spec's figures would see. It relies only on fixture facts the
// others leave alone, at the clock 2026-11-17 (invoicer/fixtures/README.md):
//   - Fabrikam has no billing portal, so Send needs no reference; its own
//     template is the books' default one;
//   - the catalog's ARCH is "Senior architect hours", h, $800,
//     revenue:consulting (library.e2e.ts puts it back);
//   - the new invoice's number is read off the New invoice page (2006 after
//     editor.e2e.ts's 2005), not assumed.
// No mouse: no click(), no fill(); focus moves by the app's keys and Tab.
// Asserting with locators is reading, not pointing.
//
// Written in 5k and NOT run by the agent that wrote it (agents never run
// Playwright); the lead runs it.

import {execFileSync} from "node:child_process";
import {readFileSync} from "node:fs";
import {resolve} from "node:path";
import {expect, test, type Locator, type Page} from "@playwright/test";
import {API_TOKEN, COPY} from "../playwright.config";

const FIXED_NOW = new Date(2026, 10, 17, 12, 0, 0); // local 2026-11-17, the fixture's clock
const BOOKS = resolve(import.meta.dirname, "..", COPY);

const read = (file: string): string => readFileSync(resolve(BOOKS, file), "utf8");

/** Open the app on the invoice list with the launch token (`/app/` is the dashboard, R1). */
async function open(page: Page): Promise<void> {
    await page.clock.setFixedTime(FIXED_NOW);
    await page.goto(`/app/invoices#token=${API_TOKEN}`);
    await expect(page).toHaveURL(/\/app\/invoices$/);
    await expect(page.getByTestId("invoice-row").first()).toBeVisible();
}

/** Press Tab until `target` has focus (a field can be several tab stops, a date input's segments). */
async function tabTo(page: Page, target: Locator, max = 20): Promise<void> {
    for (let step = 0; step < max; step += 1) {
        if (await target.evaluate((node) => node === document.activeElement || node.contains(document.activeElement))) return;
        await page.keyboard.press("Tab");
    }
    await expect(target).toBeFocused();
}

/** A two-key chord (`g i`). */
async function chord(page: Page, first: string, second: string): Promise<void> {
    await page.keyboard.press(first);
    await page.keyboard.press(second);
}

/** The computed row amounts, once they are the server's (not the display-only estimate). */
async function serverAmounts(page: Page): Promise<string[]> {
    const amounts = page.getByTestId("grid-amount");
    await expect(amounts.and(page.locator('[data-optimistic="true"]'))).toHaveCount(0);
    await expect(page.getByTestId("grid-total")).toHaveAttribute("data-optimistic", "false");
    return (await amounts.allTextContents()).map((t) => t.trim());
}

test.describe.configure({mode: "serial"});

test.describe("keyboard-first over the accrual fixture (:5199)", () => {
    test("drives the invoice list: the row cursor, o, the sort keys, / and Escape, the go-to chords", async ({page}) => {
        await open(page);
        const rows = page.getByTestId("invoice-row");
        const cursor = page.locator('[data-testid="invoice-row"][data-cursor]');

        // j puts the cursor on the first row and focuses its link; j/k move it; G and g g jump.
        await page.keyboard.press("j");
        await expect(cursor).toHaveCount(1);
        await expect(rows.first()).toHaveAttribute("data-cursor", "");
        await expect(rows.first().locator("a[data-row-link]")).toBeFocused();
        await page.keyboard.press("j");
        await expect(rows.nth(1)).toHaveAttribute("data-cursor", "");
        await page.keyboard.press("k");
        await expect(rows.first()).toHaveAttribute("data-cursor", "");
        await page.keyboard.press("G");
        await expect(rows.last()).toHaveAttribute("data-cursor", "");
        await chord(page, "g", "g");
        await expect(rows.first()).toHaveAttribute("data-cursor", "");
        await page.keyboard.press("Escape");
        await expect(cursor).toHaveCount(0);

        // o opens the cursored invoice; g i comes back.
        await page.keyboard.press("j");
        const number = await rows.first().getAttribute("data-number");
        await page.keyboard.press("o");
        await expect(page).toHaveURL(new RegExp(`/app/invoices/${number}$`));
        await expect(page.getByTestId("invoice-heading")).toBeVisible();
        await chord(page, "g", "i");
        await expect(page).toHaveURL(/\/app\/invoices$/);

        // s sorts by the next column (Date is the default, so Due); S flips it.
        await expect(rows.first()).toBeVisible();
        await page.keyboard.press("s");
        await expect(page).toHaveURL(/sort=due/);
        await page.keyboard.press("S");
        await expect(page).toHaveURL(/sort=due&dir=desc/);
        // g i on a filtered or sorted list goes to the plain list.
        await chord(page, "g", "i");
        await expect(page).toHaveURL(/\/app\/invoices$/);

        // / focuses the search without typing a slash; Escape clears it, a second leaves it.
        const search = page.getByTestId("invoice-search");
        await page.keyboard.press("/");
        await expect(search).toBeFocused();
        await expect(search).toHaveValue("");
        await page.keyboard.type("Contoso");
        await expect(page).toHaveURL(/q=Contoso/);
        await expect(rows).not.toHaveCount(0);
        await page.keyboard.press("Escape");
        await expect(search).toHaveValue("");
        await expect(page).toHaveURL(/\/app\/invoices$/);
        await expect(search).toBeFocused();
        await page.keyboard.press("Escape");
        await expect(search).not.toBeFocused();
        await page.keyboard.press("j");
        await expect(rows.first()).toHaveAttribute("data-cursor", "");

        // The go-to chords, one per section.
        for (const [key, path, heading] of [
            ["c", /\/app\/customers$/, "Customers"],
            ["k", /\/app\/catalog$/, "Catalog"],
            ["s", /\/app\/snippets$/, "Snippets"],
            ["t", /\/app\/templates$/, "Templates"],
            ["d", /\/app\/$/, "Home"],
        ] as const) {
            await chord(page, "g", key);
            await expect(page).toHaveURL(path);
            await expect(page.getByRole("heading", {level: 1, name: heading, exact: true})).toBeVisible();
        }
    });

    test("the ? sheet lists what the page binds, grouped, and Escape closes it", async ({page}) => {
        await open(page);
        await page.keyboard.press("?");
        const sheet = page.getByTestId("key-help");
        await expect(sheet).toBeVisible();
        const headings = sheet.getByRole("heading", {level: 4});
        await expect(headings).toHaveText(["Global", "Go to", "Invoices"]);
        await expect(sheet).toContainText("Search this list");
        await expect(sheet).toContainText("Go to Customers");
        await expect(sheet).toContainText("Next invoice");
        await expect(sheet).toContainText("Sort by the next column");
        await page.keyboard.press("Escape");
        await expect(sheet).toHaveCount(0);

        // The dashboard's own keys, on Home.
        await chord(page, "g", "d");
        await expect(page.getByTestId("dashboard")).toBeVisible();
        await page.keyboard.press("?");
        await expect(sheet.getByRole("heading", {level: 4})).toHaveText(["Global", "Go to", "Home"]);
        await expect(sheet).toContainText("Open the drafts");
        await page.keyboard.press("Escape");
        // 1 opens the drafts card's list.
        await page.keyboard.press("1");
        await expect(page).toHaveURL(/\/app\/invoices\?status=draft$/);
    });

    test("writes, saves, sends and exports an invoice without the mouse", async ({page}) => {
        test.setTimeout(120_000);
        await open(page);

        // n: New invoice, the customer field focused on arrival.
        await page.keyboard.press("n");
        await expect(page).toHaveURL(/\/app\/new$/);
        const customer = page.getByTestId("new-customer");
        await expect(customer).toBeFocused();
        await page.keyboard.type("Fabrik");
        await expect(page.getByRole("option", {name: /^Fabrikam/})).toBeVisible();
        await page.keyboard.press("Enter");
        await expect(customer).toHaveValue("Fabrikam");
        const numberText = await page.getByTestId("new-number").locator("span.font-medium").textContent();
        const number = (numberText ?? "").trim();
        expect(number).toMatch(/^\d+$/);
        const file = `invoices/2026-11-17 Fabrikam Invoice ${number}.md`;
        await tabTo(page, page.getByTestId("new-create"));
        await page.keyboard.press("Enter");
        await expect(page).toHaveURL(new RegExp(`/app/invoices/${number}/edit$`));
        await expect(page.getByTestId("editor-heading")).toHaveText(`Invoice ${number}`);
        await expect(page.getByTestId("grid-row")).toHaveCount(0);

        // c goes to the customer field; Escape leaves it again.
        await page.keyboard.press("c");
        await expect(page.getByTestId("header-customer")).toBeFocused();
        await page.keyboard.press("Escape");
        await expect(page.getByTestId("header-customer")).not.toBeFocused();

        // Alt+Shift+I: an item row, focused on its description; the catalog by typing and Enter.
        await page.keyboard.press("Alt+Shift+I");
        const description = page.getByLabel("Row 1 description", {exact: true});
        await expect(description).toBeFocused();
        await page.keyboard.type("Senior");
        await expect(page.getByRole("option", {name: /^Senior architect hours/})).toBeVisible();
        await page.keyboard.press("Enter");
        await expect(description).toHaveValue("Senior architect hours");
        const quantity = page.getByLabel("Row 1 quantity");
        await tabTo(page, quantity);
        await page.keyboard.press("ControlOrMeta+a");
        await page.keyboard.type("10");
        await expect(quantity).toHaveValue("10");

        // Alt+Shift+D from the quantity cell: a 10% discount below it.
        await page.keyboard.press("Alt+Shift+D");
        await expect(page.getByTestId("grid-row")).toHaveCount(2);
        await expect(page.getByLabel("Row 2 description", {exact: true})).toBeFocused();
        expect(await serverAmounts(page)).toEqual(["$8,000.00", "-$800.00"]);
        await expect(page.getByTestId("grid-total")).toHaveText("$7,200.00");

        // Alt+↑ moves the discount above the item, Alt+↓ puts it back.
        await page.keyboard.press("Alt+ArrowUp");
        await expect(page.getByLabel("Row 1 description", {exact: true})).toHaveValue("Discount");
        await page.keyboard.press("Alt+ArrowDown");
        await expect(page.getByLabel("Row 1 description", {exact: true})).toHaveValue("Senior architect hours");

        // Cmd/Ctrl+S saves from the cell.
        await expect(page.getByTestId("editor-saved")).toHaveText("Unsaved changes");
        await page.keyboard.press("ControlOrMeta+s");
        await expect(page.getByTestId("editor-saved")).toHaveText("Saved");
        expect(read(file)).toMatch(/^status: draft$/m);
        expect(read(file)).toMatch(/\*\*Total due\*\*\s+\| \*\*\$7,200\.00\*\*/);

        // The sheet in the editor lists its keys and the bill table's.
        await page.keyboard.press("Escape");
        await page.keyboard.press("?");
        const sheet = page.getByTestId("key-help");
        await expect(sheet.getByRole("heading", {level: 4})).toContainText(["Global", "Go to", "Invoice", "Invoice editor", "Bill table"]);
        await expect(sheet).toContainText("Add a subtotal row");
        await page.keyboard.press("Escape");
        await expect(sheet).toHaveCount(0);

        // Shift+S: Send…, its dialog focused on Cancel; Tab to Mark as sent, Enter.
        await page.keyboard.press("S");
        const confirm = page.getByTestId("confirm-send");
        await expect(confirm).toContainText(`Send invoice ${number}?`);
        await expect(confirm.getByRole("button", {name: "Cancel"})).toBeFocused();
        // The dialog blinds the page: Shift+V doesn't open Void… behind it.
        await page.keyboard.press("V");
        await expect(page.getByTestId("confirm-void")).toHaveCount(0);
        await tabTo(page, page.getByTestId("confirm-send-confirm"));
        await page.keyboard.press("Enter");
        await expect(page.getByTestId("editor-notice")).toContainText(`Invoice ${number} is sent.`);
        await expect(page.getByTestId("editor-status")).toHaveText("Sent");
        expect(read(file)).toMatch(/^status: sent$/m);
        expect(read("books.journal")).toContain(`invoice: ${number}, document: ${file}`);

        // Shift+P: the PDF (a sent, unchanged invoice exports at once).
        // Wait for the panel's delivery to be the sent one (it re-fetches after
        // Send): a draft's export would ask "Mark as sent?" instead, as a click
        // would. A sent invoice's button reads "Export PDF", a draft's "Export PDF…".
        const panel = page.getByTestId("send-panel");
        await expect(panel.getByTestId("send-sent")).toBeVisible();
        await expect(panel.getByTestId("send-export")).toHaveText("Export PDF");
        await expect(panel.getByTestId("send-export")).toBeEnabled();
        await page.keyboard.press("P");
        const pdfName = `invoices/pdf/2026-11-17 Fabrikam Invoice ${number}.pdf`;
        await expect(panel.getByTestId("send-exported")).toContainText(pdfName, {timeout: 60_000});
        const pdf = resolve(BOOKS, pdfName);
        expect(readFileSync(pdf).subarray(0, 5).toString("latin1")).toBe("%PDF-");
        const text = execFileSync("pdftotext", ["-layout", pdf, "-"], {encoding: "utf8"});
        expect(text).toContain("Senior architect hours");
        expect(text).toMatch(/\$7,200\.00/);
    });
});
