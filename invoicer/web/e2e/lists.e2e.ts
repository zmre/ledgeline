// The lists (invoicer PLAN 5b): filtering and sorting the invoice and
// customer lists, all of it in the URL (a reload keeps it, Back and Forward
// step through it, a link like the dashboard's opens it), and a customer's
// open POs added, edited and closed on its page, then suggested in the
// invoice editor.
//
// Its own Playwright project, after `library` (playwright.config.ts): it
// reads the shared scratch copy as the earlier specs leave it, so it only
// relies on facts they don't change, at the clock 2026-11-17
// (invoicer/fixtures/README.md):
//   - 101, 102 and 104 are paid; 103 is void; 106 (due 08-31, 78 days late)
//     and 107 (due 11-14, 3 days late) are overdue; 105 is open, not yet due.
//     invoices.e2e.ts sends 108 and editor.e2e.ts adds 2005, both open and
//     not yet due, so neither is paid, overdue nor void;
//   - Contoso, Tailspin and Wingtip have something overdue; Customer X owes
//     the most; Fred Smith (a person) owes nothing.
// The PO test puts Customer X's `open_pos` back as it found it (the
// fixture's one PO), and checks the rest of the note never moved.
//
// Written in 5b and NOT run by the agent that wrote it (agents never run
// Playwright); the lead runs it.

import {readFileSync} from "node:fs";
import {resolve} from "node:path";
import {expect, test, type Page} from "@playwright/test";
import {API_TOKEN, COPY} from "../playwright.config";

const FIXED_NOW = new Date(2026, 10, 17, 12, 0, 0); // local 2026-11-17, the fixture's clock
const BOOKS = resolve(import.meta.dirname, "..", COPY);
const CUSTOMER_X = "customers/Customer X.md";

const read = (file: string): string => readFileSync(resolve(BOOKS, file), "utf8");

/** Open the app the way `--server` prints it, then go to `path` (under /app). */
async function open(page: Page, path = "/app/invoices"): Promise<void> {
    await page.clock.setFixedTime(FIXED_NOW);
    await page.goto(`/app/#token=${API_TOKEN}`);
    await expect(page).toHaveURL(/\/app\/invoices$/);
    if (path !== "/app/invoices") await page.goto(path);
}

/** The invoice numbers listed, top to bottom. */
const numbers = (page: Page) => page.getByTestId("invoice-row").evaluateAll((rows) => rows.map((r) => (r as HTMLElement).dataset.number ?? ""));
/** The customer note paths listed, top to bottom. */
const customerPaths = (page: Page) => page.getByTestId("customer-row").evaluateAll((rows) => rows.map((r) => (r as HTMLElement).dataset.path ?? ""));

test.describe.configure({mode: "serial"});

