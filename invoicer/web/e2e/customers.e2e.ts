// Customers end to end (invoicer PLAN 4b): the list with what each owes, a
// customer's page (its invoices, backlinked from `customer:`, and MBR's
// contact card), creating a customer, editing it (only what changed is
// written; unknown keys and the body stay), and a save refused because the
// note changed on disk meanwhile.
//
// Same harness as invoices.e2e.ts: `--server` on :5199 over a scratch copy of
// invoicer/fixtures/accrual, the SPA's `vite preview` on :4273, the clock at
// 2026-11-17. Fixture facts: six customers; Contoso owes $2,000.00 on 107,
// overdue; Customer X has 104 (paid), 105 (open, $8,350.00) and 108 (draft).
//
// The specs run in order (they share the customer they create) and remove
// it at the end, so the other spec files see the fixture's six customers.
// This file runs first, as its own project (playwright.config.ts), while the
// balances are the fixture's: invoices.e2e.ts sends 108 and editor.e2e.ts
// sends 2005, both Customer X's.
//
// Written in 4b and NOT run by the agent that wrote it (agents never run
// Playwright); the lead runs it.

import {existsSync, readFileSync, rmSync, writeFileSync} from "node:fs";
import {resolve} from "node:path";
import {expect, test, type Page} from "@playwright/test";
import {API_TOKEN, COPY} from "../playwright.config";

const FIXED_NOW = new Date(2026, 10, 17, 12, 0, 0);
const BOOKS = resolve(import.meta.dirname, "..", COPY);
const NEW_NOTE = "customers/Northwind Traders.md";

const read = (file: string): string => readFileSync(resolve(BOOKS, file), "utf8");

async function open(page: Page): Promise<void> {
    await page.clock.setFixedTime(FIXED_NOW);
    await page.goto(`/app/#token=${API_TOKEN}`);
    await expect(page).toHaveURL(/\/app\/invoices$/);
}

async function openCustomers(page: Page): Promise<void> {
    await open(page);
    await page.getByTestId("sidebar").getByRole("link", {name: "Customers"}).click();
    await expect(page).toHaveURL(/\/app\/customers$/);
}

const row = (page: Page, path: string) => page.locator(`[data-testid="customer-row"][data-path="${path}"]`);

test.describe.configure({mode: "serial"});

