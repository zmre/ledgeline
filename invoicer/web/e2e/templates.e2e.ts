// The template editor end to end (invoicer PLAN 4d): create a child of the
// invoice base, override its `theme` block, see the preview change before
// saving, save it, and use it on an invoice; a broken template's error in
// plain words with its line; leaving with unsaved changes; a starter is
// read-only until customized. A click anywhere on a list row opens it; the
// code is coloured (HTML, CSS in <style>, in style attributes and in a
// child's theme block).
//
// Same harness as the other specs: `--server` on :5199 over a scratch copy of
// invoicer/fixtures/accrual, the SPA's `vite preview` on :4273. It runs last,
// as its own project after library.e2e.ts (playwright.config.ts), and puts
// back what it changes: invoice 101's file, and its template is deleted.
// Fixture facts: the books have one template, MyCo default (a child of
// `_invoice_base.v1.html`), used by every invoice; 101 is Contoso's, paid;
// the base's header is `.doc-head h1`, under the logo (`img.logo`) that 105
// and 108 carry.
//
// Written in 4d and NOT run by the agent that wrote it (agents never run
// Playwright); the lead runs it.

import {readFileSync, rmSync, writeFileSync} from "node:fs";
import {resolve} from "node:path";
import {expect, test, type Page} from "@playwright/test";
import {API_TOKEN, COPY} from "../playwright.config";

const FIXED_NOW = new Date(2026, 10, 17, 12, 0, 0);
const BOOKS = resolve(import.meta.dirname, "..", COPY);
const INVOICE_101 = "invoices/2026-09-01 Contoso Invoice 101.md";
const NAME = "E2E Ocean";
const FILE = "invoices/tpl/e2e-ocean.html";
const TEMPLATE = `/${FILE}`;
const BLUE = "rgb(0, 90, 160)";
const BODY = `{% extends "_invoice_base.v1.html" %}
{% block theme %}
{{ super() }}
.sheet .doc-head h1 { color: ${BLUE}; }
{% endblock theme %}
`;

const read = (file: string): string => readFileSync(resolve(BOOKS, file), "utf8");
const frame = (page: Page) => page.frameLocator('[data-testid="invoice-frame"]');
// The code field is CodeMirror (PLAN 4d, D2): a contenteditable `.cm-content`
// with role textbox, which `fill` types into. It has no `value`, so its text
// is read back line by line (`codeText`).
const code = (page: Page) => page.getByRole("textbox", {name: "Template code", exact: true});
const codeText = async (page: Page): Promise<string> => (await code(page).locator(".cm-line").allTextContents()).join("\n");
// Colouring: a token is found by its exact text (lang-css's classes are
// generated names), its innermost span; the theme block's CSS by the
// overlay's own classes (`cm-css-*`).
const plainColour = (page: Page): Promise<string> => code(page).evaluate((el) => getComputedStyle(el).color);
const tokenColour = (page: Page, token: string): Promise<string> =>
    code(page)
        .locator("span", {hasText: new RegExp(`^${token}$`)})
        .last()
        .evaluate((el) => getComputedStyle(el).color);

async function open(page: Page): Promise<void> {
    await page.clock.setFixedTime(FIXED_NOW);
    await page.goto(`/app/#token=${API_TOKEN}`);
    await expect(page).toHaveURL(/\/app\/invoices$/);
}

async function templatesSection(page: Page): Promise<void> {
    // A section link's name ends with its count for screen readers ("Templates 3 templates").
    await page
        .getByTestId("sidebar")
        .getByRole("link", {name: /^Templates(\s|$)/})
        .click();
    await expect(page).toHaveURL(/\/app\/templates$/);
}

test.describe.configure({mode: "serial"});

