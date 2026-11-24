// The invoice editor end to end (invoicer PLAN 4a's gate): create an invoice
// with a discount, a subtotal and a tax; save; send; check the journal text;
// export the PDF. Then an outside edit while editing: the conflict notice.
//
// Against `ledgeline-invoicer --server` over the scratch copy of
// invoicer/fixtures/accrual (playwright.config.ts). It runs after
// invoices.e2e.ts (a project dependency there), since it adds invoice 2005 and
// that spec counts eight. Fixture facts at the clock, 2026-11-17:
//   - the next number is 2005 (plan 45's booked 2001–2004 count, README);
//   - the catalog's ARCH is "Senior architect hours", h, $800, revenue:consulting;
//   - the journal declares no sales-tax account: `liabilities:sales-tax` is
//     used undeclared (the server books it; hledger allows it);
//   - invoices.e2e.ts has sent 108 by now; the conflict spec puts its file back.
//
// Written in 4a and NOT run by the agent that wrote it (agents never run
// Playwright); the lead runs it.

import {execFileSync} from "node:child_process";
import {readFileSync, writeFileSync} from "node:fs";
import {resolve} from "node:path";
import {expect, test, type Page} from "@playwright/test";
import {API_TOKEN, COPY} from "../playwright.config";

const FIXED_NOW = new Date(2026, 10, 17, 12, 0, 0); // local 2026-11-17, the fixture's clock
const BOOKS = resolve(import.meta.dirname, "..", COPY);
const NEW_INVOICE = "invoices/2026-11-17 Customer X Invoice 2005.md";
const INVOICE_108 = "invoices/2026-11-17 Customer X Invoice 108.md";

const read = (file: string): string => readFileSync(resolve(BOOKS, file), "utf8");

async function open(page: Page): Promise<void> {
    await page.clock.setFixedTime(FIXED_NOW);
    await page.goto(`/app/#token=${API_TOKEN}`);
    await expect(page).toHaveURL(/\/app\/invoices$/);
}

/** The computed row amounts the grid shows, once they are the server's (not the display-only estimate); the total is the footer's. */
async function serverAmounts(page: Page): Promise<string[]> {
    const amounts = page.getByTestId("grid-amount");
    await expect(amounts.and(page.locator('[data-optimistic="true"]'))).toHaveCount(0);
    await expect(page.getByTestId("grid-total")).toHaveAttribute("data-optimistic", "false");
    return (await amounts.allTextContents()).map((t) => t.trim());
}

test.describe.configure({mode: "serial"});

