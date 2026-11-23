// The invoicer end to end (invoicer PLAN 3b's gate): open the fixture, list
// its invoices, open one (MBR's page for it, and the Books panel), and preview
// it through another template and from unsaved text, writing nothing.
//
// Against `ledgeline-invoicer --server` on :5199 over a scratch copy of
// invoicer/fixtures/accrual (playwright.config.ts), through the SPA's
// `vite preview` on :4273. Fixture facts at the clock, 2026-11-17
// (invoicer/fixtures/README.md):
//   - eight invoices, 101–108; 105 is Customer X's open final invoice,
//     $8,350.00 after the deposit 104; 106 and 107 are overdue; 101 is paid in
//     two parts (2026-10-05); 103 is void; 108 is a draft;
//   - every invoice names /invoices/tpl/myco-default.html, the fixture's own
//     template ("Bill to …"); the starters Classic and Modern are not copied in.
//
// 3c adds the PDF: exported through the API (the `--server` of a desktop
// build prints on its own windowless event loop) and read back with
// `pdftotext` (poppler-utils, in the dev shell), then from the Send panel.
//
// Written in 3b and 3c and NOT run by the agents that wrote it (agents never
// run Playwright); the lead runs it.

import {execFileSync} from "node:child_process";
import {readFileSync, existsSync} from "node:fs";
import {resolve} from "node:path";
import {expect, test, type Page} from "@playwright/test";
import {API_TOKEN, COPY, SERVER_URL} from "../playwright.config";

const FIXED_NOW = new Date(2026, 10, 17, 12, 0, 0); // local 2026-11-17, the fixture's clock
const BOOKS = resolve(import.meta.dirname, "..", COPY);
const INVOICE_105 = "invoices/2026-11-10 Customer X Invoice 105.md";

const read = (file: string): string => readFileSync(resolve(BOOKS, file), "utf8");

/** Open the app the way `--server` prints it: the token in the launch fragment. */
async function open(page: Page): Promise<void> {
    await page.clock.setFixedTime(FIXED_NOW);
    await page.goto(`/app/#token=${API_TOKEN}`);
    await expect(page).toHaveURL(/\/app\/invoices$/);
}

const frame = (page: Page) => page.frameLocator('[data-testid="invoice-frame"]');

