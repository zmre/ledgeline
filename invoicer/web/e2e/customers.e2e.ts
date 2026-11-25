// Customers end to end (invoicer PLAN 4b, Phase 4 feedback J1–J12): the
// list with what each owes and the sidebar's counts, a customer's read-only
// page (its open and other invoices, contracts by name, the notes linking it,
// each opening inside the app), creating a customer, editing it (only what
// changed is written; unknown keys and the body stay), adding a contract from
// the books folder, the Save / Discard / Cancel prompt on leaving, a narrow
// window, and a save refused because the note changed on disk meanwhile.
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
    await page.goto(`/app/invoices#token=${API_TOKEN}`);
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
        // The sidebar counts (J11): six customers; Contoso's 107 overdue.
        await expect(page.getByTestId("sidebar-customers-count")).toHaveText("6 customers");
        await expect(page.getByTestId("sidebar-invoices-overdue")).toBeVisible();
        // Exclusive invoice counts (H17): 106 and 107 overdue; 105 open, not yet due.
        await expect(page.getByTestId("sidebar-invoices-overdue")).toHaveText("2 overdue");
        await expect(page.getByTestId("sidebar-invoices-count")).toHaveText("1 open, not yet due");
        await expect(page.getByTestId("sidebar-invoices")).toHaveAttribute("title", /1 draft, not counted/);
        await expect(page.getByTestId("sidebar-books")).not.toContainText("documents");
    });

    test("opens a customer read-only: its invoices, account, contracts and links", async ({page}) => {
        await openCustomers(page);
        // A click anywhere on the row opens it (F4).
        await row(page, "customers/Customer X.md").locator("td").nth(2).click();
        await expect(page).toHaveURL(/\/app\/customers\/customers\/Customer%20X$/);
        await expect(page.getByTestId("customer-heading")).toHaveText("Customer X");
        await expect(page.getByTestId("customer-page")).toHaveAttribute("data-mode", "view");
        await expect(page.getByTestId("account-balance")).toHaveText("$8,350.00");
        // Open and Other, never overlapping (H18): 105 open; 108 (draft) and 104 (paid) other.
        const open = page.getByTestId("customer-open-invoice-row");
        await expect(open).toHaveCount(1);
        await expect(open.nth(0)).toHaveAttribute("data-number", "105");
        await expect(open.nth(0).getByTestId("status-badge")).toHaveText("Open");
        const other = page.getByTestId("customer-other-invoice-row");
        await expect(other).toHaveCount(2);
        await expect(other.nth(0)).toHaveAttribute("data-number", "108");
        await expect(page.locator('[data-testid="customer-other-invoice-row"][data-number="104"]').getByTestId("status-badge")).toHaveText("Paid");
        // The view: the note's fields, no form and no MBR frame.
        await expect(page.getByTestId("view-emails")).toContainText("accounts_receivable@customerx.com");
        await expect(page.getByTestId("view-phones")).toContainText("+1 (303) 555-0142");
        await expect(page.getByTestId("view-portal")).toContainText("Coupa");
        await expect(page.getByTestId("customer-title")).toHaveCount(0);
        await expect(page.getByTestId("note-frame")).toHaveCount(0);
        // Contracts by name, never [[…]] (J6).
        const contracts = page.getByTestId("view-contracts");
        await expect(contracts).toContainText("Customer X MSA 2026");
        await expect(contracts).toContainText("Customer X SOW 3 renewal (signed).pdf");
        await expect(contracts).not.toContainText("[[");
        // What each linking note is (J9): Fred Smith is a person.
        const fred = page.getByTestId("customer-backlink").filter({hasText: "Fred Smith"});
        await expect(fred.getByTestId("backlink-kind")).toHaveText("Person");
        await expect(page.getByTestId("customer-backlinks")).not.toContainText("company");
        // An invoice row opens the invoice.
        await page.locator('[data-testid="customer-open-invoice-row"][data-number="105"]').getByRole("link", {name: "105", exact: true}).click();
        await expect(page).toHaveURL(/\/app\/invoices\/105$/);
    });

    test("links stay inside the app: a customer's page, a note's in-app view, and back", async ({page}) => {
        await open(page);
        await page.goto("/app/customers/customers/Customer%20X");
        await expect(page.getByTestId("customer-heading")).toHaveText("Customer X");
        // A contract note opens in the app's note view, MBR's page framed under the sidebar.
        await page.getByTestId("view-contracts").getByRole("link", {name: "Customer X MSA 2026"}).click();
        await expect(page).toHaveURL(/\/app\/notes\/customers\/contracts\/Customer%20X%20MSA%202026$/);
        await expect(page.getByTestId("note-heading")).toHaveText("Customer X MSA 2026");
        await expect(page.getByTestId("sidebar")).toBeVisible();
        await expect(page.getByTestId("note-frame")).toBeVisible();
        await page.getByTestId("note-back").click();
        await expect(page).toHaveURL(/\/app\/customers\/customers\/Customer%20X$/);
        // A linking customer opens its own page in the app, never MBR's.
        await page.getByTestId("customer-backlinks").getByRole("link", {name: "Fred Smith"}).click();
        await expect(page).toHaveURL(/\/app\/customers\/customers\/Fred%20Smith$/);
        await expect(page.getByTestId("customer-heading")).toHaveText("Fred Smith");
        // Its company by name, linked to the company's page.
        await expect(page.getByTestId("view-company")).not.toContainText("[[");
        await page.getByTestId("view-company").getByRole("link", {name: "Customer X"}).click();
        await expect(page).toHaveURL(/\/app\/customers\/customers\/Customer%20X$/);
    });

    test("Edit shows the form; Cancel returns to the view", async ({page}) => {
        await open(page);
        await page.goto("/app/customers/customers/Customer%20X");
        await page.getByTestId("edit-customer").click();
        await expect(page.getByTestId("customer-page")).toHaveAttribute("data-mode", "edit");
        await expect(page.getByTestId("customer-title")).toHaveValue("Customer X");
        await expect(page.getByTestId("labelled-emails").getByLabel("Emails 1", {exact: true})).toHaveValue("accounts_receivable@customerx.com");
        await expect(page.getByTestId("portal-toggle")).toBeChecked();
        await expect(page.getByTestId("save-customer")).toBeDisabled();
        // Contracts by name in the form too.
        await expect(page.getByTestId("contract-row")).toHaveCount(2);
        await expect(page.getByTestId("contracts")).not.toContainText("[[");
        await page.getByTestId("cancel-edit").click();
        await expect(page.getByTestId("customer-page")).toHaveAttribute("data-mode", "view");
    });

    test("a narrow window: the organization form's fields don't overlap", async ({page}) => {
        await page.setViewportSize({width: 420, height: 900});
        await open(page);
        await page.goto("/app/customers/customers/Customer%20X");
        await page.getByTestId("edit-customer").click();
        const name = await page.getByTestId("customer-title").boundingBox();
        const kind = await page.getByRole("group", {name: "Kind"}).boundingBox();
        expect(name).not.toBeNull();
        expect(kind).not.toBeNull();
        if (name !== null && kind !== null) {
            const apart = name.y + name.height <= kind.y || kind.y + kind.height <= name.y || name.x + name.width <= kind.x || kind.x + kind.width <= name.x;
            expect(apart).toBe(true);
        }
        // Nothing wider than the window (tables scroll inside their own box).
        const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
        expect(overflow).toBeLessThanOrEqual(0);
        await page.getByTestId("cancel-edit").click();
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
        await page.getByTestId("edit-customer").click();

        await page.getByRole("spinbutton", {name: "Terms (net days)"}).fill("30");
        const phones = page.getByTestId("labelled-phones");
        await phones.getByRole("button", {name: "Add phone"}).click();
        await phones.getByLabel("Phones 1", {exact: true}).fill("+1 206 555 0100");
        await expect(page.getByTestId("unsaved")).toBeVisible();
        await page.getByTestId("save-customer").click();
        // Field names, not keys (J10); back to the view.
        await expect(page.getByTestId("customer-saved")).toHaveText("Saved: Phones, Terms.");
        await expect(page.getByTestId("customer-page")).toHaveAttribute("data-mode", "view");

        const after = read(NEW_NOTE);
        expect(after).toBe(
            before.replace("terms: 15\n", "terms: 30\n").replace("account_manager: Dana\n", "account_manager: Dana\nphones:\n  - billing: +1 206 555 0100\n")
        );
        expect(after.endsWith("\nPays by ACH.\n")).toBe(true);
        // The view shows the save.
        await expect(page.getByTestId("view-phones")).toContainText("+1 206 555 0100");
        await expect(page.getByTestId("view-terms")).toContainText("Net 30 days");
    });

    test("adds a contract from the books folder by name", async ({page}) => {
        await open(page);
        await page.goto("/app/customers/customers/Northwind%20Traders");
        await page.getByTestId("edit-customer").click();
        await page.getByTestId("add-contract").click();
        const picker = page.getByTestId("contract-picker");
        await expect(picker).toBeVisible();
        // A browser has no open dialog: the searchable list.
        await expect(picker.getByTestId("contract-choose-file")).toHaveCount(0);
        await picker.getByTestId("contract-search").fill("msa 2026");
        await picker.getByTestId("contract-candidate").filter({hasText: "Customer X MSA 2026"}).click();
        await expect(picker).toHaveCount(0);
        // The row also holds its ✕ remove button.
        await expect(page.getByTestId("contract-row")).toHaveText([/^\s*Customer X MSA 2026\s*✕?\s*$/]);
        await page.getByTestId("save-customer").click();
        await expect(page.getByTestId("customer-saved")).toHaveText("Saved: Contracts.");
        expect(read(NEW_NOTE)).toMatch(/contracts:\n {2}- "?\[\[Customer X MSA 2026\]\]"?\n/);
        await expect(page.getByTestId("view-contracts").getByRole("link", {name: "Customer X MSA 2026"})).toBeVisible();
    });

    test("leaving with unsaved changes asks: Save, Discard or Cancel", async ({page}) => {
        await open(page);
        await page.goto("/app/customers/customers/Northwind%20Traders");
        await page.getByTestId("edit-customer").click();
        const before = read(NEW_NOTE);
        await page.getByRole("spinbutton", {name: "Terms (net days)"}).fill("60");
        await page.getByTestId("sidebar").getByRole("link", {name: "Customers"}).click();
        const dialog = page.getByTestId("unsaved-dialog");
        await expect(dialog).toBeVisible();
        // Cancel: stay, the edit kept.
        await dialog.getByTestId("unsaved-cancel").click();
        await expect(dialog).toHaveCount(0);
        await expect(page).toHaveURL(/Northwind%20Traders$/);
        await expect(page.getByRole("spinbutton", {name: "Terms (net days)"})).toHaveValue("60");
        // Cancel editing asks too; Discard drops the change.
        await page.getByTestId("cancel-edit").click();
        await page.getByTestId("unsaved-dialog").getByTestId("unsaved-discard").click();
        await expect(page.getByTestId("customer-page")).toHaveAttribute("data-mode", "view");
        expect(read(NEW_NOTE)).toBe(before);
        // Save, then go.
        await page.getByTestId("edit-customer").click();
        await page.getByRole("spinbutton", {name: "Terms (net days)"}).fill("45");
        await page.getByTestId("sidebar").getByRole("link", {name: "Customers"}).click();
        await page.getByTestId("unsaved-dialog").getByTestId("unsaved-save").click();
        await expect(page).toHaveURL(/\/app\/customers$/);
        expect(read(NEW_NOTE)).toContain("terms: 45\n");
    });

    test("never clobbers an outside edit: it says so, and can apply the change over it", async ({page}) => {
        await open(page);
        await page.goto("/app/customers/customers/Northwind%20Traders");
        await expect(page.getByTestId("customer-heading")).toHaveText("Northwind Traders");
        await page.getByTestId("edit-customer").click();
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
        await expect(page.getByTestId("customer-saved")).toContainText("Revenue account");
        const merged = read(NEW_NOTE);
        expect(merged).toContain("revenue_account: revenue:consulting\n");
        expect(merged).toContain("Pays by wire.");
    });
});
