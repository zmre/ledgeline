// Time → Entries and Time → Accounts end to end (plan 46 Phase 4): the
// month's timesheet, a sent invoice's locked lines, Add by `a`, an edit in
// place with the note check, a bulk write-off, Delete, a week-grid cell, and
// an account rename through its preview. Each write is checked in the files.
//
// Its own server: `--server` on :5193 over a scratch copy of
// invoicer/fixtures/time (target/e2e-invoicer-time-entries/, staged fresh per
// run; playwright.config.ts). `vite preview` proxies only to :5199, so this
// spec loads the SPA the server embeds, as cash.e2e.ts does, and skips loudly
// when the binary embeds build.rs's placeholder. Fixture facts at the clock,
// 2026-11-26 (invoicer/fixtures/README.md, `time/`):
//   - November holds 15 lines: Pat 10, Sam 4, Riley 1;
//   - Pat's two October lines on invoice 101 (sent and booked) are locked;
//   - Pat has 1h on clients:acme:advisory on Thursday 2026-11-26;
//   - internal:admin is used in Pat's and Sam's November files.
//
// Serial: every test after the first writes the copy. Written with the
// feature and NOT run by the agent that wrote it (agents never run
// Playwright); the lead runs it.

import {readFileSync} from "node:fs";
import {resolve} from "node:path";
import {expect, test, type Page} from "@playwright/test";
import {API_TOKEN, TIME_ENTRIES_COPY, TIME_ENTRIES_SERVER_URL} from "../playwright.config";

const FIXED_NOW = new Date(2026, 10, 26, 12, 0, 0); // local 2026-11-26, the fixture's clock
const PLACEHOLDER_MARKER = "Ledgeline Invoicer SPA not built";
const BOOKS = resolve(import.meta.dirname, "..", TIME_ENTRIES_COPY);
const PAT_NOVEMBER = "time/pat/2026-11.timedot";

const read = (file: string): string => readFileSync(resolve(BOOKS, file), "utf8");

test.use({baseURL: TIME_ENTRIES_SERVER_URL});
test.describe.configure({mode: "serial"});

/** Open `path` in the SPA the server embeds, with the launch token; skip when it is the placeholder. */
async function open(page: Page, path: string): Promise<void> {
    const shell = await page.request.get(`${TIME_ENTRIES_SERVER_URL}/app/`);
    const body = await shell.text();
    if (body.includes(PLACEHOLDER_MARKER) || !body.includes("__sveltekit")) {
        const why = "The server embeds build.rs's PLACEHOLDER. Run `just invoicer-e2e` (it builds invoicer/web before the binary).";
        if (process.env.REQUIRE_REAL_SPA === "1") throw new Error(`REQUIRE_REAL_SPA=1: ${why}`);
        console.warn(`\n*** SKIPPING invoicer time-entries.e2e: ${why}\n`);
        test.skip(true, why);
    }
    await page.clock.setFixedTime(FIXED_NOW);
    await page.goto(`${TIME_ENTRIES_SERVER_URL}/app${path}#token=${API_TOKEN}`);
}

const row = (page: Page, note: string) => page.getByTestId("entry-row").filter({hasText: note});

test("the month's timesheet, by day, with every line", async ({page}) => {
    await open(page, "/time/entries");
    await expect(page.getByTestId("entries-count")).toContainText("15 entries");
    await expect(page.getByTestId("entry-group").first()).toHaveAttribute("data-key", "2026-11-26");
    await expect(row(page, "call with Fred about the K-1").getByTestId("entry-billing")).toHaveText("Unbilled");
    await expect(row(page, "drive to site").getByTestId("entry-billing")).toHaveText("Not billable");
});

test("a sent invoice's lines are read-only, with a link to the invoice", async ({page}) => {
    await open(page, "/time/entries?period=last-month&billed=invoiced");
    const locked = page.getByTestId("entry-row");
    await expect(locked).toHaveCount(2);
    await expect(locked.first().getByTestId("entry-delete")).toHaveCount(0);
    await locked.first().getByTestId("entry-edit").click();
    const notice = page.getByTestId("entry-locked-notice");
    await expect(notice).toContainText("invoice 101, which is sent");
    await notice.getByRole("link", {name: "Open invoice 101"}).click();
    await expect(page).toHaveURL(/\/app\/invoices\/101$/);
});