test.describe("the invoice list's filters and sort, in the URL (:5199)", () => {
    test("a status filter is in the URL, survives a reload and steps with Back and Forward", async ({page}) => {
        await open(page);
        const all = await numbers(page);
        expect(all.length).toBeGreaterThanOrEqual(8);
        await expect(page.getByTestId("invoice-count")).toHaveText(`${all.length} invoices`);

        await page.getByTestId("invoice-filter-status").selectOption("paid");
        await expect(page).toHaveURL(/\/app\/invoices\?status=paid$/);
        await expect.poll(() => numbers(page)).toEqual(["104", "102", "101"]);
        await expect(page.getByTestId("invoice-count")).toHaveText("3 invoices match");

        await page.reload();
        await expect(page.getByTestId("invoice-filter-status")).toHaveValue("paid");
        await expect.poll(() => numbers(page)).toEqual(["104", "102", "101"]);

        await page.goBack();
        await expect(page).toHaveURL(/\/app\/invoices$/);
        await expect.poll(() => numbers(page)).toEqual(all);
        await expect(page.getByTestId("invoice-filter-status")).toHaveValue("");
        await page.goForward();
        await expect(page).toHaveURL(/\/app\/invoices\?status=paid$/);
        await expect.poll(() => numbers(page)).toEqual(["104", "102", "101"]);
    });

    test("links in, as the dashboard makes them: overdue, an aging bucket, a customer, a date range", async ({page}) => {
        await open(page, "/app/invoices?status=overdue");
        await expect.poll(() => numbers(page)).toEqual(["107", "106"]);
        // Overdue rows stand out, not only their badge.
        await expect(page.locator('[data-testid="invoice-row"][data-number="106"]')).toHaveAttribute("data-overdue", "true");

        await page.goto("/app/invoices?aged=61-90");
        await expect.poll(() => numbers(page)).toEqual(["106"]);
        await expect(page.getByTestId("invoice-filter-aged")).toHaveValue("61-90");

        await page.goto("/app/invoices?customer=Contoso");
        await expect.poll(() => numbers(page)).toEqual(["107", "101"]);
        await expect(page.getByRole("combobox", {name: "Customer", exact: true})).toHaveValue("Contoso");

        await page.goto("/app/invoices?from=2026-10-01&to=2026-10-31");
        await expect.poll(() => numbers(page)).toEqual(["107", "104", "103"]);

        // A value the server wouldn't read is dropped: the whole list, no error.
        await page.goto("/app/invoices?status=unpaid&aged=old");
        await expect(page.getByTestId("invoice-row").first()).toBeVisible();
        await expect(page.getByTestId("invoices-error")).toHaveCount(0);
    });

    test("sorts from the headers, by pointer and keyboard, with aria-sort", async ({page}) => {
        await open(page);
        const numberHeader = page.getByRole("columnheader", {name: "Number"});
        // The default order is Date, newest first.
        await expect(page.getByRole("columnheader", {name: "Date"})).toHaveAttribute("aria-sort", "descending");
        await page.getByTestId("sort-number").click();
        await expect(page).toHaveURL(/\/app\/invoices\?sort=number&dir=desc$/);
        await expect(numberHeader).toHaveAttribute("aria-sort", "descending");
        await expect(page.getByRole("columnheader", {name: "Date"})).toHaveAttribute("aria-sort", "none");
        await page.getByTestId("sort-number").click();
        await expect(page).toHaveURL(/sort=number&dir=asc$/);
        await expect.poll(async () => (await numbers(page))[0]).toBe("101");

        await page.getByTestId("sort-total").focus();
        await page.keyboard.press("Enter");
        await expect(page).toHaveURL(/sort=total&dir=desc$/);
        await expect(page.getByRole("columnheader", {name: "Total"})).toHaveAttribute("aria-sort", "descending");
        await page.reload();
        await expect(page.getByRole("columnheader", {name: "Total"})).toHaveAttribute("aria-sort", "descending");
    });

    test("searches as you type, and Enter jumps to an invoice", async ({page}) => {
        await open(page);
        const search = page.getByTestId("invoice-search");
        await search.fill("Contoso");
        await expect(page).toHaveURL(/\/app\/invoices\?q=Contoso$/);
        await expect.poll(() => numbers(page)).toEqual(["107", "101"]);
        // Typing replaces the history step: Back leaves the list.
        await search.fill("106");
        await expect(page).toHaveURL(/\/app\/invoices\?q=106$/);
        await search.press("Enter");
        await expect(page).toHaveURL(/\/app\/invoices\/106$/);
        await expect(page.getByTestId("invoice-heading")).toHaveText("Invoice 106");
        await page.goBack();
        await expect(page).toHaveURL(/\/app\/invoices\?q=106$/);
        await expect(page.getByTestId("invoice-search")).toHaveValue("106");
    });

    test("says when nothing matches, and clears the filters but not the sort", async ({page}) => {
        await open(page, "/app/invoices?status=void&q=zzz-no-such&sort=number&dir=asc");
        await expect(page.getByTestId("invoice-list-no-match")).toContainText("No invoices match these filters.");
        await expect(page.getByTestId("invoice-list-empty")).toHaveCount(0);
        await expect(page.getByTestId("invoices-new")).toBeVisible();
        await page.getByTestId("invoice-list-clear").click();
        await expect(page).toHaveURL(/\/app\/invoices\?sort=number&dir=asc$/);
        await expect(page.getByTestId("invoice-search")).toHaveValue("");
        await expect.poll(async () => (await numbers(page))[0]).toBe("101");
        await expect(page.getByTestId("invoice-filters-clear")).toHaveCount(0);
    });

    test("a narrow window: the filters wrap and the table scrolls in its own box", async ({page}) => {
        await page.setViewportSize({width: 420, height: 800});
        await open(page);
        const search = await page.getByTestId("invoice-search").boundingBox();
        const status = await page.getByTestId("invoice-filter-status").boundingBox();
        expect(search).not.toBeNull();
        expect(status).not.toBeNull();
        // The search has its own row; the status sits below it, not on top.
        expect(status!.y).toBeGreaterThanOrEqual(search!.y + search!.height);
        // The table scrolls in its own box; the page itself never scrolls sideways.
        await expect(page.getByTestId("invoice-list").locator("xpath=..")).toHaveClass(/overflow-x-auto/);
        const pageScrolls = await page.evaluate(() => document.documentElement.scrollWidth > document.documentElement.clientWidth);
        expect(pageScrolls).toBe(false);
    });
});

