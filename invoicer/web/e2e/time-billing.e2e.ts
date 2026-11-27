// Time billing end to end (plan 46 Phase 5): Acme Corp's October time billed
// from Unbilled into a draft with its rows and timesheet table, the time
// then billed in Entries and the files, the draft deleted and the time back
// in Unbilled; then Riley's contractor time paid through accounts payable,
// the missing rate and expense account asked for, the bill booked and its
// line tagged. Each write is checked in the files.
//
// Its own server: `--server` on :5192 over a scratch copy of
// invoicer/fixtures/time (target/e2e-invoicer-time-billing/, staged fresh per
// run; playwright.config.ts). `vite preview` proxies only to :5199, so this
// spec loads the SPA the server embeds, as time-entries.e2e.ts does, and
// skips loudly when the binary embeds build.rs's placeholder. Fixture facts
// at the clock, 2026-11-26 (invoicer/fixtures/README.md, `time/`):
//   - Acme Corp (monthly, 6-minute rounding up per row, daily timesheet) has
//     one October line unbilled: Pat's 50m on clients:acme:advisory
//     ("year-end planning"), item ARCH at $250, so 0.90 h and $225.00;
//   - invoice 101 is the last number, so the draft is 102;
//   - Riley Chen's note has no pay_rate or expense_account; Riley has 3h
//     unpaid on 2026-11-10, and the books one payable account.
//
// Serial: the tests build on each other's writes. Written with the feature
// and NOT run by the agent that wrote it (agents never run Playwright); the
// lead runs it.

import {readdirSync, readFileSync} from "node:fs";
import {resolve} from "node:path";
import {expect, test, type Page} from "@playwright/test";
import {API_TOKEN, TIME_BILLING_COPY, TIME_BILLING_SERVER_URL} from "../playwright.config";

const FIXED_NOW = new Date(2026, 10, 26, 12, 0, 0); // local 2026-11-26, the fixture's clock
const PLACEHOLDER_MARKER = "Ledgeline Invoicer SPA not built";
const BOOKS = resolve(import.meta.dirname, "..", TIME_BILLING_COPY);
const PAT_OCTOBER = "time/pat/2026-10.timedot";
const RILEY_NOVEMBER = "time/riley/2026-11.timedot";

const read = (file: string): string => readFileSync(resolve(BOOKS, file), "utf8");
/** The invoice file numbered `number`, or null. */
const invoiceFile = (number: string): string | null => {
    const name = readdirSync(resolve(BOOKS, "invoices")).find((f) => f.endsWith(` Invoice ${number}.md`));
    return name === undefined ? null : `invoices/${name}`;
};

test.use({baseURL: TIME_BILLING_SERVER_URL});
test.describe.configure({mode: "serial"});

/** Open `path` in the SPA the server embeds, with the launch token; skip when it is the placeholder. */
async function open(page: Page, path: string): Promise<void> {
    const shell = await page.request.get(`${TIME_BILLING_SERVER_URL}/app/`);
    const body = await shell.text();
    if (body.includes(PLACEHOLDER_MARKER) || !body.includes("__sveltekit")) {
        const why = "The server embeds build.rs's PLACEHOLDER. Run `just invoicer-e2e` (it builds invoicer/web before the binary).";
        if (process.env.REQUIRE_REAL_SPA === "1") throw new Error(`REQUIRE_REAL_SPA=1: ${why}`);
        console.warn(`\n*** SKIPPING invoicer time-billing.e2e: ${why}\n`);
        test.skip(true, why);
    }
    await page.clock.setFixedTime(FIXED_NOW);
    await page.goto(`${TIME_BILLING_SERVER_URL}/app${path}#token=${API_TOKEN}`);
}

const acme = (page: Page) => page.getByTestId("unbilled-customer").filter({hasText: "Acme Corp"});

test("Unbilled sums each customer's suggestion and counts them in the sidebar", async ({page}) => {
    await open(page, "/time/unbilled");
    await expect(page.getByTestId("unbilled-banner")).toContainText("customers have unbilled time through October");
    await expect(page.getByTestId("unbilled-banner")).toContainText("Acme Corp 50m");
    await expect(page.getByRole("link", {name: "Unbilled time"})).toBeVisible();
    await expect(page.getByTestId("unbilled-needs-customer")).toContainText("clients:orphan");
});