test.describe("the invoicer over the accrual fixture (:5199)", () => {
    test("opens the books and lists every invoice", async ({page}) => {
        await open(page);
        await expect(page.getByRole("heading", {name: "Invoices"})).toBeVisible();
        const sidebar = page.getByTestId("sidebar");
        await expect(sidebar.getByRole("link", {name: "Invoices"})).toHaveAttribute("aria-current", "page");
        await expect(page.getByTestId("sidebar-books")).toContainText("books.journal");
        await expect(page.getByTestId("sidebar-books")).toContainText("Accrual basis");

        const rows = page.getByTestId("invoice-row");
        await expect(rows).toHaveCount(8);
        const row = (n: string) => page.locator(`[data-testid="invoice-row"][data-number="${n}"]`);
        await expect(row("105")).toContainText("Customer X");
        await expect(row("105")).toContainText("$8,350.00");
        await expect(row("105").getByTestId("status-badge")).toHaveText("Open");
        await expect(row("106").getByTestId("status-badge")).toHaveText("Overdue");
        await expect(row("101").getByTestId("status-badge")).toHaveText("Paid");
        await expect(row("103").getByTestId("status-badge")).toHaveText("Void");
        await expect(row("108").getByTestId("status-badge")).toHaveText("Draft");
    });

    test("opens an invoice: its page as the template renders it, and the books", async ({page}) => {
        await open(page);
        await page.locator('[data-testid="invoice-row"][data-number="105"]').getByRole("link", {name: "105"}).click();
        await expect(page).toHaveURL(/\/app\/invoices\/105$/);
        await expect(page.getByTestId("invoice-heading")).toHaveText("Invoice 105");
        await expect(page.getByTestId("invoice-total")).toHaveText("$8,350.00");

        // MBR's own page for the invoice, through the session cookie.
        const invoice = frame(page);
        await expect(page.getByTestId("invoice-frame")).toHaveAttribute("data-mode", "page");
        await expect(invoice.getByText("Bill to Customer X")).toBeVisible();
        await expect(invoice.getByText("Less deposit (Invoice 104)")).toBeVisible();
        await expect(invoice.getByText("Total due")).toBeVisible();
        // The books column is in the page and hidden by the template's rule.
        await expect(invoice.getByText("liabilities:customer-deposits")).toBeHidden();

        // The Books panel: the booking (applying the deposit) and the open receivable.
        const books = page.getByTestId("books-panel");
        await expect(books.getByTestId("books-outstanding")).toHaveText("$8,350.00");
        await expect(books.getByTestId("books-entry").first()).toContainText("liabilities:customer-deposits");
        await expect(books.getByTestId("books-mirror")).toContainText("No");
    });

    test("previews an invoice through a starter template and writes nothing", async ({page}) => {
        const before = read(INVOICE_105);
        await open(page);
        await page.goto("/app/invoices/105");
        await expect(page.getByTestId("invoice-frame")).toHaveAttribute("data-mode", "page");

        await page.getByTestId("template-choice").selectOption("/invoices/tpl/modern.html");
        await expect(page.getByTestId("invoice-frame")).toHaveAttribute("data-mode", "preview");
        const preview = frame(page);
        await expect(preview.getByText("Billed to")).toBeVisible();
        await expect(preview.getByText("#105")).toBeVisible();
        await expect(preview.getByText("Less deposit (Invoice 104)")).toBeVisible();
        await expect(preview.getByText("liabilities:customer-deposits")).toBeHidden();

        await expect(preview.getByText("$8,350").first()).toBeVisible();

        await page.getByTestId("template-choice").selectOption("/invoices/tpl/classic.html");
        await expect(frame(page).getByText("No. 105")).toBeVisible();
        await expect(frame(page).getByText("Bill to")).toBeVisible();
        await expect(frame(page).getByText("Total due")).toBeVisible();

        // Back to the invoice's own: MBR's page again.
        await page.getByTestId("template-choice").selectOption("");
        await expect(page.getByTestId("invoice-frame")).toHaveAttribute("data-mode", "page");

        // A preview copies no starter in and touches no invoice.
        expect(existsSync(resolve(BOOKS, "invoices/tpl/modern.html"))).toBe(false);
        expect(existsSync(resolve(BOOKS, "invoices/tpl/classic.html"))).toBe(false);
        expect(read(INVOICE_105)).toBe(before);
    });

    test("a starter previews the draft 108 with its content showing, not a blank frame", async ({page}) => {
        // Patrick's blank starter preview (2026-11-23): the preview was a srcdoc
        // frame, which inherits the SPA's CSP (frame-ancestors 'none'). It is now
        // a page of its own; the frame must show the rendered invoice.
        await open(page);
        await page.goto("/app/invoices/108");
        await expect(page.getByTestId("invoice-frame")).toHaveAttribute("data-mode", "page");
        for (const [template, marker] of [
            ["/invoices/tpl/classic.html", "No. 108"],
            ["/invoices/tpl/modern.html", "#108"],
        ] as const) {
            await page.getByTestId("template-choice").selectOption(template);
            const shown = page.getByTestId("invoice-frame");
            await expect(shown).toHaveAttribute("data-mode", "preview");
            await expect(shown).toHaveAttribute("src", /^\/\.ledgeline-invoicer\/preview\?/);
            await expect(frame(page).getByText(marker)).toBeVisible();
            await expect(frame(page).getByText("Platform subscription, 1 year")).toBeVisible();
            // Sized to its content, not the empty default.
            expect((await shown.boundingBox())?.height ?? 0).toBeGreaterThan(300);
        }
    });

    test("POST /api/preview renders unsaved text", async ({request}) => {
        const before = read(INVOICE_105);
        const source = before.replace("Senior architect hours", "Principal architect hours");
        const response = await request.post(`${SERVER_URL}/api/preview`, {
            headers: {Authorization: `Bearer ${API_TOKEN}`},
            data: {path: INVOICE_105, source, template: "/invoices/tpl/classic.html"},
        });
        expect(response.status()).toBe(200);
        expect(response.headers()["server-timing"]).toMatch(/^render;dur=/);
        const body = (await response.json()) as {html: string; template: string};
        expect(body.template).toBe("/invoices/tpl/classic.html");
        expect(body.html).toContain("Principal architect hours");
        expect(body.html).toContain("mbr-export-ready");
        expect(read(INVOICE_105)).toBe(before);

        const refused = await request.post(`${SERVER_URL}/api/preview`, {data: {number: "105"}});
        expect(refused.status()).toBe(401);
    });

    test("the Books panel says paid, void and never booked", async ({page}) => {
        await open(page);
        await page.goto("/app/invoices/101");
        await expect(page.getByTestId("books-receivable")).toContainText("paid");
        await expect(page.getByTestId("books-mirror")).toContainText("Paid 2026-10-05");

        await page.goto("/app/invoices/103");
        await expect(page.getByTestId("books-panel").getByText("Void", {exact: true})).toBeVisible();

        await page.goto("/app/invoices/108");
        await expect(page.getByTestId("books-empty")).toContainText("drafts never touch the books");
        await expect(frame(page).getByText("Platform subscription, 1 year")).toBeVisible();
    });

    test("POST /api/invoices/105/pdf keeps a real PDF with the invoice's text", async ({request}) => {
        test.setTimeout(90_000);
        const response = await request.post(`${SERVER_URL}/api/invoices/105/pdf`, {
            headers: {Authorization: `Bearer ${API_TOKEN}`},
            data: {today: "2026-11-17"},
        });
        expect(response.status(), await response.text()).toBe(200);
        const body = (await response.json()) as {destination: string; kept_at: string; sent_hash: string; path?: string};
        expect(body.destination).toBe("kept");
        expect(body.kept_at).toBe("invoices/pdf/2026-11-10 Customer X Invoice 105.pdf");
        expect(body.sent_hash).toBe("written");
        expect(body.path).toBeUndefined();

        const file = resolve(BOOKS, body.kept_at);
        expect(readFileSync(file).subarray(0, 5).toString("latin1")).toBe("%PDF-");
        const text = execFileSync("pdftotext", ["-layout", file, "-"], {encoding: "utf8"});
        expect(text).toContain("Bill to Customer X");
        expect(text).toContain("Less deposit (Invoice 104)");
        // The fixture's template prints amounts as the bill table writes them.
        expect(text).toMatch(/Total due\s+\$8,350\b/);
        // The books column is in the page but hidden by the template.
        expect(text).not.toContain("liabilities:customer-deposits");
        expect(read(INVOICE_105)).toMatch(/^sent_hash: [0-9a-f]{16}$/m);

        // A draft goes to a temporary file and leaves the books alone.
        const before = read("invoices/2026-11-17 Customer X Invoice 108.md");
        const draft = await request.post(`${SERVER_URL}/api/invoices/108/pdf`, {
            headers: {Authorization: `Bearer ${API_TOKEN}`},
            data: {},
        });
        expect(draft.status(), await draft.text()).toBe(200);
        expect(((await draft.json()) as {destination: string}).destination).toBe("temp");
        expect(read("invoices/2026-11-17 Customer X Invoice 108.md")).toBe(before);
    });

    test("the Send panel exports a sent invoice and keeps the earlier PDF", async ({page}) => {
        test.setTimeout(90_000);
        await open(page);
        await page.goto("/app/invoices/105");
        const panel = page.getByTestId("send-panel");
        await expect(panel.getByTestId("send-instructions")).toContainText("Create from PO");
        await expect(panel.getByTestId("send-copy-reference")).toBeVisible();
        await panel.getByTestId("send-export").click();
        await expect(panel.getByTestId("send-exported")).toContainText("Saved invoices/pdf/2026-11-10 Customer X Invoice 105.pdf", {timeout: 60_000});
        await expect(panel.getByTestId("send-kept-pdf").first()).toContainText("2026-11-10 Customer X Invoice 105.pdf");
        // `--server` has no window: no Share… or Copy file.
        await expect(panel.getByTestId("send-share")).toHaveCount(0);

        // A draft asks first.
        await page.goto("/app/invoices/108");
        await page.getByTestId("send-export").click();
        await expect(page.getByRole("alertdialog")).toContainText("Mark invoice 108 as sent?");
        await page.getByTestId("send-draft-only").click();
        await expect(page.getByTestId("send-exported")).toContainText("temporary", {timeout: 60_000});
    });
});