test.describe("the invoice editor over the accrual fixture (:5199)", () => {
    test("creates an invoice with a discount, a subtotal and a tax; saves, sends and exports it", async ({page}) => {
        test.setTimeout(120_000);
        await open(page);

        // New invoice, from the sidebar's New button (H20).
        await page.getByTestId("sidebar").getByRole("link", {name: "New invoice", exact: true}).click();
        await expect(page).toHaveURL(/\/app\/new$/);
        // The customer list opens without typing (a click shows them all); the field shows the name, the file gets the link.
        const customer = page.getByTestId("new-customer");
        await customer.click();
        await expect(page.getByRole("listbox", {name: "Customer suggestions"}).getByRole("option")).not.toHaveCount(0);
        await customer.fill("Customer");
        await page.getByRole("option", {name: /^Customer X/}).click();
        await expect(customer).toHaveValue("Customer X");
        await expect(page.getByTestId("new-invoice")).toContainText("number 2005");
        // Why 2005 when the highest file is 108 (H19).
        await expect(page.getByTestId("new-number-reason")).toContainText("next after 2004, booked in the journal");
        await page.getByTestId("new-create").click();
        await expect(page).toHaveURL(/\/app\/invoices\/2005\/edit$/);
        await expect(page.getByTestId("editor-heading")).toHaveText("Invoice 2005");
        await expect(page.getByTestId("editor-status")).toHaveText("draft");
        await expect(page.getByTestId("header-customer")).toHaveValue("Customer X");
        // A new draft's table is only its total: the footer, not a row.
        await expect(page.getByTestId("grid-row")).toHaveCount(0);
        await expect(page.getByTestId("grid-total-label")).toHaveValue("Total due");

        // Customer X's portal requires a reference (Send refuses without
        // one): pick the customer's open PO from the suggestions.
        const reference = page.getByTestId("header-reference");
        await reference.fill("PO");
        await page.getByRole("option", {name: /^PO #PY75334/}).click();
        await expect(reference).toHaveValue("PO #PY75334");

        // An item from the catalog: ARCH, 10 hours.
        await page.getByTestId("add-item").click();
        const description = page.getByLabel("Row 1 description", {exact: true});
        await description.fill("Senior");
        await page.getByRole("option", {name: /^Senior architect hours/}).click();
        await page.getByLabel("Row 1 quantity").fill("10");
        // Each new row goes at the bottom, above the total: a discount on the
        // item, a subtotal, a tax on the subtotal. Focus is in row 1 when they
        // are added, and that doesn't matter.
        await page.getByLabel("Row 1 quantity").focus();
        await page.getByTestId("add-discount-percent").click();
        await page.getByTestId("add-subtotal").click();
        await page.getByTestId("add-tax").click();
        await page.getByLabel("Row 4 rate").fill("8.25%");
        await page.getByLabel("Row 4 account").fill("liabilities:sales-tax");
        // The subtotal is its label and amount, with no fields of its own.
        await expect(page.getByLabel("Row 3 subtotal label")).toHaveValue("Subtotal");
        await expect(page.getByLabel("Row 3 rate")).toHaveCount(0);

        expect(await serverAmounts(page)).toEqual(["$8,000.00", "-$800.00", "$7,200.00", "$594.00"]);
        await expect(page.getByTestId("grid-total")).toHaveText("$7,794.00");
        const bases = page.getByTestId("grid-base");
        await expect(bases.nth(0)).toContainText("-10% of Senior architect hours, $8,000.00");
        await expect(bases.nth(1)).toContainText("8.25% of Subtotal, $7,200.00");
        await expect(page.getByTestId("editor-total")).toHaveText("$7,794.00");
        // The draft's Books panel shows what sending would book.
        await expect(page.getByTestId("would-book-text")).toContainText("assets:accounts receivable");

        // Send and the PDF wait for a save.
        await expect(page.getByTestId("editor-send")).toHaveCount(0);
        await expect(page.getByTestId("send-panel")).toHaveCount(0);
        // Save (⌘S works too); the file holds the table, aligned, amounts written.
        await expect(page.getByTestId("editor-saved")).toHaveText("Unsaved changes");
        await page.getByTestId("editor-save").click();
        await expect(page.getByTestId("editor-saved")).toHaveText("Saved");
        const saved = read(NEW_INVOICE);
        expect(saved).toMatch(/^status: draft$/m);
        expect(saved).toMatch(/^customer: "\[\[Customer X\]\]"$/m);
        expect(saved).toMatch(/\| ARCH\s+\| Senior architect hours\s+\| 10\s+\| h\s+\| \$800\s+\| \$8,000\.00\s+\| revenue:consulting\s+\|/);
        expect(saved).toMatch(/\| Discount\s+\|\s+\|\s+\| -10%\s+\| -\$800\.00\s+\|/);
        expect(saved).toMatch(/\*\*Subtotal\*\*\s+\| \*\*\$7,200\.00\*\*/);
        expect(saved).toMatch(/\| Sales tax\s+\|\s+\|\s+\| 8\.25%\s+\| \$594\.00\s+\| liabilities:sales-tax\s+\|/);
        expect(saved).toMatch(/\*\*Total due\*\*\s+\| \*\*\$7,794\.00\*\*/);
        expect(read("books.journal")).not.toMatch(/invoice: 2005\b/); // drafts never touch the books

        // Send, after the confirmation.
        await page.getByTestId("editor-send").click();
        const confirm = page.getByTestId("confirm-send");
        await expect(confirm).toContainText("Send invoice 2005?");
        await expect(confirm).toContainText("$7,794.00");
        await expect(confirm).toContainText("assets:accounts receivable");
        await page.getByTestId("confirm-send-confirm").click();
        await expect(page.getByTestId("editor-notice")).toContainText("Invoice 2005 is sent.");
        await expect(page.getByTestId("editor-status")).toHaveText("sent");
        expect(read(NEW_INVOICE)).toMatch(/^status: sent$/m);
        const journal = read("books.journal");
        expect(journal).toContain("invoice: 2005, document: invoices/2026-11-17 Customer X Invoice 2005.md, due: 2026-12-17");
        expect(journal).toMatch(/assets:accounts receivable\s+\$7794\.00/);
        expect(journal).toMatch(/revenue:consulting\s+\$-7200\.00/);
        expect(journal).toMatch(/liabilities:sales-tax\s+\$-594\.00/);
        await expect(page.getByTestId("booked-text")).toContainText("invoice: 2005");

        // The PDF, from the Send panel (the same panel the invoice page has).
        const panel = page.getByTestId("send-panel");
        await panel.getByTestId("send-export").click();
        await expect(panel.getByTestId("send-exported")).toContainText("invoices/pdf/2026-11-17 Customer X Invoice 2005.pdf", {timeout: 60_000});
        const pdf = resolve(BOOKS, "invoices/pdf/2026-11-17 Customer X Invoice 2005.pdf");
        expect(readFileSync(pdf).subarray(0, 5).toString("latin1")).toBe("%PDF-");
        const text = execFileSync("pdftotext", ["-layout", pdf, "-"], {encoding: "utf8"});
        expect(text).toContain("Bill to Customer X");
        expect(text).toContain("Senior architect hours");
        expect(text).toMatch(/Total due\s+\$7,794\.00/);
        expect(text).not.toContain("liabilities:sales-tax"); // the books column is hidden
    });

    test("leaving with unsaved edits asks Save / Discard / Cancel; a spreadsheet paste becomes a table", async ({page}) => {
        const original = read(INVOICE_108);
        try {
            await open(page);
            await page.goto("/app/invoices/108/edit");
            await expect(page.getByTestId("editor-heading")).toHaveText("Invoice 108");
            // Its total is the footer: two rows, not three.
            await expect(page.getByTestId("grid-row")).toHaveCount(2);
            await expect(page.getByTestId("grid-total")).toHaveText("$13,600.00");

            // Cells pasted from a spreadsheet land as a markdown table (MBR's field reads it as one).
            const prose = page.getByTestId("prose-before").locator('.ProseMirror[contenteditable="true"]');
            await prose.click();
            await page.keyboard.press("End");
            await prose.evaluate((node) => {
                const data = new DataTransfer();
                data.setData("text/plain", "Milestone\tDue\nDesign\t2026-12-01");
                data.setData("text/html", "<table><tr><td>Milestone</td><td>Due</td></tr><tr><td>Design</td><td>2026-12-01</td></tr></table>");
                node.dispatchEvent(new ClipboardEvent("paste", {clipboardData: data, bubbles: true, cancelable: true}));
            });
            // Milkdown's table block wraps the content table (`table.children`,
            // the one with the table role) in an empty control table.
            await expect(page.getByTestId("prose-before").getByRole("table")).toContainText("Design");
            await page.getByLabel("Row 2 description", {exact: true}).click(); // MBR's field reports on leaving it
            await expect(page.getByTestId("editor-saved")).toHaveText("Unsaved changes");

            // Away by the sidebar: the dialog; Cancel stays, with the edits.
            await page.getByTestId("sidebar").getByRole("link", {name: "Invoices"}).click();
            const dialog = page.getByTestId("unsaved-dialog");
            await expect(dialog).toBeVisible();
            await expect(page.getByTestId("unsaved-cancel")).toBeFocused();
            await page.keyboard.press("Escape");
            await expect(dialog).toHaveCount(0);
            await expect(page).toHaveURL(/\/app\/invoices\/108\/edit$/);
            await expect(page.getByTestId("editor-saved")).toHaveText("Unsaved changes");

            // Discard changes (the button) is clean again; then an edit, and Discard in the dialog leaves without saving.
            await page.getByTestId("editor-discard").click();
            await expect(page.getByTestId("editor-saved")).not.toHaveText("Unsaved changes");
            await expect(page.getByTestId("editor-send")).toHaveCount(0); // 108 is sent by now: Re-book…, not Send…
            await expect(page.getByTestId("editor-void")).toBeVisible();
            await page.getByLabel("Row 2 description", {exact: true}).fill("Not saved");
            await page.getByTestId("sidebar").getByRole("link", {name: "Invoices"}).click();
            await page.getByTestId("unsaved-discard").click();
            await expect(page).toHaveURL(/\/app\/invoices$/);
            expect(read(INVOICE_108)).toBe(original);
        } finally {
            writeFileSync(resolve(BOOKS, INVOICE_108), original);
        }
    });

    test("an outside edit while editing raises the conflict notice; Reload theirs shows it", async ({page}) => {
        const original = read(INVOICE_108);
        try {
            await open(page);
            await page.goto("/app/invoices/108/edit");
            await expect(page.getByTestId("editor-heading")).toHaveText("Invoice 108");
            const description = page.getByLabel("Row 2 description", {exact: true});
            await expect(description).toHaveValue("Senior architect hours");
            await description.fill("Architect hours (mine)");
            await expect(page.getByTestId("editor-saved")).toHaveText("Unsaved changes");

            // Another editor saves the file meanwhile.
            writeFileSync(resolve(BOOKS, INVOICE_108), original.replace("Next month's platform subscription", "Edited elsewhere: platform subscription"));
            const notice = page.getByTestId("conflict-notice");
            await expect(notice).toContainText("changed on disk while you were editing", {timeout: 10_000});
            // Nothing was overwritten either way.
            expect(read(INVOICE_108)).toContain("Edited elsewhere");
            await expect(description).toHaveValue("Architect hours (mine)");

            await page.getByTestId("conflict-reload").click();
            await expect(notice).toHaveCount(0);
            await expect(description).toHaveValue("Senior architect hours");
            await expect(page.getByTestId("prose-before")).toContainText("Edited elsewhere");
            await expect(page.getByTestId("editor-saved")).not.toHaveText("Unsaved changes");
        } finally {
            writeFileSync(resolve(BOOKS, INVOICE_108), original);
        }
    });
});