test.describe("the customer list's filters and sort, in the URL (:5199)", () => {
    test("overdue, owes and kind filter; Owed sorts; the URL keeps it all", async ({page}) => {
        await open(page, "/app/customers?overdue=true");
        await expect.poll(() => customerPaths(page)).toEqual(["customers/Contoso.md", "customers/Tailspin.md", "customers/Wingtip.md"]);
        await expect(page.getByTestId("customer-filter-overdue")).toHaveAttribute("aria-pressed", "true");
        await expect(page.getByTestId("customer-count")).toHaveText("3 customers match");

        await page.getByTestId("customer-filter-overdue").click();
        await expect(page).toHaveURL(/\/app\/customers$/);
        await page.getByTestId("sort-owed").click();
        await expect(page).toHaveURL(/\/app\/customers\?sort=owed&dir=desc$/);
        await expect(page.getByRole("columnheader", {name: "Owed"})).toHaveAttribute("aria-sort", "descending");
        await expect.poll(async () => (await customerPaths(page))[0]).toBe(CUSTOMER_X);

        await page.getByTestId("customer-filter-owes").click();
        await expect(page).toHaveURL(/\/app\/customers\?owes=true&sort=owed&dir=desc$/);
        await expect(page.locator('[data-testid="customer-row"][data-path="customers/Fred Smith.md"]')).toHaveCount(0);

        await page.getByTestId("customer-filter-kind").selectOption("person");
        await expect(page).toHaveURL(/kind=person/);
        await expect(page.getByTestId("customer-list-no-match")).toBeVisible();
        await page.getByTestId("customer-list-clear").click();
        await expect(page).toHaveURL(/\/app\/customers\?sort=owed&dir=desc$/);

        await page.reload();
        await expect(page.getByRole("columnheader", {name: "Owed"})).toHaveAttribute("aria-sort", "descending");
        await page.goBack();
        await expect(page).toHaveURL(/kind=person/);
        await expect(page.getByTestId("customer-filter-kind")).toHaveValue("person");
    });

    test("a search for one customer, and Enter opens it", async ({page}) => {
        await open(page, "/app/customers");
        await page.getByTestId("customer-search").fill("fred");
        await expect(page).toHaveURL(/\/app\/customers\?q=fred$/);
        await expect.poll(() => customerPaths(page)).toEqual(["customers/Fred Smith.md"]);
        await page.getByTestId("customer-search").press("Enter");
        await expect(page.getByTestId("customer-heading")).toHaveText("Fred Smith");
    });
});