test.describe("the template editor over the accrual fixture (:5199)", () => {
    let invoice101 = "";

    test.beforeAll(() => {
        invoice101 = read(INVOICE_101);
    });

    test.afterAll(() => {
        writeFileSync(resolve(BOOKS, INVOICE_101), invoice101);
        rmSync(resolve(BOOKS, FILE), {force: true});
    });

    test("a child overriding theme: previewed unsaved, saved, used on an invoice, deleted", async ({page}) => {
        test.setTimeout(120_000);
        await open(page);
        await templatesSection(page);
        const myco = page.getByTestId("template-row").filter({hasText: "MyCo default"});
        await expect(myco).toContainText("Invoice base, version 1");
        await expect(myco.getByTestId("template-starter")).toHaveCount(0);

        // New template: a blank child of the base, named.
        await page.getByTestId("new-template").click();
        await page.getByTestId("new-template-name").fill(NAME);
        await expect(page.getByTestId("new-template-from")).toHaveValue("");
        await page.getByTestId("create-template").click();
        await expect(page).toHaveURL(/\/app\/templates\/invoices\/tpl\/e2e-ocean$/);
        await expect(page.getByTestId("template-heading")).toHaveText(NAME);
        expect(read(FILE)).toBe(`{# invoicer: name="${NAME}" kinds="invoice" #}\n{% extends "_invoice_base.v1.html" %}\n`);
        // The name line is the form's: the code starts with extends.
        await expect.poll(() => codeText(page)).toBe('{% extends "_invoice_base.v1.html" %}\n');
        await expect(page.getByTestId("block-panel-none")).toBeVisible();

        // The preview shows an invoice through it before anything changes.
        await expect(page.getByTestId("invoice-frame")).toHaveAttribute("data-mode", "preview");
        await expect(frame(page).locator(".doc-head h1")).toBeVisible();
        await expect(frame(page).locator(".doc-head h1")).not.toHaveCSS("color", BLUE);
        // The previewed invoice (108, the latest) carries the company's logo
        // (`invoicer_logo`): the base's header shows it, loaded from the books.
        const logo = frame(page).locator("img.logo");
        await expect(logo).toHaveAttribute("alt", "My Company");
        await expect.poll(() => logo.evaluate((img: HTMLImageElement) => img.naturalWidth)).toBeGreaterThan(0);

        // Override block: theme, with super() kept.
        await page.getByTestId("block-choice").selectOption("theme");
        await expect(page.getByTestId("block-about")).toContainText("the CSS for the sheet");
        await page.getByTestId("block-override").click();
        await expect.poll(() => codeText(page)).toMatch(/\{% block theme %\}\n\{\{ super\(\) \}\}\n\{% endblock theme %\}/);
        await expect(page.getByTestId("block-overridden")).toHaveText(["theme"]);

        // Type the rule; the preview follows the unsaved text.
        await code(page).fill(BODY);
        await expect(page.getByTestId("unsaved")).toBeVisible();
        // The theme block is CSS outside any <style>: coloured all the same.
        const property = code(page).locator(".cm-css-property", {hasText: /^color$/});
        await expect(property).toHaveCount(1);
        expect(await property.evaluate((el) => getComputedStyle(el).color)).not.toBe(await plainColour(page));
        await expect(frame(page).locator(".doc-head h1")).toHaveCSS("color", BLUE, {timeout: 10_000});
        expect(read(FILE)).not.toContain(BLUE);

        // Save (Cmd/Ctrl+S): written under the same name line.
        await page.keyboard.press("ControlOrMeta+s");
        await expect(page.getByTestId("template-saved")).toHaveText("Saved.");
        expect(read(FILE)).toBe(`{# invoicer: name="${NAME}" kinds="invoice" #}\n${BODY}`);

        // Use it on invoice 101: MBR's page for 101 now has the blue heading.
        await page.goto("/app/invoices/101");
        await page.getByTestId("template-choice").selectOption(TEMPLATE);
        await page.getByTestId("template-use").click();
        await expect(page.getByTestId("template-current")).toContainText(`This invoice uses ${NAME}`);
        await expect(page.getByTestId("invoice-frame")).toHaveAttribute("data-mode", "page");
        await expect(frame(page).locator(".doc-head h1")).toHaveCSS("color", BLUE);
        expect(read(INVOICE_101)).toContain(`template: ${TEMPLATE}\n`);

        // Listed as used; Delete is refused while 101 uses it.
        await templatesSection(page);
        const row = page.getByTestId("template-row").filter({hasText: NAME});
        await expect(row.getByTestId("template-usage")).toHaveText("1 invoice");
        await row.getByTestId("template-link").click();
        await expect(page.getByTestId("delete-template")).toBeDisabled();
        await expect(page.getByTestId("template-facts")).toContainText("invoices 101");

        // 101 back as it was (the watcher tells the page): then it deletes.
        writeFileSync(resolve(BOOKS, INVOICE_101), invoice101);
        await expect(page.getByTestId("delete-template")).toBeEnabled({timeout: 10_000});
        await page.getByTestId("delete-template").click();
        const dialog = page.getByTestId("delete-template-dialog");
        await expect(dialog.getByRole("button", {name: "Cancel"})).toBeFocused();
        await dialog.getByRole("button", {name: "Delete", exact: true}).click();
        await expect(page).toHaveURL(/\/app\/templates$/);
        await expect(page.getByTestId("template-row").filter({hasText: NAME})).toHaveCount(0);
    });

    test("a broken template says why in plain words, with its line; leaving asks first", async ({page}) => {
        await open(page);
        await templatesSection(page);
        // A click anywhere on a row opens it, not only on the name's link.
        await page.getByTestId("template-row").filter({hasText: "MyCo default"}).getByTestId("template-usage").click();
        await expect(page.getByTestId("template-heading")).toHaveText("MyCo default");
        await expect(page.getByTestId("block-overridden")).toHaveText(["ready"]);
        const saved = read("invoices/tpl/myco-default.html");

        // CSS is coloured in <style> and in a style attribute (lang-css nested
        // in lang-html), and so are the HTML tags around it.
        await code(page).fill(
            '{% extends "_invoice_base.v1.html" %}\n{% block head %}\n<style>.memo { color: red; }</style>\n{% endblock head %}\n{% block footer %}\n<p style="margin: 0">Thanks</p>\n{% endblock footer %}\n'
        );
        const plain = await plainColour(page);
        for (const token of ["color", "margin", "p"]) expect(await tokenColour(page, token), token).not.toBe(plain);
        expect(await tokenColour(page, "color")).toBe(await tokenColour(page, "margin"));

        await code(page).fill('{% extends "_invoice_base.v1.html" %}\n{% block footer %}\n{{ nope.nope }}\n{% endblock footer %}\n');
        const problem = page.getByTestId("template-problem");
        await expect(problem).toContainText("Line 3:");
        await expect(problem).toContainText("Nothing called nope.nope");
        await expect(problem).not.toContainText("/invoices/");
        await expect(page.getByTestId("template-code-problem-line")).toHaveText("3");
        await page.getByTestId("template-problem-go").click();
        await expect(code(page)).toBeFocused();

        // Leaving: Save / Discard / Cancel, focus on Cancel; Escape stays.
        await page
            .getByTestId("sidebar")
            .getByRole("link", {name: /^Templates(\s|$)/})
            .click();
        const unsaved = page.getByTestId("unsaved-dialog");
        await expect(unsaved).toBeVisible();
        await expect(page.getByTestId("unsaved-cancel")).toBeFocused();
        await page.keyboard.press("Escape");
        await expect(unsaved).toHaveCount(0);
        await expect(page).toHaveURL(/\/app\/templates\/invoices\/tpl\/myco-default$/);

        // Discard changes: the code as saved, the problem gone; nothing written.
        await page.getByTestId("discard-template").click();
        await expect(page.getByTestId("unsaved")).toHaveCount(0);
        await expect(page.getByTestId("template-problem")).toHaveCount(0);
        expect(read("invoices/tpl/myco-default.html")).toBe(saved);
    });

    test("a starter is read-only until customized", async ({page}) => {
        await open(page);
        await page.goto("/app/templates/invoices/tpl/classic");
        await expect(page.getByTestId("template-heading")).toHaveText("Classic");
        await expect(page.getByTestId("template-read-only")).toBeVisible();
        await expect(code(page)).toHaveAttribute("aria-readonly", "true");
        await expect(page.getByTestId("customize-template")).toBeVisible();
        await expect(page.getByTestId("save-template")).toHaveCount(0);
        await expect(page.getByTestId("block-choice")).toHaveCount(0);
        // Its preview renders all the same (folded into the base if need be).
        await expect(page.getByTestId("invoice-frame")).toHaveAttribute("data-mode", "preview");
    });
});
