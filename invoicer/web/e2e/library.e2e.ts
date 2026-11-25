// The catalog and snippets editors end to end (invoicer PLAN 4c): edit the
// catalog (an item added, a rate changed), see the new item offered in the
// invoice editor, and an item written to the catalog on disk offered there
// at once (live reload); then create a snippet, write its text, and see where
// the fixture's Net 30 ACH is used.
//
// Same harness as the other specs: `--server` on :5199 over a scratch copy of
// invoicer/fixtures/accrual, the SPA's `vite preview` on :4273. It runs last,
// as its own project after editor.e2e.ts (playwright.config.ts): that spec
// picks ARCH from the catalog as the fixture has it, and this one changes the
// catalog. Fixture facts: the catalog has four items (ARCH, JR, WKSHP,
// SAAS-Y) in a five-column table (Code, Description, Unit, Rate, *Account*);
// Customer X's `snippets:` names [[Net 30 ACH]], and 108 holds a copy of its
// text. Each test puts back what it changed.
//
// Written in 4c and NOT run by the agent that wrote it (agents never run
// Playwright); the lead runs it.

import {existsSync, readFileSync, rmSync, writeFileSync} from "node:fs";
import {resolve} from "node:path";
import {expect, test, type Page} from "@playwright/test";
import {API_TOKEN, COPY} from "../playwright.config";

const FIXED_NOW = new Date(2026, 10, 17, 12, 0, 0);
const BOOKS = resolve(import.meta.dirname, "..", COPY);
const CATALOG = "invoices/catalog.md";
const LATE_FEE = "invoices/tpl/snippets/Late fee.md";

const read = (file: string): string => readFileSync(resolve(BOOKS, file), "utf8");

async function open(page: Page): Promise<void> {
    await page.clock.setFixedTime(FIXED_NOW);
    await page.goto(`/app/invoices#token=${API_TOKEN}`);
    await expect(page).toHaveURL(/\/app\/invoices$/);
}

/**
 * Replace a prose field's text: MBR's `<mbr-markdown-field>` (Milkdown/ProseMirror)
 * when it loaded, else the plain textarea. The field reports a change when it
 * loses focus, so focus leaves it after.
 */
async function writeProse(page: Page, testid: string, text: string): Promise<void> {
    const rich = await page
        .waitForFunction(() => customElements.get("mbr-markdown-field") !== undefined, undefined, {timeout: 5_000})
        .then(() => true)
        .catch(() => false);
    const host = page.getByTestId(testid);
    if (rich) {
        await expect(host).toHaveAttribute("data-kind", "mbr");
        const editor = host.locator('.ProseMirror[contenteditable="true"]');
        await editor.click();
        await page.keyboard.press("ControlOrMeta+a");
        await page.keyboard.type(text);
    } else {
        await host.fill(text);
    }
    await page.getByRole("heading", {name: "Snippets", exact: true}).click();
}

test.describe.configure({mode: "serial"});