test.describe("a customer's open POs (:5199)", () => {
    test("add, edit and close a PO on the page, and the editor suggests it meanwhile", async ({page}) => {
        const before = read(CUSTOMER_X);
        /** The note without its `open_pos:` block: what must never move. */
        const outside = (text: string): string => text.replace(/^open_pos:\n(?:[ ]+.*\n)+/m, "");

        await open(page, "/app/customers/customers/Customer%20X");
        const section = page.getByTestId("customer-open-pos");
        await expect(section.locator('[data-testid="po-row"][data-reference="PO #PY75334"]')).toContainText("left of $40,000");

        // Add: Enter in a field saves the row.
        await section.getByTestId("po-add").click();
        await expect(section.getByTestId("po-reference")).toBeFocused();
        await section.getByTestId("po-reference").fill("PO #E2E-1");
        await section.getByTestId("po-amount").fill("$5,000");
        await section.getByTestId("po-amount").press("Enter");
        const added = section.locator('[data-testid="po-row"][data-reference="PO #E2E-1"]');
        await expect(added).toContainText("$5,000.00 left of $5,000");
        await expect(page.getByTestId("customer-saved")).toHaveText("Saved: Open POs.");
        expect(read(CUSTOMER_X)).toContain("PO #E2E-1");
        expect(outside(read(CUSTOMER_X))).toBe(outside(before));

        // Edit it.
        await section.getByRole("button", {name: "Edit PO #E2E-1", exact: true}).click();
        await section.getByTestId("po-amount").fill("$6,000");
        await section.getByTestId("po-save").click();
        await expect(added).toContainText("$6,000.00 left of $6,000");

        // The invoice editor offers it as a reference (nothing is changed there).
        await page.goto("/app/invoices/105/edit");
        await expect(page.getByTestId("editor-heading")).toHaveText("Invoice 105");
        await page.getByTestId("header-reference").click();
        const listbox = page.getByRole("listbox");
        await expect(listbox.getByRole("option").filter({hasText: "PO #E2E-1"})).toBeVisible();
        await expect(listbox.getByRole("option").filter({hasText: "PO #PY75334"})).toBeVisible();
        await page.keyboard.press("Escape");

        // Close it: asked first, then it comes off the note.
        await page.goto("/app/customers/customers/Customer%20X");
        await page.getByTestId("customer-open-pos").getByRole("button", {name: "Close PO #E2E-1", exact: true}).click();
        await expect(page.getByTestId("po-close-dialog")).toContainText("keep their reference");
        await page.getByTestId("po-close-dialog-confirm").click();
        await expect(page.locator('[data-testid="po-row"][data-reference="PO #E2E-1"]')).toHaveCount(0);
        const after = read(CUSTOMER_X);
        expect(after).not.toContain("E2E-1");
        expect(after).toContain("PO #PY75334");
        expect(outside(after)).toBe(outside(before));
    });

    test("the form edits POs too, saved with the rest", async ({page}) => {
        const before = read(CUSTOMER_X);
        await open(page, "/app/customers/customers/Customer%20X");
        await page.getByTestId("edit-customer").click();
        const form = page.getByTestId("form-open-pos");
        await form.getByTestId("form-po-add").click();
        await form.getByTestId("form-po-reference").last().fill("PO #E2E-2");
        await page.getByTestId("save-customer").click();
        await expect(page.getByTestId("customer-page")).toHaveAttribute("data-mode", "view");
        await expect(page.locator('[data-testid="po-row"][data-reference="PO #E2E-2"]')).toBeVisible();

        await page.getByTestId("edit-customer").click();
        await page.getByTestId("form-open-pos").getByRole("button", {name: "Close PO #E2E-2", exact: true}).click();
        await page.getByTestId("save-customer").click();
        await expect(page.getByTestId("customer-page")).toHaveAttribute("data-mode", "view");
        await expect(page.locator('[data-testid="po-row"][data-reference="PO #E2E-2"]')).toHaveCount(0);
        expect(read(CUSTOMER_X).replace(/^open_pos:\n(?:[ ]+.*\n)+/m, "")).toBe(before.replace(/^open_pos:\n(?:[ ]+.*\n)+/m, ""));
    });
});
