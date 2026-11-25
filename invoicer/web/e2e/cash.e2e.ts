// Cash books end to end (invoicer I17, ARCHITECTURE §6.3): Apply payment on
// the invoice page, by keyboard (`a`, the deposit, Tab to Apply payment), its
// preview matching what it writes; never offered once paid, nor in accrual
// books. Then New deposit invoice, by `d` and from the New menu: the draft
// opens on its deposit fields.
//
// Its own server: `--server` on :5197 over a scratch copy of
// invoicer/fixtures/cash (target/e2e-invoicer-cash/, staged fresh per run)
// with one imported wire appended to the copy (e2e/fixtures/cash-imports.journal;
// playwright.config.ts). `vite preview` proxies only to :5199, so this spec
// loads the SPA the server embeds, as empty.e2e.ts does, and skips loudly
// when the binary embeds build.rs's placeholder (REQUIRE_REAL_SPA=1 makes
// that a failure). Fixture facts at the clock, 2026-11-17 (README):
//   - 207 (Customer X) is sent and unpaid, $7,000.00 due: $12,000 of work
//     less 206's $5,000 deposit; match-rows puts the rest $3,000 on
//     revenue:consulting and $4,000 on revenue:consulting:delivery, so the
//     $3,500 wire splits $1,500.00 / $2,000.00;
//   - 201 is paid in full;
//   - the deposit account is liabilities:customer-deposits;
//   - on :5199 (the accrual copy, read only here) 106 is sent and unpaid.
//
// Serial: the apply writes the copy's journal. Written with the feature and
// NOT run by the agent that wrote it (agents never run Playwright); the lead
// runs it.

import {readFileSync} from "node:fs";
import {resolve} from "node:path";
import {expect, test, type Page} from "@playwright/test";
import {API_TOKEN, CASH_COPY, CASH_SERVER_URL, SERVER_URL} from "../playwright.config";

const FIXED_NOW = new Date(2026, 10, 17, 12, 0, 0); // local 2026-11-17, the fixture's clock
const PLACEHOLDER_MARKER = "Ledgeline Invoicer SPA not built";
const BOOKS = resolve(import.meta.dirname, "..", CASH_COPY);
const INVOICE_207 = "invoices/2026-11-16 Customer X Invoice 207.md";

const read = (file: string): string => readFileSync(resolve(BOOKS, file), "utf8");

test.use({baseURL: CASH_SERVER_URL});
test.describe.configure({mode: "serial"});

/** Open `path` in the SPA `server` embeds, with the launch token; skip when it is the placeholder. */
async function open(page: Page, path: string, server = CASH_SERVER_URL): Promise<void> {
    const shell = await page.request.get(`${server}/app/`);
    const body = await shell.text();
    if (body.includes(PLACEHOLDER_MARKER) || !body.includes("__sveltekit")) {
        const why = "The server embeds build.rs's PLACEHOLDER. Run `just invoicer-e2e` (it builds invoicer/web before the binary).";
        if (process.env.REQUIRE_REAL_SPA === "1") throw new Error(`REQUIRE_REAL_SPA=1: ${why}`);
        console.warn(`\n*** SKIPPING invoicer cash.e2e: ${why}\n`);
        test.skip(true, why);
    }
    await page.clock.setFixedTime(FIXED_NOW);
    await page.goto(`${server}/app${path}#token=${API_TOKEN}`);
    await expect(page).toHaveURL(new RegExp(`/app${path.replace(/[?]/g, "\\?")}$`));
}

/** The split's rows as "account amount". */
async function shares(page: Page): Promise<string[]> {
    const rows = page.getByTestId("apply-payment-share");
    return rows.evaluateAll((trs) => trs.map((tr) => Array.from(tr.querySelectorAll("td"), (td) => (td.textContent ?? "").trim()).join(" ")));
}