test.describe("the catalog and snippets over the accrual fixture (:5199)", () => {
    let catalogBefore = "";

    test.beforeAll(() => {
        catalogBefore = read(CATALOG);
    });

    test.afterAll(() => {
        writeFileSync(resolve(BOOKS, CATALOG), catalogBefore);
        rmSync(resolve(BOOKS, LATE_FEE), {force: true});
    });

    test("edits the catalog, and the invoice editor offers the new item", async ({page}) => {
        test.setTimeout(90_000);
        await open(page);
        await page.getByTestId("sidebar").getByRole("link", {name: "Catalog"}).click();
        await expect(page).toHaveURL(/\/app\/catalog$/);
        await expect(page.getByTestId("sidebar").getByRole("link", {name: "Catalog"})).toHaveAttribute("aria-current", "page");
        await expect(page.getByTestId("catalog-path")).toHaveText(CATALOG);
        await expect(page.getByTestId("catalog-row")).toHaveCount(4);
        await expect(page.getByLabel("Row 1 code")).toHaveValue("ARCH");

        // A rate changed, an item added. Add item always goes at the bottom
        // (as in the bill grid), whichever row is focused.
        await page.getByLabel("Row 2 rate").fill("$250");
        await page.getByLabel("Row 1 code").focus();
        await page.getByTestId("add-catalog-item").click();
        await expect(page.getByLabel("Row 5 code")).toHaveValue("");
        await expect(page.getByLabel("Row 4 code")).toHaveValue("SAAS-Y");
        await expect(page.getByTestId("catalog-row")).toHaveCount(5);
        // Its own check: a row needs a code before Save.
        await page.getByLabel("Row 5 description").fill("Security audit");
        await expect(page.getByTestId("catalog-problems")).toContainText("Row 5: a row needs a code");
        await expect(page.getByTestId("save-catalog")).toBeDisabled();
        await page.getByLabel("Row 5 code").fill("AUDIT");
        await page.getByLabel("Row 5 unit").fill("ea");
        await page.getByLabel("Row 5 rate").fill("$5,000");
        await page.getByLabel("Row 5 account").fill("revenue:consulting");
        await page.keyboard.press("Escape");
        await expect(page.getByTestId("catalog-problems")).toHaveCount(0);
        await expect(page.getByTestId("unsaved")).toBeVisible();
        await page.getByTestId("save-catalog").click();
        await expect(page.getByTestId("catalog-saved")).toHaveText("Saved: 5 items.");

        // Only the table changed; five cells a row (Code, Description, Unit, Rate, *Account*).
        const saved = read(CATALOG);
        expect(saved.slice(0, saved.indexOf("| Code"))).toBe(catalogBefore.slice(0, catalogBefore.indexOf("| Code")));
        expect(saved).toMatch(/^\| JR\s+\| Junior engineer\s+\| h\s+\| \$250\s+\| revenue:consulting:delivery\s+\|$/m);
        expect(saved).toMatch(/^\| AUDIT\s+\| Security audit\s+\| ea\s+\| \$5,000\s+\| revenue:consulting\s+\|$/m);

        // The invoice editor offers it (108: sent by now, still editable; nothing is saved here).
        await page.goto("/app/invoices/108/edit");
        await expect(page.getByTestId("editor-heading")).toHaveText("Invoice 108");
        await page.getByTestId("add-item").click();
        // `exact`: the open suggestion list is labelled "Row 3 description suggestions".
        const description = page.getByLabel("Row 3 description", {exact: true});
        await description.fill("Security");
        await page.getByRole("option", {name: /^Security audit/}).click();
        await expect(page.getByLabel("Row 3 code")).toHaveValue("AUDIT");
        await expect(page.getByLabel("Row 3 rate")).toHaveValue("$5,000");

        // An item written to the catalog on disk is offered at once (live reload).
        writeFileSync(resolve(BOOKS, CATALOG), `${read(CATALOG)}| PENTEST | Penetration test | ea | $9,000 | revenue:consulting |\n`);
        await expect(async () => {
            await description.fill("");
            await description.fill("Penetration");
            await expect(page.getByRole("option", {name: /^Penetration test/})).toBeVisible({timeout: 1_000});
        }).toPass({timeout: 15_000});
    });

    test("creates a snippet, writes its text, and shows where snippets are used", async ({page}) => {
        test.setTimeout(90_000);
        await open(page);
        await page.getByTestId("sidebar").getByRole("link", {name: "Snippets"}).click();
        await expect(page).toHaveURL(/\/app\/snippets$/);
        const items = page.getByTestId("snippet-item");
        await expect(items).toHaveCount(2);
        await expect(items.nth(0)).toContainText("Net 30 ACH");
        await expect(items.nth(1)).toContainText("Thank you");

        // Net 30 ACH, chosen first: Customer X's default, copied into 108.
        await expect(page.getByTestId("snippet-heading")).toHaveText("Net 30 ACH");
        await expect(page.getByTestId("snippet-customers")).toContainText("Customer X");
        await expect(page.getByTestId("snippet-copies")).toContainText("108");

        // A new one.
        await page.getByTestId("new-snippet").click();
        await page.getByTestId("new-snippet-title").fill("Late fee");
        await page.getByTestId("new-snippet-kind").selectOption("terms");
        await page.getByTestId("create-snippet").click();
        await expect(page.getByTestId("snippet-heading")).toHaveText("Late fee");
        await expect(items).toHaveCount(3);
        expect(existsSync(resolve(BOOKS, LATE_FEE))).toBe(true);
        expect(read(LATE_FEE)).toBe("---\ntitle: Late fee\ntype: snippet\nsnippet_kind: terms\n---\n");
        await expect(page.getByTestId("snippet-uses")).toContainText("No customer names it");

        // Its text.
        await writeProse(page, "snippet-body", "A late fee of 1.5% a month applies after the due date.");
        await expect(page.getByTestId("unsaved")).toBeVisible();
        await page.getByTestId("save-snippet").click();
        await expect(page.getByTestId("snippet-saved")).toHaveText("Saved body.");
        expect(read(LATE_FEE)).toBe(
            "---\ntitle: Late fee\ntype: snippet\nsnippet_kind: terms\n---\n\nA late fee of 1.5% a month applies after the due date.\n"
        );

        // Renamed: the old title stays an alias.
        await page.getByTestId("snippet-title").fill("Late payment fee");
        await page.getByTestId("save-snippet").click();
        await expect(page.getByTestId("snippet-heading")).toHaveText("Late payment fee");
        await expect(page.getByTestId("snippet-aliases")).toContainText("Late fee");
        expect(read(LATE_FEE)).toMatch(/^title: Late payment fee$/m);

        // Deleted (nobody names it).
        await page.getByTestId("delete-snippet").click();
        await page.getByTestId("delete-snippet-dialog").getByRole("button", {name: "Delete", exact: true}).click();
        await expect(items).toHaveCount(2);
        expect(existsSync(resolve(BOOKS, LATE_FEE))).toBe(false);
    });
});