test("Bill… makes Acme's October draft with its row and timesheet, and the time is billed", async ({page}) => {
    await open(page, "/time/unbilled");
    await acme(page).getByRole("button", {name: "Bill Acme Corp, October 2026…"}).click();
    const dialog = page.getByTestId("bill-dialog");
    const row = dialog.getByTestId("bill-row");
    await expect(row).toHaveCount(1);
    await expect(row).toHaveAttribute("data-key", "clients:acme:advisory");
    await expect(row.getByTestId("bill-row-hours")).toContainText("0.90 h");
    await expect(row.getByTestId("bill-row-source")).toHaveText("Set on the time account");
    await expect(dialog.getByTestId("bill-total-amount")).toHaveText("$225.00");
    await expect(dialog.getByTestId("bill-timesheet-row")).toContainText("year-end planning");
    await expect(dialog.getByTestId("bill-dialog-submit")).toBeEnabled();
    // Enter in a field doesn't create; ⌘↵ / Ctrl+↵ does.
    await row.getByTestId("bill-row-description").press("Enter");
    await expect(dialog).toBeVisible();
    await row.getByTestId("bill-row-description").press("ControlOrMeta+Enter");

    await expect(page).toHaveURL(/\/app\/invoices\/102\/edit$/);
    await expect(page.getByTestId("editor-heading")).toHaveText("Invoice 102");
    const file = invoiceFile("102");
    expect(file).not.toBeNull();
    const text = read(file as string);
    expect(text).toMatch(/\| ARCH \| Advisory, Oct 2026 \| 0\.90 \|/);
    expect(text).toContain("## Timesheet");
    expect(text).toMatch(/\| 2026-10-06 \| pat \| year-end planning \| 0\.83 \|/);
    expect(read(PAT_OCTOBER)).toMatch(/year-end planning.*invoice: 102/);

    await open(page, "/time/entries?period=last-month&customer=Acme+Corp");
    await expect(page.getByTestId("entry-row").filter({hasText: "year-end planning"}).getByTestId("entry-billing")).toHaveText("Invoice 102");
    await open(page, "/time/unbilled");
    await expect(acme(page).getByTestId("unbilled-period")).toHaveCount(0);
});

test("Delete draft says the time goes back, and Unbilled has it again", async ({page}) => {
    await open(page, "/invoices/102/edit");
    await page.getByTestId("editor-delete-draft").click();
    const confirm = page.getByTestId("confirm-delete-draft");
    await expect(confirm.getByTestId("confirm-delete-draft-time")).toHaveText("Its 0.83 hours of time go back to Unbilled.");
    await confirm.getByTestId("confirm-delete-draft-confirm").click();
    await expect(page).toHaveURL(/\/app\/invoices$/);
    await expect.poll(() => invoiceFile("102")).toBeNull();
    await expect.poll(() => read(PAT_OCTOBER)).not.toContain("invoice: 102");

    await open(page, "/time/unbilled");
    await expect(acme(page).getByTestId("unbilled-period")).toContainText("October 2026 · 50m");
});

test("Riley's time is paid: the missing rate is asked for, the bill booked and the line tagged", async ({page}) => {
    await open(page, "/time/unbilled");
    await page.getByRole("button", {name: "Create a bill for Riley Chen…"}).click();
    const dialog = page.getByTestId("pay-dialog");
    const rate = dialog.getByTestId("pay-rate");
    await expect(rate).toBeFocused();
    await expect(rate).toHaveValue("");
    await expect(dialog.getByTestId("pay-dialog-submit")).toBeDisabled();
    await expect(dialog.getByTestId("pay-save-to-note")).toBeChecked();
    await rate.fill("$60");
    const expense = dialog.getByRole("combobox", {name: "Expense account"});
    await expense.fill("expenses:contractors");
    await expense.press("Escape");
    await expect(dialog.getByTestId("pay-total")).toHaveText("$180.00");
    await expect(dialog.getByTestId("pay-dialog-submit")).toBeEnabled();
    await rate.press("ControlOrMeta+Enter");

    const booked = page.getByTestId("pay-booked");
    await expect(booked.getByTestId("pay-booked-reference")).toHaveText("riley timesheet 2026-11");
    await expect(booked).toContainText("$180.00 owed to Riley Chen");
    await expect(booked.getByTestId("pay-booked-saved")).toContainText("the hourly rate and the expense account");
    expect(read("books.journal")).toContain("reference: riley timesheet 2026-11");
    expect(read(RILEY_NOVEMBER)).toContain("bill: riley timesheet 2026-11");
    const note = read("people/Riley Chen.md");
    expect(note).toContain("pay_rate: $60");
    expect(note).toContain("expense_account: expenses:contractors");

    await page.getByTestId("pay-dialog-submit").click();
    await expect(page.getByTestId("unpaid-person").filter({hasText: "Riley Chen"})).toHaveCount(0);
});