test("`a` adds a line to Pat's month file; Enter edits it, refusing a tag in the note", async ({page}) => {
    await open(page, "/time/entries?person=pat");
    await expect(page.getByTestId("entries-count")).toContainText("10 entries");
    await page.keyboard.press("a");
    const add = page.getByTestId("entry-add");
    await add.getByRole("combobox", {name: "Account"}).fill("internal:admin");
    await add.getByRole("combobox", {name: "Account"}).press("Escape");
    await add.getByTestId("entry-add-duration").fill("1h30");
    await add.getByTestId("entry-add-note").fill("expense reports");
    await add.getByTestId("entry-add-note").press("Enter");
    await expect(add).toHaveCount(0);
    await expect(row(page, "expense reports")).toHaveCount(1);
    await expect.poll(() => read(PAT_NOVEMBER)).toMatch(/internal:admin\s+90m\s+; expense reports/);

    // Enter on the cursored line edits it in place.
    await row(page, "expense reports").getByTestId("entry-edit").focus();
    await page.keyboard.press("Enter");
    const edit = page.getByTestId("entry-edit-form");
    await edit.getByTestId("entry-edit-form-note").fill("re: expense reports");
    await expect(edit.getByTestId("entry-edit-form-note-warning")).toContainText("“re:”");
    await edit.getByTestId("entry-edit-form-save").click();
    await expect(edit.getByTestId("entry-edit-form-error")).toBeVisible();
    await edit.getByTestId("entry-edit-form-use-suggestion").click();
    await expect(edit.getByTestId("entry-edit-form-note")).toHaveValue("re expense reports");
    await edit.getByTestId("entry-edit-form-save").click();
    await expect(edit).toHaveCount(0);
    await expect.poll(() => read(PAT_NOVEMBER)).toContain("; re expense reports");
});

test("picked lines are written off in bulk, then one is deleted", async ({page}) => {
    await open(page, "/time/entries?person=pat&q=email");
    const emails = page.getByTestId("entry-row");
    await expect(emails).toHaveCount(2);
    await page.keyboard.press("j");
    await page.keyboard.press("x");
    await page.keyboard.press("j");
    await page.keyboard.press("x");
    await expect(page.getByTestId("bulk-count")).toHaveText("2 picked · 4m");
    await page.getByTestId("bulk-nobill").click();
    await page.getByTestId("bulk-reason").fill("courtesy");
    await page.getByTestId("bulk-reason").press("Enter");
    await expect(page.getByTestId("bulk-bar")).toHaveCount(0);
    await expect(emails.getByTestId("entry-billing").first()).toHaveText("Not billed: courtesy");
    await expect.poll(() => read(PAT_NOVEMBER).match(/nobill: courtesy/g)?.length ?? 0).toBe(2);

    await open(page, "/time/entries?person=pat&q=expense");
    await page.getByTestId("entry-row").getByTestId("entry-delete").click();
    await page.getByTestId("entry-delete-dialog-confirm").click();
    await expect(page.getByTestId("entry-row")).toHaveCount(0);
    await expect.poll(() => read(PAT_NOVEMBER)).not.toContain("expense reports");
});

test("a week-grid cell writes the difference on the day's open line", async ({page}) => {
    await open(page, "/time/entries?group=week&person=pat");
    const cell = page.getByTestId("week-row").filter({hasText: "clients:acme:advisory"}).locator("[data-testid=week-cell][data-date='2026-11-26']");
    await expect(cell).toHaveValue("1h");
    await cell.fill("2h");
    await cell.press("Enter");
    await expect(cell).toHaveValue("2h");
    // The cell shows the typed total at once; the file write follows it.
    await expect.poll(() => read(PAT_NOVEMBER)).toMatch(/clients:acme:advisory\s+(120m|2h)\s+; call with Fred about the K-1/);
});

test("Rename previews every changed line, then rewrites the files", async ({page}) => {
    await open(page, "/time/accounts");
    const admin = page.getByTestId("account-row").and(page.locator("[data-account='internal:admin']"));
    await admin.getByTestId("account-rename").click();
    const dialog = page.getByTestId("account-rename-dialog");
    await dialog.getByTestId("account-rename-to").fill("internal:office");
    await dialog.getByTestId("account-rename-dialog-submit").click();
    await expect(dialog.getByTestId("account-rename-line").first()).toBeVisible();
    // Nothing is written by a preview.
    expect(read("time-accounts.journal")).toContain("account internal:admin");
    await dialog.getByTestId("account-rename-dialog-submit").click();
    await expect(dialog).toHaveCount(0);
    await expect(page.locator("[data-testid=account-row][data-account='internal:office']")).toHaveCount(1);
    await expect.poll(() => read("time-accounts.journal")).toContain("account internal:office");
    await expect.poll(() => read(PAT_NOVEMBER)).not.toContain("internal:admin");
    await expect.poll(() => read("time/sam/2026-11.timedot")).toContain("internal:office");
});