test.describe("customers over the accrual fixture (:5199)", () => {
    test.afterAll(() => {
        rmSync(resolve(BOOKS, NEW_NOTE), {force: true});
    });

    test("lists every customer with what it owes", async ({page}) => {
        await openCustomers(page);
        await expect(page.getByRole("heading", {name: "Customers"})).toBeVisible();
        await expect(page.getByTestId("sidebar").getByRole("link", {name: "Customers"})).toHaveAttribute("aria-current", "page");
        await expect(page.getByTestId("customer-row")).toHaveCount(6);
        const contoso = row(page, "customers/Contoso.md");
        await expect(contoso).toContainText("$2,000.00");
        await expect(contoso.getByTestId("overdue-badge")).toBeVisible();
        await expect(contoso).toContainText("$1,000.00 on 2026-11-05");
        await expect(row(page, "customers/Fabrikam.md").getByTestId("overdue-badge")).toHaveCount(0);
        await expect(row(page, "customers/Fred Smith.md")).toContainText("bills Customer X");
    });

    test("opens a customer: its invoices, account and contact card", async ({page}) => {
        await openCustomers(page);
        // A click anywhere on the row opens it (F4).
        await row(page, "customers/Customer X.md").locator("td").nth(2).click();
        await expect(page).toHaveURL(/\/app\/customers\/customers\/Customer%20X$/);
        await expect(page.getByTestId("customer-heading")).toHaveText("Customer X");
        await expect(page.getByTestId("account-balance")).toHaveText("$8,350.00");
        const invoices = page.getByTestId("customer-invoice-row");
        await expect(invoices).toHaveCount(3);
        await expect(invoices.nth(0)).toHaveAttribute("data-number", "108");
        await expect(page.locator('[data-testid="customer-invoice-row"][data-number="105"]').getByTestId("status-badge")).toHaveText("Open");
        await expect(page.locator('[data-testid="customer-invoice-row"][data-number="104"]').getByTestId("status-badge")).toHaveText("Paid");
        // The form holds the note's fields.
        await expect(page.getByTestId("customer-title")).toHaveValue("Customer X");
        await expect(page.getByTestId("labelled-emails").getByLabel("Emails 1", {exact: true})).toHaveValue("accounts_receivable@customerx.com");
        await expect(page.getByTestId("portal-toggle")).toBeChecked();
        await expect(page.getByTestId("save-customer")).toBeDisabled();
        // MBR's contact card for the saved note, with its phone numbers.
        const card = page.frameLocator('[data-testid="note-frame"]');
        await expect(card.getByText("+1 (303) 555-0142")).toBeVisible();
        // An invoice row opens the invoice.
        await page.locator('[data-testid="customer-invoice-row"][data-number="105"]').getByRole("link", {name: "105"}).click();
        await expect(page).toHaveURL(/\/app\/invoices\/105$/);
    });

    test("creates a customer note in customers/, tagged customer", async ({page}) => {
        expect(existsSync(resolve(BOOKS, NEW_NOTE))).toBe(false);
        await openCustomers(page);
        await page.getByTestId("new-customer").click();
        await expect(page).toHaveURL(/\/app\/customers\/new$/);
        await page.getByTestId("customer-title").fill("Northwind Traders");
        await page.getByTestId("labelled-emails").getByLabel("Emails 1", {exact: true}).fill("ap@northwind.example");
        await page.getByTestId("labelled-addresses").getByLabel("Addresses 1", {exact: true}).fill("1 Harbor Rd\nSeattle, WA 98101");
        await page.getByRole("spinbutton", {name: "Terms (net days)"}).fill("15");
        await page.getByTestId("create-customer").click();
        await expect(page).toHaveURL(/\/app\/customers\/customers\/Northwind%20Traders$/);
        await expect(page.getByTestId("customer-heading")).toHaveText("Northwind Traders");
        await expect(page.getByTestId("customer-no-invoices")).toBeVisible();

        const text = read(NEW_NOTE);
        expect(text).toContain("title: Northwind Traders\ntype: organization\ntags:\n  - customer\n");
        expect(text).toContain("emails:\n  - billing: ap@northwind.example\n");
        expect(text).toContain("addresses:\n  - billing: |\n      1 Harbor Rd\n      Seattle, WA 98101\n");
        expect(text).toContain("terms: 15\n");

        await page.goto("/app/customers");
        await expect(page.getByTestId("customer-row")).toHaveCount(7);
        // The name is taken now.
        await page.getByTestId("new-customer").click();
        await page.getByTestId("customer-title").fill("northwind traders");
        await page.getByTestId("create-customer").click();
        await expect(page.getByTestId("customer-error")).toContainText("already exists");
    });

    test("edits only what changed, keeping the note's other keys and body", async ({page}) => {
        // Someone adds a key and a body by hand; the page (clean) shows it live.
        writeFileSync(resolve(BOOKS, NEW_NOTE), read(NEW_NOTE).replace("terms: 15\n", "terms: 15\naccount_manager: Dana\n") + "\nPays by ACH.\n");
        await open(page);
        await page.goto("/app/customers/customers/Northwind%20Traders");
        await expect(page.getByTestId("customer-heading")).toHaveText("Northwind Traders");
        const before = read(NEW_NOTE);

        await page.getByRole("spinbutton", {name: "Terms (net days)"}).fill("30");
        const phones = page.getByTestId("labelled-phones");
        await phones.getByRole("button", {name: "Add phone"}).click();
        await phones.getByLabel("Phones 1", {exact: true}).fill("+1 206 555 0100");
        await expect(page.getByTestId("unsaved")).toBeVisible();
        await page.getByTestId("save-customer").click();
        await expect(page.getByTestId("customer-saved")).toContainText("Saved");
        await expect(page.getByTestId("save-customer")).toBeDisabled();

        const after = read(NEW_NOTE);
        expect(after).toBe(
            before.replace("terms: 15\n", "terms: 30\n").replace("account_manager: Dana\n", "account_manager: Dana\nphones:\n  - billing: +1 206 555 0100\n")
        );
        expect(after.endsWith("\nPays by ACH.\n")).toBe(true);
        // MBR's card shows the save.
        await expect(page.frameLocator('[data-testid="note-frame"]').getByText("+1 206 555 0100")).toBeVisible();
    });

    test("never clobbers an outside edit: it says so, and can apply the change over it", async ({page}) => {
        await open(page);
        await page.goto("/app/customers/customers/Northwind%20Traders");
        await expect(page.getByTestId("customer-heading")).toHaveText("Northwind Traders");
        await page.getByRole("textbox", {name: /^Revenue account/}).fill("revenue:consulting");

        // Meanwhile, the note changes on disk.
        const outside = read(NEW_NOTE).replace("Pays by ACH.", "Pays by wire.");
        writeFileSync(resolve(BOOKS, NEW_NOTE), outside);
        await expect(page.getByTestId("customer-stale")).toBeVisible();
        // The form keeps the unsaved change.
        await expect(page.getByRole("textbox", {name: /^Revenue account/})).toHaveValue("revenue:consulting");

        await page.getByTestId("save-customer").click();
        await expect(page.getByTestId("customer-save-error")).toContainText("changed on disk");
        expect(read(NEW_NOTE)).toBe(outside);

        await page.getByTestId("conflict-apply").click();
        await expect(page.getByTestId("customer-saved")).toContainText("revenue_account");
        const merged = read(NEW_NOTE);
        expect(merged).toContain("revenue_account: revenue:consulting\n");
        expect(merged).toContain("Pays by wire.");
    });
});