test.describe("cash books (:5197)", () => {
    test("Apply payment from the keyboard: choose the imported wire, see its split, apply it", async ({page}) => {
        await open(page, "/invoices/207");
        const panel = page.getByTestId("books-panel");
        await expect(panel).toContainText("Cash basis");
        await expect(page.getByTestId("books-status")).toContainText("Unpaid");
        await expect(page.getByTestId("books-apply-payment")).toBeVisible();
        expect(read("books.journal")).not.toContain("invoice:207");

        // `a` opens it (listed on the ? sheet), on the best deposit.
        await page.keyboard.press("?");
        await expect(page.getByTestId("key-help")).toContainText("Apply a payment…");
        await page.keyboard.press("Escape");
        await page.keyboard.press("a");
        const dialog = page.getByTestId("apply-payment");
        await expect(dialog).toBeVisible();
        await expect(dialog.getByRole("heading", {name: "Apply a payment to invoice 207", exact: true})).toBeVisible();
        await expect(page.getByTestId("apply-payment-due")).toHaveText("$7,000.00 still due.");
        const wire = dialog.getByRole("radio", {name: /WIRE CUSTOMER X INV 207/});
        await expect(wire).toBeChecked();
        await expect(wire).toBeFocused();
        await expect(page.getByTestId("apply-payment-candidate").first()).toContainText("Names this invoice");

        // The preview: the split the apply writes, and nothing written yet.
        await expect(page.getByTestId("apply-payment-outcome")).toHaveText("Pays $3,500.00; $3,500.00 still due.");
        expect(await shares(page)).toEqual(["revenue:consulting $1,500.00", "revenue:consulting:delivery $2,000.00"]);
        expect(read("books.journal")).not.toContain("invoice:207");
        // No implementation detail: no reference, no tag syntax.
        await expect(dialog).not.toContainText("invoice:");
        await expect(dialog).not.toContainText("[[");

        // Tab past Cancel to Apply payment, Enter.
        await page.keyboard.press("Tab");
        await expect(dialog.getByRole("button", {name: "Cancel", exact: true})).toBeFocused();
        await page.keyboard.press("Tab");
        const apply = page.getByTestId("apply-payment-confirm");
        await expect(apply).toBeFocused();
        await page.keyboard.press("Enter");

        await expect(dialog).toHaveCount(0);
        await expect(page.getByTestId("books-payment-applied")).toContainText("WIRE CUSTOMER X INV 207: Applied $3,500.00; $3,500.00 still due.");
        await expect(page.getByTestId("books-status")).toContainText("Partly paid");
        await expect(page.getByTestId("books-cash-payment")).toHaveCount(1);
        const journal = read("books.journal");
        expect(journal).toContain("2026-11-17 * WIRE CUSTOMER X INV 207  ; id: wx-7, invoice:207");
        expect(journal).toContain("    assets:bank:checking   $3,500.00\n");
        await expect.poll(() => read(INVOICE_207)).toContain("paid: partial 2026-11-17 $3,500");

        // Nothing else can pay it now: the dialog says so, Apply payment stays off; Escape cancels.
        await page.keyboard.press("a");
        await expect(page.getByTestId("apply-payment-none")).toBeVisible();
        await expect(page.getByTestId("apply-payment-confirm")).toBeDisabled();
        await page.keyboard.press("Escape");
        await expect(page.getByTestId("apply-payment")).toHaveCount(0);
    });

    test("not offered on an invoice paid in full", async ({page}) => {
        await open(page, "/invoices/201");
        await expect(page.getByTestId("books-status")).toContainText("Paid in full");
        await expect(page.getByTestId("books-apply-payment")).toHaveCount(0);
        await page.keyboard.press("a");
        await expect(page.getByTestId("apply-payment")).toHaveCount(0);
    });

    test("not offered in accrual books (106 on :5199, read only)", async ({page}) => {
        await open(page, "/invoices/106", SERVER_URL);
        await expect(page.getByTestId("books-panel")).toContainText("Accrual basis");
        await expect(page.getByTestId("books-apply-payment")).toHaveCount(0);
        await page.keyboard.press("a");
        await expect(page.getByTestId("apply-payment")).toHaveCount(0);
    });

    test("New deposit invoice by d: the draft opens on its deposit fields", async ({page}) => {
        await open(page, "/invoices");
        await page.keyboard.press("d");
        await expect(page).toHaveURL(/\/app\/new\?deposit=1$/);
        await expect(page.getByTestId("new-heading")).toHaveText("New deposit invoice");
        const customer = page.getByTestId("new-customer");
        await expect(customer).toBeFocused();
        await page.keyboard.type("Customer X");
        await expect(page.getByRole("listbox", {name: "Customer suggestions"}).getByRole("option", {name: /^Customer X/})).toBeVisible();
        await page.keyboard.press("Enter");
        await expect(customer).toHaveValue("Customer X");
        await page.getByTestId("new-create").click();

        await expect(page).toHaveURL(/\/app\/invoices\/\d+\/edit$/);
        await expect(page.getByTestId("editor-status")).toHaveText("Draft");
        await expect(page.getByLabel("Deposit for note", {exact: true})).toBeVisible();
        await expect(page.getByLabel("Row 1 description", {exact: true})).toHaveValue("Deposit");
        // Nothing went wrong creating it.
        await expect(page.getByTestId("editor-create-warning")).toHaveCount(0);
        const number = /\/invoices\/(\d+)\/edit$/.exec(page.url())?.[1] ?? "";
        expect(number).not.toBe("");
        const file = read(`invoices/2026-11-17 Customer X Invoice ${number}.md`);
        expect(file).toContain("liabilities:customer-deposits");
        expect(file).toContain("status: draft");
    });

    test("New deposit invoice from the New menu, by the keyboard", async ({page}) => {
        await open(page, "/invoices");
        const toggle = page.getByTestId("new-toggle");
        await toggle.focus();
        await page.keyboard.press("ArrowDown");
        const menu = page.getByRole("menu");
        await page.keyboard.press("ArrowDown");
        await expect(menu.getByRole("menuitem", {name: "New deposit invoice", exact: true})).toBeFocused();
        await page.keyboard.press("Enter");
        await expect(page).toHaveURL(/\/app\/new\?deposit=1$/);
        await expect(page.getByTestId("new-heading")).toHaveText("New deposit invoice");
        // And back to a plain invoice: `n`, after leaving the focused field.
        await page.keyboard.press("Escape");
        await page.keyboard.press("n");
        await expect(page).toHaveURL(/\/app\/new$/);
        await expect(page.getByTestId("new-heading")).toHaveText("New invoice");
    });
});
